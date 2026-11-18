//! One Lua plugin (API 2) in its own thread: the sandbox, the `velouna`
//! table and the loop that runs events, hooks, calls and timers.
//!
//! The sandbox is the one visualizers have (no io/os/package/debug, memory
//! and time limits) plus what the plugin's granted permissions allow.
//! Everything that reaches outside goes through Rust: the network only via
//! `velouna.http` with the host check, files only in the plugin's own data
//! folder, the player only via messages to the App.

use super::host::{ButtonReg, FromPlugin, HomeReg, Level, MenuReg, Msg, PlayerCmd, Reg, Shared, SourceReg, Stats, ToPlugin, Waker};
use super::{Setting, Text, util};
use mlua::{Function, Lua, LuaOptions, MultiValue, StdLib, Table, Value, Variadic};
use serde_json::{Map, Value as Json};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// CPU time one call may use (waiting for the network does not count).
const CALL_LIMIT: Duration = Duration::from_millis(1000);
/// Loading the main file and its modules.
const LOAD_LIMIT: Duration = Duration::from_millis(3000);
const MEMORY: usize = 64 << 20;
const MAX_BODY: u64 = 16 << 20;
const MAX_STORAGE: usize = 8 << 20;
/// Stop after this many errors within a minute.
const ERROR_LIMIT: usize = 20;

pub struct Spec {
    pub id: String,
    pub version: String,
    pub dir: PathBuf,
    pub main: String,
    pub granted: Vec<String>,
    pub settings: Map<String, Json>,
    pub defs: Vec<Setting>,
    pub modules: HashMap<String, PathBuf>,
    pub data_dir: PathBuf,
}

struct Timer {
    id: u64,
    at: Instant,
    every: Option<Duration>,
}

/// The plugin's side of things, reachable from the Lua functions.
struct Ctx {
    id: String,
    dir: PathBuf,
    granted: Vec<String>,
    tx: Sender<FromPlugin>,
    wake: Waker,
    shared: Arc<Shared>,
    deadline: Arc<Mutex<Instant>>,
    /// In a fast hook: no network.
    fast: bool,
    storage: Map<String, Json>,
    storage_dirty: bool,
    data_dir: PathBuf,
    settings: Map<String, Json>,
    defs: Vec<Setting>,
    timers: Vec<Timer>,
    next_id: u64,
    stats: Arc<Mutex<Stats>>,
    modules: HashMap<String, PathBuf>,
}

impl Ctx {
    fn send(&self, msg: Msg) {
        let _ = self.tx.send(FromPlugin { id: self.id.clone(), msg });
        (self.wake)();
    }
    fn has(&self, perm: &str) -> bool {
        self.granted.iter().any(|g| g == perm)
    }
}

fn ctx(lua: &Lua) -> mlua::Result<mlua::AppDataRefMut<'_, Ctx>> {
    lua.app_data_mut::<Ctx>().ok_or_else(|| mlua::Error::runtime("plugin context missing"))
}

fn send(lua: &Lua, msg: Msg) -> mlua::Result<()> {
    ctx(lua)?.send(msg);
    Ok(())
}

fn err(s: impl Into<String>) -> mlua::Error {
    mlua::Error::runtime(s.into())
}

/// Starts the plugin thread. Fails only if the thread cannot be made.
pub fn spawn(spec: Spec, rx: Receiver<ToPlugin>, tx: Sender<FromPlugin>, shared: Arc<Shared>, stats: Arc<Mutex<Stats>>, wake: Waker) -> Result<(), String> {
    std::thread::Builder::new()
        .name(format!("plugin {}", spec.id))
        .spawn(move || run(spec, rx, tx, shared, stats, wake))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn run(spec: Spec, rx: Receiver<ToPlugin>, tx: Sender<FromPlugin>, shared: Arc<Shared>, stats: Arc<Mutex<Stats>>, wake: Waker) {
    let id = spec.id.clone();
    let say = |msg: Msg| {
        let _ = tx.send(FromPlugin { id: id.clone(), msg });
        wake();
    };
    let t0 = Instant::now();
    let lua = match setup(&spec, tx.clone(), wake.clone(), shared, stats.clone()) {
        Ok(l) => l,
        Err(e) => {
            say(Msg::Crashed(e));
            return;
        }
    };
    stats.lock().unwrap().load_ms = t0.elapsed().as_secs_f64() * 1000.0;
    say(Msg::Ready);
    let mut errors: std::collections::VecDeque<Instant> = Default::default();
    loop {
        let wait = next_timer(&lua).map(|at| at.saturating_duration_since(Instant::now())).unwrap_or(Duration::from_secs(3600));
        let msg = match rx.recv_timeout(wait) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let mut failed: Vec<String> = vec![];
        let mut stop = false;
        match msg {
            Some(ToPlugin::Stop) => stop = true,
            Some(m) => handle(&lua, m, &mut failed),
            None => {}
        }
        run_timers(&lua, &mut failed);
        flush_storage(&lua);
        for e in failed {
            stats.lock().unwrap().errors += 1;
            say(Msg::Log(Level::Error, e));
            errors.push_back(Instant::now());
        }
        while errors.front().is_some_and(|t| t.elapsed() > Duration::from_secs(60)) {
            errors.pop_front();
        }
        if errors.len() >= ERROR_LIMIT {
            say(Msg::Crashed(t!("zu viele Fehler in einer Minute").into()));
            return;
        }
        if stop {
            break;
        }
    }
    say(Msg::Stopped);
}

/// Runs `f` under the time limit and counts it.
fn guarded<R>(lua: &Lua, limit: Duration, f: impl FnOnce() -> mlua::Result<R>) -> mlua::Result<R> {
    let (deadline, stats) = {
        let c = ctx(lua)?;
        (c.deadline.clone(), c.stats.clone())
    };
    let t0 = Instant::now();
    *deadline.lock().unwrap() = t0 + limit;
    let r = f();
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let mut s = stats.lock().unwrap();
    s.calls += 1;
    s.cpu_ms += ms;
    s.max_ms = s.max_ms.max(ms);
    r
}

fn handle(lua: &Lua, m: ToPlugin, failed: &mut Vec<String>) {
    match m {
        ToPlugin::Event(name, args) => {
            let Ok(list) = callbacks(lua).and_then(|t| t.raw_get::<Option<Table>>(format!("event:{name}"))) else { return };
            let Some(list) = list else { return };
            for f in list.sequence_values::<Function>().flatten() {
                let r = guarded(lua, CALL_LIMIT, || {
                    let a = json_args(lua, &args)?;
                    f.call::<()>(a)
                });
                if let Err(e) = r {
                    failed.push(format!("{name}: {e}"));
                }
            }
        }
        ToPlugin::Hook { req, name, args, fast } => {
            let t0 = Instant::now();
            let f: Option<Function> = callbacks(lua).ok().and_then(|t| t.raw_get(format!("hook:{name}")).ok()).flatten();
            let mut value = Json::Null;
            if let Some(f) = f {
                if let Ok(mut c) = ctx(lua) {
                    c.fast = fast;
                }
                let r = guarded(lua, CALL_LIMIT, || {
                    let a = json_args(lua, &args)?;
                    let v: Value = f.call(a)?;
                    util::from_lua(&v)
                });
                if let Ok(mut c) = ctx(lua) {
                    c.fast = false;
                }
                match r {
                    Ok(v) => value = v,
                    Err(e) => failed.push(format!("hook {name}: {e}")),
                }
            }
            let _ = send(lua, Msg::HookDone { req, value, ms: t0.elapsed().as_secs_f64() * 1000.0 });
        }
        ToPlugin::Call { req, key, args } => {
            let f: Option<Function> = callbacks(lua).ok().and_then(|t| t.raw_get(key.as_str()).ok()).flatten();
            let result = match f {
                None => Err(format!("{key}: not registered")),
                Some(f) => guarded(lua, CALL_LIMIT, || {
                    let a = json_args(lua, &args)?;
                    let v: Value = f.call(a)?;
                    util::from_lua(&v)
                })
                .map_err(|e| e.to_string()),
            };
            if let Err(e) = &result {
                failed.push(e.clone());
            }
            // Dialog and timer callbacks are used once.
            if key.starts_with("dialog:") {
                if let Ok(t) = callbacks(lua) {
                    let _ = t.raw_set(key.as_str(), Value::Nil);
                }
            }
            if req != 0 {
                let _ = send(lua, Msg::CallDone { req, result });
            }
        }
        ToPlugin::Eval(code) => {
            let r = guarded(lua, CALL_LIMIT, || {
                // An expression shows its value; a statement just runs.
                let chunk = lua.load(format!("return {code}")).set_name("=console").set_mode(mlua::ChunkMode::Text);
                let f = match chunk.into_function() {
                    Ok(f) => f,
                    Err(_) => lua.load(code.as_str()).set_name("=console").set_mode(mlua::ChunkMode::Text).into_function()?,
                };
                let vals: MultiValue = f.call(())?;
                Ok(vals.iter().map(show).collect::<Vec<_>>().join("\t"))
            });
            match r {
                Ok(s) if !s.is_empty() => {
                    let _ = send(lua, Msg::Log(Level::Info, format!("= {s}")));
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = send(lua, Msg::Log(Level::Error, e.to_string()));
                }
            }
        }
        ToPlugin::Settings(v) => {
            if let Ok(mut c) = ctx(lua) {
                c.settings = v;
            }
            let m = ToPlugin::Event("settings-changed".into(), vec![]);
            handle(lua, m, failed);
        }
        ToPlugin::Message { from, msg } => {
            let m = ToPlugin::Event("message".into(), vec![Json::String(from), msg]);
            handle(lua, m, failed);
        }
        ToPlugin::Stop => {}
    }
}

fn json_args(lua: &Lua, args: &[Json]) -> mlua::Result<MultiValue> {
    let mut v = Vec::with_capacity(args.len());
    for a in args {
        v.push(util::to_lua(lua, a)?);
    }
    Ok(MultiValue::from_vec(v))
}

/// A value for the console and for print().
fn show(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::String(s) => s.to_string_lossy(),
        Value::Table(_) => util::from_lua(v).map(|j| j.to_string()).unwrap_or_else(|_| "table".into()),
        other => other.to_string().unwrap_or_else(|_| format!("{}", other.type_name())),
    }
}

fn callbacks(lua: &Lua) -> mlua::Result<Table> {
    lua.named_registry_value::<Table>("velouna.cb")
}

fn set_cb(lua: &Lua, key: &str, f: Function) -> mlua::Result<()> {
    callbacks(lua)?.raw_set(key, f)
}

fn next_timer(lua: &Lua) -> Option<Instant> {
    lua.app_data_ref::<Ctx>()?.timers.iter().map(|t| t.at).min()
}

fn run_timers(lua: &Lua, failed: &mut Vec<String>) {
    let now = Instant::now();
    let due: Vec<u64> = match lua.app_data_mut::<Ctx>() {
        Some(mut c) => {
            let due: Vec<u64> = c.timers.iter().filter(|t| t.at <= now).map(|t| t.id).collect();
            for t in c.timers.iter_mut().filter(|t| t.at <= now) {
                if let Some(e) = t.every {
                    t.at = now + e;
                }
            }
            c.timers.retain(|t| t.at > now);
            due
        }
        None => return,
    };
    for id in due {
        let key = format!("timer:{id}");
        let f: Option<Function> = callbacks(lua).ok().and_then(|t| t.raw_get(key.as_str()).ok()).flatten();
        let once = lua.app_data_ref::<Ctx>().is_some_and(|c| !c.timers.iter().any(|t| t.id == id));
        if once {
            if let Ok(t) = callbacks(lua) {
                let _ = t.raw_set(key.as_str(), Value::Nil);
            }
        }
        if let Some(f) = f {
            if let Err(e) = guarded(lua, CALL_LIMIT, || f.call::<()>(())) {
                failed.push(format!("timer: {e}"));
            }
        }
    }
}

fn flush_storage(lua: &Lua) {
    let Some(mut c) = lua.app_data_mut::<Ctx>() else { return };
    if !c.storage_dirty {
        return;
    }
    c.storage_dirty = false;
    let s = serde_json::to_vec_pretty(&Json::Object(c.storage.clone())).unwrap_or_default();
    if s.len() > MAX_STORAGE {
        c.send(Msg::Log(Level::Error, "storage: more than 8 MB, not saved".into()));
        return;
    }
    let _ = std::fs::create_dir_all(&c.data_dir);
    let tmp = c.data_dir.join("storage.json.tmp");
    if std::fs::write(&tmp, s).is_ok() {
        let _ = std::fs::rename(&tmp, c.data_dir.join("storage.json"));
    }
}

/// A path below `base` from a plugin-given relative path (no `..`, no
/// absolute paths, no drive letters).
fn below(base: &Path, rel: &str) -> mlua::Result<PathBuf> {
    let p = Path::new(rel);
    if rel.is_empty() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(err(format!("not a plain relative path: {rel}")));
    }
    Ok(base.join(p))
}

fn setup(spec: &Spec, tx: Sender<FromPlugin>, wake: Waker, shared: Arc<Shared>, stats: Arc<Mutex<Stats>>) -> Result<Lua, String> {
    let lua = Lua::new_with(StdLib::MATH | StdLib::STRING | StdLib::TABLE | StdLib::UTF8 | StdLib::COROUTINE | StdLib::OS, LuaOptions::default()).map_err(|e| e.to_string())?;
    lua.set_memory_limit(MEMORY).map_err(|e| e.to_string())?;
    let deadline = Arc::new(Mutex::new(Instant::now() + LOAD_LIMIT));
    let d = deadline.clone();
    lua.set_hook(mlua::HookTriggers::new().every_nth_instruction(10_000), move |_, _| {
        if Instant::now() > *d.lock().unwrap() {
            Err(mlua::Error::runtime("took too long (more than 1 s of computing in one call)"))
        } else {
            Ok(mlua::VmState::Continue)
        }
    });
    let storage = std::fs::read(spec.data_dir.join("storage.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Json>(&b).ok())
        .and_then(|j| j.as_object().cloned())
        .unwrap_or_default();
    lua.set_app_data(Ctx {
        id: spec.id.clone(),
        dir: spec.dir.clone(),
        granted: spec.granted.clone(),
        tx,
        wake,
        shared,
        deadline,
        fast: false,
        storage,
        storage_dirty: false,
        data_dir: spec.data_dir.clone(),
        settings: spec.settings.clone(),
        defs: spec.defs.clone(),
        timers: vec![],
        next_id: 1,
        stats,
        modules: spec.modules.clone(),
    });
    let r = (|| -> mlua::Result<()> {
        lua.set_named_registry_value("velouna.cb", lua.create_table()?)?;
        lua.set_named_registry_value("velouna.loaded", lua.create_table()?)?;
        sandbox(&lua)?;
        api(&lua, spec)?;
        let main = below(&spec.dir, &spec.main)?;
        let src = std::fs::read_to_string(&main).map_err(|e| err(format!("{}: {e}", spec.main)))?;
        lua.load(src).set_name(format!("@{}", spec.main)).set_mode(mlua::ChunkMode::Text).exec()
    })();
    r.map_err(|e| e.to_string())?;
    Ok(lua)
}

fn sandbox(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    for name in ["dofile", "loadfile", "load", "collectgarbage", "rawequal", "rawlen", "rawset", "rawget"] {
        g.raw_set(name, Value::Nil)?;
    }
    // os: only the clock and dates.
    let os_full: Table = g.raw_get("os")?;
    let os = lua.create_table()?;
    for k in ["time", "clock", "date", "difftime"] {
        os.raw_set(k, os_full.raw_get::<Value>(k)?)?;
    }
    g.raw_set("os", os)?;
    g.raw_set(
        "print",
        lua.create_function(|lua, args: Variadic<Value>| {
            let s: Vec<String> = args.iter().map(show).collect();
            send(lua, Msg::Log(Level::Info, s.join(" ")))
        })?,
    )?;
    // require: a file in the plugin ("lib.util" → lib/util.lua) or a
    // module plugin named in `requires`. Modules run with the rights of the
    // plugin that loads them.
    g.raw_set(
        "require",
        lua.create_function(|lua, name: String| {
            let loaded: Table = lua.named_registry_value("velouna.loaded")?;
            if let Some(v) = loaded.raw_get::<Option<Value>>(name.as_str())? {
                return Ok(v);
            }
            let (path, chunk) = {
                let c = ctx(lua)?;
                match c.modules.get(&name) {
                    Some(p) => (p.clone(), format!("@{name}")),
                    None => {
                        let rel = format!("{}.lua", name.replace('.', "/"));
                        (below(&c.dir, &rel)?, format!("@{rel}"))
                    }
                }
            };
            let src = std::fs::read_to_string(&path).map_err(|_| err(format!("module '{name}' not found")))?;
            let v: Value = lua.load(src).set_name(chunk).set_mode(mlua::ChunkMode::Text).call(())?;
            let v = if v.is_nil() { Value::Boolean(true) } else { v };
            loaded.raw_set(name.as_str(), v.clone())?;
            Ok(v)
        })?,
    )?;
    Ok(())
}

fn text_of(v: &Value) -> mlua::Result<Text> {
    let j = util::from_lua(v)?;
    serde_json::from_value(j).map_err(|_| err("label must be a string or { en = …, de = … }"))
}

fn strings(v: Option<Value>) -> mlua::Result<Vec<String>> {
    Ok(match v {
        None | Some(Value::Nil) => vec![],
        Some(Value::String(s)) => vec![s.to_string_lossy()],
        Some(Value::Table(t)) => t.sequence_values::<String>().collect::<mlua::Result<_>>()?,
        _ => return Err(err("expected a string or a list of strings")),
    })
}

fn id_ok(id: &str) -> mlua::Result<()> {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        return Err(err(format!("invalid id '{id}' (letters, digits, - _ . only)")));
    }
    Ok(())
}

fn api(lua: &Lua, spec: &Spec) -> mlua::Result<()> {
    let v = lua.create_table()?;
    v.raw_set("id", spec.id.as_str())?;
    v.raw_set("version", spec.version.as_str())?;
    v.raw_set("app_version", env!("CARGO_PKG_VERSION"))?;
    v.raw_set("api", super::API)?;
    v.raw_set("platform", super::this_platform())?;
    v.raw_set("language", crate::i18n::code())?;
    v.raw_set(
        "log",
        lua.create_function(|lua, args: Variadic<Value>| {
            let s: Vec<String> = args.iter().map(show).collect();
            send(lua, Msg::Log(Level::Info, s.join(" ")))
        })?,
    )?;
    v.raw_set(
        "warn",
        lua.create_function(|lua, args: Variadic<Value>| {
            let s: Vec<String> = args.iter().map(show).collect();
            send(lua, Msg::Log(Level::Warn, s.join(" ")))
        })?,
    )?;
    v.raw_set("time", lua.create_function(|_, ()| Ok(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)))?)?;
    v.raw_set(
        "uuid",
        lua.create_function(|_, ()| {
            let mut b = [0u8; 16];
            let _ = getrandom::getrandom(&mut b);
            b[6] = (b[6] & 0x0f) | 0x40;
            b[8] = (b[8] & 0x3f) | 0x80;
            let h = util::hex(&b);
            Ok(format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]))
        })?,
    )?;
    v.raw_set(
        "open_url",
        lua.create_function(|lua, url: String| {
            if !ctx(lua)?.has("open-url") {
                return Err(err("needs the permission \"open-url\""));
            }
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(err("only http(s) links can be opened"));
            }
            send(lua, Msg::OpenUrl(url))
        })?,
    )?;

    // ---------------------------------------------------------- events
    let events = lua.create_table()?;
    events.raw_set(
        "on",
        lua.create_function(|lua, (name, f): (String, Function)| {
            let cb = callbacks(lua)?;
            let key = format!("event:{name}");
            let list: Table = match cb.raw_get::<Option<Table>>(key.as_str())? {
                Some(t) => t,
                None => {
                    let t = lua.create_table()?;
                    cb.raw_set(key.as_str(), t.clone())?;
                    t
                }
            };
            list.raw_push(f)?;
            send(lua, Msg::Register(Reg::Event(name)))
        })?,
    )?;
    v.raw_set("events", events)?;

    // ---------------------------------------------------------- hooks
    let hooks = lua.create_table()?;
    hooks.raw_set(
        "on",
        lua.create_function(|lua, (name, f): (String, Function)| {
            const KNOWN: [&str; 9] = ["before-play", "play-failed", "segments", "track-info", "live-info", "display", "queue-add", "before-quit", "hls-playlist"];
            if !KNOWN.contains(&name.as_str()) {
                return Err(err(format!("unknown hook '{name}'")));
            }
            let need = match name.as_str() {
                "before-play" | "play-failed" | "queue-add" => Some("hooks:play"),
                "hls-playlist" => Some("hooks:stream"),
                _ => None,
            };
            if let Some(p) = need {
                if !ctx(lua)?.has(p) {
                    return Err(err(format!("hook '{name}' needs the permission \"{p}\"")));
                }
            }
            set_cb(lua, &format!("hook:{name}"), f)?;
            send(lua, Msg::Register(Reg::Hook(name)))
        })?,
    )?;
    v.raw_set("hooks", hooks)?;

    // ---------------------------------------------------------- player
    let player = lua.create_table()?;
    player.raw_set(
        "state",
        lua.create_function(|lua, ()| {
            let j = ctx(lua)?.shared.player.read().unwrap().clone();
            util::to_lua(lua, &j)
        })?,
    )?;
    player.raw_set(
        "current",
        lua.create_function(|lua, ()| {
            let j = ctx(lua)?.shared.current.read().unwrap().clone();
            util::to_lua(lua, &j)
        })?,
    )?;
    let cmd = |lua: &Lua, c: PlayerCmd| send(lua, Msg::Player(c));
    player.raw_set("play", lua.create_function(move |lua, ()| cmd(lua, PlayerCmd::Pause(false)))?)?;
    player.raw_set("pause", lua.create_function(move |lua, ()| cmd(lua, PlayerCmd::Pause(true)))?)?;
    player.raw_set("toggle", lua.create_function(move |lua, ()| cmd(lua, PlayerCmd::Toggle))?)?;
    player.raw_set("stop", lua.create_function(move |lua, ()| cmd(lua, PlayerCmd::Stop))?)?;
    player.raw_set("next", lua.create_function(move |lua, ()| cmd(lua, PlayerCmd::Next))?)?;
    player.raw_set("prev", lua.create_function(move |lua, ()| cmd(lua, PlayerCmd::Prev))?)?;
    player.raw_set("seek", lua.create_function(move |lua, t: f64| cmd(lua, PlayerCmd::Seek(t)))?)?;
    player.raw_set("seek_by", lua.create_function(move |lua, t: f64| cmd(lua, PlayerCmd::SeekBy(t)))?)?;
    player.raw_set("set_volume", lua.create_function(move |lua, x: f64| cmd(lua, PlayerCmd::Volume(x.clamp(0.0, 130.0))))?)?;
    player.raw_set("set_mute", lua.create_function(move |lua, x: bool| cmd(lua, PlayerCmd::Mute(x)))?)?;
    player.raw_set("set_speed", lua.create_function(move |lua, x: f64| cmd(lua, PlayerCmd::Speed(x.clamp(0.1, 8.0))))?)?;
    player.raw_set(
        "load",
        lua.create_function(|lua, (url, opts): (String, Option<Table>)| {
            check_media_url(lua, &url)?;
            let (mode, title, start) = match &opts {
                Some(o) => (o.get::<Option<String>>("mode")?.unwrap_or_else(|| "replace".into()), o.get::<Option<String>>("title")?.unwrap_or_default(), o.get::<Option<f64>>("start")?.unwrap_or(0.0)),
                None => ("replace".into(), String::new(), 0.0),
            };
            if !matches!(mode.as_str(), "replace" | "append" | "next") {
                return Err(err("mode must be replace, append or next"));
            }
            send(lua, Msg::Player(PlayerCmd::Load { url, mode, title, start }))
        })?,
    )?;
    // Subtitles: a URL or file of the plugin's own, or { text = …, format = "srt", name = … }.
    player.raw_set(
        "add_subtitle",
        lua.create_function(|lua, v: Value| {
            let (path, title) = match &v {
                Value::String(s) => {
                    let s = s.to_string_lossy();
                    if s.contains("://") {
                        check_media_url(lua, &s)?;
                        (s, String::new())
                    } else {
                        let c = ctx(lua)?;
                        (below(&c.data_dir, &s)?.to_string_lossy().to_string(), String::new())
                    }
                }
                Value::Table(t) => {
                    let text: String = t.get("text")?;
                    let format: String = t.get::<Option<String>>("format")?.unwrap_or_else(|| "srt".into());
                    if !matches!(format.as_str(), "srt" | "vtt" | "ass" | "ssa" | "lrc") {
                        return Err(err("format must be srt, vtt, ass, ssa or lrc"));
                    }
                    let name: String = t.get::<Option<String>>("name")?.unwrap_or_default();
                    let c = ctx(lua)?;
                    let dir = c.data_dir.join("subtitles");
                    let _ = std::fs::create_dir_all(&dir);
                    let mut h = 0u64;
                    for b in text.bytes() {
                        h = h.wrapping_mul(31).wrapping_add(b as u64);
                    }
                    let f = dir.join(format!("{h:016x}.{format}"));
                    std::fs::write(&f, text).map_err(|e| err(e.to_string()))?;
                    (f.to_string_lossy().to_string(), name)
                }
                _ => return Err(err("expected a URL, a file name or { text = … }")),
            };
            send(lua, Msg::Player(PlayerCmd::AddSub { path, title }))
        })?,
    )?;
    player.raw_set(
        "video_shader",
        lua.create_function(|lua, file: Option<String>| {
            let p = match file {
                None => None,
                Some(f) => {
                    let c = ctx(lua)?;
                    let p = below(&c.dir, &f)?;
                    if !p.is_file() || !matches!(p.extension().and_then(|e| e.to_str()), Some("glsl" | "hook")) {
                        return Err(err(format!("shader file not found (.glsl or .hook in the plugin): {f}")));
                    }
                    Some(p)
                }
            };
            send(lua, Msg::Player(PlayerCmd::Shader(p)))
        })?,
    )?;
    player.raw_set(
        "audio_filter",
        lua.create_function(|lua, chain: Option<String>| {
            if let Some(c) = &chain {
                check_audio_filter(c).map_err(err)?;
            }
            send(lua, Msg::Player(PlayerCmd::AudioFilter(chain)))
        })?,
    )?;
    player.raw_set(
        "add_button",
        lua.create_function(|lua, o: Table| {
            let id: String = o.get("id")?;
            id_ok(&id)?;
            let run: Function = o.get("run")?;
            let label = text_of(&o.get::<Value>("label")?)?;
            let icon: String = o.get::<Option<String>>("icon")?.unwrap_or_default();
            set_cb(lua, &format!("button:{id}"), run)?;
            send(lua, Msg::Register(Reg::Button(ButtonReg { id, label, icon })))
        })?,
    )?;
    player.raw_set("remove_button", lua.create_function(|lua, id: String| send(lua, Msg::Register(Reg::Remove("button", id))))?)?;
    v.raw_set("player", player)?;

    // ---------------------------------------------------------- media
    let media = lua.create_table()?;
    // A file next to the one playing: same name with another ending
    // ("lrc", "srt", "segments.json"), or a name in the same folder
    // ("cover.jpg" is not text; only text files up to 2 MB).
    media.raw_set(
        "sidecar",
        lua.create_function(|lua, ext: String| {
            const OK: [&str; 13] = ["lrc", "srt", "vtt", "ass", "txt", "json", "nfo", "cue", "xml", "segments.json", "chapters.txt", "edl", "md"];
            let ext = ext.trim_start_matches('.').to_ascii_lowercase();
            if !OK.contains(&ext.as_str()) {
                return Err(err(format!("sidecar files: only {}", OK.join(", "))));
            }
            let path = ctx(lua)?.shared.current.read().unwrap().get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
            if path.is_empty() || crate::media::is_url(&path) {
                return Ok(None);
            }
            let p = Path::new(&path);
            let f = p.with_extension(&ext);
            match std::fs::metadata(&f) {
                Ok(m) if m.is_file() && m.len() <= 2 << 20 => Ok(std::fs::read(&f).ok().map(|b| String::from_utf8_lossy(&b).into_owned())),
                _ => Ok(None),
            }
        })?,
    )?;
    v.raw_set("media", media)?;

    // ---------------------------------------------------------- http
    let http = lua.create_table()?;
    http.raw_set("request", lua.create_function(|lua, o: Table| http_request(lua, o))?)?;
    http.raw_set(
        "get",
        lua.create_function(|lua, (url, o): (String, Option<Table>)| {
            let o = match o {
                Some(o) => o,
                None => lua.create_table()?,
            };
            o.raw_set("url", url)?;
            o.raw_set("method", "GET")?;
            http_request(lua, o)
        })?,
    )?;
    http.raw_set(
        "post",
        lua.create_function(|lua, (url, body, o): (String, Value, Option<Table>)| {
            let o = match o {
                Some(o) => o,
                None => lua.create_table()?,
            };
            o.raw_set("url", url)?;
            o.raw_set("method", "POST")?;
            match body {
                Value::Table(t) => o.raw_set("json", t)?,
                Value::Nil => {}
                other => o.raw_set("body", other)?,
            }
            http_request(lua, o)
        })?,
    )?;
    v.raw_set("http", http)?;

    // ---------------------------------------------------------- data formats
    let json = lua.create_table()?;
    json.raw_set(
        "encode",
        lua.create_function(|_, (val, pretty): (Value, Option<bool>)| {
            let j = util::from_lua(&val)?;
            Ok(if pretty.unwrap_or(false) { serde_json::to_string_pretty(&j) } else { serde_json::to_string(&j) }.unwrap_or_default())
        })?,
    )?;
    json.raw_set(
        "decode",
        lua.create_function(|lua, s: mlua::String| match serde_json::from_slice::<Json>(&s.as_bytes()) {
            Ok(j) => Ok((util::to_lua(lua, &j)?, Value::Nil)),
            Err(e) => Ok((Value::Nil, Value::String(lua.create_string(e.to_string())?))),
        })?,
    )?;
    v.raw_set("json", json)?;
    let b64 = lua.create_table()?;
    {
        use base64::Engine;
        b64.raw_set("encode", lua.create_function(|_, s: mlua::String| Ok(base64::engine::general_purpose::STANDARD.encode(&*s.as_bytes())))?)?;
        b64.raw_set("encode_url", lua.create_function(|_, s: mlua::String| Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&*s.as_bytes())))?)?;
        b64.raw_set(
            "decode",
            lua.create_function(|lua, s: String| {
                let t = s.trim();
                let r = base64::engine::general_purpose::STANDARD
                    .decode(t)
                    .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(t.trim_end_matches('=')))
                    .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(t.trim_end_matches('=')));
                match r {
                    Ok(b) => Ok(Value::String(lua.create_string(b)?)),
                    Err(_) => Ok(Value::Nil),
                }
            })?,
        )?;
    }
    v.raw_set("base64", b64)?;
    let hash = lua.create_table()?;
    {
        use hmac::Mac;
        use sha1::Digest as _;
        hash.raw_set("md5", lua.create_function(|_, s: mlua::String| Ok(util::hex(&util::md5(&s.as_bytes()))))?)?;
        hash.raw_set("sha1", lua.create_function(|_, s: mlua::String| Ok(util::hex(&sha1::Sha1::digest(&*s.as_bytes()))))?)?;
        hash.raw_set("sha256", lua.create_function(|_, s: mlua::String| Ok(util::hex(&sha2::Sha256::digest(&*s.as_bytes()))))?)?;
        hash.raw_set(
            "hmac_sha1",
            lua.create_function(|_, (k, m): (mlua::String, mlua::String)| {
                let mut h = hmac::Hmac::<sha1::Sha1>::new_from_slice(&k.as_bytes()).map_err(|e| err(e.to_string()))?;
                h.update(&m.as_bytes());
                Ok(util::hex(&h.finalize().into_bytes()))
            })?,
        )?;
        hash.raw_set(
            "hmac_sha256",
            lua.create_function(|_, (k, m): (mlua::String, mlua::String)| {
                let mut h = hmac::Hmac::<sha2::Sha256>::new_from_slice(&k.as_bytes()).map_err(|e| err(e.to_string()))?;
                h.update(&m.as_bytes());
                Ok(util::hex(&h.finalize().into_bytes()))
            })?,
        )?;
    }
    v.raw_set("hash", hash)?;
    let url = lua.create_table()?;
    url.raw_set("encode", lua.create_function(|_, s: String| Ok(util::url_encode(&s)))?)?;
    url.raw_set("decode", lua.create_function(|_, s: String| Ok(util::url_decode(&s)))?)?;
    url.raw_set(
        "query",
        lua.create_function(|_, t: Table| {
            let mut pairs: Vec<(String, String)> = vec![];
            for p in t.pairs::<String, Value>() {
                let (k, v) = p?;
                let v = match v {
                    Value::String(s) => s.to_string_lossy(),
                    Value::Integer(i) => i.to_string(),
                    Value::Number(n) => n.to_string(),
                    Value::Boolean(b) => b.to_string(),
                    _ => continue,
                };
                pairs.push((k, v));
            }
            // Sorted, so signatures over the query are stable.
            pairs.sort();
            Ok(pairs.iter().map(|(k, v)| format!("{}={}", util::url_encode(k), util::url_encode(v))).collect::<Vec<_>>().join("&"))
        })?,
    )?;
    url.raw_set(
        "parse",
        lua.create_function(|lua, s: String| {
            let Some(u) = util::parse_url(&s) else { return Ok(Value::Nil) };
            let t = lua.create_table()?;
            t.raw_set("scheme", u.scheme)?;
            t.raw_set("host", u.host)?;
            t.raw_set("port", u.port)?;
            t.raw_set("path", u.path)?;
            t.raw_set("query", u.query.clone())?;
            let q = lua.create_table()?;
            for kv in u.query.split('&').filter(|x| !x.is_empty()) {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                q.raw_set(util::url_decode(k), util::url_decode(v))?;
            }
            t.raw_set("params", q)?;
            Ok(Value::Table(t))
        })?,
    )?;
    v.raw_set("url", url)?;
    let text = lua.create_table()?;
    text.raw_set(
        "xml",
        lua.create_function(|lua, s: String| match util::parse_xml(&s) {
            Ok(n) => Ok((Value::Table(util::xml_to_lua(lua, &n)?), Value::Nil)),
            Err(e) => Ok((Value::Nil, Value::String(lua.create_string(e)?))),
        })?,
    )?;
    text.raw_set("strip_tags", lua.create_function(|_, s: String| Ok(util::strip_tags(&s)))?)?;
    text.raw_set("unescape", lua.create_function(|_, s: String| Ok(util::unescape(&s)))?)?;
    text.raw_set("trim", lua.create_function(|_, s: String| Ok(s.trim().to_string()))?)?;
    text.raw_set(
        "split",
        lua.create_function(|lua, (s, sep): (String, String)| {
            if sep.is_empty() {
                return Err(err("separator must not be empty"));
            }
            lua.create_sequence_from(s.split(sep.as_str()).map(|x| x.to_string()))
        })?,
    )?;
    // Seconds from "1:02:03", "02:03", "03.5" or an ISO 8601 date (Unix time).
    text.raw_set(
        "time",
        lua.create_function(|_, s: String| {
            let s = s.trim();
            if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
                return Ok(Some(d.timestamp() as f64));
            }
            if let Ok(d) = chrono::DateTime::parse_from_rfc2822(s) {
                return Ok(Some(d.timestamp() as f64));
            }
            let mut total = 0.0;
            for part in s.split(':') {
                let Ok(x) = part.trim().parse::<f64>() else { return Ok(None) };
                total = total * 60.0 + x;
            }
            Ok(Some(total))
        })?,
    )?;
    v.raw_set("text", text)?;

    // ---------------------------------------------------------- storage
    let storage = lua.create_table()?;
    storage.raw_set(
        "get",
        lua.create_function(|lua, k: String| {
            let j = ctx(lua)?.storage.get(&k).cloned().unwrap_or(Json::Null);
            util::to_lua(lua, &j)
        })?,
    )?;
    storage.raw_set(
        "set",
        lua.create_function(|lua, (k, val): (String, Value)| {
            let j = util::from_lua(&val)?;
            let mut c = ctx(lua)?;
            if j.is_null() {
                c.storage.remove(&k);
            } else {
                c.storage.insert(k, j);
            }
            c.storage_dirty = true;
            Ok(())
        })?,
    )?;
    storage.raw_set(
        "remove",
        lua.create_function(|lua, k: String| {
            let mut c = ctx(lua)?;
            c.storage.remove(&k);
            c.storage_dirty = true;
            Ok(())
        })?,
    )?;
    storage.raw_set("keys", lua.create_function(|lua, ()| lua.create_sequence_from(ctx(lua)?.storage.keys().cloned().collect::<Vec<_>>()))?)?;
    v.raw_set("storage", storage)?;

    // ---------------------------------------------------------- files (own data folder)
    let files = lua.create_table()?;
    files.raw_set(
        "read",
        lua.create_function(|lua, rel: String| {
            let p = below(&ctx(lua)?.data_dir.join("files"), &rel)?;
            Ok(std::fs::read(p).ok().map(|b| lua.create_string(b)).transpose()?)
        })?,
    )?;
    files.raw_set(
        "write",
        lua.create_function(|lua, (rel, data): (String, mlua::String)| {
            let p = below(&ctx(lua)?.data_dir.join("files"), &rel)?;
            if data.as_bytes().len() > 32 << 20 {
                return Err(err("file too large (32 MB at most)"));
            }
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).map_err(|e| err(e.to_string()))?;
            }
            std::fs::write(p, &*data.as_bytes()).map_err(|e| err(e.to_string()))
        })?,
    )?;
    files.raw_set(
        "remove",
        lua.create_function(|lua, rel: String| {
            let p = below(&ctx(lua)?.data_dir.join("files"), &rel)?;
            Ok(std::fs::remove_file(p).is_ok())
        })?,
    )?;
    files.raw_set(
        "list",
        lua.create_function(|lua, rel: Option<String>| {
            let base = ctx(lua)?.data_dir.join("files");
            let dir = match rel.as_deref() {
                None | Some("") => base,
                Some(r) => below(&base, r)?,
            };
            let names: Vec<String> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
            lua.create_sequence_from(names)
        })?,
    )?;
    v.raw_set("files", files)?;

    // ---------------------------------------------------------- settings
    let settings = lua.create_table()?;
    settings.raw_set(
        "get",
        lua.create_function(|lua, k: String| {
            let c = ctx(lua)?;
            if c.defs.iter().any(|d| d.id == k && d.kind == "secret") {
                let s = crate::platform::secret::get(&super::host::secret_service(&c.id), &k);
                return Ok(match s {
                    Some(s) => Value::String(lua.create_string(s)?),
                    None => Value::Nil,
                });
            }
            let j = c.settings.get(&k).cloned().unwrap_or(Json::Null);
            drop(c);
            util::to_lua(lua, &j)
        })?,
    )?;
    // For values the plugin finds out itself (the user name after signing in).
    settings.raw_set(
        "set",
        lua.create_function(|lua, (k, val): (String, Value)| {
            let mut c = ctx(lua)?;
            if c.defs.iter().any(|d| d.id == k && d.kind == "secret") {
                let svc = super::host::secret_service(&c.id);
                match &val {
                    Value::Nil => crate::platform::secret::delete(&svc, &k),
                    v => {
                        let s = v.to_string()?;
                        if !crate::platform::secret::set(&svc, &k, &s) {
                            return Err(err("the keychain did not take the value"));
                        }
                    }
                }
                return Ok(());
            }
            let j = util::from_lua(&val)?;
            c.settings.insert(k.clone(), j.clone());
            c.send(Msg::SetSetting(k, j));
            Ok(())
        })?,
    )?;
    settings.raw_set(
        "on_action",
        lua.create_function(|lua, (name, f): (String, Function)| {
            id_ok(&name)?;
            set_cb(lua, &format!("setting:{name}"), f)
        })?,
    )?;
    v.raw_set("settings", settings)?;

    // ---------------------------------------------------------- timer
    let timer = lua.create_table()?;
    let add_timer = |lua: &Lua, secs: f64, f: Function, every: bool| -> mlua::Result<u64> {
        if !secs.is_finite() || secs < 0.0 {
            return Err(err("seconds must be 0 or more"));
        }
        let mut c = ctx(lua)?;
        if c.timers.len() >= 256 {
            return Err(err("too many timers"));
        }
        let id = c.next_id;
        c.next_id += 1;
        let d = Duration::from_secs_f64(if every { secs.max(0.1) } else { secs });
        c.timers.push(Timer { id, at: Instant::now() + d, every: every.then_some(d) });
        drop(c);
        set_cb(lua, &format!("timer:{id}"), f)?;
        Ok(id)
    };
    timer.raw_set("after", lua.create_function(move |lua, (s, f): (f64, Function)| add_timer(lua, s, f, false))?)?;
    timer.raw_set("every", lua.create_function(move |lua, (s, f): (f64, Function)| add_timer(lua, s, f, true))?)?;
    timer.raw_set(
        "cancel",
        lua.create_function(|lua, id: u64| {
            ctx(lua)?.timers.retain(|t| t.id != id);
            callbacks(lua)?.raw_set(format!("timer:{id}"), Value::Nil)
        })?,
    )?;
    v.raw_set("timer", timer)?;

    // ---------------------------------------------------------- ui
    let ui = lua.create_table()?;
    ui.raw_set("toast", lua.create_function(|lua, s: String| send(lua, Msg::Toast(s.chars().take(300).collect())))?)?;
    // A page of items: { items = { … }, view = "list" | "grid" }. Returned as it is.
    ui.raw_set("list", lua.create_function(|_, t: Table| Ok(t))?)?;
    ui.raw_set(
        "dialog",
        lua.create_function(|lua, (spec, f): (Table, Option<Function>)| {
            let key = {
                let mut c = ctx(lua)?;
                c.next_id += 1;
                format!("dialog:{}", c.next_id)
            };
            if let Some(f) = f {
                set_cb(lua, &key, f)?;
            }
            let spec = util::from_lua(&Value::Table(spec))?;
            send(lua, Msg::Dialog { key, spec })
        })?,
    )?;
    ui.raw_set(
        "progress",
        lua.create_function(|lua, (id, o): (String, Table)| {
            let text: String = o.get::<Option<String>>("text")?.unwrap_or_default();
            let value: Option<f32> = o.get("value")?;
            let done: bool = o.get::<Option<bool>>("done")?.unwrap_or(false);
            let cancel: Option<Function> = o.get("on_cancel")?;
            let has_cancel = cancel.is_some();
            if let Some(f) = cancel {
                set_cb(lua, &format!("progress:{id}"), f)?;
            }
            send(lua, Msg::Progress { id, text, value: value.map(|v| v.clamp(0.0, 1.0)), done, cancel: has_cancel })
        })?,
    )?;
    v.raw_set("ui", ui)?;

    // ---------------------------------------------------------- menu
    let menu = lua.create_table()?;
    menu.raw_set(
        "add",
        lua.create_function(|lua, o: Table| {
            let id: String = o.get("id")?;
            id_ok(&id)?;
            let run: Function = o.get("run")?;
            let label = text_of(&o.get::<Value>("label")?)?;
            let mut places = strings(o.get("where")?)?;
            if places.is_empty() {
                places.push("player".into());
            }
            for p in &places {
                if !matches!(p.as_str(), "player" | "file" | "folder" | "track" | "main" | "item") {
                    return Err(err(format!("unknown place '{p}' (player, file, folder, track, item, main)")));
                }
            }
            let kinds = strings(o.get("kinds")?)?;
            let icon: String = o.get::<Option<String>>("icon")?.unwrap_or_default();
            set_cb(lua, &format!("menu:{id}"), run)?;
            send(lua, Msg::Register(Reg::Menu(MenuReg { id, label, places, kinds, icon })))
        })?,
    )?;
    menu.raw_set("remove", lua.create_function(|lua, id: String| send(lua, Msg::Register(Reg::Remove("menu", id))))?)?;
    v.raw_set("menu", menu)?;

    // ---------------------------------------------------------- sources
    let sources = lua.create_table()?;
    sources.raw_set(
        "add",
        lua.create_function(|lua, o: Table| {
            let id: String = o.get("id")?;
            id_ok(&id)?;
            let browse: Function = o.get("browse")?;
            let label = text_of(&o.get::<Value>("label")?)?;
            let icon: String = o.get::<Option<String>>("icon")?.unwrap_or_default();
            set_cb(lua, &format!("source:{id}:browse"), browse)?;
            if let Some(f) = o.get::<Option<Function>>("resolve")? {
                set_cb(lua, &format!("source:{id}:resolve"), f)?;
            }
            let search = match o.get::<Option<Function>>("search")? {
                Some(f) => {
                    set_cb(lua, &format!("source:{id}:search"), f)?;
                    true
                }
                None => false,
            };
            send(lua, Msg::Register(Reg::Source(SourceReg { id, label, icon, search })))
        })?,
    )?;
    sources.raw_set("remove", lua.create_function(|lua, id: String| send(lua, Msg::Register(Reg::Remove("source", id))))?)?;
    v.raw_set("sources", sources)?;

    // ---------------------------------------------------------- home
    let home = lua.create_table()?;
    home.raw_set(
        "add",
        lua.create_function(|lua, o: Table| {
            let id: String = o.get("id")?;
            id_ok(&id)?;
            let items: Function = o.get("items")?;
            let label = text_of(&o.get::<Value>("label")?)?;
            set_cb(lua, &format!("home:{id}"), items)?;
            send(lua, Msg::Register(Reg::Home(HomeReg { id, label })))
        })?,
    )?;
    home.raw_set("remove", lua.create_function(|lua, id: String| send(lua, Msg::Register(Reg::Remove("home", id))))?)?;
    v.raw_set("home", home)?;

    // ---------------------------------------------------------- plugins
    let plugins = lua.create_table()?;
    plugins.raw_set(
        "send",
        lua.create_function(|lua, (to, msg): (String, Value)| {
            let msg = util::from_lua(&msg)?;
            send(lua, Msg::Send { to, msg })
        })?,
    )?;
    plugins.raw_set(
        "on_message",
        lua.create_function(|lua, f: Function| {
            let cb = callbacks(lua)?;
            let list = lua.create_table()?;
            list.raw_push(f)?;
            cb.raw_set("event:message", list)?;
            send(lua, Msg::Register(Reg::Event("message".into())))
        })?,
    )?;
    v.raw_set("plugins", plugins)?;

    lua.globals().raw_set("velouna", v)?;
    Ok(())
}

/// What a plugin may hand to mpv: http(s) on allowed hosts, nothing local
/// unless `files:read`.
fn check_media_url(lua: &Lua, url: &str) -> mlua::Result<()> {
    let c = ctx(lua)?;
    if let Some(u) = util::parse_url(url) {
        if !matches!(u.scheme.as_str(), "http" | "https" | "rtmp" | "rtmps" | "rtsp" | "srt") {
            return Err(err(format!("scheme '{}' is not allowed", u.scheme)));
        }
        if !util::host_allowed(&c.granted, &u.host) {
            return Err(err(format!("no permission for {} (add \"network:{}\")", u.host, u.host)));
        }
        return Ok(());
    }
    if url.contains("://") || (url.contains(':') && !cfg!(windows)) {
        return Err(err(format!("not a playable address: {url}")));
    }
    if !c.has("files:read") {
        return Err(err("playing local files needs the permission \"files:read\""));
    }
    Ok(())
}

/// Audio filters: a chain of known, harmless FFmpeg filters
/// ("equalizer=f=100:g=3,dynaudnorm"). No filters that read files or the
/// network (movie, amovie, sendcmd, …), no graph syntax.
pub fn check_audio_filter(chain: &str) -> Result<(), String> {
    const OK: [&str; 34] = [
        "equalizer", "bass", "treble", "lowshelf", "highshelf", "highpass", "lowpass", "bandpass", "bandreject", "allpass", "acompressor", "dynaudnorm", "loudnorm", "volume", "alimiter",
        "compand", "agate", "aecho", "chorus", "flanger", "vibrato", "tremolo", "extrastereo", "stereotools", "stereowiden", "crystalizer", "earwax", "haas", "pan", "superequalizer",
        "firequalizer", "speechnorm", "deesser", "aphaser",
    ];
    if chain.len() > 2000 || chain.contains(['[', ']', ';', '\'', '"', '\\', '\n']) {
        return Err("audio filter: plain filter chain only (no graphs, quotes or brackets)".into());
    }
    for part in chain.split(',') {
        let name = part.split('=').next().unwrap_or("").trim();
        if !OK.contains(&name) {
            return Err(format!("audio filter '{name}' is not allowed (allowed: {})", OK.join(", ")));
        }
    }
    Ok(())
}

/// `velouna.http.request{ url, method, headers, body | json, query, timeout }`
/// → response `{ status, ok, headers, body, url }`, or nil and an error.
fn http_request(lua: &Lua, o: Table) -> mlua::Result<(Value, Value)> {
    let mut url: String = o.get("url")?;
    let mut method: String = o.get::<Option<String>>("method")?.unwrap_or_else(|| "GET".into()).to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD") {
        return Err(err(format!("method {method} is not supported")));
    }
    let mut headers: Vec<(String, String)> = vec![];
    if let Some(h) = o.get::<Option<Table>>("headers")? {
        for p in h.pairs::<String, String>() {
            let (k, v) = p?;
            if k.contains(['\r', '\n', ':']) || v.contains(['\r', '\n']) {
                return Err(err("invalid header"));
            }
            headers.push((k, v));
        }
    }
    let mut body: Option<Vec<u8>> = match o.get::<Value>("body")? {
        Value::String(s) => Some(s.as_bytes().to_vec()),
        Value::Nil => None,
        _ => return Err(err("body must be a string (or use json = { … })")),
    };
    if let Some(j) = o.get::<Option<Value>>("json")? {
        body = Some(serde_json::to_vec(&util::from_lua(&j)?).unwrap_or_default());
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
            headers.push(("Content-Type".into(), "application/json".into()));
        }
    }
    if let Some(q) = o.get::<Option<Table>>("query")? {
        let mut pairs = vec![];
        for p in q.pairs::<String, Value>() {
            let (k, v) = p?;
            let v = match v {
                Value::String(s) => s.to_string_lossy(),
                Value::Nil => continue,
                other => other.to_string()?,
            };
            pairs.push(format!("{}={}", util::url_encode(&k), util::url_encode(&v)));
        }
        if !pairs.is_empty() {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&pairs.join("&"));
        }
    }
    let timeout = o.get::<Option<f64>>("timeout")?.unwrap_or(20.0).clamp(1.0, 60.0);
    let (granted, fast, deadline, stats, id) = {
        let c = ctx(lua)?;
        (c.granted.clone(), c.fast, c.deadline.clone(), c.stats.clone(), c.id.clone())
    };
    if fast {
        return Err(err("no network in this hook (fetch data earlier, e.g. in a service or in segments)"));
    }
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs_f64(timeout))
        .user_agent(&format!("Velouna/{} (plugin {id})", env!("CARGO_PKG_VERSION")))
        .build();
    let started = Instant::now();
    let result = (|| -> mlua::Result<Result<(u16, Vec<(String, String)>, Vec<u8>, String), String>> {
        for _hop in 0..=5 {
            let Some(u) = util::parse_url(&url) else { return Err(err(format!("not a valid URL: {url}"))) };
            if u.scheme != "http" && u.scheme != "https" {
                return Err(err("only http and https"));
            }
            if !util::host_allowed(&granted, &u.host) {
                return Err(err(format!("no permission for {} (add \"network:{}\" to permissions)", u.host, u.host)));
            }
            let mut req = agent.request(&method, &url);
            for (k, v) in &headers {
                req = req.set(k, v);
            }
            let r = match &body {
                Some(b) => req.send_bytes(b),
                None => req.call(),
            };
            let resp = match r {
                Ok(r) => r,
                Err(ureq::Error::Status(_, r)) => r,
                Err(e) => return Ok(Err(e.to_string())),
            };
            let status = resp.status();
            if (300..400).contains(&status) {
                if let Some(loc) = resp.header("location") {
                    url = join_url(&url, &u, loc);
                    if status == 303 || ((status == 301 || status == 302) && method == "POST") {
                        method = "GET".into();
                        body = None;
                    }
                    continue;
                }
            }
            let hs: Vec<(String, String)> = resp.headers_names().into_iter().filter_map(|n| resp.header(&n).map(|v| (n.to_ascii_lowercase(), v.to_string()))).collect();
            let mut buf = Vec::new();
            use std::io::Read;
            if let Err(e) = resp.into_reader().take(MAX_BODY + 1).read_to_end(&mut buf) {
                return Ok(Err(e.to_string()));
            }
            if buf.len() as u64 > MAX_BODY {
                return Ok(Err("response larger than 16 MB".into()));
            }
            return Ok(Ok((status, hs, buf, url.clone())));
        }
        Ok(Err("too many redirects".into()))
    })();
    // Waiting for the network does not count against the time limit.
    *deadline.lock().unwrap() += started.elapsed();
    let r = result?;
    {
        let mut s = stats.lock().unwrap();
        s.net += 1;
        if let Ok((_, _, b, _)) = &r {
            s.net_bytes += b.len() as u64;
        }
    }
    match r {
        Err(e) => Ok((Value::Nil, Value::String(lua.create_string(e)?))),
        Ok((status, hs, b, final_url)) => {
            let t = lua.create_table()?;
            t.raw_set("status", status)?;
            t.raw_set("ok", (200..300).contains(&status))?;
            let h = lua.create_table()?;
            for (k, v) in hs {
                h.raw_set(k, v)?;
            }
            t.raw_set("headers", h)?;
            t.raw_set("body", lua.create_string(b)?)?;
            t.raw_set("url", final_url)?;
            Ok((Value::Table(t), Value::Nil))
        }
    }
}

fn join_url(base: &str, u: &util::Url, loc: &str) -> String {
    if loc.contains("://") {
        return loc.to_string();
    }
    let origin = match u.port {
        Some(p) => format!("{}://{}:{p}", u.scheme, u.host),
        None => format!("{}://{}", u.scheme, u.host),
    };
    if let Some(rest) = loc.strip_prefix("//") {
        return format!("{}://{rest}", u.scheme);
    }
    if loc.starts_with('/') {
        return format!("{origin}{loc}");
    }
    let _ = base;
    let dir = u.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    format!("{origin}{dir}/{loc}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(main: &str, perms: &[&str]) -> (Lua, std::sync::mpsc::Receiver<FromPlugin>) {
        let dir = std::env::temp_dir().join(format!("velouna-rt-{}-{}", std::process::id(), main.len()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("main.lua"), main).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let spec = Spec {
            id: "dev.test.p".into(),
            version: "1.0.0".into(),
            dir: dir.clone(),
            main: "main.lua".into(),
            granted: perms.iter().map(|s| s.to_string()).collect(),
            settings: Map::new(),
            defs: vec![],
            modules: HashMap::new(),
            data_dir: dir.join("data"),
        };
        let lua = setup(&spec, tx, Arc::new(|| {}), Arc::new(Shared::default()), Arc::new(Mutex::new(Stats::default()))).unwrap();
        (lua, rx)
    }

    #[test]
    fn sandbox_is_closed() {
        let (lua, _rx) = plugin("x = 1", &[]);
        for code in ["return io", "return os.execute", "return os.remove", "return load", "return dofile", "return debug", "return package", "return os.getenv"] {
            let v: Value = lua.load(code).eval().unwrap();
            assert!(v.is_nil(), "{code} must be nil");
        }
        let t: f64 = lua.load("return os.time()").eval().unwrap();
        assert!(t > 0.0);
    }

    #[test]
    fn no_network_without_permission() {
        let (lua, _rx) = plugin("", &[]);
        let r: mlua::Result<Value> = lua.load(r#"return velouna.http.get("https://example.com/")"#).eval();
        let e = r.unwrap_err().to_string();
        assert!(e.contains("no permission for example.com"), "{e}");
        let (lua, _rx) = plugin("", &["network:api.example.com"]);
        let r: mlua::Result<Value> = lua.load(r#"return velouna.http.get("https://example.com/")"#).eval();
        assert!(r.is_err());
        let r: mlua::Result<Value> = lua.load(r#"return velouna.player.load("https://evil.example/x.mp3")"#).eval();
        assert!(r.is_err(), "mpv must not be a way around the host check");
        let r: mlua::Result<Value> = lua.load(r#"return velouna.open_url("https://example.com")"#).eval();
        assert!(r.is_err());
    }

    #[test]
    fn events_hooks_and_storage() {
        let (lua, rx) = plugin(
            r#"
            local n = 0
            velouna.events.on("track-started", function(t) n = n + 1; velouna.storage.set("last", t.title) end)
            velouna.hooks.on("segments", function(item) return { { from = 1, to = 2, action = "skip" } } end)
            function count() return n end
            "#,
            &[],
        );
        let mut failed = vec![];
        handle(&lua, ToPlugin::Event("track-started".into(), vec![serde_json::json!({"title": "A"})]), &mut failed);
        assert!(failed.is_empty(), "{failed:?}");
        let n: i64 = lua.load("return count()").eval().unwrap();
        assert_eq!(n, 1);
        let s: String = lua.load(r#"return velouna.storage.get("last")"#).eval().unwrap();
        assert_eq!(s, "A");
        handle(&lua, ToPlugin::Hook { req: 7, name: "segments".into(), args: vec![serde_json::json!({})], fast: false }, &mut failed);
        let mut got = None;
        while let Ok(m) = rx.try_recv() {
            if let Msg::HookDone { req, value, .. } = m.msg {
                got = Some((req, value));
            }
        }
        let (req, value) = got.unwrap();
        assert_eq!(req, 7);
        assert_eq!(value[0]["action"], "skip");
    }

    #[test]
    fn endless_loop_stops() {
        let (lua, _rx) = plugin("function spin() while true do end end", &[]);
        let f: Function = lua.globals().get("spin").unwrap();
        let r = guarded(&lua, Duration::from_millis(50), || f.call::<()>(()));
        assert!(r.unwrap_err().to_string().contains("took too long"));
    }

    #[test]
    fn hooks_need_permission() {
        let (lua, _rx) = plugin("", &[]);
        let r: mlua::Result<()> = lua.load(r#"velouna.hooks.on("before-play", function() end)"#).exec();
        assert!(r.is_err());
        assert!(check_audio_filter("equalizer=f=100:g=3,dynaudnorm").is_ok());
        assert!(check_audio_filter("amovie=/etc/passwd").is_err());
        assert!(check_audio_filter("volume=2[a];[a]anull").is_err());
    }
}
