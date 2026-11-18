//! Artwork loading for the Plex views (and radio station logos).
//!
//! Images are scaled by the server's photo transcoder to the size a card needs,
//! cached on disk, decoded on worker threads and kept on the GPU in a small LRU.
//! Requests are served newest-first so fast scrolling loads what is on screen.

use super::api::Http;
use eframe::egui::{self, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

const MAX_QUEUE: usize = 240;
const MAX_TEXTURES: usize = 360;
const FADE_SECS: f32 = 0.22;

struct Job {
    key: String,
    url: String,
}

type Fetch = Arc<dyn Fn(&str) -> Result<Vec<u8>, String> + Send + Sync>;

/// Where and how a loader gets its images.
#[derive(Clone)]
struct Source {
    fetch: Fetch,
    dir: PathBuf,
    /// Longest side after decoding (None: as delivered).
    max_side: Option<u32>,
}

enum Done {
    Ok { key: String, w: usize, h: usize, rgba: Vec<u8> },
    Failed(String),
    Dropped(String),
}

enum Img {
    Loading,
    Ready { tex: TextureHandle, used: u64, born: Instant },
    Failed,
}

pub struct Shown {
    pub id: egui::TextureId,
    pub size: egui::Vec2,
    pub alpha: f32,
}

pub struct Images {
    queue: Arc<(Mutex<Vec<Job>>, Condvar)>,
    rx: Receiver<Done>,
    tx: Sender<Done>,
    entries: HashMap<String, Img>,
    frame: u64,
    fading: bool,
    dir: PathBuf,
}

fn disk_path(dir: &std::path::Path, key: &str) -> PathBuf {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    let _ = std::fs::create_dir_all(dir);
    dir.join(format!("{:016x}.img", h.finish()))
}

impl Images {
    pub fn new(ctx: egui::Context, http: Http) -> Images {
        let fetch: Fetch = Arc::new(move |u: &str| http.bytes(u));
        Self::start(ctx, Source { fetch, dir: crate::config::cache_dir().join("plex_img"), max_side: None }, 6)
    }

    /// Images from anywhere on the web (radio station logos): plain
    /// requests without Plex headers, scaled down to `max_side`.
    pub fn web(ctx: egui::Context, dir: &str, max_side: u32) -> Images {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(8))
            .timeout_read(std::time::Duration::from_secs(20))
            .user_agent(&format!("Velouna/{}", env!("CARGO_PKG_VERSION")))
            .build();
        let fetch: Fetch = Arc::new(move |u: &str| {
            let r = agent.get(u).call().map_err(|e| e.to_string())?;
            let mut buf = Vec::new();
            use std::io::Read;
            r.into_reader().take(8 * 1024 * 1024).read_to_end(&mut buf).map_err(|e| e.to_string())?;
            Ok(buf)
        });
        Self::start(ctx, Source { fetch, dir: crate::config::cache_dir().join(dir), max_side: Some(max_side) }, 4)
    }

    fn start(ctx: egui::Context, src: Source, workers: usize) -> Images {
        let queue: Arc<(Mutex<Vec<Job>>, Condvar)> = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let (tx, rx) = channel();
        let dir = src.dir.clone();
        for _ in 0..workers {
            let q = queue.clone();
            let tx = tx.clone();
            let src = src.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                loop {
                    let job = {
                        let (m, cv) = &*q;
                        let mut g = m.lock().unwrap();
                        while g.is_empty() {
                            g = cv.wait(g).unwrap();
                        }
                        g.pop().unwrap()
                    };
                    let res = load(&src, &job);
                    let _ = tx.send(match res {
                        Some((w, h, rgba)) => Done::Ok { key: job.key, w, h, rgba },
                        None => Done::Failed(job.key),
                    });
                    ctx.request_repaint();
                }
            });
        }
        Images { queue, rx, tx, entries: HashMap::new(), frame: 0, fading: false, dir }
    }

    /// The downloaded original of `key` on disk, once it has been loaded.
    #[allow(dead_code)]
    pub fn cached_file(&self, key: &str) -> Option<PathBuf> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        Some(self.dir.join(format!("{:016x}.img", h.finish()))).filter(|p| p.is_file())
    }

    /// Uploads finished images and evicts unused textures. Call once per frame.
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        if self.fading {
            ctx.request_repaint();
            self.fading = false;
        }
        while let Ok(d) = self.rx.try_recv() {
            match d {
                Done::Ok { key, w, h, rgba } => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    let tex = ctx.load_texture(format!("plex:{key}"), img, TextureOptions::LINEAR);
                    self.entries.insert(key, Img::Ready { tex, used: self.frame, born: Instant::now() });
                }
                Done::Failed(key) => {
                    self.entries.insert(key, Img::Failed);
                }
                Done::Dropped(key) => {
                    self.entries.remove(&key);
                }
            }
        }
        let ready = self.entries.values().filter(|e| matches!(e, Img::Ready { .. })).count();
        if ready > MAX_TEXTURES {
            let mut used: Vec<(u64, String)> = self
                .entries
                .iter()
                .filter_map(|(k, e)| match e {
                    Img::Ready { used, .. } if *used + 1 < self.frame => Some((*used, k.clone())),
                    _ => None,
                })
                .collect();
            used.sort();
            for (_, k) in used.into_iter().take(ready - MAX_TEXTURES * 4 / 5) {
                self.entries.remove(&k);
            }
        }
    }

    /// Returns the texture for `key`, queueing `url()` if it is not loaded yet.
    pub fn get(&mut self, key: &str, url: impl FnOnce() -> String) -> Option<Shown> {
        match self.entries.get_mut(key) {
            Some(Img::Ready { tex, used, born }) => {
                *used = self.frame;
                let alpha = (born.elapsed().as_secs_f32() / FADE_SECS).min(1.0);
                if alpha < 1.0 {
                    self.fading = true;
                }
                Some(Shown { id: tex.id(), size: tex.size_vec2(), alpha })
            }
            Some(_) => None,
            None => {
                self.entries.insert(key.to_string(), Img::Loading);
                let (m, cv) = &*self.queue;
                let mut g = m.lock().unwrap();
                g.push(Job { key: key.to_string(), url: url() });
                if g.len() > MAX_QUEUE {
                    let old = g.remove(0);
                    let _ = self.tx.send(Done::Dropped(old.key));
                }
                cv.notify_one();
                None
            }
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.queue.0.lock().unwrap().clear();
    }
}

fn load(src: &Source, job: &Job) -> Option<(usize, usize, Vec<u8>)> {
    let path = disk_path(&src.dir, &job.key);
    let bytes = match std::fs::read(&path) {
        Ok(b) if !b.is_empty() => {
            crate::cache::touch(&path);
            b
        }
        _ => {
            let b = (src.fetch)(&job.url).ok()?;
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, &b).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
            b
        }
    };
    let img = match image::load_from_memory(&bytes) {
        Ok(i) => i,
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            return None;
        }
    };
    let img = match src.max_side {
        Some(m) if img.width().max(img.height()) > m => img.thumbnail(m, m),
        _ => img,
    };
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    Some((w, h, rgba.into_raw()))
}
