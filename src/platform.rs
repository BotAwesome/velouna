//! The few things that differ between Windows and Android, behind one
//! interface: opening URLs, system fonts, data folders.

#[cfg(target_os = "android")]
pub mod android;

/// True on phones/tablets (touch-first UI, no window chrome).
#[allow(dead_code)]
pub const MOBILE: bool = cfg!(target_os = "android");

/// mpv `hwdec` when hardware decoding is on. Android: MediaCodec, copied
/// back so it works with the OpenGL render API.
pub const HWDEC: &str = if MOBILE { "mediacodec-copy" } else { "auto-safe" };

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
    #[cfg(not(any(windows, target_os = "android")))]
    {
        let _ = windows_name;
        None
    }
}

/// App data folder override (Android: the app's private storage).
pub fn data_dir_override() -> Option<std::path::PathBuf> {
    std::env::var_os("VELOUR_DATA_DIR").map(Into::into)
}

pub fn cache_dir_override() -> Option<std::path::PathBuf> {
    std::env::var_os("VELOUR_CACHE_DIR").map(Into::into)
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

/// Secret storage: Windows Credential Manager on desktop; on Android a file
/// in the app's private storage (sandboxed per app by the OS).
pub mod secret {
    /// A separate data folder (`VELOUR_DATA_DIR`) is a separate profile,
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
