//! Plex integration: sign-in, server connection, cached API data, artwork,
//! and play-state reporting. The UI lives in `ui_plex.rs`.

pub mod api;
pub mod bif;
pub mod download;
pub mod images;
pub mod mb;
pub mod model;

pub use model::{Backend, Hub, Item, Kind, Section, Server, User};

use crate::config::{MediaServerConfig, PlexConfig};
use api::Http;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

pub const PAGE: usize = 60;
const KEYRING_SERVICE: &str = "Velouna";
/// Credential-store names of earlier app names, newest first.
const LEGACY_KEYRING_SERVICES: &[&str] = &["Velour", "VLB"];
const KEYRING_USER: &str = "plex-token";

// ------------------------------------------------------------ registry

/// What Velouna knows about a Plex stream that sits in mpv's playlist.
#[derive(Clone, Debug)]
pub struct Remote {
    pub rating_key: String,
    pub title: String,
    pub duration: f64,
    pub conn: Conn,
    pub kind: Kind,
    /// For the queue rows: the item's own title, the line under it (artist
    /// of a track, show of an episode), the album and an image path.
    pub name: String,
    pub sub: String,
    pub album: String,
    pub thumb: String,
}

/// How to reach one server: everything a worker needs for a request.
#[derive(Clone, Debug, Default)]
pub struct Conn {
    pub base: String,
    pub token: String,
    pub backend: Backend,
    /// Jellyfin/Emby only.
    pub user_id: String,
    pub device_id: String,
}

impl Conn {
    /// `name=token` for URLs that cannot carry headers (ffmpeg, subtitles).
    pub fn token_query(&self) -> String {
        let p = if self.backend.media_browser() { mb::token_param(self.backend) } else { "X-Plex-Token" };
        format!("{p}={}", api::url_encode(&self.token))
    }
    /// `path` (server-relative paths get the base) with the token appended.
    pub fn with_token(&self, path: &str) -> String {
        let url = if path.starts_with("http") { path.to_string() } else { format!("{}{path}", self.base) };
        format!("{url}{}{}", if url.contains('?') { '&' } else { '?' }, self.token_query())
    }
    /// Header lines for mpv's `http-header-fields`.
    pub fn stream_headers(&self) -> String {
        if self.backend.media_browser() { mb::stream_headers(&self.token) } else { format!("X-Plex-Token: {}", self.token) }
    }
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
        Some(r) => r.conn.with_token(url),
        None => url.to_string(),
    }
}

// ------------------------------------------------------------- tokens

use crate::platform::secret;

/// A stored secret; one saved under an earlier app name is moved over.
fn get_secret(user: &str) -> Option<String> {
    if let Some(t) = secret::get(KEYRING_SERVICE, user) {
        return Some(t);
    }
    for old in LEGACY_KEYRING_SERVICES {
        if let Some(t) = secret::get(old, user) {
            if secret::set(KEYRING_SERVICE, user, &t) {
                secret::delete(old, user);
            }
            return Some(t);
        }
    }
    None
}

fn load_token() -> Option<String> {
    get_secret(KEYRING_USER)
}
fn store_token(t: &str) -> bool {
    secret::set(KEYRING_SERVICE, KEYRING_USER, t)
}
fn delete_token() {
    secret::delete(KEYRING_SERVICE, KEYRING_USER);
}

/// Credential-store key for a Jellyfin/Emby token.
fn mb_key(id: &str) -> String {
    format!("mediaserver-{id}")
}

/// A saved Jellyfin/Emby server, with its token from the credential store.
fn media_server(m: &MediaServerConfig) -> Option<Server> {
    let token = get_secret(&mb_key(&m.id))?;
    Some(Server {
        id: m.id.clone(),
        name: m.name.clone(),
        token,
        owned: true,
        owner: m.user_name.clone(),
        presence: true,
        connections: vec![model::Connection { uri: m.base.clone(), ..Default::default() }],
        backend: m.backend,
        user_id: m.user_id.clone(),
        device_id: m.device_id.clone(),
    })
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
    pub backend: Backend,
    pub user_id: String,
    pub device_id: String,
}

impl ActiveServer {
    pub fn conn(&self) -> Conn {
        Conn { base: self.base.clone(), token: self.token.clone(), backend: self.backend, user_id: self.user_id.clone(), device_id: self.device_id.clone() }
    }
}

/// Sign-in form state for a Jellyfin or Emby server.
#[derive(Clone, Debug, Default)]
pub struct MbLogin {
    pub busy: bool,
    pub error: Option<String>,
}

/// Loading state handed to the UI.
pub enum St<T> {
    Loading,
    Ready(T),
    Failed(String),
}

#[derive(Clone)]
enum Loaded {
    Items(Arc<Vec<Item>>),
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
    Fetch { key: String, conn: Conn, path: String, kind: FetchKind, sections: Arc<Vec<Section>> },
    Page { list: String, page: usize, conn: Conn, path: String },
    Send { conn: Conn, method: &'static str, path: String },
    MbLogin { url: String, user: String, password: String, device_id: String },
}

enum Res {
    Pin(Result<(i64, String), String>),
    PinToken(i64, Result<Option<String>, String>),
    Account(Result<(User, Vec<Server>), String>),
    Connected(String, Result<(String, Vec<Section>), String>),
    Fetched(String, Loaded),
    Page(String, usize, Result<(Vec<Item>, i64), String>),
    MbLogin(Result<(Server, String), String>),
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
    conn: Conn,
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
    /// Offline copies (right-click > Herunterladen).
    pub downloads: download::Downloads,
    pub pending_play: Option<PlayReq>,
    pub token_store_failed: bool,
    pub mb_login: MbLogin,
    /// The Jellyfin/Emby server shown refused its token (signed out on the server).
    pub mb_expired: bool,
    token: Option<String>,
    /// Server being connected to or shown (Plex account results don't override it).
    target: Option<String>,
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
            downloads: download::Downloads::new(ctx),
            http,
            status: Status::SignedOut,
            user: None,
            servers: vec![],
            server: None,
            sections: vec![],
            images,
            pending_play: None,
            token_store_failed: false,
            mb_login: MbLogin::default(),
            mb_expired: false,
            token: None,
            target: None,
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
            p.user = Some(User { name: cfg.user_name.clone(), });
            let _ = p.jobs.send(Job::Account { token: t });
        }
        p.servers = cfg.media_servers.iter().filter_map(media_server).collect();
        // A Jellyfin/Emby server was last used (or is all there is): connect right away.
        let mb_pick = cfg.server_id.clone().filter(|id| p.servers.iter().any(|s| &s.id == id)).or_else(|| p.token.is_none().then(|| p.servers.first().map(|s| s.id.clone())).flatten());
        if let Some(id) = mb_pick {
            p.select_server(&id, false);
        }
        (p, dirty)
    }

    pub fn plex_signed_in(&self) -> bool {
        self.token.is_some()
    }
    /// Id of the server shown (or being connected to).
    pub fn active_id(&self) -> Option<String> {
        self.server.as_ref().map(|s| s.id.clone()).or_else(|| self.target.clone())
    }
    /// Kind of the server shown (or being connected to).
    pub fn backend(&self) -> Backend {
        if let Some(s) = &self.server {
            return s.backend;
        }
        self.target.as_ref().and_then(|id| self.servers.iter().find(|s| &s.id == id)).map(|s| s.backend).unwrap_or_default()
    }

    // ------------------------------------------------- Jellyfin / Emby

    /// Signs in to a Jellyfin or Emby server (result arrives in `poll`).
    pub fn add_media_server(&mut self, url: &str, user: &str, password: &str) {
        self.mb_login = MbLogin { busy: true, error: None };
        let device_id = format!("velouna-{}", random_id());
        let _ = self.jobs.send(Job::MbLogin { url: url.trim().to_string(), user: user.trim().to_string(), password: password.to_string(), device_id });
    }

    /// Signs out of a Jellyfin/Emby server and forgets it.
    pub fn remove_media_server(&mut self, id: &str, cfg: &mut PlexConfig) {
        if let Some(s) = self.servers.iter().find(|s| s.id == id && s.backend.media_browser()).cloned() {
            let conn = Conn { base: s.connections.first().map(|c| c.uri.clone()).unwrap_or_default(), token: s.token.clone(), backend: s.backend, user_id: s.user_id.clone(), device_id: s.device_id.clone() };
            let http = self.http.clone();
            std::thread::spawn(move || mb::logout(&http, &conn));
            secret::delete(KEYRING_SERVICE, &mb_key(id));
        }
        self.servers.retain(|s| s.id != id);
        cfg.media_servers.retain(|m| m.id != id);
        if self.target.as_deref() == Some(id) || self.server.as_ref().is_some_and(|s| s.id == id) {
            self.server = None;
            self.target = None;
            self.sections.clear();
            self.clear_cache();
            cfg.server_id = None;
            self.status = Status::SignedOut;
            if let Some(t) = self.token.clone() {
                self.status = Status::Connecting(t!("Anmelden…").into());
                let _ = self.jobs.send(Job::Account { token: t });
            } else if let Some(next) = self.servers.first().map(|s| s.id.clone()) {
                self.select_server(&next, false);
            }
        }
    }
    pub fn ready(&self) -> bool {
        self.status == Status::Ready && self.server.is_some()
    }

    // ------------------------------------------------------------ login

    pub fn start_login(&mut self) {
        // Signing in to Plex leaves a Jellyfin/Emby server that is shown.
        if self.backend().media_browser() {
            self.server = None;
            self.target = None;
            self.sections.clear();
            self.clear_cache();
        }
        self.status = Status::Connecting(t!("Code wird angefordert…").into());
        let _ = self.jobs.send(Job::CreatePin);
    }

    pub fn cancel_login(&mut self) {
        self.status = Status::SignedOut;
        self.pin_check_at = None;
        // Back to a Jellyfin/Emby server, if there is one.
        if let Some(id) = self.servers.iter().find(|s| s.backend.media_browser()).map(|s| s.id.clone()) {
            self.select_server(&id, false);
        }
    }

    pub fn sign_out(&mut self, cfg: &mut PlexConfig) {
        delete_token();
        self.token = None;
        self.user = None;
        self.servers.retain(|s| s.backend.media_browser());
        cfg.user_name.clear();
        if self.backend().media_browser() {
            return; // a Jellyfin/Emby server is shown: it stays
        }
        self.server = None;
        self.target = None;
        self.sections.clear();
        self.clear_cache();
        self.images.clear();
        self.status = Status::SignedOut;
        cfg.server_id = None;
        cfg.server_name.clear();
        cfg.last_uri = None;
        if let Some(id) = self.servers.first().map(|s| s.id.clone()) {
            self.select_server(&id, false);
        }
    }

    pub fn select_server(&mut self, id: &str, local_only: bool) {
        let Some(s) = self.servers.iter().find(|s| s.id == id).cloned() else { return };
        self.status = Status::Connecting(tf!("Verbinde mit {}…", s.name));
        self.mb_expired = false;
        self.target = Some(s.id.clone());
        self.server = None;
        self.sections.clear();
        self.clear_cache();
        let _ = self.jobs.send(Job::Connect { server: s, last: None, local_only });
    }

    pub fn retry(&mut self, cfg: &PlexConfig, local_only: bool) {
        if let Some(id) = self.target.clone().filter(|id| self.servers.iter().any(|s| &s.id == id && s.backend.media_browser())) {
            self.select_server(&id, local_only);
        } else if let Some(t) = self.token.clone() {
            let plex = |s: &&Server| s.backend == Backend::Plex;
            if !self.servers.iter().any(|s| s.backend == Backend::Plex) {
                self.status = Status::Connecting(t!("Anmelden…").into());
                let _ = self.jobs.send(Job::Account { token: t });
            } else if let Some(id) = cfg
                .server_id
                .clone()
                .filter(|id| self.servers.iter().filter(plex).any(|s| &s.id == id))
                .or_else(|| self.servers.iter().find(plex).map(|s| s.id.clone()))
            {
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
                    // Plex servers first, then the Jellyfin/Emby ones.
                    let others: Vec<Server> = self.servers.drain(..).filter(|s| s.backend.media_browser()).collect();
                    self.servers = servers.clone();
                    self.servers.extend(others);
                    // Already on a Jellyfin/Emby server: the account only fills the list.
                    if self.target.as_ref().is_some_and(|id| self.servers.iter().any(|s| &s.id == id && s.backend.media_browser())) {
                        continue;
                    }
                    let pick = cfg
                        .server_id
                        .as_ref()
                        .and_then(|id| servers.iter().find(|s| &s.id == id))
                        .or_else(|| servers.iter().find(|s| s.owned && s.presence))
                        .or_else(|| servers.iter().find(|s| s.presence))
                        .or_else(|| servers.first())
                        .cloned();
                    if let Some(s) = &pick {
                        self.target = Some(s.id.clone());
                    }
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
                    if self.backend().media_browser() {
                        // Plex is down or signed out, but a Jellyfin/Emby server is shown.
                        if api::is_unauthorized(&e) {
                            delete_token();
                            self.token = None;
                        }
                    } else if api::is_unauthorized(&e) {
                        delete_token();
                        self.token = None;
                        self.status = Status::Error(t!("Die Plex-Anmeldung ist abgelaufen. Bitte erneut verbinden.").into());
                    } else {
                        self.status = Status::Error(tf!("plex.tv nicht erreichbar: {e}"));
                    }
                }
                Res::Connected(id, Ok((base, sections))) => {
                    let Some(s) = self.servers.iter().find(|s| s.id == id).cloned() else { continue };
                    if self.target.as_ref() != Some(&id) {
                        continue; // the user picked another server meanwhile
                    }
                    if cfg.server_id.as_deref() != Some(&s.id) || cfg.last_uri.as_deref() != Some(&base) || cfg.server_name != s.name {
                        cfg.server_id = Some(s.id.clone());
                        cfg.server_name = s.name.clone();
                        cfg.last_uri = Some(base.clone());
                        dirty = true;
                    }
                    self.server = Some(ActiveServer { id: s.id, name: s.name, base, token: s.token, backend: s.backend, user_id: s.user_id, device_id: s.device_id });
                    self.sections = sections;
                    self.status = Status::Ready;
                    self.generation += 1;
                }
                Res::Connected(id, Err(e)) => {
                    if self.target.as_ref() == Some(&id) {
                        let mb = self.servers.iter().any(|s| s.id == id && s.backend.media_browser());
                        self.mb_expired = mb && api::is_unauthorized(&e);
                        self.status = Status::Error(if self.mb_expired { t!("Die Anmeldung gilt nicht mehr, zum Beispiel weil das Gerät auf dem Server entfernt wurde. Bitte melde dich neu an.").into() } else { e });
                    }
                }
                Res::MbLogin(Ok((server, user_name))) => {
                    self.mb_login = MbLogin::default();
                    self.token_store_failed = !secret::set(KEYRING_SERVICE, &mb_key(&server.id), &server.token);
                    cfg.media_servers.retain(|m| m.id != server.id);
                    cfg.media_servers.push(MediaServerConfig {
                        id: server.id.clone(),
                        name: server.name.clone(),
                        base: server.connections.first().map(|c| c.uri.clone()).unwrap_or_default(),
                        backend: server.backend,
                        user_id: server.user_id.clone(),
                        device_id: server.device_id.clone(),
                        user_name,
                    });
                    dirty = true;
                    let id = server.id.clone();
                    self.servers.retain(|s| s.id != id);
                    self.servers.push(server);
                    self.select_server(&id, local_only);
                }
                Res::MbLogin(Err(e)) => self.mb_login = MbLogin { busy: false, error: Some(e) },
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
                let sections = Arc::new(self.sections.clone());
                let _ = self.jobs.send(Job::Fetch { key, conn: s.conn(), path: path.to_string(), kind, sections });
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
            Some(Loaded::Items(i)) => St::Ready(i),
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
        let _ = self.jobs.send(Job::Page { list: path.to_string(), page, conn: s.conn(), path: path.to_string() });
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
                Loaded::Items(v) => fix(v),
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
        if s.backend.media_browser() {
            return Some(mb::image_url(&s.base, s.backend, path, w, h));
        }
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
    /// Name of the server shown (for download rows).
    pub fn server_name(&self) -> String {
        self.server.as_ref().map(|s| s.name.clone()).unwrap_or_default()
    }
    /// Connection of the server shown, for registering streams.
    pub fn conn(&self) -> Option<Conn> {
        self.server.as_ref().map(|s| s.conn())
    }

    fn send(&self, method: &'static str, path: String) {
        if let Some(s) = &self.server {
            self.send_to(&s.conn(), method, path);
        }
    }
    fn send_to(&self, conn: &Conn, method: &'static str, path: String) {
        let _ = self.jobs.send(Job::Send { conn: conn.clone(), method, path });
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
                    self.timeline(&s.conn, &s.rating_key, "stopped", s.last_time_ms, s.duration_ms);
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
            conn: r.conn.clone(),
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
            self.timeline(&r.conn, &rk, state, time_ms, dur_ms);
        }
        if sync && scrobble {
            self.send_to(&r.conn, "GET", format!("/:/scrobble?identifier=com.plexapp.plugins.library&key={rk}"));
            // Keep menus honest ("Als ungesehen markieren") without refetching mid-playback.
            self.patch(&rk, |i| i.view_count = i.view_count.max(1));
        }
        if due {
            self.patch(&rk, |i| i.view_offset_ms = time_ms);
        }
    }

    /// Stops the current session (app exit). Blocking with a short timeout.
    pub fn stop_now(&mut self, sync: bool) {
        if let Some(s) = self.session.take() {
            if sync {
                let path = timeline_path(&s.rating_key, "stopped", s.last_time_ms, s.duration_ms);
                let _ = send_now(&self.http, &s.conn, "GET", &path);
            }
        }
    }

    fn timeline(&self, conn: &Conn, rk: &str, state: &str, time_ms: i64, dur_ms: i64) {
        self.send_to(conn, "GET", timeline_path(rk, state, time_ms, dur_ms));
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

/// A write (timeline, scrobble …), answered by the right kind of server. Blocking.
fn send_now(http: &Http, conn: &Conn, method: &str, path: &str) -> Result<(), String> {
    if conn.backend.media_browser() { mb::send(http, conn, path) } else { http.send(&conn.base, &conn.token, method, path) }
}

fn run_job(http: &Http, job: Job) -> Option<Res> {
    Some(match job {
        Job::CreatePin => Res::Pin(http.create_pin()),
        Job::CheckPin(id) => Res::PinToken(id, http.check_pin(id)),
        Job::Account { token } => Res::Account(http.user(&token).and_then(|u| http.resources(&token).map(|s| (u, s)))),
        Job::Connect { server, .. } if server.backend.media_browser() => {
            let conn = Conn {
                base: server.connections.first().map(|c| c.uri.clone()).unwrap_or_default(),
                token: server.token.clone(),
                backend: server.backend,
                user_id: server.user_id.clone(),
                device_id: server.device_id.clone(),
            };
            Res::Connected(server.id.clone(), mb::connect(http, &conn).map(|sections| (conn.base.clone(), sections)))
        }
        Job::Connect { server, last, local_only } => {
            let id = server.id.clone();
            let r = http.pick_connection(&server, last.as_deref(), local_only).and_then(|base| {
                let v = http.get(&base, &server.token, "/library/sections")?;
                Ok((base, model::parse_sections(&v)))
            });
            Res::Connected(id, r)
        }
        Job::Fetch { key, conn, path, kind, sections } if conn.backend.media_browser() => {
            let l = match kind {
                FetchKind::Items => mb::items(http, &conn, &path).map(|(i, _)| Loaded::Items(Arc::new(i))),
                FetchKind::Hubs => mb::hubs(http, &conn, &path, &sections).map(|h| Loaded::Hubs(Arc::new(h))),
                FetchKind::Meta => {
                    let id = path.trim_start_matches("/library/metadata/").split('?').next().unwrap_or("").to_string();
                    mb::meta(http, &conn, &id).map(|i| Loaded::Meta(Arc::new(i)))
                }
                FetchKind::Chars => Ok(Loaded::Chars(Arc::new(vec![]))),
                FetchKind::Values => mb::values(http, &conn, &path).map(|v| Loaded::Values(Arc::new(v))),
            };
            Res::Fetched(key, l.unwrap_or_else(Loaded::Err))
        }
        Job::Fetch { key, conn, path, kind, .. } => {
            let (base, token) = (conn.base, conn.token);
            let l = match http.get(&base, &token, &path) {
                Ok(v) => match kind {
                    FetchKind::Items => {
                        Loaded::Items(Arc::new(model::parse_items(&v).0))
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
        Job::Page { list, page, conn, path } => {
            let paged = with_paging(&path, page * PAGE, PAGE);
            let r = if conn.backend.media_browser() {
                mb::items(http, &conn, &paged)
            } else {
                http.get(&conn.base, &conn.token, &paged).map(|v| model::parse_items(&v))
            };
            Res::Page(list, page, r)
        }
        Job::Send { conn, method, path } => {
            let _ = send_now(http, &conn, method, &path);
            return None;
        }
        Job::MbLogin { url, user, password, device_id } => Res::MbLogin(mb::login(http, &url, &user, &password, &device_id)),
    })
}
