//! Velour Setup: a small installer/uninstaller in the app's own look.
//!
//! The same exe is both: with an attached payload (see `payload.rs`) it
//! installs; the bare stub copied to `uninstall.exe` uninstalls.
//!
//! Flags: `--quiet` (no window; install with defaults / uninstall keeping data),
//! `--relaunch` (with `--quiet`, from Velour's updater: wait for Velour to
//! quit, then start it again).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[path = "../../src/logo.rs"]
#[allow(dead_code)]
mod logo;
mod install;
#[allow(dead_code)]
mod payload;

use eframe::egui::{
    self, Align2, Color32, ColorImage, CornerRadius, CursorIcon, FontFamily, FontId, Mesh, Pos2, Rect, Response, Sense, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Ui, pos2, vec2,
};
use egui_phosphor::regular as ph;
use install::{Existing, Options, Progress};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};

pub fn en() -> bool {
    static EN: OnceLock<bool> = OnceLock::new();
    *EN.get_or_init(|| !sys_locale::get_locale().is_some_and(|l| l.to_lowercase().starts_with("de")))
}

/// German / English text.
pub fn tx(de: &'static str, en_text: &'static str) -> &'static str {
    if en() { en_text } else { de }
}

const ACCENT: Color32 = Color32::from_rgb(0xD8, 0x3F, 0x5C);
const BG: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);
const ELEVATED: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x1f);
const HOVER: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x2e);
const BORDER: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x2c);
const TEXT: Color32 = Color32::from_rgb(0xe8, 0xe8, 0xe8);
const TEXT_DIM: Color32 = Color32::from_rgb(0x9a, 0x9a, 0x9a);
const TEXT_FAINT: Color32 = Color32::from_rgb(0x66, 0x66, 0x66);
const TRACK: Color32 = Color32::from_rgb(0x0b, 0x0b, 0x0b);
const DANGER: Color32 = Color32::from_rgb(0xe5, 0x48, 0x48);
const HERO_W: f32 = 270.0;

enum Mode {
    Install { exe: PathBuf, footer: payload::Footer, existing: Option<Existing> },
    Uninstall,
}

#[derive(Clone, PartialEq)]
enum Page {
    Start,
    Working,
    Done,
    Running,
    Failed(String),
}

struct Setup {
    mode: Mode,
    page: Page,
    dir: PathBuf,
    opts: Options,
    purge: bool,
    progress: Arc<Progress>,
    result: Arc<Mutex<Option<Result<(), String>>>>,
    shown: f32,
    logo: Option<TextureHandle>,
    version: String,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let quiet = args.iter().any(|a| a == "--quiet" || a.eq_ignore_ascii_case("/S"));
    // In-app update: wait for Velour to quit, start it again afterwards.
    let relaunch = args.iter().any(|a| a == "--relaunch");
    let arg_dir = args.iter().position(|a| a == "--uninstall-dir").and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let exe = std::env::current_exe().expect("current exe");
    let footer = payload::read_footer(&exe).ok().flatten();

    let (mode, dir) = match footer {
        Some(footer) => {
            let existing = install::existing().filter(|e| e.dir.is_dir());
            let dir = existing.as_ref().map(|e| e.dir.clone()).unwrap_or_else(install::default_dir);
            (Mode::Install { exe, footer, existing }, dir)
        }
        None => match arg_dir {
            Some(d) => (Mode::Uninstall, d),
            None => {
                // Started as <install dir>\uninstall.exe: continue from a temp copy.
                let dir = exe.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                if let Err(e) = install::relaunch_from_temp(&dir, quiet) {
                    rfd::MessageDialog::new().set_title("Velour").set_description(e.to_string()).show();
                }
                return;
            }
        },
    };

    if quiet {
        let progress = Arc::new(Progress::default());
        match &mode {
            Mode::Install { exe, footer, existing } => {
                let opts = existing.as_ref().map(|e| e.options).unwrap_or(Options { desktop: false, open_with: true });
                if relaunch {
                    // Started by Velour's updater, which quits right after:
                    // give it time to save and exit on its own.
                    for _ in 0..50 {
                        if !install::app_running(&dir) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                }
                if install::app_running(&dir) {
                    install::close_app();
                }
                let ok = install::install(exe, footer, &dir, opts, progress).is_ok();
                if relaunch {
                    // Also after a failed install: the old version is still there.
                    let _ = std::process::Command::new(dir.join(install::APP_EXE)).current_dir(&dir).spawn();
                }
                std::process::exit(if ok { 0 } else { 1 });
            }
            Mode::Uninstall => {
                if install::app_running(&dir) {
                    install::close_app();
                }
                let ok = install::uninstall(&dir, false, progress).is_ok();
                install::delete_self_later();
                std::process::exit(if ok { 0 } else { 1 });
            }
        }
    }

    let opts = match &mode {
        Mode::Install { existing: Some(e), .. } => e.options,
        _ => Options { desktop: false, open_with: true },
    };
    let icon = egui::IconData { rgba: logo::logo_rasterize(64), width: 64, height: 64 };
    let title = match mode {
        Mode::Install { .. } => tx("Velour installieren", "Install Velour"),
        Mode::Uninstall => tx("Velour deinstallieren", "Uninstall Velour"),
    };
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([780.0, 470.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_decorations(false)
            .with_icon(icon),
        centered: true,
        ..Default::default()
    };
    let is_uninstall = matches!(mode, Mode::Uninstall);
    let version = match &mode {
        Mode::Install { footer, .. } => footer.version.clone(),
        Mode::Uninstall => install::existing().map(|e| e.version).unwrap_or_default(),
    };
    let app = Setup {
        version,
        mode,
        page: Page::Start,
        dir,
        opts,
        purge: false,
        progress: Arc::new(Progress::default()),
        result: Arc::new(Mutex::new(None)),
        shown: 0.0,
        logo: None,
    };
    let _ = eframe::run_native(
        title,
        options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
    if is_uninstall {
        install::delete_self_later();
    }
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let read = |name: &str| std::fs::read(format!("C:\\Windows\\Fonts\\{name}")).ok();
    let mut prop = Vec::new();
    if let Some(b) = read("segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), egui::FontData::from_owned(b).into());
        prop.push("segoe".to_string());
    }
    fonts.font_data.insert("phosphor".into(), egui_phosphor::Variant::Regular.font_data().into());
    fonts.font_data.insert("phosphor-fill".into(), egui_phosphor::Variant::Fill.font_data().into());
    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    prop.push("phosphor".into());
    prop.extend(defaults.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, prop.clone());
    let mut sb = Vec::new();
    if let Some(b) = read("seguisb.ttf") {
        fonts.font_data.insert("segoe-sb".into(), egui::FontData::from_owned(b).into());
        sb.push("segoe-sb".to_string());
    }
    sb.extend(prop.iter().cloned());
    fonts.families.insert(FontFamily::Name("sb".into()), sb);
    let mut fill = vec!["phosphor-fill".to_string()];
    fill.extend(prop);
    fonts.families.insert(FontFamily::Name("fill".into()), fill);
    ctx.set_fonts(fonts);
    ctx.style_mut(|s| {
        s.visuals = egui::Visuals::dark();
        s.visuals.panel_fill = BG;
        s.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.5);
        s.interaction.selectable_labels = false;
    });
}

fn sb(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("sb".into()))
}

fn fill_icon(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("fill".into()))
}

impl eframe::App for Setup {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.page == Page::Working {
            if let Some(r) = self.result.lock().unwrap().take() {
                self.page = match r {
                    Ok(()) => Page::Done,
                    Err(e) => Page::Failed(e),
                };
            }
            ctx.request_repaint();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.page != Page::Working {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(BG)).show(ctx, |ui| {
            let full = ui.max_rect();
            self.hero(ui, Rect::from_min_max(full.min, pos2(full.left() + HERO_W, full.bottom())));
            let right = Rect::from_min_max(pos2(full.left() + HERO_W, full.top()), full.max);
            self.title_bar(ui, right);
            let content = Rect::from_min_max(right.min + vec2(40.0, 58.0), right.max - vec2(40.0, 30.0));
            self.content(ui, content);
            ui.painter().rect_stroke(full, CornerRadius::ZERO, Stroke::new(1.0, BORDER), StrokeKind::Inside);
        });
    }
}

impl Setup {
    /// Left panel: velvet curtain with the V mark.
    fn hero(&mut self, ui: &mut Ui, r: Rect) {
        let p = ui.painter();
        let top = Color32::from_rgb(0x4a, 0x12, 0x22);
        let bottom = Color32::from_rgb(0x14, 0x08, 0x0d);
        let mut m = Mesh::default();
        m.colored_vertex(r.left_top(), top);
        m.colored_vertex(r.right_top(), top);
        m.colored_vertex(r.right_bottom(), bottom);
        m.colored_vertex(r.left_bottom(), bottom);
        m.add_triangle(0, 1, 2);
        m.add_triangle(0, 2, 3);
        p.add(egui::Shape::mesh(m));
        // Folds: soft vertical highlights and shadows like hanging velvet.
        let folds = 7;
        let w = r.width() / folds as f32;
        for i in 0..folds {
            let x0 = r.left() + i as f32 * w;
            let mid = x0 + w * (0.45 + 0.1 * ((i * 7 % 5) as f32 / 4.0));
            let shade = Color32::from_black_alpha(70);
            let light = Color32::from_white_alpha(10);
            for (a, b, ca, cb) in [(x0, mid, shade, light), (mid, x0 + w, light, shade)] {
                let mut m = Mesh::default();
                m.colored_vertex(pos2(a, r.top()), ca);
                m.colored_vertex(pos2(b, r.top()), cb);
                m.colored_vertex(pos2(b, r.bottom()), cb);
                m.colored_vertex(pos2(a, r.bottom()), ca);
                m.add_triangle(0, 1, 2);
                m.add_triangle(0, 2, 3);
                p.add(egui::Shape::mesh(m));
            }
        }
        // Darker floor so the text sits on a calm background.
        let mut m = Mesh::default();
        let fade = Rect::from_min_max(pos2(r.left(), r.bottom() - 170.0), r.max);
        m.colored_vertex(fade.left_top(), Color32::TRANSPARENT);
        m.colored_vertex(fade.right_top(), Color32::TRANSPARENT);
        m.colored_vertex(fade.right_bottom(), Color32::from_black_alpha(150));
        m.colored_vertex(fade.left_bottom(), Color32::from_black_alpha(150));
        m.add_triangle(0, 1, 2);
        m.add_triangle(0, 2, 3);
        p.add(egui::Shape::mesh(m));

        let tex = self.logo.get_or_insert_with(|| {
            let n = 256;
            let img = ColorImage::from_rgba_unmultiplied([n, n], &logo::logo_mask(n));
            ui.ctx().load_texture("velour-logo", img, TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear)))
        });
        let c = pos2(r.center().x, r.top() + r.height() * 0.40);
        let size = 116.0;
        let logo_rect = Rect::from_center_size(c, vec2(size, size));
        // Soft glow behind the mark.
        let mut glow = Mesh::default();
        glow.colored_vertex(c, Color32::from_rgba_unmultiplied(0xD8, 0x3F, 0x5C, 46));
        let segs = 64;
        for i in 0..=segs {
            let a = i as f32 / segs as f32 * std::f32::consts::TAU;
            glow.colored_vertex(c + vec2(a.cos(), a.sin()) * size * 1.05, Color32::TRANSPARENT);
            if i > 0 {
                glow.add_triangle(0, i, i + 1);
            }
        }
        p.add(egui::Shape::mesh(glow));
        p.image(tex.id(), logo_rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), ACCENT);
        p.text(pos2(c.x, logo_rect.bottom() + 30.0), Align2::CENTER_CENTER, "Velour", sb(30.0), Color32::WHITE);
        let v = self.version.clone();
        if !v.is_empty() {
            p.text(pos2(c.x, logo_rect.bottom() + 58.0), Align2::CENTER_CENTER, format!("Version {v}"), FontId::proportional(12.5), Color32::from_white_alpha(120));
        }
        p.text(
            pos2(c.x, r.bottom() - 26.0),
            Align2::CENTER_CENTER,
            tx("Video · Musik · Plex", "Video · Music · Plex"),
            FontId::proportional(12.0),
            Color32::from_white_alpha(90),
        );
    }

    fn title_bar(&mut self, ui: &mut Ui, right: Rect) {
        let bar = Rect::from_min_max(right.min, pos2(right.right(), right.top() + 40.0));
        let drag = ui.interact(bar, ui.id().with("drag"), Sense::click_and_drag());
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let close = Rect::from_min_size(pos2(bar.right() - 46.0, bar.top()), vec2(46.0, 40.0));
        let min = Rect::from_min_size(pos2(close.left() - 46.0, bar.top()), vec2(46.0, 40.0));
        let working = self.page == Page::Working;
        let rc = ui.interact(close, ui.id().with("close"), if working { Sense::hover() } else { Sense::click() });
        if rc.hovered() && !working {
            ui.painter().rect_filled(close, CornerRadius::ZERO, Color32::from_rgb(0xc4, 0x2b, 0x1c));
        }
        ui.painter().text(close.center(), Align2::CENTER_CENTER, ph::X, FontId::proportional(15.0), if working { TEXT_FAINT } else { TEXT_DIM });
        if rc.clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let rm = ui.interact(min, ui.id().with("min"), Sense::click());
        if rm.hovered() {
            ui.painter().rect_filled(min, CornerRadius::ZERO, HOVER);
        }
        ui.painter().text(min.center(), Align2::CENTER_CENTER, ph::MINUS, FontId::proportional(15.0), TEXT_DIM);
        if rm.clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
    }

    fn content(&mut self, ui: &mut Ui, r: Rect) {
        let footer = Rect::from_min_max(pos2(r.left(), r.bottom() - 40.0), r.max);
        let body = Rect::from_min_max(r.min, pos2(r.right(), footer.top() - 12.0));
        let page = self.page.clone();
        let uninstall = matches!(self.mode, Mode::Uninstall);
        match page {
            Page::Start if uninstall => self.page_uninstall(ui, body, footer),
            Page::Start => self.page_options(ui, body, footer),
            Page::Working => self.page_working(ui, body),
            Page::Done => self.page_done(ui, body, footer),
            Page::Running => self.page_running(ui, body, footer),
            Page::Failed(e) => self.page_failed(ui, body, footer, &e),
        }
    }

    fn heading(ui: &mut Ui, r: Rect, title: &str, text: &str) -> f32 {
        let p = ui.painter();
        p.text(r.left_top(), Align2::LEFT_TOP, title, sb(25.0), TEXT);
        let galley = p.layout(text.to_string(), FontId::proportional(14.0), TEXT_DIM, r.width());
        let h = galley.size().y;
        p.galley(pos2(r.left(), r.top() + 42.0), galley, TEXT_DIM);
        r.top() + 42.0 + h
    }

    fn page_options(&mut self, ui: &mut Ui, body: Rect, footer: Rect) {
        let Mode::Install { footer: pack, existing, .. } = &self.mode else { return };
        let (title, text) = match existing {
            Some(e) if e.version != pack.version => (
                tx("Velour aktualisieren", "Update Velour"),
                if en() {
                    format!("Version {} → {}. Your settings, library and Plex sign-in stay as they are.", e.version, pack.version)
                } else {
                    format!("Version {} → {}. Einstellungen, Bibliothek und Plex-Anmeldung bleiben erhalten.", e.version, pack.version)
                },
            ),
            Some(_) => (
                tx("Velour reparieren", "Repair Velour"),
                tx("Diese Version ist bereits installiert. Die Programmdateien werden neu geschrieben, deine Daten bleiben erhalten.", "This version is already installed. The program files are written again, your data stays as it is.").to_string(),
            ),
            None => (
                tx("Velour installieren", "Install Velour"),
                tx(
                    "Video- und Audioplayer mit Bibliothek, Untertitel-Werkzeugen und Plex. Wiedergabe-Engine und FFmpeg sind enthalten.",
                    "Video and audio player with library, subtitle tools and Plex. Playback engine and FFmpeg are included.",
                )
                .to_string(),
            ),
        };
        let locked = existing.is_some();
        let unpacked = pack.unpacked;
        let mut y = Self::heading(ui, body, title, &text) + 26.0;

        // Location
        ui.painter().text(pos2(body.left(), y), Align2::LEFT_TOP, tx("SPEICHERORT", "LOCATION"), FontId::proportional(11.0), TEXT_FAINT);
        y += 20.0;
        let row = Rect::from_min_size(pos2(body.left(), y), vec2(body.width(), 38.0));
        let btn_w = if locked { 0.0 } else { 96.0 };
        let field = Rect::from_min_max(row.min, pos2(row.right() - btn_w - if locked { 0.0 } else { 8.0 }, row.bottom()));
        ui.painter().rect_filled(field, CornerRadius::same(6), ELEVATED);
        ui.painter().text(pos2(field.left() + 14.0, field.center().y), Align2::LEFT_CENTER, ph::FOLDER_SIMPLE, FontId::proportional(16.0), TEXT_DIM);
        let path = self.dir.display().to_string();
        let shown = elide_middle(ui, &path, FontId::proportional(13.5), field.width() - 52.0);
        ui.painter().text(pos2(field.left() + 38.0, field.center().y), Align2::LEFT_CENTER, shown, FontId::proportional(13.5), TEXT);
        if !locked {
            let b = Rect::from_min_max(pos2(row.right() - btn_w, row.top()), row.max);
            if button(ui, b, tx("Ändern…", "Change…"), Kind::Ghost).clicked() {
                let start = self.dir.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                if let Some(mut d) = rfd::FileDialog::new().set_directory(start).set_title(tx("Speicherort wählen", "Choose location")).pick_folder() {
                    if d.file_name().is_none_or(|n| !n.eq_ignore_ascii_case("Velour")) {
                        d = d.join("Velour");
                    }
                    self.dir = d;
                }
            }
        }
        y = row.bottom() + 22.0;

        // Options
        y = toggle_row(
            ui,
            pos2(body.left(), y),
            body.width(),
            &mut self.opts.desktop,
            tx("Verknüpfung auf dem Desktop", "Desktop shortcut"),
            tx("Im Startmenü ist Velour immer zu finden.", "Velour is always in the Start menu."),
        );
        toggle_row(
            ui,
            pos2(body.left(), y + 6.0),
            body.width(),
            &mut self.opts.open_with,
            tx("Bei „Öffnen mit“ anbieten", "Offer in “Open with”"),
            tx(
                "Für Videos, Musik, Playlists und Ordner. Velour wird nicht ungefragt zur Standard-App.",
                "For videos, music, playlists and folders. Velour never makes itself the default app.",
            ),
        );

        // Footer
        let info = if en() {
            format!("{:.0} MB · no admin rights", unpacked as f64 / 1e6)
        } else {
            format!("{:.0} MB · ohne Adminrechte", unpacked as f64 / 1e6)
        };
        ui.painter().text(pos2(footer.left(), footer.center().y), Align2::LEFT_CENTER, info, FontId::proportional(12.0), TEXT_FAINT);
        let label = match existing {
            Some(e) if e.version != pack.version => tx("Aktualisieren", "Update"),
            Some(_) => tx("Reparieren", "Repair"),
            None => tx("Installieren", "Install"),
        };
        let (cancel, go) = footer_buttons(footer, 110.0, 150.0);
        if button(ui, cancel, tx("Abbrechen", "Cancel"), Kind::Ghost).clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if button(ui, go, label, Kind::Accent).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.start(ui.ctx());
        }
    }

    fn page_uninstall(&mut self, ui: &mut Ui, body: Rect, footer: Rect) {
        let text = if en() {
            format!("Removes Velour from {}. Your media files are never touched; you decide about your settings at the end.", self.dir.display())
        } else {
            format!("Entfernt Velour aus {}. Deine Mediendateien bleiben unberührt; über deine Einstellungen entscheidest du am Ende.", self.dir.display())
        };
        Self::heading(ui, body, tx("Velour deinstallieren", "Uninstall Velour"), &text);
        let (cancel, go) = footer_buttons(footer, 110.0, 160.0);
        if button(ui, cancel, tx("Abbrechen", "Cancel"), Kind::Ghost).clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if button(ui, go, tx("Deinstallieren", "Uninstall"), Kind::Danger).clicked() {
            self.start(ui.ctx());
        }
    }

    fn page_working(&mut self, ui: &mut Ui, body: Rect) {
        let uninstall = matches!(self.mode, Mode::Uninstall);
        let title = if uninstall { tx("Velour wird entfernt…", "Removing Velour…") } else { tx("Velour wird installiert…", "Installing Velour…") };
        Self::heading(ui, body, title, tx("Das dauert nur einen Moment.", "This only takes a moment."));
        let total = self.progress.total.load(Ordering::Relaxed).max(1) as f32;
        let done = self.progress.done.load(Ordering::Relaxed) as f32;
        let target = (done / total).clamp(0.0, 1.0);
        let dt = ui.input(|i| i.stable_dt).min(0.05);
        self.shown += (target - self.shown) * (1.0 - (-dt * 10.0).exp());
        let bar = Rect::from_min_size(pos2(body.left(), body.top() + 130.0), vec2(body.width(), 10.0));
        let p = ui.painter();
        p.rect_filled(bar, CornerRadius::same(5), TRACK);
        let fill = Rect::from_min_size(bar.min, vec2((bar.width() * self.shown).max(10.0), bar.height()));
        p.rect_filled(fill.expand(3.0), CornerRadius::same(8), ACCENT.gamma_multiply(0.18));
        p.rect_filled(fill, CornerRadius::same(5), ACCENT);
        p.text(pos2(bar.right(), bar.bottom() + 14.0), Align2::RIGHT_TOP, format!("{:.0} %", self.shown * 100.0), sb(13.0), TEXT);
        let cur = self.progress.current.lock().unwrap().clone();
        p.text(pos2(bar.left(), bar.bottom() + 14.0), Align2::LEFT_TOP, cur, FontId::proportional(12.5), TEXT_FAINT);
    }

    fn page_done(&mut self, ui: &mut Ui, body: Rect, footer: Rect) {
        let uninstall = matches!(self.mode, Mode::Uninstall);
        let c = pos2(body.left() + 26.0, body.top() + 26.0);
        ui.painter().circle_filled(c, 26.0, ACCENT.gamma_multiply(0.18));
        ui.painter().text(c, Align2::CENTER_CENTER, ph::CHECK_CIRCLE, fill_icon(34.0), ACCENT);
        let r = Rect::from_min_max(pos2(body.left(), body.top() + 74.0), body.max);
        if uninstall {
            if self.purge {
                Self::heading(
                    ui,
                    r,
                    tx("Alles entfernt", "Everything removed"),
                    tx("Velour und deine Daten wurden gelöscht. Deine Mediendateien sind unberührt.", "Velour and your data were deleted. Your media files are untouched."),
                );
                let (_, close) = footer_buttons(footer, 0.0, 130.0);
                if button(ui, close, tx("Schließen", "Close"), Kind::Accent).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                return;
            }
            Self::heading(
                ui,
                r,
                tx("Velour wurde entfernt", "Velour has been removed"),
                tx(
                    "Möchtest du deine Einstellungen behalten? Dazu gehören Bibliothek, Verlauf, Presets, Vorschaubilder und die Plex-Anmeldung. Installierst du Velour später wieder, ist alles wie vorher.",
                    "Do you want to keep your settings? That includes library, history, presets, thumbnails and the Plex sign-in. If you install Velour again later, everything is as before.",
                ),
            );
            let (delete, keep) = footer_buttons(footer, 150.0, 130.0);
            if button(ui, delete, tx("Daten löschen", "Delete data"), Kind::Ghost).clicked() {
                install::purge_data();
                self.purge = true;
            }
            if button(ui, keep, tx("Behalten", "Keep"), Kind::Accent).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        Self::heading(
            ui,
            r,
            tx("Velour ist bereit", "Velour is ready"),
            tx("Du findest Velour ab jetzt im Startmenü. Viel Spaß beim Schauen!", "You'll find Velour in the Start menu from now on. Enjoy watching!"),
        );
        let (close, launch) = footer_buttons(footer, 110.0, 160.0);
        if button(ui, close, tx("Schließen", "Close"), Kind::Ghost).clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if button(ui, launch, tx("Velour starten", "Launch Velour"), Kind::Accent).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let _ = std::process::Command::new(self.dir.join(install::APP_EXE)).current_dir(&self.dir).spawn();
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn page_running(&mut self, ui: &mut Ui, body: Rect, footer: Rect) {
        let uninstall = matches!(self.mode, Mode::Uninstall);
        Self::heading(
            ui,
            body,
            tx("Velour läuft noch", "Velour is still running"),
            if uninstall {
                tx("Schließe Velour, damit es entfernt werden kann.", "Close Velour so it can be removed.")
            } else {
                tx("Schließe Velour, damit es aktualisiert werden kann. Eine laufende Wiedergabe wird beendet.", "Close Velour so it can be updated. Playback in progress will stop.")
            },
        );
        let (retry, close) = footer_buttons(footer, 140.0, 160.0);
        if button(ui, retry, tx("Erneut prüfen", "Check again"), Kind::Ghost).clicked() {
            self.start(ui.ctx());
        }
        if button(ui, close, tx("Velour beenden", "Close Velour"), Kind::Accent).clicked() {
            install::close_app();
            self.start(ui.ctx());
        }
    }

    fn page_failed(&mut self, ui: &mut Ui, body: Rect, footer: Rect, err: &str) {
        let c = pos2(body.left() + 26.0, body.top() + 26.0);
        ui.painter().circle_filled(c, 26.0, DANGER.gamma_multiply(0.16));
        ui.painter().text(c, Align2::CENTER_CENTER, ph::WARNING_CIRCLE, fill_icon(34.0), DANGER);
        let r = Rect::from_min_max(pos2(body.left(), body.top() + 74.0), body.max);
        let msg = if en() { format!("Something went wrong: {err}") } else { format!("Etwas ist schiefgelaufen: {err}") };
        Self::heading(ui, r, tx("Das hat nicht geklappt", "That didn't work"), &msg);
        let (close, retry) = footer_buttons(footer, 110.0, 160.0);
        if button(ui, close, tx("Schließen", "Close"), Kind::Ghost).clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if button(ui, retry, tx("Erneut versuchen", "Try again"), Kind::Accent).clicked() {
            self.start(ui.ctx());
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        if install::app_running(&self.dir) {
            self.page = Page::Running;
            return;
        }
        self.page = Page::Working;
        self.shown = 0.0;
        self.progress = Arc::new(Progress::default());
        let progress = self.progress.clone();
        let result = self.result.clone();
        let dir = self.dir.clone();
        let ctx = ctx.clone();
        match &self.mode {
            Mode::Install { exe, footer, .. } => {
                let (exe, footer, opts) = (exe.clone(), footer.clone(), self.opts);
                std::thread::spawn(move || {
                    let r = install::install(&exe, &footer, &dir, opts, progress).map_err(|e| e.to_string());
                    *result.lock().unwrap() = Some(r);
                    ctx.request_repaint();
                });
            }
            Mode::Uninstall => {
                std::thread::spawn(move || {
                    // Data stays; the last page asks whether to delete it.
                    let r = install::uninstall(&dir, false, progress).map_err(|e| e.to_string());
                    *result.lock().unwrap() = Some(r);
                    ctx.request_repaint();
                });
            }
        }
    }
}

/// Secondary/primary button rects, right-aligned in the footer.
fn footer_buttons(footer: Rect, w1: f32, w2: f32) -> (Rect, Rect) {
    let h = 38.0;
    let y = footer.center().y - h / 2.0;
    let go = Rect::from_min_size(pos2(footer.right() - w2, y), vec2(w2, h));
    let cancel = Rect::from_min_size(pos2(go.left() - 10.0 - w1, y), vec2(w1, h));
    (cancel, go)
}

#[derive(Clone, Copy)]
enum Kind {
    Accent,
    Ghost,
    Danger,
}

fn button(ui: &mut Ui, r: Rect, label: &str, kind: Kind) -> Response {
    let resp = ui.interact(r, ui.id().with(("btn", label)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    let hot = resp.hovered();
    let (bg, fg) = match kind {
        Kind::Accent => (if hot { ACCENT.linear_multiply(1.12) } else { ACCENT }, Color32::WHITE),
        Kind::Danger => (if hot { DANGER.linear_multiply(1.12) } else { DANGER }, Color32::WHITE),
        Kind::Ghost => (if hot { HOVER } else { ELEVATED }, TEXT),
    };
    let p = ui.painter();
    if matches!(kind, Kind::Accent | Kind::Danger) && hot {
        p.rect_filled(r.expand(3.0), CornerRadius::same(10), bg.gamma_multiply(0.18));
    }
    p.rect_filled(r, CornerRadius::same(7), bg);
    p.text(r.center(), Align2::CENTER_CENTER, label, sb(14.0), fg);
    resp
}

/// Toggle switch with a title and a hint below; the whole row is clickable.
fn toggle_row(ui: &mut Ui, at: Pos2, width: f32, on: &mut bool, title: &str, hint: &str) -> f32 {
    let p = ui.painter().clone();
    let hint_galley = p.layout(hint.to_string(), FontId::proportional(12.5), TEXT_FAINT, width - 60.0);
    let h = 22.0 + hint_galley.size().y + 12.0;
    let row = Rect::from_min_size(at, vec2(width, h));
    let resp = ui.interact(row, ui.id().with(("toggle", title)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    if resp.clicked() {
        *on = !*on;
    }
    let k = ui.ctx().animate_bool_with_time(resp.id, *on, 0.12);
    let sw = Rect::from_min_size(pos2(at.x, at.y + 2.0), vec2(38.0, 22.0));
    let bg = if *on { ACCENT } else if resp.hovered() { Color32::from_rgb(0x3a, 0x3a, 0x3a) } else { HOVER };
    p.rect_filled(sw, CornerRadius::same(11), bg);
    let x = egui::lerp(sw.left() + 11.0..=sw.right() - 11.0, k);
    p.circle_filled(pos2(x, sw.center().y), 8.0, if *on { Color32::WHITE } else { TEXT_DIM });
    p.text(pos2(at.x + 54.0, at.y + 2.0), Align2::LEFT_TOP, title, FontId::proportional(14.5), TEXT);
    p.galley(pos2(at.x + 54.0, at.y + 24.0), hint_galley, TEXT_FAINT);
    row.bottom()
}

/// Shortens a long path in the middle: C:\Users\…\Programs\Velour
fn elide_middle(ui: &Ui, s: &str, font: FontId, max: f32) -> String {
    let fits = |t: &str| ui.painter().layout_no_wrap(t.to_string(), font.clone(), TEXT).size().x <= max;
    if fits(s) {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut keep = chars.len();
    while keep > 8 {
        keep -= 2;
        let head: String = chars[..keep / 3].iter().collect();
        let tail: String = chars[chars.len() - (keep - keep / 3)..].iter().collect();
        let t = format!("{head}…{tail}");
        if fits(&t) {
            return t;
        }
    }
    s.to_string()
}
