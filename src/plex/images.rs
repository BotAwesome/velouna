//! Artwork loading for the Plex views.
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
}

fn disk_dir() -> PathBuf {
    let d = crate::config::cache_dir().join("plex_img");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn disk_path(key: &str) -> PathBuf {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    disk_dir().join(format!("{:016x}.img", h.finish()))
}

impl Images {
    pub fn new(ctx: egui::Context, http: Http) -> Images {
        let queue: Arc<(Mutex<Vec<Job>>, Condvar)> = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let (tx, rx) = channel();
        for _ in 0..6 {
            let q = queue.clone();
            let tx = tx.clone();
            let http = http.clone();
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
                    let res = load(&http, &job);
                    let _ = tx.send(match res {
                        Some((w, h, rgba)) => Done::Ok { key: job.key, w, h, rgba },
                        None => Done::Failed(job.key),
                    });
                    ctx.request_repaint();
                }
            });
        }
        Images { queue, rx, tx, entries: HashMap::new(), frame: 0, fading: false }
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

    pub fn failed(&self, key: &str) -> bool {
        matches!(self.entries.get(key), Some(Img::Failed))
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.queue.0.lock().unwrap().clear();
    }
}

fn load(http: &Http, job: &Job) -> Option<(usize, usize, Vec<u8>)> {
    let path = disk_path(&job.key);
    let bytes = match std::fs::read(&path) {
        Ok(b) if !b.is_empty() => b,
        _ => {
            let b = http.bytes(&job.url).ok()?;
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
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    Some((w, h, rgba.into_raw()))
}
