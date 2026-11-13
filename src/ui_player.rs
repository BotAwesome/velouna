//! Main layout: title bar, control bar, seek bar, video area, popups, OSD.

use crate::app::{App, Dialog, RightPanel, TrackTab, View};
use crate::config::{ControlItem, FullscreenControls, RepeatMode, WheelAction};
use crate::hotkeys::Action;
use crate::media;
use crate::theme;
use crate::widgets::{self, icon_button};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Layout, Order, Pos2, Rect, RectAlign, Sense,
    Stroke, StrokeKind, Ui, ViewportCommand, pos2, vec2,
};
use egui_phosphor::regular as ph;
use std::time::{Duration, Instant};

pub const TOP_H: f32 = 38.0;
pub const BOTTOM_H: f32 = 46.0;

impl App {
    pub fn shortcut(&self, a: Action) -> String {
        self.bindings.combo(a).map(|c| c.chips().join("+")).unwrap_or_default()
    }

    pub fn draw(&mut self, ctx: &egui::Context) {
        if crate::mobile::compact(ctx) {
            self.draw_mobile(ctx);
            return;
        }
        let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if fs != self.was_fullscreen {
            self.was_fullscreen = fs;
        }
        let screen = ctx.screen_rect();
        let has_media = self.has_media();
        // Leave the player only when mpv is idle, not while it loads the
        // next playlist entry.
        let active = self.player_active();
        if self.prev_has_media && !active && self.pip.is_none() {
            self.view = View::Library;
        }
        self.prev_has_media = active;
        let player_view = self.view == View::Player;
        let overlay = self.pip.is_none() && player_view && (fs || self.data.settings.autohide_windowed);

        // Bar visibility (hover near edges in overlay mode).
        let ptr = ctx.input(|i| i.pointer.hover_pos());
        let near = ptr.map(|p| p.y < TOP_H + 24.0 || p.y > screen.bottom() - BOTTOM_H - 70.0).unwrap_or(false);
        let popup_open = egui::Popup::is_any_open(ctx) || self.seek_drag.is_some();
        if near || popup_open || self.data.settings.fullscreen_controls == FullscreenControls::Always {
            self.bars_shown_at = Instant::now();
        }
        let hide_after = Duration::from_millis(if near { 0 } else { self.data.settings.hide_delay_ms.min(400) });
        let bars_visible = !overlay || self.bars_shown_at.elapsed() < hide_after || near || popup_open;
        if overlay && !bars_visible {
            ctx.request_repaint_after(Duration::from_millis(500));
        }

        let chrome = !self.ui_hidden && self.pip.is_none();
        if chrome && !overlay {
            egui::TopBottomPanel::top("top")
                .exact_height(TOP_H)
                .frame(egui::Frame::new().fill(self.t.panel))
                .show_separator_line(false)
                .show(ctx, |ui| self.top_bar(ui, fs));
            egui::TopBottomPanel::bottom("bottom")
                .exact_height(BOTTOM_H)
                .frame(egui::Frame::new().fill(self.t.panel))
                .show_separator_line(false)
                .show(ctx, |ui| self.bottom_bar(ui));
        }
        if chrome && self.lib.compact_open && player_view {
            egui::SidePanel::left("lib_compact")
                .resizable(true)
                .default_width(340.0)
                .width_range(260.0..=560.0)
                .frame(egui::Frame::new().fill(self.t.panel))
                .show_separator_line(false)
                .show(ctx, |ui| self.library_compact(ui));
        }
        if chrome && self.right != RightPanel::None {
            let w = self.data.settings.sidebar_width;
            let resp = egui::SidePanel::right("right")
                .resizable(true)
                .default_width(w)
                .width_range(280.0..=560.0)
                .frame(egui::Frame::new().fill(self.t.panel).inner_margin(egui::Margin::symmetric(12, 6)))
                .show_separator_line(false)
                .show(ctx, |ui| match self.right {
                    RightPanel::Adjust => self.adjust_panel(ui),
                    RightPanel::Queue => self.queue_panel(ui),
                    RightPanel::None => {}
                });
            let nw = resp.response.rect.width();
            if (nw - w).abs() > 2.0 {
                self.data.settings.sidebar_width = nw;
                self.dirty = true;
            }
        }

        let central_fill = if player_view || self.pip.is_some() { self.t.video_bg } else { self.t.bg };
        egui::CentralPanel::default().frame(egui::Frame::new().fill(central_fill)).show(ctx, |ui| {
            if player_view || self.pip.is_some() {
                self.video_area(ui, overlay, bars_visible);
            } else {
                self.library_full(ui);
            }
        });

        if chrome && overlay && bars_visible {
            let fill = self.t.panel.gamma_multiply(0.94);
            egui::Area::new(Id::new("ov_top")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
                egui::Frame::new().fill(fill).show(ui, |ui| {
                    ui.set_min_size(vec2(screen.width(), TOP_H));
                    ui.set_max_size(vec2(screen.width(), TOP_H));
                    self.top_bar(ui, fs);
                });
            });
            egui::Area::new(Id::new("ov_bottom"))
                .order(Order::Foreground)
                .fixed_pos(pos2(screen.left(), screen.bottom() - BOTTOM_H))
                .show(ctx, |ui| {
                    egui::Frame::new().fill(fill).show(ui, |ui| {
                        ui.set_min_size(vec2(screen.width(), BOTTOM_H));
                        ui.set_max_size(vec2(screen.width(), BOTTOM_H));
                        self.bottom_bar(ui);
                    });
                });
        }

        self.draw_resume_prompt(ctx, overlay && !bars_visible);
        self.draw_osd(ctx);
        self.draw_drop_overlay(ctx);
        self.draw_dialogs(ctx);
        self.window_resize_edges(ctx, fs);
    }

    // ------------------------------------------------------------ title bar

    fn top_bar(&mut self, ui: &mut Ui, fs: bool) {
        let t = self.t;
        let rect = ui.max_rect();
        let ctx = ui.ctx().clone();
        let bg = ui.interact(rect, Id::new("titlebar_drag"), Sense::click_and_drag());
        if bg.drag_started_by(egui::PointerButton::Primary) && !fs {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if bg.double_clicked() {
            let m = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!m));
        }
        ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.border));

        let mut left_end = rect.left();
        let mut right_start = rect.right();
        ui.allocate_ui_with_layout(rect.size(), Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(10.0);
            let (lr, lresp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
            widgets::draw_logo(ui.painter(), lr.shrink(2.0), t.accent);
            if lresp.on_hover_text("Start").on_hover_cursor(CursorIcon::PointingHand).clicked() {
                self.lib.go(crate::library::LibLoc::Home);
                self.view = View::Library;
            }
            ui.add_space(4.0);
            let mresp = icon_button(ui, &t, ph::DOTS_THREE, "", false);
            egui::Popup::menu(&mresp).width(280.0).show(|ui| self.main_menu(ui));
            let sb_active = self.lib.compact_open && self.view == View::Player;
            if icon_button(ui, &t, ph::SIDEBAR_SIMPLE, t!("Bibliothek (Seitenleiste)"), sb_active).clicked() {
                if self.view == View::Library && self.has_media() {
                    self.view = View::Player;
                    self.lib.compact_open = true;
                } else {
                    self.lib.compact_open = !self.lib.compact_open;
                }
            }
            ui.add_space(6.0);
            let (sr, _) = ui.allocate_exact_size(vec2(1.0, 18.0), Sense::hover());
            ui.painter().rect_filled(sr, CornerRadius::ZERO, t.border);
            ui.add_space(6.0);
            // Folder / show name – the "familiar thing" that opens the library.
            let folder = self.current_folder();
            let (fname, is_folder) = match &folder {
                Some(f) => (crate::library::name_of(f), true),
                None => (self.lib.title(&self.data), false),
            };
            let font = FontId::proportional(13.5);
            let galley = ui.painter().layout_no_wrap(fname.clone(), font, t.text_dim);
            let w = (galley.size().x + 34.0).min(260.0);
            let (fr, fresp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
            if fresp.hovered() {
                ui.painter().rect_filled(fr, CornerRadius::same(theme::R), t.hover);
            }
            let icon = if is_folder { ph::FOLDER } else { ph::BOOKS };
            ui.painter().text(pos2(fr.left() + 13.0, fr.center().y), Align2::CENTER_CENTER, icon, theme::icon(16.0), t.text_dim);
            widgets::painter_text_trunc(ui.painter(), pos2(fr.left() + 28.0, fr.center().y - 9.0), &fname, FontId::proportional(13.5), t.text_dim, w - 32.0);
            if fresp.on_hover_text(t!("In der Bibliothek anzeigen")).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                if let Some(f) = folder {
                    let in_videos = self.data.settings.video_folders.iter().any(|r| f.starts_with(r));
                    self.lib.go(if in_videos { crate::library::LibLoc::Videos(Some(f)) } else { crate::library::LibLoc::Folder(f) });
                }
                if self.has_media() {
                    self.view = View::Player;
                    self.lib.compact_open = !self.lib.compact_open || !is_folder;
                } else {
                    self.view = View::Library;
                }
            }
            left_end = ui.min_rect().right();
        });

        ui.allocate_ui_at_rect(rect, |ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_space(6.0);
                let close = {
                    let (r, resp) = ui.allocate_exact_size(vec2(34.0, 30.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(theme::R), Color32::from_rgb(0xc4, 0x2b, 0x1c));
                    }
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, ph::X, theme::icon(16.0), if resp.hovered() { Color32::WHITE } else { t.text_dim });
                    resp.on_hover_text(t!("Schließen"))
                };
                if close.clicked() {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
                let fs_icon = if fs { ph::CORNERS_IN } else { ph::CORNERS_OUT };
                if icon_button(ui, &t, fs_icon, t!("Vollbild"), false).clicked() {
                    self.toggle_fullscreen(&ctx);
                }
                let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                if !fs && icon_button(ui, &t, if maxed { ph::COPY } else { ph::SQUARE }, if maxed { t!("Wiederherstellen") } else { t!("Maximieren") }, false).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maxed));
                }
                if icon_button(ui, &t, ph::PICTURE_IN_PICTURE, t!("Bild-in-Bild"), self.pip.is_some()).clicked() {
                    self.toggle_pip(&ctx);
                }
                if !fs && icon_button(ui, &t, ph::MINUS, t!("Minimieren"), false).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                }
                right_start = ui.min_rect().left();
            });
        });

        // Centered title.
        let title = self
            .player
            .as_ref()
            .filter(|p| p.st.has_media())
            .map(|p| p.st.media_title.clone())
            .unwrap_or_default();
        if !title.is_empty() {
            let half = ((rect.center().x - left_end).min(right_start - rect.center().x) - 16.0).max(40.0);
            let font = FontId::proportional(12.5);
            let g = ui.painter().layout_no_wrap(title.clone(), font.clone(), t.text_dim);
            let w = g.size().x.min(half * 2.0);
            widgets::painter_text_trunc(ui.painter(), pos2(rect.center().x - w / 2.0, rect.center().y - g.size().y / 2.0), &title, font, t.text_dim, half * 2.0);
        }
    }

    fn main_menu(&mut self, ui: &mut Ui) {
        let t = self.t;
        let ctx = ui.ctx().clone();
        ui.spacing_mut().item_spacing.y = 0.0;
        if widgets::menu_item(ui, &t, ph::FILE_PLUS, t!("Datei öffnen…"), &self.shortcut(Action::OpenFile), false).clicked() {
            ui.close();
            self.dialog_open_files();
        }
        if widgets::menu_item(ui, &t, ph::FOLDER_OPEN, t!("Ordner öffnen…"), &self.shortcut(Action::OpenFolder), false).clicked() {
            ui.close();
            self.dialog_open_folder();
        }
        if widgets::menu_item(ui, &t, ph::BROADCAST, t!("Netzwerkstream öffnen…"), &self.shortcut(Action::OpenNetwork), false).clicked() {
            ui.close();
            self.dialog = Some(Dialog::Network);
        }
        if widgets::menu_item(ui, &t, ph::CLIPBOARD, t!("Aus Zwischenablage öffnen"), &self.shortcut(Action::OpenClipboard), false).clicked() {
            ui.close();
            self.open_clipboard();
        }
        ui.menu_button(tf!("{}   Zuletzt geöffnet", ph::CLOCK_COUNTER_CLOCKWISE), |ui| {
            let mut hist = self.data.history.clone();
            hist.sort_by(|a, b| b.last_played.cmp(&a.last_played));
            if hist.is_empty() {
                ui.label(egui::RichText::new(t!("Noch nichts abgespielt")).color(t.text_faint));
            }
            for h in hist.iter().take(12) {
                let icon = if h.audio_only { ph::MUSIC_NOTE } else if media::is_url(&h.path) { ph::BROADCAST } else { ph::FILE_VIDEO };
                if widgets::menu_item(ui, &t, icon, &h.title, &media::fmt_time_short(h.duration), false).clicked() {
                    ui.close();
                    self.open_paths(vec![h.path.clone()]);
                }
            }
        });
        widgets::separator(ui, &t);
        if widgets::menu_item(ui, &t, ph::SUBTITLES, t!("Untertitel hinzufügen…"), "", false).clicked() {
            ui.close();
            self.dialog = Some(Dialog::AddSubs);
        }
        if widgets::menu_item(ui, &t, ph::ARROWS_CLOCKWISE, t!("Konvertieren / Speichern…"), "", false).clicked() {
            ui.close();
            self.dlg.convert_in = self.current_path();
            self.dlg.convert_result = None;
            self.dialog = Some(Dialog::Convert);
        }
        let rec = self.player.as_ref().map(|p| !p.st.recording.is_empty()).unwrap_or(false);
        if widgets::menu_item(ui, &t, ph::RECORD, if rec { t!("Aufnahme stoppen") } else { t!("Stream aufnehmen") }, "", rec).clicked() {
            ui.close();
            self.toggle_record();
        }
        widgets::separator(ui, &t);
        if widgets::menu_item(ui, &t, ph::BOOKS, t!("Bibliothek"), &self.shortcut(Action::Library), false).clicked() {
            ui.close();
            self.view = View::Library;
        }
        if widgets::menu_item(ui, &t, ph::QUEUE, t!("Warteschlange"), &self.shortcut(Action::Queue), self.right == RightPanel::Queue).clicked() {
            ui.close();
            self.right = if self.right == RightPanel::Queue { RightPanel::None } else { RightPanel::Queue };
        }
        if widgets::menu_item(ui, &t, ph::SLIDERS_HORIZONTAL, t!("Anpassungen & Effekte"), &self.shortcut(Action::Sidebar), self.right == RightPanel::Adjust).clicked() {
            ui.close();
            self.right = if self.right == RightPanel::Adjust { RightPanel::None } else { RightPanel::Adjust };
        }
        widgets::separator(ui, &t);
        if widgets::menu_item(ui, &t, ph::KEYBOARD, t!("Tastenkürzel…"), &self.shortcut(Action::Hotkeys), false).clicked() {
            ui.close();
            self.dialog = Some(Dialog::Hotkeys);
        }
        if widgets::menu_item(ui, &t, ph::SQUARES_FOUR, t!("Oberfläche anpassen…"), "", false).clicked() {
            ui.close();
            self.dialog = Some(Dialog::Customize);
        }
        if widgets::menu_item(ui, &t, ph::GEAR, t!("Einstellungen…"), &self.shortcut(Action::Settings), false).clicked() {
            ui.close();
            self.dialog = Some(Dialog::Settings);
        }
        if widgets::menu_item(ui, &t, ph::INFO, t!("Über Velour"), "", false).clicked() {
            ui.close();
            self.dialog = Some(Dialog::About);
        }
        widgets::separator(ui, &t);
        if widgets::menu_item(ui, &t, ph::X, t!("Beenden"), &self.shortcut(Action::Quit), false).clicked() {
            ui.close();
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    pub fn toggle_record(&mut self) {
        let Some(p) = &self.player else { return };
        if !p.st.recording.is_empty() {
            let _ = p.mpv.set_str("stream-record", "");
            self.osd(t!("Aufnahme gestoppt"));
        } else if p.st.has_media() {
            let dir = self.data.settings.record_dir.clone();
            let _ = std::fs::create_dir_all(&dir);
            let name = format!("{}_{}.mkv", sanitize(&media::display_name(&p.st.path)), crate::config::now_secs());
            let path = dir.join(name);
            let _ = p.mpv.set_str("stream-record", &path.to_string_lossy());
            self.osd(tf!("Aufnahme läuft → {}", path.display()));
        }
    }

    // ---------------------------------------------------------- control bar

    fn item_width(&self, item: ControlItem) -> f32 {
        match item {
            ControlItem::Play => 44.0,
            ControlItem::Time | ControlItem::Duration => 64.0,
            ControlItem::Volume => 98.0,
            ControlItem::Speed => 50.0,
            _ => 30.0,
        }
    }

    fn bottom_bar(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        let layout = self.data.settings.layout.clone();
        let spacing = 4.0;
        ui.allocate_ui_at_rect(rect, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = spacing;
                ui.add_space(10.0);
                for item in &layout.left {
                    self.control_item(ui, *item);
                }
                ui.add_space(6.0);
                let right_w: f32 = layout.right.iter().map(|i| self.item_width(*i) + spacing).sum::<f32>() + 16.0;
                let seek_w = (ui.available_width() - right_w).max(60.0);
                self.seek_bar(ui, seek_w);
                ui.add_space(6.0);
                for item in &layout.right {
                    self.control_item(ui, *item);
                }
            });
        });
    }

    fn control_item(&mut self, ui: &mut Ui, item: ControlItem) {
        let t = self.t;
        let ctx = ui.ctx().clone();
        let Some(p) = &self.player else {
            ui.add_space(self.item_width(item));
            return;
        };
        let st = &p.st;
        let has = st.has_media();
        match item {
            ControlItem::Prev => {
                if widgets::icon_button_sized(ui, &t, ph::SKIP_BACK, t!("Vorheriger Titel"), false, 30.0, 16.0).clicked() {
                    p.prev();
                }
            }
            ControlItem::Next => {
                if widgets::icon_button_sized(ui, &t, ph::SKIP_FORWARD, t!("Nächster Titel"), false, 30.0, 16.0).clicked() {
                    p.next();
                }
            }
            ControlItem::Play => {
                let playing = has && !st.paused;
                if widgets::play_button(ui, &t, playing, 28.0).clicked() {
                    if has {
                        p.toggle_pause();
                    } else if let Some(h) = self.data.history.iter().max_by_key(|h| h.last_played).cloned() {
                        self.open_paths(vec![h.path]);
                    } else {
                        self.dialog_open_files();
                    }
                }
            }
            ControlItem::Stop => {
                if icon_button(ui, &t, ph::STOP, t!("Stopp"), false).clicked() {
                    p.stop();
                }
            }
            ControlItem::Time => {
                let s = if has { media::fmt_time(self.seek_drag.unwrap_or(st.time)) } else { "00:00:00".into() };
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 20.0), Sense::hover());
                ui.painter().text(r.center(), Align2::CENTER_CENTER, s, FontId::proportional(12.0), t.text_dim);
            }
            ControlItem::Duration => {
                let rem = self.data.settings.show_remaining_time;
                let s = if !has {
                    "00:00:00".into()
                } else if rem {
                    format!("-{}", media::fmt_time((st.duration - st.time).max(0.0)))
                } else {
                    media::fmt_time(st.duration)
                };
                let (r, resp) = ui.allocate_exact_size(vec2(64.0, 20.0), Sense::click());
                ui.painter().text(r.center(), Align2::CENTER_CENTER, s, FontId::proportional(12.0), t.text_dim);
                if resp.on_hover_text(t!("Klicken: Restzeit / Gesamtdauer")).clicked() {
                    self.data.settings.show_remaining_time = !rem;
                    self.dirty = true;
                }
            }
            ControlItem::Volume => {
                let icon = if st.muted || st.volume <= 0.0 {
                    ph::SPEAKER_SIMPLE_X
                } else if st.volume < 50.0 {
                    ph::SPEAKER_SIMPLE_LOW
                } else {
                    ph::SPEAKER_SIMPLE_HIGH
                };
                if icon_button(ui, &t, icon, t!("Stumm schalten"), false).clicked() {
                    p.set_mute(!st.muted);
                }
                let mut v = st.volume;
                let max = self.data.settings.max_volume.max(100.0);
                if widgets::volume_cone(ui, &t, &mut v, max, st.muted).changed() {
                    p.set_volume(v);
                    if st.muted {
                        p.set_mute(false);
                    }
                }
            }
            ControlItem::Audio => {
                let resp = icon_button(ui, &t, ph::WAVEFORM, t!("Audiospuren & Ausgabegerät"), false);
                if resp.clicked() {
                    self.track_tab = TrackTab::Audio;
                }
                self.tracks_popup(&resp);
            }
            ControlItem::Subtitles => {
                let on = st.selected("sub").is_some();
                let resp = icon_button(ui, &t, ph::SUBTITLES, t!("Untertitel"), on);
                if resp.clicked() {
                    self.track_tab = TrackTab::Subtitles;
                }
                self.tracks_popup(&resp);
            }
            ControlItem::Adjustments => {
                if icon_button(ui, &t, ph::SLIDERS_HORIZONTAL, t!("Anpassungen & Effekte"), self.right == RightPanel::Adjust).clicked() {
                    self.right = if self.right == RightPanel::Adjust { RightPanel::None } else { RightPanel::Adjust };
                }
            }
            ControlItem::Queue => {
                if icon_button(ui, &t, ph::QUEUE, t!("Warteschlange"), self.right == RightPanel::Queue).clicked() {
                    self.right = if self.right == RightPanel::Queue { RightPanel::None } else { RightPanel::Queue };
                }
            }
            ControlItem::Fullscreen => {
                if icon_button(ui, &t, ph::CORNERS_OUT, t!("Vollbild"), false).clicked() {
                    self.toggle_fullscreen(&ctx);
                }
            }
            ControlItem::Pip => {
                if icon_button(ui, &t, ph::PICTURE_IN_PICTURE, t!("Bild-in-Bild"), self.pip.is_some()).clicked() {
                    self.toggle_pip(&ctx);
                }
            }
            ControlItem::Screenshot => {
                if icon_button(ui, &t, ph::CAMERA, "Screenshot", false).clicked() {
                    p.screenshot();
                    let d = self.data.settings.screenshot_dir.display().to_string();
                    self.osd(tf!("Screenshot gespeichert in {d}"));
                }
            }
            ControlItem::Loop => {
                let r = self.data.settings.repeat;
                let (icon, tip) = match r {
                    RepeatMode::Off => (ph::REPEAT, t!("Wiederholen: aus")),
                    RepeatMode::All => (ph::REPEAT, t!("Wiederholen: alle")),
                    RepeatMode::One => (ph::REPEAT_ONCE, t!("Wiederholen: eine Datei")),
                };
                if icon_button(ui, &t, icon, tip, r != RepeatMode::Off).clicked() {
                    let n = match r {
                        RepeatMode::Off => RepeatMode::All,
                        RepeatMode::All => RepeatMode::One,
                        RepeatMode::One => RepeatMode::Off,
                    };
                    self.set_repeat(n);
                }
            }
            ControlItem::Shuffle => {
                if icon_button(ui, &t, ph::SHUFFLE, t!("Warteschlange mischen"), false).clicked() {
                    p.playlist_shuffle();
                    self.osd(t!("Warteschlange gemischt"));
                }
            }
            ControlItem::Speed => {
                let s = format!("{:.2}×", st.speed).replace(".00", ".0");
                let (r, resp) = ui.allocate_exact_size(vec2(50.0, 26.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(theme::R), t.hover);
                }
                ui.painter().text(r.center(), Align2::CENTER_CENTER, s, FontId::proportional(12.5), if (st.speed - 1.0).abs() > 0.01 { t.accent } else { t.text_dim });
                egui::Popup::menu(&resp).align(RectAlign::TOP).width(120.0).show(|ui| {
                    for sp in [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0] {
                        if widgets::menu_item(ui, &t, "", &format!("{sp}×"), "", (st.speed - sp).abs() < 0.01).clicked() {
                            p.set_speed(sp);
                            ui.close();
                        }
                    }
                });
            }
            ControlItem::Bookmark => {
                if icon_button(ui, &t, ph::BOOKMARK_SIMPLE, t!("Lesezeichen hinzufügen"), false).clicked() {
                    self.add_bookmark(None);
                }
            }
            ControlItem::FrameStep => {
                if icon_button(ui, &t, ph::CARET_LINE_RIGHT, t!("Nächstes Einzelbild"), false).clicked() {
                    p.frame_step();
                }
            }
            ControlItem::Record => {
                let rec = !st.recording.is_empty();
                if icon_button(ui, &t, ph::RECORD, if rec { t!("Aufnahme stoppen") } else { t!("Stream aufnehmen") }, rec).clicked() {
                    self.toggle_record();
                }
            }
        }
    }

    fn tracks_popup(&mut self, resp: &egui::Response) {
        let t = self.t;
        egui::Popup::menu(resp)
            .align(RectAlign::TOP_END)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(320.0)
            .frame(widgets::popup_frame(&t))
            .show(|ui| {
                ui.set_width(310.0);
                let mut tab = self.track_tab;
                widgets::tabs(ui, &t, &mut tab, &[(TrackTab::Audio, ph::WAVEFORM, "Audio"), (TrackTab::Subtitles, ph::SUBTITLES, t!("Untertitel")), (TrackTab::Video, ph::FILM_STRIP, "Video")]);
                self.track_tab = tab;
                ui.add_space(4.0);
                let Some(p) = &self.player else { return };
                let st = p.st.clone();
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    match tab {
                        TrackTab::Subtitles => {
                            widgets::section_label(ui, &t, t!("Spuren"));
                            if widgets::check_row(ui, &t, t!("Keine"), st.selected("sub").is_none()).clicked() {
                                p.set_track("sub", None);
                            }
                            for tr in st.tracks_of("sub") {
                                if widgets::check_row(ui, &t, &tr.label(), tr.selected).clicked() {
                                    p.set_track("sub", Some(tr.id));
                                }
                            }
                            widgets::separator(ui, &t);
                            if widgets::menu_item(ui, &t, ph::PLUS, t!("Untertitel hinzufügen…"), "", false).clicked() {
                                ui.close();
                                self.dialog = Some(Dialog::AddSubs);
                            }
                        }
                        TrackTab::Audio => {
                            widgets::section_label(ui, &t, t!("Eingebettete Spuren"));
                            if widgets::check_row(ui, &t, t!("Keine"), st.selected("audio").is_none()).clicked() {
                                p.set_track("audio", None);
                            }
                            for tr in st.tracks_of("audio") {
                                let mut l = tr.label();
                                if let Some(c) = tr.channels {
                                    l.push_str(&tf!("  · {c} Kan."));
                                }
                                if widgets::check_row(ui, &t, &l, tr.selected).clicked() {
                                    p.set_track("audio", Some(tr.id));
                                }
                            }
                            widgets::separator(ui, &t);
                            widgets::section_label(ui, &t, t!("Ausgabegerät"));
                            for d in &st.audio_devices {
                                let label = if d.name == "auto" { t!("Systemstandard").to_string() } else { d.description.clone() };
                                if widgets::check_row(ui, &t, &label, d.name == st.audio_device).clicked() {
                                    let _ = p.mpv.set_str("audio-device", &d.name);
                                }
                            }
                        }
                        TrackTab::Video => {
                            widgets::section_label(ui, &t, t!("Videospuren"));
                            if widgets::check_row(ui, &t, t!("Keine"), st.selected("video").is_none()).clicked() {
                                p.set_track("video", None);
                            }
                            for tr in st.tracks_of("video") {
                                let mut l = tr.label();
                                if let (Some(w), Some(h)) = (tr.w, tr.h) {
                                    l.push_str(&format!("  · {w}×{h}"));
                                }
                                if tr.albumart {
                                    l.push_str("  · Cover");
                                }
                                if widgets::check_row(ui, &t, &l, tr.selected).clicked() {
                                    p.set_track("video", Some(tr.id));
                                }
                            }
                        }
                    }
                });
                ui.add_space(6.0);
                let w = ui.available_width();
                if ui.add_sized([w, 28.0], egui::Button::new(t!("Einstellungen anzeigen"))).clicked() {
                    ui.close();
                    self.right = RightPanel::Adjust;
                    self.adjust_tab = match tab {
                        TrackTab::Audio => crate::app::AdjustTab::Audio,
                        TrackTab::Subtitles => crate::app::AdjustTab::Subtitles,
                        TrackTab::Video => crate::app::AdjustTab::Video,
                    };
                }
            });
    }

    // ------------------------------------------------------------ seek bar

    pub(crate) fn seek_bar(&mut self, ui: &mut Ui, width: f32) {
        let t = self.t;
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click_and_drag());
        let hovered_bar = resp.hovered() || resp.dragged();
        let th = if hovered_bar { 16.0 } else { 14.0 };
        let track = Rect::from_center_size(rect.center(), vec2(width, th));
        let painter = ui.painter().clone();
        painter.rect_filled(track, CornerRadius::same(3), t.track);
        let Some(p) = &self.player else { return };
        let st = &p.st;
        if !st.has_media() || st.duration <= 0.0 {
            if st.has_media() {
                // live stream: animated stripe
                painter.rect_filled(track, CornerRadius::same(3), t.accent_soft);
                painter.text(track.center(), Align2::CENTER_CENTER, "LIVE", FontId::proportional(10.0), t.accent);
            }
            return;
        }
        let dur = st.duration;
        let to_x = |tm: f64| track.left() + (tm / dur).clamp(0.0, 1.0) as f32 * track.width();
        let to_t = |x: f32| ((x - track.left()) / track.width()).clamp(0.0, 1.0) as f64 * dur;

        // interaction
        let s = &self.data.settings;
        let (snap_on, snap_px, snap_limit) = (s.chapter_snap, s.chapter_snap_px, s.chapter_snap_limit);
        if self.seek_snap.path != st.path {
            self.seek_snap = SnapState { path: st.path.clone(), ..Default::default() };
        }
        // Near a chapter start: snap to it, unless that chapter was already
        // snapped to `snap_limit` times (the user evidently wants nearby).
        // Only snap while moving slowly: sweeping across the bar neither
        // stutters at every chapter nor uses up its snaps.
        let slow = ui.input(|i| i.pointer.velocity().x.abs()) < 250.0;
        if resp.dragged() {
            ui.ctx().request_repaint();
        }
        let snap = &mut self.seek_snap;
        let mut snapped = |x: f32| -> f64 {
            let tm = to_t(x);
            if !snap_on || snap_px <= 0.0 || (!slow && snap.current.is_none()) {
                return tm;
            }
            let near = st.chapters.iter().filter(|c| c.time > 0.5).map(|c| (c.time, (to_x(c.time) - x).abs())).filter(|(_, d)| *d <= snap_px).min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((ct, _)) = near else {
                snap.current = None;
                return tm;
            };
            let key = (ct * 1000.0) as i64;
            if snap.current == Some(key) {
                return ct;
            }
            let n = snap.counts.entry(key).or_insert(0);
            if snap_limit > 0 && *n >= snap_limit {
                snap.current = None;
                return tm;
            }
            *n += 1;
            snap.current = Some(key);
            ct
        };
        if let Some(pp) = resp.interact_pointer_pos() {
            if resp.dragged() || resp.drag_started() {
                let tm = snapped(pp.x);
                if self.seek_drag.map(|d| (d - tm).abs() > dur / 2000.0).unwrap_or(true) {
                    p.seek_abs(tm, false);
                }
                self.seek_drag = Some(tm);
            }
        }
        if resp.drag_stopped() {
            if let Some(tm) = self.seek_drag.take() {
                p.seek_abs(tm, true);
            }
        }
        if resp.clicked() {
            if let Some(pp) = resp.interact_pointer_pos() {
                let tm = snapped(pp.x);
                p.seek_abs(tm, self.data.settings.exact_seek);
            }
        }
        if resp.drag_stopped() || resp.clicked() || !resp.dragged() {
            self.seek_snap.current = None;
        }
        let snapped_at = self.seek_snap.current;

        // cache ranges
        for (a, b) in &st.cache_ranges {
            let r = Rect::from_min_max(pos2(to_x(*a), track.top()), pos2(to_x(*b), track.bottom()));
            painter.rect_filled(r, CornerRadius::same(3), t.hover.gamma_multiply(0.5));
        }
        let pos = self.seek_drag.unwrap_or(st.time);
        let px = to_x(pos);
        painter.rect_filled(Rect::from_min_max(track.min, pos2(px, track.bottom())), CornerRadius::same(3), t.accent_dim);

        // waveform
        let show_wave = self.data.settings.show_waveform || st.is_audio_only();
        if show_wave {
            if let Some((wp, peaks)) = &self.waveform {
                if *wp == st.path && !peaks.is_empty() {
                    let n = peaks.len() as f64;
                    let mut x = track.left() + 1.0;
                    while x < track.right() - 1.0 {
                        let tm = to_t(x);
                        let idx = ((tm / dur) * n) as usize;
                        let pk = peaks.get(idx.min(peaks.len() - 1)).copied().unwrap_or(0.0);
                        let h = (track.height() - 4.0) * pk;
                        let c = if x < px { t.accent.gamma_multiply(0.75) } else { t.text_faint.gamma_multiply(0.55) };
                        painter.line_segment([pos2(x, track.center().y - h / 2.0), pos2(x, track.center().y + h / 2.0)], Stroke::new(1.5, c));
                        x += 3.0;
                    }
                }
            }
        }
        // A-B loop
        if let Some(a) = st.ab_a {
            let b = st.ab_b.unwrap_or(a);
            let r = Rect::from_min_max(pos2(to_x(a), track.top()), pos2(to_x(b).max(to_x(a) + 2.0), track.bottom()));
            painter.rect_stroke(r, CornerRadius::same(2), Stroke::new(1.5, t.accent), StrokeKind::Inside);
        }
        // chapter starts: white marks, wider while snapped to
        let mark = if t.dark || crate::mobile::touch() { Color32::WHITE } else { t.text };
        for c in &st.chapters {
            if c.time > 0.5 {
                let x = to_x(c.time);
                let on = snapped_at == Some((c.time * 1000.0) as i64);
                let w = if on { 4.0 } else { 2.0 };
                let grow = if on { 4.0 } else { 0.0 };
                painter.line_segment([pos2(x, track.top() - grow), pos2(x, track.bottom() + grow)], Stroke::new(w, mark));
            }
        }
        // bookmarks
        if let Some(bms) = self.data.bookmarks.get(&st.path) {
            for b in bms {
                let x = to_x(b.time);
                let y = track.top() - 1.0;
                painter.add(egui::Shape::convex_polygon(vec![pos2(x - 4.0, y - 5.0), pos2(x + 4.0, y - 5.0), pos2(x, y)], t.accent, Stroke::NONE));
            }
        }
        // handle
        let handle = Rect::from_center_size(pos2(px.clamp(track.left() + 3.0, track.right() - 3.0), track.center().y), vec2(6.0, th + 6.0));
        painter.rect_filled(handle, CornerRadius::same(3), t.accent);

        // hover preview
        if let Some(hp) = resp.hover_pos().or(if resp.dragged() { resp.interact_pointer_pos() } else { None }) {
            let ht = to_t(hp.x);
            painter.line_segment([pos2(hp.x, track.top()), pos2(hp.x, track.bottom())], Stroke::new(1.0, t.text.gamma_multiply(0.6)));
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            let chapter = st.chapters.iter().rev().find(|c| c.time <= ht).map(|c| c.title.clone()).filter(|s| !s.is_empty());
            let want_thumb = self.data.settings.show_seek_thumbnails && st.has_video() && !media::is_url(&st.path) && self.tools.available();
            let mut tex = None;
            if want_thumb {
                let step = (dur / 160.0).max(2.0);
                let bucket = (ht / step).round();
                let key = format!("{}@{}", st.path, bucket);
                if self.seek_thumb_req.as_deref() != Some(&key) {
                    self.tools.request_seek_thumb(&crate::plex::authorized(&st.path), bucket * step, key.clone());
                    self.seek_thumb_req = Some(key.clone());
                }
                if let Some((k, tx)) = &self.seek_thumb {
                    if k.starts_with(&format!("{}@", st.path)) {
                        tex = Some(tx.clone());
                    }
                }
            }
            let tw = media::SEEK_W as f32;
            let th_img = media::SEEK_H as f32;
            let card_w = if tex.is_some() { tw + 8.0 } else { 110.0 };
            let card_h = if tex.is_some() { th_img + 30.0 } else { 26.0 } + if chapter.is_some() { 16.0 } else { 0.0 };
            let screen = ui.ctx().screen_rect();
            let x = (hp.x - card_w / 2.0).clamp(screen.left() + 4.0, screen.right() - card_w - 4.0);
            let y = track.top() - card_h - 12.0;
            egui::Area::new(Id::new("seek_preview")).order(Order::Tooltip).fixed_pos(pos2(x, y)).interactable(false).show(ui.ctx(), |ui| {
                let r = Rect::from_min_size(pos2(x, y), vec2(card_w, card_h));
                let pt = ui.painter();
                pt.rect_filled(r, CornerRadius::same(theme::R_LG), t.surface);
                pt.rect_stroke(r, CornerRadius::same(theme::R_LG), Stroke::new(1.0, t.border), StrokeKind::Inside);
                let mut ty = r.top() + 4.0;
                if let Some(tx) = &tex {
                    let ir = Rect::from_min_size(pos2(r.left() + 4.0, ty), vec2(tw, th_img));
                    pt.image(tx.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    ty = ir.bottom() + 4.0;
                }
                if let Some(c) = &chapter {
                    widgets::painter_text_trunc(pt, pos2(r.left() + 8.0, ty), c, FontId::proportional(11.5), t.text_dim, card_w - 16.0);
                    ty += 16.0;
                }
                pt.text(pos2(r.center().x, ty + 10.0), Align2::CENTER_CENTER, media::fmt_time(ht), theme::semibold(12.5), t.text);
                ui.allocate_rect(r, Sense::hover());
            });
        }
        if !self.data.settings.show_seek_thumbnails || resp.hover_pos().is_none() {
            // nothing
        }
    }

    // ----------------------------------------------------------- video area

    pub(crate) fn video_area(&mut self, ui: &mut Ui, overlay: bool, bars_visible: bool) {
        let t = self.t;
        let rect = ui.max_rect();
        let ctx = ui.ctx().clone();
        let resp = ui.interact(rect, Id::new("video_area"), Sense::click_and_drag());
        let painter = ui.painter().clone();
        painter.rect_filled(rect, CornerRadius::ZERO, t.video_bg);
        let Some((p, st)) = self.player.as_ref().map(|p| (p.ctl.clone(), p.st.clone())) else { return };

        if st.is_audio_only() && !st.has_cover() {
            let logo = self.stream_logos.get(&st.path).cloned().and_then(|u| self.logos.get(&u, || u.clone()));
            self.music_art(&painter, rect, &st, logo);
        } else if let Some(v) = &self.video {
            v.paint(&painter, rect);
        }
        if let Some(err) = self.video.as_ref().and_then(|v| v.error()) {
            painter.text(rect.center(), Align2::CENTER_CENTER, err, FontId::proportional(14.0), t.danger);
        }
        if st.is_audio_only() && st.has_cover() {
            // Title overlay for music with cover art.
            let band = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 90.0), rect.max);
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(band.left_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(band.right_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(band.right_bottom(), Color32::from_black_alpha(200));
            mesh.colored_vertex(band.left_bottom(), Color32::from_black_alpha(200));
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(egui::Shape::mesh(mesh));
            let title = st.meta("title").map(|s| s.to_string()).unwrap_or(st.media_title.clone());
            painter.text(pos2(band.left() + 24.0, band.bottom() - 46.0), Align2::LEFT_BOTTOM, title, theme::semibold(20.0), Color32::WHITE);
            let sub = [st.meta("artist"), st.meta("album")].iter().flatten().cloned().collect::<Vec<_>>().join(" · ");
            painter.text(pos2(band.left() + 24.0, band.bottom() - 22.0), Align2::LEFT_BOTTOM, sub, FontId::proportional(13.0), Color32::from_gray(200));
        }

        if st.buffering {
            let c = rect.center();
            let tm = ctx.input(|i| i.time) as f32;
            for i in 0..8 {
                let a = tm * 4.0 + i as f32 * std::f32::consts::TAU / 8.0;
                let alpha = (i as f32 / 8.0 * 255.0) as u8;
                painter.circle_filled(c + vec2(a.cos(), a.sin()) * 18.0, 3.0, Color32::from_rgba_unmultiplied(255, 255, 255, alpha));
            }
            ctx.request_repaint();
        }

        // interactions
        if self.pip.is_some() {
            if resp.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
        }
        if crate::mobile::touch() {
            self.touch_video_input(&resp, rect);
        } else if resp.clicked() && self.data.settings.click_to_pause {
            p.toggle_pause();
        }
        if resp.double_clicked() && self.data.settings.double_click_fullscreen && !crate::mobile::touch() {
            if self.pip.is_some() {
                self.toggle_pip(&ctx);
            } else {
                self.toggle_fullscreen(&ctx);
            }
        }
        if resp.hovered() {
            let scroll = ctx.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                let up = scroll > 0.0;
                match self.data.settings.wheel_action {
                    WheelAction::Volume => {
                        let step = self.data.settings.volume_step;
                        let v = (st.volume + if up { step } else { -step }).clamp(0.0, self.data.settings.max_volume);
                        p.set_volume(v);
                        self.osd(tf!("Lautstärke {v:.0}%"));
                    }
                    WheelAction::Seek => {
                        let s = self.data.settings.seek_small;
                        p.seek_rel(if up { s } else { -s });
                    }
                    WheelAction::Nothing => {}
                }
            }
        }
        resp.context_menu(|ui| self.context_menu(ui));

        // hide the cursor when idle in fullscreen
        if overlay && !bars_visible && self.last_pointer_move.elapsed() > Duration::from_millis(1500) && resp.hovered() {
            ctx.set_cursor_icon(CursorIcon::None);
        }

        // subtitle timeline
        if self.data.settings.show_sub_timeline && !self.ui_hidden && self.pip.is_none() {
            let bottom = if overlay && bars_visible { rect.bottom() - BOTTOM_H } else { rect.bottom() };
            let strip = Rect::from_min_max(pos2(rect.left() + 16.0, bottom - 86.0), pos2(rect.right() - 16.0, bottom - 12.0));
            self.sub_timeline(ui, strip);
        }

        if self.pip.is_some() {
            self.pip_overlay(ui, rect);
        }
    }

    fn music_art(&self, painter: &egui::Painter, rect: Rect, st: &crate::player::PlayerState, logo: Option<crate::plex::images::Shown>) {
        let t = self.t;
        let c = rect.center();
        let s = (rect.height() * 0.32).clamp(60.0, 240.0);
        // Placeholder (audio without cover, or a title still loading): just
        // the logo, muted, no card behind it.
        let card = Rect::from_center_size(c - vec2(0.0, 30.0), vec2(s * 1.3, s * 1.3));
        let mix = |c: u8| (c as f32 * 0.4 + 40.0 * 0.6) as u8;
        let muted = Color32::from_rgb(mix(t.accent.r()), mix(t.accent.g()), mix(t.accent.b()));
        match logo {
            // Radio station logo on a light card (many are dark-on-transparent).
            Some(img) => {
                let k = (card.width() * 0.78 / img.size.x).min(card.height() * 0.78 / img.size.y);
                painter.rect_filled(card, CornerRadius::same(theme::R_LG + 4), Color32::from_gray(235));
                let fit = Rect::from_center_size(card.center(), img.size * k);
                painter.image(img.id, fit, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::from_white_alpha((img.alpha * 255.0) as u8));
            }
            None => widgets::draw_logo(painter, Rect::from_center_size(card.center(), vec2(s * 0.7, s * 0.7)), muted),
        }
        let title = st.meta("title").map(|s| s.to_string()).unwrap_or(st.media_title.clone());
        painter.text(pos2(c.x, card.bottom() + 28.0), Align2::CENTER_CENTER, title, theme::semibold(20.0), t.text);
        let sub = match st.now_playing() {
            Some(song) => song.to_string(),
            None => [st.meta("artist"), st.meta("album")].iter().flatten().cloned().collect::<Vec<_>>().join(" · "),
        };
        painter.text(pos2(c.x, card.bottom() + 54.0), Align2::CENTER_CENTER, sub, FontId::proportional(13.5), t.text_dim);
    }

    fn pip_overlay(&mut self, ui: &mut Ui, rect: Rect) {
        let t = self.t;
        let ctx = ui.ctx().clone();
        let hovered = ctx.input(|i| i.pointer.hover_pos()).map(|p| rect.contains(p)).unwrap_or(false);
        if !hovered {
            return;
        }
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(70));
        let Some(p) = &self.player else { return };
        let st = &p.st;
        // progress line
        if st.duration > 0.0 {
            let r = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 4.0), rect.max);
            widgets::progress_line(painter, r, (st.time / st.duration) as f32, &t);
        }
        let mut btn = |ui: &mut Ui, pos: Pos2, icon: &str, size: f32, id: &str| -> bool {
            let r = Rect::from_center_size(pos, vec2(size + 14.0, size + 14.0));
            let resp = ui.interact(r, Id::new(("pip", id)), Sense::click());
            let bg = if resp.hovered() { Color32::from_black_alpha(200) } else { Color32::from_black_alpha(140) };
            ui.painter().rect_filled(r, CornerRadius::same(6), bg);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, theme::fill_icon(size), Color32::WHITE);
            resp.clicked()
        };
        let playing = !st.paused;
        if btn(ui, rect.center(), if playing { ph::PAUSE } else { ph::PLAY }, 26.0, "play") {
            p.toggle_pause();
        }
        if btn(ui, rect.center() - vec2(64.0, 0.0), ph::SKIP_BACK, 18.0, "prev") {
            p.prev();
        }
        if btn(ui, rect.center() + vec2(64.0, 0.0), ph::SKIP_FORWARD, 18.0, "next") {
            p.next();
        }
        if btn(ui, rect.right_top() + vec2(-22.0, 22.0), ph::X, 14.0, "close") {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if btn(ui, rect.right_top() + vec2(-58.0, 22.0), ph::ARROW_SQUARE_OUT, 14.0, "exit") {
            self.toggle_pip(&ctx);
        }
    }

    fn context_menu(&mut self, ui: &mut Ui) {
        let t = self.t;
        let ctx = ui.ctx().clone();
        ui.set_min_width(250.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        let Some(p) = &self.player else { return };
        let st = p.st.clone();
        if widgets::menu_item(ui, &t, if st.paused { ph::PLAY } else { ph::PAUSE }, if st.paused { t!("Wiedergabe") } else { "Pause" }, &self.shortcut(Action::PlayPause), false).clicked() {
            p.toggle_pause();
            ui.close();
        }
        if widgets::menu_item(ui, &t, ph::STOP, t!("Stopp"), &self.shortcut(Action::Stop), false).clicked() {
            p.stop();
            ui.close();
        }
        if widgets::menu_item(ui, &t, ph::SKIP_BACK, t!("Vorheriger"), &self.shortcut(Action::Prev), false).clicked() {
            p.prev();
            ui.close();
        }
        if widgets::menu_item(ui, &t, ph::SKIP_FORWARD, t!("Nächster"), &self.shortcut(Action::Next), false).clicked() {
            p.next();
            ui.close();
        }
        widgets::separator(ui, &t);
        ui.menu_button(format!("{}   Audio", ph::WAVEFORM), |ui| {
            for tr in st.tracks_of("audio") {
                if widgets::check_row(ui, &t, &tr.label(), tr.selected).clicked() {
                    p.set_track("audio", Some(tr.id));
                }
            }
            widgets::separator(ui, &t);
            for d in &st.audio_devices {
                let label = if d.name == "auto" { t!("Systemstandard").to_string() } else { d.description.clone() };
                if widgets::check_row(ui, &t, &label, d.name == st.audio_device).clicked() {
                    let _ = p.mpv.set_str("audio-device", &d.name);
                }
            }
        });
        ui.menu_button(tf!("{}   Untertitel", ph::SUBTITLES), |ui| {
            if widgets::check_row(ui, &t, t!("Keine"), st.selected("sub").is_none()).clicked() {
                p.set_track("sub", None);
            }
            for tr in st.tracks_of("sub") {
                if widgets::check_row(ui, &t, &tr.label(), tr.selected).clicked() {
                    p.set_track("sub", Some(tr.id));
                }
            }
            widgets::separator(ui, &t);
            if widgets::menu_item(ui, &t, ph::PLUS, t!("Untertitel hinzufügen…"), "", false).clicked() {
                ui.close();
                self.dialog = Some(Dialog::AddSubs);
            }
        });
        ui.menu_button(format!("{}   Video", ph::FILM_STRIP), |ui| {
            widgets::section_label(ui, &t, t!("Seitenverhältnis"));
            for (label, v) in [(t!("Automatisch"), "no"), ("16:9", "16:9"), ("4:3", "4:3"), ("21:9", "2.33"), ("1:1", "1:1"), ("2.35:1", "2.35")] {
                if widgets::check_row(ui, &t, label, st.aspect == v || (v == "no" && st.aspect.starts_with('-'))).clicked() {
                    let _ = p.mpv.set_str("video-aspect-override", v);
                }
            }
            widgets::separator(ui, &t);
            if widgets::menu_item(ui, &t, ph::ARROW_CLOCKWISE, t!("Um 90° drehen"), "", false).clicked() {
                p.set("video-rotate", ((st.rotate + 90) % 360) as f64);
            }
            if widgets::menu_item(ui, &t, ph::CAMERA, "Screenshot", &self.shortcut(Action::Screenshot), false).clicked() {
                p.screenshot();
                ui.close();
            }
        });
        ui.menu_button(tf!("{}   Geschwindigkeit", ph::SPEEDOMETER), |ui| {
            for sp in [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0] {
                if widgets::check_row(ui, &t, &format!("{sp}×"), (st.speed - sp).abs() < 0.01).clicked() {
                    p.set_speed(sp);
                }
            }
        });
        widgets::separator(ui, &t);
        if widgets::menu_item(ui, &t, ph::BOOKMARK_SIMPLE, t!("Lesezeichen hinzufügen"), &self.shortcut(Action::Bookmark), false).clicked() {
            ui.close();
            self.add_bookmark(None);
        }
        if widgets::menu_item(ui, &t, ph::SLIDERS_HORIZONTAL, t!("Anpassungen & Effekte"), &self.shortcut(Action::Sidebar), false).clicked() {
            ui.close();
            self.right = RightPanel::Adjust;
        }
        if widgets::menu_item(ui, &t, ph::PICTURE_IN_PICTURE, t!("Bild-in-Bild"), &self.shortcut(Action::Pip), self.pip.is_some()).clicked() {
            ui.close();
            self.toggle_pip(&ctx);
        }
        if widgets::menu_item(ui, &t, ph::CORNERS_OUT, t!("Vollbild"), &self.shortcut(Action::Fullscreen), false).clicked() {
            ui.close();
            self.toggle_fullscreen(&ctx);
        }
        if widgets::menu_item(ui, &t, ph::EYE_SLASH, if self.ui_hidden { t!("Oberfläche einblenden") } else { t!("Oberfläche ausblenden") }, &self.shortcut(Action::HideUi), self.ui_hidden).clicked() {
            ui.close();
            self.ui_hidden = !self.ui_hidden;
            if self.ui_hidden {
                let k = self.shortcut(Action::HideUi);
                self.osd(tf!("Oberfläche ausgeblendet – {k} zum Einblenden"));
            }
        }
        widgets::separator(ui, &t);
        if widgets::menu_item(ui, &t, ph::FILE_PLUS, t!("Datei öffnen…"), &self.shortcut(Action::OpenFile), false).clicked() {
            ui.close();
            self.dialog_open_files();
        }
        if widgets::menu_item(ui, &t, ph::GEAR, t!("Einstellungen…"), &self.shortcut(Action::Settings), false).clicked() {
            ui.close();
            self.dialog = Some(Dialog::Settings);
        }
    }

    // ------------------------------------------------------ subtitle timeline

    fn sub_timeline(&mut self, ui: &mut Ui, strip: Rect) {
        let t = self.t;
        let Some(p) = &self.player else { return };
        let st = &p.st;
        let painter = ui.painter().clone();
        painter.rect_filled(strip, CornerRadius::same(theme::R_LG), t.panel.gamma_multiply(0.92));
        painter.rect_stroke(strip, CornerRadius::same(theme::R_LG), Stroke::new(1.0, t.border), StrokeKind::Inside);
        let span = 24.0;
        let now = st.time;
        let start = now - span / 3.0;
        let inner = strip.shrink2(vec2(10.0, 8.0));
        let lane = Rect::from_min_max(pos2(inner.left(), inner.top() + 16.0), inner.max);
        let to_x = |tm: f64| lane.left() + ((tm - start) / span) as f32 * lane.width();
        let secs_per_px = span / lane.width() as f64;

        // waveform (audio reference for syncing)
        if let Some((wp, peaks)) = &self.waveform {
            if *wp == st.path && !peaks.is_empty() {
                let mut x = lane.left();
                while x < lane.right() {
                    let tm = start + ((x - lane.left()) as f64) * secs_per_px;
                    if tm >= 0.0 {
                        let idx = (tm * 10.0) as usize;
                        if let Some(pk) = peaks.get(idx) {
                            let h = lane.height() * 0.9 * pk;
                            painter.line_segment([pos2(x, lane.center().y - h / 2.0), pos2(x, lane.center().y + h / 2.0)], Stroke::new(1.5, t.text_faint.gamma_multiply(0.45)));
                        }
                    }
                    x += 3.0;
                }
            }
        }
        // second ticks
        let mut s = start.ceil();
        while s < start + span {
            let x = to_x(s);
            let major = (s as i64) % 5 == 0;
            painter.line_segment([pos2(x, lane.top() - if major { 6.0 } else { 3.0 }), pos2(x, lane.top())], Stroke::new(1.0, t.text_faint));
            if major && s >= 0.0 {
                painter.text(pos2(x + 2.0, inner.top()), Align2::LEFT_TOP, media::fmt_time_short(s.max(0.001)), FontId::proportional(10.0), t.text_faint);
            }
            s += 1.0;
        }
        // cues
        let delay = st.sub_delay;
        match &self.cues {
            Some((_, cues)) if !cues.is_empty() => {
                let clip = painter.with_clip_rect(lane);
                for c in cues {
                    let (a, b) = (c.start + delay, c.end + delay);
                    if b < start || a > start + span {
                        continue;
                    }
                    let r = Rect::from_min_max(pos2(to_x(a), lane.top() + 6.0), pos2(to_x(b).max(to_x(a) + 3.0), lane.bottom() - 6.0));
                    let active = now >= a && now <= b;
                    clip.rect_filled(r, CornerRadius::same(4), if active { t.accent_dim } else { t.accent_soft });
                    clip.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, if active { t.accent } else { t.accent.gamma_multiply(0.5) }), StrokeKind::Inside);
                    let tclip = painter.with_clip_rect(r.shrink(3.0).intersect(lane));
                    tclip.text(pos2(r.left() + 5.0, r.center().y), Align2::LEFT_CENTER, &c.text, FontId::proportional(11.5), t.text);
                }
            }
            Some(_) => {
                painter.text(lane.center(), Align2::CENTER_CENTER, t!("Keine Textuntertitel in dieser Spur (Bild-Untertitel werden nicht angezeigt)"), FontId::proportional(12.0), t.text_faint);
            }
            None => {
                let msg = if st.selected("sub").is_some() { t!("Untertitel werden gelesen…") } else { t!("Keine Untertitelspur ausgewählt") };
                painter.text(lane.center(), Align2::CENTER_CENTER, msg, FontId::proportional(12.0), t.text_faint);
            }
        }
        // now line
        let nx = to_x(now);
        painter.line_segment([pos2(nx, inner.top()), pos2(nx, inner.bottom())], Stroke::new(2.0, t.accent));

        // drag to shift subtitles
        let resp = ui.interact(lane, Id::new("sub_timeline"), Sense::drag());
        if resp.dragged() {
            let dx = resp.drag_delta().x as f64;
            p.set("sub-delay", st.sub_delay + dx * secs_per_px);
        }
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        painter.text(
            pos2(inner.right(), inner.top()),
            Align2::RIGHT_TOP,
            tf!("Untertitel {:+.3} s  ·  ziehen zum Verschieben", st.sub_delay),
            FontId::proportional(10.5),
            if st.sub_delay.abs() > 0.0005 { t.accent } else { t.text_dim },
        );
    }

    // ---------------------------------------------------- overlays/prompts

    pub(crate) fn draw_resume_prompt(&mut self, ctx: &egui::Context, bars_hidden: bool) {
        let Some(rp) = &self.resume_prompt else { return };
        if Instant::now() > rp.until || rp.path != self.current_path() {
            self.resume_prompt = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(200));
        let t = self.t;
        let screen = ctx.screen_rect();
        let (pos, remaining) = (rp.pos, rp.until.saturating_duration_since(Instant::now()).as_secs_f32() / 10.0);
        let mut action = 0;
        if crate::mobile::compact(ctx) {
            // Phone: full-width card above the player's seek bar or the
            // bottom navigation, text on top and buttons below.
            let mm = crate::mobile::m();
            // Full width in portrait, a compact card in landscape.
            let w = (screen.width() - mm.pad * 2.0).min(400.0);
            let bottom = if self.view == View::Player && self.has_media() {
                mm.seek_h + 40.0
            } else {
                (if crate::mobile::portrait(ctx) { mm.nav_h } else { 0.0 }) + if self.has_media() { mm.mini_h } else { 0.0 } + 12.0
            };
            let area = egui::Area::new(Id::new("resume")).order(Order::Foreground).pivot(Align2::LEFT_BOTTOM).fixed_pos(pos2(screen.left() + mm.pad, screen.bottom() - bottom));
            area.show(ctx, |ui| {
                widgets::popup_frame(&t).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                    ui.set_width(w - 28.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(ph::CLOCK_COUNTER_CLOCKWISE).font(theme::icon(18.0)).color(t.accent));
                        ui.add(egui::Label::new(egui::RichText::new(tf!("Dort weitermachen, wo du aufgehört hast? ({})", media::fmt_time_short(pos))).color(t.text)).wrap());
                    });
                    ui.add_space(4.0);
                    let bw = ui.available_width();
                    ui.allocate_ui_with_layout(vec2(bw, 40.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::accent_button(ui, &t, t!("Fortsetzen")).clicked() {
                            action = 1;
                        }
                        if widgets::ghost_button(ui, &t, t!("Von vorn")).clicked() {
                            action = 2;
                        }
                    });
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 2.0), Sense::hover());
                    widgets::progress_line(ui.painter(), r, remaining, &t);
                });
            });
        } else {
        let left = screen.left() + if self.lib.compact_open && self.view == View::Player { 352.0 } else { 12.0 };
        let y = screen.bottom() - if bars_hidden { 20.0 } else { BOTTOM_H } - 62.0;
        egui::Area::new(Id::new("resume")).order(Order::Foreground).fixed_pos(pos2(left, y)).show(ctx, |ui| {
            widgets::popup_frame(&t).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(ph::CLOCK_COUNTER_CLOCKWISE).font(theme::icon(18.0)).color(t.accent));
                    ui.label(egui::RichText::new(tf!("Dort weitermachen, wo du aufgehört hast? ({})", media::fmt_time(pos))).color(t.text));
                    ui.add_space(6.0);
                    if widgets::accent_button(ui, &t, t!("Fortsetzen")).clicked() {
                        action = 1;
                    }
                    if widgets::ghost_button(ui, &t, t!("Von vorn")).clicked() {
                        action = 2;
                    }
                });
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 2.0), Sense::hover());
                widgets::progress_line(ui.painter(), r, remaining, &t);
            });
        });
        }
        match action {
            1 => {
                if let Some(p) = &self.player {
                    p.seek_abs(pos, true);
                }
                self.resume_prompt = None;
            }
            2 => self.resume_prompt = None,
            _ => {}
        }
    }

    pub(crate) fn draw_osd(&mut self, ctx: &egui::Context) {
        let Some(o) = &self.osd else { return };
        let now = Instant::now();
        if now > o.until {
            self.osd = None;
            return;
        }
        let left = o.until.duration_since(now).as_secs_f32();
        let alpha = (left / 0.3).min(1.0);
        ctx.request_repaint_after(Duration::from_millis(50));
        let t = self.t;
        let screen = ctx.screen_rect();
        let chips = o.chips.clone();
        let text = o.text.clone();
        egui::Area::new(Id::new("osd"))
            .order(Order::Tooltip)
            .anchor(Align2::CENTER_TOP, vec2(0.0, if self.pip.is_some() { 8.0 } else { TOP_H + 18.0 }))
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                egui::Frame::new()
                    .fill(t.surface.gamma_multiply(0.96))
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(CornerRadius::same(theme::R_LG))
                    .inner_margin(egui::Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(screen.width() * 0.7);
                        ui.horizontal(|ui| {
                            for (i, c) in chips.iter().enumerate() {
                                if i > 0 {
                                    ui.label(egui::RichText::new("+").color(t.text_faint));
                                }
                                widgets::key_chip(ui, &t, c);
                            }
                            if !chips.is_empty() {
                                ui.add_space(4.0);
                            }
                            ui.label(egui::RichText::new(text).font(theme::semibold(14.0)).color(t.text));
                        });
                    });
            });
    }

    fn draw_drop_overlay(&self, ctx: &egui::Context) {
        if !self.drag_hover_files {
            return;
        }
        let t = self.t;
        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(Order::Foreground, Id::new("drop")));
        painter.rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(160));
        let r = screen.shrink(24.0);
        painter.rect_stroke(r, CornerRadius::same(12), Stroke::new(2.0, t.accent), StrokeKind::Inside);
        painter.text(r.center() - vec2(0.0, 18.0), Align2::CENTER_CENTER, t!("Loslassen zum Abspielen"), theme::semibold(24.0), Color32::WHITE);
        let sub = if self.has_media() {
            t!("Untertiteldateien werden zum aktuellen Video hinzugefügt · Shift oder offene Warteschlange: anhängen")
        } else {
            t!("Dateien und Ordner werden in die Warteschlange gelegt")
        };
        painter.text(r.center() + vec2(0.0, 16.0), Align2::CENTER_CENTER, sub, FontId::proportional(14.0), Color32::from_gray(210));
    }

    fn window_resize_edges(&self, ctx: &egui::Context, fs: bool) {
        let maxed = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if fs || maxed {
            return;
        }
        let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else { return };
        let r = ctx.screen_rect();
        let m = 5.0;
        let (l, rr, top, b) = (p.x - r.left() < m, r.right() - p.x < m, p.y - r.top() < m, r.bottom() - p.y < m);
        use egui::ResizeDirection as D;
        let dir = match (l, rr, top, b) {
            (true, _, true, _) => Some((D::NorthWest, CursorIcon::ResizeNorthWest)),
            (_, true, true, _) => Some((D::NorthEast, CursorIcon::ResizeNorthEast)),
            (true, _, _, true) => Some((D::SouthWest, CursorIcon::ResizeSouthWest)),
            (_, true, _, true) => Some((D::SouthEast, CursorIcon::ResizeSouthEast)),
            (true, ..) => Some((D::West, CursorIcon::ResizeWest)),
            (_, true, ..) => Some((D::East, CursorIcon::ResizeEast)),
            (_, _, true, _) => Some((D::North, CursorIcon::ResizeNorth)),
            (.., true) => Some((D::South, CursorIcon::ResizeSouth)),
            _ => None,
        };
        if let Some((d, c)) = dir {
            ctx.set_cursor_icon(c);
            if ctx.input(|i| i.pointer.primary_pressed()) {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(d));
            }
        }
    }
}

pub fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' }).collect()
}

/// Chapter snapping on the seek bar: how often each chapter start (ms) was
/// snapped to in the current file, and which one is snapped right now.
#[derive(Default)]
pub struct SnapState {
    pub path: String,
    pub counts: std::collections::HashMap<i64, u32>,
    pub current: Option<i64>,
}
