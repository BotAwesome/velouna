//! Lua plugins in the App (plugin API 2): starting them, player events,
//! hooks, player commands, segments, the plugins' dialogs, toasts and
//! buttons, their settings and console, and plugin sources in the library.

use crate::app::{App, View};
use crate::library::{ExtLoc, LibLoc};
use crate::plugins::host::{self, Chain, ChainKind, FromPlugin, Level, Msg, PageKey, PageState, Pending, PlayerCmd, PluginDialog, Progress, ToPlugin};
use crate::plugins::{Kind, util};
use crate::{media, mpv, theme, widgets};
use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Order, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as ph;
use serde_json::{Map, Value as Json, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// App-side state of the plugin integration.
#[derive(Default)]
pub struct ExtUi {
    pub console_input: String,
    /// Secret settings being typed: (plugin, setting) → text.
    pub secret_buf: HashMap<(String, String), String>,
    snapshot_at: Option<Instant>,
    last_paused: Option<bool>,
    last_progress: Option<Instant>,
    last_tick: Option<Instant>,
    seek_pending: bool,
    queue_ptr: usize,
    queue_changed_at: Option<Instant>,
    /// The item playing now, as plugins see it.
    pub current: Json,
    played: f64,
    live_title: String,
    /// A `live-info` hook muted or lowered the sound: what to restore.
    live_restore: Option<(bool, f64)>,
    /// Media loaded from plugin sources: URL → (plugin, item).
    pub items: HashMap<String, (String, Json)>,
    /// A `before-play` hook cancelled playback, with this reason.
    pub cancel_reason: Option<String>,
    /// Search text on a plugin source page.
    pub search: String,
    /// Plugins just started while a file plays: tell them about it once
    /// they have loaded (so their hooks see it too).
    initial: Option<Instant>,
    /// A plugin dialog was opened from the settings: go back there after it.
    pub back_to_settings: bool,
}

const BEFORE_PLAY_BUDGET: Duration = Duration::from_millis(30);
const HOOK_BUDGET: Duration = Duration::from_millis(50);
const FAILED_BUDGET: Duration = Duration::from_secs(3);

/// Phosphor glyph for an icon name used by plugins ("heart", "star" …).
pub fn icon_glyph(name: &str) -> &'static str {
    match name {
        "heart" => ph::HEART,
        "star" => ph::STAR,
        "bookmark" => ph::BOOKMARK_SIMPLE,
        "share" => ph::SHARE_NETWORK,
        "music" | "music-note" => ph::MUSIC_NOTE,
        "microphone" | "podcast" => ph::MICROPHONE,
        "moon" => ph::MOON,
        "sparkle" => ph::SPARKLE,
        "lightning" => ph::LIGHTNING,
        "chat" => ph::CHAT_CIRCLE,
        "info" => ph::INFO,
        "download" => ph::DOWNLOAD_SIMPLE,
        "plus" => ph::PLUS,
        "check" => ph::CHECK,
        "flag" => ph::FLAG,
        "bell" => ph::BELL,
        "globe" => ph::GLOBE,
        "thumbs-up" => ph::THUMBS_UP,
        "thumbs-down" => ph::THUMBS_DOWN,
        "list" => ph::LIST,
        "eye" => ph::EYE,
        "timer" => ph::TIMER,
        "rss" => ph::RSS,
        "radio" => ph::RADIO,
        "tv" | "television" => ph::TELEVISION,
        "film" => ph::FILM_STRIP,
        "folder" => ph::FOLDER_SIMPLE,
        "cloud" => ph::CLOUD,
        "book" => ph::BOOK_OPEN,
        "game" => ph::GAME_CONTROLLER,
        "skip" => ph::SKIP_FORWARD,
        "subtitles" => ph::SUBTITLES,
        "lyrics" | "text" => ph::TEXT_ALIGN_LEFT,
        "chart" => ph::CHART_BAR,
        "user" => ph::USER,
        "house" | "home" => ph::HOUSE,
        "magic" => ph::MAGIC_WAND,
        "palette" => ph::PALETTE,
        "search" => ph::MAGNIFYING_GLASS,
        _ => ph::PUZZLE_PIECE,
    }
}

fn kind_glyph(kind: &str, playable: bool) -> &'static str {
    match kind {
        "folder" | "album" | "artist" => ph::FOLDER_SIMPLE,
        "audio" | "track" => ph::MUSIC_NOTE,
        "episode" | "podcast" => ph::MICROPHONE,
        "stream" | "live" => ph::BROADCAST,
        "picture" => ph::IMAGE,
        "movie" | "video" => ph::FILM_STRIP,
        _ if playable => ph::PLAY_CIRCLE,
        _ => ph::FOLDER_SIMPLE,
    }
}

fn s<'a>(v: &'a Json, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

/// Items of a page: a list, or `{ items = …, view = … }` (velouna.ui.list).
fn page_items(v: &Json) -> (Vec<Json>, &str) {
    match v {
        Json::Array(a) => (a.clone(), "list"),
        Json::Object(o) => (o.get("items").and_then(|x| x.as_array()).cloned().unwrap_or_default(), o.get("view").and_then(|x| x.as_str()).unwrap_or("list")),
        _ => (vec![], "list"),
    }
}

fn playable(item: &Json) -> bool {
    item.get("playable").and_then(|x| x.as_bool()).unwrap_or_else(|| item.get("url").is_some())
}

impl App {
    // ------------------------------------------------------------ lifecycle

    /// Once per frame, after mpv's events.
    pub fn ext_tick(&mut self, ctx: &egui::Context) {
        if self.ext.waker.is_none() {
            let c = ctx.clone();
            self.ext.waker = Some(Arc::new(move || c.request_repaint()));
        }
        if self.ext.disabled {
            return;
        }
        // Plugins start once the window is up and the first file (if any)
        // had its half second: the start stays as fast as without them.
        if !self.ext.started && ctx.input(|i| i.time) > 1.5 {
            self.ext.started = true;
            self.ext_start_where(|m| m.starts_at_idle());
            if !self.ext.running.is_empty() {
                self.ext.emit("app-idle", vec![]);
                self.ext_ui.initial = Some(Instant::now());
            }
        }
        for m in self.ext.drain() {
            self.ext_apply(m);
        }
        // The file that was already playing when the plugins started.
        if let Some(at) = self.ext_ui.initial {
            if self.ext.all_ready() || at.elapsed() > Duration::from_secs(2) {
                self.ext_ui.initial = None;
                if self.has_media() && self.ext_ui.current.is_null() {
                    self.ext_file_loaded();
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(20));
            }
        }
        for c in self.ext.advance_chains() {
            self.ext_chain_done(c);
        }
        if self.ext.running.is_empty() {
            return;
        }
        self.ext_mpv_hooks();
        self.ext_watch_player();
        self.ext_segments_tick(ctx);
    }

    /// Starts the enabled Lua plugins that `want` picks.
    pub fn ext_start_where(&mut self, want: impl Fn(&crate::plugins::Manifest) -> bool) {
        self.plugins.ensure();
        let mut errors = vec![];
        for p in &self.plugins.list {
            let m = &p.manifest;
            if m.kind != Kind::Lua || m.is_module() || p.error.is_some() || self.ext.is_running(&m.id) || !want(m) {
                continue;
            }
            let Some(st) = self.data.plugins.get(&m.id).filter(|s| s.enabled) else { continue };
            if let Err(e) = self.ext.start(p, st, &self.plugins) {
                errors.push((m.id.clone(), e));
            }
        }
        for (id, e) in errors {
            self.ext.log(&id, Level::Error, e.clone());
            self.ext.errors.insert(id, e);
        }
    }

    /// Starts one plugin now (switched on, installed, restarted).
    pub fn ext_start(&mut self, id: &str) {
        if self.ext.disabled {
            return;
        }
        self.ext.started = true;
        let id = id.to_string();
        self.ext_start_where(|m| m.id == id);
        if self.has_media() && self.ext.is_running(&id) {
            // Bring it up to date with what is playing.
            let item = self.ext_ui.current.clone();
            if !item.is_null() {
                if let Some(r) = self.ext.get_mut(&id) {
                    r.send(ToPlugin::Event("track-started".into(), vec![item]));
                }
            }
        }
    }

    pub fn ext_stop(&mut self, id: &str) {
        let Some(r) = self.ext.stop(id) else { return };
        if let (Some(sh), Some(p)) = (&r.shader, &self.player) {
            let _ = p.mpv.command(&["change-list", "glsl-shaders", "remove", &sh.to_string_lossy()]);
        }
        if r.filter.is_some() {
            self.apply_audio_filters();
        }
    }

    pub fn ext_restart(&mut self, id: &str) {
        self.ext_stop(id);
        self.ext_start(id);
    }

    /// After "Reload": stop all, read the folders again, start again.
    pub fn ext_restart_all(&mut self) {
        let ids: Vec<String> = self.ext.running.iter().map(|r| r.id.clone()).collect();
        for id in &ids {
            self.ext_stop(id);
        }
        self.plugins.rescan();
        self.ext.errors.clear();
        if self.ext.started {
            self.ext_start_where(|m| m.starts_at_idle() || ids.contains(&m.id));
        }
    }

    pub fn ext_quit(&mut self) {
        self.ext.shutdown(Duration::from_secs(1));
    }

    /// Sends an event; plugins that start on it (`on:<event>`) start first.
    pub fn ext_emit(&mut self, name: &str, args: Vec<Json>) {
        if self.ext.disabled || !self.ext.started {
            return;
        }
        let lazy = self.plugins.list.iter().any(|p| p.manifest.starts_on(name) && !self.ext.is_running(&p.manifest.id) && self.data.plugins.get(&p.manifest.id).is_some_and(|s| s.enabled));
        if lazy {
            let n = name.to_string();
            self.ext_start_where(|m| m.starts_on(&n));
        }
        self.ext.emit(name, args);
    }

    // ------------------------------------------------------------ messages

    fn ext_apply(&mut self, m: FromPlugin) {
        let id = m.id;
        match m.msg {
            Msg::Toast(s) => self.ext.toast(&id, s, 4.0, None),
            Msg::OpenUrl(u) => crate::platform::open_url(&u),
            Msg::Player(c) => self.ext_player(&id, c),
            Msg::CallDone { req, result } => self.ext_call_done(req, result),
            Msg::Dialog { key, spec } => {
                let mut values = Map::new();
                for f in spec.get("fields").and_then(|x| x.as_array()).into_iter().flatten() {
                    let fid = s(f, "id").to_string();
                    if fid.is_empty() {
                        continue;
                    }
                    let d = f.get("default").cloned().unwrap_or(match s(f, "type") {
                        "toggle" => json!(false),
                        "number" => json!(0),
                        _ => json!(""),
                    });
                    values.insert(fid, d);
                }
                if self.ext.dialogs.len() < 8 {
                    self.ext.dialogs.push_back(PluginDialog { plugin: id, key, spec, values });
                }
            }
            Msg::Progress { id: pid, text, value, done, cancel } => {
                self.ext.progress.retain(|p| !(p.plugin == id && p.id == pid));
                if !done {
                    self.ext.progress.push(Progress { plugin: id, id: pid, text, value, cancel });
                }
            }
            Msg::Send { to, msg } => {
                if let Some(r) = self.ext.get_mut(&to) {
                    if r.events.contains("message") {
                        r.send(ToPlugin::Message { from: id, msg });
                    }
                }
            }
            Msg::SetSetting(k, v) => {
                self.data.plugins.entry(id).or_default().settings.insert(k, v);
                self.mark_dirty();
            }
            Msg::Crashed(e) => {
                let name = self.plugins.get(&id).map(|p| p.manifest.name.get()).unwrap_or_else(|| id.clone());
                self.ext_stop(&id);
                self.ext.errors.insert(id, e.clone());
                self.notice(tf!("Plugin {} angehalten: {}", name, e));
            }
            _ => {}
        }
    }

    fn ext_call_done(&mut self, req: u64, result: Result<Json, String>) {
        let Some((plugin, purpose)) = self.ext.calls.remove(&req) else { return };
        match purpose {
            Pending::Nothing | Pending::Eval => {}
            Pending::Browse { key } | Pending::Search { key } => {
                let st = match result {
                    Ok(v) => PageState::Ready(v),
                    Err(e) => PageState::Failed(e),
                };
                self.ext.pages.insert(key, st);
            }
            Pending::Resolve { item } => match result {
                Ok(v) => self.ext_play_resolved(&plugin, item, v),
                Err(e) => self.notice(tf!("Kann nicht abgespielt werden: {}", e)),
            },
            Pending::Home { plugin, id } => {
                let st = match result {
                    Ok(v) => PageState::Ready(v),
                    Err(e) => PageState::Failed(e),
                };
                self.ext.homes.insert((plugin, id), st);
            }
        }
    }

    fn ext_player(&mut self, id: &str, c: PlayerCmd) {
        let Some(p) = &self.player else { return };
        let ctl = p.ctl.clone();
        match c {
            PlayerCmd::Pause(b) => ctl.set_pause(b),
            PlayerCmd::Toggle => ctl.toggle_pause(),
            PlayerCmd::Stop => ctl.stop(),
            PlayerCmd::Next => ctl.next(),
            PlayerCmd::Prev => p.prev(),
            PlayerCmd::Seek(t) => ctl.seek_abs(t, true),
            PlayerCmd::SeekBy(d) => ctl.seek_rel(d),
            PlayerCmd::Volume(v) => ctl.set_volume(v),
            PlayerCmd::Mute(b) => ctl.set_mute(b),
            PlayerCmd::Speed(s) => ctl.set_speed(s),
            PlayerCmd::Load { url, mode, title, start } => {
                self.ext_ui.items.insert(url.clone(), (id.to_string(), json!({ "title": title })));
                self.ext_load(&url, &mode, &title, start, "");
            }
            PlayerCmd::AddSub { path, title } => {
                let mut args = vec!["sub-add", path.as_str(), "select"];
                if !title.is_empty() {
                    args.push(&title);
                }
                let _ = ctl.mpv.command(&args);
            }
            PlayerCmd::Shader(sh) => {
                let old = self.ext.get_mut(id).and_then(|r| r.shader.take());
                if let Some(o) = old {
                    let _ = ctl.mpv.command(&["change-list", "glsl-shaders", "remove", &o.to_string_lossy()]);
                }
                if let Some(n) = &sh {
                    let _ = ctl.mpv.command(&["change-list", "glsl-shaders", "append", &n.to_string_lossy()]);
                }
                if let Some(r) = self.ext.get_mut(id) {
                    r.shader = sh;
                }
            }
            PlayerCmd::AudioFilter(f) => {
                if let Some(r) = self.ext.get_mut(id) {
                    r.filter = f;
                }
                self.apply_audio_filters();
            }
        }
    }

    /// Filters of running plugins, for `apply_audio_filters`.
    pub fn ext_audio_filters(&self) -> Vec<String> {
        self.ext.running.iter().filter_map(|r| r.filter.clone()).collect()
    }

    /// Plays a URL from a plugin. `headers`: "Name: value" lines joined by ",".
    fn ext_load(&mut self, url: &str, mode: &str, title: &str, start: f64, headers: &str) {
        let Some(p) = &self.player else { return };
        let mut opts = vec![];
        // %n% quotes a value of n bytes (commas in titles).
        if !title.is_empty() {
            opts.push(format!("force-media-title=%{}%{}", title.len(), title));
        }
        if start > 0.0 {
            opts.push(format!("start={start:.3}"));
        }
        if !headers.is_empty() {
            opts.push(format!("http-header-fields=%{}%{}", headers.len(), headers));
        }
        let idle = !p.st.has_media();
        let flag = match mode {
            "append" => "append-play",
            "next" if !idle => "insert-next",
            "next" => "append-play",
            _ => "replace",
        };
        let joined = opts.join(",");
        if flag == "replace" || idle {
            // A file that ended paused (keep-open) would leave the new one paused.
            p.set_pause(false);
        }
        let r = if opts.is_empty() { p.mpv.command(&["loadfile", url, flag]) } else { p.mpv.command(&["loadfile", url, flag, "-1", &joined]) };
        if let Err(e) = r {
            self.notice(tf!("Kann nicht abgespielt werden: {}", e));
        }
    }

    // ------------------------------------------------------------ player events

    /// mpv events, before the App's own handling.
    pub fn ext_mpv_event(&mut self, e: &mpv::Event) {
        match e {
            mpv::Event::Hook { name, id } => self.ext_mpv_hook(name, *id),
            _ if !self.ext.started || self.ext.running.is_empty() => {}
            mpv::Event::FileLoaded => self.ext_file_loaded(),
            mpv::Event::EndFile { reason, .. } => self.ext_track_ended(*reason),
            mpv::Event::Seek => self.ext_ui.seek_pending = true,
            mpv::Event::PlaybackRestart => {
                if std::mem::take(&mut self.ext_ui.seek_pending) && !self.ext_ui.current.is_null() {
                    let t = self.player.as_ref().map(|p| p.st.time).unwrap_or(0.0);
                    let item = self.ext_ui.current.clone();
                    self.ext_emit("seeked", vec![item, json!(t)]);
                }
            }
            _ => {}
        }
    }

    /// The item for the file mpv just loaded.
    fn ext_item_now(&self) -> Option<(Json, String)> {
        let p = self.player.as_ref()?;
        let path = p.mpv.get_string("path")?;
        let get = |k: &str| p.mpv.get_string(k).filter(|s| !s.is_empty());
        let title = get("media-title").unwrap_or_else(|| media::display_name(&path));
        let artist = get("metadata/by-key/artist");
        let album = get("metadata/by-key/album");
        let duration = get("duration").and_then(|d| d.parse::<f64>().ok()).unwrap_or(0.0);
        let vid = get("vid");
        let audio = vid.as_deref().is_none_or(|v| v == "no") || get("current-tracks/video/albumart").as_deref() == Some("yes") || get("current-tracks/video/image").as_deref() == Some("yes");
        if let Some(r) = crate::plex::remote(&path) {
            // A media server: title and kind, not its address.
            let mut item = host::item_json("", &title, artist.as_deref(), album.as_deref(), duration.max(r.duration), audio, &r.conn.backend.name().to_lowercase());
            item["title"] = json!(if r.name.is_empty() { r.title.clone() } else { r.name.clone() });
            return Some((item, path));
        }
        let source = if self.ext_ui.items.contains_key(&path) {
            "plugin"
        } else if self.radio_urls.contains(&path) {
            "radio"
        } else if self.live_urls.contains(&path) {
            "tv"
        } else if media::is_url(&path) {
            "stream"
        } else {
            "file"
        };
        let mut item = host::item_json(&path, &title, artist.as_deref(), album.as_deref(), duration, audio, source);
        if let Some((plugin, it)) = self.ext_ui.items.get(&path) {
            item["plugin"] = json!(plugin);
            if let Some(i) = it.get("id") {
                item["id"] = i.clone();
            }
            item["item"] = it.clone();
        }
        Some((item, path))
    }

    pub fn ext_file_loaded(&mut self) {
        let Some((item, path)) = self.ext_item_now() else { return };
        self.ext_ui.current = item.clone();
        self.ext_ui.played = 0.0;
        self.ext_ui.last_paused = None;
        self.ext_ui.live_title.clear();
        *self.ext.shared.current.write().unwrap() = item.clone();
        if self.ext.segments.path != path {
            self.ext.segments = Default::default();
        }
        if self.ext.has_hook("track-info") {
            self.ext.start_chain("track-info", ChainKind::TrackInfo, item, &path, HOOK_BUDGET);
            for c in self.ext.advance_chains() {
                self.ext_chain_done(c);
            }
        } else {
            self.ext_track_started(&path);
        }
    }

    fn ext_track_started(&mut self, path: &str) {
        let item = self.ext_ui.current.clone();
        self.ext_emit("track-started", vec![item.clone()]);
        if self.ext.has_hook("segments") {
            self.ext.request_segments(path, &item);
        }
    }

    fn ext_track_ended(&mut self, reason: i32) {
        if self.ext_ui.current.is_null() {
            return;
        }
        let item = std::mem::take(&mut self.ext_ui.current);
        let pos = self.player.as_ref().map(|p| p.st.time).unwrap_or(0.0);
        let why = match reason {
            mpv::END_EOF => "eof",
            mpv::END_STOP => "stop",
            mpv::END_ERROR => "error",
            _ => "other",
        };
        let played = self.ext_ui.played;
        self.ext_emit("track-ended", vec![item, json!({ "reason": why, "position": pos, "played": played })]);
        *self.ext.shared.current.write().unwrap() = Json::Null;
        self.ext_live_restore();
        if let (Some(prev), Some(p)) = (self.ext.segments.muted.take(), &self.player) {
            p.set_mute(prev);
        }
    }

    /// Adds mpv's load hooks once a plugin wants them (never before: the
    /// first file plays without waiting for anything).
    fn ext_mpv_hooks(&mut self) {
        let Some(p) = &self.player else { return };
        for (hook, mpv_name) in [("before-play", "on_load"), ("play-failed", "on_load_fail")] {
            if self.ext.has_hook(hook) && !self.ext.mpv_hooks.contains(mpv_name) {
                if !p.mpv.hook_add(mpv_name, 50) {
                    log::warn!("plugins: this libmpv has no hooks; {hook} does not run");
                }
                self.ext.mpv_hooks.insert(mpv_name.to_string());
            }
        }
    }

    fn ext_mpv_hook(&mut self, name: &str, id: u64) {
        let Some(p) = &self.player else { return };
        let (hook, budget) = match name {
            "on_load" => ("before-play", BEFORE_PLAY_BUDGET),
            "on_load_fail" => ("play-failed", FAILED_BUDGET),
            _ => {
                p.mpv.hook_continue(id);
                return;
            }
        };
        let url = p.mpv.get_string("stream-open-filename").or_else(|| p.mpv.get_string("path")).unwrap_or_default();
        // Media servers stay private: their streams carry sign-in data.
        if !self.ext.has_hook(hook) || url.is_empty() || crate::plex::is_remote(&url) {
            p.mpv.hook_continue(id);
            return;
        }
        let mut item = json!({ "url": url, "path": url });
        if let Some((plugin, it)) = self.ext_ui.items.get(&url) {
            item["plugin"] = json!(plugin);
            item["item"] = it.clone();
        }
        if let Some(y) = util::youtube_id(&url) {
            item["youtube_id"] = json!(y);
        }
        if hook == "play-failed" {
            item["error"] = json!(p.mpv.get_string("error").unwrap_or_default());
        }
        self.ext.start_chain(hook, ChainKind::Mpv(id), item, &url, budget);
        for c in self.ext.advance_chains() {
            self.ext_chain_done(c);
        }
    }

    /// May a hook's new URL be played? Only on hosts one of the changing
    /// plugins may reach.
    fn ext_url_ok(&self, plugins: &[String], url: &str) -> bool {
        let Some(u) = util::parse_url(url) else { return plugins.iter().any(|p| self.ext.get(p).is_some_and(|r| r.granted.iter().any(|g| g == "files:read"))) };
        matches!(u.scheme.as_str(), "http" | "https") && plugins.iter().any(|p| self.ext.get(p).is_some_and(|r| util::host_allowed(&r.granted, &u.host)))
    }

    fn ext_chain_done(&mut self, c: Chain) {
        match c.kind {
            ChainKind::Mpv(id) => {
                let Some(p) = &self.player else { return };
                let v = &c.value;
                if !c.changed_by.is_empty() {
                    if let Some(reason) = v.get("cancel").and_then(|x| x.as_str()).filter(|_| c.name == "before-play") {
                        self.ext_ui.cancel_reason = Some(reason.chars().take(200).collect());
                        let _ = p.mpv.set_str("stream-open-filename", "velouna-cancelled://");
                    } else {
                        let url = s(v, "url");
                        if !url.is_empty() && url != c.path {
                            if self.ext_url_ok(&c.changed_by, url) {
                                let _ = p.mpv.set_str("stream-open-filename", url);
                            } else {
                                self.ext.log(&c.changed_by.join(", "), Level::Error, format!("{}: no permission for {url}", c.name));
                            }
                        }
                        if let Some(h) = v.get("headers").and_then(|x| x.as_object()) {
                            let lines: Vec<String> = h.iter().filter_map(|(k, x)| x.as_str().map(|x| format!("{k}: {x}"))).filter(|l| !l.contains(['\r', '\n'])).collect();
                            let _ = p.mpv.set_str("file-local-options/http-header-fields", &lines.join(","));
                        }
                        if c.name == "before-play" {
                            if let Some(st) = v.get("start").and_then(|x| x.as_f64()) {
                                let _ = p.mpv.set_str("file-local-options/start", &format!("{st:.3}"));
                            }
                            for (k, opt) in [("title", "force-media-title"), ("audio_lang", "alang"), ("sub_lang", "slang"), ("ytdl_format", "ytdl-format")] {
                                let x = s(v, k);
                                if !x.is_empty() {
                                    let _ = p.mpv.set_str(&format!("file-local-options/{opt}"), x);
                                }
                            }
                        }
                        if let Some(m) = v.get("message").and_then(|x| x.as_str()) {
                            self.ext_ui.cancel_reason = Some(m.chars().take(200).collect());
                        }
                    }
                    log::info!("plugins: {} changed by {}", c.name, c.changed_by.join(", "));
                }
                if let Some(p) = &self.player {
                    p.mpv.hook_continue(id);
                }
            }
            ChainKind::TrackInfo => {
                let cur_path = self.player.as_ref().map(|p| p.st.path.clone()).unwrap_or_default();
                if c.path != cur_path && !self.player.as_ref().is_some_and(|p| p.mpv.get_string("path").as_deref() == Some(c.path.as_str())) {
                    return;
                }
                if !c.changed_by.is_empty() {
                    let old_title = s(&self.ext_ui.current, "title").to_string();
                    if let (Json::Object(cur), Json::Object(new)) = (&mut self.ext_ui.current, &c.value) {
                        for k in ["title", "artist", "album"] {
                            if let Some(x) = new.get(k).filter(|x| x.is_string()) {
                                cur.insert(k.into(), x.clone());
                            }
                        }
                    }
                    let t = s(&self.ext_ui.current, "title").to_string();
                    if t != old_title && !t.is_empty() {
                        if let Some(p) = &self.player {
                            let _ = p.mpv.set_str("file-local-options/force-media-title", &t);
                        }
                    }
                    *self.ext.shared.current.write().unwrap() = self.ext_ui.current.clone();
                }
                let path = c.path.clone();
                self.ext_track_started(&path);
            }
            ChainKind::LiveInfo => {
                let action = s(&c.value, "action").to_string();
                match action.as_str() {
                    "mute" | "quiet" => {
                        if let Some(p) = &self.player {
                            if self.ext_ui.live_restore.is_none() {
                                self.ext_ui.live_restore = Some((p.st.muted, p.st.volume));
                            }
                            if action == "mute" {
                                p.set_mute(true);
                            } else if let Some((_, vol)) = self.ext_ui.live_restore {
                                p.set_volume(vol * 0.3);
                            }
                        }
                    }
                    _ => self.ext_live_restore(),
                }
            }
        }
    }

    fn ext_live_restore(&mut self) {
        if let (Some((muted, vol)), Some(p)) = (self.ext_ui.live_restore.take(), &self.player) {
            p.set_mute(muted);
            p.set_volume(vol);
        }
    }

    /// Pause, progress, queue and radio titles, compared each frame.
    fn ext_watch_player(&mut self) {
        let Some(p) = &self.player else { return };
        let st = &p.st;
        let now = Instant::now();
        let dt = self.ext_ui.last_tick.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0).min(1.0);
        self.ext_ui.last_tick = Some(now);
        if self.ext_ui.snapshot_at.is_none_or(|t| now.duration_since(t) > Duration::from_millis(250)) {
            self.ext_ui.snapshot_at = Some(now);
            let remote = crate::plex::is_remote(&st.path);
            *self.ext.shared.player.write().unwrap() = json!({
                "time": st.time,
                "duration": st.duration,
                "paused": st.paused,
                "volume": st.volume,
                "muted": st.muted,
                "speed": st.speed,
                "idle": !st.has_media(),
                "title": st.media_title,
                "path": if remote { "" } else { st.path.as_str() },
                "playlist_pos": st.playlist_pos,
                "playlist_count": st.playlist.len(),
            });
        }
        let has = st.has_media() && !self.ext_ui.current.is_null();
        let mut emits: Vec<(&str, Vec<Json>)> = vec![];
        if has {
            if !st.paused {
                self.ext_ui.played += dt;
            }
            if self.ext_ui.last_paused.is_some_and(|w| w != st.paused) {
                emits.push((if st.paused { "paused" } else { "resumed" }, vec![self.ext_ui.current.clone(), json!(st.time)]));
            }
            self.ext_ui.last_paused = Some(st.paused);
            if !st.paused && self.ext.listens("track-progress") && self.ext_ui.last_progress.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1)) {
                self.ext_ui.last_progress = Some(now);
                emits.push(("track-progress", vec![self.ext_ui.current.clone(), json!(st.time)]));
            }
        }
        let ptr = Arc::as_ptr(&st.playlist) as usize;
        if ptr != self.ext_ui.queue_ptr {
            self.ext_ui.queue_ptr = ptr;
            self.ext_ui.queue_changed_at = Some(now);
        }
        if self.ext_ui.queue_changed_at.is_some_and(|t| now.duration_since(t) > Duration::from_millis(300)) {
            self.ext_ui.queue_changed_at = None;
            emits.push(("queue-changed", vec![json!({ "count": st.playlist.len(), "position": st.playlist_pos })]));
        }
        let icy = if media::is_url(&st.path) { st.meta("icy-title").unwrap_or("").to_string() } else { String::new() };
        let live = !icy.is_empty() && icy != self.ext_ui.live_title && self.ext.has_hook("live-info");
        if !icy.is_empty() {
            self.ext_ui.live_title = icy.clone();
        }
        let path = st.path.clone();
        for (name, args) in emits {
            self.ext_emit(name, args);
        }
        if live {
            let mut item = self.ext_ui.current.clone();
            if item.is_object() {
                item["live_title"] = json!(icy);
                self.ext.start_chain("live-info", ChainKind::LiveInfo, item, &path, HOOK_BUDGET);
            }
        }
    }

    // ------------------------------------------------------------ segments

    fn ext_segments_tick(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.player else { return };
        let st = &p.st;
        let segs = &mut self.ext.segments;
        if segs.list.is_empty() || segs.path != st.path || !st.has_media() || self.seek_drag.is_some() {
            return;
        }
        let pos = st.time;
        let mut skip = None;
        let mut mute = false;
        for sg in segs.list.iter_mut() {
            if pos < sg.from - 0.5 {
                sg.done = false;
            }
            let inside = pos >= sg.from && pos < sg.to - 0.05;
            match sg.action.as_str() {
                "skip" if inside && !sg.done && !st.paused && skip.is_none() => {
                    sg.done = true;
                    skip = Some((sg.from, sg.to, sg.label.clone(), sg.plugin.clone()));
                }
                "mute" if inside => mute = true,
                _ => {}
            }
        }
        if mute && segs.muted.is_none() {
            segs.muted = Some(st.muted);
            p.set_mute(true);
        } else if !mute {
            if let Some(prev) = segs.muted.take() {
                p.set_mute(prev);
            }
        }
        if let Some((from, to, label, plugin)) = skip {
            let end = if st.duration > 0.0 { to.min(st.duration) } else { to };
            p.seek_abs(end, true);
            let text = if label.is_empty() { t!("Abschnitt übersprungen").to_string() } else { tf!("Übersprungen: {}", label) };
            self.ext.toast(&plugin, text, 5.0, Some(from));
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// Draws the segments onto the seek bar track.
    pub fn ext_draw_segments(&self, painter: &egui::Painter, track: Rect, dur: f64, path: &str) {
        let segs = &self.ext.segments;
        if segs.list.is_empty() || segs.path != path || dur <= 0.0 {
            return;
        }
        let to_x = |tm: f64| track.left() + (tm / dur).clamp(0.0, 1.0) as f32 * track.width();
        for sg in &segs.list {
            if sg.action == "chapter" {
                let x = to_x(sg.from);
                painter.line_segment([pos2(x, track.top()), pos2(x, track.bottom())], Stroke::new(2.0, Color32::WHITE));
                continue;
            }
            let c = match sg.color {
                Some([r, g, b]) => Color32::from_rgb(r, g, b),
                None => match sg.action.as_str() {
                    "skip" => Color32::from_rgb(0, 190, 110),
                    "ask" => Color32::from_rgb(60, 140, 250),
                    "mute" => Color32::from_rgb(230, 170, 0),
                    _ => Color32::from_rgb(170, 120, 255),
                },
            };
            let r = Rect::from_min_max(pos2(to_x(sg.from), track.top()), pos2(to_x(sg.to).max(to_x(sg.from) + 2.0), track.bottom()));
            painter.rect_filled(r, CornerRadius::same(2), c.gamma_multiply(0.75));
        }
    }

    /// Label of the segment under the seek bar's hover position.
    pub fn ext_segment_at(&self, path: &str, t: f64) -> Option<String> {
        let segs = &self.ext.segments;
        if segs.path != path {
            return None;
        }
        segs.list.iter().find(|s| s.action != "chapter" && t >= s.from && t < s.to).or_else(|| segs.list.iter().rev().find(|s| s.action == "chapter" && s.from <= t)).map(|s| s.label.clone()).filter(|l| !l.is_empty())
    }

    // ------------------------------------------------------------ overlays

    /// Toasts, "skip" buttons, progress cards and plugin dialogs.
    pub fn ext_overlays(&mut self, ctx: &egui::Context) {
        if self.ext.toasts.is_empty() && self.ext.progress.is_empty() && self.ext.dialogs.is_empty() && self.ext.segments.list.is_empty() {
            return;
        }
        let t = self.t;
        let now = Instant::now();
        self.ext.toasts.retain(|x| x.until > now);
        let screen = ctx.screen_rect();
        let phone = crate::mobile::compact(ctx);
        let bottom = if self.view == View::Player && self.has_media() {
            if phone { 150.0 } else { crate::ui_player::BOTTOM_H + 70.0 }
        } else if phone {
            170.0
        } else {
            110.0
        };
        let mut cards: Vec<Card> = vec![];
        // "Skip" while inside an `ask` segment.
        if let Some(p) = &self.player {
            let st = &p.st;
            if self.ext.segments.path == st.path {
                if let Some(sg) = self.ext.segments.list.iter().find(|s| s.action == "ask" && st.time >= s.from && st.time < s.to - 0.2) {
                    cards.push(Card::Ask(sg.to, if sg.label.is_empty() { t!("Überspringen").to_string() } else { tf!("{} überspringen", sg.label) }));
                }
            }
        }
        for (i, x) in self.ext.toasts.iter().enumerate() {
            cards.push(Card::Toast(i, x.text.clone(), x.undo));
        }
        for (i, x) in self.ext.progress.iter().enumerate() {
            cards.push(Card::Progress(i, x.text.clone(), x.value, x.cancel));
        }
        let mut act: Option<CardAct> = None;
        if !cards.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
            egui::Area::new(Id::new("ext_cards")).order(Order::Foreground).pivot(Align2::RIGHT_BOTTOM).fixed_pos(pos2(screen.right() - 16.0, screen.bottom() - bottom)).show(ctx, |ui| {
                ui.set_max_width((screen.width() - 32.0).min(380.0));
                // Newest card at the bottom, the Area grows upwards from its pivot.
                ui.with_layout(Layout::top_down(Align::Max), |ui| {
                    for c in cards.iter().rev() {
                        widgets::popup_frame(&t).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| match c {
                            Card::Ask(to, label) => {
                                if widgets::accent_button(ui, &t, &format!("{}  {label}", ph::SKIP_FORWARD)).clicked() {
                                    act = Some(CardAct::SeekTo(*to));
                                }
                            }
                            Card::Toast(i, text, undo) => {
                                // Not ui.horizontal: under the right-aligned column it would run right to left.
                                // As wide as the content, so the card does not stretch.
                                let body = egui::TextStyle::Body.resolve(ui.style());
                                let width_of = |ui: &Ui, s: &str| ui.fonts(|f| f.layout_no_wrap(s.to_string(), body.clone(), t.text).size().x);
                                let gap = ui.spacing().item_spacing.x;
                                let mut w = 18.0 + gap + width_of(ui, text) + 2.0;
                                if undo.is_some() {
                                    w += gap + width_of(ui, t!("Rückgängig")) + 2.0 * ui.spacing().button_padding.x + 8.0;
                                }
                                let row = vec2(w.min(ui.available_width()), ui.spacing().interact_size.y);
                                ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
                                    ui.label(RichText::new(ph::PUZZLE_PIECE).font(theme::icon(15.0)).color(t.accent));
                                    ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
                                    if let Some(u) = undo {
                                        if widgets::ghost_button(ui, &t, t!("Rückgängig")).clicked() {
                                            act = Some(CardAct::Undo(*i, *u));
                                        }
                                    }
                                });
                            }
                            Card::Progress(i, text, value, cancel) => {
                                ui.set_width(300.0);
                                let row = vec2(ui.available_width(), ui.spacing().interact_size.y);
                                ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
                                    ui.add(egui::Label::new(RichText::new(text).color(t.text)).truncate());
                                    if *cancel {
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            if widgets::icon_button(ui, &t, ph::X, t!("Abbrechen"), false).clicked() {
                                                act = Some(CardAct::Cancel(*i));
                                            }
                                        });
                                    }
                                });
                                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 3.0), Sense::hover());
                                match value {
                                    Some(v) => widgets::progress_line(ui.painter(), r, *v, &t),
                                    None => {
                                        let ph_ = (ui.input(|i| i.time) as f32 * 0.8).fract();
                                        let w = r.width() * 0.25;
                                        let x = r.left() + (r.width() + w) * ph_ - w;
                                        ui.painter().rect_filled(Rect::from_min_max(pos2(x.max(r.left()), r.top()), pos2((x + w).min(r.right()), r.bottom())), CornerRadius::same(1), t.accent);
                                    }
                                }
                            }
                        });
                        ui.add_space(6.0);
                    }
                });
            });
        }
        match act {
            Some(CardAct::SeekTo(to)) => {
                if let Some(p) = &self.player {
                    p.seek_abs(to, true);
                }
            }
            Some(CardAct::Undo(i, from)) => {
                if let Some(p) = &self.player {
                    p.seek_abs(from, true);
                }
                if i < self.ext.toasts.len() {
                    self.ext.toasts.remove(i);
                }
            }
            Some(CardAct::Cancel(i)) => {
                if i < self.ext.progress.len() {
                    let pr = self.ext.progress.remove(i);
                    self.ext.call(&pr.plugin, &format!("progress:{}", pr.id), vec![], Pending::Nothing);
                }
            }
            None => {}
        }
        if self.dialog.is_none() && !self.ext.dialogs.is_empty() {
            self.ext_dialog(ctx);
        }
    }

    /// The first dialog a plugin asked for.
    fn ext_dialog(&mut self, ctx: &egui::Context) {
        let t = self.t;
        let Some(d) = self.ext.dialogs.front_mut() else { return };
        let name = self.plugins.get(&d.plugin).map(|p| p.manifest.name.get()).unwrap_or_else(|| d.plugin.clone());
        let spec = d.spec.clone();
        let mut answer: Option<String> = None;
        let modal = egui::Modal::new(Id::new("ext_dialog")).backdrop_color(Color32::from_black_alpha(150)).frame(
            egui::Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(theme::R_LG + 2)).inner_margin(egui::Margin::same(18)),
        );
        let resp = modal.show(ctx, |ui| {
            crate::ui_dialogs::dlg_width(ui, 420.0);
            ui.label(RichText::new(s(&spec, "title")).font(theme::semibold(17.0)).color(t.text));
            ui.label(RichText::new(format!("{}  {}", ph::PUZZLE_PIECE, name)).size(12.0).color(t.text_faint));
            ui.add_space(8.0);
            let text = s(&spec, "text");
            if !text.is_empty() {
                ui.add(egui::Label::new(RichText::new(text).color(t.text_dim)).wrap());
                ui.add_space(8.0);
            }
            for f in spec.get("fields").and_then(|x| x.as_array()).into_iter().flatten() {
                let fid = s(f, "id").to_string();
                let Some(val) = d.values.get_mut(&fid) else { continue };
                let label = s(f, "label");
                if !label.is_empty() && s(f, "type") != "toggle" {
                    ui.label(RichText::new(label).size(13.0).color(t.text));
                }
                match s(f, "type") {
                    "toggle" => {
                        let mut b = val.as_bool().unwrap_or(false);
                        ui.horizontal(|ui| {
                            if widgets::toggle(ui, &t, &mut b).changed() {
                                *val = json!(b);
                            }
                            ui.label(RichText::new(label).color(t.text));
                        });
                    }
                    "choice" => {
                        let cur = val.as_str().unwrap_or("").to_string();
                        for o in f.get("options").and_then(|x| x.as_array()).into_iter().flatten() {
                            let (v, l) = option_pair(o);
                            if widgets::check_row(ui, &t, &l, v == cur).clicked() {
                                *val = json!(v);
                            }
                        }
                    }
                    "number" => {
                        let mut n = val.as_f64().unwrap_or(0.0);
                        if ui.add(egui::DragValue::new(&mut n)).changed() {
                            *val = json!(n);
                        }
                    }
                    kind => {
                        let mut txt = val.as_str().unwrap_or("").to_string();
                        let edit = egui::TextEdit::singleline(&mut txt).password(kind == "password").desired_width(f32::INFINITY);
                        if ui.add(edit).changed() {
                            *val = json!(txt);
                        }
                    }
                }
                ui.add_space(6.0);
            }
            ui.add_space(6.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let buttons: Vec<Json> = spec.get("buttons").and_then(|x| x.as_array()).cloned().unwrap_or_else(|| vec![json!({ "id": "ok", "label": "OK", "primary": true })]);
                for b in buttons.iter().rev() {
                    let label = match b.get("label") {
                        Some(l) => serde_json::from_value::<crate::plugins::Text>(l.clone()).map(|x| x.get()).unwrap_or_default(),
                        None => s(b, "id").to_string(),
                    };
                    let primary = b.get("primary").and_then(|x| x.as_bool()).unwrap_or(false);
                    let clicked = if primary { widgets::accent_button(ui, &t, &label).clicked() } else { widgets::ghost_button(ui, &t, &label).clicked() };
                    if clicked {
                        answer = Some(s(b, "id").to_string());
                    }
                }
            });
        });
        if resp.should_close() && answer.is_none() {
            answer = Some("close".into());
        }
        if let Some(a) = answer {
            let d = self.ext.dialogs.pop_front().unwrap();
            self.ext.call(&d.plugin, &d.key, vec![json!(a), Json::Object(d.values)], Pending::Nothing);
        }
    }

    // ------------------------------------------------------------ menus and buttons

    /// Plugin entries for a menu place ("player", "main", "file", …).
    pub fn ext_menu_items(&mut self, ui: &mut Ui, place: &str, item: Json) {
        let entries: Vec<(String, host::MenuReg)> = self
            .ext
            .running
            .iter()
            .flat_map(|r| r.menus.iter().filter(|m| m.places.iter().any(|p| p == place)).map(move |m| (r.id.clone(), m.clone())))
            .filter(|(_, m)| m.kinds.is_empty() || m.kinds.iter().any(|k| k == s(&item, "kind")))
            .collect();
        if entries.is_empty() {
            return;
        }
        let t = self.t;
        widgets::separator(ui, &t);
        for (plugin, m) in entries {
            if widgets::menu_item(ui, &t, icon_glyph(&m.icon), &m.label.get(), "", false).clicked() {
                ui.close();
                self.ext.call(&plugin, &format!("menu:{}", m.id), vec![item.clone()], Pending::Nothing);
            }
        }
    }

    pub fn ext_buttons_width(&self) -> f32 {
        self.ext.running.iter().map(|r| r.buttons.len()).sum::<usize>() as f32 * 36.0
    }

    /// Plugin buttons in the player bar.
    pub fn ext_buttons(&mut self, ui: &mut Ui) {
        let t = self.t;
        let list: Vec<(String, host::ButtonReg)> = self.ext.running.iter().flat_map(|r| r.buttons.iter().map(move |b| (r.id.clone(), b.clone()))).collect();
        for (plugin, b) in list {
            if widgets::icon_button(ui, &t, icon_glyph(&b.icon), &b.label.get(), false).clicked() {
                let item = self.ext_ui.current.clone();
                self.ext.call(&plugin, &format!("button:{}", b.id), vec![item], Pending::Nothing);
            }
        }
    }

    // ------------------------------------------------------------ sources

    /// Plugin sources for the library navigation: (plugin, source).
    pub fn ext_sources(&self) -> Vec<(String, host::SourceReg)> {
        self.ext.running.iter().flat_map(|r| r.sources.iter().map(move |s| (r.id.clone(), s.clone()))).collect()
    }

    pub fn ext_source_loc(plugin: &str, src: &host::SourceReg) -> LibLoc {
        LibLoc::Ext(ExtLoc { plugin: plugin.into(), source: src.id.clone(), label: src.label.get(), path: String::new(), title: src.label.get(), query: String::new(), trail: vec![] })
    }

    /// One page of a plugin source.
    pub fn ext_page(&mut self, ui: &mut Ui, loc: ExtLoc, touch: bool) {
        let t = self.t;
        let key = PageKey { plugin: loc.plugin.clone(), source: loc.source.clone(), path: loc.path.clone(), query: loc.query.clone() };
        let src = self.ext.get(&loc.plugin).and_then(|r| r.sources.iter().find(|s| s.id == loc.source).cloned());
        let Some(src) = src else {
            ui.add_space(20.0);
            ui.label(RichText::new(t!("Dieses Plugin läuft gerade nicht.")).color(t.text_dim));
            return;
        };
        if !self.ext.pages.contains_key(&key) {
            let (fkey, args, purpose) = if loc.query.is_empty() {
                (format!("source:{}:browse", loc.source), vec![json!(loc.path)], Pending::Browse { key: key.clone() })
            } else {
                (format!("source:{}:search", loc.source), vec![json!(loc.query)], Pending::Search { key: key.clone() })
            };
            if self.ext.call(&loc.plugin, &fkey, args, purpose).is_some() {
                self.ext.pages.insert(key.clone(), PageState::Loading(Instant::now()));
            }
        }
        if !touch {
            self.header_nav(ui, false);
            ui.add_space(4.0);
        }
        ui.horizontal(|ui| {
            if src.search {
                let id = Id::new(("ext_search", &loc.plugin, &loc.source));
                let out = widgets::search_field(ui, &t, id, &mut self.ext_ui.search, t!("In dieser Quelle suchen"), 260.0);
                if out.submitted && !self.ext_ui.search.trim().is_empty() {
                    let mut l = loc.clone();
                    l.query = self.ext_ui.search.trim().to_string();
                    l.title = tf!("Suche: {}", l.query);
                    l.trail = vec![];
                    l.path = String::new();
                    self.lib.go(LibLoc::Ext(l));
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_button(ui, &t, ph::ARROWS_CLOCKWISE, t!("Neu laden"), false).clicked() {
                    self.ext.pages.remove(&key);
                }
            });
        });
        widgets::separator(ui, &t);
        let state = match self.ext.pages.get(&key) {
            Some(PageState::Ready(v)) => Ok(v.clone()),
            Some(PageState::Failed(e)) => Err(Some(e.clone())),
            _ => Err(None),
        };
        match state {
            Err(None) => {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(t!("Wird geladen …")).color(t.text_dim));
                });
            }
            Err(Some(e)) => {
                ui.add_space(20.0);
                ui.add(egui::Label::new(RichText::new(tf!("Fehler im Plugin: {}", e)).color(t.danger)).wrap());
            }
            Ok(v) => {
                let (items, view) = page_items(&v);
                if items.is_empty() {
                    ui.add_space(20.0);
                    ui.label(RichText::new(t!("Hier ist nichts.")).color(t.text_dim));
                }
                let granted = self.ext.get(&loc.plugin).map(|r| r.granted.clone()).unwrap_or_default();
                let item_menus: Vec<(String, host::MenuReg)> =
                    self.ext.running.iter().flat_map(|r| r.menus.iter().filter(|m| m.places.iter().any(|p| p == "item")).map(move |m| (r.id.clone(), m.clone()))).collect();
                let mut click: Option<Json> = None;
                let mut act: Option<ItemAct> = None;
                let menu = |r: &egui::Response, it: &Json, act: &mut Option<ItemAct>| {
                    r.context_menu(|ui| {
                        ui.set_min_width(220.0);
                        if playable(it) {
                            if widgets::menu_item(ui, &t, ph::PLAY, t!("Abspielen"), "", false).clicked() {
                                ui.close();
                                *act = Some(ItemAct::Play(it.clone(), "replace"));
                            }
                            if widgets::menu_item(ui, &t, ph::QUEUE, t!("Zur Warteschlange"), "", false).clicked() {
                                ui.close();
                                *act = Some(ItemAct::Play(it.clone(), "append"));
                            }
                        }
                        for (plugin, m) in &item_menus {
                            if (m.kinds.is_empty() || m.kinds.iter().any(|k| k == s(it, "kind"))) && widgets::menu_item(ui, &t, icon_glyph(&m.icon), &m.label.get(), "", false).clicked() {
                                ui.close();
                                *act = Some(ItemAct::Menu(plugin.clone(), m.id.clone(), it.clone()));
                            }
                        }
                    });
                };
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if view == "grid" && !touch {
                        let w = 168.0;
                        let cols = ((ui.available_width() + 12.0) / (w + 12.0)).floor().max(1.0) as usize;
                        for row in items.chunks(cols) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 12.0;
                                for it in row {
                                    let r = self.ext_tile(ui, it, w, &granted);
                                    if r.clicked() {
                                        click = Some(it.clone());
                                    }
                                    menu(&r, it, &mut act);
                                }
                            });
                            ui.add_space(12.0);
                        }
                    } else {
                        for it in &items {
                            let r = self.ext_row(ui, it, &granted);
                            if r.clicked() {
                                click = Some(it.clone());
                            }
                            menu(&r, it, &mut act);
                        }
                    }
                    ui.add_space(if self.has_media() { 90.0 } else { 20.0 });
                });
                match act {
                    Some(ItemAct::Play(it, mode)) => self.ext_play(&loc.plugin, &loc.source, it, mode),
                    Some(ItemAct::Menu(plugin, mid, it)) => {
                        self.ext.call(&plugin, &format!("menu:{mid}"), vec![it], Pending::Nothing);
                    }
                    None => {}
                }
                if let Some(it) = click {
                    if playable(&it) {
                        self.ext_play(&loc.plugin, &loc.source, it, "replace");
                    } else {
                        let mut l = loc.clone();
                        l.trail.push((loc.title.clone(), loc.path.clone()));
                        l.path = s(&it, "id").to_string();
                        l.title = s(&it, "title").to_string();
                        self.lib.go(LibLoc::Ext(l));
                    }
                }
            }
        }
    }

    fn ext_image(&mut self, url: &str, granted: &[String]) -> Option<crate::plex::images::Shown> {
        if url.is_empty() {
            return None;
        }
        let u = util::parse_url(url)?;
        if !matches!(u.scheme.as_str(), "http" | "https") || !util::host_allowed(granted, &u.host) {
            return None;
        }
        let url = url.to_string();
        self.logos.get(&url, || url.clone())
    }

    fn ext_row(&mut self, ui: &mut Ui, it: &Json, granted: &[String]) -> egui::Response {
        let t = self.t;
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(theme::R), t.hover);
        }
        let thumb = it.get("art").map(|a| s(a, "thumb")).filter(|x| !x.is_empty()).or_else(|| Some(s(it, "image")).filter(|x| !x.is_empty())).unwrap_or("").to_string();
        let ir = Rect::from_min_size(pos2(r.left() + 6.0, r.top() + 6.0), vec2(40.0, 40.0));
        match self.ext_image(&thumb, granted) {
            Some(img) => {
                ui.painter().image(img.id, ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(img.alpha));
            }
            None => {
                ui.painter().rect_filled(ir, CornerRadius::same(theme::R), t.elevated);
                ui.painter().text(ir.center(), Align2::CENTER_CENTER, kind_glyph(s(it, "kind"), playable(it)), theme::icon(18.0), t.text_dim);
            }
        }
        let x = ir.right() + 12.0;
        let w = r.right() - x - 90.0;
        let sub = s(it, "subtitle");
        let ty = if sub.is_empty() { r.center().y - 8.0 } else { r.top() + 9.0 };
        widgets::painter_text_trunc(ui.painter(), pos2(x, ty), s(it, "title"), FontId::proportional(14.0), t.text, w);
        if !sub.is_empty() {
            widgets::painter_text_trunc(ui.painter(), pos2(x, r.top() + 28.0), sub, FontId::proportional(12.0), t.text_faint, w);
        }
        let dur = it.get("duration").and_then(|x| x.as_f64()).unwrap_or(0.0);
        if dur > 0.0 {
            ui.painter().text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, media::fmt_time_short(dur), FontId::proportional(12.0), t.text_faint);
        } else if !playable(it) {
            ui.painter().text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, ph::CARET_RIGHT, theme::icon(14.0), t.text_faint);
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    fn ext_tile(&mut self, ui: &mut Ui, it: &Json, w: f32, granted: &[String]) -> egui::Response {
        let t = self.t;
        let (r, resp) = ui.allocate_exact_size(vec2(w, w + 44.0), Sense::click());
        let ir = Rect::from_min_size(r.min, vec2(w, w));
        let art = it.get("art").map(|a| if s(a, "poster").is_empty() { s(a, "thumb") } else { s(a, "poster") }).unwrap_or("");
        let art = if art.is_empty() { s(it, "image") } else { art }.to_string();
        match self.ext_image(&art, granted) {
            Some(img) => {
                ui.painter().rect_filled(ir, CornerRadius::same(theme::R_LG), t.elevated);
                let a = img.size.x / img.size.y.max(1.0);
                let fit = if a > 1.0 { vec2(w, w / a) } else { vec2(w * a, w) };
                ui.painter().image(img.id, Rect::from_center_size(ir.center(), fit), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(img.alpha));
            }
            None => {
                ui.painter().rect_filled(ir, CornerRadius::same(theme::R_LG), t.elevated);
                ui.painter().text(ir.center(), Align2::CENTER_CENTER, kind_glyph(s(it, "kind"), playable(it)), theme::icon(34.0), t.text_dim);
            }
        }
        if resp.hovered() {
            ui.painter().rect_stroke(ir, CornerRadius::same(theme::R_LG), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }
        widgets::painter_text_trunc(ui.painter(), pos2(r.left(), ir.bottom() + 6.0), s(it, "title"), theme::semibold(13.0), t.text, w);
        widgets::painter_text_trunc(ui.painter(), pos2(r.left(), ir.bottom() + 24.0), s(it, "subtitle"), FontId::proportional(12.0), t.text_faint, w);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    /// Plays a plugin item: its `url`, or what `resolve` returns.
    pub fn ext_play(&mut self, plugin: &str, source: &str, item: Json, mode: &str) {
        let url = s(&item, "url").to_string();
        if !url.is_empty() {
            let v = json!({ "url": url });
            self.ext_play_url(plugin, item, v, mode);
            return;
        }
        let mut it = item.clone();
        it["_mode"] = json!(mode);
        if self.ext.call(plugin, &format!("source:{source}:resolve"), vec![item], Pending::Resolve { item: it }).is_none() {
            self.notice(t!("Dieses Plugin läuft gerade nicht."));
        }
    }

    fn ext_play_resolved(&mut self, plugin: &str, item: Json, v: Json) {
        let mode = s(&item, "_mode").to_string();
        let v = if v.is_string() { json!({ "url": v }) } else { v };
        self.ext_play_url(plugin, item, v, if mode.is_empty() { "replace" } else { &mode });
    }

    fn ext_play_url(&mut self, plugin: &str, mut item: Json, v: Json, mode: &str) {
        let url = s(&v, "url").to_string();
        let granted = self.ext.get(plugin).map(|r| r.granted.clone()).unwrap_or_default();
        let ok = util::parse_url(&url).is_some_and(|u| matches!(u.scheme.as_str(), "http" | "https" | "rtmp" | "rtsp" | "rtmps" | "srt") && util::host_allowed(&granted, &u.host));
        if !ok {
            self.ext.log(plugin, Level::Error, format!("no permission to play {url}"));
            self.notice(t!("Das Plugin darf diese Adresse nicht abspielen."));
            return;
        }
        if let Some(o) = item.as_object_mut() {
            o.remove("_mode");
        }
        let title = if s(&v, "title").is_empty() { s(&item, "title").to_string() } else { s(&v, "title").to_string() };
        let headers: Vec<String> = v.get("headers").and_then(|x| x.as_object()).map(|h| h.iter().filter_map(|(k, x)| x.as_str().map(|x| format!("{k}: {x}"))).collect()).unwrap_or_default();
        let start = v.get("start").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let kind = s(&item, "kind").to_string();
        self.ext_ui.items.insert(url.clone(), (plugin.to_string(), item));
        self.ext_load(&url, mode, &title, start, &headers.join(","));
        if mode == "replace" && !matches!(kind.as_str(), "audio" | "track" | "episode" | "podcast" | "stream") {
            self.view = View::Player;
        }
    }

    /// Sections plugins add to the start page.
    pub fn ext_home(&mut self, ui: &mut Ui) {
        let homes: Vec<(String, host::HomeReg)> = self.ext.running.iter().flat_map(|r| r.homes.iter().map(move |h| (r.id.clone(), h.clone()))).collect();
        let t = self.t;
        for (plugin, h) in homes {
            let key = (plugin.clone(), h.id.clone());
            let stale = match self.ext.homes.get(&key) {
                None => true,
                Some(PageState::Loading(at)) => at.elapsed() > Duration::from_secs(60),
                _ => false,
            };
            if stale {
                if self.ext.call(&plugin, &format!("home:{}", h.id), vec![], Pending::Home { plugin: plugin.clone(), id: h.id.clone() }).is_some() {
                    self.ext.homes.insert(key.clone(), PageState::Loading(Instant::now()));
                }
            }
            let Some(PageState::Ready(v)) = self.ext.homes.get(&key) else { continue };
            let (items, _) = page_items(v);
            if items.is_empty() {
                continue;
            }
            ui.add_space(18.0);
            ui.label(RichText::new(h.label.get()).font(theme::semibold(17.0)).color(t.text));
            ui.add_space(6.0);
            let granted = self.ext.get(&plugin).map(|r| r.granted.clone()).unwrap_or_default();
            let mut click = None;
            egui::ScrollArea::horizontal().id_salt(("ext_home", &plugin, &h.id)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    for it in items.iter().take(30) {
                        if self.ext_tile(ui, it, 150.0, &granted).clicked() {
                            click = Some(it.clone());
                        }
                    }
                });
            });
            if let Some(it) = click {
                if playable(&it) {
                    let src = s(&it, "source").to_string();
                    self.ext_play(&plugin, &src, it, "replace");
                } else if let Some(src) = self.ext.get(&plugin).and_then(|r| r.sources.first().cloned()) {
                    let mut l = match Self::ext_source_loc(&plugin, &src) {
                        LibLoc::Ext(l) => l,
                        _ => unreachable!(),
                    };
                    l.path = s(&it, "id").to_string();
                    l.title = s(&it, "title").to_string();
                    l.trail = vec![(src.label.get(), String::new())];
                    self.lib.go(LibLoc::Ext(l));
                }
            }
        }
    }
}

enum Card {
    Ask(f64, String),
    Toast(usize, String, Option<f64>),
    Progress(usize, String, Option<f32>, bool),
}

enum ItemAct {
    Play(Json, &'static str),
    Menu(String, String, Json),
}

enum CardAct {
    SeekTo(f64),
    Undo(usize, f64),
    Cancel(usize),
}

/// (value, label) of a choice option: "x" or { value = "x", label = … }.
pub fn option_pair(o: &Json) -> (String, String) {
    match o {
        Json::Object(m) => {
            let v = m.get("value").map(|x| x.as_str().map(|s| s.to_string()).unwrap_or_else(|| x.to_string())).unwrap_or_default();
            let l = m.get("label").and_then(|l| serde_json::from_value::<crate::plugins::Text>(l.clone()).ok()).map(|x| x.get()).unwrap_or_else(|| v.clone());
            (v, l)
        }
        Json::String(s) => (s.clone(), s.clone()),
        other => (other.to_string(), other.to_string()),
    }
}

// ---------------------------------------------------------------- settings / console

/// Text and level of a permission for dialogs.
pub fn permission_text(p: &str) -> (String, bool) {
    match p {
        "network:*" => (t!("Darf sich mit beliebigen Servern verbinden").into(), true),
        "network:local" => (t!("Darf Geräte in deinem Heimnetz ansprechen").into(), false),
        "open-url" => (t!("Darf Links im Browser öffnen").into(), false),
        "hooks:play" => (t!("Darf ändern, was abgespielt wird").into(), false),
        "hooks:stream" => (t!("Darf Streams verändern, bevor sie abgespielt werden").into(), false),
        "files:read" => (t!("Darf Dateien in Ordnern lesen, die du auswählst").into(), false),
        "files:write" => (t!("Darf Dateien in Ordnern ändern, die du auswählst").into(), true),
        "library" => (t!("Darf die Bibliothek lesen und Metadaten schreiben").into(), false),
        "history" => (t!("Darf deinen Verlauf und Fortschritt lesen").into(), false),
        "history:write" => (t!("Darf Verlauf und Fortschritt ändern").into(), false),
        "servers" => (t!("Zugriff auf deine Plex-, Jellyfin- und Emby-Anmeldungen").into(), true),
        "process" => (t!("Darf Programme starten").into(), true),
        "clipboard" => (t!("Darf die Zwischenablage lesen und schreiben").into(), false),
        "server" => (t!("Darf Verbindungen von anderen Geräten annehmen").into(), true),
        "tags" => (t!("Darf Tags in deinen Dateien ändern").into(), true),
        "cast" => (t!("Darf die Wiedergabe an andere Geräte schicken").into(), false),
        _ => match (p.strip_prefix("network:"), p.strip_prefix("ipc:")) {
            (Some(host), _) => (tf!("Darf Verbindungen zu {} aufnehmen", host), false),
            (_, Some(name)) => (tf!("Darf mit dem Programm „{}“ sprechen", name), false),
            _ => (tf!("Unbekannte Berechtigung: {}", p), true),
        },
    }
}

/// Permissions in the API 2 that Velouna does not offer yet: plugins may
/// ask for them, but the functions behind them are not there.
pub fn permission_pending(p: &str) -> bool {
    matches!(p, "hooks:stream" | "library" | "history" | "history:write" | "servers" | "clipboard" | "process" | "server" | "tags" | "cast" | "files:write") || p.starts_with("ipc:")
}

impl App {
    /// "Allow and switch on" for a plugin whose permissions are not granted.
    pub(crate) fn dlg_plugin_grant(&mut self, ui: &mut Ui, id: &str) {
        let t = self.t;
        crate::ui_dialogs::dlg_width(ui, 440.0);
        if self.dlg_title(ui, t!("Plugin einschalten")) {
            return;
        }
        let Some(p) = self.plugins.get(id) else {
            self.dialog = None;
            return;
        };
        let m = p.manifest.clone();
        ui.label(RichText::new(m.name.get()).font(theme::semibold(15.0)).color(t.text));
        ui.add_space(6.0);
        ui.label(RichText::new(t!("Das Plugin möchte:")).font(theme::semibold(13.0)).color(t.text));
        for x in &m.permissions {
            let (text, critical) = permission_text(x);
            ui.horizontal(|ui| {
                ui.label(RichText::new(if critical { ph::WARNING_OCTAGON } else { ph::WARNING }).font(theme::icon(15.0)).color(if critical { t.danger } else { crate::ui_plugins::WARN }));
                ui.add(egui::Label::new(RichText::new(text).size(13.0).color(t.text)).wrap());
            });
        }
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::accent_button(ui, &t, t!("Erlauben und einschalten")).clicked() {
                let st = self.data.plugins.entry(id.to_string()).or_default();
                st.granted = m.permissions.clone();
                st.granted_version = m.version.clone();
                st.enabled = true;
                self.mark_dirty();
                self.dialog = None;
                self.ext_start(id);
            }
            if widgets::ghost_button(ui, &t, t!("Abbrechen")).clicked() {
                self.dialog = None;
            }
        });
    }

    /// The settings page Velouna draws from a plugin's manifest.
    pub(crate) fn dlg_plugin_settings(&mut self, ui: &mut Ui, id: &str) {
        let t = self.t;
        crate::ui_dialogs::dlg_width(ui, 480.0);
        let Some(p) = self.plugins.get(id) else {
            self.dialog = None;
            return;
        };
        let m = p.manifest.clone();
        if self.dlg_title(ui, &tf!("{} · Einstellungen", m.name.get())) {
            return;
        }
        let mut changed = false;
        let mut action: Option<String> = None;
        egui::ScrollArea::vertical().max_height(ui.ctx().screen_rect().height() * 0.6).show(ui, |ui| {
            for d in &m.settings {
                let label = d.label.get();
                let label = if label.is_empty() { d.id.clone() } else { label };
                let help = d.help.get();
                let st = self.data.plugins.entry(id.to_string()).or_default();
                let cur = st.settings.get(&d.id).cloned().unwrap_or_else(|| d.default.clone());
                match d.kind.as_str() {
                    "toggle" => {
                        let mut b = cur.as_bool().unwrap_or(false);
                        widgets::setting_row(ui, &t, &label, &help, |ui| {
                            if widgets::toggle(ui, &t, &mut b).changed() {
                                st.settings.insert(d.id.clone(), json!(b));
                                changed = true;
                            }
                        });
                    }
                    "slider" => {
                        let (lo, hi) = (d.min.unwrap_or(0.0), d.max.unwrap_or(100.0));
                        let mut x = cur.as_f64().unwrap_or(lo);
                        widgets::setting_row(ui, &t, &label, &help, |ui| {
                            ui.label(RichText::new(format!("{x:.0}")).color(t.text_dim));
                            if widgets::slider(ui, &t, &mut x, lo, hi, 180.0).changed() {
                                if let Some(step) = d.step.filter(|s| *s > 0.0) {
                                    x = (x / step).round() * step;
                                }
                                st.settings.insert(d.id.clone(), json!(x));
                                changed = true;
                            }
                        });
                    }
                    "choice" => {
                        let curs = cur.as_str().map(|s| s.to_string()).unwrap_or_else(|| cur.to_string());
                        ui.label(RichText::new(&label).color(t.text));
                        for o in &d.options {
                            let (v, l) = option_pair(o);
                            if widgets::check_row(ui, &t, &l, v == curs).clicked() {
                                st.settings.insert(d.id.clone(), json!(v));
                                changed = true;
                            }
                        }
                        ui.add_space(6.0);
                    }
                    "multi" => {
                        let mut sel: Vec<String> = cur.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
                        ui.label(RichText::new(&label).color(t.text));
                        for o in &d.options {
                            let (v, l) = option_pair(o);
                            let on = sel.contains(&v);
                            if widgets::check_row(ui, &t, &l, on).clicked() {
                                if on {
                                    sel.retain(|x| *x != v);
                                } else {
                                    sel.push(v);
                                }
                                st.settings.insert(d.id.clone(), json!(sel));
                                changed = true;
                            }
                        }
                        ui.add_space(6.0);
                    }
                    "secret" => {
                        let svc = host::secret_service(id);
                        let key = (id.to_string(), d.id.clone());
                        let stored = crate::platform::secret::get(&svc, &d.id).is_some();
                        ui.label(RichText::new(&label).color(t.text));
                        if !help.is_empty() {
                            ui.label(RichText::new(&help).size(12.0).color(t.text_faint));
                        }
                        ui.horizontal(|ui| {
                            let buf = self.ext_ui.secret_buf.entry(key.clone()).or_default();
                            ui.add(egui::TextEdit::singleline(buf).password(true).desired_width(220.0).hint_text(if stored { t!("gespeichert") } else { "" }));
                            if widgets::ghost_button(ui, &t, t!("Speichern")).clicked() && !buf.is_empty() {
                                if crate::platform::secret::set(&svc, &d.id, buf) {
                                    buf.clear();
                                    changed = true;
                                }
                            }
                            if stored && widgets::ghost_button(ui, &t, t!("Löschen")).clicked() {
                                crate::platform::secret::delete(&svc, &d.id);
                                changed = true;
                            }
                        });
                        ui.add_space(6.0);
                    }
                    "action" => {
                        widgets::setting_row(ui, &t, &label, &help, |ui| {
                            if widgets::accent_button(ui, &t, &label).clicked() {
                                action = Some(if d.run.is_empty() { d.id.clone() } else { d.run.clone() });
                            }
                        });
                    }
                    #[cfg(not(target_os = "android"))]
                    "folder" => {
                        let path = cur.as_str().unwrap_or("").to_string();
                        widgets::setting_row(ui, &t, &label, &help, |ui| {
                            if widgets::ghost_button(ui, &t, t!("Ordner wählen…")).clicked() {
                                if let Some(f) = rfd::FileDialog::new().pick_folder() {
                                    st.settings.insert(d.id.clone(), json!(f.to_string_lossy()));
                                    changed = true;
                                }
                            }
                            ui.label(RichText::new(if path.is_empty() { t!("keiner") } else { &path }).color(t.text_dim));
                        });
                    }
                    _ => {
                        let mut s = cur.as_str().map(|s| s.to_string()).unwrap_or_else(|| if cur.is_null() { String::new() } else { cur.to_string() });
                        ui.label(RichText::new(&label).color(t.text));
                        if !help.is_empty() {
                            ui.label(RichText::new(&help).size(12.0).color(t.text_faint));
                        }
                        if ui.add(egui::TextEdit::singleline(&mut s).desired_width(f32::INFINITY)).changed() {
                            st.settings.insert(d.id.clone(), json!(s));
                            changed = true;
                        }
                        ui.add_space(6.0);
                    }
                }
            }
        });
        if changed {
            self.mark_dirty();
            if let (Some(p), Some(st)) = (self.plugins.get(id), self.data.plugins.get(id)) {
                self.ext.send_settings(p, st);
            }
        }
        if let Some(a) = action {
            if !self.ext.is_running(id) {
                self.ext_start(id);
            }
            if self.ext.call(id, &format!("setting:{a}"), vec![], Pending::Nothing).is_none() {
                self.notice(t!("Das Plugin ist ausgeschaltet."));
            }
        }
    }

    /// Log, costs and a Lua prompt for one plugin.
    pub(crate) fn dlg_plugin_console(&mut self, ui: &mut Ui, id: &str) {
        let t = self.t;
        crate::ui_dialogs::dlg_width(ui, 720.0);
        let name = self.plugins.get(id).map(|p| p.manifest.name.get()).unwrap_or_else(|| id.to_string());
        if self.dlg_title(ui, &tf!("{} · Konsole", name)) {
            return;
        }
        let running = self.ext.is_running(id);
        let stats = self.ext.get(id).map(|r| r.stats.lock().unwrap().clone());
        ui.horizontal_wrapped(|ui| {
            let (txt, col) = if running {
                (t!("läuft"), Color32::from_rgb(60, 190, 110))
            } else if let Some(e) = self.ext.errors.get(id) {
                (e.as_str(), t.danger)
            } else {
                (t!("angehalten"), t.text_faint)
            };
            ui.label(RichText::new(format!("● {txt}")).color(col));
            if let Some(s) = &stats {
                ui.label(
                    RichText::new(tf!(
                        "Laden {:.1} ms · {} Aufrufe · {:.0} ms gesamt · längster {:.1} ms · {} Anfragen ({} KB) · {} Fehler",
                        s.load_ms,
                        s.calls,
                        s.cpu_ms,
                        s.max_ms,
                        s.net,
                        s.net_bytes / 1024,
                        s.errors
                    ))
                    .size(12.0)
                    .color(t.text_dim),
                );
            }
            if let Some(r) = self.ext.get(id) {
                if !r.hooks_off.is_empty() {
                    ui.label(RichText::new(tf!("abgeschaltete Hooks: {}", r.hooks_off.iter().cloned().collect::<Vec<_>>().join(", "))).size(12.0).color(t.danger));
                }
            }
        });
        ui.add_space(6.0);
        let lines: Vec<host::LogLine> = self.ext.logs.get(id).map(|l| l.iter().cloned().collect()).unwrap_or_default();
        egui::Frame::new().fill(t.bg).corner_radius(CornerRadius::same(theme::R)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(320.0).min_scrolled_height(320.0).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                if lines.is_empty() {
                    ui.label(RichText::new(t!("Noch keine Meldungen.")).color(t.text_faint));
                }
                for l in &lines {
                    let col = match l.level {
                        Level::Info => t.text,
                        Level::Warn => crate::ui_plugins::WARN,
                        Level::Error => t.danger,
                    };
                    ui.add(egui::Label::new(RichText::new(format!("{}  {}", l.at.format("%H:%M:%S"), l.text)).font(FontId::monospace(12.0)).color(col)).wrap());
                }
            });
        });
        ui.add_space(6.0);
        let mut run = false;
        ui.horizontal(|ui| {
            let r = ui.add_enabled(running, egui::TextEdit::singleline(&mut self.ext_ui.console_input).font(FontId::monospace(13.0)).hint_text(RichText::new(t!("Lua eingeben, Enter führt aus")).font(FontId::monospace(13.0)).color(t.text_faint)).desired_width(ui.available_width() - 120.0));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                run = true;
                r.request_focus();
            }
            if ui.add_enabled_ui(running, |ui| widgets::ghost_button(ui, &t, t!("Ausführen"))).inner.clicked() {
                run = true;
            }
        });
        if run && !self.ext_ui.console_input.trim().is_empty() {
            let code = std::mem::take(&mut self.ext_ui.console_input);
            self.ext.log(id, Level::Info, format!("> {code}"));
            self.ext.eval(id, &code);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if widgets::ghost_button(ui, &t, if running { t!("Neu starten") } else { t!("Starten") }).clicked() {
                self.plugins.rescan();
                self.ext_restart(id);
            }
            if widgets::ghost_button(ui, &t, t!("Log leeren")).clicked() {
                self.ext.logs.remove(id);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn options() {
        assert_eq!(super::option_pair(&serde_json::json!("a")), ("a".into(), "a".into()));
        assert_eq!(super::option_pair(&serde_json::json!({"value": "x", "label": "Ex"})), ("x".into(), "Ex".into()));
    }
}
