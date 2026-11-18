//! Song analysis for the visualizers: envelopes, drum hits, beats, bars and
//! sections (calm/groove/build/drop) over a whole song, from the mix alone.
//! Ported from SunoViz (analysis.rs) without the stem paths.

use rustfft::{FftPlanner, num_complex::Complex32};
use serde::{Deserialize, Serialize};

/// Bumped whenever the result changes, so old cache files are ignored.
/// 2: curves stored as bytes.
pub const VERSION: u32 = 2;

/// 0..1 curves stored as one byte per frame (1/255 steps are invisible in
/// the visuals), written as the change from the frame before: small numbers
/// that pack well.
mod q8 {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &[f32], s: S) -> Result<S::Ok, S::Error> {
        let mut prev = 0u8;
        let d: Vec<i8> = v
            .iter()
            .map(|x| {
                let b = (x.clamp(0.0, 1.0) * 255.0).round() as u8;
                let delta = b.wrapping_sub(prev) as i8;
                prev = b;
                delta
            })
            .collect();
        d.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f32>, D::Error> {
        let mut prev = 0u8;
        Ok(Vec::<i8>::deserialize(d)?
            .into_iter()
            .map(|delta| {
                prev = prev.wrapping_add(delta as u8);
                prev as f32 / 255.0
            })
            .collect())
    }
}

/// Times and strengths rounded to a millisecond / a thousandth.
mod ms {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &[f32], s: S) -> Result<S::Ok, S::Error> {
        let r: Vec<f64> = v.iter().map(|x| (*x as f64 * 1000.0).round() / 1000.0).collect();
        r.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f32>, D::Error> {
        Vec::<f32>::deserialize(d)
    }
}

fn round3<S: serde::Serializer>(x: &f32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64((*x as f64 * 1000.0).round() / 1000.0)
}
/// Envelope frames per second.
pub const FPS: f32 = 100.0;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub struct Hit {
    #[serde(serialize_with = "round3")]
    pub t: f32,
    #[serde(serialize_with = "round3")]
    pub s: f32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionKind {
    Calm,
    Groove,
    Build,
    Drop,
}

impl SectionKind {
    /// 0..1, how strongly visuals react in this section.
    pub fn energy(self) -> f32 {
        match self {
            SectionKind::Calm => 0.25,
            SectionKind::Groove => 0.6,
            SectionKind::Build => 0.75,
            SectionKind::Drop => 1.0,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            SectionKind::Calm => "calm",
            SectionKind::Groove => "groove",
            SectionKind::Build => "build",
            SectionKind::Drop => "drop",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Section {
    pub start: f32,
    pub end: f32,
    pub kind: SectionKind,
}

/// 0..1 curves at [`FPS`].
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Envs {
    #[serde(with = "q8")]
    pub mix: Vec<f32>,
    #[serde(with = "q8")]
    pub sub: Vec<f32>,
    #[serde(with = "q8")]
    pub bass: Vec<f32>,
    #[serde(with = "q8")]
    pub drums: Vec<f32>,
    #[serde(with = "q8")]
    pub mids: Vec<f32>,
    #[serde(with = "q8")]
    pub presence: Vec<f32>,
    #[serde(with = "q8")]
    pub high: Vec<f32>,
    #[serde(with = "q8")]
    pub intensity: Vec<f32>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Analysis {
    pub version: u32,
    pub dur: f32,
    pub bpm: f32,
    #[serde(with = "ms")]
    pub beats: Vec<f32>,
    #[serde(with = "ms")]
    pub downbeats: Vec<f32>,
    pub kick: Vec<Hit>,
    pub snare: Vec<Hit>,
    pub hat: Vec<Hit>,
    /// Note/chord onsets in the mids (keys, guitars, vocals).
    pub tonal: Vec<Hit>,
    pub env: Envs,
    /// Peak envelope for a waveform (100/s).
    #[serde(with = "q8")]
    pub wave: Vec<f32>,
    pub sections: Vec<Section>,
}

/// Linear interpolation in a 100-fps curve.
pub fn env_at(v: &[f32], t: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let x = (t * FPS).max(0.0);
    let i = x.floor() as usize;
    if i + 1 >= v.len() {
        return *v.last().unwrap();
    }
    let f = x - i as f32;
    v[i] * (1.0 - f) + v[i + 1] * f
}

impl Analysis {
    pub fn section_at(&self, t: f32) -> Option<&Section> {
        self.sections.iter().find(|s| t >= s.start && t < s.end).or(self.sections.last())
    }
    pub fn beat_period(&self) -> f32 {
        if self.bpm > 1.0 { 60.0 / self.bpm } else { 0.5 }
    }
    /// Position within the beat and the bar (0..1).
    pub fn phases(&self, t: f32) -> (f32, f32) {
        let ph = |v: &[f32], per: f32| -> f32 {
            if v.len() < 2 {
                return (t / per).fract();
            }
            let i = v.partition_point(|&b| b <= t);
            if i == 0 {
                let d = v[1] - v[0];
                return ((t - v[0]) / d).rem_euclid(1.0);
            }
            if i >= v.len() {
                let d = v[v.len() - 1] - v[v.len() - 2];
                return ((t - v[v.len() - 1]) / d).rem_euclid(1.0);
            }
            (t - v[i - 1]) / (v[i] - v[i - 1]).max(1e-3)
        };
        let p = self.beat_period();
        (ph(&self.beats, p), ph(&self.downbeats, p * 4.0))
    }
}

// ---------------------------------------------------------------- spectral features

struct BandFeat {
    energy: Vec<Vec<f32>>,
    /// Positive difference of the linear band sum (drum hits).
    lin_on: Vec<Vec<f32>>,
    flux: Vec<Vec<f32>>,
    rms: Vec<f32>,
    peak: Vec<f32>,
}

/// STFT (window 2048, hop sr/100, centred): per band log energy, positive
/// spectral flux and the rise of the linear magnitude sum.
fn band_features(x: &[f32], sr: u32, bands: &[(f32, f32)]) -> BandFeat {
    let win = 2048usize;
    let hop = (sr as f32 / FPS).round() as usize;
    let n_frames = x.len() / hop + 1;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(win);
    let hann: Vec<f32> = (0..win).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / win as f32).cos()).collect();
    let bin_hz = sr as f32 / win as f32;
    let ranges: Vec<(usize, usize)> = bands
        .iter()
        .map(|&(a, b)| (((a / bin_hz).floor() as usize).max(1), ((b / bin_hz).ceil() as usize).min(win / 2)))
        .collect();
    let mut energy = vec![vec![0f32; n_frames]; bands.len()];
    let mut flux = vec![vec![0f32; n_frames]; bands.len()];
    let mut lin_on = vec![vec![0f32; n_frames]; bands.len()];
    let mut prev_lin = vec![0f32; bands.len()];
    let mut mag = vec![0f32; win / 2 + 1];
    let mut rms = vec![0f32; n_frames];
    let mut peak = vec![0f32; n_frames];
    let mut buf = vec![Complex32::new(0.0, 0.0); win];
    let mut prev = vec![0f32; win / 2 + 1];
    let mut cur = vec![0f32; win / 2 + 1];
    let mut scratch = vec![Complex32::new(0.0, 0.0); fft.get_inplace_scratch_len()];
    for n in 0..n_frames {
        let c = n * hop;
        let start = c as isize - (win / 2) as isize;
        for i in 0..win {
            let j = start + i as isize;
            let v = if j >= 0 && (j as usize) < x.len() { x[j as usize] } else { 0.0 };
            buf[i] = Complex32::new(v * hann[i], 0.0);
        }
        let (mut sq, mut pk) = (0f32, 0f32);
        for j in c..(c + hop).min(x.len()) {
            sq += x[j] * x[j];
            pk = pk.max(x[j].abs());
        }
        rms[n] = (sq / hop as f32).sqrt();
        peak[n] = pk;
        fft.process_with_scratch(&mut buf, &mut scratch);
        for k in 0..=win / 2 {
            mag[k] = buf[k].norm();
            cur[k] = (1.0 + 100.0 * mag[k]).ln();
        }
        for (b, &(k0, k1)) in ranges.iter().enumerate() {
            let (mut e, mut fl) = (0f32, 0f32);
            for k in k0..k1 {
                e += cur[k];
                let d = cur[k] - prev[k];
                if d > 0.0 {
                    fl += d;
                }
            }
            let w = (k1 - k0).max(1) as f32;
            energy[b][n] = e / w;
            flux[b][n] = fl / w;
            let lin: f32 = mag[k0..k1].iter().sum();
            lin_on[b][n] = (lin - prev_lin[b]).max(0.0);
            prev_lin[b] = lin;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    BandFeat { energy, lin_on, flux, rms, peak }
}

fn percentile(v: &[f32], p: f32) -> f32 {
    let mut s: Vec<f32> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return 0.0;
    }
    s.sort_by(|a, b| a.total_cmp(b));
    s[((s.len() - 1) as f32 * p).round() as usize]
}

/// RMS → 0..1 (dB between the noise floor and the 98th percentile), with
/// attack/release smoothing.
fn rms_env(rms: &[f32]) -> Vec<f32> {
    let db: Vec<f32> = rms.iter().map(|&r| 20.0 * (r.max(1e-5)).log10()).collect();
    let hi = percentile(&db, 0.98);
    let lo = (hi - 42.0).max(-70.0);
    let mut out = Vec::with_capacity(db.len());
    let mut y = 0f32;
    let att = 1.0 - (-1.0 / (FPS * 0.012)).exp();
    let rel = 1.0 - (-1.0 / (FPS * 0.14)).exp();
    for &d in &db {
        let v = ((d - lo) / (hi - lo)).clamp(0.0, 1.0);
        let v = v * v;
        y += (v - y) * if v > y { att } else { rel };
        out.push(y);
    }
    out
}

fn energy_env(e: &[f32]) -> Vec<f32> {
    let hi = percentile(e, 0.98).max(1e-6);
    let lo = percentile(e, 0.05);
    let mut out = Vec::with_capacity(e.len());
    let mut y = 0f32;
    let att = 1.0 - (-1.0 / (FPS * 0.015)).exp();
    let rel = 1.0 - (-1.0 / (FPS * 0.18)).exp();
    for &v in e {
        let v = ((v - lo) / (hi - lo).max(1e-6)).clamp(0.0, 1.0);
        y += (v - y) * if v > y { att } else { rel };
        out.push(y);
    }
    out
}

/// Peak picking in a flux curve: local maximum, adaptive threshold, minimum gap.
fn pick(flux: &[f32], min_gap: f32, delta: f32, floor: f32) -> Vec<Hit> {
    let scale = percentile(&flux.iter().copied().filter(|&v| v > 0.0).collect::<Vec<_>>(), 0.995).max(1e-6);
    let f: Vec<f32> = flux.iter().map(|v| v / scale).collect();
    let n = f.len();
    let gap = (min_gap * FPS).round() as usize;
    let mut out: Vec<(usize, f32)> = Vec::new();
    for i in 0..n {
        let v = f[i];
        if v < floor {
            continue;
        }
        let a = i.saturating_sub(3);
        let b = (i + 4).min(n);
        if f[a..b].iter().any(|&w| w > v) {
            continue;
        }
        let ma = i.saturating_sub(20);
        let mb = (i + 8).min(n);
        let mean = f[ma..mb].iter().sum::<f32>() / (mb - ma) as f32;
        if v < mean + delta {
            continue;
        }
        if let Some(last) = out.last_mut() {
            if i - last.0 < gap {
                if v > last.1 {
                    *last = (i, v);
                }
                continue;
            }
        }
        out.push((i, v));
    }
    let ref_s = percentile(&out.iter().map(|x| x.1).collect::<Vec<_>>(), 0.9).max(1e-6);
    out.into_iter().map(|(i, v)| Hit { t: i as f32 / FPS, s: (v / ref_s).clamp(0.05, 1.0) }).collect()
}

fn smooth(v: &[f32], radius: usize) -> Vec<f32> {
    let n = v.len();
    let mut pre = vec![0f64; n + 1];
    for i in 0..n {
        pre[i + 1] = pre[i] + v[i] as f64;
    }
    (0..n)
        .map(|i| {
            let a = i.saturating_sub(radius);
            let b = (i + radius + 1).min(n);
            ((pre[b] - pre[a]) / (b - a) as f64) as f32
        })
        .collect()
}

fn norm01(v: &[f32], plo: f32, phi: f32) -> Vec<f32> {
    let lo = percentile(v, plo);
    let hi = percentile(v, phi);
    v.iter().map(|x| ((x - lo) / (hi - lo).max(1e-6)).clamp(0.0, 1.0)).collect()
}

// ---------------------------------------------------------------- tempo and beats

fn estimate_period(o: &[f32], hits: &[Hit]) -> f32 {
    // Autocorrelation of the onset curve with harmonics (L, 2L, 4L) against 3:2 mix-ups.
    let n = o.len();
    let mean = o.iter().sum::<f32>() / n.max(1) as f32;
    let x: Vec<f32> = o.iter().map(|v| v - mean).collect();
    let min_lag = (FPS * 60.0 / 200.0) as usize;
    let max_lag = (FPS * 60.0 / 60.0) as usize;
    let top = max_lag * 4 + 4;
    let mut ac = vec![0f32; top + 2];
    for lag in min_lag..=top {
        let mut s = 0f32;
        for i in 0..n.saturating_sub(lag) {
            s += x[i] * x[i + lag];
        }
        ac[lag] = s / (n.saturating_sub(lag)).max(1) as f32;
    }
    let acl = |l: f32| -> f32 {
        let i = l.floor() as usize;
        if i + 1 >= ac.len() {
            return 0.0;
        }
        let f = l - i as f32;
        ac[i] * (1.0 - f) + ac[i + 1] * f
    };
    let score = |l: f32| acl(l) + acl(2.0 * l) + 0.5 * acl(4.0 * l);
    let mut best = (min_lag as f32, f32::MIN);
    let mut l = min_lag as f32;
    while l <= max_lag as f32 {
        let bpm = 60.0 * FPS / l;
        let w = (-0.5 * (bpm / 120.0).log2().powi(2)).exp();
        let v = score(l) * (0.5 + 0.5 * w);
        if v > best.1 {
            best = (l, v);
        }
        l += 0.25;
    }
    let mut lag = best.0;
    let bpm = 60.0 * FPS / lag;
    if bpm < 95.0 && acl(lag / 2.0) > 0.45 * acl(lag) {
        lag /= 2.0;
    } else if bpm > 190.0 {
        lag *= 2.0;
    }
    // Fine tuning: phase coherence of the hits on the eighth-note grid (±3 %).
    if hits.len() > 16 {
        let b0 = 60.0 * FPS / lag;
        let mut bb = (b0, 0f32);
        let mut b = b0 * 0.97;
        while b <= b0 * 1.03 {
            let g = 30.0 / b;
            let (mut re, mut im) = (0f32, 0f32);
            for h in hits {
                let ph = std::f32::consts::TAU * h.t / g;
                re += h.s * ph.cos();
                im += h.s * ph.sin();
            }
            let c = (re * re + im * im).sqrt();
            if c > bb.1 {
                bb = (b, c);
            }
            b += 0.01;
        }
        lag = 60.0 * FPS / bb.0;
    }
    lag
}

/// Dynamic programming beat tracker (Ellis 2007).
fn track_beats(o: &[f32], period: f32) -> Vec<f32> {
    let n = o.len();
    if n < 10 {
        return vec![];
    }
    let sd = {
        let m = o.iter().sum::<f32>() / n as f32;
        (o.iter().map(|v| (v - m) * (v - m)).sum::<f32>() / n as f32).sqrt().max(1e-6)
    };
    let oo: Vec<f32> = o.iter().map(|v| v / sd).collect();
    let alpha = 100.0f32;
    let mut score = vec![0f32; n];
    let mut back = vec![usize::MAX; n];
    let lo = (period * 0.5).round() as usize;
    let hi = (period * 2.0).round() as usize;
    for i in 0..n {
        let mut best = 0f32;
        let mut arg = usize::MAX;
        if i > lo {
            let a = i.saturating_sub(hi);
            for j in a..i - lo {
                let d = (i - j) as f32 / period;
                let v = score[j] - alpha * d.ln().powi(2);
                if arg == usize::MAX || v > best {
                    best = v;
                    arg = j;
                }
            }
        }
        score[i] = oo[i] + if arg == usize::MAX { 0.0 } else { best };
        back[i] = arg;
    }
    let tail = n.saturating_sub(period as usize + 1);
    let mut i = (tail..n).max_by(|&a, &b| score[a].total_cmp(&score[b])).unwrap_or(n - 1);
    let mut beats = vec![];
    loop {
        beats.push(i as f32 / FPS);
        let j = back[i];
        if j == usize::MAX {
            break;
        }
        i = j;
    }
    beats.reverse();
    // Leading silence: drop first beats without onset energy.
    let thr = 0.15 * percentile(o, 0.95);
    let mut first = 0;
    while first < beats.len() && {
        let k = (beats[first] * FPS) as usize;
        let a = k.saturating_sub(3);
        o[a.min(n)..(k + 4).min(n)].iter().fold(0f32, |m, v| m.max(*v)) < thr
    } {
        first += 1;
    }
    beats.split_off(first.min(beats.len().saturating_sub(1)))
}

// ---------------------------------------------------------------- sections

fn detect_sections(dur: f32, downbeats: &[f32], intensity: &[f32], density: &dyn Fn(f32, f32) -> f32) -> Vec<Section> {
    // Bars from the downbeats, padded with the same length before and after.
    let mut bars: Vec<f32> = downbeats.to_vec();
    if bars.len() < 4 {
        let step = 2.0;
        bars = (0..((dur / step) as usize + 1)).map(|i| i as f32 * step).collect();
    }
    if bars.len() < 2 {
        return vec![Section { start: 0.0, end: dur, kind: SectionKind::Groove }];
    }
    let blen = ((bars[bars.len() - 1] - bars[0]) / (bars.len() - 1) as f32).max(0.25);
    while bars[0] > 0.05 {
        let v = (bars[0] - blen).max(0.0);
        bars.insert(0, v);
    }
    while *bars.last().unwrap() < dur {
        let v = bars.last().unwrap() + blen;
        bars.push(v.min(dur));
        if v >= dur {
            break;
        }
    }
    let nb = bars.len() - 1;
    let mean_in = |a: f32, b: f32| -> f32 {
        let i0 = (a * FPS) as usize;
        let i1 = ((b * FPS) as usize).min(intensity.len()).max(i0 + 1);
        if i0 >= intensity.len() {
            return 0.0;
        }
        intensity[i0..i1].iter().sum::<f32>() / (i1 - i0) as f32
    };
    let lv: Vec<f32> = (0..nb).map(|i| mean_in(bars[i], bars[i + 1])).collect();
    let mut kind: Vec<SectionKind> = lv
        .iter()
        .map(|&v| if v > 0.62 { SectionKind::Drop } else if v < 0.3 { SectionKind::Calm } else { SectionKind::Groove })
        .collect();
    // Smoothing: runs shorter than 4 bars take a neighbour's kind (repeatedly).
    for _ in 0..4 {
        let mut i = 0;
        while i < nb {
            let mut j = i;
            while j < nb && kind[j] == kind[i] {
                j += 1;
            }
            if j - i < 4 {
                let left = if i > 0 { Some(kind[i - 1]) } else { None };
                let right = if j < nb { Some(kind[j]) } else { None };
                let rep = match (left, right) {
                    (Some(l), Some(r)) => {
                        let ml = lv[i..j].iter().sum::<f32>() / (j - i) as f32;
                        if (ml - l.energy()).abs() < (ml - r.energy()).abs() { l } else { r }
                    }
                    (Some(l), None) => l,
                    (None, Some(r)) => r,
                    _ => kind[i],
                };
                for k in kind.iter_mut().take(j).skip(i) {
                    *k = rep;
                }
            }
            i = j;
        }
    }
    // Build-up: before each drop, when the onset density rises in the last bars.
    let mut i = 1;
    while i < nb {
        if kind[i] == SectionKind::Drop && kind[i - 1] != SectionKind::Drop {
            let mut a = i;
            while a > 0 && kind[a - 1] == kind[i - 1] {
                a -= 1;
            }
            let run = i - a;
            if run >= 6 {
                let tail = (run / 2).clamp(2, 8);
                let early = density(bars[a], bars[i - tail]);
                let late = density(bars[i - tail], bars[i]);
                let rising = late > early * 1.25 + 0.5;
                if rising || kind[i - 1] == SectionKind::Calm {
                    let tail = if rising { tail } else { tail.min(4) };
                    for k in kind.iter_mut().take(i).skip(i - tail) {
                        *k = SectionKind::Build;
                    }
                }
            }
        }
        i += 1;
    }
    let mut out: Vec<Section> = vec![];
    for b in 0..nb {
        match out.last_mut() {
            Some(s) if s.kind == kind[b] => s.end = bars[b + 1],
            _ => out.push(Section { start: bars[b], end: bars[b + 1], kind: kind[b] }),
        }
    }
    if let Some(s) = out.first_mut() {
        s.start = 0.0;
    }
    if let Some(s) = out.last_mut() {
        s.end = dur;
    }
    out
}

// ---------------------------------------------------------------- main entry

/// Analyses a mono signal at `sr` Hz.
pub fn analyze(mono: &[f32], sr: u32) -> Analysis {
    let dur = mono.len() as f32 / sr as f32;
    let n_frames = mono.len() / ((sr as f32 / FPS).round() as usize) + 1;
    let fit = |mut v: Vec<f32>| -> Vec<f32> {
        v.resize(n_frames, 0.0);
        v
    };
    // One STFT pass for all bands: 0-4 spectrum bands, 5-8 drum bands.
    let bands = [
        (20.0, 60.0),
        (60.0, 250.0),
        (250.0, 2000.0),
        (2000.0, 6000.0),
        (6000.0, 16000.0),
        (30.0, 120.0),
        (180.0, 2500.0),
        (1500.0, 6000.0),
        (6000.0, 11000.0),
    ];
    let f = band_features(mono, sr, &bands);

    let mut env = Envs {
        mix: fit(rms_env(&f.rms)),
        sub: fit(energy_env(&f.energy[0])),
        bass: fit(energy_env(&f.energy[1])),
        mids: fit(energy_env(&f.energy[2])),
        presence: fit(energy_env(&f.energy[3])),
        high: fit(energy_env(&f.energy[4])),
        ..Default::default()
    };
    // Drum loudness without stems: low and high drum band energy together.
    let drum_raw: Vec<f32> = (0..n_frames).map(|i| 0.6 * f.energy[5][i] + 0.4 * f.energy[8][i]).collect();
    env.drums = fit(energy_env(&drum_raw));

    let kick = pick(&f.lin_on[5], 0.09, 0.12, 0.1);
    let snare_raw = pick(&f.lin_on[6], 0.07, 0.1, 0.1);
    let snare: Vec<Hit> = snare_raw.into_iter().filter(|h| !kick.iter().any(|k| (k.t - h.t).abs() < 0.015)).collect();
    let hat = pick(&f.lin_on[8], 0.05, 0.1, 0.1);
    let snare_flux: Vec<f32> = f.flux[6].iter().zip(&f.flux[7]).map(|(a, b)| 0.4 * a + 0.6 * b).collect();
    let tonal = pick(&f.flux[2], 0.15, 0.12, 0.25);

    let onset: Vec<f32> = (0..f.flux[5].len()).map(|i| f.flux[5][i] + 0.8 * snare_flux[i] + 0.3 * f.flux[8][i]).collect();
    let all_hits: Vec<Hit> = kick.iter().chain(snare.iter()).chain(hat.iter()).copied().collect();
    let period = estimate_period(&onset, &all_hits);
    let bpm = 60.0 * FPS / period;
    let beats = track_beats(&onset, period);
    // Downbeat phase: the one with the most kick weight.
    let kick_near = |t: f32| -> f32 { kick.iter().filter(|k| (k.t - t).abs() < 0.06).map(|k| k.s).sum() };
    let mut best = (0usize, -1f32);
    for ph in 0..4 {
        let s: f32 = beats.iter().skip(ph).step_by(4).map(|&b| kick_near(b)).sum();
        if s > best.1 {
            best = (ph, s);
        }
    }
    let downbeats: Vec<f32> = beats.iter().skip(best.0).step_by(4).copied().collect();

    let raw: Vec<f32> = (0..n_frames).map(|i| 0.45 * env.drums[i] + 0.33 * env.bass[i] + 0.22 * env.mix[i]).collect();
    env.intensity = norm01(&smooth(&raw, (FPS * 1.2) as usize), 0.04, 0.97);

    let density = |a: f32, b: f32| -> f32 {
        let n = snare.iter().chain(hat.iter()).filter(|h| h.t >= a && h.t < b).count() as f32;
        n / (b - a).max(0.5)
    };
    let sections = detect_sections(dur, &downbeats, &env.intensity, &density);

    let m = percentile(&f.peak, 0.999).max(1e-6);
    let wave = fit(f.peak.iter().map(|v| (v / m).min(1.0)).collect());

    Analysis { version: VERSION, dur, bpm, beats, downbeats, kick, snare, hat, tonal, env, wave, sections }
}
