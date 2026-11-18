//! Per-user install (no admin rights): files under %LOCALAPPDATA%\Programs,
//! Start-menu/desktop shortcuts, "Apps & features" entry and the optional
//! "Open with" registration. Everything lives under HKCU.

use crate::payload::{self, Footer};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use winreg::RegKey;
use winreg::enums::*;

include!(concat!(env!("OUT_DIR"), "/exts.rs"));

pub const APP_EXE: &str = "velouna.exe";
pub const UNINSTALL_EXE: &str = "uninstall.exe";
const PROG_ID: &str = "Velouna.Media";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Velouna";
const CAPS_KEY: &str = r"Software\Velouna\Capabilities";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub desktop: bool,
    pub open_with: bool,
}

/// What is already on this machine.
#[derive(Clone, Debug)]
pub struct Existing {
    pub dir: PathBuf,
    pub version: String,
    pub options: Options,
    /// An install under the earlier name Velour, replaced by this one.
    pub legacy: bool,
}

// The app was called Velour until 0.2.x: an install of it is replaced (its
// files, shortcuts and registry entries go; settings and library stay and
// are moved over by the app on its first start).
const LEGACY_EXE: &str = "velour.exe";
const LEGACY_PROG_ID: &str = "Velour.Media";
const LEGACY_UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Velour";
const LEGACY_CAPS_KEY: &str = r"Software\Velour\Capabilities";

/// A Velour install from before the rename, if there is one.
pub fn legacy_existing() -> Option<Existing> {
    let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey(LEGACY_UNINSTALL_KEY).ok()?;
    let old: String = k.get_value("InstallLocation").ok()?;
    let old = PathBuf::from(old);
    let flag = |n: &str| k.get_value::<u32, _>(n).map(|v| v != 0);
    // Its default folder ...\Velour becomes ...\Velouna; a folder of the
    // user's own naming stays.
    let dir = if old.file_name().is_some_and(|n| n.eq_ignore_ascii_case("Velour")) { old.with_file_name("Velouna") } else { old };
    Some(Existing {
        dir,
        version: k.get_value("DisplayVersion").unwrap_or_default(),
        options: Options { desktop: flag("VelourDesktop").unwrap_or(false), open_with: flag("VelourOpenWith").unwrap_or(true) },
        legacy: true,
    })
}

/// True while the old Velour is running.
pub fn legacy_running() -> bool {
    let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(LEGACY_UNINSTALL_KEY) else { return false };
    let Ok(dir) = k.get_value::<String, _>("InstallLocation") else { return false };
    let exe = Path::new(&dir).join(LEGACY_EXE);
    exe.exists() && std::fs::OpenOptions::new().write(true).open(&exe).is_err()
}

/// Removes what a Velour install put on the machine, except the user's
/// data. `dir` is where Velouna now lives (may be the same folder).
fn remove_legacy(dir: &Path) {
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(k) = hk.open_subkey(LEGACY_UNINSTALL_KEY) else { return };
    let old: Option<PathBuf> = k.get_value::<String, _>("InstallLocation").ok().map(PathBuf::from);
    drop(k);
    if let Some(old) = old {
        let _ = std::fs::remove_file(old.join(LEGACY_EXE));
        if old != dir {
            for name in ["libmpv-2.dll", "ffmpeg.exe", "ffprobe.exe", UNINSTALL_EXE] {
                let _ = std::fs::remove_file(old.join(name));
            }
            let _ = std::fs::remove_dir_all(old.join("licenses"));
            let _ = std::fs::remove_dir(&old); // only if empty now
        }
    }
    let links = [dirs::data_dir().map(|d| d.join(r"Microsoft\Windows\Start Menu\Programs\Velour.lnk")), dirs::desktop_dir().map(|d| d.join("Velour.lnk"))];
    for l in links.into_iter().flatten() {
        let _ = std::fs::remove_file(l);
    }
    let _ = hk.delete_subkey_all(format!(r"Software\Classes\{LEGACY_PROG_ID}"));
    let _ = hk.delete_subkey_all(format!(r"Software\Classes\Applications\{LEGACY_EXE}"));
    let _ = hk.delete_subkey_all(r"Software\Classes\Directory\shell\Velour");
    let _ = hk.delete_subkey_all(LEGACY_CAPS_KEY);
    if let Ok(v) = hk.open_subkey_with_flags(r"Software\Velour", KEY_READ) {
        if v.enum_keys().next().is_none() && v.enum_values().next().is_none() {
            drop(v);
            let _ = hk.delete_subkey(r"Software\Velour");
        }
    }
    if let Ok(r) = hk.open_subkey_with_flags(r"Software\RegisteredApplications", KEY_SET_VALUE) {
        let _ = r.delete_value("Velour");
    }
    for e in MEDIA_EXT {
        if let Ok(k) = hk.open_subkey_with_flags(format!(r"Software\Classes\.{e}\OpenWithProgids"), KEY_SET_VALUE) {
            let _ = k.delete_value(LEGACY_PROG_ID);
        }
    }
    let _ = hk.delete_subkey_all(LEGACY_UNINSTALL_KEY);
}

pub fn default_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("Programs").join("Velouna")
}

pub fn existing() -> Option<Existing> {
    let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey(UNINSTALL_KEY).ok()?;
    let dir: String = k.get_value("InstallLocation").ok()?;
    let flag = |n: &str| k.get_value::<u32, _>(n).map(|v| v != 0);
    Some(Existing {
        dir: PathBuf::from(dir),
        version: k.get_value("DisplayVersion").unwrap_or_default(),
        options: Options { desktop: flag("VelounaDesktop").unwrap_or(false), open_with: flag("VelounaOpenWith").unwrap_or(true) },
        legacy: false,
    })
}

fn start_menu_link() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(r"Microsoft\Windows\Start Menu\Programs\Velouna.lnk"))
}

fn desktop_link() -> Option<PathBuf> {
    Some(dirs::desktop_dir()?.join("Velouna.lnk"))
}

/// True while Velouna (from `dir`) is running: its exe can't be opened for writing.
pub fn app_running(dir: &Path) -> bool {
    let exe = dir.join(APP_EXE);
    exe.exists() && std::fs::OpenOptions::new().write(true).open(&exe).is_err()
}

pub fn close_app() {
    let _ = hidden("taskkill", &["/IM", APP_EXE, "/F"]).status();
    if legacy_running() {
        let _ = hidden("taskkill", &["/IM", LEGACY_EXE, "/F"]).status();
    }
    std::thread::sleep(std::time::Duration::from_millis(600));
}

fn hidden(program: &str, args: &[&str]) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = std::process::Command::new(program);
    c.args(args).creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    c
}

/// Shared progress between the worker thread and the UI.
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub current: Mutex<String>,
}

pub fn install(exe: &Path, footer: &Footer, dir: &Path, opts: Options, progress: Arc<Progress>) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    progress.total.store(footer.unpacked, Ordering::Relaxed);
    let p2 = progress.clone();
    let base = dir.to_path_buf();
    payload::unpack(
        exe,
        footer,
        |name| {
            *p2.current.lock().unwrap() = name.to_string();
            let path = base.join(name.replace('/', "\\"));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            Ok(Box::new(io::BufWriter::with_capacity(1 << 20, std::fs::File::create(path)?)))
        },
        |n| progress.done.store(n, Ordering::Relaxed),
    )?;

    // The stub alone (without payload) is the uninstaller.
    *progress.current.lock().unwrap() = UNINSTALL_EXE.into();
    let bytes = std::fs::read(exe)?;
    std::fs::write(dir.join(UNINSTALL_EXE), &bytes[..footer.offset as usize])?;

    let app = dir.join(APP_EXE);
    *progress.current.lock().unwrap() = crate::tx("Verknüpfungen", "Shortcuts").into();
    if let Some(l) = start_menu_link() {
        make_link(&app, dir, &l)?;
    }
    if let Some(l) = desktop_link() {
        if opts.desktop {
            make_link(&app, dir, &l)?;
        } else {
            let _ = std::fs::remove_file(l);
        }
    }

    // Before the new entries: the old ones share extensions and menus.
    remove_legacy(dir);
    register(dir, &footer.version, opts)?;
    if opts.open_with {
        register_open_with(&app)?;
    } else {
        unregister_open_with();
    }
    notify_assoc_changed();
    Ok(())
}

fn make_link(target: &Path, dir: &Path, link: &Path) -> io::Result<()> {
    if let Some(p) = link.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut sl = mslnk::ShellLink::new(target).map_err(io::Error::other)?;
    sl.set_working_dir(Some(dir.to_string_lossy().to_string()));
    sl.set_icon_location(Some(target.to_string_lossy().to_string()));
    sl.set_name(Some("Velouna".into()));
    sl.create_lnk(link).map_err(io::Error::other)
}

fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

fn register(dir: &Path, version: &str, opts: Options) -> io::Result<()> {
    let (k, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(UNINSTALL_KEY)?;
    let app = dir.join(APP_EXE).to_string_lossy().to_string();
    let un = dir.join(UNINSTALL_EXE).to_string_lossy().to_string();
    k.set_value("DisplayName", &"Velouna")?;
    k.set_value("DisplayVersion", &version)?;
    k.set_value("Publisher", &"Velouna")?;
    k.set_value("DisplayIcon", &format!("{app},0"))?;
    k.set_value("InstallLocation", &dir.to_string_lossy().to_string())?;
    k.set_value("UninstallString", &format!("\"{un}\""))?;
    k.set_value("QuietUninstallString", &format!("\"{un}\" --quiet"))?;
    k.set_value("NoModify", &1u32)?;
    k.set_value("NoRepair", &1u32)?;
    k.set_value("EstimatedSize", &((dir_size(dir) / 1024) as u32))?;
    k.set_value("VelounaDesktop", &(opts.desktop as u32))?;
    k.set_value("VelounaOpenWith", &(opts.open_with as u32))?;
    Ok(())
}

fn register_open_with(app: &Path) -> io::Result<()> {
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    let app_s = app.to_string_lossy().to_string();
    let cmd = format!("\"{app_s}\" \"%1\"");
    let media = if crate::en() { "Media file" } else { "Mediendatei" };
    let desc = if crate::en() { "Video and audio player with library and Plex" } else { "Video- und Audioplayer mit Bibliothek und Plex" };
    let play = if crate::en() { "Play with Velouna" } else { "Mit Velouna abspielen" };

    let (k, _) = hk.create_subkey(format!(r"Software\Classes\{PROG_ID}"))?;
    k.set_value("", &media)?;
    hk.create_subkey(format!(r"Software\Classes\{PROG_ID}\DefaultIcon"))?.0.set_value("", &format!("{app_s},0"))?;
    hk.create_subkey(format!(r"Software\Classes\{PROG_ID}\shell\open"))?.0.set_value("FriendlyAppName", &"Velouna")?;
    hk.create_subkey(format!(r"Software\Classes\{PROG_ID}\shell\open\command"))?.0.set_value("", &cmd)?;

    let (a, _) = hk.create_subkey(format!(r"Software\Classes\Applications\{APP_EXE}"))?;
    a.set_value("FriendlyAppName", &"Velouna")?;
    hk.create_subkey(format!(r"Software\Classes\Applications\{APP_EXE}\shell\open\command"))?.0.set_value("", &cmd)?;
    let (types, _) = hk.create_subkey(format!(r"Software\Classes\Applications\{APP_EXE}\SupportedTypes"))?;

    let (caps, _) = hk.create_subkey(CAPS_KEY)?;
    caps.set_value("ApplicationName", &"Velouna")?;
    caps.set_value("ApplicationDescription", &desc)?;
    let (fa, _) = hk.create_subkey(format!(r"{CAPS_KEY}\FileAssociations"))?;
    hk.create_subkey(r"Software\RegisteredApplications")?.0.set_value("Velouna", &CAPS_KEY)?;

    for e in MEDIA_EXT {
        hk.create_subkey(format!(r"Software\Classes\.{e}\OpenWithProgids"))?.0.set_value(PROG_ID, &"")?;
        types.set_value(format!(".{e}"), &"")?;
        fa.set_value(format!(".{e}"), &PROG_ID)?;
    }

    let (d, _) = hk.create_subkey(r"Software\Classes\Directory\shell\Velouna")?;
    d.set_value("", &play)?;
    d.set_value("Icon", &format!("{app_s},0"))?;
    hk.create_subkey(r"Software\Classes\Directory\shell\Velouna\command")?.0.set_value("", &cmd)?;
    Ok(())
}

fn unregister_open_with() {
    let hk = RegKey::predef(HKEY_CURRENT_USER);
    let _ = hk.delete_subkey_all(format!(r"Software\Classes\{PROG_ID}"));
    let _ = hk.delete_subkey_all(format!(r"Software\Classes\Applications\{APP_EXE}"));
    let _ = hk.delete_subkey_all(r"Software\Classes\Directory\shell\Velouna");
    let _ = hk.delete_subkey_all(CAPS_KEY);
    if let Ok(v) = hk.open_subkey_with_flags(r"Software\Velouna", KEY_READ) {
        if v.enum_keys().next().is_none() && v.enum_values().next().is_none() {
            drop(v);
            let _ = hk.delete_subkey(r"Software\Velouna");
        }
    }
    if let Ok(r) = hk.open_subkey_with_flags(r"Software\RegisteredApplications", KEY_SET_VALUE) {
        let _ = r.delete_value("Velouna");
    }
    for e in MEDIA_EXT {
        if let Ok(k) = hk.open_subkey_with_flags(format!(r"Software\Classes\.{e}\OpenWithProgids"), KEY_SET_VALUE) {
            let _ = k.delete_value(PROG_ID);
        }
    }
}

fn notify_assoc_changed() {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHChangeNotify(event: i32, flags: u32, a: *const u8, b: *const u8);
    }
    const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, 0, std::ptr::null(), std::ptr::null()) };
}

/// Removes Velouna. With `purge` also settings, library data, caches and the
/// stored Plex sign-in.
pub fn uninstall(dir: &Path, purge: bool, progress: Arc<Progress>) -> io::Result<()> {
    progress.total.store(4, Ordering::Relaxed);
    let step = |n: u64, what: &str| {
        *progress.current.lock().unwrap() = what.to_string();
        progress.done.store(n, Ordering::Relaxed);
    };
    step(0, APP_EXE);
    // Files: never remove anything but what an install put there.
    for name in [APP_EXE, "libmpv-2.dll", "ffmpeg.exe", "ffprobe.exe", UNINSTALL_EXE] {
        let p = dir.join(name);
        if p.exists() {
            std::fs::remove_file(&p)?;
        }
    }
    let _ = std::fs::remove_dir_all(dir.join("licenses"));
    let _ = std::fs::remove_dir(dir); // only if empty now

    step(1, crate::tx("Verknüpfungen", "Shortcuts"));
    for l in [start_menu_link(), desktop_link()].into_iter().flatten() {
        let _ = std::fs::remove_file(l);
    }

    step(2, "Registry");
    unregister_open_with();
    let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(UNINSTALL_KEY);
    notify_assoc_changed();

    if purge {
        step(3, crate::tx("Einstellungen", "Settings"));
        purge_data();
    }
    step(4, "");
    Ok(())
}

/// Settings, library, history, caches and the stored Plex sign-in.
pub fn purge_data() {
    // Also what an earlier name left behind.
    for base in [dirs::config_dir(), dirs::cache_dir()].into_iter().flatten() {
        for name in ["Velouna", "Velour"] {
            let _ = std::fs::remove_dir_all(base.join(name));
        }
    }
    for service in ["Velouna", "Velour"] {
        if let Ok(e) = keyring::Entry::new(service, "plex-token") {
            let _ = e.delete_credential();
        }
    }
}

/// The uninstaller can't delete itself while running from the install
/// folder, so it relaunches from a temp copy first.
pub fn relaunch_from_temp(dir: &Path, quiet: bool) -> io::Result<()> {
    let me = std::env::current_exe()?;
    let tmp = std::env::temp_dir().join(format!("velouna-uninstall-{}.exe", std::process::id()));
    std::fs::copy(&me, &tmp)?;
    let mut c = std::process::Command::new(&tmp);
    c.arg("--uninstall-dir").arg(dir);
    if quiet {
        c.arg("--quiet");
    }
    c.spawn()?;
    Ok(())
}

/// Deletes the temp uninstaller shortly after it exits.
pub fn delete_self_later() {
    if let Ok(me) = std::env::current_exe() {
        if me.starts_with(std::env::temp_dir()) {
            use std::os::windows::process::CommandExt;
            // raw_arg: cmd.exe must see the quotes exactly as written.
            let line = format!("/c ping 127.0.0.1 -n 3 > nul & del /f /q \"{}\"", me.display());
            let _ = hidden("cmd", &[]).raw_arg(line).spawn();
        }
    }
}
