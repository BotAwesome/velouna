//! Whole-song decoding for the analysis: a second, headless mpv decodes the
//! playing file (or server URL) as fast as it can into a temporary file of
//! raw mono f32 samples. Same on Windows, Linux and Android, every format
//! the player itself can play, no extra decoder.

use crate::mpv::{Event, Mpv};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const RATE: u32 = 44100;

/// Decodes `src` to mono f32 at [`RATE`]. `headers` are mpv's
/// `http-header-fields` for server URLs. Stops early (Err) when `cancel` is set.
pub fn decode_mono(src: &str, headers: Option<&str>, tmp: &Path, cancel: &Arc<AtomicBool>) -> Result<Vec<f32>, String> {
    let _ = std::fs::remove_file(tmp);
    let res = run(src, headers, tmp, cancel);
    let out = res.and_then(|_| read_f32(tmp));
    let _ = std::fs::remove_file(tmp);
    out
}

fn run(src: &str, headers: Option<&str>, tmp: &Path, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    let mpv = Mpv::create()?;
    let tmp_s = tmp.to_string_lossy().to_string();
    for (k, v) in [
        ("config", "no"),
        ("terminal", "no"),
        ("idle", "yes"),
        ("vid", "no"),
        ("sid", "no"),
        ("vo", "null"),
        ("ao", "pcm"),
        ("ao-pcm-file", tmp_s.as_str()),
        ("ao-pcm-waveheader", "no"),
        ("audio-format", "float"),
        ("audio-channels", "mono"),
        ("audio-samplerate", "44100"),
        ("untimed", "yes"),
        ("audio-display", "no"),
        ("ytdl", "no"),
        ("load-scripts", "no"),
        ("input-default-bindings", "no"),
        ("resume-playback", "no"),
        ("save-position-on-quit", "no"),
        ("replaygain", "no"),
        ("volume", "100"),
        ("gapless-audio", "no"),
        ("keep-open", "no"),
    ] {
        mpv.set_option(k, v).map_err(|e| format!("{k}: {e}"))?;
    }
    if let Some(h) = headers.filter(|h| !h.is_empty()) {
        mpv.set_option("http-header-fields", h)?;
    }
    mpv.initialize()?;
    mpv.command_untimed(&["loadfile", src, "replace"])?;
    let started = Instant::now();
    let mut loaded = false;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        // A file that never opens (dead server) must not hang the worker.
        if !loaded && started.elapsed() > Duration::from_secs(30) {
            return Err("timeout".into());
        }
        match mpv.wait_event(0.25) {
            Some(Event::FileLoaded) => loaded = true,
            Some(Event::EndFile { reason, error }) => {
                return if reason == crate::mpv::END_EOF {
                    Ok(())
                } else {
                    Err(format!("end {reason} ({})", mpv.error_string(error)))
                };
            }
            Some(Event::Shutdown) => return Err("shutdown".into()),
            _ => {}
        }
    }
}

fn read_f32(p: &Path) -> Result<Vec<f32>, String> {
    let b = std::fs::read(p).map_err(|e| e.to_string())?;
    Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}
