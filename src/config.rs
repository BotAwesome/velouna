//! Persistent settings, library data, history, bookmarks and presets.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Language {
    #[default]
    System,
    German,
    English,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ThemeMode {
    Light,
    #[default]
    Dark,
    System,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ResumeMode {
    #[default]
    Ask,
    Always,
    Never,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SubAuto {
    Off,
    Exact,
    #[default]
    Fuzzy,
    All,
}

impl SubAuto {
    pub fn mpv(self) -> &'static str {
        match self {
            SubAuto::Off => "no",
            SubAuto::Exact => "exact",
            SubAuto::Fuzzy => "fuzzy",
            SubAuto::All => "all",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SubAuto::Off => t!("Aus"),
            SubAuto::Exact => t!("Exakt gleicher Dateiname"),
            SubAuto::Fuzzy => t!("Dateiname enthält den Titel"),
            SubAuto::All => t!("Jede Untertiteldatei im Ordner"),
        }
    }
    pub const ALL: [SubAuto; 4] = [SubAuto::Off, SubAuto::Exact, SubAuto::Fuzzy, SubAuto::All];
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WheelAction {
    #[default]
    Volume,
    Seek,
    Nothing,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FullscreenControls {
    #[default]
    Hover,
    Always,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ControlItem {
    Prev,
    Play,
    Next,
    Stop,
    Time,
    Duration,
    Volume,
    Audio,
    Subtitles,
    Adjustments,
    Queue,
    Fullscreen,
    Pip,
    Screenshot,
    Loop,
    Shuffle,
    Speed,
    Bookmark,
    FrameStep,
    Record,
}

impl ControlItem {
    pub const ALL: [ControlItem; 20] = [
        ControlItem::Prev,
        ControlItem::Play,
        ControlItem::Next,
        ControlItem::Stop,
        ControlItem::Time,
        ControlItem::Duration,
        ControlItem::Volume,
        ControlItem::Audio,
        ControlItem::Subtitles,
        ControlItem::Adjustments,
        ControlItem::Queue,
        ControlItem::Fullscreen,
        ControlItem::Pip,
        ControlItem::Screenshot,
        ControlItem::Loop,
        ControlItem::Shuffle,
        ControlItem::Speed,
        ControlItem::Bookmark,
        ControlItem::FrameStep,
        ControlItem::Record,
    ];
    pub fn label(self) -> &'static str {
        match self {
            ControlItem::Prev => t!("Zurück"),
            ControlItem::Play => t!("Wiedergabe"),
            ControlItem::Next => t!("Weiter"),
            ControlItem::Stop => t!("Stopp"),
            ControlItem::Time => t!("Aktuelle Zeit"),
            ControlItem::Duration => t!("Gesamtdauer"),
            ControlItem::Volume => t!("Lautstärke"),
            ControlItem::Audio => t!("Audiospuren"),
            ControlItem::Subtitles => t!("Untertitel"),
            ControlItem::Adjustments => t!("Seitenleiste"),
            ControlItem::Queue => t!("Warteschlange"),
            ControlItem::Fullscreen => t!("Vollbild"),
            ControlItem::Pip => t!("Bild-in-Bild"),
            ControlItem::Screenshot => "Screenshot",
            ControlItem::Loop => t!("Wiederholen"),
            ControlItem::Shuffle => t!("Zufall"),
            ControlItem::Speed => t!("Geschwindigkeit"),
            ControlItem::Bookmark => t!("Lesezeichen"),
            ControlItem::FrameStep => t!("Einzelbild"),
            ControlItem::Record => t!("Aufnahme"),
        }
    }
}

/// Buttons of the phone player's bottom row (below the seek bar).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum TouchItem {
    Lock,
    Repeat,
    Tracks,
    Scale,
    Rotate,
    Speed,
    Shuffle,
    Bookmark,
    SeekBack,
    SeekForward,
    Prev,
    Next,
    Sleep,
    Audio,
    Subtitles,
    Mute,
    Chapters,
    Queue,
    Adjust,
}

/// Bottom-row elements by category; most useful first in each.
pub const TOUCH_CATEGORIES: [(&str, &[TouchItem]); 5] = {
    use TouchItem::*;
    [
        ("Häufig genutzt", &[Lock, Tracks, Scale, Rotate]),
        ("Wiedergabe", &[SeekBack, SeekForward, Next, Prev, Speed, Repeat, Sleep, Shuffle]),
        ("Ton & Untertitel", &[Subtitles, Audio, Mute]),
        ("Navigation", &[Chapters, Queue, Bookmark]),
        ("Werkzeuge", &[Adjust]),
    ]
};

impl TouchItem {
    pub fn label(self) -> &'static str {
        match self {
            TouchItem::Lock => t!("Bildschirm sperren"),
            TouchItem::Repeat => t!("Wiederholen"),
            TouchItem::Tracks => t!("Audio & Untertitel"),
            TouchItem::Scale => t!("Bildgröße"),
            TouchItem::Rotate => t!("Drehen"),
            TouchItem::Speed => t!("Geschwindigkeit"),
            TouchItem::Shuffle => t!("Zufall"),
            TouchItem::Bookmark => t!("Lesezeichen"),
            TouchItem::SeekBack => t!("10 s zurück"),
            TouchItem::SeekForward => t!("10 s vor"),
            TouchItem::Prev => t!("Vorherige"),
            TouchItem::Next => t!("Nächste"),
            TouchItem::Sleep => t!("Sleep-Timer"),
            TouchItem::Audio => t!("Tonspur"),
            TouchItem::Subtitles => t!("Untertitel"),
            TouchItem::Mute => t!("Stumm"),
            TouchItem::Chapters => t!("Kapitel"),
            TouchItem::Queue => t!("Warteschlange"),
            TouchItem::Adjust => t!("Anpassungen"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct TouchControls {
    pub left: Vec<TouchItem>,
    pub right: Vec<TouchItem>,
}

impl Default for TouchControls {
    fn default() -> Self {
        use TouchItem::*;
        Self { left: vec![Lock, Repeat], right: vec![Tracks, Scale] }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ControlLayout {
    pub left: Vec<ControlItem>,
    pub right: Vec<ControlItem>,
}

impl Default for ControlLayout {
    fn default() -> Self {
        use ControlItem::*;
        Self { left: vec![Prev, Play, Next, Time], right: vec![Duration, Volume, Audio, Subtitles, Adjustments, Queue] }
    }
}

impl ControlLayout {
    pub fn minimal() -> Self {
        use ControlItem::*;
        Self { left: vec![Play, Time], right: vec![Duration, Volume, Fullscreen] }
    }
    pub fn power() -> Self {
        use ControlItem::*;
        Self {
            left: vec![Stop, Prev, Play, Next, FrameStep, Time],
            right: vec![Duration, Speed, Loop, Shuffle, Bookmark, Screenshot, Volume, Audio, Subtitles, Adjustments, Queue],
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    // General
    pub theme: ThemeMode,
    pub accent: [u8; 3],
    pub onboarding_done: bool,
    /// Phone player: swipe left half = volume, right half = brightness.
    pub touch_gestures: bool,
    pub gesture_intro_seen: bool,
    /// Phone player: buttons left and right below the seek bar.
    pub touch_controls: TouchControls,
    /// The swipe hint for the Plex libraries drawer was shown.
    pub plex_drawer_hint_seen: bool,
    /// Dragging the seek bar near a chapter start snaps to it.
    pub chapter_snap: bool,
    /// Snap distance in points.
    pub chapter_snap_px: f32,
    /// After this many snaps to the same chapter it is no longer snapped
    /// to (for the current file); 0 = always snap.
    pub chapter_snap_limit: u32,
    /// UI language; System follows the Windows display language.
    pub language: Language,
    pub ui_scale: f32,
    /// Look for a new GitHub release on start (at most once a day).
    pub update_auto: bool,
    /// Unix seconds of the last automatic check.
    pub update_last_check: u64,
    /// A release the user chose to skip (no automatic prompt for it).
    pub update_skip: String,
    /// Version of the last start ("What's new" after an update).
    pub last_run_version: String,
    // Interface
    pub autohide_windowed: bool,
    pub fullscreen_controls: FullscreenControls,
    pub hide_delay_ms: u64,
    pub show_seek_thumbnails: bool,
    pub show_waveform: bool,
    pub show_sub_timeline: bool,
    pub show_remaining_time: bool,
    pub osd_show_keys: bool,
    pub always_on_top: bool,
    pub library_add_folder_to_queue: bool,
    // Privacy
    pub remember_history: bool,
    pub remember_positions: bool,
    pub history_limit: usize,
    // Controls
    pub wheel_action: WheelAction,
    pub click_to_pause: bool,
    pub double_click_fullscreen: bool,
    pub seek_small: f64,
    pub seek_large: f64,
    pub volume_step: f64,
    // Playback
    pub resume: ResumeMode,
    pub autoplay_on_open: bool,
    pub default_speed: f64,
    pub repeat: RepeatMode,
    pub play_folder_on_open: bool,
    pub exact_seek: bool,
    // Video
    pub hwdec: bool,
    pub deinterlace: bool,
    pub keep_aspect: bool,
    // Audio
    pub default_volume: f64,
    pub max_volume: f64,
    pub remember_volume: bool,
    pub last_volume: f64,
    pub normalize: bool,
    pub alang: String,
    pub audio_channels: String,
    // Subtitles
    pub sub_auto: SubAuto,
    pub slang: String,
    pub sub_font: String,
    pub sub_font_size: f64,
    pub sub_color: [u8; 4],
    pub sub_border_color: [u8; 4],
    pub sub_border_size: f64,
    pub sub_back_opacity: f64,
    pub sub_bold: bool,
    pub sub_pos: f64,
    pub sub_size_pixels: bool,
    pub osd_font_size: f64,
    // Library
    pub video_folders: Vec<PathBuf>,
    pub music_folders: Vec<PathBuf>,
    pub scan_on_start: bool,
    pub show_hidden: bool,
    // Plex
    pub plex_sync_progress: bool,
    pub plex_on_home: bool,
    pub plex_local_only: bool,
    /// Featured/detail info in a side column next to the content (default) instead of a banner on top.
    pub plex_side_layout: bool,
    // Network
    pub user_agent: String,
    pub cache_secs: f64,
    pub ytdl: bool,
    pub network_timeout: f64,
    // Performance
    pub demuxer_max_mb: u32,
    pub video_sync_display: bool,
    pub gpu_api: String,
    // Recording
    pub screenshot_dir: PathBuf,
    pub screenshot_format: String,
    pub record_dir: PathBuf,
    // misc
    pub layout: ControlLayout,
    pub sidebar_width: f32,
}

impl Default for Settings {
    fn default() -> Self {
        let videos = dirs::video_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
        let music = dirs::audio_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
        let pictures = dirs::picture_dir().unwrap_or_else(|| videos.clone());
        Self {
            theme: ThemeMode::Dark,
            accent: [0xD8, 0x3F, 0x5C],
            onboarding_done: false,
            touch_gestures: true,
            gesture_intro_seen: false,
            touch_controls: TouchControls::default(),
            plex_drawer_hint_seen: false,
            chapter_snap: true,
            chapter_snap_px: 10.0,
            chapter_snap_limit: 3,
            language: Language::System,
            ui_scale: 1.0,
            update_auto: true,
            update_last_check: 0,
            update_skip: String::new(),
            last_run_version: String::new(),
            autohide_windowed: false,
            fullscreen_controls: FullscreenControls::Hover,
            hide_delay_ms: 1200,
            show_seek_thumbnails: true,
            show_waveform: false,
            show_sub_timeline: false,
            show_remaining_time: false,
            osd_show_keys: true,
            always_on_top: false,
            library_add_folder_to_queue: true,
            remember_history: true,
            remember_positions: true,
            history_limit: 200,
            wheel_action: WheelAction::Volume,
            click_to_pause: true,
            double_click_fullscreen: true,
            seek_small: 5.0,
            seek_large: 60.0,
            volume_step: 5.0,
            resume: ResumeMode::Ask,
            autoplay_on_open: true,
            default_speed: 1.0,
            repeat: RepeatMode::Off,
            play_folder_on_open: true,
            exact_seek: true,
            hwdec: true,
            deinterlace: false,
            keep_aspect: true,
            default_volume: 100.0,
            max_volume: 150.0,
            remember_volume: true,
            last_volume: 100.0,
            normalize: false,
            alang: String::new(),
            audio_channels: "auto-safe".into(),
            sub_auto: SubAuto::Fuzzy,
            slang: String::new(),
            sub_font: "Segoe UI Semibold".into(),
            sub_font_size: 46.0,
            sub_color: [255, 255, 255, 255],
            sub_border_color: [12, 12, 12, 255],
            sub_border_size: 2.5,
            sub_back_opacity: 0.0,
            sub_bold: false,
            sub_pos: 100.0,
            sub_size_pixels: false,
            osd_font_size: 32.0,
            video_folders: vec![videos.clone()],
            music_folders: vec![music],
            scan_on_start: true,
            show_hidden: false,
            plex_sync_progress: true,
            plex_on_home: true,
            plex_local_only: false,
            plex_side_layout: true,
            user_agent: String::new(),
            cache_secs: 60.0,
            ytdl: true,
            network_timeout: 30.0,
            demuxer_max_mb: 150,
            video_sync_display: false,
            gpu_api: "auto".into(),
            screenshot_dir: pictures.join("Velour"),
            screenshot_format: "png".into(),
            record_dir: videos.join("Velour"),
            layout: ControlLayout::default(),
            sidebar_width: 340.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Stream {
    pub name: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Playlist {
    pub name: String,
    pub items: Vec<String>,
    pub created: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PinTarget {
    Folder(PathBuf),
    Playlist(String),
    Stream(String),
    File(String),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Pin {
    pub name: String,
    pub target: PinTarget,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HistoryEntry {
    pub path: String,
    pub title: String,
    pub position: f64,
    pub duration: f64,
    pub last_played: u64,
    pub finished: bool,
    pub audio_only: bool,
}

impl HistoryEntry {
    pub fn progress(&self) -> f32 {
        if self.finished {
            return 1.0;
        }
        if self.duration > 0.0 { (self.position / self.duration).clamp(0.0, 1.0) as f32 } else { 0.0 }
    }
    pub fn resumable(&self) -> bool {
        !self.finished && self.position > 10.0 && (self.duration <= 0.0 || self.position < self.duration - 15.0)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Bookmark {
    pub time: f64,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Preset {
    pub name: String,
    pub sub_delay: f64,
    pub audio_delay: f64,
    pub sub_font: String,
    pub sub_font_size: f64,
    pub sub_scale: f64,
    pub sub_size_pixels: bool,
    pub sub_color: [u8; 4],
    pub sub_border_color: [u8; 4],
    pub sub_border_size: f64,
    pub sub_back_opacity: f64,
    pub sub_pos: f64,
    pub sub_bold: bool,
    pub sub_track_title: Option<String>,
    pub sub_lang: Option<String>,
    pub sub_off: bool,
    pub audio_track_title: Option<String>,
    pub audio_lang: Option<String>,
    pub speed: f64,
    pub eq_preset: String,
    pub video_preset: String,
}

impl Default for Preset {
    fn default() -> Self {
        let s = Settings::default();
        Self {
            name: "Neues Preset".into(),
            sub_delay: 0.0,
            audio_delay: 0.0,
            sub_font: s.sub_font,
            sub_font_size: s.sub_font_size,
            sub_scale: 1.0,
            sub_size_pixels: false,
            sub_color: s.sub_color,
            sub_border_color: s.sub_border_color,
            sub_border_size: s.sub_border_size,
            sub_back_opacity: 0.0,
            sub_pos: 100.0,
            sub_bold: false,
            sub_track_title: None,
            sub_lang: None,
            sub_off: false,
            audio_track_title: None,
            audio_lang: None,
            speed: 1.0,
            eq_preset: "Flat".into(),
            video_preset: "Keiner".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Data {
    pub settings: Settings,
    pub streams: Vec<Stream>,
    pub playlists: Vec<Playlist>,
    pub pins: Vec<Pin>,
    pub history: Vec<HistoryEntry>,
    pub bookmarks: HashMap<String, Vec<Bookmark>>,
    /// Named presets.
    pub presets: Vec<Preset>,
    /// Folder (show) -> preset name, auto-applied for every file in it.
    pub folder_presets: HashMap<String, String>,
    /// Video path -> subtitle path, from the "add subtitles to folder" dialog.
    pub sub_map: HashMap<String, String>,
    /// Action name -> key combo string.
    pub hotkeys: HashMap<String, String>,
    pub eq_custom: Vec<f64>,
    pub stream_defaults_added: bool,
    pub plex: PlexConfig,
}

/// Persistent Plex identity. The token itself lives in the Windows credential store.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PlexConfig {
    pub client_id: String,
    pub server_id: Option<String>,
    pub server_name: String,
    pub last_uri: Option<String>,
    pub user_name: String,
}

/// Folder name the app used while it was still called "VLB".
const LEGACY_DIR: &str = "VLB";

/// `<base>/Velour`, moving a pre-rename `<base>/VLB` over on first use.
fn app_dir(base: Option<PathBuf>) -> PathBuf {
    let base = base.unwrap_or_else(std::env::temp_dir);
    let d = base.join("Velour");
    if !d.exists() {
        let old = base.join(LEGACY_DIR);
        if old.is_dir() {
            let _ = std::fs::rename(&old, &d);
        }
    }
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn data_dir() -> PathBuf {
    crate::platform::data_dir_override().unwrap_or_else(|| app_dir(dirs::config_dir()))
}

pub fn cache_dir() -> PathBuf {
    crate::platform::cache_dir_override().unwrap_or_else(|| app_dir(dirs::cache_dir()))
}

impl Data {
    pub fn load() -> Data {
        let p = data_dir().join("config.json");
        let mut d: Data = std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if !d.stream_defaults_added {
            d.streams.push(Stream {
                name: "Öffentliche TV-Sender (iptv-org)".into(),
                url: "https://iptv-org.github.io/iptv/index.m3u".into(),
            });
            d.streams.push(Stream {
                name: "SomaFM Groove Salad (Radio)".into(),
                url: "https://ice1.somafm.com/groovesalad-128-mp3".into(),
            });
            d.stream_defaults_added = true;
        }
        // Default folders from before the rename still carry the working title.
        let s = &mut d.settings;
        if s.screenshot_dir.file_name().is_some_and(|n| n == LEGACY_DIR) {
            s.screenshot_dir = s.screenshot_dir.with_file_name("Velour");
        }
        // The old default accent was VLC-like orange; Velour's is velvet red.
        if s.accent == [0xF0, 0x86, 0x00] {
            s.accent = [0xD8, 0x3F, 0x5C];
        }
        if s.record_dir.file_name().is_some_and(|n| n == "VLB Aufnahmen") {
            s.record_dir = s.record_dir.with_file_name("Velour");
        }
        if d.eq_custom.len() != 10 {
            d.eq_custom = vec![0.0; 10];
        }
        d
    }

    pub fn save(&self) {
        let p = data_dir().join("config.json");
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }

    pub fn history_for(&self, path: &str) -> Option<&HistoryEntry> {
        self.history.iter().find(|h| h.path == path)
    }

    pub fn preset(&self, name: &str) -> Option<&Preset> {
        self.presets.iter().find(|p| p.name == name)
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
