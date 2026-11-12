//! Player state mirrored from mpv property observations plus convenience commands.

use crate::mpv::{Event, Mpv, Value};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct Track {
    pub id: i64,
    pub kind: String,
    pub title: Option<String>,
    pub lang: Option<String>,
    pub codec: String,
    pub selected: bool,
    pub external: bool,
    pub external_filename: Option<String>,
    pub albumart: bool,
    pub image: bool,
    pub ff_index: Option<i64>,
    pub channels: Option<i64>,
    pub w: Option<i64>,
    pub h: Option<i64>,
    pub default: bool,
    pub forced: bool,
}

impl Track {
    pub fn label(&self) -> String {
        let mut s = tf!("Spur {}", self.id);
        if let Some(t) = &self.title {
            if !t.is_empty() {
                s = format!("{s} – {t}");
            }
        }
        if let Some(l) = &self.lang {
            if !l.is_empty() {
                s = format!("{s} [{}]", crate::media::language_name(l));
            }
        }
        if self.external {
            s.push_str(t!("  · extern"));
        }
        if self.forced {
            s.push_str("  · forced");
        }
        s
    }
}

#[derive(Clone, Debug, Default)]
pub struct PlaylistEntry {
    pub filename: String,
    pub title: Option<String>,
    pub current: bool,
    pub id: i64,
}

impl PlaylistEntry {
    pub fn display(&self) -> String {
        if let Some(t) = crate::plex::remote_title(&self.filename) {
            return t;
        }
        if let Some(t) = &self.title {
            let t = clean_title(t);
            if !t.is_empty() {
                return t.to_string();
            }
        }
        crate::media::display_name(&self.filename)
    }
}

/// mpv takes an `#EXTINF` title from the first comma on, so a quoted
/// attribute that contains commas (IPTV lists: `http-user-agent="… (KHTML,
/// like Gecko) …",Name`) leaks into it. The real name follows the last `",`.
fn clean_title(t: &str) -> &str {
    match t.rfind("\",") {
        Some(i) => t[i + 2..].trim(),
        None => t.trim(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn extinf_titles() {
        let t = " like Gecko) Chrome/149.0.0.0 Safari/537.36\",1+1 International (720p)";
        assert_eq!(super::clean_title(t), "1+1 International (720p)");
        assert_eq!(super::clean_title("Plain, with comma"), "Plain, with comma");
    }
}

#[derive(Clone, Debug, Default)]
pub struct Chapter {
    pub title: String,
    pub time: f64,
}

#[derive(Clone, Debug, Default)]
pub struct AudioDevice {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct PlayerState {
    pub time: f64,
    pub duration: f64,
    pub paused: bool,
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub media_title: String,
    pub path: String,
    pub tracks: Vec<Track>,
    pub playlist: Arc<Vec<PlaylistEntry>>,
    pub playlist_pos: i64,
    pub chapters: Vec<Chapter>,
    pub chapter: i64,
    pub sub_delay: f64,
    pub audio_delay: f64,
    pub audio_devices: Vec<AudioDevice>,
    pub audio_device: String,
    pub idle: bool,
    pub eof: bool,
    pub cache_ranges: Vec<(f64, f64)>,
    pub buffering: bool,
    pub video_w: i64,
    pub video_h: i64,
    pub metadata: Vec<(String, String)>,
    pub sub_scale: f64,
    pub sub_pos: f64,
    pub sub_font_size: f64,
    pub sub_border_size: f64,
    pub sub_visible: bool,
    pub sub_speed: f64,
    pub rotate: i64,
    pub zoom: f64,
    pub brightness: f64,
    pub contrast: f64,
    pub saturation: f64,
    pub gamma: f64,
    pub hue: f64,
    pub loop_file: bool,
    pub loop_playlist: bool,
    pub ab_a: Option<f64>,
    pub ab_b: Option<f64>,
    pub hwdec: String,
    pub fps: f64,
    pub video_codec: String,
    pub audio_codec: String,
    pub file_format: String,
    pub file_size: i64,
    pub direction_backward: bool,
    pub aspect: String,
    pub recording: String,
    pub seekable: bool,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            time: 0.0,
            duration: 0.0,
            paused: false,
            volume: 100.0,
            muted: false,
            speed: 1.0,
            media_title: String::new(),
            path: String::new(),
            tracks: vec![],
            playlist: Arc::new(vec![]),
            playlist_pos: -1,
            chapters: vec![],
            chapter: -1,
            sub_delay: 0.0,
            audio_delay: 0.0,
            audio_devices: vec![],
            audio_device: String::new(),
            idle: true,
            eof: false,
            cache_ranges: vec![],
            buffering: false,
            video_w: 0,
            video_h: 0,
            metadata: vec![],
            sub_scale: 1.0,
            sub_pos: 100.0,
            sub_font_size: 55.0,
            sub_border_size: 3.0,
            sub_visible: true,
            sub_speed: 1.0,
            rotate: 0,
            zoom: 0.0,
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            gamma: 0.0,
            hue: 0.0,
            loop_file: false,
            loop_playlist: false,
            ab_a: None,
            ab_b: None,
            hwdec: String::new(),
            fps: 0.0,
            video_codec: String::new(),
            audio_codec: String::new(),
            file_format: String::new(),
            file_size: 0,
            direction_backward: false,
            aspect: "no".into(),
            recording: String::new(),
            seekable: true,
        }
    }
}

pub const OBSERVED: &[&str] = &[
    "time-pos",
    "duration",
    "pause",
    "volume",
    "mute",
    "speed",
    "media-title",
    "path",
    "track-list",
    "playlist",
    "playlist-pos",
    "chapter-list",
    "chapter",
    "sub-delay",
    "audio-delay",
    "audio-device-list",
    "audio-device",
    "idle-active",
    "eof-reached",
    "demuxer-cache-state",
    "paused-for-cache",
    "video-params",
    "metadata",
    "sub-scale",
    "sub-pos",
    "sub-font-size",
    "sub-border-size",
    "sub-visibility",
    "sub-speed",
    "video-rotate",
    "video-zoom",
    "brightness",
    "contrast",
    "saturation",
    "gamma",
    "hue",
    "loop-file",
    "loop-playlist",
    "ab-loop-a",
    "ab-loop-b",
    "hwdec-current",
    "container-fps",
    "video-codec",
    "audio-codec-name",
    "file-format",
    "file-size",
    "play-direction",
    "video-aspect-override",
    "stream-record",
    "seekable",
];

fn opt_time(v: &Value) -> Option<f64> {
    match v {
        Value::Str(s) if s == "no" => None,
        other => other.as_f64(),
    }
}

impl PlayerState {
    pub fn apply(&mut self, name: &str, v: &Value) {
        let f = |d: f64| v.as_f64().unwrap_or(d);
        match name {
            "time-pos" => self.time = f(0.0),
            "duration" => self.duration = f(0.0),
            "pause" => self.paused = v.as_bool().unwrap_or(false),
            "volume" => self.volume = f(100.0),
            "mute" => self.muted = v.as_bool().unwrap_or(false),
            "speed" => self.speed = f(1.0),
            "media-title" => self.media_title = v.as_str().unwrap_or("").to_string(),
            "path" => self.path = v.as_str().unwrap_or("").to_string(),
            "track-list" => {
                self.tracks = v
                    .as_array()
                    .iter()
                    .map(|t| Track {
                        id: t.i64_of("id").unwrap_or(0),
                        kind: t.str_of("type").unwrap_or_default(),
                        title: t.str_of("title"),
                        lang: t.str_of("lang"),
                        codec: t.str_of("codec").unwrap_or_default(),
                        selected: t.bool_of("selected"),
                        external: t.bool_of("external"),
                        external_filename: t.str_of("external-filename"),
                        albumart: t.bool_of("albumart"),
                        image: t.bool_of("image"),
                        ff_index: t.i64_of("ff-index"),
                        channels: t.i64_of("demux-channel-count"),
                        w: t.i64_of("demux-w"),
                        h: t.i64_of("demux-h"),
                        default: t.bool_of("default"),
                        forced: t.bool_of("forced"),
                    })
                    .collect();
            }
            "playlist" => {
                self.playlist = Arc::new(
                    v.as_array()
                        .iter()
                        .map(|e| PlaylistEntry {
                            filename: e.str_of("filename").unwrap_or_default(),
                            title: e.str_of("title"),
                            current: e.bool_of("current"),
                            id: e.i64_of("id").unwrap_or(0),
                        })
                        .collect(),
                );
            }
            "playlist-pos" => self.playlist_pos = v.as_i64().unwrap_or(-1),
            "chapter-list" => {
                self.chapters = v
                    .as_array()
                    .iter()
                    .map(|c| Chapter {
                        title: c.str_of("title").unwrap_or_default(),
                        time: c.f64_of("time").unwrap_or(0.0),
                    })
                    .collect();
            }
            "chapter" => self.chapter = v.as_i64().unwrap_or(-1),
            "sub-delay" => self.sub_delay = f(0.0),
            "audio-delay" => self.audio_delay = f(0.0),
            "audio-device-list" => {
                self.audio_devices = v
                    .as_array()
                    .iter()
                    .map(|d| AudioDevice {
                        name: d.str_of("name").unwrap_or_default(),
                        description: d.str_of("description").unwrap_or_default(),
                    })
                    .collect();
            }
            "audio-device" => self.audio_device = v.as_str().unwrap_or("auto").to_string(),
            "idle-active" => self.idle = v.as_bool().unwrap_or(false),
            "eof-reached" => self.eof = v.as_bool().unwrap_or(false),
            "demuxer-cache-state" => {
                self.cache_ranges = v
                    .get("seekable-ranges")
                    .map(|r| {
                        r.as_array()
                            .iter()
                            .filter_map(|x| Some((x.f64_of("start")?, x.f64_of("end")?)))
                            .collect()
                    })
                    .unwrap_or_default();
            }
            "paused-for-cache" => self.buffering = v.as_bool().unwrap_or(false),
            "video-params" => {
                self.video_w = v.i64_of("dw").or(v.i64_of("w")).unwrap_or(0);
                self.video_h = v.i64_of("dh").or(v.i64_of("h")).unwrap_or(0);
            }
            "metadata" => {
                self.metadata = match v {
                    Value::Map(m) => m
                        .iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                        .collect(),
                    _ => vec![],
                };
            }
            "sub-scale" => self.sub_scale = f(1.0),
            "sub-pos" => self.sub_pos = f(100.0),
            "sub-font-size" => self.sub_font_size = f(55.0),
            "sub-border-size" => self.sub_border_size = f(3.0),
            "sub-visibility" => self.sub_visible = v.as_bool().unwrap_or(true),
            "sub-speed" => self.sub_speed = f(1.0),
            "video-rotate" => self.rotate = v.as_i64().unwrap_or(0),
            "video-zoom" => self.zoom = f(0.0),
            "brightness" => self.brightness = f(0.0),
            "contrast" => self.contrast = f(0.0),
            "saturation" => self.saturation = f(0.0),
            "gamma" => self.gamma = f(0.0),
            "hue" => self.hue = f(0.0),
            "loop-file" => self.loop_file = !matches!(v, Value::Str(s) if s == "no") && v.as_bool() != Some(false),
            "loop-playlist" => {
                self.loop_playlist = !matches!(v, Value::Str(s) if s == "no") && v.as_bool() != Some(false)
            }
            "ab-loop-a" => self.ab_a = opt_time(v),
            "ab-loop-b" => self.ab_b = opt_time(v),
            "hwdec-current" => self.hwdec = v.as_str().unwrap_or("").to_string(),
            "container-fps" => self.fps = f(0.0),
            "video-codec" => self.video_codec = v.as_str().unwrap_or("").to_string(),
            "audio-codec-name" => self.audio_codec = v.as_str().unwrap_or("").to_string(),
            "file-format" => self.file_format = v.as_str().unwrap_or("").to_string(),
            "file-size" => self.file_size = v.as_i64().unwrap_or(0),
            "play-direction" => self.direction_backward = v.as_str() == Some("backward"),
            "video-aspect-override" => {
                self.aspect = match v {
                    Value::Str(s) => s.clone(),
                    Value::Double(d) if *d <= 0.0 => "no".into(),
                    Value::Double(d) => format!("{d:.3}"),
                    _ => "no".into(),
                }
            }
            "stream-record" => self.recording = v.as_str().unwrap_or("").to_string(),
            "seekable" => self.seekable = v.as_bool().unwrap_or(true),
            _ => {}
        }
    }

    pub fn has_media(&self) -> bool {
        !self.idle && !self.path.is_empty()
    }

    pub fn tracks_of<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Track> + 'a {
        self.tracks.iter().filter(move |t| t.kind == kind)
    }

    pub fn selected(&self, kind: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.kind == kind && t.selected)
    }

    /// True when the current file has a real (non cover-art) video stream.
    pub fn has_video(&self) -> bool {
        self.tracks.iter().any(|t| t.kind == "video" && !t.albumart && !t.image)
    }

    pub fn is_audio_only(&self) -> bool {
        self.has_media() && !self.has_video()
    }

    pub fn has_cover(&self) -> bool {
        self.tracks.iter().any(|t| t.kind == "video" && t.selected && (t.albumart || t.image))
    }

    pub fn meta(&self, key: &str) -> Option<&str> {
        self.metadata
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
            .filter(|s| !s.is_empty())
    }

    pub fn current_chapter_title(&self) -> Option<&str> {
        if self.chapter >= 0 {
            self.chapters.get(self.chapter as usize).map(|c| c.title.as_str())
        } else {
            None
        }
    }
}

pub enum LoadMode {
    Replace,
    Append,
    AppendPlay,
}

/// Cheap, cloneable command handle.
#[derive(Clone)]
pub struct Ctl {
    pub mpv: Arc<Mpv>,
}

pub struct Player {
    pub ctl: Ctl,
    pub mpv: Arc<Mpv>,
    pub st: PlayerState,
}

impl std::ops::Deref for Player {
    type Target = Ctl;
    fn deref(&self) -> &Ctl {
        &self.ctl
    }
}

impl Player {
    pub fn new(mpv: Arc<Mpv>) -> Self {
        for (i, p) in OBSERVED.iter().enumerate() {
            mpv.observe(i as u64 + 1, p);
        }
        Self { ctl: Ctl { mpv: mpv.clone() }, mpv, st: PlayerState::default() }
    }

    pub fn poll(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Some(e) = self.mpv.poll_event() {
            if let Event::PropertyChange { name, value } = &e {
                self.st.apply(name, value);
            }
            out.push(e);
            if out.len() > 2000 {
                break;
            }
        }
        out
    }

    pub fn prev(&self) {
        self.ctl.prev_with(self.st.time, self.st.playlist_pos);
    }
}

impl Ctl {
    pub fn load(&self, path: &str, mode: LoadMode, start: Option<f64>) {
        let flag = match mode {
            LoadMode::Replace => "replace",
            LoadMode::Append => "append",
            LoadMode::AppendPlay => "append-play",
        };
        match start {
            Some(t) if t > 0.0 => {
                let opt = format!("start={t:.3}");
                let _ = self.mpv.command(&["loadfile", path, flag, "-1", &opt]);
            }
            _ => {
                let _ = self.mpv.command(&["loadfile", path, flag]);
            }
        }
    }
    pub fn toggle_pause(&self) {
        let _ = self.mpv.command(&["cycle", "pause"]);
    }
    pub fn set_pause(&self, p: bool) {
        let _ = self.mpv.set_flag("pause", p);
    }
    pub fn seek_rel(&self, secs: f64) {
        let _ = self.mpv.command(&["seek", &format!("{secs}"), "relative"]);
    }
    pub fn seek_abs(&self, t: f64, exact: bool) {
        let mode = if exact { "absolute+exact" } else { "absolute+keyframes" };
        let _ = self.mpv.command(&["seek", &format!("{:.3}", t.max(0.0)), mode]);
    }
    pub fn seek_percent(&self, pct: f64) {
        let _ = self.mpv.command(&["seek", &format!("{pct}"), "absolute-percent"]);
    }
    pub fn set_volume(&self, v: f64) {
        let _ = self.mpv.set_f64("volume", v);
    }
    pub fn set_mute(&self, m: bool) {
        let _ = self.mpv.set_flag("mute", m);
    }
    pub fn next(&self) {
        let _ = self.mpv.command(&["playlist-next", "weak"]);
    }
    pub fn prev_with(&self, time: f64, pos: i64) {
        if time > 5.0 || pos <= 0 {
            self.seek_abs(0.0, true);
        } else {
            let _ = self.mpv.command(&["playlist-prev", "weak"]);
        }
    }
    pub fn stop(&self) {
        let _ = self.mpv.command(&["stop"]);
    }
    pub fn set_speed(&self, s: f64) {
        let _ = self.mpv.set_f64("speed", s.clamp(0.1, 8.0));
    }
    pub fn set_track(&self, kind: &str, id: Option<i64>) {
        let prop = match kind {
            "video" => "vid",
            "audio" => "aid",
            _ => "sid",
        };
        match id {
            Some(id) => {
                let _ = self.mpv.set_i64(prop, id);
            }
            None => {
                let _ = self.mpv.set_str(prop, "no");
            }
        }
    }
    pub fn cycle(&self, prop: &str) {
        let _ = self.mpv.command(&["cycle", prop]);
    }
    pub fn sub_add(&self, path: &str, select: bool) {
        let _ = self.mpv.command(&["sub-add", path, if select { "select" } else { "auto" }]);
    }
    pub fn playlist_play(&self, idx: usize) {
        let _ = self.mpv.command(&["playlist-play-index", &idx.to_string()]);
    }
    pub fn playlist_remove(&self, idx: usize) {
        let _ = self.mpv.command(&["playlist-remove", &idx.to_string()]);
    }
    pub fn playlist_move(&self, from: usize, to: usize) {
        let _ = self.mpv.command(&["playlist-move", &from.to_string(), &to.to_string()]);
    }
    pub fn playlist_clear(&self) {
        let _ = self.mpv.command(&["playlist-clear"]);
    }
    pub fn playlist_shuffle(&self) {
        let _ = self.mpv.command(&["playlist-shuffle"]);
    }
    pub fn screenshot(&self) {
        let _ = self.mpv.command(&["screenshot", "subtitles"]);
    }
    pub fn frame_step(&self) {
        let _ = self.mpv.command(&["frame-step"]);
    }
    pub fn frame_back_step(&self) {
        let _ = self.mpv.command(&["frame-back-step"]);
    }
    pub fn set(&self, prop: &str, v: f64) {
        let _ = self.mpv.set_f64(prop, v);
    }
    pub fn set_str(&self, prop: &str, v: &str) {
        let _ = self.mpv.set_str(prop, v);
    }
    pub fn set_flag(&self, prop: &str, v: bool) {
        let _ = self.mpv.set_flag(prop, v);
    }
}
