//! Touch layout for phones and tablets (portrait: bottom navigation,
//! landscape: navigation rail) plus the tools to design it quickly:
//!
//! - **Live metrics:** sizes come from `layout.json` in the data folder and
//!   are re-read whenever the file changes, without a rebuild or restart.
//! - **Phone simulator:** `velouna.exe --phone` opens a phone-sized window
//!   with simulated system bars; F9 rotates it.

use eframe::egui::{self, Rect, vec2};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Instant, SystemTime};

/// Every size of the touch layout, in points (≈ Android dp).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Metrics {
    /// Desktop windows narrower than this also get the touch layout.
    pub compact_below: f32,
    pub header_h: f32,
    /// Header height in landscape, where height is scarce.
    pub header_h_land: f32,
    pub nav_h: f32,
    pub rail_w: f32,
    pub mini_h: f32,
    /// Side padding of page content.
    pub pad: f32,
    pub tab_icon: f32,
    pub tab_label: f32,
    pub tab_pill_w: f32,
    pub tab_pill_h: f32,
    pub title_size: f32,
    pub icon_btn: f32,
    pub play_btn: f32,
    pub ctl_btn: f32,
    pub ctl_icon: f32,
    pub seek_h: f32,
    /// Controls hide after this long without a touch while playing.
    pub hide_after_ms: f32,
    /// Width of a double-tap seek zone (fraction of the video width).
    pub seek_zone: f32,
    pub seek_step_s: f32,
    /// Simulated system bars in the desktop simulator.
    pub sim_status: f32,
    pub sim_nav: f32,
    pub sim_cutout: f32,
    /// Simulated back-gesture strip (0 = button navigation).
    pub sim_gesture: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics {
            compact_below: 0.0,
            header_h: 54.0,
            header_h_land: 44.0,
            nav_h: 66.0,
            rail_w: 80.0,
            mini_h: 60.0,
            pad: 14.0,
            tab_icon: 22.0,
            tab_label: 11.5,
            tab_pill_w: 58.0,
            tab_pill_h: 30.0,
            title_size: 19.0,
            icon_btn: 42.0,
            play_btn: 66.0,
            ctl_btn: 48.0,
            ctl_icon: 24.0,
            seek_h: 34.0,
            hide_after_ms: 3500.0,
            seek_zone: 0.33,
            seek_step_s: 10.0,
            sim_status: 28.0,
            sim_nav: 18.0,
            sim_cutout: 30.0,
            sim_gesture: 0.0,
        }
    }
}

static METRICS: RwLock<Option<Metrics>> = RwLock::new(None);
static WATCH: Mutex<Option<(Instant, Option<SystemTime>)>> = Mutex::new(None);
static SIM: AtomicBool = AtomicBool::new(false);
static INSETS_PX: Mutex<Option<(Instant, [f32; 4])>> = Mutex::new(None);
/// Last time the insets changed or a text field had focus (Android).
static ACTIVE: Mutex<Option<Instant>> = Mutex::new(None);

pub fn m() -> Metrics {
    METRICS.read().ok().and_then(|g| *g).unwrap_or_default()
}

fn path() -> PathBuf {
    crate::config::data_dir().join("layout.json")
}

/// Re-reads `layout.json` when it changed (checked twice a second). Writes
/// the defaults once so there is a file to edit.
pub fn tick(ctx: &egui::Context) {
    // egui only draws on input, but the keyboard and system bars come and
    // go without any: keep drawing around text input and inset changes so
    // the layout follows. (Must be requested inside the frame; requests made
    // in raw_input_hook are dropped when the frame starts.)
    if crate::platform::MOBILE {
        if ctx.wants_keyboard_input() {
            *ACTIVE.lock().unwrap() = Some(Instant::now());
        }
        if ACTIVE.lock().unwrap().is_some_and(|at| at.elapsed().as_millis() < 2000) {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
    let mut w = WATCH.lock().unwrap();
    let due = w.as_ref().is_none_or(|(at, _)| at.elapsed().as_millis() > 500);
    if !due {
        return;
    }
    let p = path();
    let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
    let changed = w.as_ref().is_none_or(|(_, old)| *old != mtime);
    *w = Some((Instant::now(), mtime));
    drop(w);
    if !changed {
        return;
    }
    match std::fs::read_to_string(&p) {
        Ok(s) => match serde_json::from_str::<Metrics>(&s) {
            Ok(v) => {
                *METRICS.write().unwrap() = Some(v);
                log::info!("layout.json reloaded");
                // Add keys introduced since the file was written, so every
                // value stays editable.
                let have = serde_json::from_str::<serde_json::Value>(&s).ok().and_then(|j| j.as_object().map(|o| o.len())).unwrap_or(0);
                let full = serde_json::to_value(v).ok().and_then(|j| j.as_object().map(|o| o.len())).unwrap_or(0);
                if have < full {
                    let _ = std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap_or_default());
                }
            }
            Err(e) => log::warn!("layout.json: {e}"),
        },
        Err(_) if touch() => {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(&p, serde_json::to_string_pretty(&Metrics::default()).unwrap_or_default());
        }
        Err(_) => {}
    }
    ctx.request_repaint();
}

// ------------------------------------------------------------ simulator

pub fn set_simulator(on: bool) {
    SIM.store(on, Ordering::Relaxed);
}

pub fn simulator() -> bool {
    SIM.load(Ordering::Relaxed)
}

/// Phone or simulator: finger input (tap toggles controls, no hover).
pub fn touch() -> bool {
    crate::platform::MOBILE || simulator()
}

/// Use the touch layout (phones, simulator, very narrow desktop windows).
pub fn compact(ctx: &egui::Context) -> bool {
    touch() || ctx.screen_rect().width() < m().compact_below
}

pub fn portrait(ctx: &egui::Context) -> bool {
    let r = ctx.screen_rect();
    r.height() >= r.width()
}

/// F9 in the simulator swaps width and height.
pub fn simulator_keys(ctx: &egui::Context) {
    if simulator() && ctx.input(|i| i.key_pressed(egui::Key::F9)) {
        let r = ctx.input(|i| i.viewport().inner_rect).unwrap_or(ctx.screen_rect());
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(r.height(), r.width())));
    }
}

static FORCED_ROTATION: AtomicBool = AtomicBool::new(false);

/// Player rotate button: lock to the other orientation.
pub fn rotate(ctx: &egui::Context) {
    if simulator() {
        let r = ctx.input(|i| i.viewport().inner_rect).unwrap_or(ctx.screen_rect());
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(r.height(), r.width())));
        return;
    }
    #[cfg(target_os = "android")]
    {
        crate::platform::android::set_orientation(if portrait(ctx) { 6 } else { 7 });
        FORCED_ROTATION.store(true, Ordering::Relaxed);
    }
}

/// Landscape for the player-bar editor (released like `rotate`).
pub fn force_landscape(ctx: &egui::Context) {
    if simulator() {
        if portrait(ctx) {
            rotate(ctx);
        }
        return;
    }
    #[cfg(target_os = "android")]
    {
        crate::platform::android::set_orientation(6);
        FORCED_ROTATION.store(true, Ordering::Relaxed);
    }
}

/// Leaving the player gives the orientation back to the system.
pub fn release_rotation() {
    if FORCED_ROTATION.swap(false, Ordering::Relaxed) {
        #[cfg(target_os = "android")]
        crate::platform::android::set_orientation(-1);
    }
}

/// The Android back key (Escape in the simulator), consumed.
pub fn back_pressed(ctx: &egui::Context) -> bool {
    ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::BrowserBack) || (simulator() && i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)))
}

// ------------------------------------------------------------ safe area

/// Space taken by status bar, navigation bar and display cutout, in points:
/// [left, top, right, bottom].
pub fn insets(ctx: &egui::Context, full: Rect) -> [f32; 4] {
    if simulator() {
        let m = m();
        return if full.height() >= full.width() {
            [0.0, m.sim_status, 0.0, m.sim_nav]
        } else {
            [m.sim_cutout, m.sim_status * 0.85, 0.0, m.sim_nav * 0.7]
        };
    }
    #[cfg(target_os = "android")]
    {
        let mut g = INSETS_PX.lock().unwrap();
        if g.as_ref().is_none_or(|(at, _)| at.elapsed().as_millis() > 250) {
            let px = crate::platform::android::system_insets_px().unwrap_or([0.0; 4]);
            if g.as_ref().is_none_or(|(_, old)| *old != px) {
                *ACTIVE.lock().unwrap() = Some(Instant::now());
                ctx.request_repaint();
            }
            *g = Some((Instant::now(), px));
        }
        let ppp = ctx.pixels_per_point().max(0.5);
        let px = g.as_ref().map(|(_, v)| *v).unwrap_or([0.0; 4]);
        return px.map(|v| v / ppp);
    }
    #[allow(unreachable_code)]
    {
        let _ = (ctx, full, &INSETS_PX);
        [0.0; 4]
    }
}

// ------------------------------------------------- system back gesture

static GESTURE_EDGE: Mutex<Option<(Instant, f32)>> = Mutex::new(None);
static EXCL_WANT: Mutex<Option<[i32; 4]>> = Mutex::new(None);
static EXCL_SET: Mutex<Option<Option<[i32; 4]>>> = Mutex::new(None);

/// Width (points) of the left screen-edge strip the system uses for its
/// back gesture; 0 with button navigation. Simulator: `sim_gesture`.
pub fn gesture_edge_left(ctx: &egui::Context) -> f32 {
    if simulator() {
        return m().sim_gesture;
    }
    #[cfg(target_os = "android")]
    {
        let mut g = GESTURE_EDGE.lock().unwrap();
        if g.as_ref().is_none_or(|(at, _)| at.elapsed().as_millis() > 1000) {
            let px = crate::platform::android::gesture_edges_px().map(|v| v[0]).unwrap_or(0.0);
            *g = Some((Instant::now(), px));
        }
        return g.as_ref().map(|(_, v)| *v).unwrap_or(0.0) / ctx.pixels_per_point().max(0.5);
    }
    #[allow(unreachable_code)]
    {
        let _ = (ctx, &GESTURE_EDGE);
        0.0
    }
}

/// This frame wants `r` (points) kept free of the system back gesture.
pub fn exclude_from_back_gesture(ctx: &egui::Context, r: Rect) {
    let ppp = ctx.pixels_per_point();
    let px = [(r.left() * ppp) as i32, (r.top() * ppp) as i32, (r.width() * ppp).ceil() as i32, (r.height() * ppp).ceil() as i32];
    *EXCL_WANT.lock().unwrap() = Some(px);
}

/// Sends last frame's exclusion wish to Android when it changed, and starts
/// a new frame with none.
pub fn apply_back_gesture_exclusion() {
    let want = EXCL_WANT.lock().unwrap().take();
    let mut set = EXCL_SET.lock().unwrap();
    if *set != Some(want) {
        *set = Some(want);
        #[cfg(target_os = "android")]
        crate::platform::android::set_gesture_exclusion(want);
    }
}

/// Paints fake status/navigation bars around the app in the simulator.
pub fn paint_simulator_chrome(ctx: &egui::Context, full: Rect, inner: Rect, t: &crate::theme::Theme) {
    if !simulator() {
        return;
    }
    let mut p = ctx.layer_painter(egui::LayerId::new(egui::Order::Debug, egui::Id::new("sim_chrome")));
    p.set_clip_rect(Rect::EVERYTHING);
    let bar = egui::Color32::from_rgb(0x08, 0x08, 0x08);
    let top = Rect::from_min_max(full.min, egui::pos2(full.right(), inner.top()));
    let bottom = Rect::from_min_max(egui::pos2(full.left(), inner.bottom()), full.max);
    let left = Rect::from_min_max(egui::pos2(full.left(), inner.top()), egui::pos2(inner.left(), inner.bottom()));
    for r in [top, bottom, left] {
        if r.width() > 0.0 && r.height() > 0.0 {
            p.rect_filled(r, 0.0, bar);
        }
    }
    let small = egui::FontId::proportional(12.0);
    p.text(egui::pos2(top.left() + 18.0 + left.width(), top.center().y), egui::Align2::LEFT_CENTER, "12:00", small.clone(), t.text_dim);
    p.text(egui::pos2(top.right() - 18.0, top.center().y), egui::Align2::RIGHT_CENTER, "▮ 86%", small, t.text_dim);
    if bottom.height() > 4.0 {
        p.rect_filled(Rect::from_center_size(bottom.center(), vec2(110.0, 4.0)), 2.0, t.text_faint);
    }
    if left.width() > 0.0 {
        p.circle_filled(egui::pos2(left.center().x, inner.center().y), 6.0, egui::Color32::BLACK);
    }
}
