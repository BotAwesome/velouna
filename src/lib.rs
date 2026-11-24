//! Velouna as a library: the Windows exe (src/main.rs) and the Android
//! library (android/velouna-android) both start the same app from here.

#[macro_use]
extern crate velouna_i18n_macros;

// On Android the file dialog API comes from a stand-in crate under the same name.
#[cfg(target_os = "android")]
extern crate velouna_rfd_shim as rfd;

mod app;
mod config;
mod crash;
mod epg;
mod flags;
mod hotkeys;
mod i18n;
#[cfg(not(target_os = "android"))]
mod instance;
mod iptv;
mod library;
mod logo;
mod media;
mod mobile;
mod mpv;
mod pick;
mod platform;
pub mod plugins;
mod player;
mod plex;
mod radio;
mod theme;
mod ui_dialogs;
mod ui_library;
mod ui_mobile;
mod ui_touch_editor;
mod ui_plex_drawer;
mod ui_panels;
mod ui_player;
mod ui_plex;
mod ui_plugins;
mod ui_ext;
mod ui_update;
mod cache;
mod ui_queue;
mod ui_viz;
mod update;
mod video;
mod viz;
mod widgets;

use eframe::egui;

/// The Velouna mark as window/taskbar icon.
fn app_icon() -> egui::IconData {
    let n = 64usize;
    egui::IconData { rgba: logo::logo_rasterize(n), width: n as u32, height: n as u32 }
}

fn base_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        vsync: true,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    }
}

/// Set at startup; the first frame then maximizes the window.
pub(crate) static MAXIMIZE_AFTER_SHOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(windows)]
mod win32 {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct WinRect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }
    #[repr(C)]
    #[derive(Default)]
    pub struct Placement {
        pub length: u32,
        pub flags: u32,
        pub show_cmd: u32,
        pub min_pos: [i32; 2],
        pub max_pos: [i32; 2],
        pub normal: WinRect,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn SystemParametersInfoW(action: u32, param: u32, pv: *mut c_void, win_ini: u32) -> i32;
        pub fn GetDpiForSystem() -> u32;
        pub fn GetWindowPlacement(hwnd: *mut c_void, p: *mut Placement) -> i32;
        pub fn SetWindowPlacement(hwnd: *mut c_void, p: *const Placement) -> i32;
    }

    /// Primary monitor's work area (without the taskbar). Before winit makes
    /// the process DPI aware this is in 96-dpi units, afterwards in pixels.
    pub fn work_area_raw() -> Option<WinRect> {
        const SPI_GETWORKAREA: u32 = 0x0030;
        let mut r = WinRect::default();
        let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut r as *mut WinRect).cast(), 0) } != 0;
        (ok && r.right - r.left > 300 && r.bottom - r.top > 300).then_some(r)
    }
}

/// The primary monitor's work area (without the taskbar) in points.
#[cfg(windows)]
fn work_area() -> Option<egui::Rect> {
    let r = win32::work_area_raw()?;
    // GetDpiForSystem says 96 while the process is not DPI aware yet.
    let scale = unsafe { win32::GetDpiForSystem() }.max(96) as f32 / 96.0;
    Some(egui::Rect::from_min_max(egui::pos2(r.left as f32, r.top as f32) / scale, egui::pos2(r.right as f32, r.bottom as f32) / scale))
}

/// Maximizes the window and gives it a sensible "restore" size (centered,
/// `normal` points): it was created as large as the work area, which Windows
/// would otherwise restore to.
#[cfg(windows)]
pub(crate) fn maximize_with_restore_size(frame: &eframe::Frame, normal: egui::Vec2, pixels_per_point: f32) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = frame.window_handle() else { return };
    let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
    let hwnd = h.hwnd.get() as *mut std::ffi::c_void;
    let Some(area) = win32::work_area_raw() else { return };
    let mut p = win32::Placement { length: std::mem::size_of::<win32::Placement>() as u32, ..Default::default() };
    if unsafe { win32::GetWindowPlacement(hwnd, &mut p) } == 0 {
        return;
    }
    // rcNormalPosition is relative to the work area ("workspace coordinates").
    let (aw, ah) = (area.right - area.left, area.bottom - area.top);
    let w = ((normal.x * pixels_per_point) as i32).min(aw);
    let h = ((normal.y * pixels_per_point) as i32).min(ah);
    let (x, y) = ((aw - w) / 2, (ah - h) / 2);
    p.normal = win32::WinRect { left: x, top: y, right: x + w, bottom: y + h };
    p.show_cmd = 3; // SW_SHOWMAXIMIZED
    p.flags = 0;
    unsafe { win32::SetWindowPlacement(hwnd, &p) };
}

#[cfg(not(windows))]
fn work_area() -> Option<egui::Rect> {
    None
}

/// Desktop entry point; `files` are paths/URLs from the command line.
/// `phone` opens the phone simulator (touch layout, F9 rotates).
/// Desktop log: stderr and `<cache>/velouna/velouna.log` (new each start).
#[cfg(not(target_os = "android"))]
mod desktop_log {
    use std::io::Write;
    use std::sync::Mutex;
    struct Logger(Mutex<Option<std::fs::File>>);
    impl log::Log for Logger {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= log::Level::Info && (m.target().starts_with("velouna") || m.level() <= log::Level::Warn)
        }
        fn log(&self, r: &log::Record) {
            if !self.enabled(r.metadata()) {
                return;
            }
            let line = format!("[{}] {}\n", r.level(), r.args());
            eprint!("{line}");
            if let Ok(mut f) = self.0.lock() {
                if let Some(f) = f.as_mut() {
                    let _ = f.write_all(line.as_bytes());
                }
            }
        }
        fn flush(&self) {}
    }
    pub fn init() {
        let dir = crate::platform::cache_dir_override().or_else(|| dirs::cache_dir().map(|d| d.join("velouna")));
        let file = dir.and_then(|d| {
            let _ = std::fs::create_dir_all(&d);
            std::fs::File::create(d.join("velouna.log")).ok()
        });
        let logger: &'static Logger = Box::leak(Box::new(Logger(Mutex::new(file))));
        if log::set_logger(logger).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
        log::info!("Velouna {} ({} {})", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
    }
}

/// The Linux packages bring their own OpenSSL (inside libmpv and ffmpeg),
/// built on Ubuntu, which looks for CA certificates in /usr/lib/ssl. Point
/// it at the distribution's bundle so https streams verify everywhere;
/// ffmpeg, ffprobe and yt-dlp inherit the variable.
#[cfg(target_os = "linux")]
fn set_cert_bundle() {
    if std::env::var_os("SSL_CERT_FILE").is_some() {
        return;
    }
    let bundles = [
        "/etc/ssl/certs/ca-certificates.crt",                // Debian, Ubuntu, Pop!_OS, Arch, Gentoo
        "/etc/pki/tls/certs/ca-bundle.crt",                  // Fedora, RHEL
        "/etc/ssl/ca-bundle.pem",                            // openSUSE
        "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem", // Fedora (newer)
        "/etc/ssl/cert.pem",                                 // Alpine, Void
    ];
    match bundles.iter().find(|p| std::path::Path::new(p).is_file()) {
        // SAFETY: runs first thing in run_desktop, before any thread starts.
        Some(p) => unsafe { std::env::set_var("SSL_CERT_FILE", p) },
        None => eprintln!("no CA certificate bundle found; https streams may fail (install ca-certificates)"),
    }
}

/// Start of the click-to-picture clock (see `mpv::timing`).
pub fn program_start() {
    mpv::timing::program_start();
}

pub fn run_desktop(files: Vec<String>, phone: bool) -> eframe::Result<()> {
    crash::install();
    // First, while no other thread runs (set_var).
    #[cfg(target_os = "linux")]
    {
        set_cert_bundle();
        update::linux::init();
    }
    // Another Velouna is running: it takes the files, this one quits. The
    // phone simulator is a development tool and always opens its own window.
    #[cfg(not(target_os = "android"))]
    let note = match if phone { instance::Start::Run(None) } else { instance::hand_over_or_listen(&files) } {
        instance::Start::HandedOver => return Ok(()),
        instance::Start::Run(note) => note,
    };
    #[cfg(not(target_os = "android"))]
    {
        desktop_log::init();
        if let Some(n) = note {
            log::warn!("{n}");
        }
    }
    mobile::set_simulator(phone);
    let size = if phone { [390.0, 800.0] } else { [1280.0, 760.0] };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(if phone { "Velouna – Handy-Simulator (F9 dreht)" } else { "Velouna" })
        .with_inner_size(size)
        .with_min_inner_size([300.0, 300.0])
        .with_decorations(false)
        .with_resizable(true)
        .with_drag_and_drop(true)
        .with_icon(app_icon())
        // Linux: ties the window to velouna.desktop (icon, dock, "Open with").
        .with_app_id("velouna")
        // An alpha channel, so the controls over an HDR video can be read
        // back with their transparency (platform::hdr). Every paint is
        // opaque otherwise.
        .with_transparent(cfg!(windows));
    let mut centered = !phone;
    if phone {
        // Top left, clear of the taskbar.
        viewport = viewport.with_position([40.0, 4.0]);
    } else if let Some(area) = work_area() {
        // Not `with_maximized`: on Windows that shows the still hidden,
        // unpainted window (white) and hides it again, two or three white
        // flashes on startup. Instead the window starts as large as the
        // work area and is maximized after the first frame is shown.
        viewport = viewport.with_position(area.min).with_inner_size(area.size());
        centered = false;
        MAXIMIZE_AFTER_SHOW.store(true, std::sync::atomic::Ordering::Relaxed);
    } else {
        viewport = viewport.with_maximized(true);
    }
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions { viewport, centered, ..base_options() };
    // winit 0.30 has no file drag & drop on Wayland; VELOUNA_X11=1 runs
    // Velouna through XWayland, where dropping files works.
    #[cfg(target_os = "linux")]
    if std::env::var_os("VELOUNA_X11").is_some_and(|v| v != "0") && std::env::var_os("DISPLAY").is_some() {
        log::info!("VELOUNA_X11: using X11 (XWayland)");
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_x11();
        }));
    }
    eframe::run_native("Velouna", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, files)))))
}

/// Android entry point, called from `android_main` in the velouna-android crate.
#[cfg(target_os = "android")]
pub fn run_android(android_app: winit::platform::android::activity::AndroidApp) {
    platform::android::init(&android_app);
    crash::install();
    let options = eframe::NativeOptions { android_app: Some(android_app), ..base_options() };
    if let Err(e) = eframe::run_native("Velouna", options, Box::new(|cc| Ok(Box::new(app::App::new(cc, Vec::new()))))) {
        log::error!("eframe: {e}");
    }
}
