//! Plex integration: sign-in, server connection, cached API data, artwork,
//! and play-state reporting. The UI lives in `ui_plex.rs`.

pub mod api;
pub mod images;
pub mod model;

pub use model::{Hub, Item, Kind, Section, Server, User};

use crate::config::PlexConfig;
use api::Http;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

pub const PAGE: usize = 60;
const KEYRING_SERVICE: &str = "Velour";
const LEGACY_KEYRING_SERVICE: &str = "VLB";
const KEYRING_USER: &str = "plex-token";

// ------------------------------------------------------------ registry

/// What Velour knows about a Plex stream that sits in mpv's playlist.
#[derive(Clone, Debug)]
pub struct Remote {
    pub rating_key: String,
    pub title: String,
    pub duration: f64,
    pub base: String,
    pub token: String,
    pub kind: Kind,
}

static REMOTES: LazyLock<Mutex<HashMap<String, Remote>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn register(url: &str, r: Remote) {
    REMOTES.lock().unwrap().insert(url.to_string(), r);
}
pub fn remote(url: &str) -> Option<Remote> {
    REMOTES.lock().unwrap().get(url).cloned()
}
pub fn is_remote(url: &str) -> bool {
    REMOTES.lock().unwrap().contains_key(url)
}
pub fn remote_title(url: &str) -> Option<String> {
    REMOTES.lock().unwrap().get(url).map(|r| r.title.clone())
}
/// URL usable by ffmpeg (which cannot take mpv's per-file header option).
pub fn authorized(url: &str) -> String {
    match remote(url) {
        Some(r) => format!("{url}{}X-Plex-Token={}", if url.contains('?') { '&' } else { '?' }, api::url_encode(&r.token)),
        None => url.to_string(),
    }
}

// ------------------------------------------------------------- tokens

use crate::platform::secret;

fn load_token() -> Option<String> {
    if let Some(t) = secret::get(KEYRING_SERVICE, KEYRING_USER) {
        return Some(t);
    }
    // Sign-ins from before the rename live under the working title; move them over.
    let t = secret::get(LEGACY_KEYRING_SERVICE, KEYRING_USER)?;
    if store_token(&t) {
        secret::delete(LEGACY_KEYRING_SERVICE, KEYRING_USER);
    }
    Some(t)
}
fn store_token(t: &str) -> bool {
    secret::set(KEYRING_SERVICE, KEYRING_USER, t)
}
fn delete_token() {
    secret::delete(KEYRING_SERVICE, KEYRING_USER);
}

fn random_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

// --------------------------------------------------------------- state

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    SignedOut,
    /// Waiting for the user to confirm the code on plex.tv.
    Pin { id: i64, code: String, url: String },
    Connecting(String),
    Ready,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct ActiveServer {
    pub id: String,
    pub name: String,
    pub base: String,
    pub token: String,
    pub owned: bool,
}

/// Loading state handed to the UI.
pub enum St<T> {
    Loading,
    Ready(T),
    Failed(String),
}

#[derive(Clone)]
enum Loaded {
    Items(Arc<Vec<Item>>, i64),
    Hubs(Arc<Vec<Hub>>),
    Meta(Arc<Item>),
    Chars(Arc<Vec<(String, i64)>>),
    Values(Arc<Vec<(String, String)>>),
    Err(String),
}

#[derive(Clone, Copy)]
enum FetchKind {
    Items,
    Hubs,
    Meta,
    Chars,
    Values,
}

enum Job {
    CreatePin,
    CheckPin(i64),
    Account { token: String },
    Connect { server: Server, last: Option<String>, local_only: bool },
    Fetch { key: String, base: String, token: String, path: String, kind: FetchKind },
    Page { list: String, page: usize, base: String, token: String, path: String },
    Send { base: String, token: String, method: &'static str, path: String },
}

enum Res {
    Pin(Result<(i64, String), String>),
    PinToken(i64, Result<Option<String>, String>),
    Account(Result<(User, Vec<Server>), String>),
    Connected(String, Result<(String, Vec<Section>), String>),
    Fetched(String, Loaded),
    Page(String, usize, Result<(Vec<Item>, i64), String>),
}

#[derive(Default)]
struct PagedList {
    total: Option<i64>,
    pages: HashMap<usize, Arc<Vec<Item>>>,
    pending: HashSet<usize>,
    error: Option<String>,
}

/// Something to start playing once its list has loaded.
#[derive(Clone, Debug)]
pub struct PlayReq {
    /// Path of a list (`/library/metadata/1/allLeaves`, `/library/metadata/2` …).
    pub list: String,
    pub start_key: Option<String>,
    /// Some(t): start at t seconds without asking; None: follow the resume setting.
    pub start_at: Option<f64>,
    pub shuffle: bool,
    pub mode: PlayMode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayMode {
    Play,
    Next,
    Enqueue,
}

struct Session {
    url: String,
    rating_key: String,
    duration_ms: i64,
    last_sent: Instant,
    last_state: &'static str,
    last_time_ms: i64,
    scrobbled: bool,
}

pub struct Plex {
    pub http: Http,
    pub status: Status,
    pub user: Option<User>,
    pub servers: Vec<Server>,
    pub server: Option<ActiveServer>,
    pub sections: Vec<Section>,
    pub images: images::Images,
    pub pending_play: Option<PlayReq>,
    pub token_store_failed: bool,
    token: Option<String>,
    jobs: Sender<Job>,
    results: Receiver<Res>,
    cache: HashMap<String, Loaded>,
    pending: HashSet<String>,
    lists: HashMap<String, PagedList>,
    pin_check_at: Option<Instant>,
    pin_checking: bool,
    session: Option<Session>,
    /// Bumped whenever cached data changes (lets views drop derived state).
    pub generation: u64,
}

impl Plex {
    pub fn new(ctx: &egui::Context, cfg: &mut PlexConfig) -> (Plex, bool) {
        let mut dirty = false;
        if cfg.client_id.is_empty() {
            cfg.client_id = random_id();
            dirty = true;
        }
        let http = Http::new(cfg.client_id.clone());
        let (jtx, jrx) = channel::<Job>();
        let (rtx, rrx) = channel::<Res>();
        let jrx = Arc::new(Mutex::new(jrx));
        for _ in 0..4 {
            let jrx = jrx.clone();
            let rtx = rtx.clone();
            let http = http.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                loop {
                    let job = match jrx.lock().unwrap().recv() {
                        Ok(j) => j,
                        Err(_) => return,
                    };
                    if let Some(r) = run_job(&http, job) {
                        let _ = rtx.send(r);
                        ctx.request_repaint();
                    }
                }
            });
        }
        let images = images::Images::new(ctx.clone(), http.clone());
        let token = load_token();
        let mut p = Plex {
            http,
            status: Status::SignedOut,
            user: None,
            servers: vec![],
            server: None,
            sections: vec![],
            images,
            pending_play: None,
            token_store_failed: false,
            token: None,
            jobs: jtx,
            results: rrx,
            cache: HashMap::new(),
            pending: HashSet::new(),
            lists: HashMap::new(),
            pin_check_at: None,
            pin_checking: false,
            session: None,
            generation: 0,
        };
        if let Some(t) = token {
            p.token = Some(t.clone());
            p.status = Status::Connecting(if cfg.server_name.is_empty() { t!("Anmelden…").into() } else { tf!("Verbinde mit {}…", cfg.server_name) });
            p.user = Some(User { name: cfg.user_name.clone(), thumb: String::new() });
            let _ = p.jobs.send(Job::Account { token: t });
        }
        (p, dirty)
    }

    pub fn signed_in(&self) -> bool {
        self.token.is_some()
    }
    pub fn ready(&self) -> bool {
        self.status == Status::Ready && self.server.is_some()
    }

    // ------------------------------------------------------------ login

    pub fn start_login(&mut self) {
        self.status = Status::Connecting(t!("Code wird angefordert…").into());
        let _ = self.jobs.send(Job::CreatePin);
    }

    pub fn cancel_login(&mut self) {
        self.status = Status::SignedOut;
        self.pin_check_at = None;
    }

    pub fn sign_out(&mut self, cfg: &mut PlexConfig) {
        delete_token();
        self.token = None;
        self.user = None;
        self.servers.clear();
        self.server = None;
        self.sections.clear();
        self.clear_cache();
        self.images.clear();
        self.status = Status::SignedOut;
        cfg.server_id = None;
        cfg.server_name.clear();
        cfg.last_uri = None;
        cfg.user_name.clear();
    }

    pub fn select_server(&mut self, id: &str, local_only: bool) {
        let Some(s) = self.servers.iter().find(|s| s.id == id).cloned() else { return };
        self.status = Status::Connecting(tf!("Verbinde mit {}…", s.name));
        self.server = None;
        self.sections.clear();
        self.clear_cache();
        let _ = self.jobs.send(Job::Connect { server: s, last: None, local_only });
    }

    pub fn retry(&mut self, cfg: &PlexConfig, local_only: bool) {
        if let Some(t) = self.token.clone() {
            if self.servers.is_empty() {
                self.status = Status::Connecting(t!("Anmelden…").into());
                let _ = self.jobs.send(Job::Account { token: t });
            } else if let Some(id) = cfg.server_id.clone().or_else(|| self.servers.first().map(|s| s.id.clone())) {
                self.select_server(&id, local_only);
            }
        }
    }

    // ------------------------------------------------------------- poll

    /// Processes worker results. Returns true when `cfg` changed.
    pub fn poll(&mut self, ctx: &egui::Context, cfg: &mut PlexConfig, local_only: bool) -> bool {
        let mut dirty = false;
        self.images.begin_frame(ctx);
        while let Ok(r) = self.results.try_recv() {
            match r {
                Res::Pin(Ok((id, code))) => {
                    let url = self.http.auth_url(&code);
                    open_browser(&url);
                    self.status = Status::Pin { id, code, url };
                    self.pin_check_at = Some(Instant::now() + Duration::from_secs(2));
                }
                Res::Pin(Err(e)) => self.status = Status::Error(tf!("Anmeldung nicht möglich: {e}")),
                Res::PinToken(id, r) => {
                    self.pin_checking = false;
                    let current = matches!(&self.status, Status::Pin { id: i, .. } if *i == id);
                    if !current {
                        continue;
                    }
                    match r {
                        Ok(Some(token)) => {
                            self.token_store_failed = !store_token(&token);
                            self.token = Some(token.clone());
                            self.pin_check_at = None;
                            self.status = Status::Connecting(t!("Anmelden…").into());
                            let _ = self.jobs.send(Job::Account { token });
                        }
                        Ok(None) => self.pin_check_at = Some(Instant::now() + Duration::from_secs(2)),
                        Err(_) => self.pin_check_at = Some(Instant::now() + Duration::from_secs(4)),
                    }
                }
                Res::Account(Ok((user, servers))) => {
                    if !user.name.is_empty() && cfg.user_name != user.name {
                        cfg.user_name = user.name.clone();
                        dirty = true;
                    }
                    self.user = Some(user);
                    self.servers = servers;
                    let pick = cfg
                        .server_id
                        .as_ref()
                        .and_then(|id| self.servers.iter().find(|s| &s.id == id))
                        .or_else(|| self.servers.iter().find(|s| s.owned && s.presence))
                        .or_else(|| self.servers.iter().find(|s| s.presence))
                        .or_else(|| self.servers.first())
                        .cloned();
                    match pick {
                        Some(s) => {
                            let last = if cfg.server_id.as_deref() == Some(&s.id) { cfg.last_uri.clone() } else { None };
                            self.status = Status::Connecting(tf!("Verbinde mit {}…", s.name));
                            let _ = self.jobs.send(Job::Connect { server: s, last, local_only });
                        }
                        None => self.status = Status::Error(t!("Mit diesem Konto ist kein Plex Media Server verknüpft.").into()),
                    }
                }
                Res::Account(Err(e)) => {
                    if api::is_unauthorized(&e) {
                        delete_token();
                        self.token = None;
                        self.status = Status::Error(t!("Die Plex-Anmeldung ist abgelaufen. Bitte erneut verbinden.").into());
                    } else {
                        self.status = Status::Error(tf!("plex.tv nicht erreichbar: {e}"));
                    }
                }
                Res::Connected(id, Ok((base, sections))) => {
                    let Some(s) = self.servers.iter().find(|s| s.id == id).cloned() else { continue };
                    if cfg.server_id.as_deref() != Some(&s.id) || cfg.last_uri.as_deref() != Some(&base) || cfg.server_name != s.name {
                        cfg.server_id = Some(s.id.clone());
                        cfg.server_name = s.name.clone();
                        cfg.last_uri = Some(base.clone());
                        dirty = true;
                    }
                    self.server = Some(ActiveServer { id: s.id, name: s.name, base, token: s.token, owned: s.owned });
                    self.sections = sections;
                    self.status = Status::Ready;
                    self.generation += 1;
                }
                Res::Connected(_, Err(e)) => self.status = Status::Error(e),
                Res::Fetched(key, l) => {
                    self.pending.remove(&key);
                    self.cache.insert(key, l);
                    self.generation += 1;
                }
                Res::Page(list, page, r) => {
                    let l = self.lists.entry(list).or_default();
                    l.pending.remove(&page);
                    match r {
                        Ok((items, total)) => {
                            l.total = Some(total);
                            l.pages.insert(page, Arc::new(items));
                            l.error = None;
                        }
                        Err(e) => l.error = Some(e),
                    }
                    self.generation += 1;
                }
            }
        }
        if let (Status::Pin { id, .. }, Some(at)) = (&self.status, self.pin_check_at) {
            if Instant::now() >= at && !self.pin_checking {
                self.pin_checking = true;
                self.pin_check_at = None;
                let _ = self.jobs.send(Job::CheckPin(*id));
            } else {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
        dirty
    }

    // ------------------------------------------------------------ cache

    pub fn clear_cache(&mut self) {
        self.cache.clear();
        self.pending.clear();
        self.lists.clear();
        self.generation += 1;
    }

    /// Drops cached entries whose request path contains `needle`.
    pub fn invalidate(&mut self, needle: &str) {
        self.cache.retain(|k, _| !k.contains(needle));
        self.lists.retain(|k, _| !k.contains(needle));
        self.generation += 1;
    }

    /// Drops everything that shows play state (hubs, on deck …), keeping big lists.
    pub fn invalidate_play_state(&mut self) {
        self.cache.retain(|k, _| !(k.starts_with("H|") || k.contains("/children") || k.contains("allLeaves") || k.starts_with("M|")));
        self.generation += 1;
    }

    fn fetch(&mut self, kind: FetchKind, prefix: &str, path: &str) -> Option<Loaded> {
        let key = format!("{prefix}|{path}");
        if let Some(l) = self.cache.get(&key) {
            return Some(l.clone());
        }
        if !self.pending.contains(&key) {
            if let Some(s) = &self.server {
                self.pending.insert(key.clone());
                let _ = self.jobs.send(Job::Fetch { key, base: s.base.clone(), token: s.token.clone(), path: path.to_string(), kind });
            }
        }
        None
    }

    pub fn hubs(&mut self, path: &str) -> St<Arc<Vec<Hub>>> {
        match self.fetch(FetchKind::Hubs, "H", path) {
            Some(Loaded::Hubs(h)) => St::Ready(h),
            Some(Loaded::Err(e)) => St::Failed(e),
            _ => St::Loading,
        }
    }
    pub fn items(&mut self, path: &str) -> St<Arc<Vec<Item>>> {
        match self.fetch(FetchKind::Items, "I", path) {
            Some(Loaded::Items(i, _)) => St::Ready(i),
            Some(Loaded::Err(e)) => St::Failed(e),
            _ => St::Loading,
        }
    }
    pub fn meta(&mut self, rating_key: &str) -> St<Arc<Item>> {
        let path = format!("/library/metadata/{rating_key}?includeOnDeck=1&includeChapters=1&includeMarkers=1&includeExtras=0");
        match self.fetch(FetchKind::Meta, "M", &path) {
            Some(Loaded::Meta(i)) => St::Ready(i),
            Some(Loaded::Err(e)) => St::Failed(e),
            _ => St::Loading,
        }
    }
    pub fn chars(&mut self, path: &str) -> St<Arc<Vec<(String, i64)>>> {
        match self.fetch(FetchKind::Chars, "C", path) {
            Some(Loaded::Chars(c)) => St::Ready(c),
            Some(Loaded::Err(e)) => St::Failed(e),
            _ => St::Loading,
        }
    }
    pub fn values(&mut self, path: &str) -> St<Arc<Vec<(String, String)>>> {
        match self.fetch(FetchKind::Values, "V", path) {
            Some(Loaded::Values(c)) => St::Ready(c),
            Some(Loaded::Err(e)) => St::Failed(e),
            _ => St::Loading,
        }
    }

    /// Total size of a paged list (loads the first page if needed).
    pub fn paged_total(&mut self, path: &str) -> St<i64> {
        self.ensure_page(path, 0);
        let l = self.lists.get(path);
        match l {
            Some(PagedList { total: Some(t), .. }) => St::Ready(*t),
            Some(PagedList { error: Some(e), .. }) => St::Failed(e.clone()),
            _ => St::Loading,
        }
    }
    /// Item `idx` of a paged list; queues its page when missing.
    pub fn paged_item(&mut self, path: &str, idx: usize) -> Option<Item> {
        let page = idx / PAGE;
        self.ensure_page(path, page);
        self.lists.get(path).and_then(|l| l.pages.get(&page)).and_then(|p| p.get(idx % PAGE)).cloned()
    }
    fn ensure_page(&mut self, path: &str, page: usize) {
        let Some(s) = &self.server else { return };
        let l = self.lists.entry(path.to_string()).or_default();
        if l.pages.contains_key(&page) || l.pending.contains(&page) || (l.error.is_some() && page == 0) {
            return;
        }
        if let Some(t) = l.total {
            if page * PAGE >= t as usize && page > 0 {
                return;
            }
        }
        l.pending.insert(page);
        let _ = self.jobs.send(Job::Page { list: path.to_string(), page, base: s.base.clone(), token: s.token.clone(), path: path.to_string() });
    }
    pub fn retry_list(&mut self, path: &str) {
        self.lists.remove(path);
    }

    /// Applies a change to every cached copy of an item (progress, watched …).
    pub fn patch(&mut self, rating_key: &str, f: impl Fn(&mut Item)) {
        let fix = |v: &mut Arc<Vec<Item>>| {
            if v.iter().any(|i| i.rating_key == rating_key) {
                for i in Arc::make_mut(v).iter_mut().filter(|i| i.rating_key == rating_key) {
                    f(i);
                }
            }
        };
        for l in self.cache.values_mut() {
            match l {
                Loaded::Items(v, _) => fix(v),
                Loaded::Hubs(h) => {
                    if h.iter().any(|h| h.items.iter().any(|i| i.rating_key == rating_key)) {
                        for hub in Arc::make_mut(h).iter_mut() {
                            for i in hub.items.iter_mut().filter(|i| i.rating_key == rating_key) {
                                f(i);
                            }
                        }
                    }
                }
                Loaded::Meta(m) => {
                    if m.rating_key == rating_key {
                        f(Arc::make_mut(m));
                    }
                }
                _ => {}
            }
        }
        for l in self.lists.values_mut() {
            for p in l.pages.values_mut() {
                fix(p);
            }
        }
        self.generation += 1;
    }

    // ------------------------------------------------------- artwork

    /// Transcoded artwork URL for a Plex image path (or an absolute URL).
    pub fn image_url(&self, path: &str, w: u32, h: u32) -> Option<String> {
        let s = self.server.as_ref()?;
        Some(format!(
            "{}/photo/:/transcode?width={w}&height={h}&minSize=1&upscale=1&url={}&X-Plex-Token={}",
            s.base,
            api::url_encode(path),
            api::url_encode(&s.token)
        ))
    }

    /// Texture for an image, sized in buckets so similar cards share cache entries.
    pub fn image(&mut self, path: &str, w: f32, h: f32) -> Option<images::Shown> {
        if path.is_empty() {
            return None;
        }
        let server = self.server.as_ref()?.id.clone();
        let bucket = |x: f32| ((x.max(1.0) / 80.0).ceil() * 80.0).min(1920.0) as u32;
        let (bw, bh) = (bucket(w), bucket(h));
        let key = format!("{server}|{path}|{bw}x{bh}");
        let url = self.image_url(path, bw, bh)?;
        self.images.get(&key, move || url)
    }

    // ------------------------------------------------------- playback

    pub fn stream_url(&self, part_key: &str) -> Option<String> {
        let s = self.server.as_ref()?;
        Some(format!("{}{}", s.base, part_key))
    }
    pub fn token_for_playback(&self) -> Option<String> {
        self.server.as_ref().map(|s| s.token.clone())
    }

    fn send(&self, method: &'static str, path: String) {
        if let Some(s) = &self.server {
            let _ = self.jobs.send(Job::Send { base: s.base.clone(), token: s.token.clone(), method, path });
        }
    }

    pub fn set_watched(&mut self, item: &Item, watched: bool) {
        let p = if watched { "/:/scrobble" } else { "/:/unscrobble" };
        self.send("GET", format!("{p}?identifier=com.plexapp.plugins.library&key={}", item.rating_key));
        let kind = item.kind;
        self.patch(&item.rating_key, |i| {
            i.view_offset_ms = 0;
            if matches!(kind, Kind::Show | Kind::Season) {
                i.viewed_leaf_count = if watched { i.leaf_count } else { 0 };
            } else {
                i.view_count = if watched { i.view_count.max(1) } else { 0 };
            }
        });
        // Parents/children change too: refetch what shows aggregated state.
        self.invalidate_play_state();
    }

    /// Reports the player state for the current Plex stream (call every frame;
    /// throttled internally). `path` is mpv's current path.
    pub fn report(&mut self, path: &str, time: f64, duration: f64, paused: bool, active: bool, sync: bool) {
        let remote = if active { remote(path) } else { None };
        // Stream changed or stopped: close the previous session.
        if let Some(s) = &self.session {
            if remote.is_none() || s.url != path {
                let s = self.session.take().unwrap();
                if sync {
                    self.timeline(&s.rating_key, "stopped", s.last_time_ms, s.duration_ms);
                }
                self.finish_progress(&s.rating_key, s.last_time_ms, s.duration_ms);
            }
        }
        let Some(r) = remote else { return };
        let time_ms = (time * 1000.0) as i64;
        let dur_ms = if duration > 0.0 { (duration * 1000.0) as i64 } else { (r.duration * 1000.0) as i64 };
        let state = if paused { "paused" } else { "playing" };
        let s = self.session.get_or_insert_with(|| Session {
            url: path.to_string(),
            rating_key: r.rating_key.clone(),
            duration_ms: dur_ms,
            last_sent: Instant::now() - Duration::from_secs(60),
            last_state: "",
            last_time_ms: 0,
            scrobbled: false,
        });
        s.duration_ms = dur_ms;
        s.last_time_ms = time_ms;
        let due = s.last_state != state || s.last_sent.elapsed() > Duration::from_secs(10);
        let scrobble = !s.scrobbled && dur_ms > 0 && time_ms as f64 / dur_ms as f64 > 0.9;
        if scrobble {
            s.scrobbled = true;
        }
        if due {
            s.last_sent = Instant::now();
            s.last_state = state;
        }
        let rk = r.rating_key.clone();
        if sync && due {
            self.timeline(&rk, state, time_ms, dur_ms);
        }
        if sync && scrobble {
            self.send("GET", format!("/:/scrobble?identifier=com.plexapp.plugins.library&key={rk}"));
            // Keep menus honest ("Als ungesehen markieren") without refetching mid-playback.
            self.patch(&rk, |i| i.view_count = i.view_count.max(1));
        }
        if due {
            self.patch(&rk, |i| i.view_offset_ms = time_ms);
        }
    }

    /// Stops the current session (app exit). Blocking with a short timeout.
    pub fn stop_now(&mut self, sync: bool) {
        if let (Some(s), Some(srv)) = (self.session.take(), &self.server) {
            if sync {
                let path = timeline_path(&s.rating_key, "stopped", s.last_time_ms, s.duration_ms);
                let _ = self.http.send(&srv.base, &srv.token, "GET", &path);
            }
        }
    }

    fn timeline(&self, rk: &str, state: &str, time_ms: i64, dur_ms: i64) {
        self.send("GET", timeline_path(rk, state, time_ms, dur_ms));
    }

    fn finish_progress(&mut self, rk: &str, time_ms: i64, dur_ms: i64) {
        let done = dur_ms > 0 && time_ms as f64 / dur_ms as f64 > 0.9;
        self.patch(rk, |i| {
            if done {
                i.view_offset_ms = 0;
                i.view_count = i.view_count.max(1);
            } else {
                i.view_offset_ms = time_ms;
            }
        });
        self.invalidate_play_state();
    }
}

fn timeline_path(rk: &str, state: &str, time_ms: i64, dur_ms: i64) -> String {
    format!(
        "/:/timeline?ratingKey={rk}&key=%2Flibrary%2Fmetadata%2F{rk}&identifier=com.plexapp.plugins.library&state={state}&time={time_ms}&duration={dur_ms}&playbackTime={time_ms}"
    )
}

pub fn open_browser(url: &str) {
    crate::platform::open_url(url);
}

fn with_paging(path: &str, start: usize, size: usize) -> String {
    format!("{path}{}X-Plex-Container-Start={start}&X-Plex-Container-Size={size}", if path.contains('?') { '&' } else { '?' })
}

fn run_job(http: &Http, job: Job) -> Option<Res> {
    Some(match job {
        Job::CreatePin => Res::Pin(http.create_pin()),
        Job::CheckPin(id) => Res::PinToken(id, http.check_pin(id)),
        Job::Account { token } => Res::Account(http.user(&token).and_then(|u| http.resources(&token).map(|s| (u, s)))),
        Job::Connect { server, last, local_only } => {
            let id = server.id.clone();
            let r = http.pick_connection(&server, last.as_deref(), local_only).and_then(|base| {
                let v = http.get(&base, &server.token, "/library/sections")?;
                Ok((base, model::parse_sections(&v)))
            });
            Res::Connected(id, r)
        }
        Job::Fetch { key, base, token, path, kind } => {
            let l = match http.get(&base, &token, &path) {
                Ok(v) => match kind {
                    FetchKind::Items => {
                        let (items, total) = model::parse_items(&v);
                        Loaded::Items(Arc::new(items), total)
                    }
                    FetchKind::Hubs => Loaded::Hubs(Arc::new(model::parse_hubs(&v))),
                    FetchKind::Meta => match model::parse_items(&v).0.into_iter().next() {
                        Some(i) => Loaded::Meta(Arc::new(i)),
                        None => Loaded::Err(t!("Eintrag nicht gefunden").into()),
                    },
                    FetchKind::Chars => Loaded::Chars(Arc::new(model::parse_first_chars(&v))),
                    FetchKind::Values => Loaded::Values(Arc::new(model::parse_filter_values(&v))),
                },
                Err(e) => Loaded::Err(e),
            };
            Res::Fetched(key, l)
        }
        Job::Page { list, page, base, token, path } => {
            let r = http.get(&base, &token, &with_paging(&path, page * PAGE, PAGE)).map(|v| model::parse_items(&v));
            Res::Page(list, page, r)
        }
        Job::Send { base, token, method, path } => {
            let _ = http.send(&base, &token, method, &path);
            return None;
        }
    })
}
