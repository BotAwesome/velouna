//! Offline copies of server items. Velouna fetches the same original file it
//! streams (Plex part URL, Jellyfin/Emby `stream?Static=true`), so neither
//! Plex Pass nor the servers' own download/sync features are involved.
//!
//! One file at a time, split into up to `parts` byte ranges that load in
//! parallel when the server allows ranges. Every range lives in its own
//! `.part` file and continues from its length after a pause, an error or a
//! restart; at the end the parts are joined into the real file.
//! Tokens are never stored: an unfinished download remembers the server and
//! the part path, and continues when that server is connected.

use super::{Conn, Item};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RUN: u8 = 0;
const PAUSE: u8 = 1;
const CANCEL: u8 = 2;
/// Smaller files load in one piece.
const MIN_SPLIT: u64 = if cfg!(test) { 1024 * 1024 } else { 16 * 1024 * 1024 };

/// A finished download.
#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub title: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub server_name: String,
    /// Unix seconds.
    #[serde(default)]
    pub done_at: i64,
}

/// An unfinished download: what is needed to continue it later.
#[derive(Clone, Serialize, Deserialize)]
pub struct Pending {
    pub key: String,
    pub server: String,
    pub server_name: String,
    pub part_key: String,
    pub name_hint: String,
    pub title: String,
    pub dir: PathBuf,
    pub parts: u32,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub done: u64,
    #[serde(default)]
    pub error: String,
    /// Paused by the user: stays paused. Otherwise (app closed, network gone)
    /// it continues by itself once its server is connected.
    #[serde(default)]
    pub user_paused: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    done: HashMap<String, Entry>,
    #[serde(default)]
    pending: Vec<Pending>,
}

struct Live {
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    speed: Arc<AtomicU64>,
    stop: Arc<AtomicU8>,
}

#[derive(Default)]
struct State {
    saved: Saved,
    /// Queued or running: key -> counters. Not in here = paused / stopped.
    live: HashMap<String, Live>,
    news: Vec<(String, Result<(), String>)>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum DlState {
    None,
    Queued,
    Running(f32),
    Paused(f32),
    Done,
}

/// One row of the download manager.
#[derive(Clone)]
pub struct Row {
    pub key: String,
    pub server: String,
    pub rating_key: String,
    pub title: String,
    pub server_name: String,
    pub state: DlState,
    pub done: u64,
    pub total: u64,
    /// Bytes per second while running.
    pub speed: u64,
    pub path: String,
    pub error: String,
    pub done_at: i64,
    pub user_paused: bool,
}

struct Job {
    key: String,
    url: String,
}

pub struct Downloads {
    st: Arc<Mutex<State>>,
    tx: Sender<Job>,
}

fn index_file() -> PathBuf {
    crate::config::data_dir().join("downloads.json")
}

fn save(s: &Saved) {
    if let Ok(j) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(index_file(), j);
    }
}

fn load() -> Saved {
    let Ok(text) = std::fs::read_to_string(index_file()) else { return Saved::default() };
    if let Ok(s) = serde_json::from_str::<Saved>(&text)
        && (!s.done.is_empty() || !s.pending.is_empty() || text.contains("\"pending\""))
    {
        return s;
    }
    // 0.6.0 kept only the finished map.
    Saved { done: serde_json::from_str(&text).unwrap_or_default(), pending: Vec::new() }
}

pub fn key(server: &str, rating_key: &str) -> String {
    format!("{server}|{rating_key}")
}

fn split_key(k: &str) -> (String, String) {
    let (a, b) = k.split_once('|').unwrap_or(("", k));
    (a.to_string(), b.to_string())
}

/// Downloads/Velouna, or inside the app data where there is no download folder.
pub fn default_dir() -> PathBuf {
    dirs::download_dir().map(|d| d.join("Velouna")).unwrap_or_else(|| crate::config::data_dir().join("Downloads"))
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
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

fn part_base(p: &Pending) -> PathBuf {
    p.dir.join(format!(".{}", p.key.replace(['|', '/', '\\', ':'], "_")))
}

fn remove_parts(p: &Pending) {
    let base = part_base(p);
    let _ = std::fs::remove_file(base.with_extension("part"));
    for i in 0..p.parts.max(1) {
        let _ = std::fs::remove_file(base.with_extension(format!("part{i}")));
    }
}

impl Downloads {
    pub fn new(ctx: &egui::Context) -> Downloads {
        let st = Arc::new(Mutex::new(State { saved: load(), ..Default::default() }));
        let (tx, rx) = channel();
        let (st2, ctx) = (st.clone(), ctx.clone());
        std::thread::Builder::new().name("downloads".into()).spawn(move || worker(rx, st2, ctx)).ok();
        Downloads { st, tx }
    }

    fn state_of(st: &State, k: &str) -> DlState {
        if let Some(l) = st.live.get(k) {
            let (d, t) = (l.done.load(Ordering::Relaxed), l.total.load(Ordering::Relaxed));
            return if t == 0 && d == 0 { DlState::Queued } else { DlState::Running(if t > 0 { d as f32 / t as f32 } else { 0.0 }) };
        }
        if let Some(p) = st.saved.pending.iter().find(|p| p.key == k) {
            return DlState::Paused(if p.total > 0 { p.done as f32 / p.total as f32 } else { 0.0 });
        }
        match st.saved.done.get(k) {
            Some(e) if Path::new(&e.path).exists() => DlState::Done,
            _ => DlState::None,
        }
    }

    pub fn state(&self, server: &str, rating_key: &str) -> DlState {
        Self::state_of(&self.st.lock().unwrap(), &key(server, rating_key))
    }

    /// Something to show in the manager (finished, paused or running).
    pub fn any(&self) -> bool {
        let st = self.st.lock().unwrap();
        !st.saved.done.is_empty() || !st.saved.pending.is_empty() || !st.live.is_empty()
    }

    pub fn running(&self) -> usize {
        self.st.lock().unwrap().live.len()
    }

    /// Local copy of an item, if one is complete on disk.
    pub fn local(&self, server: &str, rating_key: &str) -> Option<PathBuf> {
        let st = self.st.lock().unwrap();
        st.saved.done.get(&key(server, rating_key)).map(|e| PathBuf::from(&e.path)).filter(|p| p.exists())
    }

    /// All downloads: running and paused first, then finished (newest first).
    pub fn rows(&self) -> Vec<Row> {
        let st = self.st.lock().unwrap();
        let mut v: Vec<Row> = st
            .saved
            .pending
            .iter()
            .map(|p| {
                let (server, rating_key) = split_key(&p.key);
                let live = st.live.get(&p.key);
                let (done, total, speed) = match live {
                    Some(l) => (l.done.load(Ordering::Relaxed), l.total.load(Ordering::Relaxed), l.speed.load(Ordering::Relaxed)),
                    None => (p.done, p.total, 0),
                };
                Row {
                    key: p.key.clone(),
                    server,
                    rating_key,
                    title: p.title.clone(),
                    server_name: p.server_name.clone(),
                    state: Self::state_of(&st, &p.key),
                    done,
                    total,
                    speed,
                    path: String::new(),
                    error: if live.is_some() { String::new() } else { p.error.clone() },
                    done_at: 0,
                    user_paused: p.user_paused,
                }
            })
            .collect();
        let mut fin: Vec<Row> = st
            .saved
            .done
            .iter()
            .map(|(k, e)| {
                let (server, rating_key) = split_key(k);
                Row {
                    key: k.clone(),
                    server,
                    rating_key,
                    title: e.title.clone(),
                    server_name: e.server_name.clone(),
                    state: if Path::new(&e.path).exists() { DlState::Done } else { DlState::None },
                    done: e.size,
                    total: e.size,
                    speed: 0,
                    path: e.path.clone(),
                    error: String::new(),
                    done_at: e.done_at,
                    user_paused: false,
                }
            })
            .collect();
        fin.sort_by(|a, b| b.done_at.cmp(&a.done_at).then(a.title.cmp(&b.title)));
        v.extend(fin);
        v
    }

    fn enqueue(&self, st: &mut State, p: &Pending, conn: &Conn) {
        st.live.insert(
            p.key.clone(),
            Live {
                done: Arc::new(AtomicU64::new(p.done)),
                total: Arc::new(AtomicU64::new(p.total)),
                speed: Arc::new(AtomicU64::new(0)),
                stop: Arc::new(AtomicU8::new(RUN)),
            },
        );
        let _ = self.tx.send(Job { key: p.key.clone(), url: conn.with_token(&p.part_key) });
    }

    /// Queues an item; false when it has no file or is already there or queued.
    pub fn start(&self, server: &str, server_name: &str, item: &Item, conn: &Conn, dir: PathBuf, parts: u32) -> bool {
        let Some(part) = item.first_part() else { return false };
        let mut st = self.st.lock().unwrap();
        let k = key(server, &item.rating_key);
        match Self::state_of(&st, &k) {
            DlState::None => {}
            DlState::Paused(_) => {
                drop(st);
                return self.resume(&k, conn);
            }
            _ => return false,
        }
        let file = part.file.rsplit(['/', '\\']).next().unwrap_or("");
        let name_hint = if file.contains('.') { sanitize(file) } else { sanitize(&item.play_title()) };
        let p = Pending {
            key: k,
            server: server.into(),
            server_name: server_name.into(),
            part_key: part.key.clone(),
            name_hint,
            title: item.play_title(),
            dir,
            parts: parts.clamp(1, 8),
            total: 0,
            done: 0,
            error: String::new(),
            user_paused: false,
        };
        st.saved.pending.push(p.clone());
        save(&st.saved);
        self.enqueue(&mut st, &p, conn);
        true
    }

    /// Continues a paused download; `conn` must be its server's.
    pub fn resume(&self, k: &str, conn: &Conn) -> bool {
        let mut st = self.st.lock().unwrap();
        if st.live.contains_key(k) {
            return false;
        }
        let Some(p) = st.saved.pending.iter_mut().find(|p| p.key == k) else { return false };
        p.error.clear();
        p.user_paused = false;
        let p = p.clone();
        self.enqueue(&mut st, &p, conn);
        true
    }

    pub fn pause(&self, k: &str) {
        let mut st = self.st.lock().unwrap();
        if let Some(l) = st.live.get(k) {
            l.stop.store(PAUSE, Ordering::Relaxed);
        }
        if let Some(p) = st.saved.pending.iter_mut().find(|p| p.key == k) {
            p.user_paused = true;
        }
        save(&st.saved);
    }

    /// Removes a finished entry whose file is gone (deleted outside Velouna).
    pub fn forget(&self, k: &str) {
        let mut st = self.st.lock().unwrap();
        if st.saved.done.remove(k).is_some() {
            save(&st.saved);
        }
    }

    /// Stops and forgets an unfinished download, including its parts.
    pub fn cancel(&self, k: &str) {
        let mut st = self.st.lock().unwrap();
        if let Some(l) = st.live.get(k) {
            // The worker removes the parts once it lets go of them.
            l.stop.store(CANCEL, Ordering::Relaxed);
            return;
        }
        if let Some(i) = st.saved.pending.iter().position(|p| p.key == k) {
            let p = st.saved.pending.remove(i);
            remove_parts(&p);
            save(&st.saved);
        }
    }

    /// Deletes a finished download's file and entry.
    pub fn delete(&self, k: &str) {
        let mut st = self.st.lock().unwrap();
        if let Some(e) = st.saved.done.remove(k) {
            let _ = std::fs::remove_file(&e.path);
            save(&st.saved);
        }
    }

    /// Interrupted (not user-paused, no error) downloads of a server, to
    /// continue once it is connected.
    pub fn interrupted_of(&self, server: &str) -> Vec<String> {
        let st = self.st.lock().unwrap();
        st.saved.pending.iter().filter(|p| p.server == server && !p.user_paused && p.error.is_empty() && !st.live.contains_key(&p.key)).map(|p| p.key.clone()).collect()
    }

    pub fn take_news(&self) -> Vec<(String, Result<(), String>)> {
        std::mem::take(&mut self.st.lock().unwrap().news)
    }
}

enum Stop {
    Paused,
    Cancelled,
    Failed(String),
}

fn worker(rx: Receiver<Job>, st: Arc<Mutex<State>>, ctx: egui::Context) {
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout_read(Duration::from_secs(60)).build();
    for job in rx {
        let (p, live) = {
            let s = st.lock().unwrap();
            let Some(p) = s.saved.pending.iter().find(|p| p.key == job.key).cloned() else { continue };
            let Some(l) = s.live.get(&job.key) else { continue };
            (p, Live { done: l.done.clone(), total: l.total.clone(), speed: l.speed.clone(), stop: l.stop.clone() })
        };
        let res = match live.stop.load(Ordering::Relaxed) {
            PAUSE => Err(Stop::Paused),
            CANCEL => Err(Stop::Cancelled),
            _ => fetch(&agent, &job.url, &p, &live, &ctx),
        };
        let mut s = st.lock().unwrap();
        s.live.remove(&job.key);
        let idx = s.saved.pending.iter().position(|x| x.key == job.key);
        match res {
            Ok((path, size)) => {
                if let Some(i) = idx {
                    s.saved.pending.remove(i);
                }
                s.saved.done.insert(job.key.clone(), Entry { path: path.to_string_lossy().into(), title: p.title.clone(), size, server_name: p.server_name.clone(), done_at: now() });
                s.news.push((p.title, Ok(())));
            }
            Err(Stop::Cancelled) => {
                if let Some(i) = idx {
                    s.saved.pending.remove(i);
                }
                remove_parts(&p);
            }
            Err(stop) => {
                if let Some(x) = idx.map(|i| &mut s.saved.pending[i]) {
                    x.done = live.done.load(Ordering::Relaxed);
                    x.total = live.total.load(Ordering::Relaxed);
                    if let Stop::Failed(e) = &stop {
                        x.error = e.clone();
                    }
                }
                if let Stop::Failed(e) = stop {
                    s.news.push((p.title, Err(e)));
                }
            }
        }
        save(&s.saved);
        drop(s);
        ctx.request_repaint();
    }
}

fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, r) => format!("HTTP {code} {}", r.status_text()),
        ureq::Error::Transport(t) => t.to_string(),
    }
}

/// Size, range support and content type, from a one-byte range request.
fn probe(agent: &ureq::Agent, url: &str) -> Result<(u64, bool, String), String> {
    let r = agent.get(url).set("Range", "bytes=0-0").call().map_err(http_err)?;
    let ctype = r.header("Content-Type").unwrap_or("").to_string();
    if r.status() == 206 {
        let total = r.header("Content-Range").and_then(|c| c.rsplit('/').next()).and_then(|n| n.trim().parse().ok()).unwrap_or(0);
        return Ok((total, total > 0, ctype));
    }
    Ok((r.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0), false, ctype))
}

/// Loads bytes `start..end` (end exclusive, 0 = to the end) into `file`,
/// continuing from what the file already holds. Retries a few times.
fn fetch_range(agent: &ureq::Agent, url: &str, file: &Path, start: u64, end: u64, ranges: bool, live: &Live) -> Result<(), Stop> {
    let mut tries = 0;
    loop {
        let have = if ranges { std::fs::metadata(file).map(|m| m.len()).unwrap_or(0) } else { 0 };
        if end > 0 && start + have >= end {
            return Ok(());
        }
        let mut req = agent.get(url);
        if ranges {
            let last = if end > 0 { (end - 1).to_string() } else { String::new() };
            req = req.set("Range", &format!("bytes={}-{last}", start + have));
        }
        let res = req.call().map_err(http_err).and_then(|r| {
            let mut out = std::fs::OpenOptions::new().create(true).write(true).append(ranges).truncate(!ranges).open(file).map_err(|e| e.to_string())?;
            let mut body = r.into_reader();
            let mut buf = vec![0u8; 128 * 1024];
            loop {
                match live.stop.load(Ordering::Relaxed) {
                    PAUSE | CANCEL => return Ok(false),
                    _ => {}
                }
                let n = body.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    return Ok(true);
                }
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                live.done.fetch_add(n as u64, Ordering::Relaxed);
            }
        });
        match res {
            Ok(true) => {
                let have = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
                if end == 0 || start + have >= end {
                    return Ok(());
                }
            }
            Ok(false) => return Err(if live.stop.load(Ordering::Relaxed) == CANCEL { Stop::Cancelled } else { Stop::Paused }),
            Err(e) => {
                tries += 1;
                if tries >= 4 || !ranges {
                    return Err(Stop::Failed(e));
                }
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn fetch(agent: &ureq::Agent, url: &str, p: &Pending, live: &Live, ctx: &egui::Context) -> Result<(PathBuf, u64), Stop> {
    std::fs::create_dir_all(&p.dir).map_err(|e| Stop::Failed(e.to_string()))?;
    let (total, ranges, ctype) = probe(agent, url).map_err(Stop::Failed)?;
    live.total.store(total, Ordering::Relaxed);
    let base = part_base(p);
    let n = if ranges && total >= MIN_SPLIT { p.parts.max(1) as u64 } else { 1 };
    let files: Vec<PathBuf> = if n == 1 { vec![base.with_extension("part")] } else { (0..n).map(|i| base.with_extension(format!("part{i}"))).collect() };
    let bounds: Vec<(u64, u64)> = (0..n).map(|i| (total * i / n, if n == 1 && !ranges { 0 } else { total * (i + 1) / n })).collect();
    // Progress so far = what the part files hold.
    let have: u64 = if ranges { files.iter().map(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0)).sum() } else { 0 };
    live.done.store(have, Ordering::Relaxed);

    let result: Result<(), Stop> = std::thread::scope(|sc| {
        let handles: Vec<_> = files.iter().zip(&bounds).map(|(f, &(s, e))| sc.spawn(move || fetch_range(agent, url, f, s, e, ranges, live))).collect();
        // Speed over the last second while the parts load.
        let mut last = (Instant::now(), live.done.load(Ordering::Relaxed));
        while !handles.iter().all(|h| h.is_finished()) {
            std::thread::sleep(Duration::from_millis(250));
            let el = last.0.elapsed().as_secs_f64();
            if el >= 1.0 {
                let d = live.done.load(Ordering::Relaxed);
                live.speed.store(((d.saturating_sub(last.1)) as f64 / el) as u64, Ordering::Relaxed);
                last = (Instant::now(), d);
            }
            ctx.request_repaint();
        }
        let mut out = Ok(());
        for h in handles {
            match h.join().unwrap_or(Err(Stop::Failed("thread".into()))) {
                Ok(()) => {}
                // A failed part stops the others; the first real reason wins.
                Err(e) => {
                    if matches!(out, Ok(())) || matches!(e, Stop::Failed(_)) && !matches!(out, Err(Stop::Failed(_))) {
                        if matches!(e, Stop::Failed(_)) {
                            live.stop.store(PAUSE, Ordering::Relaxed);
                        }
                        out = Err(e);
                    }
                }
            }
        }
        out
    });
    result?;
    let size: u64 = files.iter().map(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0)).sum();
    if total > 0 && size != total {
        return Err(Stop::Failed(t!("Verbindung abgebrochen").into()));
    }
    let mut name = p.name_hint.clone();
    if Path::new(&name).extension().is_none_or(|e| e.len() > 5) {
        let ext = ext_for(&ctype);
        name = format!("{name}.{}", if ext.is_empty() { "mkv" } else { ext });
    }
    let dest = unique(p.dir.join(name));
    if files.len() == 1 {
        std::fs::rename(&files[0], &dest).map_err(|e| Stop::Failed(e.to_string()))?;
    } else {
        let join = || -> std::io::Result<()> {
            let tmp = base.with_extension("joining");
            let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
            for f in &files {
                std::io::copy(&mut std::fs::File::open(f)?, &mut out)?;
            }
            out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
            std::fs::rename(&tmp, &dest)?;
            for f in &files {
                let _ = std::fs::remove_file(f);
            }
            Ok(())
        };
        join().map_err(|e| Stop::Failed(e.to_string()))?;
    }
    Ok((dest, size))
}
