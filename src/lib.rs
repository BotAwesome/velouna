//! Velour as a library: the Windows exe (src/main.rs) and the Android
//! library (android/velour-android) both start the same app from here.

#[macro_use]
extern crate velour_i18n_macros;

// On Android the file dialog API comes from a stand-in crate under the same name.
#[cfg(target_os = "android")]
extern crate velour_rfd_shim as rfd;

mod app;
mod config;
mod epg;
mod flags;
mod hotkeys;
mod i18n;
mod iptv;
mod library;
mod logo;
mod media;
mod mobile;
mod mpv;
mod pick;
mod platform;
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
mod ui_update;
mod update;
mod video;
mod widgets;

use eframe::egui;

/// The Velour mark as window/taskbar icon.
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
pub fn run_desktop(files: Vec<String>, phone: bool) -> eframe::Result<()> {
    mobile::set_simulator(phone);
    let size = if phone { [390.0, 800.0] } else { [1280.0, 760.0] };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(if phone { "Velour – Handy-Simulator (F9 dreht)" } else { "Velour" })
        .with_inner_size(size)
        .with_min_inner_size([300.0, 300.0])
        .with_decorations(false)
        .with_resizable(true)
        .with_drag_and_drop(true)
        .with_icon(app_icon())
        // Linux: ties the window to velour.desktop (icon, dock, "Open with").
        .with_app_id("velour");
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
    let options = eframe::NativeOptions { viewport, centered, ..base_options() };
    eframe::run_native("Velour", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, files)))))
}

/// Android entry point, called from `android_main` in the velour-android crate.
#[cfg(target_os = "android")]
pub fn run_android(android_app: winit::platform::android::activity::AndroidApp) {
    platform::android::init(&android_app);
    let options = eframe::NativeOptions { android_app: Some(android_app), ..base_options() };
    if let Err(e) = eframe::run_native("Velour", options, Box::new(|cc| Ok(Box::new(app::App::new(cc, Vec::new()))))) {
        log::error!("eframe: {e}");
    }
}
