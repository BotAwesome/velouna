//! Running Lua plugins: one thread each (see `runtime`), messages in both
//! directions, hook chains with time budgets, and what plugins registered
//! (menus, buttons, sources, …). The App applies the effects (ui_ext.rs).

use super::{Installed, Plugins, State};
use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Velouna → plugin.
pub enum ToPlugin {
    /// An event: all handlers registered with `velouna.events.on`.
    Event(String, Vec<Json>),
    /// A hook: the handler's result comes back as `Msg::HookDone`.
    /// `fast`: no network in this call.
    Hook { req: u64, name: String, args: Vec<Json>, fast: bool },
    /// Calls a registered function (menu entry, button, source, dialog …).
    /// `req` 0: no answer wanted.
    Call { req: u64, key: String, args: Vec<Json> },
    /// Console input.
    Eval(String),
    Settings(Map<String, Json>),
    Message { from: String, msg: Json },
    Stop,
}

/// Plugin → Velouna.
pub struct FromPlugin {
    pub id: String,
    pub msg: Msg,
}

pub enum Msg {
    Ready,
    Log(Level, String),
    Toast(String),
    OpenUrl(String),
    Player(PlayerCmd),
    Register(Reg),
    HookDone { req: u64, value: Json, ms: f64 },
    CallDone { req: u64, result: Result<Json, String> },
    Dialog { key: String, spec: Json },
    Progress { id: String, text: String, value: Option<f32>, done: bool, cancel: bool },
    Send { to: String, msg: Json },
    SetSetting(String, Json),
    /// Load error or too many errors: the plugin stopped.
    Crashed(String),
    Stopped,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Info,
    Warn,
    Error,
}

pub enum PlayerCmd {
    Pause(bool),
    Toggle,
    Stop,
    Next,
    Prev,
    Seek(f64),
    SeekBy(f64),
    Volume(f64),
    Mute(bool),
    Speed(f64),
    Load { url: String, mode: String, title: String, start: f64 },
    AddSub { path: String, title: String },
    Shader(Option<PathBuf>),
    AudioFilter(Option<String>),
}

pub enum Reg {
    Event(String),
    Hook(String),
    Menu(MenuReg),
    Button(ButtonReg),
    Source(SourceReg),
    Home(HomeReg),
    Remove(&'static str, String),
}

#[derive(Clone, Debug)]
pub struct MenuReg {
    pub id: String,
    pub label: super::Text,
    /// "player", "file", "folder", "track", "main"
    pub places: Vec<String>,
    /// Only for these item kinds ("audio", "video", "stream" …); empty: all.
    pub kinds: Vec<String>,
    pub icon: String,
}

#[derive(Clone, Debug)]
pub struct ButtonReg {
    pub id: String,
    pub label: super::Text,
    pub icon: String,
}

#[derive(Clone, Debug)]
pub struct SourceReg {
    pub id: String,
    pub label: super::Text,
    pub icon: String,
    pub search: bool,
}

#[derive(Clone, Debug)]
pub struct HomeReg {
    pub id: String,
    pub label: super::Text,
}

/// Per-plugin costs, shown in the console.
#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub load_ms: f64,
    pub calls: u64,
    pub cpu_ms: f64,
    pub max_ms: f64,
    pub net: u64,
    pub net_bytes: u64,
    pub errors: u64,
}

/// What every plugin thread can read without asking: the player now.
#[derive(Default)]
pub struct Shared {
    pub player: RwLock<Json>,
    pub current: RwLock<Json>,
}

pub struct Running {
    pub id: String,
    pub name: String,
    tx: Sender<ToPlugin>,
    pub ready: bool,
    pub stats: Arc<Mutex<Stats>>,
    pub events: BTreeSet<String>,
    pub hooks: BTreeSet<String>,
    /// Hooks switched off after running over their budget again and again.
    pub hooks_off: BTreeSet<String>,
    overruns: HashMap<String, u32>,
    pub menus: Vec<MenuReg>,
    pub buttons: Vec<ButtonReg>,
    pub sources: Vec<SourceReg>,
    pub homes: Vec<HomeReg>,
    pub shader: Option<PathBuf>,
    pub filter: Option<String>,
    pub granted: Vec<String>,
    /// Calls queued while the plugin loads.
    early: Vec<ToPlugin>,
}

impl Running {
    pub fn send(&mut self, m: ToPlugin) {
        if self.ready || matches!(m, ToPlugin::Stop) {
            let _ = self.tx.send(m);
        } else if self.early.len() < 64 {
            self.early.push(m);
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub at: chrono::DateTime<chrono::Local>,
    pub level: Level,
    pub text: String,
}

/// What the answer to a call is for.
pub enum Pending {
    Nothing,
    Eval,
    Browse { key: PageKey },
    Search { key: PageKey },
    Resolve { item: Json },
    Home { plugin: String, id: String },
}

/// A page of a plugin source: plugin, source, path ("" = its start).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PageKey {
    pub plugin: String,
    pub source: String,
    pub path: String,
    /// Set for a search page.
    pub query: String,
}

pub enum PageState {
    Loading(Instant),
    Ready(Json),
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChainKind {
    /// mpv waits at this hook until `hook_continue`.
    Mpv(u64),
    TrackInfo,
    LiveInfo,
}

/// One hook running through the plugins that have it, one after another.
pub struct Chain {
    pub name: String,
    pub kind: ChainKind,
    pub value: Json,
    pub path: String,
    queue: VecDeque<String>,
    current: Option<(String, u64, Instant)>,
    budget: Duration,
    pub changed_by: Vec<String>,
}

pub struct Segment {
    pub from: f64,
    pub to: f64,
    /// skip, ask, mark, mute, chapter
    pub action: String,
    pub label: String,
    pub color: Option<[u8; 3]>,
    pub plugin: String,
    /// Skipped (or undone) in this pass; cleared when playback goes back before it.
    pub done: bool,
}

#[derive(Default)]
pub struct Segments {
    pub path: String,
    pub list: Vec<Segment>,
    /// Muted by a `mute` segment (and whether it was muted before).
    pub muted: Option<bool>,
}

pub struct Toast {
    pub plugin: String,
    pub text: String,
    pub until: Instant,
    /// Undo after a skip: seek back here.
    pub undo: Option<f64>,
}

pub struct PluginDialog {
    pub plugin: String,
    pub key: String,
    pub spec: Json,
    /// Field values while the dialog is open.
    pub values: Map<String, Json>,
}

pub struct Progress {
    pub plugin: String,
    pub id: String,
    pub text: String,
    pub value: Option<f32>,
    pub cancel: bool,
}

pub struct Host {
    pub running: Vec<Running>,
    tx: Sender<FromPlugin>,
    rx: Receiver<FromPlugin>,
    pub shared: Arc<Shared>,
    pub logs: HashMap<String, VecDeque<LogLine>>,
    next_req: u64,
    pub calls: HashMap<u64, (String, Pending)>,
    pub chains: Vec<Chain>,
    seg_reqs: HashMap<u64, (String, String)>,
    pub segments: Segments,
    pub toasts: Vec<Toast>,
    pub dialogs: VecDeque<PluginDialog>,
    pub progress: Vec<Progress>,
    pub pages: HashMap<PageKey, PageState>,
    pub homes: HashMap<(String, String), PageState>,
    /// Last stop reason per plugin (shown on the Extensions page).
    pub errors: HashMap<String, String>,
    pub waker: Option<Waker>,
    /// `--no-plugins` or the user's "start without plugins".
    pub disabled: bool,
    /// Lua plugins were started after the first idle moment.
    pub started: bool,
    /// mpv hooks already added ("on_load", "on_load_fail").
    pub mpv_hooks: BTreeSet<String>,
}

impl Default for Host {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Host {
            running: vec![],
            tx,
            rx,
            shared: Arc::new(Shared::default()),
            logs: HashMap::new(),
            next_req: 1,
            calls: HashMap::new(),
            chains: vec![],
            seg_reqs: HashMap::new(),
            segments: Segments::default(),
            toasts: vec![],
            dialogs: VecDeque::new(),
            progress: vec![],
            pages: HashMap::new(),
            homes: HashMap::new(),
            errors: HashMap::new(),
            waker: None,
            disabled: std::env::args().any(|a| a == "--no-plugins"),
            started: false,
            mpv_hooks: BTreeSet::new(),
        }
    }
}

const LOG_LINES: usize = 500;

impl Host {
    pub fn get(&self, id: &str) -> Option<&Running> {
        self.running.iter().find(|r| r.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Running> {
        self.running.iter_mut().find(|r| r.id == id)
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    /// All running plugins have loaded their main file.
    pub fn all_ready(&self) -> bool {
        self.running.iter().all(|r| r.ready)
    }

    pub fn log(&mut self, id: &str, level: Level, text: impl Into<String>) {
        let text = text.into();
        match level {
            Level::Info => log::info!("plugin {id}: {text}"),
            Level::Warn => log::warn!("plugin {id}: {text}"),
            Level::Error => log::error!("plugin {id}: {text}"),
        }
        let l = self.logs.entry(id.to_string()).or_default();
        l.push_back(LogLine { at: chrono::Local::now(), level, text });
        while l.len() > LOG_LINES {
            l.pop_front();
        }
    }

    /// Permissions the plugin asks for that the user has not agreed to.
    pub fn missing_grants(p: &Installed, st: Option<&State>) -> Vec<String> {
        p.manifest.permissions.iter().filter(|x| !st.is_some_and(|s| s.granted.contains(x))).cloned().collect()
    }

    /// Starts a Lua plugin (its permissions must be granted).
    pub fn start(&mut self, p: &Installed, st: &State, all: &Plugins) -> Result<(), String> {
        let m = &p.manifest;
        if self.is_running(&m.id) {
            return Ok(());
        }
        if !Self::missing_grants(p, Some(st)).is_empty() {
            return Err(t!("Berechtigungen fehlen").into());
        }
        let mut modules = HashMap::new();
        for r in &m.requires {
            let Some(dep) = all.get(&r.id) else { return Err(tf!("Braucht das Plugin {}", r.id)) };
            if let Some(min) = r.version.strip_prefix(">=") {
                if super::version_lt(&dep.manifest.version, min.trim()) {
                    return Err(tf!("Braucht {} {} oder neuer", r.id, min.trim()));
                }
            }
            modules.insert(r.id.clone(), dep.dir.join(dep.manifest.main_file()));
        }
        let granted: Vec<String> = m.permissions.iter().filter(|x| st.granted.contains(x)).cloned().collect();
        let (tx, rx) = mpsc::channel();
        let stats = Arc::new(Mutex::new(Stats::default()));
        let spec = super::runtime::Spec {
            id: m.id.clone(),
            version: m.version.clone(),
            dir: p.dir.clone(),
            main: m.main_file().to_string(),
            granted: granted.clone(),
            settings: settings_values(p, st),
            defs: m.settings.clone(),
            modules,
            data_dir: super::data_root().join(&m.id),
        };
        let waker = self.waker.clone().unwrap_or_else(|| Arc::new(|| {}));
        super::runtime::spawn(spec, rx, self.tx.clone(), self.shared.clone(), stats.clone(), waker)?;
        self.errors.remove(&m.id);
        self.log(&m.id, Level::Info, format!("{} {} starting", m.name.get(), m.version));
        self.running.push(Running {
            id: m.id.clone(),
            name: m.name.get(),
            tx,
            ready: false,
            stats,
            events: BTreeSet::new(),
            hooks: BTreeSet::new(),
            hooks_off: BTreeSet::new(),
            overruns: HashMap::new(),
            menus: vec![],
            buttons: vec![],
            sources: vec![],
            homes: vec![],
            shader: None,
            filter: None,
            granted,
            early: vec![],
        });
        Ok(())
    }

    /// Stops a plugin; its thread ends on its own. Returns what the App
    /// has to undo (shader, audio filter).
    pub fn stop(&mut self, id: &str) -> Option<Running> {
        let i = self.running.iter().position(|r| r.id == id)?;
        let mut r = self.running.remove(i);
        r.send(ToPlugin::Event("app-quit".into(), vec![]));
        r.send(ToPlugin::Stop);
        self.segments.list.retain(|s| s.plugin != id);
        self.toasts.retain(|t| t.plugin != id);
        self.dialogs.retain(|d| d.plugin != id);
        self.progress.retain(|p| p.plugin != id);
        self.pages.retain(|k, _| k.plugin != id);
        self.homes.retain(|k, _| k.0 != id);
        // Chains go on without it.
        for c in &mut self.chains {
            c.queue.retain(|q| q != id);
            if c.current.as_ref().is_some_and(|(p, _, _)| p == id) {
                c.current = None;
            }
        }
        self.log(id, Level::Info, "stopped");
        Some(r)
    }

    /// Asks all plugins to finish (`app-quit`) and waits up to `limit`.
    pub fn shutdown(&mut self, limit: Duration) {
        if self.running.is_empty() {
            return;
        }
        let mut left: BTreeSet<String> = self.running.iter().map(|r| r.id.clone()).collect();
        for r in &mut self.running {
            r.ready = true;
            r.send(ToPlugin::Event("app-quit".into(), vec![]));
            r.send(ToPlugin::Stop);
        }
        let end = Instant::now() + limit;
        while !left.is_empty() {
            let now = Instant::now();
            if now >= end {
                break;
            }
            match self.rx.recv_timeout(end - now) {
                Ok(FromPlugin { id, msg: Msg::Stopped | Msg::Crashed(_) }) => {
                    left.remove(&id);
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        self.running.clear();
    }

    pub fn next_req(&mut self) -> u64 {
        self.next_req += 1;
        self.next_req
    }

    /// Sends an event to every running plugin that listens to it.
    pub fn emit(&mut self, name: &str, args: Vec<Json>) {
        for r in &mut self.running {
            if !r.ready || r.events.contains(name) {
                r.send(ToPlugin::Event(name.to_string(), args.clone()));
            }
        }
    }

    pub fn listens(&self, name: &str) -> bool {
        self.running.iter().any(|r| r.events.contains(name))
    }

    pub fn has_hook(&self, name: &str) -> bool {
        self.running.iter().any(|r| r.hooks.contains(name) && !r.hooks_off.contains(name))
    }

    /// Calls a registered function of a plugin.
    pub fn call(&mut self, plugin: &str, key: &str, args: Vec<Json>, purpose: Pending) -> Option<u64> {
        let req = self.next_req();
        let r = self.running.iter_mut().find(|r| r.id == plugin)?;
        r.send(ToPlugin::Call { req, key: key.to_string(), args });
        self.calls.insert(req, (plugin.to_string(), purpose));
        Some(req)
    }

    pub fn eval(&mut self, plugin: &str, code: &str) {
        if let Some(r) = self.get_mut(plugin) {
            r.send(ToPlugin::Eval(code.to_string()));
        }
    }

    pub fn send_settings(&mut self, p: &Installed, st: &State) {
        let v = settings_values(p, st);
        if let Some(r) = self.get_mut(&p.manifest.id) {
            r.send(ToPlugin::Settings(v));
        }
    }

    /// Starts a hook chain through all plugins with this hook, in list order.
    pub fn start_chain(&mut self, name: &str, kind: ChainKind, value: Json, path: &str, budget: Duration) {
        let queue: VecDeque<String> = self.running.iter().filter(|r| r.ready && r.hooks.contains(name) && !r.hooks_off.contains(name)).map(|r| r.id.clone()).collect();
        self.chains.push(Chain { name: name.into(), kind, value, path: path.into(), queue, current: None, budget, changed_by: vec![] });
    }

    /// Moves chains on; returns the finished ones.
    pub fn advance_chains(&mut self) -> Vec<Chain> {
        let now = Instant::now();
        let mut done = vec![];
        let mut i = 0;
        while i < self.chains.len() {
            // Over budget: go on without this plugin.
            if let Some((p, _, deadline)) = &self.chains[i].current {
                if now > *deadline {
                    let (p, name) = (p.clone(), self.chains[i].name.clone());
                    self.chains[i].current = None;
                    self.overrun(&p, &name);
                }
            }
            while self.chains[i].current.is_none() {
                let Some(p) = self.chains[i].queue.pop_front() else { break };
                let req = self.next_req();
                let fast = matches!(self.chains[i].name.as_str(), "before-play" | "display");
                let (name, value, budget) = (self.chains[i].name.clone(), self.chains[i].value.clone(), self.chains[i].budget);
                if let Some(r) = self.get_mut(&p) {
                    r.send(ToPlugin::Hook { req, name, args: vec![value], fast });
                    self.chains[i].current = Some((p, req, now + budget));
                }
            }
            if self.chains[i].current.is_none() {
                done.push(self.chains.remove(i));
            } else {
                i += 1;
            }
        }
        if !self.chains.is_empty() {
            if let Some(w) = &self.waker {
                // Look again when the budget runs out.
                let w = w.clone();
                let wait = self.chains.iter().filter_map(|c| c.current.as_ref().map(|x| x.2)).min().map(|d| d.saturating_duration_since(now)).unwrap_or_default();
                std::thread::spawn(move || {
                    std::thread::sleep(wait + Duration::from_millis(1));
                    w();
                });
            }
        }
        done
    }

    fn overrun(&mut self, plugin: &str, hook: &str) {
        let Some(r) = self.get_mut(plugin) else { return };
        let n = r.overruns.entry(hook.to_string()).or_insert(0);
        *n += 1;
        let off = *n >= 5;
        if off {
            r.hooks_off.insert(hook.to_string());
        }
        self.log(plugin, Level::Warn, format!("hook '{hook}' ran over its time budget and was passed over"));
        if off {
            self.log(plugin, Level::Error, format!("hook '{hook}' is switched off: too slow five times"));
        }
    }

    /// Asks every plugin with a `segments` hook for parts of this file.
    pub fn request_segments(&mut self, path: &str, item: &Json) {
        if self.segments.path != path {
            self.segments = Segments { path: path.into(), ..Default::default() };
        }
        let ids: Vec<String> = self.running.iter().filter(|r| r.ready && r.hooks.contains("segments") && !r.hooks_off.contains("segments")).map(|r| r.id.clone()).collect();
        for id in ids {
            let req = self.next_req();
            if let Some(r) = self.get_mut(&id) {
                r.send(ToPlugin::Hook { req, name: "segments".into(), args: vec![item.clone()], fast: false });
                self.seg_reqs.insert(req, (id, path.to_string()));
            }
        }
    }

    /// Takes everything the plugins sent since the last frame. Bookkeeping
    /// (registrations, logs, hook answers) happens here; the rest goes to
    /// the App.
    pub fn drain(&mut self) -> Vec<FromPlugin> {
        let mut out = vec![];
        while let Ok(m) = self.rx.try_recv() {
            let id = m.id.clone();
            match m.msg {
                Msg::Ready => {
                    if let Some(r) = self.get_mut(&id) {
                        r.ready = true;
                        for e in std::mem::take(&mut r.early) {
                            let _ = r.tx.send(e);
                        }
                        let ms = r.stats.lock().unwrap().load_ms;
                        self.log(&id, Level::Info, format!("loaded in {ms:.1} ms"));
                    }
                }
                Msg::Log(level, text) => self.log(&id, level, text),
                Msg::Register(reg) => self.register(&id, reg),
                Msg::HookDone { req, value, ms } => {
                    if let Some((plugin, path)) = self.seg_reqs.remove(&req) {
                        if path == self.segments.path {
                            self.add_segments(&plugin, &value);
                        }
                        continue;
                    }
                    let mut over = None;
                    if let Some(c) = self.chains.iter_mut().find(|c| c.current.as_ref().is_some_and(|x| x.1 == req)) {
                        let (p, _, deadline) = c.current.take().unwrap();
                        if Instant::now() > deadline {
                            over = Some((p, c.name.clone()));
                        } else if let Json::Object(o) = value {
                            if !o.is_empty() {
                                if let Json::Object(cur) = &mut c.value {
                                    for (k, v) in o {
                                        cur.insert(k, v);
                                    }
                                } else {
                                    c.value = Json::Object(o);
                                }
                                c.changed_by.push(p);
                            }
                        }
                    }
                    if let Some((p, name)) = over {
                        self.overrun(&p, &name);
                    }
                    let _ = ms;
                }
                Msg::Crashed(e) => {
                    self.log(&id, Level::Error, format!("stopped: {e}"));
                    self.errors.insert(id.clone(), e.clone());
                    out.push(FromPlugin { id, msg: Msg::Crashed(e) });
                }
                Msg::Stopped => {}
                msg => out.push(FromPlugin { id, msg }),
            }
        }
        out
    }

    fn register(&mut self, id: &str, reg: Reg) {
        let Some(r) = self.get_mut(id) else { return };
        match reg {
            Reg::Event(e) => {
                r.events.insert(e);
            }
            Reg::Hook(h) => {
                r.hooks.insert(h);
            }
            Reg::Menu(m) => {
                r.menus.retain(|x| x.id != m.id);
                r.menus.push(m);
            }
            Reg::Button(b) => {
                r.buttons.retain(|x| x.id != b.id);
                r.buttons.push(b);
            }
            Reg::Source(s) => {
                r.sources.retain(|x| x.id != s.id);
                r.sources.push(s);
            }
            Reg::Home(h) => {
                r.homes.retain(|x| x.id != h.id);
                r.homes.push(h);
            }
            Reg::Remove(kind, rid) => match kind {
                "menu" => r.menus.retain(|x| x.id != rid),
                "button" => r.buttons.retain(|x| x.id != rid),
                "source" => r.sources.retain(|x| x.id != rid),
                "home" => r.homes.retain(|x| x.id != rid),
                _ => {}
            },
        }
    }

    fn add_segments(&mut self, plugin: &str, v: &Json) {
        let Some(list) = v.as_array() else { return };
        let num = |o: &Map<String, Json>, a: &str, b: &str| o.get(a).or_else(|| o.get(b)).and_then(|x| x.as_f64());
        let mut n = 0;
        for s in list.iter().filter_map(|x| x.as_object()).take(500) {
            let (Some(from), Some(to)) = (num(s, "from", "start"), num(s, "to", "stop")) else { continue };
            if !(from.is_finite() && to.is_finite()) || to < from || from < 0.0 {
                continue;
            }
            let action = s.get("action").and_then(|x| x.as_str()).unwrap_or("mark");
            let action = if matches!(action, "skip" | "ask" | "mark" | "mute" | "chapter") { action } else { "mark" };
            let color = s.get("color").and_then(|x| x.as_str()).and_then(parse_color);
            let label = s.get("label").and_then(|x| x.as_str()).unwrap_or("").chars().take(80).collect();
            self.segments.list.push(Segment { from, to, action: action.into(), label, color, plugin: plugin.into(), done: false });
            n += 1;
        }
        self.segments.list.sort_by(|a, b| a.from.total_cmp(&b.from));
        if n > 0 {
            self.log(plugin, Level::Info, format!("{n} segments"));
        }
    }

    pub fn toast(&mut self, plugin: &str, text: impl Into<String>, secs: f32, undo: Option<f64>) {
        self.toasts.retain(|t| !(t.plugin == plugin && t.undo.is_none() && undo.is_none()));
        self.toasts.push(Toast { plugin: plugin.into(), text: text.into(), until: Instant::now() + Duration::from_secs_f32(secs), undo });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    pub fn wake(&self) {
        if let Some(w) = &self.waker {
            w();
        }
    }
}

/// `#rrggbb` → rgb.
pub fn parse_color(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// The plugin's settings: stored values over the defaults (secrets are
/// read by the plugin itself, from the keychain).
pub fn settings_values(p: &Installed, st: &State) -> Map<String, Json> {
    let mut out = Map::new();
    for d in &p.manifest.settings {
        if d.kind == "secret" || d.kind == "action" {
            continue;
        }
        let v = st.settings.get(&d.id).cloned().unwrap_or_else(|| d.default.clone());
        out.insert(d.id.clone(), v);
    }
    // Values the plugin stored itself with settings.set.
    for (k, v) in &st.settings {
        out.entry(k.clone()).or_insert_with(|| v.clone());
    }
    out
}

/// Keychain service of a plugin's secrets.
pub fn secret_service(id: &str) -> String {
    format!("Velouna plugin {id}")
}

/// An item for the player events: what is playing.
pub fn item_json(path: &str, title: &str, artist: Option<&str>, album: Option<&str>, duration: f64, audio: bool, source: &str) -> Json {
    let mut o = json!({
        "path": path,
        "title": title,
        "duration": duration,
        "kind": if audio { "audio" } else { "video" },
        "source": source,
    });
    if let Some(a) = artist {
        o["artist"] = json!(a);
    }
    if let Some(a) = album {
        o["album"] = json!(a);
    }
    if crate::media::is_url(path) {
        o["url"] = json!(path);
    }
    if let Some(y) = super::util::youtube_id(path) {
        o["youtube_id"] = json!(y);
    }
    o
}
