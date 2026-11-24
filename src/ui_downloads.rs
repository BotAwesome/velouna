//! Download manager: running, paused and finished server downloads.
//! Opened from the start page (when there is something to show) and the
//! main menu; the downloads themselves live in `plex::download`.

use crate::app::App;
use crate::plex::download::{DlState, Row};
use crate::plex::{PlayMode, PlayReq};
use crate::theme;
use crate::widgets::{self, icon_button};
use eframe::egui::{self, Align, CornerRadius, Id, Layout, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as ph;

pub(crate) fn fmt_bytes(b: u64) -> String {
    crate::media::fmt_size(b as i64)
}

fn fmt_eta(secs: u64) -> String {
    if secs >= 3600 {
        tf!("noch {} h {} min", secs / 3600, secs % 3600 / 60)
    } else if secs >= 60 {
        tf!("noch {} min", secs.div_ceil(60))
    } else {
        tf!("noch {} s", secs.max(1))
    }
}

fn fmt_date(unix: i64) -> String {
    if unix <= 0 {
        return String::new();
    }
    let days = (unix - crate::epg::now() as i64).abs() / 86400;
    match days {
        0 => t!("heute").to_string(),
        1 => t!("gestern").to_string(),
        d => tf!("vor {d} Tagen"),
    }
}

enum Do {
    Pause(String),
    Resume(String),
    Cancel(String),
    Delete(String),
    Forget(String),
    Play(Row),
    Reveal(String),
    PauseAll,
    ResumeAll,
}

impl App {
    pub fn downloads_page(&mut self, ui: &mut Ui, touch: bool) {
        let t = self.t;
        let rows = self.plex.downloads.rows();
        let active_server = self.plex.active_id().unwrap_or_default();
        let mut todo: Vec<Do> = Vec::new();

        let (open, fin): (Vec<&Row>, Vec<&Row>) = rows.iter().partition(|r| r.path.is_empty());
        let running: Vec<&&Row> = open.iter().filter(|r| matches!(r.state, DlState::Running(_) | DlState::Queued)).collect();
        let speed: u64 = running.iter().map(|r| r.speed).sum();

        // Header: title, summary, actions.
        ui.horizontal(|ui| {
            ui.label(RichText::new("Downloads").font(theme::semibold(if touch { 21.0 } else { 24.0 })).color(t.text));
            ui.add_space(8.0);
            let summary = if !running.is_empty() {
                tf!("{} laufen · {}/s", running.len(), fmt_bytes(speed))
            } else if !open.is_empty() {
                tf!("{} angehalten", open.len())
            } else {
                String::new()
            };
            ui.label(RichText::new(summary).size(13.0).color(t.text_dim));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let dir = self.data.settings.download_dir.clone();
                if icon_button(ui, &t, ph::GEAR, t!("Download-Einstellungen"), false).clicked() {
                    self.settings_page = crate::app::SettingsPage::Plex;
                    self.dialog = Some(crate::app::Dialog::Settings);
                }
                if !crate::platform::MOBILE && widgets::icon_text_button(ui, &t, ph::FOLDER_OPEN, t!("Ordner öffnen")).clicked() {
                    let _ = std::fs::create_dir_all(&dir);
                    open_folder(&dir);
                }
                if open.iter().any(|r| matches!(r.state, DlState::Running(_) | DlState::Queued)) {
                    if widgets::icon_text_button(ui, &t, ph::PAUSE, t!("Alle anhalten")).clicked() {
                        todo.push(Do::PauseAll);
                    }
                } else if open.iter().any(|r| r.server == active_server) && widgets::icon_text_button(ui, &t, ph::PLAY, t!("Alle fortsetzen")).clicked() {
                    todo.push(Do::ResumeAll);
                }
            });
        });
        ui.add_space(10.0);

        if rows.is_empty() {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(ph::DOWNLOAD_SIMPLE).font(theme::icon(48.0)).color(t.text_faint));
                ui.add_space(8.0);
                ui.label(RichText::new(t!("Noch keine Downloads")).font(theme::semibold(17.0)).color(t.text));
                ui.add_space(4.0);
                ui.label(RichText::new(t!("Rechtsklick auf einen Film, eine Folge oder ein Lied bei Plex, Jellyfin oder Emby → Herunterladen.")).color(t.text_dim));
            });
            return;
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if !open.is_empty() {
                section(ui, &t, t!("Aktiv"));
                for r in &open {
                    self.download_row(ui, r, &active_server, touch, &mut todo);
                }
                ui.add_space(14.0);
            }
            if !fin.is_empty() {
                let total: u64 = fin.iter().filter(|r| r.state == DlState::Done).map(|r| r.total).sum();
                section(ui, &t, &tf!("Fertig · {}", fmt_bytes(total)));
                for r in &fin {
                    self.download_row(ui, r, &active_server, touch, &mut todo);
                }
            }
            ui.add_space(20.0);
        });

        let conn = self.plex.conn();
        for d in todo {
            let dl = &self.plex.downloads;
            match d {
                Do::Pause(k) => dl.pause(&k),
                Do::Resume(k) => {
                    if let Some(c) = &conn {
                        dl.resume(&k, c);
                    }
                }
                Do::Cancel(k) => dl.cancel(&k),
                Do::Delete(k) => {
                    dl.delete(&k);
                    self.osd(t!("Download gelöscht"));
                }
                Do::Forget(k) => dl.forget(&k),
                Do::Reveal(p) => crate::ui_panels::show_in_explorer(&p),
                Do::PauseAll => {
                    for r in rows.iter().filter(|r| matches!(r.state, DlState::Running(_) | DlState::Queued)) {
                        dl.pause(&r.key);
                    }
                }
                Do::ResumeAll => {
                    if let Some(c) = &conn {
                        for r in rows.iter().filter(|r| matches!(r.state, DlState::Paused(_)) && r.server == active_server) {
                            dl.resume(&r.key, c);
                        }
                    }
                }
                Do::Play(r) => {
                    // Through the server when it is connected (progress syncs,
                    // plex_execute picks the local copy); offline as a plain file.
                    if r.server == active_server && self.plex.ready() {
                        self.plex.pending_play = Some(PlayReq {
                            list: format!("/library/metadata/{}", r.rating_key),
                            start_key: Some(r.rating_key.clone()),
                            start_at: None,
                            shuffle: false,
                            mode: PlayMode::Play,
                        });
                    } else {
                        self.open_paths(vec![r.path.clone()]);
                    }
                }
            }
        }
    }

    fn download_row(&mut self, ui: &mut Ui, r: &Row, active_server: &str, touch: bool, todo: &mut Vec<Do>) {
        let t = self.t;
        let bar_h = if touch { 4.0 } else { 8.0 };
        let h = if r.path.is_empty() { 62.0 + bar_h } else { 54.0 };
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
        let hovered = resp.hovered();
        if hovered {
            ui.painter().rect_filled(rect, CornerRadius::same(theme::R_LG), t.hover);
        }
        let missing = !r.path.is_empty() && r.state != DlState::Done;
        let icon = match r.state {
            DlState::Done => ph::CHECK_CIRCLE,
            DlState::Running(_) | DlState::Queued => ph::DOWNLOAD_SIMPLE,
            DlState::Paused(_) if !r.error.is_empty() => ph::WARNING,
            DlState::Paused(_) => ph::PAUSE_CIRCLE,
            DlState::None => ph::FILE_X,
        };
        let icol = match r.state {
            DlState::Done | DlState::Running(_) => t.accent,
            DlState::Paused(_) if !r.error.is_empty() => t.danger,
            _ => t.text_faint,
        };
        ui.painter().text(egui::pos2(rect.left() + 20.0, rect.top() + 24.0), egui::Align2::CENTER_CENTER, icon, theme::icon(20.0), icol);

        // Buttons from the right.
        let btn = 32.0;
        let mut bx = rect.right() - 8.0 - btn;
        let mut button = |ui: &mut Ui, icon: &str, tip: &str, enabled: bool| -> bool {
            let r2 = egui::Rect::from_min_size(egui::pos2(bx, rect.top() + (h - btn) / 2.0), vec2(btn, btn));
            bx -= btn + 4.0;
            let mut clicked = false;
            ui.scope_builder(egui::UiBuilder::new().max_rect(r2), |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    clicked = icon_button(ui, &t, icon, tip, false).clicked();
                });
            });
            clicked
        };
        let other_server = !r.server.is_empty() && r.server != active_server;
        let connect_tip = tf!("Verbinde dich mit {}, um fortzusetzen", if r.server_name.is_empty() { "dem Server" } else { &r.server_name });
        let mut confirm_delete = false;
        let cid = Id::new(("dl-del", &r.key));
        match r.state {
            DlState::Running(_) | DlState::Queued => {
                if button(ui, ph::X, t!("Abbrechen"), true) {
                    todo.push(Do::Cancel(r.key.clone()));
                }
                if button(ui, ph::PAUSE, t!("Anhalten"), true) {
                    todo.push(Do::Pause(r.key.clone()));
                }
            }
            DlState::Paused(_) => {
                if button(ui, ph::X, t!("Abbrechen"), true) {
                    todo.push(Do::Cancel(r.key.clone()));
                }
                let tip = if other_server { connect_tip.as_str() } else if r.error.is_empty() { t!("Fortsetzen") } else { t!("Erneut versuchen") };
                if button(ui, if r.error.is_empty() { ph::PLAY } else { ph::ARROW_CLOCKWISE }, tip, !other_server) {
                    todo.push(Do::Resume(r.key.clone()));
                }
            }
            DlState::Done => {
                // Deleting removes the file: the first click asks.
                let armed = ui.ctx().data(|d| d.get_temp::<bool>(cid)).unwrap_or(false);
                if button(ui, ph::TRASH, if armed { t!("Nochmal klicken: Datei löschen") } else { t!("Löschen") }, true) {
                    if armed {
                        todo.push(Do::Delete(r.key.clone()));
                    } else {
                        confirm_delete = true;
                    }
                }
                if armed && !confirm_delete && !hovered {
                    ui.ctx().data_mut(|d| d.remove::<bool>(cid));
                }
                if !crate::platform::MOBILE && button(ui, ph::FOLDER_OPEN, crate::platform::show_in_folder_label(), true) {
                    todo.push(Do::Reveal(r.path.clone()));
                }
                if button(ui, ph::PLAY, t!("Abspielen"), true) {
                    todo.push(Do::Play(r.clone()));
                }
            }
            DlState::None => {
                if button(ui, ph::X, t!("Aus der Liste entfernen"), true) {
                    todo.push(Do::Forget(r.key.clone()));
                }
            }
        }
        if confirm_delete {
            ui.ctx().data_mut(|d| d.insert_temp(cid, true));
        }

        // Title and status lines.
        let tx = rect.left() + 42.0;
        let tw = (bx + btn - 8.0 - tx).max(60.0);
        widgets::painter_text_trunc(ui.painter(), egui::pos2(tx, rect.top() + 9.0), &r.title, theme::semibold(14.0), t.text, tw);
        let status = match r.state {
            DlState::Queued => t!("Wartet").to_string(),
            DlState::Running(_) => {
                let eta = if r.speed > 0 && r.total > r.done { format!(" · {}", fmt_eta((r.total - r.done) / r.speed)) } else { String::new() };
                format!("{} / {} · {}/s{eta}", fmt_bytes(r.done), fmt_bytes(r.total), fmt_bytes(r.speed))
            }
            DlState::Paused(_) if !r.error.is_empty() => tf!("Fehler: {}", r.error),
            DlState::Paused(_) if other_server => tf!("{} / {} · wartet auf {}", fmt_bytes(r.done), fmt_bytes(r.total), r.server_name),
            DlState::Paused(_) if r.user_paused => tf!("{} / {} · angehalten", fmt_bytes(r.done), fmt_bytes(r.total)),
            DlState::Paused(_) => tf!("{} / {} · unterbrochen", fmt_bytes(r.done), fmt_bytes(r.total)),
            DlState::Done => [r.server_name.clone(), fmt_bytes(r.total), fmt_date(r.done_at)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("  ·  "),
            DlState::None => t!("Datei nicht mehr da").to_string(),
        };
        let scol = if missing || (!r.error.is_empty() && r.path.is_empty()) { t.danger } else { t.text_faint };
        widgets::painter_text_trunc(ui.painter(), egui::pos2(tx, rect.top() + 30.0), &status, egui::FontId::proportional(12.0), scol, tw);
        if r.path.is_empty() {
            let frac = match r.state {
                DlState::Running(f) | DlState::Paused(f) => f,
                _ => 0.0,
            };
            let bar = egui::Rect::from_min_size(egui::pos2(tx, rect.top() + 50.0), vec2(tw, bar_h));
            let cr = CornerRadius::same((bar_h / 2.0) as u8);
            ui.painter().rect_filled(bar, cr, t.border);
            let col = if matches!(r.state, DlState::Running(_)) { t.accent } else { t.text_faint };
            ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, vec2(tw * frac.clamp(0.0, 1.0), bar_h)), cr, col);
        }
        // Double-click (desktop) or tap (touch) on a finished row plays it.
        if r.state == DlState::Done && (resp.double_clicked() || (touch && resp.clicked())) {
            todo.push(Do::Play(r.clone()));
        }
    }
}

fn section(ui: &mut Ui, t: &theme::Theme, label: &str) {
    ui.label(RichText::new(label.to_uppercase()).size(11.5).color(t.text_faint));
    ui.add_space(4.0);
}

fn open_folder(dir: &std::path::Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    #[cfg(target_os = "android")]
    let _ = dir;
}
