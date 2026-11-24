//! Media helpers: file type detection, ffmpeg/ffprobe workers (thumbnails,
//! probing, waveform, subtitle cue extraction, conversion), subtitle parsing
//! and episode matching.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};

pub const VIDEO_EXT: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "ts", "m2ts", "mts", "3gp", "ogv",
    "vob", "divx", "rmvb", "asf", "f4v",
];
pub const AUDIO_EXT: &[&str] = &[
    "mp3", "flac", "wav", "ogg", "opus", "m4a", "aac", "wma", "alac", "aiff", "aif", "ape", "wv", "mka", "dsf",
    "mpc",
];
pub const SUB_EXT: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "idx", "sup", "smi", "txt"];
pub const PLAYLIST_EXT: &[&str] = &["m3u", "m3u8", "pls", "cue", "xspf"];

fn ext_of(p: &str) -> String {
    Path::new(p).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}
pub fn is_video(p: &str) -> bool {
    VIDEO_EXT.contains(&ext_of(p).as_str())
}
pub fn is_audio(p: &str) -> bool {
    AUDIO_EXT.contains(&ext_of(p).as_str())
}
pub fn is_sub(p: &str) -> bool {
    SUB_EXT.contains(&ext_of(p).as_str())
}
pub fn is_playlist(p: &str) -> bool {
    PLAYLIST_EXT.contains(&ext_of(p).as_str())
}
pub fn is_media(p: &str) -> bool {
    is_video(p) || is_audio(p) || is_playlist(p)
}
pub fn is_url(p: &str) -> bool {
    p.contains("://")
}

pub fn display_name(p: &str) -> String {
    if is_url(p) {
        let trimmed = p.trim_end_matches('/');
        return trimmed.rsplit('/').next().unwrap_or(p).to_string();
    }
    Path::new(p)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string())
}

pub fn parent_folder_name(p: &str) -> Option<String> {
    if is_url(p) {
        return None;
    }
    Path::new(p).parent().and_then(|d| d.file_name()).map(|s| s.to_string_lossy().to_string())
}

pub fn fmt_time(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "--:--:--".into();
    }
    let s = secs as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

pub fn fmt_time_short(secs: f64) -> String {
    if !secs.is_finite() || secs <= 0.0 {
        return "–".into();
    }
    let s = secs as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

pub fn fmt_size(b: i64) -> String {
    let b = b as f64;
    if b > 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b > 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
}

pub fn language_name(code: &str) -> String {
    let l = code.to_ascii_lowercase();
    let n = match l.as_str() {
        "en" | "eng" => t!("Englisch"),
        "de" | "ger" | "deu" => t!("Deutsch"),
        "fr" | "fre" | "fra" => t!("Französisch"),
        "es" | "spa" => t!("Spanisch"),
        "it" | "ita" => t!("Italienisch"),
        "ja" | "jpn" => t!("Japanisch"),
        "ko" | "kor" => t!("Koreanisch"),
        "zh" | "chi" | "zho" => t!("Chinesisch"),
        "ru" | "rus" => t!("Russisch"),
        "pt" | "por" => t!("Portugiesisch"),
        "nl" | "dut" | "nld" => t!("Niederländisch"),
        "pl" | "pol" => t!("Polnisch"),
        "tr" | "tur" => t!("Türkisch"),
        "ar" | "ara" => t!("Arabisch"),
        "sv" | "swe" => t!("Schwedisch"),
        "da" | "dan" => t!("Dänisch"),
        "fi" | "fin" => t!("Finnisch"),
        "no" | "nor" => t!("Norwegisch"),
        "cs" | "cze" | "ces" => t!("Tschechisch"),
        "hu" | "hun" => t!("Ungarisch"),
        "el" | "gre" | "ell" => t!("Griechisch"),
        "he" | "heb" => t!("Hebräisch"),
        "hi" | "hin" => "Hindi",
        "und" => t!("Unbekannt"),
        _ => return code.to_string(),
    };
    n.to_string()
}

/// Natural ordering ("E2" < "E10").
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(|c| c.is_ascii_digit()) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nb.push(c);
                    bi.next();
                }
                let (va, vb) = (na.parse::<u128>().unwrap_or(0), nb.parse::<u128>().unwrap_or(0));
                if va != vb {
                    return va.cmp(&vb);
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

/// Extracts (season, episode) from a file name like "Show_S01E05", "1x05" or "Episode 5".
pub fn episode_key(name: &str) -> Option<(u32, u32)> {
    let s = name.to_lowercase();
    let b = s.as_bytes();
    let num_at = |i: usize| -> Option<(u32, usize)> {
        let mut j = i;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > i { s[i..j].parse().ok().map(|n| (n, j)) } else { None }
    };
    for i in 0..b.len() {
        if b[i] == b's' && (i == 0 || !b[i - 1].is_ascii_alphabetic()) {
            if let Some((season, j)) = num_at(i + 1) {
                let mut k = j;
                while k < b.len() && (b[k] == b' ' || b[k] == b'.' || b[k] == b'_' || b[k] == b'-') {
                    k += 1;
                }
                if k < b.len() && b[k] == b'e' {
                    if let Some((ep, _)) = num_at(k + 1) {
                        return Some((season, ep));
                    }
                }
            }
        }
    }
    for i in 0..b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_digit()) {
            if let Some((season, j)) = num_at(i) {
                if j < b.len() && b[j] == b'x' {
                    if let Some((ep, _)) = num_at(j + 1) {
                        if season < 100 {
                            return Some((season, ep));
                        }
                    }
                }
            }
        }
    }
    // Fallback: last number that is not a resolution/year.
    let mut last = None;
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            if let Some((n, j)) = num_at(i) {
                if !(n == 720 || n == 1080 || n == 2160 || n == 480 || (1900..2100).contains(&n) || n == 264 || n == 265)
                {
                    last = Some(n);
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    last.map(|e| (0, e))
}

/// Picks the best subtitle file for every video (by episode number, then name similarity).
pub fn match_subtitles(videos: &[String], subs: &[String]) -> Vec<Option<String>> {
    videos
        .iter()
        .map(|v| {
            let vn = display_name(v).to_lowercase();
            let vk = episode_key(&vn);
            if let Some(exact) = subs.iter().find(|s| display_name(s).to_lowercase().starts_with(&vn)) {
                return Some(exact.clone());
            }
            if let Some(vk) = vk {
                let cands: Vec<&String> = subs.iter().filter(|s| episode_key(&display_name(s)) == Some(vk)).collect();
                if let Some(c) = cands.first() {
                    return Some((*c).clone());
                }
                // season unknown on one side
                if let Some(c) = subs.iter().find(|s| episode_key(&display_name(s)).map(|k| k.1) == Some(vk.1)) {
                    return Some(c.clone());
                }
            }
            None
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Subtitle cues
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

fn parse_ts(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    let (h, m, sec) = match parts.len() {
        3 => (parts[0].trim().parse::<f64>().ok()?, parts[1].parse::<f64>().ok()?, parts[2].parse::<f64>().ok()?),
        2 => (0.0, parts[0].trim().parse::<f64>().ok()?, parts[1].parse::<f64>().ok()?),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + sec)
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut depth_curly = 0;
    let mut depth_angle = 0;
    for c in s.chars() {
        match c {
            '{' => depth_curly += 1,
            '}' => depth_curly = (depth_curly - 1).max(0),
            '<' => depth_angle += 1,
            '>' => depth_angle = (depth_angle - 1).max(0),
            _ if depth_curly == 0 && depth_angle == 0 => out.push(c),
            _ => {}
        }
    }
    out.replace("\\N", " ").replace("\\n", " ").trim().to_string()
}

pub fn decode_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let u: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&u);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let u: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&u);
    }
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        // Latin-1 fallback for old subtitle files.
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

pub fn parse_cues(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i].trim();
        if let Some(rest) = l.strip_prefix("Dialogue:") {
            let f: Vec<&str> = rest.splitn(10, ',').collect();
            if f.len() == 10 {
                if let (Some(s), Some(e)) = (parse_ts(f[1]), parse_ts(f[2])) {
                    cues.push(Cue { start: s, end: e, text: strip_tags(f[9]) });
                }
            }
            i += 1;
            continue;
        }
        if l.contains("-->") {
            let mut parts = l.split("-->");
            let s = parts.next().and_then(parse_ts);
            let e = parts.next().and_then(|x| parse_ts(x.split_whitespace().next().unwrap_or("")));
            i += 1;
            let mut t = Vec::new();
            while i < lines.len() && !lines[i].trim().is_empty() {
                t.push(strip_tags(lines[i]));
                i += 1;
            }
            if let (Some(s), Some(e)) = (s, e) {
                cues.push(Cue { start: s, end: e, text: t.join(" ") });
            }
            continue;
        }
        i += 1;
    }
    cues.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    cues
}

// ---------------------------------------------------------------------------
// ffmpeg / ffprobe workers
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ProbeInfo {
    pub duration: f64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub track: Option<u32>,
    pub has_video: bool,
    pub has_cover: bool,
    pub width: u32,
    pub height: u32,
    pub mtime: u64,
}

pub enum MediaResult {
    Probe(String, ProbeInfo),
    Thumb { key: String, w: usize, h: usize, rgba: Vec<u8> },
    ThumbFailed(String),
    SeekThumb { key: String, w: usize, h: usize, rgba: Vec<u8> },
    Waveform { path: String, peaks: Vec<f32> },
    Cues { key: String, cues: Vec<Cue> },
    ConvertProgress { done: f64, total: f64 },
    ConvertDone(Result<String, String>),
}

enum Job {
    Probe(String),
    Thumb { path: String, key: String },
}

pub const THUMB_W: usize = 320;
pub const THUMB_H: usize = 180;
pub const SEEK_W: usize = 224;
pub const SEEK_H: usize = 126;

fn hide_console(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn find_tool(name: &str) -> Option<PathBuf> {
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    if let Ok(cur) = std::env::current_exe() {
        if let Some(d) = cur.parent() {
            let p = d.join(&exe);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(&exe)).find(|p| p.is_file())
}

fn mtime(p: &str) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hash_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn run_capture(cmd: &mut Command) -> Option<Vec<u8>> {
    let out = hide_console(cmd).stdin(Stdio::null()).stderr(Stdio::null()).stdout(Stdio::piped()).output().ok()?;
    Some(out.stdout)
}

fn extract_frame(ffmpeg: &Path, path: &str, at: f64, w: usize, h: usize, fast: bool) -> Option<Vec<u8>> {
    let vf = format!(
        "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=0x101010"
    );
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", "error", "-nostdin"]);
    if fast {
        cmd.args(["-skip_frame", "nokey"]);
    }
    if at > 0.0 {
        cmd.args(["-ss", &format!("{at:.2}")]);
    }
    cmd.args(["-i", path, "-map", "0:v:0", "-frames:v", "1", "-an", "-sn", "-vf", &vf])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"]);
    let out = run_capture(&mut cmd)?;
    if out.len() == w * h * 4 { Some(out) } else { None }
}

/// Android decodes frames and reads tags itself (no ffmpeg there).
const SYSTEM_FRAMES: bool = cfg!(target_os = "android");

/// A frame as `w`×`h` RGBA with dark bars: ffmpeg when there is one,
/// otherwise the system's decoder.
pub fn any_frame(ffmpeg: Option<&Path>, path: &str, at: f64, w: usize, h: usize, fast: bool) -> Option<Vec<u8>> {
    if let Some(ff) = ffmpeg {
        return extract_frame(ff, path, at, w, h, fast);
    }
    #[cfg(target_os = "android")]
    {
        let jpg = crate::platform::android::video_frame(path, at, w, h)?;
        return fit_rgba(&image::load_from_memory(&jpg).ok()?, w, h);
    }
    #[allow(unreachable_code)]
    {
        let _ = (path, at, w, h, fast);
        None
    }
}

/// `img` scaled into `w`×`h`, centred on dark bars, as RGBA.
pub fn fit_rgba(img: &image::DynamicImage, w: usize, h: usize) -> Option<Vec<u8>> {
    let fit = img.resize(w as u32, h as u32, image::imageops::FilterType::Triangle).to_rgba8();
    let mut out = image::RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([0x10, 0x10, 0x10, 0xFF]));
    let (x, y) = ((w as u32 - fit.width()) / 2, (h as u32 - fit.height()) / 2);
    image::imageops::overlay(&mut out, &fit, x as i64, y as i64);
    Some(out.into_raw())
}

/// Tags, length and size from the system (Android), for when there is no
/// ffprobe. Songs without tags in Android's reader fall back to the file.
fn system_probe(path: &str) -> Option<ProbeInfo> {
    #[cfg(target_os = "android")]
    {
        let j: serde_json::Value = serde_json::from_str(&crate::platform::android::probe_media(path)?).ok()?;
        let s = |k: &str| j[k].as_str().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        let n = |k: &str| s(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let rot = n("rotation") as i64;
        let (w, h) = (n("width") as u32, n("height") as u32);
        let (width, height) = if rot == 90 || rot == 270 { (h, w) } else { (w, h) };
        return Some(ProbeInfo {
            duration: n("duration_ms") / 1000.0,
            title: s("title"),
            artist: s("artist").or_else(|| s("album_artist")),
            album: s("album"),
            genre: s("genre"),
            track: s("track").and_then(|t| t.split('/').next().and_then(|v| v.trim().parse().ok())),
            has_video: s("has_video").as_deref() == Some("yes"),
            has_cover: j["has_cover"].as_bool().unwrap_or(false),
            width,
            height,
            mtime: mtime(path),
        });
    }
    #[allow(unreachable_code)]
    {
        let _ = path;
        None
    }
}

/// The embedded cover of `path` as a `size`×`size` RGBA square (a cover
/// that isn't square gets dark bars).
pub fn cover_art(ffmpeg: Option<&Path>, path: &str, size: usize) -> Option<Vec<u8>> {
    native_cover(path, size, size).or_else(|| extract_frame(ffmpeg?, path, 0.0, size, size, false))
}

/// The embedded cover read straight from the file (ID3v2, FLAC, MP4), fitted
/// into `w`×`h` with dark bars. Works without ffmpeg (Android).
pub fn native_cover(path: &str, w: usize, h: usize) -> Option<Vec<u8>> {
    let bytes = embedded_picture(path)?;
    fit_rgba(&image::load_from_memory(&bytes).ok()?, w, h)
}

/// Raw bytes of the first embedded picture of an audio file.
fn embedded_picture(path: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
    let syncsafe = |b: &[u8]| (b[0] as usize) << 21 | (b[1] as usize) << 14 | (b[2] as usize) << 7 | b[3] as usize;
    let read_all = |f: &mut std::fs::File, n: usize| -> Option<Vec<u8>> {
        let mut v = Vec::new();
        f.take(n as u64).read_to_end(&mut v).ok()?;
        Some(v)
    };
    let cut = |b: &[u8], from: usize| -> Option<usize> { b.get(from..)?.iter().position(|&c| c == 0).map(|p| from + p) };
    if &head[..3] == b"ID3" {
        let ver = head[3];
        let size = syncsafe(&head[6..10]);
        let mut tag = head[10..].to_vec();
        tag.extend(read_all(&mut f, size.saturating_sub(2))?);
        let mut i = 0;
        if ver >= 3 && head[5] & 0x40 != 0 {
            let n = if ver == 4 { syncsafe(tag.get(0..4)?) } else { be32(tag.get(0..4)?) + 4 };
            i = n;
        }
        while i + 10 <= tag.len() {
            let (id, len, hdr) = if ver == 2 {
                (&tag[i..i + 3], (tag[i + 3] as usize) << 16 | (tag[i + 4] as usize) << 8 | tag[i + 5] as usize, 6)
            } else if ver == 4 {
                (&tag[i..i + 4], syncsafe(&tag[i + 4..i + 8]), 10)
            } else {
                (&tag[i..i + 4], be32(&tag[i + 4..i + 8]), 10)
            };
            if id[0] == 0 || len == 0 {
                break;
            }
            let body = tag.get(i + hdr..i + hdr + len)?;
            if id == b"APIC" || id == b"PIC" {
                let enc = body[0];
                let after_mime = if id == b"PIC" { 4 } else { cut(body, 1)? + 1 };
                let desc_from = after_mime + 1; // picture type
                let data = if enc == 1 || enc == 2 {
                    let mut k = desc_from;
                    while k + 1 < body.len() && !(body[k] == 0 && body[k + 1] == 0) {
                        k += 2;
                    }
                    k + 2
                } else {
                    cut(body, desc_from)? + 1
                };
                return body.get(data..).map(<[u8]>::to_vec);
            }
            i += hdr + len;
        }
        return None;
    }
    if &head[..4] == b"fLaC" {
        let mut buf = head[4..].to_vec();
        buf.extend(read_all(&mut f, 16 << 20)?);
        let mut i = 0;
        while i + 4 <= buf.len() {
            let (last, kind, len) = (buf[i] & 0x80 != 0, buf[i] & 0x7F, (buf[i + 1] as usize) << 16 | (buf[i + 2] as usize) << 8 | buf[i + 3] as usize);
            let body = buf.get(i + 4..i + 4 + len)?;
            if kind == 6 {
                let mut k = 4;
                let ml = be32(body.get(k..k + 4)?);
                k += 4 + ml;
                let dl = be32(body.get(k..k + 4)?);
                k += 4 + dl + 16;
                let n = be32(body.get(k..k + 4)?);
                return body.get(k + 4..k + 4 + n).map(<[u8]>::to_vec);
            }
            if last {
                break;
            }
            i += 4 + len;
        }
        return None;
    }
    if &head[4..8] == b"ftyp" {
        let mut buf = head.to_vec();
        buf.extend(read_all(&mut f, 64 << 20)?);
        // walk moov/udta/meta/ilst/covr/data
        fn find<'a>(b: &'a [u8], path: &[&[u8; 4]]) -> Option<&'a [u8]> {
            let mut i = 0;
            while i + 8 <= b.len() {
                let size = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
                if size < 8 {
                    return None;
                }
                let name = &b[i + 4..i + 8];
                let body = b.get(i + 8..(i + size).min(b.len()))?;
                if name == path[0] {
                    if path.len() == 1 {
                        return Some(body);
                    }
                    let inner = if name == b"meta" { body.get(4..)? } else { body };
                    return find(inner, &path[1..]);
                }
                i += size;
            }
            None
        }
        let data = find(&buf, &[b"moov", b"udta", b"meta", b"ilst", b"covr", b"data"])?;
        return data.get(8..).map(<[u8]>::to_vec);
    }
    None
}

fn probe(ffprobe: &Path, path: &str) -> Option<ProbeInfo> {
    let mut cmd = Command::new(ffprobe);
    cmd.args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams", path]);
    let out = run_capture(&mut cmd)?;
    let j: serde_json::Value = serde_json::from_slice(&out).ok()?;
    let mut info = ProbeInfo { mtime: mtime(path), ..Default::default() };
    let fmt = &j["format"];
    info.duration = fmt["duration"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let tag = |k: &str| -> Option<String> {
        let tags = fmt["tags"].as_object()?;
        tags.iter().find(|(kk, _)| kk.eq_ignore_ascii_case(k)).and_then(|(_, v)| v.as_str()).map(|s| s.to_string())
    };
    info.title = tag("title");
    info.artist = tag("artist").or_else(|| tag("album_artist"));
    info.album = tag("album");
    info.genre = tag("genre");
    info.track = tag("track").and_then(|t| t.split('/').next().and_then(|n| n.trim().parse().ok()));
    if let Some(streams) = j["streams"].as_array() {
        for s in streams {
            if s["codec_type"] == "video" {
                let cover = s["disposition"]["attached_pic"].as_i64() == Some(1);
                if cover {
                    info.has_cover = true;
                } else {
                    info.has_video = true;
                    info.width = s["width"].as_u64().unwrap_or(0) as u32;
                    info.height = s["height"].as_u64().unwrap_or(0) as u32;
                }
            }
        }
    }
    Some(info)
}

pub struct MediaTools {
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
    jobs: Option<Sender<Job>>,
    seek_req: Arc<(Mutex<Option<(String, f64, String)>>, Condvar)>,
    pub results: Receiver<MediaResult>,
    tx: Sender<MediaResult>,
    ctx: eframe::egui::Context,
    pub probe_cache: HashMap<String, ProbeInfo>,
    probe_pending: std::collections::HashSet<String>,
    probe_dirty: bool,
    pub convert_child: Arc<Mutex<Option<std::process::Child>>>,
}

impl MediaTools {
    pub fn new(ctx: eframe::egui::Context) -> Self {
        let ffmpeg = find_tool("ffmpeg");
        let ffprobe = find_tool("ffprobe");
        let (tx, results) = channel::<MediaResult>();
        let jobs;
        {
            let (jtx, jrx) = channel::<Job>();
            let jrx = Arc::new(Mutex::new(jrx));
            for _ in 0..3 {
                let jrx = jrx.clone();
                let tx = tx.clone();
                let ctx = ctx.clone();
                let ffmpeg = ffmpeg.clone();
                let ffprobe = ffprobe.clone();
                std::thread::spawn(move || {
                    loop {
                        let job = {
                            let Ok(rx) = jrx.lock() else { return };
                            match rx.recv() {
                                Ok(j) => j,
                                Err(_) => return,
                            }
                        };
                        match job {
                            Job::Probe(p) => {
                                let info = match &ffprobe {
                                    // Windows: songs' tags and length from the shell
                                    // (no process per file); videos still via ffprobe
                                    #[cfg(windows)]
                                    Some(_) if is_audio(&p) && let Some(mut i) = crate::platform::win::shell_props(&p).filter(|i| i.duration > 0.0) => {
                                        i.has_cover = embedded_picture(&p).is_some();
                                        i.mtime = mtime(&p);
                                        Some(i)
                                    }
                                    Some(fp) => probe(fp, &p),
                                    None => system_probe(&p),
                                };
                                if ffprobe.is_some() || info.is_some() {
                                    let _ = tx.send(MediaResult::Probe(p, info.unwrap_or_default()));
                                }
                            }
                            Job::Thumb { path, key } => {
                                let res = thumb_job(ffmpeg.as_deref(), ffprobe.as_deref(), &path);
                                let _ = tx.send(match res {
                                    Some(rgba) => MediaResult::Thumb { key, w: THUMB_W, h: THUMB_H, rgba },
                                    None => MediaResult::ThumbFailed(key),
                                });
                            }
                        }
                        ctx.request_repaint();
                    }
                });
            }
            jobs = Some(jtx);
        }
        let seek_req: Arc<(Mutex<Option<(String, f64, String)>>, Condvar)> =
            Arc::new((Mutex::new(None), Condvar::new()));
        if ffmpeg.is_some() || SYSTEM_FRAMES {
            let ff = ffmpeg.clone();
            let req = seek_req.clone();
            let tx = tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                loop {
                    let (path, t, key) = {
                        let (m, cv) = &*req;
                        let mut g = m.lock().unwrap();
                        while g.is_none() {
                            g = cv.wait(g).unwrap();
                        }
                        g.take().unwrap()
                    };
                    if let Some(rgba) = any_frame(ff.as_deref(), &path, t, SEEK_W, SEEK_H, true) {
                        let _ = tx.send(MediaResult::SeekThumb { key, w: SEEK_W, h: SEEK_H, rgba });
                        ctx.request_repaint();
                    }
                }
            });
        }
        let probe_cache: HashMap<String, ProbeInfo> =
            std::fs::read_to_string(crate::config::cache_dir().join("probe_cache.json"))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
        Self {
            ffmpeg,
            ffprobe,
            jobs,
            seek_req,
            results,
            tx,
            ctx,
            probe_cache,
            probe_pending: Default::default(),
            probe_dirty: false,
            convert_child: Arc::new(Mutex::new(None)),
        }
    }

    pub fn available(&self) -> bool {
        self.ffmpeg.is_some()
    }

    /// Whether video frames (thumbnails, seek previews) can be made: with
    /// ffmpeg, or by the system on Android.
    pub fn frames(&self) -> bool {
        self.ffmpeg.is_some() || SYSTEM_FRAMES
    }

    pub fn request_probe(&mut self, path: &str) -> Option<&ProbeInfo> {
        if is_url(path) {
            return None;
        }
        if !self.probe_cache.contains_key(path) && !self.probe_pending.contains(path) {
            if let Some(j) = &self.jobs {
                if self.ffprobe.is_some() || SYSTEM_FRAMES {
                    self.probe_pending.insert(path.to_string());
                    let _ = j.send(Job::Probe(path.to_string()));
                }
            }
        }
        self.probe_cache.get(path)
    }

    pub fn on_probe(&mut self, path: String, info: ProbeInfo) {
        self.probe_pending.remove(&path);
        self.probe_cache.insert(path, info);
        self.probe_dirty = true;
    }

    pub fn save_cache(&mut self) {
        if self.probe_dirty {
            if let Ok(s) = serde_json::to_string(&self.probe_cache) {
                let _ = std::fs::write(crate::config::cache_dir().join("probe_cache.json"), s);
            }
            self.probe_dirty = false;
        }
    }

    pub fn thumb_key(path: &str) -> String {
        format!("{:016x}_{}", hash_str(path), mtime(path))
    }

    /// Loads a cached library thumbnail from disk, or schedules generation.
    pub fn request_thumb(&self, path: &str, key: &str) -> Option<Vec<u8>> {
        if let Some(b) = load_thumb(key) {
            return Some(b);
        }
        if let Some(j) = &self.jobs {
            let _ = j.send(Job::Thumb { path: path.to_string(), key: key.to_string() });
        }
        None
    }

    pub fn request_seek_thumb(&self, path: &str, t: f64, key: String) {
        if !self.frames() {
            return;
        }
        let (m, cv) = &*self.seek_req;
        if let Ok(mut g) = m.lock() {
            *g = Some((path.to_string(), t, key));
            cv.notify_one();
        }
    }

    pub fn request_waveform(&self, path: &str) {
        let Some(ff) = self.ffmpeg.clone() else { return };
        let (tx, ctx, path) = (self.tx.clone(), self.ctx.clone(), path.to_string());
        std::thread::spawn(move || {
            let rate = 400;
            let mut cmd = Command::new(&ff);
            cmd.args(["-v", "error", "-nostdin", "-i", &path, "-map", "0:a:0", "-ac", "1", "-ar", &rate.to_string()])
                .args(["-f", "s16le", "pipe:1"]);
            let Some(out) = run_capture(&mut cmd) else { return };
            let samples: Vec<i16> = out.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
            if samples.is_empty() {
                return;
            }
            // 10 peaks per second of audio.
            let per = (rate / 10) as usize;
            let mut peaks: Vec<f32> = samples
                .chunks(per)
                .map(|c| {
                    let rms = (c.iter().map(|&s| (s as f32 / 32768.0).powi(2)).sum::<f32>() / c.len() as f32).sqrt();
                    rms
                })
                .collect();
            let max = peaks.iter().cloned().fold(0.0001f32, f32::max);
            for p in &mut peaks {
                *p = (*p / max).sqrt().min(1.0);
            }
            let _ = tx.send(MediaResult::Waveform { path, peaks });
            ctx.request_repaint();
        });
    }

    pub fn request_cues(&self, key: String, media_path: &str, external: Option<String>, ff_index: Option<i64>) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let ff = self.ffmpeg.clone();
        let media_path = media_path.to_string();
        std::thread::spawn(move || {
            let text = if let Some(ext) = external {
                std::fs::read(&ext).ok().map(|b| decode_text(&b))
            } else if let (Some(ff), Some(idx)) = (ff, ff_index) {
                let mut cmd = Command::new(&ff);
                cmd.args(["-v", "error", "-nostdin", "-i", &media_path, "-map", &format!("0:{idx}"), "-f", "srt", "pipe:1"]);
                run_capture(&mut cmd).map(|b| decode_text(&b))
            } else {
                None
            };
            let cues = text.map(|t| parse_cues(&t)).unwrap_or_default();
            let _ = tx.send(MediaResult::Cues { key, cues });
            ctx.request_repaint();
        });
    }

    pub fn convert(&self, input: String, output: String, args: Vec<String>, total: f64) {
        let Some(ff) = self.ffmpeg.clone() else {
            let _ = self.tx.send(MediaResult::ConvertDone(Err(t!("ffmpeg wurde nicht gefunden").into())));
            return;
        };
        let (tx, ctx, child_slot) = (self.tx.clone(), self.ctx.clone(), self.convert_child.clone());
        std::thread::spawn(move || {
            let mut cmd = Command::new(&ff);
            cmd.args(["-y", "-v", "error", "-nostdin", "-i", &input]).args(&args).args([
                "-progress",
                "pipe:1",
                "-nostats",
                &output,
            ]);
            let child = hide_console(&mut cmd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
            let mut child = match child {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(MediaResult::ConvertDone(Err(e.to_string())));
                    return;
                }
            };
            let stdout = child.stdout.take();
            let mut stderr = child.stderr.take();
            *child_slot.lock().unwrap() = Some(child);
            if let Some(mut so) = stdout {
                let mut buf = [0u8; 4096];
                let mut acc = String::new();
                while let Ok(n) = so.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                    while let Some(pos) = acc.find('\n') {
                        let line: String = acc.drain(..=pos).collect();
                        if let Some(v) = line.trim().strip_prefix("out_time_us=").or(line.trim().strip_prefix("out_time_ms=")) {
                            if let Ok(us) = v.parse::<f64>() {
                                let _ = tx.send(MediaResult::ConvertProgress { done: us / 1e6, total });
                                ctx.request_repaint();
                            }
                        }
                    }
                }
            }
            let status = child_slot.lock().unwrap().take().map(|mut c| c.wait());
            let mut err = String::new();
            if let Some(se) = stderr.as_mut() {
                let _ = se.read_to_string(&mut err);
            }
            let res = match status {
                Some(Ok(s)) if s.success() => Ok(output),
                Some(Ok(_)) => Err(if err.trim().is_empty() { t!("Konvertierung abgebrochen").into() } else { err }),
                Some(Err(e)) => Err(e.to_string()),
                None => Err(t!("Konvertierung abgebrochen").into()),
            };
            let _ = tx.send(MediaResult::ConvertDone(res));
            ctx.request_repaint();
        });
    }

    pub fn cancel_convert(&self) {
        if let Ok(mut g) = self.convert_child.lock() {
            if let Some(c) = g.as_mut() {
                let _ = c.kill();
            }
        }
    }
}

#[cfg(windows)]
/// A picture of `path` for the Windows media overlay: its cover written to
/// the cache, or its library thumbnail.
pub fn overlay_picture(path: &str) -> Option<String> {
    if is_url(path) {
        return None;
    }
    if is_audio(path) {
        let file = crate::config::cache_dir().join("overlay").join(format!("{:016x}.img", hash_str(path)));
        if !file.exists() {
            let Some(bytes) = embedded_picture(path) else { return velouna_cover() };
            std::fs::create_dir_all(file.parent()?).ok()?;
            std::fs::write(&file, bytes).ok()?;
        }
        return Some(file.to_string_lossy().to_string());
    }
    let f = thumb_file(&MediaTools::thumb_key(path));
    f.exists().then(|| f.to_string_lossy().to_string())
}

#[cfg(windows)]
/// The Velouna cover (the app icon) as a PNG in the cache, for songs
/// without a picture of their own.
fn velouna_cover() -> Option<String> {
    let file = crate::config::cache_dir().join("overlay").join("velouna.png");
    if !file.exists() {
        let n = 256;
        let img = image::RgbaImage::from_raw(n as u32, n as u32, crate::logo::logo_rasterize(n))?;
        std::fs::create_dir_all(file.parent()?).ok()?;
        img.save(&file).ok()?;
    }
    Some(file.to_string_lossy().to_string())
}

fn thumb_job(ffmpeg: Option<&Path>, ffprobe: Option<&Path>, path: &str) -> Option<Vec<u8>> {
    if is_audio(path) {
        if let Some(rgba) = native_cover(path, THUMB_W, THUMB_H) {
            save_thumb(&MediaTools::thumb_key(path), &rgba);
            return Some(rgba);
        }
    }
    // Windows: the thumbnail Explorer has (or makes) is much faster than
    // starting ffmpeg for every file
    #[cfg(windows)]
    if !is_audio(path) {
        if let Some((px, w, h)) = crate::platform::win::shell_thumb(path, THUMB_W as i32) {
            if let Some(rgba) = image::RgbaImage::from_raw(w as u32, h as u32, px).and_then(|img| fit_rgba(&image::DynamicImage::ImageRgba8(img), THUMB_W, THUMB_H)) {
                save_thumb(&MediaTools::thumb_key(path), &rgba);
                return Some(rgba);
            }
        }
    }
    if ffmpeg.is_none() && (!SYSTEM_FRAMES || is_audio(path)) {
        return None;
    }
    let at = if is_audio(path) {
        0.0
    } else {
        let d = match ffprobe {
            Some(fp) => probe(fp, path),
            None => system_probe(path),
        };
        let d = d.map(|p| p.duration).unwrap_or(0.0);
        if d > 0.0 { (d * 0.15).min(120.0) } else { 0.0 }
    };
    // keyframes only: much faster on long files, the exact frame doesn't matter
    let rgba = any_frame(ffmpeg, path, at, THUMB_W, THUMB_H, true)
        .or_else(|| any_frame(ffmpeg, path, at, THUMB_W, THUMB_H, false))
        .or_else(|| if at > 0.0 { any_frame(ffmpeg, path, 0.0, THUMB_W, THUMB_H, false) } else { None })?;
    save_thumb(&MediaTools::thumb_key(path), &rgba);
    Some(rgba)
}

fn thumb_file(key: &str) -> PathBuf {
    crate::config::cache_dir().join("thumbs").join(format!("{key}.jpg"))
}

/// Stores a library thumbnail as JPEG (~12 KB instead of 225 KB raw).
fn save_thumb(key: &str, rgba: &[u8]) {
    let Some(img) = image::RgbaImage::from_raw(THUMB_W as u32, THUMB_H as u32, rgba.to_vec()) else { return };
    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
    let mut out = Vec::new();
    if image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 82).encode_image(&rgb).is_err() {
        return;
    }
    let file = thumb_file(key);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // write next to it and rename, so a reader never sees half a file
    let tmp = file.with_extension("tmp");
    if std::fs::write(&tmp, &out).is_ok() {
        let _ = std::fs::rename(&tmp, &file);
    }
}

/// A cached thumbnail as RGBA, if there is one.
fn load_thumb(key: &str) -> Option<Vec<u8>> {
    let file = thumb_file(key);
    let b = std::fs::read(&file).ok()?;
    let img = image::load_from_memory_with_format(&b, image::ImageFormat::Jpeg).ok()?.to_rgba8();
    crate::cache::touch(&file);
    (img.width() as usize == THUMB_W && img.height() as usize == THUMB_H).then(|| img.into_raw())
}

pub fn url_encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            b' ' => o.push_str("%20"),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

/// Lists media files directly inside `dir` (and sub folders) – non recursive.
pub fn list_dir(dir: &Path, show_hidden: bool) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut folders = Vec::new();
    let mut files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            if p.is_dir() {
                folders.push(p);
            } else if is_media(&p.to_string_lossy()) {
                files.push(p);
            }
        }
    }
    folders.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    files.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    (folders, files)
}

/// Recursively collects media files matching `pred` (bounded depth).
pub fn scan_recursive(root: &Path, pred: fn(&str) -> bool, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 || out.len() > 20000 {
        return;
    }
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let p = e.path();
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if p.is_dir() {
                scan_recursive(&p, pred, out, depth + 1);
            } else if pred(&p.to_string_lossy()) {
                out.push(p);
            }
        }
    }
}

pub fn sibling_media(path: &str) -> Vec<String> {
    let Some(dir) = Path::new(path).parent() else { return vec![] };
    let (_, files) = list_dir(dir, false);
    files.into_iter().map(|p| p.to_string_lossy().to_string()).filter(|p| !is_playlist(p)).collect()
}

pub fn file_mtime(p: &Path) -> u64 {
    mtime(&p.to_string_lossy())
}
