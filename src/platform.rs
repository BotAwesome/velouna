//! The few things that differ between Windows and Android, behind one
//! interface: opening URLs, system fonts, data folders.

#[cfg(target_os = "android")]
pub mod android;
#[cfg(windows)]
pub mod win;
#[cfg(windows)]
pub mod hdr;

/// The window is minimized: mpv's wakeups repaint only now and then.
pub static BACKGROUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True on phones/tablets (touch-first UI, no window chrome).
#[allow(dead_code)]
pub const MOBILE: bool = cfg!(target_os = "android");

/// mpv `hwdec` when hardware decoding is on. Android: MediaCodec, copied
/// back so it works with the OpenGL render API. Desktop: `auto-safe` tries
/// the zero-copy decoders first and falls back to their copy-back variants
/// (on Linux under Wayland: vaapi-copy / nvdec-copy).
pub const HWDEC: &str = if MOBILE { "mediacodec-copy" } else { "auto-safe" };

/// Name of the system for Plex's device list (X-Plex-Platform).
pub const PLEX_PLATFORM: &str = if cfg!(target_os = "android") {
    "Android"
} else if cfg!(target_os = "linux") {
    "Linux"
} else {
    "Windows"
};

/// "Show in Explorer" / "Show in file manager", as a menu entry.
pub fn show_in_folder_label() -> &'static str {
    if cfg!(windows) { t!("Im Explorer anzeigen") } else { t!("Im Dateimanager anzeigen") }
}

/// Opens a web page in the system browser.
pub fn open_url(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    }
    #[cfg(target_os = "android")]
    {
        android::open_url(url);
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

/// What playback needs to stay awake: nothing, the system (music: no
/// suspend) or also the display (video: no screen saver, no blanking).
#[cfg(not(target_os = "android"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Awake {
    No = 0,
    System = 1,
    Display = 2,
}

/// Keeps the computer awake while something plays. Called every frame; acts
/// only on changes. libmpv's render API leaves this to the app (mpv's own
/// `stop-screensaver` lives in its window code, which Velouna does not use).
/// Windows: SetThreadExecutionState on the UI thread. Linux: the desktop
/// portal's Inhibit and org.freedesktop.ScreenSaver, from a thread that
/// keeps the D-Bus connection (and so the inhibition) open.
#[cfg(not(target_os = "android"))]
pub fn keep_awake(want: Awake) {
    use std::sync::atomic::{AtomicU8, Ordering};
    static CUR: AtomicU8 = AtomicU8::new(Awake::No as u8);
    if CUR.swap(want as u8, Ordering::Relaxed) == want as u8 {
        return;
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetThreadExecutionState(flags: u32) -> u32;
        }
        const CONTINUOUS: u32 = 0x8000_0000;
        const SYSTEM: u32 = 0x1;
        const DISPLAY: u32 = 0x2;
        let flags = match want {
            Awake::No => CONTINUOUS,
            Awake::System => CONTINUOUS | SYSTEM,
            Awake::Display => CONTINUOUS | SYSTEM | DISPLAY,
        };
        unsafe { SetThreadExecutionState(flags) };
    }
    #[cfg(target_os = "linux")]
    linux_awake::set(want);
}

#[cfg(target_os = "linux")]
mod linux_awake {
    use super::Awake;
    use std::collections::HashMap;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Mutex, OnceLock};
    use zbus::zvariant::{OwnedObjectPath, Value};

    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const SAVER: &str = "org.freedesktop.ScreenSaver";

    pub fn set(want: Awake) {
        static TX: OnceLock<Mutex<Sender<Awake>>> = OnceLock::new();
        let tx = TX.get_or_init(|| {
            let (tx, rx) = channel();
            let _ = std::thread::Builder::new().name("velouna-inhibit".into()).spawn(move || run(rx));
            Mutex::new(tx)
        });
        if let Ok(tx) = tx.lock() {
            let _ = tx.send(want);
        }
    }

    fn run(rx: Receiver<Awake>) {
        let conn = match zbus::blocking::Connection::session() {
            Ok(c) => c,
            Err(e) => {
                log::warn!("keep awake: no session bus: {e}");
                return;
            }
        };
        let mut request: Option<OwnedObjectPath> = None;
        let mut cookie: Option<u32> = None;
        for want in rx {
            if let Some(h) = request.take() {
                let _ = conn.call_method(Some(PORTAL), h.as_str(), Some("org.freedesktop.portal.Request"), "Close", &());
            }
            if let Some(c) = cookie.take() {
                let _ = conn.call_method(Some(SAVER), "/org/freedesktop/ScreenSaver", Some(SAVER), "UnInhibit", &(c,));
            }
            if want == Awake::No {
                continue;
            }
            let display = want == Awake::Display;
            let reason = if display { t!("Ein Video läuft") } else { t!("Musik läuft") };
            // Portal flags: 4 = suspend, 8 = idle (screen saver, blanking).
            let flags: u32 = if display { 4 | 8 } else { 4 };
            let mut opts: HashMap<&str, Value> = HashMap::new();
            opts.insert("reason", Value::from(reason));
            match conn.call_method(Some(PORTAL), "/org/freedesktop/portal/desktop", Some("org.freedesktop.portal.Inhibit"), "Inhibit", &("", flags, opts)) {
                Ok(reply) => request = reply.body().deserialize::<OwnedObjectPath>().ok(),
                Err(e) => log::info!("keep awake: portal: {e}"),
            }
            // Desktops whose portal has no Inhibit still have this one.
            if display {
                match conn.call_method(Some(SAVER), "/org/freedesktop/ScreenSaver", Some(SAVER), "Inhibit", &("Velouna", reason)) {
                    Ok(reply) => cookie = reply.body().deserialize::<u32>().ok(),
                    Err(e) => log::info!("keep awake: ScreenSaver: {e}"),
                }
            }
            if request.is_none() && cookie.is_none() {
                log::warn!("keep awake: neither the portal nor org.freedesktop.ScreenSaver took the inhibition");
            }
        }
    }
}

/// Reads a system font by its Windows file name; on Android the closest
/// system font is used instead.
pub fn font(windows_name: &str) -> Option<Vec<u8>> {
    #[cfg(windows)]
    {
        std::fs::read(format!("C:\\Windows\\Fonts\\{windows_name}")).ok()
    }
    #[cfg(target_os = "android")]
    {
        let candidates: &[&str] = match windows_name {
            "segoeui.ttf" => &["RobotoStatic-Regular.ttf", "Roboto-Regular.ttf", "DroidSans.ttf"],
            "seguisb.ttf" => &["DroidSans-Bold.ttf", "RobotoStatic-Regular.ttf"],
            "seguisym.ttf" => &["NotoSansSymbols-Regular-Subsetted.ttf"],
            "consola.ttf" => &["DroidSansMono.ttf"],
            // One CJK collection covers Japanese, Chinese and Korean.
            "YuGothM.ttc" => &["NotoSansCJK-Regular.ttc"],
            _ => &[],
        };
        candidates.iter().find_map(|f| std::fs::read(format!("/system/fonts/{f}")).ok())
    }
    #[cfg(target_os = "linux")]
    {
        // Closest fonts of common distributions (Pop!_OS: Fira and Noto).
        let candidates: &[&str] = match windows_name {
            "segoeui.ttf" => &["NotoSans-Regular.ttf", "FiraSans-Regular.ttf", "Ubuntu-R.ttf", "Cantarell-Regular.otf", "DejaVuSans.ttf"],
            "seguisb.ttf" => &["NotoSans-SemiBold.ttf", "FiraSans-SemiBold.ttf", "NotoSans-Bold.ttf", "Ubuntu-M.ttf", "DejaVuSans-Bold.ttf"],
            "seguisym.ttf" => &["NotoSansSymbols2-Regular.ttf", "NotoSansSymbols-Regular.ttf", "DejaVuSans.ttf"],
            "consola.ttf" => &["FiraMono-Regular.ttf", "NotoSansMono-Regular.ttf", "UbuntuMono-R.ttf", "DejaVuSansMono.ttf"],
            "YuGothM.ttc" => &["NotoSansCJK-Regular.ttc", "NotoSansCJKjp-Regular.otf", "DroidSansFallbackFull.ttf"],
            _ => &[],
        };
        linux_fonts::find(candidates)
    }
    #[cfg(not(any(windows, target_os = "android", target_os = "linux")))]
    {
        let _ = windows_name;
        None
    }
}

#[cfg(target_os = "linux")]
mod linux_fonts {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    /// All font files below the usual font folders, indexed once.
    fn index() -> &'static Vec<PathBuf> {
        static I: OnceLock<Vec<PathBuf>> = OnceLock::new();
        I.get_or_init(|| {
            let mut roots = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
            if let Some(h) = std::env::var_os("HOME") {
                roots.push(Path::new(&h).join(".local/share/fonts"));
                roots.push(Path::new(&h).join(".fonts"));
            }
            let mut out = Vec::new();
            for r in roots {
                walk(&r, &mut out, 0);
            }
            out
        })
    }
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: u8) {
        if depth > 6 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out, depth + 1);
            } else {
                out.push(p);
            }
        }
    }
    pub fn find(names: &[&str]) -> Option<Vec<u8>> {
        let idx = index();
        names.iter().find_map(|n| idx.iter().find(|p| p.file_name().is_some_and(|f| f == *n)).and_then(|p| std::fs::read(p).ok()))
    }
}

/// App data folder override (Android: the app's private storage).
pub fn data_dir_override() -> Option<std::path::PathBuf> {
    std::env::var_os("VELOUNA_DATA_DIR").map(Into::into)
}

pub fn cache_dir_override() -> Option<std::path::PathBuf> {
    std::env::var_os("VELOUNA_CACHE_DIR").map(Into::into)
}

/// Text from the system clipboard.
pub fn clipboard_text() -> String {
    #[cfg(not(target_os = "android"))]
    {
        arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()).unwrap_or_default()
    }
    #[cfg(target_os = "android")]
    {
        android::clipboard_text().unwrap_or_default()
    }
}

// ------------------------------------------------------------ secrets

/// Secret storage: Windows Credential Manager or the Linux Secret Service
/// (GNOME Keyring, KWallet) on desktop; on Android a file
/// in the app's private storage (sandboxed per app by the OS).
pub mod secret {
    /// A separate data folder (`VELOUNA_DATA_DIR`) is a separate profile,
    /// sign-ins included: its secrets get their own entries.
    #[cfg(not(target_os = "android"))]
    fn service_name(service: &str) -> String {
        match super::data_dir_override() {
            Some(d) => format!("{service} ({})", d.display()),
            None => service.to_string(),
        }
    }
    #[cfg(not(target_os = "android"))]
    pub fn get(service: &str, user: &str) -> Option<String> {
        keyring::Entry::new(&service_name(service), user).ok()?.get_password().ok().filter(|s| !s.is_empty())
    }
    #[cfg(not(target_os = "android"))]
    pub fn set(service: &str, user: &str, value: &str) -> bool {
        keyring::Entry::new(&service_name(service), user).map(|e| e.set_password(value).is_ok()).unwrap_or(false)
    }
    #[cfg(not(target_os = "android"))]
    pub fn delete(service: &str, user: &str) {
        if let Ok(e) = keyring::Entry::new(&service_name(service), user) {
            let _ = e.delete_credential();
        }
    }

    #[cfg(target_os = "android")]
    fn path(service: &str, user: &str) -> Option<std::path::PathBuf> {
        Some(super::data_dir_override()?.join("secrets").join(format!("{service}.{user}")))
    }
    #[cfg(target_os = "android")]
    pub fn get(service: &str, user: &str) -> Option<String> {
        std::fs::read_to_string(path(service, user)?).ok().filter(|s| !s.is_empty())
    }
    #[cfg(target_os = "android")]
    pub fn set(service: &str, user: &str, value: &str) -> bool {
        let Some(p) = path(service, user) else { return false };
        p.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(p, value).is_ok()
    }
    #[cfg(target_os = "android")]
    pub fn delete(service: &str, user: &str) {
        if let Some(p) = path(service, user) {
            let _ = std::fs::remove_file(p);
        }
    }
}
