//! Offline copies of server items. Velouna fetches the same original file it
//! streams (Plex part URL, Jellyfin/Emby `stream?Static=true`), so neither
//! Plex Pass nor the servers' own download/sync features are involved.
//! One worker downloads one file at a time; a half-finished `.part` file is
//! continued with an HTTP range request.

use super::{Conn, Item};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use eframe::egui;
use std::time::{Duration, Instant};

#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub title: String,
}

struct Active {
    done: u64,
    total: u64,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    /// "server|rating_key" -> finished file
    index: HashMap<String, Entry>,
    active: HashMap<String, Active>,
    /// Finished or failed downloads the UI has not announced yet.
    news: Vec<(String, Result<(), String>)>,
}

pub enum DlState {
    None,
    Queued,
    Running(f32),
    Done,
}

struct Job {
    key: String,
    url: String,
    name_hint: String,
    title: String,
    dir: PathBuf,
    cancel: Arc<AtomicBool>,
}

pub struct Downloads {
    st: Arc<Mutex<State>>,
    tx: Sender<Job>,
}

fn index_file() -> PathBuf {
    crate::config::data_dir().join("downloads.json")
}

fn save(index: &HashMap<String, Entry>) {
    if let Ok(s) = serde_json::to_string_pretty(index) {
        let _ = std::fs::write(index_file(), s);
    }
}

pub fn key(server: &str, rating_key: &str) -> String {
    format!("{server}|{rating_key}")
}

/// Default folder: Downloads/Velouna, or inside the app data where the
/// system has no download folder (Android).
pub fn default_dir() -> PathBuf {
    dirs::download_dir().map(|d| d.join("Velouna")).unwrap_or_else(|| crate::config::data_dir().join("Downloads"))
}

fn sanitize(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() { "Download".into() } else { s.chars().take(150).collect() }
}

fn ext_for(content_type: &str) -> &'static str {
    match content_type.split(';').next().unwrap_or("").trim() {
        "video/mp4" => "mp4",
        "video/x-matroska" | "video/webm" => "mkv",
        "video/x-msvideo" => "avi",
        "video/mp2t" => "ts",
        "audio/mpeg" => "mp3",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/mp4" | "audio/aac" => "m4a",
        "audio/ogg" => "ogg",
        "audio/wav" | "audio/x-wav" => "wav",
        _ => "",
    }
}

fn unique(p: PathBuf) -> PathBuf {
    if !p.exists() {
        return p;
    }
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Download").to_string();
    let ext = p.extension().and_then(|s| s.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    (2..).map(|n| p.with_file_name(format!("{stem} ({n}){ext}"))).find(|c| !c.exists()).unwrap()
}

impl Downloads {
    pub fn new(ctx: &egui::Context) -> Downloads {
        let index: HashMap<String, Entry> = std::fs::read_to_string(index_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let st = Arc::new(Mutex::new(State { index, ..Default::default() }));
        let (tx, rx) = channel();
        let (st2, ctx) = (st.clone(), ctx.clone());
        std::thread::Builder::new().name("downloads".into()).spawn(move || worker(rx, st2, ctx)).ok();
        Downloads { st, tx }
    }

    pub fn state(&self, server: &str, rating_key: &str) -> DlState {
        let k = key(server, rating_key);
        let st = self.st.lock().unwrap();
        if let Some(a) = st.active.get(&k) {
            return if a.total == 0 && a.done == 0 { DlState::Queued } else { DlState::Running(if a.total > 0 { a.done as f32 / a.total as f32 } else { 0.0 }) };
        }
        match st.index.get(&k) {
            Some(e) if Path::new(&e.path).exists() => DlState::Done,
            _ => DlState::None,
        }
    }

    /// Local copy of an item, if one is complete on disk.
    pub fn local(&self, server: &str, rating_key: &str) -> Option<PathBuf> {
        let st = self.st.lock().unwrap();
        st.index.get(&key(server, rating_key)).map(|e| PathBuf::from(&e.path)).filter(|p| p.exists())
    }

    /// Queues an item; false when it has no file or is already there or queued.
    pub fn start(&self, server: &str, item: &Item, conn: &Conn, dir: PathBuf) -> bool {
        let Some(part) = item.first_part() else { return false };
        if !matches!(self.state(server, &item.rating_key), DlState::None) {
            return false;
        }
        let k = key(server, &item.rating_key);
        let cancel = Arc::new(AtomicBool::new(false));
        self.st.lock().unwrap().active.insert(k.clone(), Active { done: 0, total: 0, cancel: cancel.clone() });
        let server_name = part.file.rsplit(['/', '\\']).next().unwrap_or("");
        let name_hint = if server_name.contains('.') { sanitize(server_name) } else { sanitize(&item.play_title()) };
        let _ = self.tx.send(Job { key: k, url: conn.with_token(&part.key), name_hint, title: item.play_title(), dir, cancel });
        true
    }

    pub fn cancel(&self, server: &str, rating_key: &str) {
        if let Some(a) = self.st.lock().unwrap().active.get(&key(server, rating_key)) {
            a.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Deletes the local copy (the file itself, not only the entry).
    pub fn delete(&self, server: &str, rating_key: &str) {
        let mut st = self.st.lock().unwrap();
        if let Some(e) = st.index.remove(&key(server, rating_key)) {
            let _ = std::fs::remove_file(&e.path);
            save(&st.index);
        }
    }

    /// Titles of downloads that ended since the last call: Ok or the error.
    pub fn take_news(&self) -> Vec<(String, Result<(), String>)> {
        std::mem::take(&mut self.st.lock().unwrap().news)
    }
}

fn worker(rx: Receiver<Job>, st: Arc<Mutex<State>>, ctx: egui::Context) {
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout_read(Duration::from_secs(60)).build();
    for job in rx {
        let res = if job.cancel.load(Ordering::Relaxed) { Err(String::new()) } else { fetch(&agent, &job, &st, &ctx) };
        let mut s = st.lock().unwrap();
        s.active.remove(&job.key);
        match res {
            Ok(path) => {
                s.index.insert(job.key.clone(), Entry { path: path.to_string_lossy().into(), title: job.title.clone() });
                save(&s.index);
                s.news.push((job.title, Ok(())));
            }
            // Cancelled: no message.
            Err(e) if e.is_empty() => {}
            Err(e) => s.news.push((job.title, Err(e))),
        }
        drop(s);
        ctx.request_repaint();
    }
}

fn fetch(agent: &ureq::Agent, job: &Job, st: &Mutex<State>, ctx: &egui::Context) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&job.dir).map_err(|e| e.to_string())?;
    // Partial file named after the job, so a restart continues it.
    let tmp = job.dir.join(format!(".{}.part", job.key.replace(['|', '/', '\\', ':'], "_")));
    let have = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    let mut req = agent.get(&job.url);
    if have > 0 {
        req = req.set("Range", &format!("bytes={have}-"));
    }
    let resp = req.call().map_err(|e| match e {
        ureq::Error::Status(code, r) => format!("HTTP {code} {}", r.status_text()),
        ureq::Error::Transport(t) => t.to_string(),
    })?;
    let resumed = resp.status() == 206;
    let start = if resumed { have } else { 0 };
    let len: u64 = resp.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0);
    let ext = ext_for(resp.header("Content-Type").unwrap_or(""));
    let mut name = job.name_hint.clone();
    if !name.contains('.') || Path::new(&name).extension().is_some_and(|e| e.len() > 5) {
        name = format!("{name}.{}", if ext.is_empty() { "mkv" } else { ext });
    }
    let mut out = std::fs::OpenOptions::new().create(true).write(true).append(resumed).truncate(!resumed).open(&tmp).map_err(|e| e.to_string())?;
    let total = if len > 0 { start + len } else { 0 };
    let mut done = start;
    let mut body = resp.into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut last = Instant::now();
    loop {
        if job.cancel.load(Ordering::Relaxed) {
            drop(out);
            let _ = std::fs::remove_file(&tmp);
            return Err(String::new());
        }
        let n = body.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if last.elapsed() > Duration::from_millis(250) {
            last = Instant::now();
            if let Some(a) = st.lock().unwrap().active.get_mut(&job.key) {
                a.done = done;
                a.total = total.max(1);
            }
            ctx.request_repaint();
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    drop(out);
    if total > 0 && done < total {
        return Err(t!("Verbindung abgebrochen").into());
    }
    let dest = unique(job.dir.join(name));
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}
