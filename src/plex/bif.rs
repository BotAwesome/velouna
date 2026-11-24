//! Plex's own seek previews: the BIF index of a part (a list of JPEGs, one
//! every few seconds), fetched once per file.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, LazyLock, Mutex};

struct Bif {
    /// Start of each picture in ms, and where its JPEG lies in `data`.
    frames: Vec<(u32, usize, usize)>,
    data: Vec<u8>,
}

enum State {
    Loading,
    Ready(Arc<Bif>),
    Failed,
}

static CACHE: LazyLock<Mutex<HashMap<String, State>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Address of the BIF for a Plex stream (direct play of a part).
fn bif_url(url: &str) -> Option<String> {
    let r = super::remote(url)?;
    if r.conn.backend.media_browser() {
        return None;
    }
    let rest = url.split("/library/parts/").nth(1)?;
    let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if id.is_empty() {
        return None;
    }
    Some(r.conn.with_token(&format!("/library/parts/{id}/indexes/sd")))
}

fn parse(data: Vec<u8>) -> Option<Bif> {
    if data.len() < 64 || &data[..8] != [0x89, b'B', b'I', b'F', 0x0d, 0x0a, 0x1a, 0x0a] {
        return None;
    }
    let u32_at = |i: usize| data.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let count = u32_at(12)? as usize;
    let interval = match u32_at(16)? {
        0 => 1000,
        n => n,
    };
    let mut frames = Vec::with_capacity(count);
    for k in 0..count {
        let (ts, off) = (u32_at(64 + k * 8)?, u32_at(64 + k * 8 + 4)? as usize);
        let end = u32_at(64 + (k + 1) * 8 + 4)? as usize;
        if ts == u32::MAX || end <= off || end > data.len() {
            break;
        }
        frames.push((ts.saturating_mul(interval), off, end));
    }
    (!frames.is_empty()).then_some(Bif { frames, data })
}

/// Whether `url` is a Plex stream that may have previews (not known yet
/// counts as yes; a failed fetch as no).
pub fn has(url: &str) -> bool {
    let Some(b) = bif_url(url) else { return false };
    !matches!(CACHE.lock().unwrap().get(&b), Some(State::Failed))
}

/// The JPEG shown at `secs` and its index, fetching the BIF on first use.
pub fn frame(ctx: &eframe::egui::Context, url: &str, secs: f64) -> Option<(usize, Vec<u8>)> {
    let b = bif_url(url)?;
    let mut cache = CACHE.lock().unwrap();
    match cache.get(&b) {
        Some(State::Ready(bif)) => {
            let ms = (secs * 1000.0).max(0.0) as u32;
            let i = bif.frames.partition_point(|f| f.0 <= ms).saturating_sub(1);
            let (_, s, e) = bif.frames[i];
            Some((i, bif.data[s..e].to_vec()))
        }
        Some(_) => None,
        None => {
            cache.insert(b.clone(), State::Loading);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let got = ureq::get(&b).timeout(std::time::Duration::from_secs(30)).call().ok().and_then(|r| {
                    let mut v = Vec::new();
                    r.into_reader().take(256 << 20).read_to_end(&mut v).ok()?;
                    parse(v)
                });
                let st = match got {
                    Some(bif) => State::Ready(Arc::new(bif)),
                    None => State::Failed,
                };
                CACHE.lock().unwrap().insert(b, st);
                ctx.request_repaint();
            });
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_bif() {
        let jpgs: [&[u8]; 2] = [b"AAAA", b"BBBBBB"];
        let mut d = vec![0x89, b'B', b'I', b'F', 0x0d, 0x0a, 0x1a, 0x0a];
        d.extend(0u32.to_le_bytes());
        d.extend(2u32.to_le_bytes());
        d.extend(5000u32.to_le_bytes());
        d.resize(64, 0);
        let start = 64 + 3 * 8;
        let offs = [start, start + 4, start + 10];
        d.extend(0u32.to_le_bytes());
        d.extend((offs[0] as u32).to_le_bytes());
        d.extend(1u32.to_le_bytes());
        d.extend((offs[1] as u32).to_le_bytes());
        d.extend(u32::MAX.to_le_bytes());
        d.extend((offs[2] as u32).to_le_bytes());
        for j in jpgs {
            d.extend_from_slice(j);
        }
        let b = super::parse(d).unwrap();
        assert_eq!(b.frames.len(), 2);
        assert_eq!(b.frames[1].0, 5000);
        assert_eq!(&b.data[b.frames[1].1..b.frames[1].2], b"BBBBBB");
    }
}
