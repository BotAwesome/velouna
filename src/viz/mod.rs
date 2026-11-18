//! Visualizers: the song analysis service (core) and, later, the GL host
//! that runs visualizer plugins (see velouna-plugins/docs/shaders.md).

pub mod analysis;
pub mod decode;
pub mod gl;
pub mod preset;
pub mod score;

use analysis::Analysis;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub enum Status {
    Working,
    Ready(Arc<Analysis>),
    /// Analysis failed (logged); the player falls back to its normal view.
    Failed,
}

struct Job {
    key: String,
    status: Arc<Mutex<Status>>,
    cancel: Arc<AtomicBool>,
}

/// Analyses the playing song in the background, one at a time; results are
/// cached on disk. Only asked for while a visualizer is visible.
#[derive(Default)]
pub struct Songs {
    job: Option<Job>,
}

impl Songs {
    /// The analysis of `src`, starting it when it is not the current job.
    /// `headers` are mpv's `http-header-fields` of a server URL.
    pub fn get(&mut self, src: &str, headers: Option<&str>, repaint: impl Fn() + Send + 'static) -> Status {
        let key = cache_key(src);
        if let Some(j) = &self.job {
            if j.key == key {
                return j.status.lock().unwrap().clone();
            }
        }
        self.cancel();
        let status = Arc::new(Mutex::new(Status::Working));
        let cancel = Arc::new(AtomicBool::new(false));
        let (st, cn, src, headers, k) =
            (status.clone(), cancel.clone(), src.to_string(), headers.map(str::to_string), key.clone());
        let _ = std::thread::Builder::new().name("song analysis".into()).spawn(move || {
            let r = run(&src, headers.as_deref(), &k, &cn);
            if cn.load(Ordering::Relaxed) {
                return;
            }
            *st.lock().unwrap() = match r {
                Ok(a) => Status::Ready(Arc::new(a)),
                Err(e) => {
                    log::warn!("song analysis: {e}");
                    Status::Failed
                }
            };
            repaint();
        });
        self.job = Some(Job { key, status: status.clone(), cancel });
        Status::Working
    }

    /// Stops a running analysis (visualizer hidden, player closed).
    pub fn cancel(&mut self) {
        if let Some(j) = self.job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl Drop for Songs {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn cache_dir() -> PathBuf {
    crate::config::cache_dir().join("analysis")
}

/// Local files: path, size and change time; URLs: the URL itself (hashed,
/// so no token ends up in a file name).
fn cache_key(src: &str) -> String {
    let mut h = Sha256::new();
    h.update(src.as_bytes());
    if let Ok(m) = std::fs::metadata(src) {
        h.update(m.len().to_le_bytes());
        if let Ok(t) = m.modified().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).map_err(std::io::Error::other)) {
            h.update(t.as_secs().to_le_bytes());
        }
    }
    h.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// Cache files are gzipped JSON with the curves as bytes (~60 KB a song).
fn encode(a: &Analysis) -> Option<Vec<u8>> {
    use std::io::Write;
    let json = serde_json::to_vec(a).ok()?;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gz.write_all(&json).ok()?;
    gz.finish().ok()
}

fn decode_cached(b: &[u8]) -> Option<Analysis> {
    use std::io::Read;
    let mut json = Vec::new();
    flate2::read::GzDecoder::new(b).read_to_end(&mut json).ok()?;
    let a: Analysis = serde_json::from_slice(&json).ok()?;
    (a.version == analysis::VERSION).then_some(a)
}

fn load_cached(p: &Path) -> Option<Analysis> {
    let a = decode_cached(&std::fs::read(p).ok()?)?;
    crate::cache::touch(p);
    Some(a)
}

fn run(src: &str, headers: Option<&str>, key: &str, cancel: &Arc<AtomicBool>) -> Result<Analysis, String> {
    let dir = cache_dir();
    let file = dir.join(format!("{key}.gz"));
    if let Some(a) = load_cached(&file) {
        return Ok(a);
    }
    let _ = std::fs::create_dir_all(&dir);
    let t0 = std::time::Instant::now();
    let pcm = decode::decode_mono(src, headers, &dir.join(format!("{key}.pcm")), cancel)?;
    if pcm.len() < decode::RATE as usize {
        return Err("too short".into());
    }
    let t1 = std::time::Instant::now();
    let a = analysis::analyze(&pcm, decode::RATE);
    log::info!(
        "song analysis: {:.0} s of audio, decode {} ms, analysis {} ms, {:.1} bpm, {} sections",
        a.dur,
        (t1 - t0).as_millis(),
        t1.elapsed().as_millis(),
        a.bpm,
        a.sections.len()
    );
    // Return what the cache will give next time, so the first play looks
    // exactly like every later one.
    match encode(&a) {
        Some(b) => {
            let tmp = file.with_extension("tmp");
            if std::fs::write(&tmp, &b).is_ok() {
                let _ = std::fs::rename(&tmp, &file);
            }
            Ok(decode_cached(&b).unwrap_or(a))
        }
        None => Ok(a),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cache_roundtrip() {
        use super::analysis::{Analysis, Hit, VERSION};
        let n = 24_000; // four minutes at 100 fps
        let curve = |k: f32| (0..n).map(|i| (i as f32 * k).sin() * 0.5 + 0.5).collect::<Vec<f32>>();
        let mut a = Analysis { version: VERSION, dur: 240.0, bpm: 128.0, ..Default::default() };
        a.env.mix = curve(0.013);
        a.env.bass = curve(0.051);
        a.wave = curve(0.27);
        a.beats = (0..512).map(|i| i as f32 * 0.46875 + 0.0001234).collect();
        a.kick = (0..500).map(|i| Hit { t: i as f32 * 0.47, s: 0.8123456 }).collect();
        let b = super::encode(&a).unwrap();
        let back = super::decode_cached(&b).unwrap();
        assert_eq!(back.env.mix.len(), n);
        assert!(back.env.mix.iter().zip(&a.env.mix).all(|(x, y)| (x - y).abs() <= 0.5 / 255.0 + 1e-6));
        assert!(back.beats.iter().zip(&a.beats).all(|(x, y)| (x - y).abs() < 0.001));
        assert!((back.kick[3].s - 0.812).abs() < 1e-6);
        assert!(b.len() < 100 * 1024, "{} bytes", b.len());
    }

    /// `VELOUNA_VIZ_SONG=<file> cargo test --release song_analysis -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn song_analysis() {
        let Ok(src) = std::env::var("VELOUNA_VIZ_SONG") else { return };
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let tmp = std::env::temp_dir().join("velouna-viz-test.pcm");
        let t0 = std::time::Instant::now();
        let pcm = super::decode::decode_mono(&src, None, &tmp, &cancel).expect("decode");
        let t1 = std::time::Instant::now();
        let a = super::analysis::analyze(&pcm, super::decode::RATE);
        println!(
            "{:.1} s audio, decode {} ms, analysis {} ms",
            a.dur,
            (t1 - t0).as_millis(),
            t1.elapsed().as_millis()
        );
        println!("bpm {:.2}, beats {}, downbeats {}", a.bpm, a.beats.len(), a.downbeats.len());
        println!("kick {}, snare {}, hat {}, tonal {}", a.kick.len(), a.snare.len(), a.hat.len(), a.tonal.len());
        for s in &a.sections {
            println!("  {:6.1}-{:6.1} {}", s.start, s.end, s.kind.name());
        }
        assert!(a.dur > 1.0);
        let b = super::encode(&a).expect("encode");
        println!("cache file {} KB", b.len() / 1024);
        let back = super::decode_cached(&b).expect("read back");
        assert_eq!(back.env.mix.len(), a.env.mix.len());
        assert!(b.len() < 150 * 1024);

        // The ink score on this song: build once, then the whole song in steps.
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/plugins/velounizer/ink/score.lua")).unwrap();
        let params: Vec<(String, f32)> =
            [("size", 1.0), ("life", 14.0), ("flow", 1.0), ("combs", 1.0), ("vortex", 1.0), ("start", 1.0), ("gloss", 0.4), ("edges", 0.6)].iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let react = [("kick", 1.0), ("snare", 1.0), ("hats", 0.7), ("bass", 1.0), ("voice", 1.0), ("keys", 0.8), ("dynamics", 0.8), ("camera", 0.6)];
        let ctx = super::score::BuildCtx { seed: 1, aw: 16.0 / 9.0, ah: 1.0, params: &params, react: &react };
        let t2 = std::time::Instant::now();
        let sc = super::score::Score::new(&src, "ink/score.lua", std::sync::Arc::new(a.clone()), &ctx).expect("build");
        let t3 = std::time::Instant::now();
        let mut ops = vec![];
        let (mut total, mut most) = (0usize, 0usize);
        let steps = (a.dur * 60.0) as usize;
        for i in 0..steps {
            ops.clear();
            let d = sc.step(i as f32 / 60.0, 1.0 / 60.0, i == 0, &mut ops, 64).expect("step");
            assert!(d > 0.9 && d <= 1.0);
            total += ops.len();
            most = most.max(ops.len());
        }
        println!(
            "ink score: build {} ms, {} steps in {} ms ({:.1} us/step), {} ops, at most {} per step",
            (t3 - t2).as_millis(),
            steps,
            t3.elapsed().as_millis(),
            t3.elapsed().as_micros() as f64 / steps as f64,
            total,
            most
        );
    }
}
