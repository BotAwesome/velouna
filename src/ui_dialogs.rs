//! Modal dialogs: settings, hotkeys, customization, onboarding, network, convert,
//! add subtitles, playlists, presets, about, errors.

use crate::app::{App, CONVERT_PROFILES, Dialog, SettingsPage, SubScope};
use crate::config::{HandOver, Language, MissingMedia, 
    self, ControlItem, ControlLayout, FullscreenControls, Playlist, RepeatMode, ResumeMode, Settings, Stream, SubAuto, ThemeMode,
    WheelAction,
};
use crate::hotkeys::{self, ACTIONS, Bindings, Combo};
use crate::media;
use crate::pick::Pick;
use crate::theme::{self, ACCENTS};
use crate::widgets::{self, icon_button};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use egui_phosphor::regular as ph;

#[derive(Clone, Copy, PartialEq)]
enum Zone {
    Left,
    Right,
    Palette,
}

#[derive(Clone, Copy)]
struct ControlDrag {
    item: ControlItem,
    from: Zone,
}

fn item_icon(i: ControlItem) -> &'static str {
    match i {
        ControlItem::Prev => ph::SKIP_BACK,
        ControlItem::Play => ph::PLAY,
        ControlItem::Next => ph::SKIP_FORWARD,
        ControlItem::Stop => ph::STOP,
        ControlItem::Time => ph::CLOCK,
        ControlItem::Duration => ph::HOURGLASS,
        ControlItem::Volume => ph::SPEAKER_HIGH,
        ControlItem::Audio => ph::WAVEFORM,
        ControlItem::Subtitles => ph::SUBTITLES,
        ControlItem::Adjustments => ph::SLIDERS_HORIZONTAL,
        ControlItem::Queue => ph::QUEUE,
        ControlItem::Fullscreen => ph::CORNERS_OUT,
        ControlItem::Pip => ph::PICTURE_IN_PICTURE,
        ControlItem::Screenshot => ph::CAMERA,
        ControlItem::Loop => ph::REPEAT,
        ControlItem::Shuffle => ph::SHUFFLE,
        ControlItem::Speed => ph::SPEEDOMETER,
        ControlItem::Bookmark => ph::BOOKMARK_SIMPLE,
        ControlItem::FrameStep => ph::CARET_LINE_RIGHT,
        ControlItem::Record => ph::RECORD,
        ControlItem::Chapters => ph::LIST_NUMBERS,
    }
}

/// Pages that mean something on this device: phones have no mouse or
/// keyboard settings (the double-tap step is part of the touch layout).
fn page_shown(p: SettingsPage) -> bool {
    !(crate::platform::MOBILE && p == SettingsPage::Controls)
}

const PAGES: &[(SettingsPage, &str, &str, &str)] = &[
    (SettingsPage::General, ph::SLIDERS, "Allgemein", "App"),
    (SettingsPage::Interface, ph::CURSOR, "Oberfläche", "App"),
    (SettingsPage::Privacy, ph::SHIELD_CHECK, "Datenschutz", "App"),
    (SettingsPage::Controls, ph::KEYBOARD, "Steuerung", "App"),
    (SettingsPage::Playback, ph::PLAY, "Wiedergabe", "Medien"),
    (SettingsPage::Video, ph::FILM_STRIP, "Video", "Medien"),
    (SettingsPage::Audio, ph::WAVEFORM, "Audio", "Medien"),
    (SettingsPage::Subtitles, ph::SUBTITLES, "Untertitel", "Medien"),
    (SettingsPage::Library, ph::BOOKS, "Bibliothek", "Medien"),
    (SettingsPage::Plex, ph::HARD_DRIVES, "Medienserver", "Medien"),
    (SettingsPage::Network, ph::GLOBE, "Netzwerk", "System"),
    (SettingsPage::Performance, ph::CPU, "Leistung", "System"),
    (SettingsPage::Recording, ph::RECORD, "Aufnahme", "System"),
    (SettingsPage::Extensions, ph::PUZZLE_PIECE, "Erweiterungen", "System"),
];

/// Settings row helper honoring the search filter.
struct Rows<'a> {
    t: theme::Theme,
    filter: &'a str,
    shown: usize,
}

impl Rows<'_> {
    fn row(&mut self, ui: &mut Ui, label: &str, info: &str, add: impl FnOnce(&mut Ui)) {
        if !self.filter.is_empty() {
            let f = self.filter.to_lowercase();
            if !label.to_lowercase().contains(&f) && !info.to_lowercase().contains(&f) {
                return;
            }
        }
        self.shown += 1;
        widgets::setting_row(ui, &self.t, label, info, add);
    }
    fn section(&self, ui: &mut Ui, title: &str) {
        if self.filter.is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new(title).font(theme::semibold(14.0)).color(self.t.text));
            ui.add_space(2.0);
        }
    }
}

/// Small picture of the phone player's bottom area with the chosen buttons.
fn touch_controls_preview(ui: &mut Ui, t: &theme::Theme, tc: &config::TouchControls) {
    let w = ui.available_width().min(420.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 74.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(theme::R_LG), Color32::from_rgb(0x10, 0x10, 0x12));
    let font = FontId::proportional(11.0);
    let seek_y = r.top() + 24.0;
    p.text(pos2(r.left() + 12.0, seek_y), Align2::LEFT_CENTER, "9:18", font.clone(), Color32::WHITE);
    p.text(pos2(r.right() - 12.0, seek_y), Align2::RIGHT_CENTER, "23:40", font, Color32::from_white_alpha(200));
    let track = Rect::from_min_max(pos2(r.left() + 46.0, seek_y - 2.0), pos2(r.right() - 50.0, seek_y + 2.0));
    p.rect_filled(track, CornerRadius::same(2), Color32::from_white_alpha(50));
    p.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * 0.38, track.height())), CornerRadius::same(2), t.accent);
    for f in [0.12, 0.55, 0.8] {
        let x = track.left() + track.width() * f;
        p.line_segment([pos2(x, track.top() - 1.0), pos2(x, track.bottom() + 1.0)], Stroke::new(2.0, Color32::WHITE));
    }
    let icon_y = r.bottom() - 22.0;
    let step = 30.0;
    for (i, it) in tc.left.iter().enumerate() {
        p.text(pos2(r.left() + 22.0 + i as f32 * step, icon_y), Align2::CENTER_CENTER, crate::ui_mobile::touch_item_icon(*it), theme::icon(18.0), Color32::WHITE);
    }
    for (i, it) in tc.right.iter().rev().enumerate() {
        p.text(pos2(r.right() - 22.0 - i as f32 * step, icon_y), Align2::CENTER_CENTER, crate::ui_mobile::touch_item_icon(*it), theme::icon(18.0), Color32::WHITE);
    }
}

/// Dialog width, narrowed to fit phone screens (frame margins included).
pub(crate) fn dlg_width(ui: &mut Ui, w: f32) {
    let screen = ui.ctx().screen_rect().width();
    let pad = if crate::mobile::compact(ui.ctx()) { crate::mobile::m().pad } else { 20.0 };
    ui.set_width(w.min(screen - pad * 2.0 - 36.0).max(200.0));
}

fn modal_frame(t: &theme::Theme) -> egui::Frame {
    egui::Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(theme::R_LG + 2))
        .inner_margin(egui::Margin::same(18))
        .shadow(egui::Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha(140) })
}

impl App {
    pub fn draw_dialogs(&mut self, ctx: &egui::Context) {
        let Some(d) = self.dialog.clone() else { return };
        let t = self.t;
        if matches!(d, Dialog::Settings) && crate::mobile::compact(ctx) {
            self.mobile_settings(ctx);
            return;
        }
        // One Id per kind of dialog, so a small one does not keep the size of the last big one.
        let modal = egui::Modal::new(Id::new(("dialog", std::mem::discriminant(&d)))).frame(if matches!(d, Dialog::Settings) { modal_frame(&t).inner_margin(egui::Margin::ZERO) } else { modal_frame(&t) }).backdrop_color(Color32::from_black_alpha(150));
        let resp = modal.show(ctx, |ui| match &d {
            Dialog::Settings => self.dlg_settings(ui),
            Dialog::Hotkeys => self.dlg_hotkeys(ui),
            Dialog::Customize => self.dlg_customize(ui),
            Dialog::Network => self.dlg_network(ui),
            Dialog::Convert => self.dlg_convert(ui),
            Dialog::AddSubs => self.dlg_add_subs(ui),
            Dialog::NewPlaylist(items) => self.dlg_new_playlist(ui, items.clone()),
            Dialog::AddToPlaylist(items) => self.dlg_add_to_playlist(ui, items.clone()),
            Dialog::SavePreset => self.dlg_save_preset(ui),
            Dialog::About => self.dlg_about(ui),
            Dialog::Onboarding(step) => self.dlg_onboarding(ui, *step),
            Dialog::Error(e) => self.dlg_error(ui, e.clone()),
            Dialog::Update => self.dlg_update(ui),
            Dialog::WhatsNew(notes) => self.dlg_whats_new(ui, notes.clone()),
            Dialog::TvGuide { title, country, keys } => self.dlg_tv_guide(ui, title, country, keys),
            Dialog::MissingMedia(list) => self.dlg_missing_media(ui, list.clone()),
            Dialog::PluginInstall(pkg) => self.dlg_plugin_install(ui, pkg),
            Dialog::PluginRemove(id) => self.dlg_plugin_remove(ui, id),
            Dialog::RepoAdd(f) => self.dlg_repo_add(ui, f),
            Dialog::RepoLink => self.dlg_repo_link(ui),
            Dialog::RepoKey(url) => self.dlg_repo_key(ui, url),
            Dialog::PluginGrant(id) => self.dlg_plugin_grant(ui, id),
            Dialog::PluginSettings(id) => self.dlg_plugin_settings(ui, id),
            Dialog::PluginConsole(id) => self.dlg_plugin_console(ui, id),
        });
        let blocking = matches!(d, Dialog::Onboarding(_)) || self.rebinding.is_some() || matches!(d, Dialog::Convert if self.dlg.convert_progress.is_some());
        if resp.should_close() && !blocking && self.dialog.as_ref() == Some(&d) {
            self.dialog = None;
        }
        if matches!(d, Dialog::PluginGrant(_) | Dialog::PluginSettings(_) | Dialog::PluginConsole(_)) && self.dialog.is_none() && std::mem::take(&mut self.ext_ui.back_to_settings) {
            self.dialog = Some(Dialog::Settings);
        }
    }

    /// Program of one TV channel, from now on, grouped by day.
    fn dlg_tv_guide(&mut self, ui: &mut Ui, title: &str, country: &str, keys: &[String]) {
        let t = self.t;
        dlg_width(ui, 560.0);
        if self.dlg_title(ui, &tf!("Programm · {}", title)) {
            return;
        }
        let guide = self.epg.guide(country);
        let now = crate::epg::now();
        let progs: Vec<crate::epg::Prog> = guide.as_ref().and_then(|g| g.lookup(keys)).map(|p| p.iter().filter(|x| x.stop > now).cloned().collect()).unwrap_or_default();
        if progs.is_empty() {
            let msg = if guide.is_none() && self.epg.loading() { t!("Programm wird geladen …") } else { t!("Für diesen Sender gibt es kein Programm.") };
            ui.label(RichText::new(msg).color(t.text_dim));
            return;
        }
        let max_h = (ui.ctx().screen_rect().height() * 0.65).clamp(200.0, 620.0);
        let today = crate::epg::day_of(now);
        egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
            let mut last_day = None;
            for p in &progs {
                let day = crate::epg::day_of(p.start.max(now));
                if last_day != Some(day) {
                    if last_day.is_some() {
                        ui.add_space(10.0);
                    }
                    let label = match day - today {
                        0 => t!("Heute").to_string(),
                        1 => t!("Morgen").to_string(),
                        _ => t!("Später").to_string(),
                    };
                    ui.label(RichText::new(label).font(theme::semibold(13.0)).color(t.text_faint));
                    ui.add_space(4.0);
                    last_day = Some(day);
                }
                let running = p.start <= now;
                ui.horizontal_top(|ui| {
                    let time = RichText::new(crate::epg::clock(p.start)).font(theme::semibold(14.0)).color(if running { t.accent } else { t.text_dim });
                    ui.add_sized([48.0, 18.0], egui::Label::new(time));
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(RichText::new(&p.title).font(theme::semibold(14.0)).color(t.text)).wrap());
                        if running {
                            let frac = ((now - p.start) as f32 / (p.stop - p.start).max(1) as f32).clamp(0.0, 1.0);
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(220.0), 3.0), Sense::hover());
                            ui.painter().rect_filled(r, CornerRadius::same(2), t.border);
                            ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(r.width() * frac, r.height())), CornerRadius::same(2), t.accent);
                            ui.label(RichText::new(tf!("läuft bis {}", crate::epg::clock(p.stop))).size(12.0).color(t.text_faint));
                        }
                        if !p.desc.is_empty() {
                            let mut d: String = p.desc.chars().take(240).collect();
                            if d.len() < p.desc.len() {
                                d.push('…');
                            }
                            ui.add(egui::Label::new(RichText::new(d).size(12.5).color(t.text_dim)).wrap());
                        }
                    });
                });
                ui.add_space(8.0);
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new(t!("Programmdaten: epgshare01.online")).size(11.0).color(t.text_faint));
    }

    pub(crate) fn dlg_title(&mut self, ui: &mut Ui, title: &str) -> bool {
        let t = self.t;
        let mut close = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).font(theme::semibold(17.0)).color(t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, &t, ph::X, t!("Schließen"), false).clicked() {
                    close = true;
                }
            });
        });
        ui.add_space(8.0);
        if close {
            self.dialog = None;
            self.rebinding = None;
        }
        close
    }

    // ========================================================== settings

    fn dlg_settings(&mut self, ui: &mut Ui) {
        let t = self.t;
        let screen = ui.ctx().screen_rect();
        let size = vec2((screen.width() - 80.0).clamp(560.0, 900.0), (screen.height() - 80.0).clamp(380.0, 640.0));
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let nav = Rect::from_min_size(rect.min, vec2(210.0, rect.height()));
        ui.painter().rect_filled(nav, CornerRadius { nw: theme::R_LG + 2, sw: theme::R_LG + 2, ne: 0, se: 0 }, t.panel);
        let mut page = self.settings_page;
        ui.scope_builder(egui::UiBuilder::new().max_rect(nav.shrink(12.0)), |ui| {
            let w = ui.available_width();
            widgets::search_field(ui, &t, egui::Id::new("settings-search-field"), &mut self.settings_search, t!("Einstellungen suchen"), w);
            ui.add_space(8.0);
            egui::ScrollArea::vertical().id_salt("set_nav").show(ui, |ui| {
                let mut group = "";
                for (p, icon, label, g) in PAGES.iter().filter(|x| page_shown(x.0)) {
                    if *g != group {
                        group = g;
                        ui.add_space(6.0);
                        ui.label(RichText::new(crate::i18n::tr(g)).size(11.0).color(t.text_faint));
                    }
                    let w = ui.available_width();
                    let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                    let sel = page == *p && self.settings_search.is_empty();
                    if sel {
                        ui.painter().rect_filled(r, CornerRadius::same(theme::R), t.elevated);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(theme::R), t.hover);
                    }
                    ui.painter().text(pos2(r.left() + 14.0, r.center().y), Align2::CENTER_CENTER, *icon, theme::icon(15.0), if sel { t.accent } else { t.text_dim });
                    ui.painter().text(pos2(r.left() + 32.0, r.center().y), Align2::LEFT_CENTER, crate::i18n::tr(label), FontId::proportional(13.0), if sel { t.text } else { t.text_dim });
                    if resp.clicked() {
                        page = *p;
                        self.settings_search.clear();
                    }
                }
            });
        });
        self.settings_page = page;

        let content = Rect::from_min_max(pos2(nav.right() + 20.0, rect.top() + 14.0), pos2(rect.right() - 16.0, rect.bottom() - 10.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            let title = if self.settings_search.is_empty() {
                tf!("{}-Einstellungen", crate::i18n::tr(PAGES.iter().find(|x| x.0 == page).map(|x| x.2).unwrap_or("")))
            } else {
                tf!("Suchergebnisse für „{}“", self.settings_search)
            };
            if self.dlg_title(ui, &title) {
                return;
            }
            egui::ScrollArea::vertical().id_salt("set_content").auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width() - 8.0);
                self.settings_body(ui, page);
            });
        });
    }

    /// One settings page, or the search results over all pages.
    fn settings_body(&mut self, ui: &mut Ui, page: SettingsPage) {
        let t = self.t;
        let filter = self.settings_search.clone();
        let mut s = self.data.settings.clone();
        let mut rows = Rows { t, filter: &filter, shown: 0 };
        if filter.is_empty() {
            self.settings_page_ui(ui, page, &mut s, &mut rows);
        } else {
            for (p, _, label, _) in PAGES.iter().filter(|x| page_shown(x.0)) {
                let before = rows.shown;
                ui.scope(|ui| self.settings_page_ui(ui, *p, &mut s, &mut rows));
                if rows.shown > before {
                    ui.label(RichText::new(format!("↑ {}", crate::i18n::tr(label))).size(10.5).color(t.text_faint));
                    widgets::separator(ui, &t);
                }
            }
            if rows.shown == 0 {
                ui.label(RichText::new(t!("Keine Einstellung gefunden.")).color(t.text_dim));
            }
        }
        self.commit_settings(ui.ctx(), s);
    }

    /// Back key in the phone settings: page → list → search → closed.
    pub(crate) fn mobile_settings_back(&mut self, ctx: &egui::Context) {
        let open_id = Id::new("m_settings_page_open");
        if ctx.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false) {
            ctx.data_mut(|d| d.insert_temp(open_id, false));
        } else if !self.settings_search.is_empty() {
            self.settings_search.clear();
        } else {
            self.dialog = None;
        }
    }

    /// Phone settings: full screen, category list first, then the page.
    fn mobile_settings(&mut self, ctx: &egui::Context) {
        let t = self.t;
        let mm = crate::mobile::m();
        let screen = ctx.screen_rect();
        let open_id = Id::new("m_settings_page_open");
        let mut page_open = ctx.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
        egui::Area::new(Id::new("m_settings"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .constrain(false)
            .show(ctx, |ui| {
                ui.set_min_size(screen.size());
                ui.set_max_size(screen.size());
                ui.painter().rect_filled(screen, CornerRadius::ZERO, t.bg);
                // Swallow touches so nothing behind reacts.
                ui.interact(screen, Id::new("m_settings_bg"), Sense::click_and_drag());

                let head_h = if crate::mobile::portrait(ctx) { mm.header_h } else { mm.header_h_land };
                let head = Rect::from_min_size(screen.min, vec2(screen.width(), head_h));
                let b = mm.icon_btn;
                let back = Rect::from_center_size(pos2(head.left() + 6.0 + b / 2.0, head.center().y), vec2(b, b));
                let searching = !self.settings_search.is_empty();
                let back_r = ui.interact(back, Id::new("m_set_back"), Sense::click());
                ui.painter().text(back.center(), Align2::CENTER_CENTER, ph::ARROW_LEFT, theme::icon(22.0), t.text);
                if back_r.clicked() {
                    if page_open {
                        page_open = false;
                    } else if searching {
                        self.settings_search.clear();
                    } else {
                        self.dialog = None;
                    }
                }
                let title = if page_open {
                    crate::i18n::tr(PAGES.iter().find(|x| x.0 == self.settings_page).map(|x| x.2).unwrap_or("")).to_string()
                } else {
                    t!("Einstellungen").to_string()
                };
                widgets::painter_text_trunc(ui.painter(), pos2(back.right() + 4.0, head.center().y - mm.title_size * 0.62), &title, theme::semibold(mm.title_size), t.text, head.right() - back.right() - 16.0);

                let pad = mm.pad;
                let body = Rect::from_min_max(pos2(screen.left() + pad, head.bottom()), pos2(screen.right() - pad, screen.bottom()));
                ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                    if page_open {
                        egui::ScrollArea::vertical().id_salt(("m_set_page", self.settings_page as u8)).auto_shrink([false, false]).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let page = self.settings_page;
                            self.settings_body(ui, page);
                            ui.add_space(24.0);
                        });
                        return;
                    }
                    let w = ui.available_width();
                    widgets::search_field(ui, &t, Id::new("settings-search-field"), &mut self.settings_search, t!("Einstellungen suchen"), w);
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().id_salt("m_set_list").auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if !self.settings_search.is_empty() {
                            let page = self.settings_page;
                            self.settings_body(ui, page);
                            ui.add_space(24.0);
                            return;
                        }
                        let mut group = "";
                        for (p, icon, label, g) in PAGES.iter().filter(|x| page_shown(x.0)) {
                            if *g != group {
                                group = g;
                                ui.add_space(10.0);
                                ui.label(RichText::new(crate::i18n::tr(g)).size(12.0).color(t.text_faint));
                                ui.add_space(2.0);
                            }
                            let w = ui.available_width();
                            let (r, resp) = ui.allocate_exact_size(vec2(w, 50.0), Sense::click());
                            if resp.is_pointer_button_down_on() {
                                ui.painter().rect_filled(r, CornerRadius::same(theme::R), t.hover);
                            }
                            ui.painter().text(pos2(r.left() + 16.0, r.center().y), Align2::CENTER_CENTER, *icon, theme::icon(19.0), t.accent);
                            ui.painter().text(pos2(r.left() + 42.0, r.center().y), Align2::LEFT_CENTER, crate::i18n::tr(label), FontId::proportional(15.0), t.text);
                            ui.painter().text(pos2(r.right() - 8.0, r.center().y), Align2::RIGHT_CENTER, ph::CARET_RIGHT, theme::icon(14.0), t.text_faint);
                            if resp.clicked() {
                                self.settings_page = *p;
                                page_open = true;
                            }
                        }
                        ui.add_space(24.0);
                    });
                });
            });
        if self.dialog.is_none() {
            page_open = false;
        }
        ctx.data_mut(|d| d.insert_temp(open_id, page_open));
    }

    fn commit_settings(&mut self, ctx: &egui::Context, s: Settings) {
        let old = self.data.settings.clone();
        if format!("{s:?}") == format!("{old:?}") {
            return;
        }
        if s.theme != old.theme || s.accent != old.accent || (s.ui_scale - old.ui_scale).abs() > 0.001 {
            self.theme_dirty = true;
        }
        if s.language != old.language {
            crate::i18n::apply(s.language);
            // Plex metadata (genres, summaries) comes back in the new language.
            self.plex.clear_cache();
        }
        // Turned on: this window takes the files of later starts from now on.
        #[cfg(not(target_os = "android"))]
        if s.single_instance && !old.single_instance {
            crate::instance::listen();
        }
        if s.always_on_top != old.always_on_top {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if s.always_on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal }));
        }
        if let Some(p) = &self.player {
            let m = &p.mpv;
            if s.hwdec != old.hwdec {
                let _ = m.set_str("hwdec", if s.hwdec { crate::platform::HWDEC } else { "no" });
            }
            if s.sub_auto != old.sub_auto {
                let _ = m.set_str("sub-auto", s.sub_auto.mpv());
            }
            if s.max_volume != old.max_volume {
                let _ = m.set_f64("volume-max", s.max_volume.max(100.0));
            }
            if s.screenshot_dir != old.screenshot_dir {
                let _ = std::fs::create_dir_all(&s.screenshot_dir);
                let _ = m.set_str("screenshot-directory", &s.screenshot_dir.to_string_lossy());
            }
            if s.screenshot_format != old.screenshot_format {
                let _ = m.set_str("screenshot-format", &s.screenshot_format);
            }
            if s.cache_secs != old.cache_secs {
                let _ = m.set_f64("cache-secs", s.cache_secs);
            }
            if s.ytdl != old.ytdl {
                let _ = m.set_flag("ytdl", s.ytdl);
            }
            if s.user_agent != old.user_agent {
                let _ = m.set_str("user-agent", &s.user_agent);
            }
            if s.alang != old.alang {
                let _ = m.set_str("alang", &s.alang);
            }
            if s.slang != old.slang {
                let _ = m.set_str("slang", &s.slang);
            }
            if s.deinterlace != old.deinterlace {
                let _ = m.set_flag("deinterlace", s.deinterlace);
            }
            if s.keep_aspect != old.keep_aspect {
                let _ = m.set_flag("keepaspect", s.keep_aspect);
            }
            if s.video_sync_display != old.video_sync_display {
                let _ = m.set_str("video-sync", if s.video_sync_display { "display-resample" } else { "audio" });
            }
            if s.demuxer_max_mb != old.demuxer_max_mb {
                let _ = m.set_str("demuxer-max-bytes", &format!("{}MiB", s.demuxer_max_mb));
            }
            if s.network_timeout != old.network_timeout {
                let _ = m.set_f64("network-timeout", s.network_timeout);
            }
        }
        let sub_changed = s.sub_font != old.sub_font
            || s.sub_font_size != old.sub_font_size
            || s.sub_color != old.sub_color
            || s.sub_border_size != old.sub_border_size
            || s.sub_back_opacity != old.sub_back_opacity
            || s.sub_bold != old.sub_bold
            || s.sub_pos != old.sub_pos;
        let audio_changed = s.normalize != old.normalize || s.audio_channels != old.audio_channels;
        let repeat_changed = s.repeat != old.repeat;
        let scan = s.music_folders != old.music_folders;
        self.data.settings = s;
        if sub_changed {
            self.apply_sub_style_from_settings();
        }
        if audio_changed {
            self.fx_normalize = self.data.settings.normalize;
            self.apply_audio_filters();
        }
        if repeat_changed {
            self.apply_repeat();
        }
        if scan {
            self.lib.scanning = false;
            self.lib.start_music_scan(&self.data, ctx);
        }
        self.mark_dirty();
    }

    fn settings_page_ui(&mut self, ui: &mut Ui, page: SettingsPage, s: &mut Settings, r: &mut Rows) {
        let t = self.t;
        match page {
            SettingsPage::General => {
                r.section(ui, t!("Sprache"));
                let lang_info = if crate::platform::MOBILE { t!("Sprache der Oberfläche. „System“ folgt der Gerätesprache.") } else { if cfg!(windows) { t!("Sprache der Oberfläche. „System“ folgt der Windows-Anzeigesprache.") } else { t!("Sprache der Oberfläche. „System“ folgt der Systemsprache.") } };
                r.row(ui, t!("Sprache"), lang_info, |ui| {
                    widgets::segmented(ui, &t, &mut s.language, &[(Language::System, "System"), (Language::German, "Deutsch"), (Language::English, "English")]);
                });
                r.section(ui, t!("Darstellung"));
                r.row(ui, t!("Design"), t!("Hell, dunkel oder wie das System."), |ui| {
                    widgets::segmented(ui, &t, &mut s.theme, &[(ThemeMode::Light, t!("Hell")), (ThemeMode::Dark, t!("Dunkel")), (ThemeMode::System, "System")]);
                });
                r.row(ui, t!("Akzentfarbe"), t!("Färbt Hervorhebungen, Schaltflächen und das Logo."), |ui| {
                    let mut c = s.accent;
                    ui.color_edit_button_srgb(&mut c);
                    s.accent = c;
                    for a in ACCENTS.iter().rev() {
                        if widgets::color_swatch(ui, &t, Color32::from_rgb(a[0], a[1], a[2]), s.accent == *a, 22.0).clicked() {
                            s.accent = *a;
                        }
                    }
                });
                r.row(ui, t!("Oberflächengröße"), t!("Skaliert die gesamte Oberfläche."), |ui| {
                    let mut v = s.ui_scale as f64;
                    widgets::value_box(ui, &t, &format!("{:.0} %", v * 100.0), 60.0);
                    if widgets::slider(ui, &t, &mut v, 0.8, 1.5, 160.0).drag_stopped() || ui.input(|i| i.pointer.any_released()) {
                        s.ui_scale = ((v * 20.0).round() / 20.0) as f32;
                    }
                });
                r.section(ui, t!("Updates"));
                r.row(ui, t!("Automatisch nach Updates suchen"), t!("Einmal am Tag beim Start; fragt, bevor etwas installiert wird."), |ui| {
                    widgets::toggle(ui, &t, &mut s.update_auto);
                });
                let status = self.update_settings_status();
                r.row(ui, &tf!("Version {}", crate::update::CURRENT), &status, |ui| {
                    if ui.add_enabled(!self.update.busy(), egui::Button::new(t!("Jetzt suchen"))).clicked() {
                        self.open_update_dialog(ui.ctx());
                    }
                });
                r.row(ui, t!("Änderungen"), t!("Was sich in den bisherigen Versionen getan hat."), |ui| {
                    if ui.button(t!("Anzeigen")).clicked() {
                        self.dialog = Some(Dialog::WhatsNew(crate::update::changes_since("0.0.0")));
                    }
                });
                r.section(ui, t!("Einführung"));
                r.row(ui, t!("Einführung erneut anzeigen"), if crate::platform::MOBILE { t!("Design, Farbe und Bibliothek.") } else { t!("Design, Farbe und wichtige Tastenkürzel.") }, |ui| {
                    if ui.button(t!("Starten")).clicked() {
                        self.dialog = Some(Dialog::Onboarding(0));
                    }
                });
                r.section(ui, t!("Erweitert"));
                r.row(ui, t!("Alle Einstellungen zurücksetzen"), t!("Setzt Einstellungen (nicht Bibliothek und Verlauf) auf Standard."), |ui| {
                    if ui.button(RichText::new(t!("Zurücksetzen")).color(t.danger)).clicked() {
                        let folders = (s.video_folders.clone(), s.music_folders.clone());
                        *s = Settings::default();
                        s.onboarding_done = true;
                        s.video_folders = folders.0;
                        s.music_folders = folders.1;
                    }
                });
            }
            // Touch: the phone has its own player bar (touch editor); window,
            // mouse and keyboard options don't apply there.
            SettingsPage::Interface if crate::mobile::touch() => {
                r.section(ui, t!("Player-Leiste"));
                let mut open_editor = false;
                r.row(ui, t!("Player-Leiste bearbeiten"), t!("Öffnet den Player: Elemente per Ziehen links oder rechts unter der Zeitleiste platzieren."), |ui| {
                    if widgets::accent_button(ui, &t, t!("Bearbeiten")).clicked() {
                        open_editor = true;
                    }
                });
                if open_editor {
                    self.data.settings = s.clone();
                    self.open_touch_editor(ui.ctx());
                }
                r.section(ui, t!("Leiste unten"));
                for (id, label, _) in crate::config::NAV_ITEMS.iter().filter(|x| matches!(x.0, "servers" | "videos" | "music")) {
                    r.row(ui, crate::i18n::tr(label), "", |ui| {
                        let mut on = s.nav_shown(id);
                        widgets::toggle(ui, &t, &mut on);
                        if on != s.nav_shown(id) {
                            s.set_nav_shown(id, on);
                        }
                    });
                }
                r.section(ui, t!("Erweitert"));
                r.row(ui, t!("Ausblendverzögerung"), t!("Wie lange die Leisten nach dem Wegbewegen sichtbar bleiben."), |ui| {
                    let mut v = s.hide_delay_ms as f64;
                    widgets::value_box(ui, &t, &format!("{:.1} s", v / 1000.0), 60.0);
                    widgets::slider(ui, &t, &mut v, 0.0, 3000.0, 140.0);
                    s.hide_delay_ms = (v / 100.0).round() as u64 * 100;
                });
                r.row(ui, t!("Bibliothek: ganzen Ordner einreihen"), t!("Beim Abspielen aus der Bibliothek landen alle Folgen des Ordners in der Warteschlange."), |ui| {
                    widgets::toggle(ui, &t, &mut s.library_add_folder_to_queue);
                });
            }
            SettingsPage::Interface => {
                r.section(ui, t!("Steuerleiste"));
                r.row(ui, t!("Steuerleiste im Fenster automatisch ausblenden"), t!("Leisten erscheinen nur, wenn die Maus in ihre Nähe kommt."), |ui| {
                    widgets::toggle(ui, &t, &mut s.autohide_windowed);
                });
                r.row(ui, t!("Steuerelemente im Vollbild"), t!("Titel und Steuerleiste bleiben im Vollbild erreichbar."), |ui| {
                    widgets::segmented(ui, &t, &mut s.fullscreen_controls, &[(FullscreenControls::Hover, t!("Beim Darüberfahren")), (FullscreenControls::Always, t!("Immer"))]);
                });
                r.row(ui, t!("Vorschaubilder beim Spulen"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.show_seek_thumbnails);
                });
                r.row(ui, t!("Wellenform in der Suchleiste"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.show_waveform);
                });
                r.row(ui, t!("Restzeit statt Gesamtdauer"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.show_remaining_time);
                });
                r.row(ui, t!("Oberfläche anpassen"), t!("Schaltflächen per Drag & Drop anordnen."), |ui| {
                    if ui.button(t!("Öffnen…")).clicked() {
                        self.dialog = Some(Dialog::Customize);
                    }
                });
                r.section(ui, t!("Seitenleiste"));
                for (id, label, _) in crate::config::NAV_ITEMS.iter() {
                    r.row(ui, crate::i18n::tr(label), "", |ui| {
                        let mut on = s.nav_shown(id);
                        widgets::toggle(ui, &t, &mut on);
                        if on != s.nav_shown(id) {
                            s.set_nav_shown(id, on);
                        }
                    });
                }
                r.section(ui, t!("Fenster"));
                r.row(ui, t!("Immer im Vordergrund"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.always_on_top);
                });
                if !crate::platform::MOBILE {
                    r.row(ui, t!("Nur ein Velouna-Fenster"), t!("Öffnest du eine Datei, während Velouna schon läuft, übernimmt das laufende Fenster sie. Aus: jede Datei öffnet ein neues Fenster."), |ui| {
                        widgets::toggle(ui, &t, &mut s.single_instance);
                    });
                    if s.single_instance {
                        r.row(ui, t!("Dateien im laufenden Fenster"), t!("Mehrere auf einmal geöffnete Dateien kommen zusammen an."), |ui| {
                            widgets::segmented(ui, &t, &mut s.single_instance_files, &[(HandOver::Play, t!("Sofort abspielen")), (HandOver::Enqueue, t!("Einreihen"))]);
                        });
                    }
                }
                r.row(ui, t!("Bildschirmmeldungen zeigen Tastenkürzel"), t!("Zeigt z. B. [Strg]+[B] zusammen mit der ausgelösten Aktion."), |ui| {
                    widgets::toggle(ui, &t, &mut s.osd_show_keys);
                });
                r.section(ui, t!("Erweitert"));
                r.row(ui, t!("Ausblendverzögerung"), t!("Wie lange die Leisten nach dem Wegbewegen sichtbar bleiben."), |ui| {
                    let mut v = s.hide_delay_ms as f64;
                    widgets::value_box(ui, &t, &format!("{:.1} s", v / 1000.0), 60.0);
                    widgets::slider(ui, &t, &mut v, 0.0, 3000.0, 140.0);
                    s.hide_delay_ms = (v / 100.0).round() as u64 * 100;
                });
                r.row(ui, t!("Bibliothek: ganzen Ordner einreihen"), t!("Beim Abspielen aus der Bibliothek landen alle Folgen des Ordners in der Warteschlange."), |ui| {
                    widgets::toggle(ui, &t, &mut s.library_add_folder_to_queue);
                });
            }
            SettingsPage::Privacy => {
                r.section(ui, t!("Verlauf"));
                r.row(ui, t!("Verlauf speichern"), t!("Zuletzt gespielte Medien auf der Startseite."), |ui| {
                    widgets::toggle(ui, &t, &mut s.remember_history);
                });
                r.row(ui, t!("Wiedergabepositionen merken"), t!("Für „Weiterschauen“ und das Fortsetzen."), |ui| {
                    widgets::toggle(ui, &t, &mut s.remember_positions);
                });
                r.row(ui, t!("Maximale Einträge"), "", |ui| {
                    let mut v = s.history_limit as f64;
                    widgets::value_box(ui, &t, &format!("{}", s.history_limit), 60.0);
                    widgets::slider(ui, &t, &mut v, 20.0, 1000.0, 140.0);
                    s.history_limit = (v / 10.0).round() as usize * 10;
                });
                r.row(ui, t!("Verlauf löschen"), "", |ui| {
                    if ui.button(RichText::new(t!("Löschen")).color(t.danger)).clicked() {
                        self.data.history.clear();
                        self.osd(t!("Verlauf gelöscht"));
                    }
                });
                r.section(ui, t!("Speicher"));
                let used = self.cache_used.load(std::sync::atomic::Ordering::Relaxed);
                let desc = if used > 0 {
                    tf!("Vorschaubilder, Song-Analysen und Cover. Belegt: {} MB.", (used + 524_288) / 1_048_576)
                } else {
                    t!("Vorschaubilder, Song-Analysen und Cover.").to_string()
                };
                r.row(ui, t!("Cache-Größe höchstens"), &desc, |ui| {
                    let before = s.cache_limit_mb;
                    widgets::segmented(ui, &t, &mut s.cache_limit_mb, &[(256, "256 MB"), (1024, "1 GB"), (4096, "4 GB"), (16384, "16 GB")]);
                    if s.cache_limit_mb != before {
                        crate::cache::trim_in_background(s.cache_limit_mb, self.cache_used.clone());
                    }
                });
                r.row(ui, t!("Cache leeren"), "", |ui| {
                    if ui.button(t!("Leeren")).clicked() {
                        for d in ["thumbs", "analysis"] {
                            let _ = std::fs::remove_dir_all(config::cache_dir().join(d));
                        }
                        self.thumbs.clear();
                        crate::cache::trim_in_background(s.cache_limit_mb, self.cache_used.clone());
                        self.osd(t!("Cache geleert"));
                    }
                });
            }
            SettingsPage::Controls => {
                r.section(ui, t!("Maus"));
                r.row(ui, t!("Mausrad über dem Video"), "", |ui| {
                    widgets::segmented(ui, &t, &mut s.wheel_action, &[(WheelAction::Volume, t!("Lautstärke")), (WheelAction::Seek, t!("Spulen")), (WheelAction::Nothing, t!("Nichts"))]);
                });
                r.row(ui, t!("Klick auf das Video pausiert"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.click_to_pause);
                });
                r.row(ui, t!("Doppelklick für Vollbild"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.double_click_fullscreen);
                });
                r.section(ui, t!("Tastatur"));
                r.row(ui, t!("Tastenkürzel"), t!("Alle Kürzel ansehen und neu belegen."), |ui| {
                    if ui.button(t!("Bearbeiten…")).clicked() {
                        self.dialog = Some(Dialog::Hotkeys);
                    }
                });
                r.row(ui, t!("Kurzer Sprung"), "", |ui| {
                    widgets::value_box(ui, &t, &format!("{:.0} s", s.seek_small), 56.0);
                    widgets::slider(ui, &t, &mut s.seek_small, 1.0, 30.0, 140.0);
                    s.seek_small = s.seek_small.round();
                });
                r.row(ui, t!("Langer Sprung"), "", |ui| {
                    widgets::value_box(ui, &t, &format!("{:.0} s", s.seek_large), 56.0);
                    widgets::slider(ui, &t, &mut s.seek_large, 10.0, 300.0, 140.0);
                    s.seek_large = (s.seek_large / 5.0).round() * 5.0;
                });
                r.row(ui, t!("Lautstärkeschritt"), "", |ui| {
                    widgets::value_box(ui, &t, &format!("{:.0} %", s.volume_step), 56.0);
                    widgets::slider(ui, &t, &mut s.volume_step, 1.0, 20.0, 140.0);
                    s.volume_step = s.volume_step.round();
                });
            }
            SettingsPage::Playback => {
                r.section(ui, "Start");
                r.row(ui, t!("Dort weitermachen, wo du aufgehört hast"), t!("Fragen zeigt einen Hinweis direkt neben den Steuerelementen."), |ui| {
                    widgets::segmented(ui, &t, &mut s.resume, &[(ResumeMode::Ask, t!("Fragen")), (ResumeMode::Always, t!("Immer")), (ResumeMode::Never, t!("Nie"))]);
                });
                r.row(ui, t!("Sofort abspielen, wenn eine Datei geöffnet wird"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.autoplay_on_open);
                });
                r.row(ui, t!("Restliche Dateien im Ordner einreihen"), t!("Öffnest du eine Folge, kommen die nächsten automatisch in die Warteschlange."), |ui| {
                    widgets::toggle(ui, &t, &mut s.play_folder_on_open);
                });
                r.row(ui, t!("Standardgeschwindigkeit"), "", |ui| {
                    widgets::value_box(ui, &t, &format!("{:.2}×", s.default_speed), 60.0);
                    widgets::slider_ex(ui, &t, &mut s.default_speed, 0.25, 2.0, 160.0, Some(1.0), &[1.0]);
                    s.default_speed = (s.default_speed * 20.0).round() / 20.0;
                });
                r.section(ui, t!("Reihenfolge"));
                r.row(ui, t!("Wiederholen"), "", |ui| {
                    widgets::segmented(ui, &t, &mut s.repeat, &[(RepeatMode::Off, t!("Aus")), (RepeatMode::All, t!("Alle")), (RepeatMode::One, t!("Eine"))]);
                });
                if crate::mobile::touch() {
                    r.section(ui, t!("Gesten"));
                    r.row(ui, t!("Wischgesten"), t!("Im Player links senkrecht wischen ändert die Lautstärke, rechts die Helligkeit."), |ui| {
                        widgets::toggle(ui, &t, &mut s.touch_gestures);
                    });
                    r.row(ui, t!("Einführung erneut zeigen"), "", |ui| {
                        if ui.button(t!("Zeigen")).clicked() {
                            s.gesture_intro_seen = false;
                            s.touch_gestures = true;
                        }
                    });
                    r.section(ui, t!("Player-Leiste"));
                    if r.filter.is_empty() {
                        touch_controls_preview(ui, &t, &s.touch_controls);
                        ui.add_space(4.0);
                    }
                    let mut open_editor = false;
                    r.row(ui, t!("Player-Leiste bearbeiten"), t!("Öffnet den Player: Elemente per Ziehen links oder rechts unter der Zeitleiste platzieren."), |ui| {
                        if widgets::accent_button(ui, &t, t!("Bearbeiten")).clicked() {
                            open_editor = true;
                        }
                    });
                    if open_editor {
                        // Save what was changed on this page so far first.
                        self.data.settings = s.clone();
                        self.open_touch_editor(ui.ctx());
                    }
                }
                r.section(ui, t!("Spulen"));
                r.row(ui, t!("Exaktes Spulen"), t!("Genauer, aber bei manchen Dateien etwas langsamer."), |ui| {
                    widgets::toggle(ui, &t, &mut s.exact_seek);
                });
                r.row(ui, t!("An Kapiteln einrasten"), t!("Beim Ziehen der Zeitleiste rastet die Position an Kapitelanfängen ein."), |ui| {
                    widgets::toggle(ui, &t, &mut s.chapter_snap);
                });
                if s.chapter_snap {
                    r.row(ui, t!("Einrastbereich"), t!("Wie nah du einem Kapitel kommen musst, damit es einrastet."), |ui| {
                        widgets::value_box(ui, &t, &format!("{:.0} px", s.chapter_snap_px), 56.0);
                        let mut v = s.chapter_snap_px as f64;
                        widgets::slider_ex(ui, &t, &mut v, 4.0, 30.0, 140.0, Some(10.0), &[10.0]);
                        s.chapter_snap_px = v.round() as f32;
                    });
                    r.row(ui, t!("Einrasten ignorieren nach"), t!("Rastet dieselbe Stelle so oft ein, willst du offenbar knapp daneben: dann rastet sie in dieser Datei nicht mehr ein. 0 = nie ignorieren."), |ui| {
                        let label = if s.chapter_snap_limit == 0 { t!("nie").to_string() } else { format!("{}×", s.chapter_snap_limit) };
                        widgets::value_box(ui, &t, &label, 56.0);
                        let mut v = s.chapter_snap_limit as f64;
                        widgets::slider_ex(ui, &t, &mut v, 0.0, 10.0, 140.0, Some(3.0), &[3.0]);
                        s.chapter_snap_limit = v.round() as u32;
                    });
                }
            }
            SettingsPage::Video => {
                r.section(ui, t!("Dekodierung"));
                r.row(ui, t!("Hardware-Dekodierung"), t!("Nutzt die Grafikkarte zum Dekodieren (spart CPU und Akku)."), |ui| {
                    widgets::toggle(ui, &t, &mut s.hwdec);
                });
                r.row(ui, "Deinterlacing", t!("Für Halbbild-Material (TV-Aufnahmen, DVDs)."), |ui| {
                    widgets::toggle(ui, &t, &mut s.deinterlace);
                });
                r.row(ui, t!("Seitenverhältnis beibehalten"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.keep_aspect);
                });
                r.section(ui, t!("Erweitert"));
                r.row(ui, t!("An Bildschirm-Bildrate anpassen"), t!("Flüssigere Bewegungen durch Resampling (display-resample)."), |ui| {
                    widgets::toggle(ui, &t, &mut s.video_sync_display);
                });
                #[cfg(windows)]
                r.row(ui, t!("HDR-Ausgabe"), t!("HDR-Videos werden in HDR gezeigt, wenn in Windows „HDR verwenden“ an ist. Aus: immer in SDR umgerechnet."), |ui| {
                    widgets::toggle(ui, &t, &mut s.hdr_output);
                });
            }
            SettingsPage::Audio => {
                r.section(ui, t!("Lautstärke"));
                r.row(ui, t!("Lautstärke merken"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.remember_volume);
                });
                r.row(ui, t!("Standardlautstärke"), "", |ui| {
                    widgets::value_box(ui, &t, &format!("{:.0} %", s.default_volume), 60.0);
                    widgets::slider(ui, &t, &mut s.default_volume, 0.0, 100.0, 140.0);
                    s.default_volume = s.default_volume.round();
                });
                r.row(ui, t!("Maximale Lautstärke"), t!("Erlaubt Verstärkung über 100 %."), |ui| {
                    widgets::value_box(ui, &t, &format!("{:.0} %", s.max_volume), 60.0);
                    widgets::slider(ui, &t, &mut s.max_volume, 100.0, 200.0, 140.0);
                    s.max_volume = (s.max_volume / 10.0).round() * 10.0;
                });
                r.row(ui, t!("Lautstärke normalisieren"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.normalize);
                });
                r.section(ui, t!("Spuren"));
                r.row(ui, t!("Bevorzugte Audiosprache"), t!("Sprachcodes, z. B. „de,en“."), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.alang).desired_width(120.0).hint_text("de,en"));
                });
                r.row(ui, t!("Kanäle"), "", |ui| {
                    let opts = [("auto-safe", t!("Automatisch")), ("stereo", "Stereo"), ("5.1", "5.1"), ("7.1", "7.1"), ("mono", "Mono")];
                    let cur = opts.iter().find(|x| x.0 == s.audio_channels).map(|x| x.1).unwrap_or(t!("Automatisch"));
                    egui::ComboBox::from_id_salt("set_ch").selected_text(cur).show_ui(ui, |ui| {
                        for (v, l) in opts {
                            ui.selectable_value(&mut s.audio_channels, v.to_string(), l);
                        }
                    });
                });
            }
            SettingsPage::Subtitles => {
                r.section(ui, t!("Laden"));
                r.row(ui, t!("Untertitel automatisch laden"), t!("Welche Untertiteldateien im Ordner des Videos automatisch geladen werden."), |ui| {
                    egui::ComboBox::from_id_salt("set_subauto").selected_text(s.sub_auto.label()).width(230.0).show_ui(ui, |ui| {
                        for a in SubAuto::ALL {
                            ui.selectable_value(&mut s.sub_auto, a, a.label());
                        }
                    });
                });
                r.row(ui, t!("Bevorzugte Untertitelsprache"), t!("Sprachcodes, z. B. „de,en“."), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.slang).desired_width(120.0).hint_text("de,en"));
                });
                r.section(ui, t!("Darstellung"));
                r.row(ui, t!("Schriftart"), "", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.sub_font).desired_width(160.0));
                });
                r.row(ui, t!("Schriftgröße"), t!("Exakter Wert in Pixeln (bezogen auf 720p)."), |ui| {
                    widgets::value_box(ui, &t, &format!("{:.0} px", s.sub_font_size), 64.0);
                    widgets::slider(ui, &t, &mut s.sub_font_size, 16.0, 110.0, 140.0);
                    s.sub_font_size = s.sub_font_size.round();
                });
                r.row(ui, t!("Umrissstärke"), "", |ui| {
                    widgets::value_box(ui, &t, &format!("{:.1} px", s.sub_border_size), 64.0);
                    widgets::slider(ui, &t, &mut s.sub_border_size, 0.0, 8.0, 140.0);
                });
                r.row(ui, t!("Hintergrund-Deckkraft"), "", |ui| {
                    widgets::value_box(ui, &t, &format!("{:.0} %", s.sub_back_opacity * 100.0), 64.0);
                    widgets::slider(ui, &t, &mut s.sub_back_opacity, 0.0, 1.0, 140.0);
                });
                r.row(ui, t!("Fett"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.sub_bold);
                });
            }
            SettingsPage::Library => {
                if crate::platform::MOBILE {
                    let st = self.storage;
                    r.section(ui, t!("Speicherzugriff"));
                    r.row(ui, t!("Videos und Musik"), t!("Ohne diese Berechtigung bleibt die Bibliothek leer."), |ui| {
                        if st.media() {
                            ui.label(RichText::new(t!("erlaubt")).color(t.text_dim));
                        } else if ui.button(t!("Erlauben…")).clicked() {
                            crate::pick::request_storage();
                        }
                    });
                    r.row(ui, t!("Zugriff auf alle Dateien"), t!("Nur für Playlists (.m3u) und seltene Untertitelformate neben den Videos – .srt-Untertitel gehen auch ohne. Öffnet die Android-Einstellungen."), |ui| {
                        if st.all_files {
                            ui.label(RichText::new(t!("erlaubt")).color(t.text_dim));
                        } else if ui.button(t!("Einstellungen öffnen…")).clicked() {
                            crate::pick::request_all_files();
                        }
                    });
                }
                r.section(ui, t!("Ordner"));
                if r.filter.is_empty() || t!("videoordner musikordner").contains(&r.filter.to_lowercase()) {
                    for (music, (label, list)) in [(t!("Videoordner"), &mut s.video_folders), (t!("Musikordner"), &mut s.music_folders)].into_iter().enumerate().map(|(i, x)| (i == 1, x)) {
                        ui.label(RichText::new(label).color(t.text_dim));
                        let mut remove = None;
                        for (i, f) in list.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(ph::FOLDER_SIMPLE).color(t.accent));
                                ui.label(f.to_string_lossy());
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if icon_button(ui, &t, ph::TRASH, t!("Entfernen"), false).clicked() {
                                        remove = Some(i);
                                    }
                                });
                            });
                        }
                        if let Some(i) = remove {
                            list.remove(i);
                        }
                        if ui.button(tf!("{}  Ordner hinzufügen…", ph::PLUS)).clicked() {
                            self.pick(Pick::LibraryFolder { music, show: false });
                            // Desktop answers right away: keep this page's copy in step.
                            *list = if music { self.data.settings.music_folders.clone() } else { self.data.settings.video_folders.clone() };
                        }
                        ui.add_space(8.0);
                    }
                }
                r.row(ui, t!("Musikbibliothek beim Start durchsuchen"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.scan_on_start);
                });
                r.row(ui, t!("Versteckte Dateien anzeigen"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.show_hidden);
                });
                r.row(ui, t!("Fehlende Medien"), t!("Abgespielte Dateien, die verschoben oder gelöscht wurden: aus Verlauf, Playlists und Anheftungen entfernen oder behalten."), |ui| {
                    widgets::segmented(ui, &t, &mut s.missing_media, &[(MissingMedia::Ask, t!("Fragen")), (MissingMedia::Remove, t!("Entfernen")), (MissingMedia::Keep, t!("Behalten"))]);
                });
            }
            SettingsPage::Plex => {
                r.section(ui, t!("Plex-Konto"));
                let signed_in = self.plex.plex_signed_in();
                let user = self.plex.user.as_ref().map(|u| u.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| self.data.plex.user_name.clone());
                let status = match &self.plex.status {
                    crate::plex::Status::Ready => t!("Verbunden").to_string(),
                    crate::plex::Status::Connecting(m) => m.clone(),
                    crate::plex::Status::Error(e) => e.clone(),
                    crate::plex::Status::Pin { .. } => t!("Warte auf Bestätigung im Browser…").into(),
                    crate::plex::Status::SignedOut => t!("Nicht angemeldet").into(),
                };
                let status = if self.plex.backend().media_browser() && signed_in { t!("Angemeldet").to_string() } else { status };
                let mut sign_out = false;
                let mut sign_in = false;
                r.row(ui, if signed_in && !user.is_empty() { &user } else { t!("Plex-Konto") }, &status, |ui| {
                    if signed_in {
                        if ui.button(t!("Abmelden")).clicked() {
                            sign_out = true;
                        }
                    } else if ui.button(t!("Anmelden…")).clicked() {
                        sign_in = true;
                    }
                });
                if sign_out {
                    self.plex.sign_out(&mut self.data.plex);
                    self.mark_dirty();
                }
                if sign_in {
                    self.px.plex_connect = false;
                    self.px.mb_form = None;
                    self.plex.start_login();
                    self.lib.go(crate::library::LibLoc::Plex(crate::library::PlexLoc::Home));
                    self.view = crate::app::View::Library;
                    self.dialog = None;
                }
                r.section(ui, "Jellyfin & Emby");
                let mut remove = None;
                for sv in self.plex.servers.iter().filter(|x| x.backend.media_browser()) {
                    let base = sv.connections.first().map(|c| c.uri.clone()).unwrap_or_default();
                    let sub = format!("{} · {} · {}", sv.backend.name(), base, self.data.plex.media_servers.iter().find(|m| m.id == sv.id).map(|m| m.user_name.clone()).unwrap_or_default());
                    r.row(ui, &sv.name, &sub, |ui| {
                        if ui.button(t!("Entfernen")).clicked() {
                            remove = Some(sv.id.clone());
                        }
                    });
                }
                if let Some(id) = remove {
                    self.plex.remove_media_server(&id, &mut self.data.plex);
                    self.mark_dirty();
                }
                let mut add = false;
                r.row(ui, t!("Server hinzufügen"), t!("Mit Adresse, Benutzername und Passwort. Velouna speichert nur einen Zugangsschlüssel."), |ui| {
                    if ui.button(t!("Hinzufügen…")).clicked() {
                        add = true;
                    }
                });
                if add {
                    self.px.mb_form = Some(crate::ui_plex::MbForm::default());
                    self.lib.go(crate::library::LibLoc::Plex(crate::library::PlexLoc::Home));
                    self.view = crate::app::View::Library;
                    self.dialog = None;
                }
                if self.plex.servers.len() > 1 || (signed_in && !self.plex.servers.is_empty()) {
                    r.section(ui, t!("Anzeige"));
                    let servers = self.plex.servers.clone();
                    let cur = self.plex.server.as_ref().map(|s| s.id.clone()).or(self.data.plex.server_id.clone()).unwrap_or_default();
                    let cur_name = servers.iter().find(|s| s.id == cur).map(|s| s.name.clone()).unwrap_or_else(|| t!("Auswählen").into());
                    let mut pick: Option<String> = None;
                    r.row(ui, "Server", t!("Der Server, dessen Mediatheken Velouna zeigt."), |ui| {
                        egui::ComboBox::from_id_salt("plex-server").selected_text(cur_name).width(220.0).show_ui(ui, |ui| {
                            for sv in &servers {
                                let label = if sv.backend.media_browser() { format!("{} ({})", sv.name, sv.backend.name()) } else if sv.owned { sv.name.clone() } else { tf!("{} (geteilt von {})", sv.name, sv.owner) };
                                if ui.selectable_label(sv.id == cur, label).clicked() && sv.id != cur {
                                    pick = Some(sv.id.clone());
                                }
                            }
                        });
                    });
                    if let Some(id) = pick {
                        self.plex.select_server(&id, s.plex_local_only);
                    }
                    if let Some(sv) = &self.plex.server {
                        let kind = if sv.backend.media_browser() {
                            if sv.base.starts_with("https://") { t!("Direkt (verschlüsselt)") } else { t!("Direkt") }
                        } else if sv.base.contains("plex.direct") && !sv.base.contains("://192-168") && !sv.base.contains("://10-") {
                            t!("Remote (verschlüsselt)")
                        } else if sv.base.starts_with("http://") {
                            t!("Lokales Netzwerk")
                        } else {
                            t!("Lokales Netzwerk (verschlüsselt)")
                        };
                        r.row(ui, t!("Verbindung"), "", |ui| {
                            ui.label(RichText::new(kind).color(t.text_dim));
                        });
                    }
                }
                r.section(ui, t!("Darstellung"));
                r.row(ui, "Layout", t!("Seitenleiste: Highlights und Infos links, Inhalte rechts – weniger Scrollen. Banner: großes Bild oben."), |ui| {
                    widgets::segmented(ui, &t, &mut s.plex_side_layout, &[(true, t!("Seitenleiste")), (false, t!("Banner oben"))]);
                });
                r.section(ui, t!("Verhalten"));
                r.row(ui, t!("Fortschritt mit dem Server synchronisieren"), t!("Wiedergabeposition und „gesehen“ werden an den Server gemeldet – wie bei den offiziellen Apps."), |ui| {
                    widgets::toggle(ui, &t, &mut s.plex_sync_progress);
                });
                r.row(ui, t!("„Weiterschauen“ vom Server auf der Startseite"), "", |ui| {
                    widgets::toggle(ui, &t, &mut s.plex_on_home);
                });
                r.row(ui, t!("Plex: nur lokale Verbindungen"), t!("Verbindet sich nie über das Internet oder das Plex-Relay."), |ui| {
                    widgets::toggle(ui, &t, &mut s.plex_local_only);
                });
                r.row(ui, t!("Zwischenspeicher leeren"), t!("Lädt Mediatheken und Bilder neu vom Server."), |ui| {
                    if ui.button(t!("Leeren")).clicked() {
                        self.plex.clear_cache();
                        self.plex.images.clear();
                        let _ = std::fs::remove_dir_all(crate::config::cache_dir().join("plex_img"));
                    }
                });
            }
            SettingsPage::Network => {
                r.section(ui, "Streaming");
                // Phones have no yt-dlp to call.
                if !crate::platform::MOBILE {
                    r.row(ui, t!("yt-dlp für Webseiten verwenden"), t!("Erlaubt das Abspielen von Links zu Videoportalen, wenn yt-dlp installiert ist."), |ui| {
                        widgets::toggle(ui, &t, &mut s.ytdl);
                    });
                }
                r.row(ui, t!("Puffer (Sekunden)"), "", |ui| {
                    widgets::value_box(ui, &t, &format!("{:.0} s", s.cache_secs), 60.0);
                    widgets::slider(ui, &t, &mut s.cache_secs, 5.0, 600.0, 140.0);
                    s.cache_secs = s.cache_secs.round();
                });
                r.section(ui, t!("Erweitert"));
                r.row(ui, "User-Agent", t!("Leer = Standard von mpv."), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.user_agent).desired_width(200.0));
                });
                r.row(ui, t!("Zeitlimit"), "", |ui| {
                    widgets::value_box(ui, &t, &format!("{:.0} s", s.network_timeout), 60.0);
                    widgets::slider(ui, &t, &mut s.network_timeout, 5.0, 120.0, 140.0);
                    s.network_timeout = s.network_timeout.round();
                });
            }
            SettingsPage::Performance => {
                r.section(ui, t!("Speicher"));
                r.row(ui, t!("Vorauslese-Puffer"), t!("Maximaler Speicher für das Vorauslesen von Dateien und Streams."), |ui| {
                    let mut v = s.demuxer_max_mb as f64;
                    widgets::value_box(ui, &t, &format!("{} MiB", s.demuxer_max_mb), 76.0);
                    widgets::slider(ui, &t, &mut v, 32.0, 2048.0, 140.0);
                    s.demuxer_max_mb = ((v / 16.0).round() * 16.0) as u32;
                });
                r.section(ui, t!("Dekodierung"));
                if let Some(p) = &self.player {
                    let hw = p.st.hwdec.clone();
                    r.row(ui, t!("Aktueller Decoder"), "", |ui| {
                        ui.label(RichText::new(if hw.is_empty() || hw == "no" { "Software".to_string() } else { hw }).color(t.text_dim));
                    });
                }
            }
            SettingsPage::Recording => {
                r.section(ui, "Screenshots");
                r.row(ui, t!("Screenshot-Ordner"), "", |ui| {
                    if ui.button(t!("Ändern…")).clicked() {
                        self.pick(Pick::ScreenshotDir);
                        s.screenshot_dir = self.data.settings.screenshot_dir.clone();
                    }
                    ui.label(RichText::new(s.screenshot_dir.to_string_lossy()).size(12.0).color(t.text_dim));
                });
                r.row(ui, "Format", "", |ui| {
                    egui::ComboBox::from_id_salt("shot_fmt").selected_text(s.screenshot_format.to_uppercase()).show_ui(ui, |ui| { for f in ["png", "jpg", "webp"] { ui.selectable_value(&mut s.screenshot_format, f.to_string(), f.to_uppercase()); } });
                });
                // Phones have no record button and no FFmpeg to convert with.
                if !crate::platform::MOBILE {
                    r.section(ui, t!("Aufnahmen"));
                    r.row(ui, t!("Aufnahme-Ordner"), t!("Ziel für „Stream aufnehmen“."), |ui| {
                        if ui.button(t!("Ändern…")).clicked() {
                            self.pick(Pick::RecordDir);
                            s.record_dir = self.data.settings.record_dir.clone();
                        }
                        ui.label(RichText::new(s.record_dir.to_string_lossy()).size(12.0).color(t.text_dim));
                    });
                    r.row(ui, t!("Konvertieren / Speichern"), t!("Dateien in andere Formate umwandeln."), |ui| {
                        if ui.button(t!("Öffnen…")).clicked() {
                            self.dlg.convert_in = self.current_path();
                            self.dialog = Some(Dialog::Convert);
                        }
                    });
                }
            }
            SettingsPage::Extensions => {
                r.section(ui, t!("Plugins"));
                self.extensions_page(ui, r.filter);
                ui.add_space(8.0);
                r.section(ui, t!("Komponenten"));
                let mpv = self.player.as_ref().map(|p| {
                    let (a, b) = p.mpv.api_version();
                    (p.mpv.api.path.to_string_lossy().to_string(), tf!("Client-API {a}.{b}"), p.mpv.get_string("mpv-version").unwrap_or_default())
                });
                r.row(ui, t!("Wiedergabe-Engine (libmpv)"), "", |ui| {
                    match &mpv {
                        Some((path, api, ver)) => {
                            ui.label(RichText::new(format!("{ver} · {api}")).color(t.text_dim)).on_hover_text(path.clone());
                        }
                        None => {
                            ui.label(RichText::new(t!("nicht geladen")).color(t.danger));
                        }
                    }
                });
                // Phones have no FFmpeg programs (thumbnails come from mpv there).
                if !crate::platform::MOBILE {
                    // Bundled next to velouna.exe (installer) or found on PATH; the full
                    // path is in the tooltip.
                    let describe = |p: &std::path::PathBuf| {
                        let bundled = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())).is_some_and(|d| p.parent() == Some(d.as_path()));
                        if bundled { t!("mitgeliefert").to_string() } else { t!("aus PATH").to_string() }
                    };
                    let ff_path = self.tools.ffmpeg.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                    let fp_path = self.tools.ffprobe.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                    let ff = self.tools.ffmpeg.as_ref().map(describe);
                    r.row(ui, t!("FFmpeg (Vorschaubilder, Wellenform, Konvertieren)"), "", |ui| {
                        ui.label(RichText::new(ff.clone().unwrap_or(t!("nicht gefunden").into())).size(12.0).color(if ff.is_some() { t.text_dim } else { t.danger })).on_hover_text(&ff_path);
                    });
                    let fp = self.tools.ffprobe.as_ref().map(describe);
                    r.row(ui, t!("FFprobe (Metadaten)"), "", |ui| {
                        ui.label(RichText::new(fp.clone().unwrap_or(t!("nicht gefunden").into())).size(12.0).color(if fp.is_some() { t.text_dim } else { t.danger })).on_hover_text(&fp_path);
                    });
                }
            }
        }
    }

    // =========================================================== hotkeys

    fn dlg_hotkeys(&mut self, ui: &mut Ui) {
        let t = self.t;
        dlg_width(ui, 620.0);
        if self.dlg_title(ui, t!("Tastenkürzel")) {
            return;
        }
        // capture
        if let Some(id) = self.rebinding {
            let ev = ui.ctx().input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, m)) = ev {
                if key == egui::Key::Escape && !m.any() {
                    self.rebinding = None;
                } else if key == egui::Key::Backspace && !m.any() {
                    self.data.hotkeys.insert(id.to_string(), String::new());
                    self.rebinding = None;
                } else {
                    let combo = Combo::from_event(key, m);
                    // remove conflicts
                    if let Some(other) = self.bindings.find(combo) {
                        let oid = hotkeys::info(other).id;
                        if oid != id {
                            self.data.hotkeys.insert(oid.to_string(), String::new());
                            self.osd(tf!("„{}“ hatte diese Taste – jetzt ohne Kürzel", crate::i18n::tr(hotkeys::info(other).name)));
                        }
                    }
                    self.data.hotkeys.insert(id.to_string(), combo.to_string());
                    self.rebinding = None;
                }
                self.bindings = Bindings::from_overrides(&self.data.hotkeys);
                self.mark_dirty();
            }
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.dlg.hotkey_search).hint_text(tf!("{}  Aktion suchen", ph::MAGNIFYING_GLASS)).desired_width(300.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(t!("Alle zurücksetzen")).clicked() {
                    self.data.hotkeys.clear();
                    self.bindings = Bindings::from_overrides(&self.data.hotkeys);
                    self.mark_dirty();
                }
            });
        });
        ui.label(RichText::new(t!("Klicke auf ein Kürzel und drücke die neue Tastenkombination. Esc bricht ab, ⌫ entfernt das Kürzel.")).size(12.0).color(t.text_faint));
        ui.add_space(6.0);
        let q = self.dlg.hotkey_search.to_lowercase();
        egui::ScrollArea::vertical().max_height(440.0).show(ui, |ui| {
            let mut group = "";
            for a in ACTIONS {
                if !q.is_empty() && !crate::i18n::tr(a.name).to_lowercase().contains(&q) {
                    continue;
                }
                if a.group != group {
                    group = a.group;
                    ui.add_space(8.0);
                    ui.label(RichText::new(crate::i18n::tr(group)).font(theme::semibold(13.5)).color(t.text));
                }
                ui.horizontal(|ui| {
                    ui.set_min_height(30.0);
                    ui.label(RichText::new(crate::i18n::tr(a.name)).color(t.text_dim));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let overridden = self.data.hotkeys.contains_key(a.id);
                        if overridden && icon_button(ui, &t, ph::ARROW_COUNTER_CLOCKWISE, t!("Standard wiederherstellen"), false).clicked() {
                            self.data.hotkeys.remove(a.id);
                            self.bindings = Bindings::from_overrides(&self.data.hotkeys);
                            self.mark_dirty();
                        }
                        let capturing = self.rebinding == Some(a.id);
                        let label = if capturing {
                            t!("Taste drücken…").to_string()
                        } else {
                            self.bindings.combo(a.action).map(|c| c.chips().join(" + ")).unwrap_or_else(|| "—".into())
                        };
                        let b = egui::Button::new(RichText::new(label).color(if capturing { t.accent } else { t.text })).min_size(vec2(150.0, 26.0));
                        if ui.add(b).clicked() {
                            self.rebinding = Some(a.id);
                        }
                    });
                });
            }
        });
    }

    // ========================================================= customize

    fn dlg_customize(&mut self, ui: &mut Ui) {
        let t = self.t;
        dlg_width(ui, 760.0);
        if self.dlg_title(ui, t!("Oberfläche anpassen")) {
            return;
        }
        ui.label(RichText::new(t!("Ziehe Schaltflächen in die Steuerleiste, sortiere sie um oder ziehe sie zurück in die Ablage. Die Suchleiste füllt immer den Platz dazwischen.")).color(t.text_dim));
        ui.add_space(10.0);
        let mut layout = self.data.settings.layout.clone();
        let mut drop: Option<(ControlDrag, Zone, Option<usize>)> = None;

        let chip = |ui: &mut Ui, item: ControlItem, zone: Zone, idx: usize, drop: &mut Option<(ControlDrag, Zone, Option<usize>)>| {
            let id = Id::new(("ctl_chip", zone as u8, idx, item as u8));
            let resp = ui
                .dnd_drag_source(id, ControlDrag { item, from: zone }, |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(if zone == Zone::Palette { 150.0 } else { 40.0 }, 36.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(theme::R), t.elevated);
                    ui.painter().rect_stroke(r, CornerRadius::same(theme::R), Stroke::new(1.0, t.border), StrokeKind::Inside);
                    if zone == Zone::Palette {
                        ui.painter().text(pos2(r.left() + 18.0, r.center().y), Align2::CENTER_CENTER, item_icon(item), theme::icon(16.0), t.accent);
                        ui.painter().text(pos2(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, item.label(), FontId::proportional(12.5), t.text);
                    } else {
                        let col = if item == ControlItem::Play { t.accent } else { t.text };
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, item_icon(item), theme::icon(17.0), col);
                    }
                })
                .response
                .on_hover_text(item.label());
            if let Some(p) = resp.dnd_release_payload::<ControlDrag>() {
                *drop = Some((*p, zone, Some(idx)));
            }
            if resp.dnd_hover_payload::<ControlDrag>().is_some() {
                ui.painter().line_segment([resp.rect.left_top() - vec2(3.0, 0.0), resp.rect.left_bottom() - vec2(3.0, 0.0)], Stroke::new(2.0, t.accent));
            }
        };

        // Preview bar
        egui::Frame::new().fill(t.panel).corner_radius(CornerRadius::same(theme::R_LG)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // A full bar is wider than a narrow window: scroll instead of
            // growing the dialog past the screen edge.
            egui::ScrollArea::horizontal().id_salt("ctl_preview").show(ui, |ui| ui.horizontal(|ui| {
                let (resp_l, p_l) = ui.dnd_drop_zone::<ControlDrag, ()>(egui::Frame::new().inner_margin(egui::Margin::same(4)), |ui| {
                    ui.set_min_size(vec2(60.0, 40.0));
                    ui.horizontal(|ui| {
                        for (i, it) in layout.left.iter().enumerate() {
                            chip(ui, *it, Zone::Left, i, &mut drop);
                        }
                    });
                });
                let _ = resp_l;
                if let Some(p) = p_l {
                    if drop.is_none() {
                        drop = Some((*p, Zone::Left, None));
                    }
                }
                let w = (ui.available_width() - 260.0).max(80.0);
                let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
                let track = Rect::from_center_size(r.center(), vec2(w - 10.0, 14.0));
                ui.painter().rect_filled(track, CornerRadius::same(3), t.track);
                ui.painter().rect_filled(Rect::from_min_size(track.min, vec2(track.width() * 0.35, track.height())), CornerRadius::same(3), t.accent_dim);
                let (_, p_r) = ui.dnd_drop_zone::<ControlDrag, ()>(egui::Frame::new().inner_margin(egui::Margin::same(4)), |ui| {
                    ui.set_min_size(vec2(60.0, 40.0));
                    ui.horizontal(|ui| {
                        for (i, it) in layout.right.iter().enumerate() {
                            chip(ui, *it, Zone::Right, i, &mut drop);
                        }
                    });
                });
                if let Some(p) = p_r {
                    if drop.is_none() {
                        drop = Some((*p, Zone::Right, None));
                    }
                }
            }));
        });
        ui.add_space(12.0);
        ui.label(RichText::new(t!("Ablage")).font(theme::semibold(13.5)));
        let unused: Vec<ControlItem> = ControlItem::ALL.iter().copied().filter(|i| !layout.left.contains(i) && !layout.right.contains(i)).collect();
        let (_, p_pal) = ui.dnd_drop_zone::<ControlDrag, ()>(egui::Frame::new().fill(t.panel).corner_radius(CornerRadius::same(theme::R_LG)).inner_margin(egui::Margin::same(10)), |ui| {
            ui.set_min_size(vec2(ui.available_width(), 60.0));
            ui.horizontal_wrapped(|ui| {
                for (i, it) in unused.iter().enumerate() {
                    chip(ui, *it, Zone::Palette, i, &mut drop);
                }
                if unused.is_empty() {
                    ui.label(RichText::new(t!("Alle Schaltflächen sind in Benutzung.")).color(t.text_faint));
                }
            });
        });
        if let Some(p) = p_pal {
            if drop.is_none() {
                drop = Some((*p, Zone::Palette, None));
            }
        }
        if let Some((d, to, idx)) = drop {
            layout.left.retain(|x| *x != d.item);
            layout.right.retain(|x| *x != d.item);
            match to {
                Zone::Left => {
                    let i = idx.unwrap_or(layout.left.len()).min(layout.left.len());
                    layout.left.insert(i, d.item);
                }
                Zone::Right => {
                    let i = idx.unwrap_or(layout.right.len()).min(layout.right.len());
                    layout.right.insert(i, d.item);
                }
                Zone::Palette => {}
            }
            let _ = d.from;
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(t!("Vorlagen:"));
            if ui.button(t!("Standard")).clicked() {
                layout = ControlLayout::default();
            }
            if ui.button("Minimal").clicked() {
                layout = ControlLayout::minimal();
            }
            if ui.button(t!("Profi")).clicked() {
                layout = ControlLayout::power();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::accent_button(ui, &t, t!("Fertig")).clicked() {
                    self.dialog = None;
                }
            });
        });
        if layout != self.data.settings.layout {
            self.data.settings.layout = layout;
            self.mark_dirty();
        }
    }

    // ============================================================ network

    fn dlg_network(&mut self, ui: &mut Ui) {
        let t = self.t;
        dlg_width(ui, 520.0);
        if self.dlg_title(ui, t!("Netzwerkstream öffnen")) {
            return;
        }
        ui.label(RichText::new(t!("URL (http, https, rtsp, rtmp, udp, m3u-Listen, YouTube mit yt-dlp …)")).color(t.text_dim));
        let r = ui.add(egui::TextEdit::singleline(&mut self.dlg.url).hint_text("https://…").desired_width(f32::INFINITY));
        if self.dlg.url.is_empty() {
            r.request_focus();
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Name (optional)").color(t.text_dim));
        ui.add(egui::TextEdit::singleline(&mut self.dlg.url_name).desired_width(f32::INFINITY));
        ui.add_space(6.0);
        ui.checkbox(&mut self.dlg.url_save, t!("In der Bibliothek unter „Netzwerk“ speichern"));
        ui.add_space(12.0);
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let mut act = 0;
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::accent_button(ui, &t, t!("Abspielen")).clicked() || enter {
                    act = 1;
                }
                if widgets::ghost_button(ui, &t, t!("Zur Warteschlange")).clicked() {
                    act = 2;
                }
            });
        });
        if act > 0 && !self.dlg.url.trim().is_empty() {
            let url = self.dlg.url.trim().to_string();
            if self.dlg.url_save && !self.data.streams.iter().any(|s| s.url == url) {
                let name = if self.dlg.url_name.trim().is_empty() { media::display_name(&url) } else { self.dlg.url_name.trim().to_string() };
                self.data.streams.push(Stream { name, url: url.clone() });
                self.mark_dirty();
            }
            if act == 1 {
                self.play_list(&[url], 0);
            } else {
                self.enqueue(&[url]);
            }
            self.dlg.url.clear();
            self.dlg.url_name.clear();
            self.dialog = None;
        }
    }

    // ============================================================ convert

    fn dlg_convert(&mut self, ui: &mut Ui) {
        let t = self.t;
        dlg_width(ui, 560.0);
        if self.dlg.convert_progress.is_none() && self.dlg_title(ui, t!("Konvertieren / Speichern")) {
            return;
        }
        if self.dlg.convert_progress.is_some() {
            ui.label(RichText::new(t!("Konvertieren / Speichern")).font(theme::semibold(17.0)));
            ui.add_space(8.0);
        }
        if !self.tools.available() {
            ui.label(RichText::new(t!("FFmpeg wurde nicht gefunden. Installiere FFmpeg und füge es zum PATH hinzu.")).color(t.danger));
            return;
        }
        ui.label(RichText::new(t!("Quelle")).color(t.text_dim));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.dlg.convert_in).desired_width(440.0));
            if ui.button(t!("Durchsuchen…")).clicked() {
                if let Some(f) = rfd::FileDialog::new().pick_file() {
                    self.dlg.convert_in = f.to_string_lossy().to_string();
                    self.dlg.convert_out.clear();
                }
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new(t!("Profil")).color(t.text_dim));
        let cur = crate::i18n::tr(CONVERT_PROFILES.get(self.dlg.convert_profile).map(|p| p.0).unwrap_or(""));
        egui::ComboBox::from_id_salt("conv_prof").selected_text(cur).width(400.0).show_ui(ui, |ui| {
            for (i, p) in CONVERT_PROFILES.iter().enumerate() {
                if ui.selectable_label(i == self.dlg.convert_profile, crate::i18n::tr(p.0)).clicked() {
                    self.dlg.convert_profile = i;
                    self.dlg.convert_out.clear();
                }
            }
        });
        let ext = CONVERT_PROFILES.get(self.dlg.convert_profile).map(|p| p.1).unwrap_or("mp4");
        if self.dlg.convert_out.is_empty() && !self.dlg.convert_in.is_empty() && !media::is_url(&self.dlg.convert_in) {
            let p = std::path::Path::new(&self.dlg.convert_in);
            let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(t!("ausgabe").into());
            let out = p.with_file_name(tf!("{stem}_konvertiert.{ext}"));
            self.dlg.convert_out = out.to_string_lossy().to_string();
        }
        ui.add_space(6.0);
        ui.label(RichText::new(t!("Ziel")).color(t.text_dim));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.dlg.convert_out).desired_width(440.0));
            if ui.button(t!("Speichern unter…")).clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter(ext, &[ext]).save_file() {
                    self.dlg.convert_out = f.to_string_lossy().to_string();
                }
            }
        });
        ui.add_space(12.0);
        if let Some((done, total)) = self.dlg.convert_progress {
            let frac = if total > 0.0 { (done / total).clamp(0.0, 1.0) as f32 } else { 0.0 };
            ui.add(egui::ProgressBar::new(frac).text(format!("{} / {}", media::fmt_time(done), media::fmt_time(total))).fill(t.accent_dim));
            ui.add_space(8.0);
            if widgets::ghost_button(ui, &t, t!("Abbrechen")).clicked() {
                self.tools.cancel_convert();
            }
            return;
        }
        if let Some(res) = &self.dlg.convert_result {
            match res {
                Ok(p) => {
                    let p = p.clone();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tf!("{}  Fertig: {}", ph::CHECK_CIRCLE, media::display_name(&p))).color(t.accent));
                        if ui.button(crate::platform::show_in_folder_label()).clicked() {
                            crate::ui_panels::show_in_explorer(&p);
                        }
                    });
                }
                Err(e) => {
                    ui.label(RichText::new(tf!("Fehler: {}", e.lines().last().unwrap_or(e))).color(t.danger));
                }
            }
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::accent_button(ui, &t, t!("Starten")).clicked() && !self.dlg.convert_in.is_empty() && !self.dlg.convert_out.is_empty() {
                    let args: Vec<String> = CONVERT_PROFILES[self.dlg.convert_profile].2.iter().map(|s| s.to_string()).collect();
                    let total = self.tools.probe_cache.get(&self.dlg.convert_in).map(|p| p.duration).or_else(|| {
                        self.player.as_ref().filter(|p| p.st.path == self.dlg.convert_in).map(|p| p.st.duration)
                    });
                    self.dlg.convert_result = None;
                    self.dlg.convert_progress = Some((0.0, total.unwrap_or(0.0)));
                    self.tools.convert(self.dlg.convert_in.clone(), self.dlg.convert_out.clone(), args, total.unwrap_or(0.0));
                }
            });
        });
    }

    // ======================================================== subtitles

    fn dlg_add_subs(&mut self, ui: &mut Ui) {
        let t = self.t;
        dlg_width(ui, 640.0);
        if self.dlg_title(ui, t!("Untertitel hinzufügen")) {
            self.dlg.sub_scope = None;
            self.dlg.sub_matches.clear();
            return;
        }
        let has = self.has_media();
        let folder = self.current_folder();
        if !self.dlg.sub_matches.is_empty() {
            // Matching table
            ui.label(RichText::new(t!("Zuordnung prüfen – Untertitel werden für alle zukünftigen Wiedergaben gespeichert.")).color(t.text_dim));
            ui.add_space(6.0);
            let subs = self.dlg.sub_files.clone();
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                egui::Grid::new("submatch").num_columns(2).spacing([16.0, 6.0]).striped(true).show(ui, |ui| {
                    for (video, sub) in self.dlg.sub_matches.iter_mut() {
                        ui.label(media::display_name(video));
                        let cur = sub.as_ref().map(|s| media::display_name(s)).unwrap_or_else(|| "—".into());
                        egui::ComboBox::from_id_salt(("sm", video.as_str())).selected_text(cur).width(260.0).show_ui(ui, |ui| {
                            if ui.selectable_label(sub.is_none(), "—").clicked() {
                                *sub = None;
                            }
                            for s in &subs {
                                if ui.selectable_label(sub.as_deref() == Some(s), media::display_name(s)).clicked() {
                                    *sub = Some(s.clone());
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
            });
            ui.add_space(10.0);
            let matched = self.dlg.sub_matches.iter().filter(|m| m.1.is_some()).count();
            ui.horizontal(|ui| {
                ui.label(RichText::new(tf!("{matched} von {} zugeordnet", self.dlg.sub_matches.len())).color(t.text_faint));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::accent_button(ui, &t, t!("Speichern")).clicked() {
                        let cur = self.current_path();
                        for (v, s) in &self.dlg.sub_matches {
                            if let Some(s) = s {
                                self.data.sub_map.insert(v.clone(), s.clone());
                                if *v == cur {
                                    if let Some(p) = &self.player {
                                        p.sub_add(s, true);
                                    }
                                }
                            }
                        }
                        self.mark_dirty();
                        self.osd(tf!("{matched} Untertitel zugeordnet"));
                        self.dlg.sub_matches.clear();
                        self.dlg.sub_scope = None;
                        self.dialog = None;
                    }
                    if widgets::ghost_button(ui, &t, t!("Zurück")).clicked() {
                        self.dlg.sub_matches.clear();
                    }
                });
            });
            return;
        }

        ui.label(RichText::new(t!("Untertitel hinzufügen zu")).font(theme::semibold(13.5)));
        ui.add_space(4.0);
        let mut scope = self.dlg.sub_scope.unwrap_or(SubScope::File);
        ui.horizontal(|ui| {
            for (s, icon, label, enabled) in [
                (SubScope::File, ph::FILE_VIDEO, t!("Dieser Datei"), has),
                (SubScope::Folder, ph::FOLDER_SIMPLE, t!("Diesem Ordner"), folder.is_some()),
                (SubScope::Queue, ph::QUEUE, t!("Dieser Warteschlange"), has),
            ] {
                let (r, resp) = ui.allocate_exact_size(vec2(196.0, 74.0), if enabled { Sense::click() } else { Sense::hover() });
                let sel = scope == s && enabled;
                ui.painter().rect_filled(r, CornerRadius::same(theme::R_LG), if sel { t.accent_soft } else if resp.hovered() { t.hover } else { t.elevated });
                ui.painter().rect_stroke(r, CornerRadius::same(theme::R_LG), Stroke::new(if sel { 1.5 } else { 1.0 }, if sel { t.accent } else { t.border }), StrokeKind::Inside);
                let col = if enabled { if sel { t.accent } else { t.text } } else { t.text_faint };
                ui.painter().text(pos2(r.center().x, r.top() + 26.0), Align2::CENTER_CENTER, icon, theme::icon(22.0), col);
                ui.painter().text(pos2(r.center().x, r.bottom() - 18.0), Align2::CENTER_CENTER, label, FontId::proportional(13.0), col);
                if resp.clicked() {
                    scope = s;
                }
            }
        });
        self.dlg.sub_scope = Some(scope);
        ui.add_space(14.0);
        ui.label(RichText::new(t!("Quelle")).font(theme::semibold(13.5)));
        ui.add_space(4.0);
        let mut pick_local = false;
        let mut online = false;
        ui.horizontal(|ui| {
            if widgets::icon_text_button(ui, &t, ph::HARD_DRIVES, if crate::platform::MOBILE { t!("Vom Gerät…") } else { t!("Von diesem Computer…") }).clicked() {
                pick_local = true;
            }
            if widgets::icon_text_button(ui, &t, ph::GLOBE, t!("Online suchen (OpenSubtitles)")).clicked() {
                online = true;
            }
        });
        ui.add_space(6.0);
        ui.label(
            RichText::new(t!("Tipp: Für eine ganze Staffel einfach alle Untertiteldateien auf einmal auswählen – sie werden anhand von Folgennummern (S01E05, 1x05 …) automatisch zugeordnet."))
                .size(12.0)
                .color(t.text_faint),
        );
        if online {
            let q = self
                .player
                .as_ref()
                .map(|p| p.st.media_title.clone())
                .filter(|s| !s.is_empty())
                .or_else(|| folder.as_ref().map(|f| crate::library::name_of(f)))
                .unwrap_or_default();
            let url = format!("https://www.opensubtitles.org/{}/search2/sublanguageid-all/moviename-{}", crate::i18n::code(), media::url_encode(&q));
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
        if pick_local {
            self.pick(Pick::SubtitlesFor(scope));
        }
    }

    // ========================================================= playlists

    fn dlg_new_playlist(&mut self, ui: &mut Ui, items: Vec<String>) {
        let t = self.t;
        dlg_width(ui, 420.0);
        if self.dlg_title(ui, t!("Neue Playlist")) {
            return;
        }
        if !items.is_empty() {
            ui.label(RichText::new(tf!("{} Titel werden hinzugefügt.", items.len())).color(t.text_dim));
        }
        let r = ui.add(egui::TextEdit::singleline(&mut self.dlg.playlist_name).hint_text(t!("Name der Playlist")).desired_width(f32::INFINITY));
        if self.dlg.playlist_name.is_empty() {
            r.request_focus();
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(10.0);
        let mut pick = false;
        let mut create = enter;
        ui.horizontal(|ui| {
            if items.is_empty() && widgets::ghost_button(ui, &t, t!("Dateien auswählen…")).clicked() {
                pick = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::accent_button(ui, &t, t!("Erstellen")).clicked() {
                    create = true;
                }
            });
        });
        if pick {
            self.pick(Pick::PlaylistFiles(items.clone()));
        }
        if create && !self.dlg.playlist_name.trim().is_empty() {
            let name = self.dlg.playlist_name.trim().to_string();
            self.data.playlists.push(Playlist { name: name.clone(), items, created: config::now_secs() });
            self.mark_dirty();
            self.osd(tf!("Playlist „{name}“ erstellt"));
            self.dialog = None;
        }
    }

    fn dlg_add_to_playlist(&mut self, ui: &mut Ui, items: Vec<String>) {
        let t = self.t;
        dlg_width(ui, 380.0);
        if self.dlg_title(ui, t!("Zu Playlist hinzufügen")) {
            return;
        }
        let mut chosen = None;
        for (i, p) in self.data.playlists.iter().enumerate() {
            if widgets::menu_item(ui, &t, ph::PLAYLIST, &p.name, &tf!("{} Titel", p.items.len()), false).clicked() {
                chosen = Some(i);
            }
        }
        widgets::separator(ui, &t);
        if widgets::menu_item(ui, &t, ph::PLUS, t!("Neue Playlist…"), "", false).clicked() {
            self.dlg.playlist_name.clear();
            self.dialog = Some(Dialog::NewPlaylist(items.clone()));
        }
        if let Some(i) = chosen {
            let n = items.len();
            let name = self.data.playlists[i].name.clone();
            self.data.playlists[i].items.extend(items);
            self.mark_dirty();
            self.osd(tf!("{n} Titel zu „{name}“ hinzugefügt"));
            self.dialog = None;
        }
    }

    fn dlg_save_preset(&mut self, ui: &mut Ui) {
        let t = self.t;
        dlg_width(ui, 440.0);
        if self.dlg_title(ui, t!("Preset speichern")) {
            return;
        }
        ui.label(RichText::new(t!("Speichert Untertitelverzögerung, Darstellung, gewählte Untertitel- und Audiospur, Audioverzögerung, Geschwindigkeit, Equalizer und Videofilter.")).size(12.0).color(t.text_dim));
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(&mut self.dlg.preset_name).hint_text("Name").desired_width(f32::INFINITY));
        let folder = self.current_folder();
        if let Some(f) = &folder {
            ui.checkbox(&mut self.dlg.preset_for_folder, tf!("Automatisch für alle Folgen in „{}“ anwenden", crate::library::name_of(f)));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::accent_button(ui, &t, t!("Speichern")).clicked() && !self.dlg.preset_name.trim().is_empty() {
                    let name = self.dlg.preset_name.trim().to_string();
                    let pr = self.capture_preset(&name);
                    self.data.presets.retain(|p| p.name != name);
                    self.data.presets.push(pr);
                    if self.dlg.preset_for_folder {
                        if let Some(f) = &folder {
                            self.data.folder_presets.insert(f.to_string_lossy().to_string(), name.clone());
                        }
                    }
                    self.mark_dirty();
                    self.osd(tf!("Preset „{name}“ gespeichert"));
                    self.dialog = None;
                }
            });
        });
    }

    // ============================================================= about

    /// Played local files that are gone: remove them or keep them. Asked
    /// once; the answer becomes the setting for every later case.
    fn dlg_missing_media(&mut self, ui: &mut Ui, list: Vec<String>) {
        let t = self.t;
        dlg_width(ui, 480.0);
        if self.dlg_title(ui, t!("Medien nicht gefunden")) {
            return;
        }
        let intro = if list.len() == 1 {
            t!("Diese Datei wurde mit Velouna abgespielt, ist aber nicht mehr erreichbar – vielleicht wurde sie verschoben oder gelöscht.").to_string()
        } else {
            tf!("{} Dateien wurden mit Velouna abgespielt, sind aber nicht mehr erreichbar – vielleicht wurden sie verschoben oder gelöscht.", list.len())
        };
        ui.label(RichText::new(intro).color(t.text_dim));
        ui.add_space(8.0);
        const SHOWN: usize = 6;
        for p in list.iter().take(SHOWN) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(ph::FILE_X).color(t.danger));
                ui.add(egui::Label::new(RichText::new(media::display_name(p)).color(t.text)).truncate());
            });
        }
        if list.len() > SHOWN {
            ui.label(RichText::new(tf!("und {} weitere", list.len() - SHOWN)).color(t.text_faint));
        }
        ui.add_space(10.0);
        ui.label(RichText::new(t!("Aus Verlauf, Playlists und Anheftungen entfernen?")).color(t.text));
        ui.label(
            RichText::new(t!("Deine Wahl gilt ab jetzt automatisch. Ändern kannst du sie unter Einstellungen → Bibliothek."))
                .size(12.0)
                .color(t.text_faint),
        );
        ui.add_space(12.0);
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let mut choice = None;
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::accent_button(ui, &t, t!("Entfernen")).clicked() || enter {
                    choice = Some(MissingMedia::Remove);
                }
                if widgets::ghost_button(ui, &t, t!("Behalten")).clicked() {
                    choice = Some(MissingMedia::Keep);
                }
            });
        });
        let Some(choice) = choice else { return };
        self.data.settings.missing_media = choice;
        if choice == MissingMedia::Remove {
            self.forget_media(&list);
            self.osd(if list.len() == 1 {
                tf!("„{}“ entfernt", media::display_name(&list[0]))
            } else {
                tf!("{} Medien entfernt", list.len())
            });
        } else {
            self.remember_missing(&list);
        }
        self.mark_dirty();
        self.dialog = None;
    }

    fn dlg_about(&mut self, ui: &mut Ui) {
        let t = self.t;
        dlg_width(ui, 440.0);
        if self.dlg_title(ui, t!("Über")) {
            return;
        }
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(80.0, 80.0), Sense::hover());
            widgets::draw_logo(ui.painter(), r, t.accent);
            ui.add_space(8.0);
            ui.label(RichText::new("Velouna Media Player").font(theme::semibold(20.0)));
            ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(t.text_dim));
            ui.add_space(8.0);
            ui.label(RichText::new(t!("Ein Video- und Audioplayer in Rust – Oberfläche nach dem Redesign-Konzept: ruhige, dünne Leisten, dicke Suchleiste, Seitenleisten statt Dialoge, Bibliothek direkt neben dem Video.")).color(t.text_dim));
            ui.add_space(8.0);
            if let Some(p) = &self.player {
                ui.label(RichText::new(tf!("Wiedergabe: {}", p.mpv.get_string("mpv-version").unwrap_or_else(|| "libmpv".into()))).size(12.0).color(t.text_faint));
            }
            ui.label(RichText::new(t!("Werkzeuge: FFmpeg · Oberfläche: egui · Icons: Phosphor")).size(12.0).color(t.text_faint));
        });
    }

    fn dlg_error(&mut self, ui: &mut Ui, e: String) {
        let t = self.t;
        dlg_width(ui, 560.0);
        if self.dlg_title(ui, t!("Wiedergabe-Engine nicht gefunden")) {
            return;
        }
        ui.label(RichText::new(t!("Velouna nutzt libmpv zur Wiedergabe. Die Bibliothek konnte nicht geladen werden:")).color(t.text_dim));
        ui.add_space(4.0);
        ui.label(RichText::new(e).monospace().color(t.danger));
        ui.add_space(8.0);
        ui.label(t!("So behebst du das:"));
        if cfg!(target_os = "linux") {
            // The packages bring libmpv in /opt/velouna/lib (or the per-user
            // folder); missing means a damaged install or a bare binary.
            ui.label(t!("1. Installiere Velouna neu (das Paket bringt libmpv mit) – oder installiere libmpv vom System: sudo apt install libmpv2"));
            ui.label(t!("2. Alternativ: Umgebungsvariable VELOUNA_LIBMPV auf den Pfad von libmpv.so.2 setzen."));
        } else {
            ui.label(t!("1. Lade ein „mpv-dev-x86_64“-Paket herunter (z. B. von github.com/shinchiro/mpv-winbuild-cmake/releases)."));
            ui.label(t!("2. Lege die enthaltene libmpv-2.dll neben velouna.exe oder in den Ordner „lib“."));
            ui.label(t!("3. Alternativ: Umgebungsvariable VELOUNA_LIBMPV auf den Pfad der DLL setzen."));
        }
    }

    // ======================================================== onboarding

    fn dlg_onboarding(&mut self, ui: &mut Ui, step: u8) {
        let t = self.t;
        dlg_width(ui, 640.0);
        let top_y = ui.cursor().top();
        let mut next = None;
        match step {
            0 => {
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                    widgets::draw_logo(ui.painter(), r, t.accent);
                    ui.add_space(6.0);
                    ui.label(RichText::new(t!("Willkommen bei Velouna")).font(theme::semibold(22.0)));
                    ui.label(RichText::new(t!("Wähle dein bevorzugtes Design und deine Farbe.")).color(t.text_dim));
                    ui.add_space(16.0);
                    // Tiles and dots shrink on narrow phones instead of widening the dialog.
                    let gap = 24.0_f32.min(ui.available_width() * 0.05);
                    let tile = ((ui.available_width() - 2.0 * gap) / 3.0).min(96.0);
                    ui.horizontal(|ui| {
                        let total = 3.0 * tile + 2.0 * gap;
                        ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                        ui.spacing_mut().item_spacing.x = gap;
                        for (mode, label) in [(ThemeMode::Light, t!("Hell")), (ThemeMode::Dark, t!("Dunkel")), (ThemeMode::System, "System")] {
                            ui.vertical(|ui| {
                                let (r, resp) = ui.allocate_exact_size(vec2(tile, tile * 0.75), Sense::click());
                                let sel = self.data.settings.theme == mode;
                                match mode {
                                    ThemeMode::Light => {
                                        ui.painter().rect_filled(r, CornerRadius::same(8), Color32::from_gray(225));
                                    }
                                    ThemeMode::Dark => {
                                        ui.painter().rect_filled(r, CornerRadius::same(8), Color32::from_gray(28));
                                    }
                                    ThemeMode::System => {
                                        ui.painter().rect_filled(r, CornerRadius::same(8), Color32::from_gray(28));
                                        ui.painter().add(egui::Shape::convex_polygon(vec![pos2(r.center().x + 10.0, r.top()), r.right_top(), r.right_bottom(), pos2(r.center().x - 10.0, r.bottom())], Color32::from_gray(200), Stroke::NONE));
                                    }
                                }
                                if sel {
                                    ui.painter().rect_stroke(r.expand(3.0), CornerRadius::same(10), Stroke::new(2.0, t.accent), StrokeKind::Outside);
                                }
                                ui.label(RichText::new(label).color(if sel { t.text } else { t.text_dim }));
                                if resp.clicked() {
                                    self.data.settings.theme = mode;
                                    self.theme_dirty = true;
                                }
                            });
                        }
                    });
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        let dot = (ui.available_width() / ACCENTS.len() as f32 - 4.0).min(30.0);
                        let total = ACCENTS.len() as f32 * (dot + 4.0);
                        ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for a in ACCENTS {
                            if widgets::color_swatch(ui, &t, Color32::from_rgb(a[0], a[1], a[2]), self.data.settings.accent == a, dot).clicked() {
                                self.data.settings.accent = a;
                                self.theme_dirty = true;
                            }
                        }
                    });
                    ui.add_space(14.0);
                    // live preview bar
                    // Narrower on phones, so the dialog does not grow past the screen.
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(520.0), 36.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(6), t.panel);
                    let pb = Rect::from_min_size(pos2(r.left() + 40.0, r.top() + 6.0), vec2(38.0, 24.0));
                    ui.painter().rect_filled(pb, CornerRadius::same(4), t.accent);
                    ui.painter().text(pb.center(), Align2::CENTER_CENTER, ph::PLAY, theme::fill_icon(13.0), t.on_accent);
                    let tr = Rect::from_min_max(pos2(r.left() + 140.0, r.center().y - 6.0), pos2(r.right() - 120.0, r.center().y + 6.0));
                    ui.painter().rect_filled(tr, CornerRadius::same(3), t.track);
                    ui.painter().rect_filled(Rect::from_min_size(tr.min, vec2(tr.width() * 0.4, tr.height())), CornerRadius::same(3), t.accent_dim);
                    ui.painter().rect_filled(Rect::from_center_size(pos2(tr.left() + tr.width() * 0.4, tr.center().y), vec2(6.0, 18.0)), CornerRadius::same(3), t.accent);
                });
                // Phones have no keyboard shortcuts to show.
                next = Some(if crate::platform::MOBILE { 2 } else { 1 });
            }
            1 => {
                ui.label(RichText::new(t!("Die wichtigsten Tastenkürzel")).font(theme::semibold(20.0)));
                ui.label(RichText::new(t!("Die meisten steuern Velouna mit der Tastatur. Alle Kürzel kannst du später unter Einstellungen → Steuerung ändern.")).color(t.text_dim));
                ui.add_space(12.0);
                let list = [
                    hotkeys::Action::PlayPause,
                    hotkeys::Action::Fullscreen,
                    hotkeys::Action::SeekBack,
                    hotkeys::Action::SeekFwd,
                    hotkeys::Action::VolUp,
                    hotkeys::Action::Mute,
                    hotkeys::Action::Bookmark,
                    hotkeys::Action::SubDelayPlus,
                    hotkeys::Action::Library,
                    hotkeys::Action::HideUi,
                ];
                egui::Grid::new("ob_keys").num_columns(4).spacing([18.0, 10.0]).show(ui, |ui| {
                    for (i, a) in list.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if let Some(c) = self.bindings.combo(*a) {
                                for (j, chip) in c.chips().iter().enumerate() {
                                    if j > 0 {
                                        ui.label("+");
                                    }
                                    widgets::key_chip(ui, &t, chip);
                                }
                            }
                        });
                        ui.label(RichText::new(crate::i18n::tr(hotkeys::info(*a).name)).color(t.text_dim));
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
                next = Some(2);
            }
            2 => {
                ui.label(RichText::new(t!("Deine Seitenleiste")).font(theme::semibold(20.0)));
                let touch = crate::platform::MOBILE;
                ui.label(
                    RichText::new(if touch {
                        t!("Wähle, welche Bereiche die Leiste unten zeigt. Du kannst das jederzeit in den Einstellungen ändern.")
                    } else {
                        t!("Wähle, was Velouna links anzeigt – passend dazu, wofür du es nutzt. Du kannst das jederzeit unter Einstellungen → Oberfläche ändern.")
                    })
                    .color(t.text_dim),
                );
                ui.add_space(12.0);
                let items: Vec<(&str, &str, &str)> = crate::config::NAV_ITEMS.iter().copied().filter(|x| !touch || matches!(x.0, "servers" | "videos" | "music")).collect();
                // Quick picks for the usual ways to use Velouna.
                let presets: [(&str, &[&str]); 4] = [
                    (t!("Alles"), &["videos", "music", "tv", "radio", "network", "playlists", "recent", "servers", "pins"]),
                    (t!("Eigene Dateien"), &["videos", "music", "playlists", "recent", "pins", "network"]),
                    (crate::i18n::tr("Medienserver"), &["servers", "recent"]),
                    (t!("TV & Radio"), &["tv", "radio", "recent"]),
                ];
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    for (label, on) in presets {
                        let active = items.iter().all(|(id, _, _)| self.data.settings.nav_shown(id) == on.contains(id));
                        if widgets::chip(ui, &t, label, active).clicked() {
                            for (id, _, _) in &items {
                                self.data.settings.set_nav_shown(id, on.contains(id));
                            }
                            self.mark_dirty();
                        }
                    }
                });
                ui.add_space(12.0);
                // Two columns where they fit, one on phones.
                let two = ui.available_width() >= 2.0 * 250.0 + 28.0;
                let cell = if two { 250.0 } else { ui.available_width() };
                egui::Grid::new("ob_nav").num_columns(if two { 2 } else { 1 }).spacing([28.0, 6.0]).show(ui, |ui| {
                    for (i, (id, label, icon)) in items.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.set_min_width(cell);
                            let mut on = self.data.settings.nav_shown(id);
                            ui.label(RichText::new(*icon).font(theme::icon(17.0)).color(if on { t.accent } else { t.text_faint }));
                            ui.label(RichText::new(crate::i18n::tr(label)).color(if on { t.text } else { t.text_dim }));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                widgets::toggle(ui, &t, &mut on);
                            });
                            if on != self.data.settings.nav_shown(id) {
                                self.data.settings.set_nav_shown(id, on);
                                self.mark_dirty();
                            }
                        });
                        if !two || i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
                next = Some(3);
            }
            _ => {
                ui.label(RichText::new(t!("Deine Bibliothek")).font(theme::semibold(20.0)));
                ui.label(RichText::new(t!("Diese Ordner erscheinen in der Bibliothek. Du kannst jederzeit weitere hinzufügen.")).color(t.text_dim));
                ui.add_space(10.0);
                let mut s = self.data.settings.clone();
                for (music, (label, list)) in [("Videos", &mut s.video_folders), (t!("Musik"), &mut s.music_folders)].into_iter().enumerate().map(|(i, x)| (i == 1, x)) {
                    ui.label(RichText::new(label).font(theme::semibold(13.5)));
                    let mut rm = None;
                    for (i, f) in list.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(ph::FOLDER_SIMPLE).color(t.accent));
                            ui.label(f.to_string_lossy());
                            if ui.small_button(t!("Entfernen")).clicked() {
                                rm = Some(i);
                            }
                        });
                    }
                    if let Some(i) = rm {
                        list.remove(i);
                    }
                    if ui.button(tf!("{}  Ordner hinzufügen", ph::PLUS)).clicked() {
                        self.pick(Pick::LibraryFolder { music, show: false });
                        *list = if music { self.data.settings.music_folders.clone() } else { self.data.settings.video_folders.clone() };
                    }
                    ui.add_space(8.0);
                }
                if crate::platform::MOBILE && !self.storage.media() {
                    ui.label(RichText::new(t!("Damit Velouna deine Videos und Musik findet, braucht es Zugriff auf den Speicher.")).color(t.text_dim));
                    if ui.button(t!("Zugriff erlauben")).clicked() {
                        crate::pick::request_storage();
                    }
                }
                if s.video_folders != self.data.settings.video_folders || s.music_folders != self.data.settings.music_folders {
                    let ctx = ui.ctx().clone();
                    self.commit_settings(&ctx, s);
                }
            }
        }
        let used = ui.cursor().top() - top_y;
        if used < 400.0 {
            ui.add_space(400.0 - used);
        }
        ui.horizontal(|ui| {
            for i in (0..4u8).filter(|i| !(crate::platform::MOBILE && *i == 1)) {
                let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, if i == step { t.accent } else { t.hover });
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if next.is_some() { t!("Weiter") } else { t!("Los geht's") };
                if widgets::accent_button(ui, &t, label).clicked() {
                    match next {
                        Some(n) => self.dialog = Some(Dialog::Onboarding(n)),
                        None => {
                            self.data.settings.onboarding_done = true;
                            self.mark_dirty();
                            self.dialog = None;
                        }
                    }
                }
                if step > 0 && widgets::ghost_button(ui, &t, t!("Zurück")).clicked() {
                    self.dialog = Some(Dialog::Onboarding(if crate::platform::MOBILE && step == 2 { 0 } else { step - 1 }));
                }
                if widgets::ghost_button(ui, &t, t!("Überspringen")).clicked() {
                    self.data.settings.onboarding_done = true;
                    self.mark_dirty();
                    self.dialog = None;
                }
            });
        });
    }
}
