//! Keyboard shortcuts: actions, default bindings, parsing and matching.

use eframe::egui::{Key, Modifiers};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Action {
    PlayPause,
    Stop,
    Next,
    Prev,
    SeekFwd,
    SeekBack,
    SeekFwdLarge,
    SeekBackLarge,
    FrameStep,
    FrameBack,
    VolUp,
    VolDown,
    Mute,
    Fullscreen,
    ExitFullscreen,
    Pip,
    HideUi,
    ShowTime,
    SpeedUp,
    SpeedDown,
    SpeedReset,
    SubDelayMinus,
    SubDelayPlus,
    AudioDelayMinus,
    AudioDelayPlus,
    CycleSub,
    CycleAudio,
    ToggleSubs,
    Bookmark,
    Screenshot,
    LoopAB,
    LoopFile,
    OpenFile,
    OpenFolder,
    OpenNetwork,
    OpenClipboard,
    Library,
    Queue,
    Sidebar,
    Settings,
    Hotkeys,
    Quit,
}

pub struct ActionInfo {
    pub action: Action,
    pub id: &'static str,
    pub name: &'static str,
    pub group: &'static str,
    pub default: &'static str,
}

macro_rules! a {
    ($a:ident, $id:literal, $name:literal, $group:literal, $def:literal) => {
        ActionInfo { action: Action::$a, id: $id, name: $name, group: $group, default: $def }
    };
}

pub const ACTIONS: &[ActionInfo] = &[
    a!(PlayPause, "play_pause", "Wiedergabe / Pause", "Wiedergabe", "Space"),
    a!(Stop, "stop", "Stopp", "Wiedergabe", "S"),
    a!(Next, "next", "Nächster Titel", "Wiedergabe", "N"),
    a!(Prev, "prev", "Vorheriger Titel", "Wiedergabe", "P"),
    a!(SeekFwd, "seek_fwd", "Kurz vorspulen", "Wiedergabe", "ArrowRight"),
    a!(SeekBack, "seek_back", "Kurz zurückspulen", "Wiedergabe", "ArrowLeft"),
    a!(SeekFwdLarge, "seek_fwd_large", "Weit vorspulen", "Wiedergabe", "Ctrl+ArrowRight"),
    a!(SeekBackLarge, "seek_back_large", "Weit zurückspulen", "Wiedergabe", "Ctrl+ArrowLeft"),
    a!(FrameStep, "frame_step", "Nächstes Einzelbild", "Wiedergabe", "E"),
    a!(FrameBack, "frame_back", "Vorheriges Einzelbild", "Wiedergabe", "Shift+E"),
    a!(SpeedUp, "speed_up", "Schneller", "Wiedergabe", "CloseBracket"),
    a!(SpeedDown, "speed_down", "Langsamer", "Wiedergabe", "OpenBracket"),
    a!(SpeedReset, "speed_reset", "Normale Geschwindigkeit", "Wiedergabe", "Equals"),
    a!(LoopAB, "loop_ab", "A-B-Schleife setzen", "Wiedergabe", "L"),
    a!(LoopFile, "loop_file", "Datei wiederholen", "Wiedergabe", "Shift+L"),
    a!(Bookmark, "bookmark", "Lesezeichen hinzufügen", "Wiedergabe", "Ctrl+B"),
    a!(VolUp, "vol_up", "Lauter", "Audio", "ArrowUp"),
    a!(VolDown, "vol_down", "Leiser", "Audio", "ArrowDown"),
    a!(Mute, "mute", "Stumm schalten", "Audio", "M"),
    a!(CycleAudio, "cycle_audio", "Audiospur wechseln", "Audio", "B"),
    a!(AudioDelayMinus, "audio_delay_minus", "Audioverzögerung −50 ms", "Audio", "J"),
    a!(AudioDelayPlus, "audio_delay_plus", "Audioverzögerung +50 ms", "Audio", "K"),
    a!(CycleSub, "cycle_sub", "Untertitelspur wechseln", "Untertitel", "V"),
    a!(ToggleSubs, "toggle_subs", "Untertitel ein/aus", "Untertitel", "Shift+V"),
    a!(SubDelayMinus, "sub_delay_minus", "Untertitel −100 ms", "Untertitel", "G"),
    a!(SubDelayPlus, "sub_delay_plus", "Untertitel +100 ms", "Untertitel", "H"),
    a!(Fullscreen, "fullscreen", "Vollbild umschalten", "Ansicht", "F"),
    a!(ExitFullscreen, "exit_fullscreen", "Vollbild verlassen / Dialog schließen", "Ansicht", "Escape"),
    a!(Pip, "pip", "Bild-in-Bild", "Ansicht", "Ctrl+P"),
    a!(HideUi, "hide_ui", "Oberfläche ausblenden", "Ansicht", "Ctrl+H"),
    a!(ShowTime, "show_time", "Position und Zeit anzeigen", "Ansicht", "T"),
    a!(Screenshot, "screenshot", "Screenshot", "Ansicht", "Shift+S"),
    a!(Library, "library", "Bibliothek", "Navigation", "Ctrl+L"),
    a!(Queue, "queue", "Warteschlange", "Navigation", "Q"),
    a!(Sidebar, "sidebar", "Seitenleiste (Anpassungen)", "Navigation", "Ctrl+E"),
    a!(OpenFile, "open_file", "Datei öffnen", "Navigation", "Ctrl+O"),
    a!(OpenFolder, "open_folder", "Ordner öffnen", "Navigation", "Ctrl+F"),
    a!(OpenNetwork, "open_network", "Netzwerkstream öffnen", "Navigation", "Ctrl+N"),
    a!(OpenClipboard, "open_clipboard", "Aus Zwischenablage öffnen", "Navigation", "Ctrl+V"),
    a!(Settings, "settings", "Einstellungen", "Navigation", "Ctrl+Comma"),
    a!(Hotkeys, "hotkeys", "Tastenkürzel", "Navigation", "Ctrl+K"),
    a!(Quit, "quit", "Beenden", "Navigation", "Ctrl+Q"),
];

pub fn info(a: Action) -> &'static ActionInfo {
    ACTIONS.iter().find(|i| i.action == a).expect("action info")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Combo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Combo {
    pub fn parse(s: &str) -> Option<Combo> {
        let mut c = Combo { ctrl: false, shift: false, alt: false, key: Key::Space };
        let mut key = None;
        for part in s.split('+') {
            match part.trim() {
                "Ctrl" => c.ctrl = true,
                "Shift" => c.shift = true,
                "Alt" => c.alt = true,
                k => key = Key::from_name(k),
            }
        }
        c.key = key?;
        Some(c)
    }

    pub fn from_event(key: Key, m: Modifiers) -> Combo {
        Combo { ctrl: m.ctrl || m.command, shift: m.shift, alt: m.alt, key }
    }

    pub fn to_string(self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        s.push_str(self.key.name());
        s
    }

    /// Human readable key chips, e.g. ["Strg", "B"].
    pub fn chips(self) -> Vec<String> {
        let mut v = Vec::new();
        if self.ctrl {
            v.push(t!("Strg").to_string());
        }
        if self.shift {
            v.push("Shift".to_string());
        }
        if self.alt {
            v.push("Alt".to_string());
        }
        v.push(key_label(self.key));
        v
    }
}

pub fn key_label(k: Key) -> String {
    match k {
        Key::Space => t!("Leertaste").into(),
        Key::ArrowLeft => "←".into(),
        Key::ArrowRight => "→".into(),
        Key::ArrowUp => "↑".into(),
        Key::ArrowDown => "↓".into(),
        Key::Escape => "Esc".into(),
        Key::OpenBracket => "[".into(),
        Key::CloseBracket => "]".into(),
        Key::Equals => "=".into(),
        Key::Comma => ",".into(),
        Key::Period => ".".into(),
        Key::Minus => "-".into(),
        Key::Plus => "+".into(),
        Key::Enter => "Enter".into(),
        Key::Delete => t!("Entf").into(),
        Key::Backspace => "⌫".into(),
        other => other.name().to_string(),
    }
}

pub struct Bindings {
    pub map: Vec<(Action, Combo)>,
}

impl Bindings {
    pub fn from_overrides(over: &HashMap<String, String>) -> Bindings {
        let mut map = Vec::new();
        for i in ACTIONS {
            let s = over.get(i.id).map(|s| s.as_str()).unwrap_or(i.default);
            if s.is_empty() {
                continue;
            }
            if let Some(c) = Combo::parse(s) {
                map.push((i.action, c));
            }
        }
        Bindings { map }
    }

    pub fn find(&self, c: Combo) -> Option<Action> {
        self.map.iter().find(|(_, b)| *b == c).map(|(a, _)| *a)
    }

    pub fn combo(&self, a: Action) -> Option<Combo> {
        self.map.iter().find(|(x, _)| *x == a).map(|(_, c)| *c)
    }
}
