//! Application state, mpv lifecycle and non-visual logic.

use crate::config::{self, Bookmark, Data, HistoryEntry, Preset, RepeatMode, Settings};
use crate::hotkeys::{self, Action, Bindings, Combo};
use crate::library::LibraryState;
use crate::media::{self, Cue, MediaResult, MediaTools};
use crate::mpv::{self, Event, Mpv};
use crate::player::{LoadMode, Player};
use crate::theme::Theme;
use crate::video::VideoRenderer;
use eframe::egui::{self, TextureHandle, ViewportCommand};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Player,
    Library,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RightPanel {
    None,
    Adjust,
    Queue,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdjustTab {
    Playback,
    Video,
    Audio,
    Subtitles,
    Marks,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrackTab {
    Audio,
    Subtitles,
    Video,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsPage {
    General,
    Interface,
    Privacy,
    Controls,
    Playback,
    Video,
    Audio,
    Subtitles,
    Library,
    Plex,
    Network,
    Performance,
    Recording,
    Extensions,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Dialog {
    Settings,
    Hotkeys,
    Customize,
    Network,
    Convert,
    AddSubs,
    NewPlaylist(Vec<String>),
    AddToPlaylist(Vec<String>),
    SavePreset,
    About,
    Onboarding(u8),
    Error(String),
    /// Update check / download / install.
    Update,
    /// Changelog after an update (markdown).
    WhatsNew(String),
    /// Program guide of a TV channel.
    TvGuide { title: String, country: String, keys: Vec<String> },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SubScope {
    File,
    Folder,
    Queue,
}

pub enum ThumbState {
    Loading,
    Ready(TextureHandle),
    Failed,
}

pub struct Osd {
    pub chips: Vec<String>,
    pub text: String,
    pub until: Instant,
}

pub struct ResumePrompt {
    pub path: String,
    pub pos: f64,
    pub until: Instant,
}

pub struct PipState {
    pub prev_size: egui::Vec2,
    pub prev_pos: Option<egui::Pos2>,
    pub was_maximized: bool,
}

#[derive(Default)]
pub struct DialogBuffers {
    pub url: String,
    pub url_name: String,
    pub url_save: bool,
    pub playlist_name: String,
    pub preset_name: String,
    pub preset_for_folder: bool,
    pub sub_scope: Option<SubScope>,
    pub sub_matches: Vec<(String, Option<String>)>,
    pub sub_files: Vec<String>,
    pub convert_in: String,
    pub convert_out: String,
    pub convert_profile: usize,
    pub convert_progress: Option<(f64, f64)>,
    pub convert_result: Option<Result<String, String>>,
    pub hotkey_search: String,
    pub onboarding_theme: Option<config::ThemeMode>,
}

pub const EQ_FREQS: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
pub const EQ_PRESETS: &[(&str, [f64; 10])] = &[
    ("Flat", [0.0; 10]),
    ("Bass-Boost", [6.0, 5.0, 4.0, 2.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Höhen-Boost", [0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 2.0, 4.0, 5.0, 6.0]),
    ("Sprache", [-4.0, -3.0, -1.0, 1.0, 3.0, 4.0, 4.0, 2.0, 0.0, -2.0]),
    ("Rock", [4.5, 3.5, 2.0, -1.0, -1.5, 0.0, 2.0, 3.5, 4.0, 4.5]),
    ("Pop", [-1.0, 1.0, 3.0, 4.0, 3.0, 0.0, -1.0, -1.0, 1.0, 2.0]),
    ("Klassik", [3.0, 2.0, 1.0, 0.0, 0.0, 0.0, -1.0, -1.5, 1.0, 2.5]),
    ("Elektronisch", [4.0, 3.5, 1.0, 0.0, -2.0, 1.5, 0.5, 1.0, 3.5, 4.5]),
    ("Loudness", [5.0, 3.0, 0.0, 0.0, -1.5, 0.0, -0.5, -4.0, 4.0, 1.0]),
];

pub const VIDEO_PRESETS: &[(&str, [f64; 5], &str, [u8; 3], [u8; 3])] = &[
    // name, [brightness, contrast, saturation, gamma, hue], extra vf, swatch colors
    ("Keiner", [0.0, 0.0, 0.0, 0.0, 0.0], "", [60, 60, 60], [120, 120, 120]),
    ("Lebendig", [0.0, 12.0, 35.0, 0.0, 0.0], "", [255, 70, 40], [40, 160, 255]),
    ("Warm", [2.0, 4.0, 8.0, 0.0, 0.0], "colortemperature=temperature=4800", [255, 150, 60], [200, 90, 40]),
    ("Kühl", [0.0, 4.0, 0.0, 0.0, 0.0], "colortemperature=temperature=9500", [90, 160, 255], [40, 90, 200]),
    ("Kino", [-3.0, 10.0, -12.0, -6.0, 0.0], "", [30, 60, 70], [220, 140, 60]),
    ("Schwarz-Weiß", [0.0, 8.0, -100.0, 0.0, 0.0], "", [20, 20, 20], [230, 230, 230]),
    ("Sepia", [0.0, 0.0, 0.0, 0.0, 0.0], "colorchannelmixer=.393:.769:.189:0:.349:.686:.168:0:.272:.534:.131", [112, 66, 20], [230, 200, 150]),
    ("Nacht", [-12.0, -5.0, -10.0, -8.0, 0.0], "", [10, 10, 30], [60, 60, 90]),
];

pub const CONVERT_PROFILES: &[(&str, &str, &[&str])] = &[
    ("Video – MP4 (H.264 + AAC)", "mp4", &["-c:v", "libx264", "-preset", "medium", "-crf", "21", "-c:a", "aac", "-b:a", "192k"]),
    ("Video – MP4 (H.265 + AAC)", "mp4", &["-c:v", "libx265", "-preset", "medium", "-crf", "24", "-c:a", "aac", "-b:a", "160k", "-tag:v", "hvc1"]),
    ("Video – WebM (VP9 + Opus)", "webm", &["-c:v", "libvpx-vp9", "-crf", "32", "-b:v", "0", "-c:a", "libopus", "-b:a", "128k"]),
    ("Video – MKV (nur umpacken)", "mkv", &["-c", "copy"]),
    ("Audio – MP3 (320 kbit/s)", "mp3", &["-vn", "-c:a", "libmp3lame", "-b:a", "320k"]),
    ("Audio – FLAC (verlustfrei)", "flac", &["-vn", "-c:a", "flac"]),
    ("Audio – OGG (Vorbis)", "ogg", &["-vn", "-c:a", "libvorbis", "-q:a", "6"]),
    ("Animiertes GIF (480p, 12 fps)", "gif", &["-vf", "fps=12,scale=480:-1:flags=lanczos", "-loop", "0"]),
];

pub struct App {
    pub t: Theme,
    pub data: Data,
    pub dirty: bool,
    last_save: Instant,
    pub player: Option<Player>,
    pub mpv_error: Option<String>,
    pub video: Option<VideoRenderer>,
    pub tools: MediaTools,
    pub bindings: Bindings,
    pub lib: LibraryState,
    pub view: View,
    pub right: RightPanel,
    pub adjust_tab: AdjustTab,
    pub track_tab: TrackTab,
    pub dialog: Option<Dialog>,
    pub settings_page: SettingsPage,
    pub settings_search: String,
    pub osd: Option<Osd>,
    pub resume_prompt: Option<ResumePrompt>,
    pub ui_hidden: bool,
    pub pip: Option<PipState>,
    pub last_pointer_move: Instant,
    pub last_pointer_pos: Option<egui::Pos2>,
    pub bars_shown_at: Instant,
    /// Touch layout: player controls shown (tap on the video toggles).
    pub touch_bars: bool,
    pub seek_drag: Option<f64>,
    /// Chapter snapping on the seek bar (counts per chapter, current file).
    pub seek_snap: crate::ui_player::SnapState,
    pub seek_thumb: Option<(String, TextureHandle)>,
    pub seek_thumb_req: Option<String>,
    pub thumbs: HashMap<String, ThumbState>,
    pub thumb_keys: HashMap<String, String>,
    pub waveform: Option<(String, Vec<f32>)>,
    waveform_req: Option<String>,
    pub cues: Option<(String, Vec<Cue>)>,
    cues_req: Option<String>,
    pub sleep_at: Option<Instant>,
    /// Sleep at the end of the title that was playing when this was set.
    pub sleep_eof: bool,
    pub sleep_eof_path: String,
    /// Volume before the sleep fade-out started.
    sleep_fade_from: Option<f64>,
    last_history: Instant,
    pub eq_enabled: bool,
    pub eq_preset: String,
    pub eq_bands: [f64; 10],
    pub eq_preamp: f64,
    pub fx_normalize: bool,
    pub fx_night: bool,
    pub fx_mono: bool,
    pub fx_widen: bool,
    pub video_preset: String,
    pub flip_h: bool,
    pub flip_v: bool,
    pub rebinding: Option<&'static str>,
    pub dlg: DialogBuffers,
    pub loaded_path: String,
    pub system_dark: bool,
    pub theme_dirty: bool,
    pub last_title: String,
    pub sub_size_pixels: bool,
    pub eq_open: bool,
    pub info_open: bool,
    pub drag_hover_files: bool,
    pub quit_requested: bool,
    pub was_fullscreen: bool,
    /// Whole window before the safe-area insets (touch layout).
    pub full_screen: egui::Rect,
    /// Safe-area insets of the last frame [left, top, right, bottom].
    pub safe_insets: [f32; 4],
    pub launch_files: Vec<String>,
    pub pending_resume_restore: Option<config::ResumeMode>,
    pub prev_has_media: bool,
    /// Phone player state (gestures, zoom, brightness).
    pub mob: crate::ui_mobile::MobileState,
    pub plex: crate::plex::Plex,
    pub px: crate::ui_plex::PlexUi,
    /// In-app updates from GitHub releases.
    pub update: crate::update::Updater,
    /// Public TV channel list (iptv-org), loaded on first use.
    pub tv: crate::iptv::Tv,
    /// Program guides for public TV, per country.
    pub epg: crate::epg::Epg,
    /// Public radio directory.
    pub radio: crate::radio::Radio,
    /// Radio station logos.
    pub logos: crate::plex::images::Images,
    /// Stream URL -> logo URL of radio stations put in the queue.
    pub stream_logos: HashMap<String, String>,
    /// Stream URLs of radio stations put in the queue (live, no end).
    pub radio_urls: std::collections::HashSet<String>,
}

fn init_mpv(s: &Settings) -> Result<Arc<Mpv>, String> {
    let mpv = Mpv::create()?;
    let opts: Vec<(&str, String)> = vec![
        ("vo", "libmpv".into()),
        ("hwdec", if s.hwdec { crate::platform::HWDEC.into() } else { "no".into() }),
        ("keep-open", "yes".into()),
        ("idle", "yes".into()),
        ("terminal", "no".into()),
        ("config", "no".into()),
        ("load-scripts", "no".into()),
        ("input-default-bindings", "no".into()),
        ("input-vo-keyboard", "no".into()),
        ("osc", "no".into()),
        ("osd-level", "0".into()),
        ("osd-bar", "no".into()),
        ("ytdl", if s.ytdl { "yes".into() } else { "no".into() }),
        ("sub-auto", s.sub_auto.mpv().into()),
        ("audio-file-auto", "fuzzy".into()),
        ("volume", format!("{}", if s.remember_volume { s.last_volume } else { s.default_volume })),
        ("volume-max", format!("{}", s.max_volume.max(100.0))),
        ("screenshot-directory", s.screenshot_dir.to_string_lossy().to_string()),
        ("screenshot-format", s.screenshot_format.clone()),
        ("cache-secs", format!("{}", s.cache_secs)),
        ("demuxer-max-bytes", format!("{}MiB", s.demuxer_max_mb)),
        ("network-timeout", format!("{}", s.network_timeout)),
        ("deinterlace", if s.deinterlace { "yes".into() } else { "no".into() }),
        ("keepaspect", if s.keep_aspect { "yes".into() } else { "no".into() }),
        ("audio-channels", s.audio_channels.clone()),
        ("video-sync", if s.video_sync_display { "display-resample".into() } else { "audio".into() }),
        ("sub-font", s.sub_font.clone()),
        ("speed", format!("{}", s.default_speed)),
    ];
    for (k, v) in &opts {
        let _ = mpv.set_option(k, v);
    }
    if !s.user_agent.is_empty() {
        let _ = mpv.set_option("user-agent", &s.user_agent);
    }
    if !s.alang.is_empty() {
        let _ = mpv.set_option("alang", &s.alang);
    }
    if !s.slang.is_empty() {
        let _ = mpv.set_option("slang", &s.slang);
    }
    let _ = std::fs::create_dir_all(&s.screenshot_dir);
    mpv.initialize()?;
    Ok(Arc::new(mpv))
}

fn color_hex(c: [u8; 4]) -> String {
    format!("#{:02X}{:02X}{:02X}{:02X}", c[3], c[0], c[1], c[2])
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, launch_files: Vec<String>) -> Self {
        let ctx = cc.egui_ctx.clone();
        crate::theme::setup_fonts(&ctx);
        // One pass per frame: a discarded pass would replay clicks and run actions twice.
        ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(1).unwrap());
        let mut data = Data::load();
        crate::i18n::apply(data.settings.language);
        let (plex, plex_dirty) = crate::plex::Plex::new(&ctx, &mut data.plex);
        let system_dark = cc.egui_ctx.system_theme().map(|t| t == egui::Theme::Dark).unwrap_or(true);
        let t = Theme::from_settings(&data.settings, system_dark);
        t.apply(&ctx, data.settings.ui_scale);

        let (player, video, mpv_error) = match init_mpv(&data.settings) {
            Ok(mpv) => {
                let c2 = ctx.clone();
                mpv.set_wakeup_callback(Box::new(move || c2.request_repaint()));
                let video = VideoRenderer::new(mpv.clone(), ctx.clone());
                (Some(Player::new(mpv)), Some(video), None)
            }
            Err(e) => (None, None, Some(e)),
        };
        let tools = MediaTools::new(ctx.clone());
        let bindings = Bindings::from_overrides(&data.hotkeys);
        let mut lib = LibraryState::new();
        // Phones start with the list (the toolbar button switches to tiles).
        if cfg!(target_os = "android") {
            lib.view = crate::library::ViewMode::List;
        }
        if data.settings.scan_on_start {
            lib.start_music_scan(&data, &ctx);
        }
        let show_onboarding = !data.settings.onboarding_done;
        let mut app = App {
            t,
            dirty: plex_dirty,
            plex,
            px: Default::default(),
            last_save: Instant::now(),
            player,
            mpv_error: mpv_error.clone(),
            video,
            tools,
            bindings,
            lib,
            view: View::Library,
            right: RightPanel::None,
            adjust_tab: AdjustTab::Subtitles,
            track_tab: TrackTab::Subtitles,
            dialog: None,
            settings_page: SettingsPage::General,
            settings_search: String::new(),
            osd: None,
            resume_prompt: None,
            ui_hidden: false,
            pip: None,
            last_pointer_move: Instant::now(),
            last_pointer_pos: None,
            bars_shown_at: Instant::now(),
            touch_bars: true,
            seek_drag: None,
            seek_snap: Default::default(),
            seek_thumb: None,
            seek_thumb_req: None,
            thumbs: HashMap::new(),
            thumb_keys: HashMap::new(),
            waveform: None,
            waveform_req: None,
            cues: None,
            cues_req: None,
            sleep_at: None,
            sleep_eof: false,
            sleep_eof_path: String::new(),
            sleep_fade_from: None,
            last_history: Instant::now(),
            eq_enabled: false,
            eq_preset: "Flat".into(),
            eq_bands: [0.0; 10],
            eq_preamp: 0.0,
            fx_normalize: data.settings.normalize,
            fx_night: false,
            fx_mono: false,
            fx_widen: false,
            video_preset: "Keiner".into(),
            flip_h: false,
            flip_v: false,
            rebinding: None,
            dlg: DialogBuffers { url_save: true, ..Default::default() },
            loaded_path: String::new(),
            system_dark,
            theme_dirty: false,
            last_title: String::new(),
            sub_size_pixels: data.settings.sub_size_pixels,
            eq_open: false,
            info_open: false,
            drag_hover_files: false,
            quit_requested: false,
            was_fullscreen: false,
            full_screen: egui::Rect::NOTHING,
            safe_insets: [0.0; 4],
            launch_files,
            pending_resume_restore: None,
            prev_has_media: false,
            mob: Default::default(),
            update: Default::default(),
            tv: crate::iptv::Tv::new(ctx.clone()),
            epg: crate::epg::Epg::new(ctx.clone()),
            radio: crate::radio::Radio::new(ctx.clone()),
            logos: crate::plex::images::Images::web(ctx.clone(), "logo_img", 192),
            stream_logos: HashMap::new(),
            radio_urls: Default::default(),
            data,
        };
        app.apply_sub_style_from_settings();
        app.apply_audio_filters();
        app.apply_repeat();
        // First start of a new version: show what changed (not on a fresh install).
        let last = std::mem::replace(&mut app.data.settings.last_run_version, crate::update::CURRENT.to_string());
        if last != crate::update::CURRENT {
            app.dirty = true;
        }
        let whats_new = if !last.is_empty() && crate::update::newer(crate::update::CURRENT, &last) { Some(crate::update::changes_since(&last)).filter(|s| !s.is_empty()) } else { None };
        if let Some(e) = mpv_error {
            app.dialog = Some(Dialog::Error(e));
        } else if show_onboarding {
            app.dialog = Some(Dialog::Onboarding(0));
        } else if let Some(notes) = whats_new {
            app.dialog = Some(Dialog::WhatsNew(notes));
        }
        if app.update.restore() {
            app.update.prompt = true;
        }
        if app.data.settings.always_on_top {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
        }
        app
    }

    // ---------------------------------------------------------------- updates

    /// Daily background check on start, results of update work, and the
    /// prompt for a found release (held back while something plays).
    fn update_tick(&mut self, ctx: &egui::Context) {
        use crate::update::Status;
        if !self.update.auto_done && ctx.cumulative_pass_nr() > 2 {
            self.update.auto_done = true;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let s = &self.data.settings;
            if s.update_auto && now.saturating_sub(s.update_last_check) > 20 * 3600 && matches!(self.update.status, Status::Idle) {
                self.data.settings.update_last_check = now;
                self.mark_dirty();
                self.update.check(ctx, false);
            }
        }
        if self.update.poll() {
            let skipped = matches!(&self.update.status, Status::Available(r) if r.version == self.data.settings.update_skip);
            if self.update.manual {
                self.dialog = Some(Dialog::Update);
            } else if !skipped {
                self.update.prompt = true;
            }
        }
        if self.update.prompt && self.dialog.is_none() && !(self.view == View::Player && self.player_active()) {
            self.update.prompt = false;
            self.dialog = Some(Dialog::Update);
        }
    }

    // ---------------------------------------------------------------- utils

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn osd(&mut self, text: impl Into<String>) {
        self.osd = Some(Osd { chips: vec![], text: text.into(), until: Instant::now() + Duration::from_millis(1600) });
    }

    pub fn osd_keys(&mut self, combo: Option<Combo>, text: impl Into<String>) {
        let chips = if self.data.settings.osd_show_keys { combo.map(|c| c.chips()).unwrap_or_default() } else { vec![] };
        self.osd = Some(Osd { chips, text: text.into(), until: Instant::now() + Duration::from_millis(1600) });
    }

    pub fn has_media(&self) -> bool {
        self.player.as_ref().map(|p| p.st.has_media()).unwrap_or(false)
    }

    /// Playing, or mpv is loading the next playlist entry (no media for a
    /// moment, but not idle either).
    pub fn player_active(&self) -> bool {
        self.player.as_ref().is_some_and(|p| p.st.has_media() || !p.st.idle)
    }

    pub fn current_path(&self) -> String {
        self.player.as_ref().map(|p| p.st.path.clone()).unwrap_or_default()
    }

    pub fn current_folder(&self) -> Option<std::path::PathBuf> {
        let p = self.current_path();
        if p.is_empty() || media::is_url(&p) {
            return None;
        }
        std::path::Path::new(&p).parent().map(|d| d.to_path_buf())
    }

    pub fn rebuild_theme(&mut self, ctx: &egui::Context) {
        self.t = Theme::from_settings(&self.data.settings, self.system_dark);
        self.t.apply(ctx, self.data.settings.ui_scale);
    }

    // ------------------------------------------------------------- opening

    pub fn open_paths(&mut self, paths: Vec<String>) {
        let paths: Vec<String> = paths.into_iter().filter(|p| !p.trim().is_empty()).collect();
        if paths.is_empty() || self.player.is_none() {
            return;
        }
        let (subs, media): (Vec<String>, Vec<String>) = paths.into_iter().partition(|p| media::is_sub(p));
        if !subs.is_empty() && self.has_media() {
            for s in &subs {
                self.player.as_ref().unwrap().sub_add(s, true);
            }
            self.osd(tf!("Untertitel hinzugefügt: {}", media::display_name(&subs[0])));
        }
        if media.is_empty() {
            return;
        }
        let mut files = Vec::new();
        for p in media {
            let path = std::path::Path::new(&p);
            if path.is_dir() {
                let (_, f) = media::list_dir(path, false);
                files.extend(f.into_iter().map(|x| x.to_string_lossy().to_string()));
            } else {
                files.push(p);
            }
        }
        if files.is_empty() {
            self.osd(t!("Keine abspielbaren Dateien gefunden"));
            return;
        }
        if files.len() == 1 && self.data.settings.play_folder_on_open && !media::is_url(&files[0]) && !media::is_playlist(&files[0]) {
            let sib = media::sibling_media(&files[0]);
            if let Some(idx) = sib.iter().position(|s| s == &files[0]) {
                self.play_list(&sib, idx);
                return;
            }
        }
        self.play_list(&files, 0);
    }

    /// Replace the queue with `list` and start playing at `start`.
    pub fn play_list(&mut self, list: &[String], start: usize) {
        let Some(p) = &self.player else { return };
        if list.is_empty() {
            return;
        }
        let start = start.min(list.len() - 1);
        p.load(&list[start], LoadMode::Replace, None);
        for (i, item) in list.iter().enumerate().take(start) {
            let _ = p.mpv.command(&["loadfile", item, "insert-at", &i.to_string()]);
        }
        for item in list.iter().skip(start + 1) {
            p.load(item, LoadMode::Append, None);
        }
        if !self.data.settings.autoplay_on_open {
            p.set_pause(true);
        } else {
            p.set_pause(false);
        }
        // Music stays in the library (mini player); there is nothing to watch.
        if !media::is_audio(&list[start]) {
            self.view = View::Player;
        }
    }

    pub fn enqueue(&mut self, list: &[String]) {
        let Some(p) = &self.player else { return };
        let was_idle = !p.st.has_media();
        for (i, item) in list.iter().enumerate() {
            if was_idle && i == 0 {
                p.load(item, LoadMode::AppendPlay, None);
            } else {
                p.load(item, LoadMode::Append, None);
            }
        }
        let n = list.len();
        self.osd(if n == 1 { tf!("Zur Warteschlange hinzugefügt: {}", media::display_name(&list[0])) } else { tf!("{n} Titel zur Warteschlange hinzugefügt") });
    }

    pub fn play_next(&mut self, list: &[String]) {
        let Some(p) = &self.player else { return };
        for item in list.iter().rev() {
            let _ = p.mpv.command(&["loadfile", item, "insert-next"]);
        }
        self.osd(t!("Wird als Nächstes abgespielt"));
    }

    pub fn dialog_open_files(&mut self) {
        let mut d = rfd::FileDialog::new().set_title(t!("Medien öffnen"));
        let mut all: Vec<&str> = media::VIDEO_EXT.to_vec();
        all.extend_from_slice(media::AUDIO_EXT);
        all.extend_from_slice(media::PLAYLIST_EXT);
        d = d.add_filter(t!("Medien"), &all).add_filter("Video", media::VIDEO_EXT).add_filter("Audio", media::AUDIO_EXT).add_filter(t!("Alle Dateien"), &["*"]);
        if let Some(dir) = self.current_folder() {
            d = d.set_directory(dir);
        }
        if let Some(files) = d.pick_files() {
            self.open_paths(files.into_iter().map(|p| p.to_string_lossy().to_string()).collect());
        }
    }

    pub fn dialog_open_folder(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().set_title(t!("Ordner öffnen")).pick_folder() {
            self.open_paths(vec![dir.to_string_lossy().to_string()]);
        }
    }

    pub fn dialog_add_to_queue(&mut self) {
        let mut all: Vec<&str> = media::VIDEO_EXT.to_vec();
        all.extend_from_slice(media::AUDIO_EXT);
        if let Some(files) = rfd::FileDialog::new().add_filter(t!("Medien"), &all).add_filter(t!("Alle Dateien"), &["*"]).pick_files() {
            let v: Vec<String> = files.into_iter().map(|p| p.to_string_lossy().to_string()).collect();
            self.enqueue(&v);
        }
    }

    pub fn open_clipboard(&mut self) {
        let text = crate::platform::clipboard_text();
        let text = text.trim().trim_matches('"').to_string();
        if text.is_empty() {
            self.osd(t!("Zwischenablage ist leer"));
            return;
        }
        let items: Vec<String> = text.lines().map(|l| l.trim().trim_matches('"').to_string()).filter(|l| !l.is_empty()).collect();
        self.open_paths(items);
    }

    pub fn dialog_add_subtitle_file(&mut self) {
        let mut d = rfd::FileDialog::new().add_filter(t!("Untertitel"), media::SUB_EXT);
        if let Some(dir) = self.current_folder() {
            d = d.set_directory(dir);
        }
        if let Some(f) = d.pick_file() {
            if let Some(p) = &self.player {
                p.sub_add(&f.to_string_lossy(), true);
            }
            self.osd(tf!("Untertitel geladen: {}", media::display_name(&f.to_string_lossy())));
        }
    }

    // ------------------------------------------------------------ actions

    pub fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        if self.pip.is_some() {
            self.toggle_pip(ctx);
        }
        let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fs));
    }

    pub fn toggle_pip(&mut self, ctx: &egui::Context) {
        if let Some(st) = self.pip.take() {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(if self.data.settings.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            }));
            if st.was_maximized {
                ctx.send_viewport_cmd(ViewportCommand::Maximized(true));
            } else {
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(st.prev_size));
                if let Some(p) = st.prev_pos {
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(p));
                }
            }
        } else {
            let (size, pos, maxed, fs, monitor) = ctx.input(|i| {
                let v = i.viewport();
                (
                    v.inner_rect.map(|r| r.size()).unwrap_or(egui::vec2(1280.0, 720.0)),
                    v.outer_rect.map(|r| r.min),
                    v.maximized.unwrap_or(false),
                    v.fullscreen.unwrap_or(false),
                    v.monitor_size,
                )
            });
            if fs {
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
            }
            if maxed {
                ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
            }
            let st = &self.player.as_ref().map(|p| p.st.clone()).unwrap_or_default();
            let aspect = if st.video_w > 0 && st.video_h > 0 { st.video_w as f32 / st.video_h as f32 } else { 16.0 / 9.0 };
            let w = 480.0;
            let new_size = egui::vec2(w, (w / aspect).round());
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(new_size));
            if let Some(m) = monitor {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(m.x - new_size.x - 32.0, m.y - new_size.y - 72.0)));
            }
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            self.pip = Some(PipState { prev_size: size, prev_pos: pos, was_maximized: maxed });
            self.view = View::Player;
        }
    }

    pub fn add_bookmark(&mut self, combo: Option<Combo>) {
        let Some(p) = &self.player else { return };
        if !p.st.has_media() {
            return;
        }
        let path = p.st.path.clone();
        let t = p.st.time;
        let list = self.data.bookmarks.entry(path).or_default();
        let name = tf!("Lesezeichen {}", list.len() + 1);
        list.push(Bookmark { time: t, name });
        list.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
        self.mark_dirty();
        self.osd_keys(combo, tf!("Lesezeichen bei {} hinzugefügt", media::fmt_time(t)));
    }

    pub fn set_repeat(&mut self, r: RepeatMode) {
        self.data.settings.repeat = r;
        self.apply_repeat();
        self.mark_dirty();
    }

    pub fn apply_repeat(&self) {
        let Some(p) = &self.player else { return };
        match self.data.settings.repeat {
            RepeatMode::Off => {
                p.set_str("loop-playlist", "no");
                p.set_str("loop-file", "no");
            }
            RepeatMode::All => {
                p.set_str("loop-playlist", "inf");
                p.set_str("loop-file", "no");
            }
            RepeatMode::One => {
                p.set_str("loop-playlist", "no");
                p.set_str("loop-file", "inf");
            }
        }
    }

    pub fn cycle_ab_loop(&mut self, combo: Option<Combo>) {
        let Some(p) = &self.player else { return };
        let _ = p.mpv.command(&["ab-loop"]);
        let msg = match (p.mpv.get_f64("ab-loop-a"), p.mpv.get_f64("ab-loop-b")) {
            (Some(a), Some(b)) => tf!("A-B-Schleife: {} – {}", media::fmt_time(a), media::fmt_time(b)),
            (Some(a), None) => tf!("Schleifenstart A: {}", media::fmt_time(a)),
            _ => t!("A-B-Schleife entfernt").into(),
        };
        self.osd_keys(combo, msg);
    }

    pub fn run_action(&mut self, ctx: &egui::Context, a: Action, combo: Option<Combo>) {
        let s = self.data.settings.clone();
        let name = crate::i18n::tr(hotkeys::info(a).name);
        match a {
            Action::OpenFile => return self.dialog_open_files(),
            Action::OpenFolder => return self.dialog_open_folder(),
            Action::OpenNetwork => {
                self.dialog = Some(Dialog::Network);
                return;
            }
            Action::OpenClipboard => return self.open_clipboard(),
            Action::Settings => {
                self.dialog = Some(Dialog::Settings);
                return;
            }
            Action::Hotkeys => {
                self.dialog = Some(Dialog::Hotkeys);
                return;
            }
            Action::Quit => {
                ctx.send_viewport_cmd(ViewportCommand::Close);
                return;
            }
            Action::Library => {
                if self.view == View::Library && self.has_media() {
                    self.view = View::Player;
                } else {
                    self.lib.compact_open = !self.lib.compact_open;
                }
                return;
            }
            Action::Queue => {
                self.right = if self.right == RightPanel::Queue { RightPanel::None } else { RightPanel::Queue };
                return;
            }
            Action::Sidebar => {
                self.right = if self.right == RightPanel::Adjust { RightPanel::None } else { RightPanel::Adjust };
                return;
            }
            Action::Fullscreen => {
                self.toggle_fullscreen(ctx);
                return;
            }
            Action::ExitFullscreen => {
                if self.dialog.is_some() {
                    self.dialog = None;
                } else if self.pip.is_some() {
                    self.toggle_pip(ctx);
                } else if self.ui_hidden {
                    self.ui_hidden = false;
                } else if ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
                    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                } else if self.right != RightPanel::None {
                    self.right = RightPanel::None;
                }
                return;
            }
            Action::Pip => {
                self.toggle_pip(ctx);
                return;
            }
            Action::HideUi => {
                self.ui_hidden = !self.ui_hidden;
                let msg = if self.ui_hidden { t!("Oberfläche ausgeblendet – erneut drücken zum Einblenden") } else { t!("Oberfläche eingeblendet") };
                self.osd_keys(combo, msg);
                return;
            }
            Action::Bookmark => return self.add_bookmark(combo),
            Action::LoopAB => return self.cycle_ab_loop(combo),
            _ => {}
        }
        if matches!(a, Action::Next | Action::Prev) {
            self.mob.skip_at = Some(std::time::Instant::now());
        }
        let Some(p) = &self.player else { return };
        let st = p.st.clone();
        let msg: String = match a {
            Action::PlayPause => {
                p.toggle_pause();
                if st.paused { t!("Wiedergabe").into() } else { "Pause".into() }
            }
            Action::Stop => {
                p.stop();
                self.view = View::Library;
                t!("Gestoppt").into()
            }
            Action::Next => {
                p.next();
                t!("Nächster Titel").into()
            }
            Action::Prev => {
                p.prev();
                t!("Vorheriger Titel").into()
            }
            Action::SeekFwd => {
                p.seek_rel(s.seek_small);
                format!("+{}s  ·  {}", s.seek_small, media::fmt_time(st.time + s.seek_small))
            }
            Action::SeekBack => {
                p.seek_rel(-s.seek_small);
                format!("−{}s  ·  {}", s.seek_small, media::fmt_time((st.time - s.seek_small).max(0.0)))
            }
            Action::SeekFwdLarge => {
                p.seek_rel(s.seek_large);
                format!("+{}s  ·  {}", s.seek_large, media::fmt_time(st.time + s.seek_large))
            }
            Action::SeekBackLarge => {
                p.seek_rel(-s.seek_large);
                format!("−{}s  ·  {}", s.seek_large, media::fmt_time((st.time - s.seek_large).max(0.0)))
            }
            Action::FrameStep => {
                p.frame_step();
                t!("Nächstes Einzelbild").into()
            }
            Action::FrameBack => {
                p.frame_back_step();
                t!("Vorheriges Einzelbild").into()
            }
            Action::VolUp => {
                let v = (st.volume + s.volume_step).min(s.max_volume);
                p.set_volume(v);
                tf!("Lautstärke {v:.0}%")
            }
            Action::VolDown => {
                let v = (st.volume - s.volume_step).max(0.0);
                p.set_volume(v);
                tf!("Lautstärke {v:.0}%")
            }
            Action::Mute => {
                p.set_mute(!st.muted);
                if st.muted { t!("Ton an").into() } else { t!("Stumm").into() }
            }
            Action::ShowTime => format!("{} / {}", media::fmt_time(st.time), media::fmt_time(st.duration)),
            Action::SpeedUp => {
                let v = ((st.speed + 0.25) * 4.0).round() / 4.0;
                p.set_speed(v);
                tf!("Geschwindigkeit {v:.2}×")
            }
            Action::SpeedDown => {
                let v = (((st.speed - 0.25) * 4.0).round() / 4.0).max(0.25);
                p.set_speed(v);
                tf!("Geschwindigkeit {v:.2}×")
            }
            Action::SpeedReset => {
                p.set_speed(1.0);
                t!("Geschwindigkeit 1.00×").into()
            }
            Action::SubDelayMinus => {
                let v = st.sub_delay - 0.1;
                p.set("sub-delay", v);
                tf!("Untertitelverzögerung {:.0} ms", v * 1000.0)
            }
            Action::SubDelayPlus => {
                let v = st.sub_delay + 0.1;
                p.set("sub-delay", v);
                tf!("Untertitelverzögerung {:.0} ms", v * 1000.0)
            }
            Action::AudioDelayMinus => {
                let v = st.audio_delay - 0.05;
                p.set("audio-delay", v);
                tf!("Audioverzögerung {:.0} ms", v * 1000.0)
            }
            Action::AudioDelayPlus => {
                let v = st.audio_delay + 0.05;
                p.set("audio-delay", v);
                tf!("Audioverzögerung {:.0} ms", v * 1000.0)
            }
            Action::CycleSub => {
                p.cycle("sid");
                let t = p.mpv.get("current-tracks/sub/title").and_then(|v| v.as_str().map(|s| s.to_string()));
                let l = p.mpv.get("current-tracks/sub/lang").and_then(|v| v.as_str().map(|s| media::language_name(s)));
                match (t, l) {
                    (Some(t), _) => tf!("Untertitel: {t}"),
                    (None, Some(l)) => tf!("Untertitel: {l}"),
                    _ => t!("Untertitel: aus/gewechselt").into(),
                }
            }
            Action::ToggleSubs => {
                p.set_flag("sub-visibility", !st.sub_visible);
                if st.sub_visible { t!("Untertitel ausgeblendet").into() } else { t!("Untertitel eingeblendet").into() }
            }
            Action::CycleAudio => {
                p.cycle("aid");
                t!("Audiospur gewechselt").into()
            }
            Action::LoopFile => {
                let r = if s.repeat == RepeatMode::One { RepeatMode::Off } else { RepeatMode::One };
                self.set_repeat(r);
                if r == RepeatMode::One { t!("Datei wird wiederholt").into() } else { t!("Wiederholung aus").into() }
            }
            Action::Screenshot => {
                p.screenshot();
                tf!("Screenshot gespeichert in {}", s.screenshot_dir.display())
            }
            _ => name.into(),
        };
        self.osd_keys(combo, msg);
    }

    // ---------------------------------------------------- filters/presets

    pub fn apply_audio_filters(&mut self) {
        let Some(p) = &self.player else { return };
        let mut parts: Vec<String> = Vec::new();
        if self.eq_enabled {
            if self.eq_preamp.abs() > 0.01 {
                parts.push(format!("volume={:.1}dB", self.eq_preamp));
            }
            for (i, g) in self.eq_bands.iter().enumerate() {
                if g.abs() > 0.01 {
                    parts.push(format!("equalizer=f={}:t=o:w=1:g={:.1}", EQ_FREQS[i], g));
                }
            }
        }
        if self.fx_night {
            parts.push("acompressor=threshold=0.05:ratio=6:attack=20:release=250:makeup=3".into());
        }
        if self.fx_normalize {
            parts.push("dynaudnorm=f=300:g=15".into());
        }
        if self.fx_widen {
            parts.push("extrastereo=m=1.8".into());
        }
        let chain = if parts.is_empty() { String::new() } else { format!("lavfi=[{}]", parts.join(",")) };
        let _ = p.mpv.set_str("af", &chain);
        let ch = if self.fx_mono { "mono".to_string() } else { self.data.settings.audio_channels.clone() };
        let _ = p.mpv.set_str("audio-channels", &ch);
    }

    pub fn apply_video_filters(&mut self) {
        let Some(p) = &self.player else { return };
        let mut vf: Vec<String> = Vec::new();
        if self.flip_h {
            vf.push("hflip".into());
        }
        if self.flip_v {
            vf.push("vflip".into());
        }
        if let Some(pr) = VIDEO_PRESETS.iter().find(|x| x.0 == self.video_preset) {
            if !pr.2.is_empty() {
                vf.push(format!("lavfi=[{}]", pr.2));
            }
        }
        let _ = p.mpv.set_str("vf", &vf.join(","));
    }

    pub fn set_video_preset(&mut self, name: &str) {
        self.video_preset = name.to_string();
        if let (Some(p), Some(pr)) = (&self.player, VIDEO_PRESETS.iter().find(|x| x.0 == name)) {
            for (prop, v) in ["brightness", "contrast", "saturation", "gamma", "hue"].iter().zip(pr.1) {
                p.set(prop, v);
            }
        }
        self.apply_video_filters();
    }

    pub fn set_eq_preset(&mut self, name: &str) {
        if let Some((_, b)) = EQ_PRESETS.iter().find(|(n, _)| *n == name) {
            self.eq_bands = *b;
            self.eq_preset = name.to_string();
            self.eq_enabled = name != "Flat" || self.eq_enabled;
            self.apply_audio_filters();
        }
    }

    pub fn apply_sub_style_from_settings(&mut self) {
        let s = &self.data.settings;
        let pr = Preset {
            sub_font: s.sub_font.clone(),
            sub_font_size: s.sub_font_size,
            sub_scale: 1.0,
            sub_size_pixels: s.sub_size_pixels,
            sub_color: s.sub_color,
            sub_border_color: s.sub_border_color,
            sub_border_size: s.sub_border_size,
            sub_back_opacity: s.sub_back_opacity,
            sub_pos: s.sub_pos,
            sub_bold: s.sub_bold,
            ..Default::default()
        };
        self.apply_sub_style(&pr);
    }

    pub fn apply_sub_style(&self, pr: &Preset) {
        let Some(p) = &self.player else { return };
        let m = &p.mpv;
        let _ = m.set_str("sub-font", &pr.sub_font);
        let _ = m.set_f64("sub-font-size", pr.sub_font_size);
        let _ = m.set_f64("sub-scale", pr.sub_scale);
        let _ = m.set_str("sub-color", &color_hex(pr.sub_color));
        if m.set_str("sub-outline-color", &color_hex(pr.sub_border_color)).is_err() {
            let _ = m.set_str("sub-border-color", &color_hex(pr.sub_border_color));
        }
        if m.set_f64("sub-outline-size", pr.sub_border_size).is_err() {
            let _ = m.set_f64("sub-border-size", pr.sub_border_size);
        }
        if pr.sub_back_opacity > 0.01 {
            let a = (pr.sub_back_opacity.clamp(0.0, 1.0) * 255.0) as u8;
            let _ = m.set_str("sub-back-color", &color_hex([0, 0, 0, a]));
            let _ = m.set_str("sub-border-style", "background-box");
        } else {
            let _ = m.set_str("sub-border-style", "outline-and-shadow");
        }
        let _ = m.set_f64("sub-pos", pr.sub_pos);
        let _ = m.set_flag("sub-bold", pr.sub_bold);
        // Make ASS subtitles follow the user style too.
        let _ = m.set_str("sub-ass-override", "force");
    }

    /// Captures the current subtitle/audio state as a preset.
    pub fn capture_preset(&self, name: &str) -> Preset {
        let s = &self.data.settings;
        let mut pr = Preset { name: name.to_string(), ..Default::default() };
        pr.sub_font = s.sub_font.clone();
        pr.sub_font_size = s.sub_font_size;
        pr.sub_size_pixels = s.sub_size_pixels;
        pr.sub_color = s.sub_color;
        pr.sub_border_color = s.sub_border_color;
        pr.sub_border_size = s.sub_border_size;
        pr.sub_back_opacity = s.sub_back_opacity;
        pr.sub_pos = s.sub_pos;
        pr.sub_bold = s.sub_bold;
        pr.eq_preset = if self.eq_enabled { self.eq_preset.clone() } else { "Flat".into() };
        pr.video_preset = self.video_preset.clone();
        if let Some(p) = &self.player {
            pr.sub_delay = p.st.sub_delay;
            pr.audio_delay = p.st.audio_delay;
            pr.sub_scale = p.st.sub_scale;
            pr.speed = p.st.speed;
            match p.st.selected("sub") {
                Some(t) => {
                    pr.sub_track_title = t.title.clone();
                    pr.sub_lang = t.lang.clone();
                }
                None => pr.sub_off = p.st.tracks_of("sub").count() > 0,
            }
            if let Some(t) = p.st.selected("audio") {
                pr.audio_track_title = t.title.clone();
                pr.audio_lang = t.lang.clone();
            }
        }
        pr
    }

    pub fn apply_preset(&mut self, pr: &Preset, include_tracks: bool) {
        {
            let s = &mut self.data.settings;
            s.sub_font = pr.sub_font.clone();
            s.sub_font_size = pr.sub_font_size;
            s.sub_size_pixels = pr.sub_size_pixels;
            s.sub_color = pr.sub_color;
            s.sub_border_color = pr.sub_border_color;
            s.sub_border_size = pr.sub_border_size;
            s.sub_back_opacity = pr.sub_back_opacity;
            s.sub_pos = pr.sub_pos;
            s.sub_bold = pr.sub_bold;
        }
        self.sub_size_pixels = pr.sub_size_pixels;
        self.apply_sub_style(pr);
        if let Some(p) = &self.player {
            p.set("sub-delay", pr.sub_delay);
            p.set("audio-delay", pr.audio_delay);
            if (pr.speed - 1.0).abs() > 0.001 {
                p.set_speed(pr.speed);
            }
        }
        if include_tracks {
            self.select_tracks_from_preset(pr);
        }
        if pr.eq_preset != "Flat" {
            self.set_eq_preset(&pr.eq_preset.clone());
        }
        if pr.video_preset != "Keiner" {
            self.set_video_preset(&pr.video_preset.clone());
        }
    }

    pub fn select_tracks_from_preset(&self, pr: &Preset) {
        let Some(p) = &self.player else { return };
        if pr.sub_off {
            p.set_track("sub", None);
        } else {
            let pick = p.st.tracks_of("sub").find(|t| pr.sub_track_title.is_some() && t.title == pr.sub_track_title && (pr.sub_lang.is_none() || t.lang == pr.sub_lang))
                .or_else(|| p.st.tracks_of("sub").find(|t| pr.sub_lang.is_some() && t.lang == pr.sub_lang));
            if let Some(t) = pick {
                p.set_track("sub", Some(t.id));
            }
        }
        let pick = p.st.tracks_of("audio").find(|t| pr.audio_track_title.is_some() && t.title == pr.audio_track_title && (pr.audio_lang.is_none() || t.lang == pr.audio_lang))
            .or_else(|| p.st.tracks_of("audio").find(|t| pr.audio_lang.is_some() && t.lang == pr.audio_lang));
        if let Some(t) = pick {
            p.set_track("audio", Some(t.id));
        }
    }

    pub fn folder_preset_name(&self) -> Option<String> {
        let f = self.current_folder()?;
        self.data.folder_presets.get(&f.to_string_lossy().to_string()).cloned()
    }

    // --------------------------------------------------------------- sleep

    /// Sleep timer: `Some(minutes)` from now, or `None` to turn it off.
    pub fn sleep_minutes(&mut self, m: Option<u64>) {
        self.sleep_eof = false;
        self.restore_sleep_volume();
        self.sleep_at = m.map(|m| Instant::now() + Duration::from_secs(m * 60));
        self.osd(match m {
            Some(m) => tf!("Sleep-Timer: {m} Minuten"),
            None => t!("Sleep-Timer aus").into(),
        });
    }

    /// Sleep timer: stop when the current episode/movie ends.
    pub fn sleep_end_of_title(&mut self) {
        self.sleep_at = None;
        self.restore_sleep_volume();
        self.sleep_eof = true;
        self.sleep_eof_path = self.player.as_ref().and_then(|p| p.mpv.get_string("path")).unwrap_or_default();
        self.osd(t!("Sleep-Timer: am Ende der Folge / des Films"));
    }

    pub fn sleep_active(&self) -> bool {
        self.sleep_at.is_some() || self.sleep_eof
    }

    /// Short state for buttons: "23:41" left, "Ende" or empty.
    pub fn sleep_label(&self) -> String {
        match (self.sleep_at, self.sleep_eof) {
            (Some(at), _) => {
                let r = at.saturating_duration_since(Instant::now()).as_secs();
                format!("{}:{:02}", r / 60, r % 60)
            }
            (None, true) => t!("Ende").into(),
            _ => String::new(),
        }
    }

    fn restore_sleep_volume(&mut self) {
        if let (Some(v), Some(p)) = (self.sleep_fade_from.take(), &self.player) {
            p.set_volume(v);
        }
    }

    fn sleep_done(&mut self) {
        self.restore_sleep_volume();
        self.osd(t!("Sleep-Timer: Wiedergabe angehalten"));
    }

    // --------------------------------------------------------------- frame

    fn on_file_loaded(&mut self) {
        let Some(p) = self.player.as_ref().map(|p| p.ctl.clone()) else { return };
        let path = p.mpv.get_string("path").unwrap_or_default();
        self.loaded_path = path.clone();
        self.cues = None;
        self.cues_req = None;
        self.seek_thumb = None;
        self.seek_thumb_req = None;
        self.resume_prompt = None;
        if self.sleep_eof && path != self.sleep_eof_path {
            // The next episode/file started: stop here.
            p.set_pause(true);
            self.sleep_eof = false;
            self.sleep_done();
        }
        if crate::plex::is_remote(&path) {
            // Plex keeps play state on the server; no local history or folder presets.
            if let Some(m) = self.pending_resume_restore.take() {
                self.data.settings.resume = m;
            }
            self.waveform = None;
            self.waveform_req = None;
            self.plex_file_loaded(&path);
            return;
        }
        if self.data.settings.remember_positions {
            if let Some(h) = self.data.history_for(&path) {
                if h.resumable() {
                    match self.data.settings.resume {
                        config::ResumeMode::Ask => {
                            self.resume_prompt = Some(ResumePrompt { path: path.clone(), pos: h.position, until: Instant::now() + Duration::from_secs(10) })
                        }
                        config::ResumeMode::Always => {
                            p.seek_abs(h.position, true);
                            let pos = h.position;
                            self.osd(tf!("Fortgesetzt bei {}", media::fmt_time(pos)));
                        }
                        config::ResumeMode::Never => {}
                    }
                }
            }
        }
        if let Some(m) = self.pending_resume_restore.take() {
            self.data.settings.resume = m;
        }
        if let Some(sub) = self.data.sub_map.get(&path).cloned() {
            if std::path::Path::new(&sub).is_file() {
                p.sub_add(&sub, true);
            }
        }
        // Per-show preset.
        if let Some(name) = self.folder_preset_name() {
            if let Some(pr) = self.data.preset(&name).cloned() {
                self.apply_preset(&pr, true);
            }
        }
        // Waveform for audio, or when the user enabled it.
        self.waveform = self.waveform.take().filter(|(p2, _)| *p2 == path);
        self.waveform_req = None;
        self.add_history(&path);
    }

    fn add_history(&mut self, path: &str) {
        if !self.data.settings.remember_history || path.is_empty() {
            return;
        }
        let title = self.player.as_ref().map(|p| p.st.media_title.clone()).unwrap_or_default();
        let audio_only = media::is_audio(path);
        let now = config::now_secs();
        if let Some(h) = self.data.history.iter_mut().find(|h| h.path == path) {
            h.last_played = now;
            if !title.is_empty() {
                h.title = title;
            }
            if h.finished {
                h.finished = false;
                h.position = 0.0;
            }
        } else {
            self.data.history.push(HistoryEntry {
                path: path.to_string(),
                title: if title.is_empty() { media::display_name(path) } else { title },
                position: 0.0,
                duration: 0.0,
                last_played: now,
                finished: false,
                audio_only,
            });
        }
        let limit = self.data.settings.history_limit.max(10);
        if self.data.history.len() > limit {
            self.data.history.sort_by(|a, b| b.last_played.cmp(&a.last_played));
            self.data.history.truncate(limit);
        }
        self.mark_dirty();
    }

    fn update_history_position(&mut self) {
        let Some(p) = &self.player else { return };
        let st = &p.st;
        if !st.has_media() || !self.data.settings.remember_history {
            return;
        }
        let (path, t, d) = (st.path.clone(), st.time, st.duration);
        if crate::plex::is_remote(&path) {
            return;
        }
        if let Some(h) = self.data.history.iter_mut().find(|h| h.path == path) {
            h.position = t;
            if d > 0.0 {
                h.duration = d;
            }
            if d > 0.0 && (t > d - 20.0 || t / d > 0.95) {
                h.finished = true;
            }
            if !st.media_title.is_empty() && !media::is_url(&path) {
                h.title = st.media_title.clone();
            }
            self.dirty = true;
        }
    }

    pub fn mark_watched(&mut self, path: &str, watched: bool) {
        if let Some(h) = self.data.history.iter_mut().find(|h| h.path == path) {
            h.finished = watched;
            if !watched {
                h.position = 0.0;
            }
        } else if watched {
            self.data.history.push(HistoryEntry {
                path: path.into(),
                title: media::display_name(path),
                finished: true,
                last_played: config::now_secs(),
                ..Default::default()
            });
        }
        self.mark_dirty();
    }

    pub fn ensure_cues(&mut self) {
        let Some(p) = &self.player else { return };
        let Some(t) = p.st.selected("sub").cloned() else {
            self.cues = None;
            return;
        };
        let key = format!("{}#{}", p.st.path, t.id);
        if self.cues_req.as_deref() == Some(&key) {
            return;
        }
        self.cues_req = Some(key.clone());
        let ext = if t.external { t.external_filename.clone() } else { None };
        self.tools.request_cues(key, &crate::plex::authorized(&p.st.path), ext, t.ff_index);
    }

    pub fn ensure_waveform(&mut self) {
        let path = self.current_path();
        if path.is_empty() || media::is_url(&path) {
            return;
        }
        if self.waveform.as_ref().map(|w| w.0 == path).unwrap_or(false) || self.waveform_req.as_deref() == Some(&path) {
            return;
        }
        self.waveform_req = Some(path.clone());
        self.tools.request_waveform(&path);
    }

    pub fn frame_start(&mut self, ctx: &egui::Context) {
        // mpv events
        let events = self.player.as_mut().map(|p| p.poll()).unwrap_or_default();
        for e in events {
            match e {
                Event::FileLoaded => self.on_file_loaded(),
                Event::EndFile { reason, error } if reason == mpv::END_ERROR => {
                    let msg = self.player.as_ref().map(|p| p.mpv.error_string(error)).unwrap_or_default();
                    self.osd(tf!("Datei konnte nicht abgespielt werden: {msg}"));
                }
                Event::Shutdown => ctx.send_viewport_cmd(ViewportCommand::Close),
                Event::PropertyChange { name, .. } if name == "volume" => {
                    if let Some(p) = &self.player {
                        if self.sleep_fade_from.is_none() && self.data.settings.remember_volume && (self.data.settings.last_volume - p.st.volume).abs() > 0.5 {
                            self.data.settings.last_volume = p.st.volume;
                            self.dirty = true;
                        }
                    }
                }
                Event::PropertyChange { name, .. } if name == "track-list" => {
                    if self.data.settings.show_sub_timeline {
                        self.cues_req = None;
                    }
                }
                _ => {}
            }
        }
        // background results
        while let Ok(r) = self.tools.results.try_recv() {
            match r {
                MediaResult::Probe(path, info) => self.tools.on_probe(path, info),
                MediaResult::Thumb { key, w, h, rgba } => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    let tex = ctx.load_texture(&key, img, egui::TextureOptions::LINEAR);
                    self.thumbs.insert(key, ThumbState::Ready(tex));
                }
                MediaResult::ThumbFailed(key) => {
                    self.thumbs.insert(key, ThumbState::Failed);
                }
                MediaResult::SeekThumb { key, w, h, rgba } => {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    let tex = ctx.load_texture("seek-thumb", img, egui::TextureOptions::LINEAR);
                    self.seek_thumb = Some((key, tex));
                }
                MediaResult::Waveform { path, peaks } => self.waveform = Some((path, peaks)),
                MediaResult::Cues { key, cues } => {
                    if self.cues_req.as_deref() == Some(&key) {
                        self.cues = Some((key, cues));
                    }
                }
                MediaResult::ConvertProgress { done, total } => self.dlg.convert_progress = Some((done, total)),
                MediaResult::ConvertDone(r) => {
                    self.dlg.convert_progress = None;
                    let msg = match &r {
                        Ok(p) => tf!("Konvertierung fertig: {}", media::display_name(p)),
                        Err(_) => t!("Konvertierung fehlgeschlagen").into(),
                    };
                    self.dlg.convert_result = Some(r);
                    self.osd(msg);
                }
            }
        }
        self.lib.poll_scan();
        self.plex_frame(ctx);

        if self.data.settings.show_sub_timeline && self.has_media() {
            self.ensure_cues();
        }
        let audio_only = self.player.as_ref().map(|p| p.st.is_audio_only()).unwrap_or(false);
        if (self.data.settings.show_waveform || audio_only || self.data.settings.show_sub_timeline) && self.has_media() && self.tools.available() {
            self.ensure_waveform();
        }

        // sleep timer
        if let Some(at) = self.sleep_at {
            let left = at.saturating_duration_since(Instant::now()).as_secs_f64();
            if left <= 0.0 {
                self.sleep_at = None;
                if let Some(p) = &self.player {
                    p.set_pause(true);
                }
                self.sleep_done();
            } else {
                // Fade out over the last 30 seconds.
                if left < 30.0 {
                    if let Some(p) = &self.player {
                        let from = *self.sleep_fade_from.get_or_insert(p.st.volume);
                        p.set_volume((from * left / 30.0).max(0.0));
                    }
                    ctx.request_repaint_after(Duration::from_millis(250));
                } else {
                    ctx.request_repaint_after(Duration::from_secs(1));
                }
            }
        }
        if self.sleep_eof {
            if let Some(p) = &self.player {
                let st = &p.st;
                // Last file ended (keep-open leaves it paused at the end).
                if st.has_media() && st.duration > 0.0 && st.time >= st.duration - 1.0 && st.paused {
                    self.sleep_eof = false;
                    self.sleep_done();
                }
            }
        }
        #[cfg(target_os = "android")]
        {
            // Keep the screen on while something plays; let it sleep after.
            let playing = self.player.as_ref().is_some_and(|p| p.st.has_media() && !p.st.paused);
            crate::platform::android::keep_screen_on(playing);
        }

        if self.last_history.elapsed() > Duration::from_secs(3) {
            self.last_history = Instant::now();
            self.update_history_position();
        }
        if self.dirty && self.last_save.elapsed() > Duration::from_secs(3) {
            self.data.save();
            self.tools.save_cache();
            self.dirty = false;
            self.last_save = Instant::now();
        }

        // window title
        let title = self
            .player
            .as_ref()
            .filter(|p| p.st.has_media())
            .map(|p| format!("{} – Velour", p.st.media_title))
            .unwrap_or_else(|| "Velour Media Player".into());
        if title != self.last_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        // system theme follow
        if self.data.settings.theme == config::ThemeMode::System {
            let dark = ctx.system_theme().map(|t| t == egui::Theme::Dark).unwrap_or(true);
            if dark != self.system_dark {
                self.system_dark = dark;
                self.theme_dirty = true;
            }
        }
        if self.theme_dirty {
            self.theme_dirty = false;
            self.rebuild_theme(ctx);
        }

        // Files from the command line wait for mpv's render context (made at
        // the end of the first frame): opened before it, a video plays
        // without a picture.
        let video_ready = self.video.as_ref().is_none_or(|v| v.ready());
        if !self.launch_files.is_empty() && self.player.is_some() && video_ready {
            let files = std::mem::take(&mut self.launch_files);
            self.open_paths(files);
        }

        // keep time display fresh while playing
        if self.has_media() && !self.player.as_ref().map(|p| p.st.paused).unwrap_or(true) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    pub fn handle_input(&mut self, ctx: &egui::Context) {
        // drag and drop from the OS
        let (dropped, hovering) = ctx.input(|i| (i.raw.dropped_files.clone(), !i.raw.hovered_files.is_empty()));
        self.drag_hover_files = hovering;
        if !dropped.is_empty() {
            let paths: Vec<String> = dropped.iter().filter_map(|f| f.path.as_ref().map(|p| p.to_string_lossy().to_string())).collect();
            let all_subs = paths.iter().all(|p| media::is_sub(p));
            if all_subs || !self.has_media() {
                self.open_paths(paths);
            } else if self.right == RightPanel::Queue || ctx.input(|i| i.modifiers.shift) {
                let mut files = Vec::new();
                for p in paths {
                    let pp = std::path::Path::new(&p);
                    if pp.is_dir() {
                        files.extend(media::list_dir(pp, false).1.into_iter().map(|x| x.to_string_lossy().to_string()));
                    } else if media::is_media(&p) {
                        files.push(p);
                    }
                }
                self.enqueue(&files);
            } else {
                self.open_paths(paths);
            }
        }

        // pointer activity
        let pos = ctx.input(|i| i.pointer.latest_pos());
        if pos != self.last_pointer_pos {
            self.last_pointer_pos = pos;
            self.last_pointer_move = Instant::now();
        }

        // hotkeys
        if self.rebinding.is_some() {
            return;
        }
        if ctx.wants_keyboard_input() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            if let egui::Event::Key { key, pressed: true, modifiers, repeat, .. } = ev {
                let combo = Combo::from_event(key, modifiers);
                if let Some(a) = self.bindings.find(combo) {
                    if repeat && matches!(a, Action::PlayPause | Action::Fullscreen | Action::Mute | Action::Bookmark | Action::Pip | Action::HideUi) {
                        continue;
                    }
                    // Esc with modal handled by the modal itself.
                    if a == Action::ExitFullscreen && self.dialog.is_some() {
                        continue;
                    }
                    self.run_action(ctx, a, Some(combo));
                } else if !modifiers.any() {
                    // number keys: jump to 0..90%
                    let n = match key {
                        egui::Key::Num0 => Some(0),
                        egui::Key::Num1 => Some(1),
                        egui::Key::Num2 => Some(2),
                        egui::Key::Num3 => Some(3),
                        egui::Key::Num4 => Some(4),
                        egui::Key::Num5 => Some(5),
                        egui::Key::Num6 => Some(6),
                        egui::Key::Num7 => Some(7),
                        egui::Key::Num8 => Some(8),
                        egui::Key::Num9 => Some(9),
                        _ => None,
                    };
                    if let (Some(n), Some(p)) = (n, &self.player) {
                        if p.st.has_media() {
                            p.seek_percent(n as f64 * 10.0);
                            self.osd_keys(Some(combo), tf!("Sprung zu {}%", n * 10));
                        }
                    }
                }
            }
        }
    }

    pub fn on_exit_save(&mut self) {
        self.update_history_position();
        self.plex.stop_now(self.data.settings.plex_sync_progress);
        self.data.save();
        self.tools.save_cache();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.logos.begin_frame(ctx);
        // Maximized only once the first frame is painted and the window is
        // shown (see `run_desktop`); it already has the work area's size.
        #[cfg(not(target_os = "android"))]
        if crate::MAXIMIZE_AFTER_SHOW.load(std::sync::atomic::Ordering::Relaxed) {
            if ctx.cumulative_pass_nr() >= 1 {
                crate::MAXIMIZE_AFTER_SHOW.store(false, std::sync::atomic::Ordering::Relaxed);
                #[cfg(windows)]
                crate::maximize_with_restore_size(_frame, egui::vec2(1280.0, 760.0), ctx.pixels_per_point());
                // Keeps winit's own state in step.
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            } else {
                ctx.request_repaint();
            }
        }
        self.update_tick(ctx);
        crate::mobile::tick(ctx);
        crate::mobile::simulator_keys(ctx);
        self.frame_start(ctx);
        if crate::mobile::compact(ctx) && crate::mobile::back_pressed(ctx) {
            self.mobile_back(ctx);
        }
        self.handle_input(ctx);
        self.draw(ctx);
        // The render context is made on the first video paint; a local file
        // opened before any video was on screen found none and stayed black.
        if let Some(v) = self.video.as_ref().filter(|v| !v.ready()) {
            v.paint(&ctx.layer_painter(egui::LayerId::background()), egui::Rect::from_min_size(ctx.screen_rect().min, egui::vec2(1.0, 1.0)));
            ctx.request_repaint();
        }
        crate::mobile::paint_simulator_chrome(ctx, self.full_screen, ctx.screen_rect(), &self.t);
    }

    /// Keeps the UI out of status bar, navigation bar and display cutout.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if !crate::mobile::touch() {
            return;
        }
        if let Some(full) = raw.screen_rect {
            self.full_screen = full;
            let [l, t, r, b] = crate::mobile::insets(ctx, full);
            self.safe_insets = [l, t, r, b];
            // The player uses the whole display, notch / punch hole included;
            // it keeps its controls inside the safe area itself.
            if !(self.view == View::Player && self.has_media()) {
                raw.screen_rect = Some(egui::Rect::from_min_max(full.min + egui::vec2(l, t), full.max - egui::vec2(r, b)));
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = self.t.bg;
        [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, 1.0]
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.on_exit_save();
        if let Some(v) = &self.video {
            v.destroy(gl);
        }
    }
}
