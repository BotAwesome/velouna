//! Touch layout (phones, tablets, simulator): header, bottom navigation in
//! portrait / navigation rail in landscape, mini player, and a full-screen
//! player with tap-to-show controls. Sizes come from `mobile::m()`.

use crate::app::{App, Dialog, RightPanel, View};
use crate::config::{RepeatMode, TouchItem};
use crate::hotkeys::Action;
use crate::library::{LibLoc, MusicView, PlexLoc};
use crate::mobile::{self, m};
use crate::theme;
use crate::widgets;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Frame, Id, Margin, Order, Rect, RectAlign, Sense, Stroke, Ui, UiBuilder, pos2, vec2,
};
use egui_phosphor::regular as ph;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Home,
    Plex,
    Videos,
    Music,
    More,
}

impl App {
    fn tab(&self) -> Tab {
        match self.lib.loc.section() {
            0 => Tab::Home,
            1 => Tab::Videos,
            2 => Tab::Music,
            6 => Tab::Plex,
            _ => Tab::More,
        }
    }

    pub fn draw_mobile(&mut self, ctx: &egui::Context) {
        let t = self.t;
        mobile::apply_back_gesture_exclusion();
        let mm = m();
        let portrait = mobile::portrait(ctx);
        let has_media = self.has_media();
        // Between two files (next episode, playlist) there is no media
        // while mpv loads the next one, which for a Plex stream can take
        // several seconds: stay in the (black) player as long as mpv is
        // busy, and leave only once it is idle (nothing follows). The short
        // grace covers the moment between "idle" and the next load.
        let active = self.player_active();
        // Any change of title (button, queue, end of file) counts as a skip.
        let pos = self.player.as_ref().map(|p| p.st.playlist_pos).unwrap_or(-1);
        if pos >= 0 && pos != self.mob.last_pos {
            if self.mob.last_pos >= 0 {
                self.mob.skip_at = Some(Instant::now());
            }
            self.mob.last_pos = pos;
        }
        if active {
            self.mob.media_lost_at = None;
        } else if self.prev_has_media {
            self.mob.media_lost_at = Some(Instant::now());
        }
        // Right after skipping (next / previous) wait longer: a slow server
        // may need a while before the next title reports anything.
        let skipped = self.mob.skip_at.is_some_and(|at| at.elapsed() < Duration::from_secs(15));
        let grace = if skipped { Duration::from_secs(15) } else { Duration::from_millis(2500) };
        let in_gap = self.mob.media_lost_at.is_some_and(|at| at.elapsed() < grace);
        if !active && !in_gap && self.mob.media_lost_at.take().is_some() {
            if let Some(p) = &self.player {
                log::info!(
                    "leaving player: no media (idle={}, path empty={}, playlist {}/{}, skipped={skipped})",
                    p.st.idle,
                    p.st.path.is_empty(),
                    p.st.playlist_pos,
                    p.st.playlist.len()
                );
            }
            self.view = View::Library;
        }
        if in_gap || (active && !has_media) {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        self.prev_has_media = active;
        let in_player = self.view == View::Player && (active || in_gap);

        // Player-bar editor: full screen over everything (settings stay
        // open underneath).
        if self.mob.editor.is_some() {
            #[cfg(target_os = "android")]
            crate::platform::android::immersive(false);
            egui::CentralPanel::default().frame(Frame::new().fill(t.video_bg)).show(ctx, |ui| self.touch_editor(ui));
            self.draw_osd(ctx);
            return;
        }

        // Full screen video: hide the system bars in the player.
        #[cfg(target_os = "android")]
        crate::platform::android::immersive(in_player);
        if in_player && !has_media {
            egui::CentralPanel::default().frame(Frame::new().fill(t.video_bg)).show(ctx, |_| {});
        } else if in_player {
            egui::CentralPanel::default().frame(Frame::new().fill(t.video_bg)).show(ctx, |ui| self.mobile_player(ui));
        } else {
            mobile::release_rotation();
            self.mob.locked = false;
            // Brightness set by swiping only applies in the player.
            if self.mob.brightness.take().is_some() {
                #[cfg(target_os = "android")]
                crate::platform::android::set_brightness(-1.0);
            }
            // While a file is still loading the view may already be Player.
            if !portrait {
                egui::SidePanel::left("m_rail")
                    .exact_width(mm.rail_w)
                    .resizable(false)
                    .frame(Frame::new().fill(t.panel))
                    .show_separator_line(false)
                    .show(ctx, |ui| {
                        let r = ui.max_rect();
                        ui.painter().line_segment([r.right_top(), r.right_bottom()], Stroke::new(1.0, t.border));
                        self.mobile_tabs(ui, r, false);
                    });
            }
            let bottom_h = if portrait { mm.nav_h } else { 0.0 } + if has_media { mm.mini_h } else { 0.0 };
            if bottom_h > 0.0 {
                egui::TopBottomPanel::bottom("m_bottom")
                    .exact_height(bottom_h)
                    .frame(Frame::new().fill(t.panel))
                    .show_separator_line(false)
                    .show(ctx, |ui| {
                        let r = ui.max_rect();
                        ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, t.border));
                        if has_media {
                            self.mini_bar(ui, Rect::from_min_size(r.min, vec2(r.width(), mm.mini_h)));
                        }
                        if portrait {
                            self.mobile_tabs(ui, Rect::from_min_max(pos2(r.left(), r.bottom() - mm.nav_h), r.max), true);
                        }
                    });
            }
            egui::TopBottomPanel::top("m_head")
                .exact_height(if portrait { mm.header_h } else { mm.header_h_land })
                .frame(Frame::new().fill(t.bg))
                .show_separator_line(false)
                .show(ctx, |ui| self.mobile_header(ui));
            let pad = mm.pad.round().clamp(0.0, 120.0) as i8;
            let content = egui::CentralPanel::default()
                .frame(Frame::new().fill(t.bg).inner_margin(Margin { left: pad, right: pad, top: 0, bottom: 0 }))
                .show(ctx, |ui| self.library_content(ui, true))
                .response
                .rect;
            // Plex libraries: swipe-in drawer instead of chips.
            if self.tab() == Tab::Plex && self.plex.ready() {
                let on_home = matches!(self.lib.loc, LibLoc::Plex(PlexLoc::Home));
                self.plex_drawer(ctx, ctx.screen_rect(), content.bottom(), on_home);
            } else {
                self.mob.drawer.close();
            }
        }

        // mpv's render context is made on the first video paint and mpv
        // can't open files without it: keep one alive even when no video
        // is visible (1 px under the status bar).
        let video_shown = has_media && (self.view == View::Player || self.player.as_ref().is_some_and(|p| p.st.has_video()));
        if !video_shown {
            if let Some(v) = &self.video {
                let p = ctx.layer_painter(egui::LayerId::background());
                let at = if self.full_screen.is_finite() { self.full_screen.min } else { ctx.screen_rect().min };
                v.paint(&p, Rect::from_min_size(at, vec2(1.0, 1.0)));
            }
        }

        self.draw_resume_prompt(ctx, false);
        self.draw_osd(ctx);
        self.draw_dialogs(ctx);
    }

    /// Android back: innermost thing first, then history, then the start
    /// tab, then leave the app.
    pub fn mobile_back(&mut self, ctx: &egui::Context) {
        if self.mob.editor.is_some() {
            return self.close_touch_editor();
        }
        if self.mob.drawer.is_open() && self.dialog.is_none() {
            return self.mob.drawer.close();
        }
        match &self.dialog {
            Some(Dialog::Settings) => return self.mobile_settings_back(ctx),
            Some(Dialog::Onboarding(_)) => return,
            Some(_) => {
                self.dialog = None;
                self.rebinding = None;
                return;
            }
            None => {}
        }
        if self.view == View::Player && self.player_active() {
            if self.mob.locked {
                // Locked: back only shows the unlock button.
                self.touch_bars = true;
                self.bars_shown_at = Instant::now();
            } else if self.right != RightPanel::None {
                self.right = RightPanel::None;
            } else {
                self.view = View::Library;
            }
        } else if !self.lib.back.is_empty() {
            self.lib.go_back();
        } else if self.tab() != Tab::Home {
            self.lib.go_root(LibLoc::Home);
        } else {
            #[cfg(target_os = "android")]
            crate::platform::android::move_to_back();
        }
    }

    // ------------------------------------------------------------ header

    fn mobile_header(&mut self, ui: &mut Ui) {
        let t = self.t;
        let mm = m();
        let r = ui.max_rect();
        let p = ui.painter().clone();
        let mut x = r.left() + 6.0;
        let b = mm.icon_btn;
        if !self.lib.back.is_empty() {
            let br = Rect::from_center_size(pos2(x + b / 2.0, r.center().y), vec2(b, b));
            if touch_icon(ui, br, ph::ARROW_LEFT, &t, false).clicked() {
                self.lib.go_back();
            }
            x = br.right() + 2.0;
        } else {
            let lr = Rect::from_center_size(pos2(x + b / 2.0 + 2.0, r.center().y), vec2(26.0, 26.0));
            widgets::draw_logo(&p, lr, t.accent);
            x = lr.right() + 12.0;
        }
        let title = match (self.tab(), &self.lib.loc) {
            (Tab::Home, _) => "Velouna".to_string(),
            _ => self.lib.title(&self.data),
        };
        // Right actions
        let gear = Rect::from_center_size(pos2(r.right() - 6.0 - b / 2.0, r.center().y), vec2(b, b));
        if touch_icon(ui, gear, ph::GEAR_SIX, &t, false).clicked() {
            self.dialog = Some(Dialog::Settings);
        }
        widgets::painter_text_trunc(&p, pos2(x, r.center().y - mm.title_size * 0.62), &title, theme::semibold(mm.title_size), t.text, gear.left() - x - 8.0);
    }

    // ------------------------------------------------------------ navigation

    fn mobile_tabs(&mut self, ui: &mut Ui, r: Rect, horizontal: bool) {
        let t = self.t;
        let mm = m();
        let cur = self.tab();
        let shown = |id: &str| self.data.settings.nav_shown(id);
        let tabs: Vec<(Tab, &str, &str)> = [
            (Tab::Home, ph::HOUSE, "Start", true),
            (Tab::Plex, ph::TELEVISION_SIMPLE, self.plex.backend().name(), shown("servers")),
            (Tab::Videos, ph::FILM_STRIP, "Videos", shown("videos")),
            (Tab::Music, ph::MUSIC_NOTES, t!("Musik"), shown("music")),
            (Tab::More, ph::DOTS_THREE_OUTLINE, t!("Mehr"), true),
        ]
        .into_iter()
        .filter(|x| x.3)
        .map(|(a, b, c, _)| (a, b, c))
        .collect();
        let n = tabs.len() as f32;
        for (i, (tab, icon, label)) in tabs.into_iter().enumerate() {
            let cell = if horizontal {
                let w = r.width() / n;
                Rect::from_min_size(pos2(r.left() + w * i as f32, r.top()), vec2(w, r.height()))
            } else {
                let h = 70.0_f32.min((r.height() - 16.0) / n);
                Rect::from_min_size(pos2(r.left(), r.top() + 10.0 + h * i as f32), vec2(r.width(), h))
            };
            let resp = ui.interact(cell, Id::new(("m_tab", i)), Sense::click());
            let sel = cur == tab;
            let icon_c = pos2(cell.center().x, cell.center().y - mm.tab_label * 0.75);
            let pill = Rect::from_center_size(icon_c, vec2(mm.tab_pill_w, mm.tab_pill_h));
            if sel {
                ui.painter().rect_filled(pill, CornerRadius::same((mm.tab_pill_h / 2.0) as u8), t.accent_soft);
            } else if resp.is_pointer_button_down_on() {
                ui.painter().rect_filled(pill, CornerRadius::same((mm.tab_pill_h / 2.0) as u8), t.hover);
            }
            let font = if sel { theme::fill_icon(mm.tab_icon) } else { theme::icon(mm.tab_icon) };
            ui.painter().text(icon_c, Align2::CENTER_CENTER, icon, font, if sel { t.accent } else { t.text_dim });
            ui.painter().text(
                pos2(cell.center().x, pill.bottom() + 4.0),
                Align2::CENTER_TOP,
                label,
                if sel { theme::semibold(mm.tab_label) } else { FontId::proportional(mm.tab_label) },
                if sel { t.text } else { t.text_dim },
            );
            if tab == Tab::More {
                let align = if horizontal { RectAlign::TOP_END } else { RectAlign::RIGHT_END };
                egui::Popup::menu(&resp).align(align).width(240.0).show(|ui| self.more_menu(ui));
            } else if resp.clicked() {
                self.lib.go_root(match tab {
                    Tab::Home => LibLoc::Home,
                    Tab::Plex => LibLoc::Plex(PlexLoc::Home),
                    Tab::Videos => LibLoc::Videos(None),
                    _ => LibLoc::Music(MusicView::Songs),
                });
            }
        }
    }

    fn more_menu(&mut self, ui: &mut Ui) {
        let t = self.t;
        let sec = self.lib.loc.section();
        for (icon, label, loc, s) in [
            (ph::TELEVISION, t!("TV-Sender"), LibLoc::Tv(None), 7u8),
            (ph::RADIO, t!("Radiosender"), LibLoc::Radio(None), 8),
            (ph::BROADCAST, t!("Netzwerk"), LibLoc::Network, 3),
            (ph::PLAYLIST, "Playlists", LibLoc::Playlists(None), 4),
            (ph::CLOCK_COUNTER_CLOCKWISE, t!("Zuletzt gespielt"), LibLoc::Recent, 5),
        ] {
            if widgets::menu_item(ui, &t, icon, label, "", sec == s).clicked() {
                self.lib.go_root(loc);
                ui.close();
            }
        }
        for (plugin, src) in self.ext_sources() {
            let sel = matches!(&self.lib.loc, LibLoc::Ext(e) if e.plugin == plugin && e.source == src.id);
            if widgets::menu_item(ui, &t, crate::ui_ext::icon_glyph(&src.icon), &src.label.get(), "", sel).clicked() {
                self.lib.go_root(Self::ext_source_loc(&plugin, &src));
                ui.close();
            }
        }
        ui.separator();
        if widgets::menu_item(ui, &t, ph::GEAR_SIX, t!("Einstellungen"), "", false).clicked() {
            self.dialog = Some(Dialog::Settings);
            ui.close();
        }
        if widgets::menu_item(ui, &t, ph::INFO, t!("Über Velouna"), "", false).clicked() {
            self.dialog = Some(Dialog::About);
            ui.close();
        }
    }

    // ------------------------------------------------------------ mini player

    fn mini_bar(&mut self, ui: &mut Ui, r: Rect) {
        let t = self.t;
        let mm = m();
        let Some(st) = self.player.as_ref().map(|p| p.st.clone()) else { return };
        let p = ui.painter().clone();
        let open = ui.interact(r, Id::new("m_mini"), Sense::click());
        // Radio is live: a full line instead of the buffer position.
        let radio = self.radio_urls.contains(&st.path);
        if radio || st.duration > 0.0 {
            let f = if radio { 1.0 } else { (st.time / st.duration).clamp(0.0, 1.0) as f32 };
            p.rect_filled(Rect::from_min_size(r.min, vec2(r.width() * f, 2.0)), CornerRadius::ZERO, t.accent);
        }
        let th = r.height() - 16.0;
        let thumb = Rect::from_min_size(pos2(r.left() + 10.0, r.top() + 8.0), vec2(if st.has_video() { th * 16.0 / 9.0 } else { th }, th));
        p.rect_filled(thumb, CornerRadius::same(theme::R), t.elevated);
        if st.has_video() {
            if let Some(v) = &self.video {
                v.paint(&p, thumb);
            }
        } else {
            let logo = self.stream_logos.get(&st.path).cloned().and_then(|u| self.logos.get(&u, || u.clone()));
            match logo {
                Some(img) => {
                    p.rect_filled(thumb, CornerRadius::same(theme::R), Color32::from_gray(235));
                    let inner = thumb.shrink(th * 0.1);
                    let k = (inner.width() / img.size.x).min(inner.height() / img.size.y);
                    let fit = Rect::from_center_size(inner.center(), img.size * k);
                    p.image(img.id, fit, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::from_white_alpha((img.alpha * 255.0) as u8));
                }
                None => widgets::draw_logo(&p, thumb.shrink(th * 0.2), t.accent),
            }
        }
        let btn = mm.icon_btn;
        let play = Rect::from_center_size(pos2(r.right() - 10.0 - btn * 1.5, r.center().y), vec2(btn, btn));
        let stop = Rect::from_center_size(pos2(r.right() - 6.0 - btn * 0.5, r.center().y), vec2(btn, btn));
        let tx = thumb.right() + 12.0;
        let tw = play.left() - tx - 6.0;
        widgets::painter_text_trunc(&p, pos2(tx, r.center().y - 18.0), &st.media_title, theme::semibold(14.0), t.text, tw);
        let sub = match st.now_playing() {
            Some(song) if !st.paused => song,
            _ if st.paused => t!("Pausiert"),
            _ => t!("Wird abgespielt"),
        };
        widgets::painter_text_trunc(&p, pos2(tx, r.center().y + 2.0), sub, FontId::proportional(12.0), t.text_dim, tw);
        if touch_icon(ui, play, if st.paused { ph::PLAY } else { ph::PAUSE }, &t, false).clicked() {
            if let Some(pl) = &self.player {
                pl.ctl.toggle_pause();
            }
        } else if touch_icon(ui, stop, ph::X, &t, false).clicked() {
            let ctx = ui.ctx().clone();
            self.run_action(&ctx, Action::Stop, None);
        } else if open.clicked() {
            self.view = View::Player;
            self.touch_bars = true;
            self.bars_shown_at = Instant::now();
        }
    }

    // ------------------------------------------------------------ player

    pub fn touch_bars_visible(&self) -> bool {
        // Music keeps its player bar: nothing to see behind it, and a song
        // should not need a tap before it can be paused or skipped.
        if self.mobile_audio_bar() {
            return true;
        }
        let paused = self.player.as_ref().is_none_or(|p| p.st.paused);
        self.touch_bars && (paused || self.bars_shown_at.elapsed() < Duration::from_millis(m().hide_after_ms as u64) || egui_popup_open(self))
    }

    /// The chosen phone buttons that fit what plays: no picture tools for
    /// audio, nothing to seek, repeat or speed up for live streams.
    pub fn touch_items(&self) -> crate::config::TouchControls {
        use crate::config::TouchItem as I;
        let (audio, live, _) = self.media_kind();
        let fits = |i: &I| match i {
            I::Subtitles | I::Tracks | I::Scale | I::Rotate => !audio,
            I::Repeat | I::Shuffle | I::Speed | I::Bookmark | I::SeekBack | I::SeekForward => !live,
            _ => true,
        };
        let mut tc = self.data.settings.touch_controls.clone();
        tc.left.retain(fits);
        tc.right.retain(fits);
        tc
    }

    /// Audio without a picture plays with a fixed player bar at the bottom.
    pub fn mobile_audio_bar(&self) -> bool {
        !self.mob.locked && self.player.as_ref().is_some_and(|p| p.st.is_audio_only())
    }

    /// Height of that bar above the safe area: buttons, seek row, controls.
    pub fn mobile_audio_bar_h(&self) -> f32 {
        let mm = m();
        let tc = self.touch_items();
        let buttons = if tc.left.is_empty() && tc.right.is_empty() { 10.0 } else { mm.icon_btn + 8.0 };
        buttons + mm.seek_h + 10.0 + mm.play_btn + 18.0
    }

    fn mobile_player(&mut self, ui: &mut Ui) {
        // Video covers the whole display; controls stay in the safe area.
        let [l, t, r, b] = self.safe_insets;
        let full = ui.max_rect();
        let rect = Rect::from_min_max(full.min + vec2(l, t), full.max - vec2(r, b));
        let visible = self.touch_bars_visible();
        // With a sheet open the video moves into the free part of the screen
        // (above it in portrait, left of it in landscape) instead of hiding
        // behind it.
        let sheet = sheet_rect(rect);
        let free = if rect.height() >= rect.width() {
            Rect::from_min_max(full.min, pos2(full.right(), sheet.top()))
        } else {
            Rect::from_min_max(full.min, pos2(sheet.left(), full.bottom()))
        };
        let k = ui.ctx().animate_bool_with_time(Id::new("m_sheet_anim"), self.right != RightPanel::None, 0.2);
        let vrect = full.lerp_towards(&free, k);
        ui.painter().rect_filled(full, CornerRadius::ZERO, self.t.video_bg);
        ui.scope_builder(UiBuilder::new().max_rect(vrect), |ui| {
            self.video_area(ui, true, visible);
            if self.touch_bars_visible() {
                self.mobile_player_overlay(ui, vrect.intersect(rect));
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
        });
        ui.scope_builder(UiBuilder::new().max_rect(vrect), |ui| self.gesture_hud(ui, vrect.intersect(rect)));
        if self.right != RightPanel::None {
            self.mobile_sheet(ui.ctx(), sheet, rect.height() >= rect.width());
        }
        if self.player.as_ref().is_some_and(|p| p.st.has_video()) {
            self.gesture_intro(ui.ctx(), rect);
        }
    }

    /// Tap on the video: show/hide controls; double tap left/right seeks.
    pub fn touch_video_input(&mut self, resp: &egui::Response, rect: Rect) {
        let mm = m();
        if self.mob.locked {
            // Locked: a tap only shows (or hides) the unlock button.
            self.mob.gesture = None;
            if resp.clicked() {
                self.touch_bars = !self.touch_bars_visible();
                self.bars_shown_at = Instant::now();
            }
            return;
        }
        // The now-playing page of music has nothing to swipe or double-tap on,
        // and a live stream nothing to seek in.
        let (audio, live, _) = self.media_kind();
        if self.data.settings.touch_gestures && !audio {
            self.swipe_gestures(resp, rect);
        }
        if resp.double_clicked() && !audio && !live {
            if let (Some(pos), Some(p)) = (resp.interact_pointer_pos(), &self.player) {
                let f = (pos.x - rect.left()) / rect.width();
                if f < mm.seek_zone {
                    p.ctl.seek_rel(-mm.seek_step_s as f64);
                    self.osd(format!("{} −{:.0} s", ph::REWIND, mm.seek_step_s));
                } else if f > 1.0 - mm.seek_zone {
                    p.ctl.seek_rel(mm.seek_step_s as f64);
                    self.osd(format!("{} +{:.0} s", ph::FAST_FORWARD, mm.seek_step_s));
                } else {
                    p.ctl.toggle_pause();
                }
            }
            return;
        }
        if resp.clicked() {
            if self.touch_bars_visible() {
                self.touch_bars = false;
            } else {
                self.touch_bars = true;
                self.bars_shown_at = Instant::now();
            }
        }
    }

    /// Audio track and subtitle choice for the phone player.
    fn mobile_tracks_menu(&mut self, ui: &mut Ui, audio: bool, subs: bool) {
        let t = self.t;
        let Some(p) = &self.player else { return };
        let st = p.st.clone();
        egui::ScrollArea::vertical().max_height(ui.ctx().screen_rect().height() * 0.6).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if audio && !subs && st.tracks_of("audio").next().is_none() {
                ui.label(egui::RichText::new(t!("Keine Tonspur")).color(t.text_dim));
            }
            if audio && st.tracks_of("audio").next().is_some() {
                widgets::section_label(ui, &t, "Audio");
                for tr in st.tracks_of("audio") {
                    if widgets::check_row(ui, &t, &tr.label(), tr.selected).clicked() {
                        p.set_track("audio", Some(tr.id));
                    }
                }
                ui.add_space(6.0);
            }
            if !subs {
                return;
            }
            widgets::section_label(ui, &t, t!("Untertitel"));
            if widgets::check_row(ui, &t, t!("Untertitel aus"), st.selected("sub").is_none()).clicked() {
                p.set_track("sub", None);
            }
            for tr in st.tracks_of("sub") {
                if widgets::check_row(ui, &t, &tr.label(), tr.selected).clicked() {
                    p.set_track("sub", Some(tr.id));
                }
            }
        });
    }

    /// Sleep timer choices (moon menu and bottom-row button).
    fn sleep_menu(&mut self, ui: &mut Ui) {
        let t = self.t;
        ui.label(egui::RichText::new(t!("Sleep-Timer")).font(theme::semibold(14.0)).color(t.text));
        ui.add_space(4.0);
        for m in [15u64, 30, 45, 60, 90] {
            if widgets::menu_item(ui, &t, ph::TIMER, &format!("{m} min"), "", false).clicked() {
                self.sleep_minutes(Some(m));
                ui.close();
            }
        }
        if widgets::menu_item(ui, &t, ph::FLAG_CHECKERED, t!("Ende der Folge / des Films"), "", self.sleep_eof).clicked() {
            self.sleep_end_of_title();
            ui.close();
        }
        if self.sleep_active() {
            ui.separator();
            if widgets::menu_item(ui, &t, ph::X, t!("Sleep-Timer aus"), "", false).clicked() {
                self.sleep_minutes(None);
                ui.close();
            }
        }
    }

    /// Vertical swipe: left half volume, right half brightness.
    fn swipe_gestures(&mut self, resp: &egui::Response, rect: Rect) {
        let Some(pos) = resp.interact_pointer_pos() else {
            self.mob.gesture = None;
            return;
        };
        if resp.drag_started() {
            // Leave the screen edges to the system's own swipes.
            let full = if self.full_screen.is_finite() { self.full_screen } else { rect };
            let start = pos - resp.drag_delta();
            if start.y < full.top() + 48.0 || start.y > full.bottom() - 48.0 {
                self.mob.gesture = None;
                return;
            }
            let kind = if start.x < rect.center().x { GestureKind::Volume } else { GestureKind::Brightness };
            let start_val = match kind {
                GestureKind::Volume => self.player.as_ref().map(|p| (p.st.volume / 100.0) as f32).unwrap_or(1.0),
                GestureKind::Brightness => self.mob.brightness.or_else(current_brightness).unwrap_or(0.5),
            };
            self.mob.gesture = Some(Gesture { kind, start, start_val, vertical: None });
        }
        if resp.drag_stopped() || !resp.dragged() {
            self.mob.gesture = None;
            return;
        }
        let Some(mut g) = self.mob.gesture else { return };
        let d = pos - g.start;
        if g.vertical.is_none() && d.length() > 14.0 {
            g.vertical = Some(d.y.abs() > d.x.abs() * 1.3);
        }
        self.mob.gesture = Some(g);
        if g.vertical != Some(true) {
            return;
        }
        // Full range over ~70 % of the video height; up = more.
        let v = (g.start_val - d.y / (rect.height() * 0.7)).clamp(0.0, 1.0);
        match g.kind {
            GestureKind::Volume => {
                if let Some(p) = &self.player {
                    p.ctl.set_volume((v * 100.0).round() as f64);
                    if p.st.muted && v > 0.0 {
                        p.ctl.set_mute(false);
                    }
                }
            }
            GestureKind::Brightness => {
                self.mob.brightness = Some(v.max(0.01));
                #[cfg(target_os = "android")]
                crate::platform::android::set_brightness(v.max(0.01));
            }
        }
        self.mob.hud = Some((g.kind, v, Instant::now()));
    }

    /// Volume / brightness level while swiping (and shortly after).
    fn gesture_hud(&mut self, ui: &mut Ui, rect: Rect) {
        let t = self.t;
        // Simulator: no real brightness, so dim the picture instead.
        if mobile::simulator() {
            if let Some(b) = self.mob.brightness {
                ui.painter().rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(((1.0 - b) * 200.0) as u8));
            }
        }
        let Some((kind, v, at)) = self.mob.hud else { return };
        let age = at.elapsed().as_secs_f32();
        let swiping = self.mob.gesture.is_some_and(|g| g.vertical == Some(true));
        let alpha = if swiping { 1.0 } else { (1.0 - (age - 0.7) / 0.3).clamp(0.0, 1.0) };
        if alpha <= 0.0 {
            self.mob.hud = None;
            return;
        }
        ui.ctx().request_repaint();
        let h = (rect.height() * 0.45).clamp(120.0, 220.0);
        let w = 46.0;
        let x = match kind {
            GestureKind::Volume => rect.left() + 40.0 + w / 2.0,
            GestureKind::Brightness => rect.right() - 40.0 - w / 2.0,
        };
        let pill = Rect::from_center_size(pos2(x, rect.center().y), vec2(w, h));
        let p = ui.painter();
        let a = |c: Color32| c.gamma_multiply(alpha);
        p.rect_filled(pill, CornerRadius::same(23), a(Color32::from_black_alpha(170)));
        let track = Rect::from_min_max(pos2(pill.left() + 8.0, pill.top() + 40.0), pos2(pill.right() - 8.0, pill.bottom() - 30.0));
        p.rect_filled(track, CornerRadius::same(6), a(Color32::from_white_alpha(40)));
        let fill = Rect::from_min_max(pos2(track.left(), track.bottom() - track.height() * v), track.max);
        p.rect_filled(fill, CornerRadius::same(6), a(t.accent));
        let icon = match kind {
            GestureKind::Volume if v <= 0.0 => ph::SPEAKER_X,
            GestureKind::Volume if v < 0.5 => ph::SPEAKER_LOW,
            GestureKind::Volume => ph::SPEAKER_HIGH,
            GestureKind::Brightness if v < 0.5 => ph::SUN_DIM,
            GestureKind::Brightness => ph::SUN,
        };
        p.text(pos2(pill.center().x, pill.top() + 21.0), Align2::CENTER_CENTER, icon, theme::icon(20.0), a(Color32::WHITE));
        p.text(pos2(pill.center().x, pill.bottom() - 15.0), Align2::CENTER_CENTER, format!("{:.0}", v * 100.0), theme::semibold(12.0), a(Color32::WHITE));
    }

    /// First time in the player: show the swipe gestures with a short
    /// animation and where to turn them off.
    fn gesture_intro(&mut self, ctx: &egui::Context, rect: Rect) {
        if !self.data.settings.touch_gestures || self.data.settings.gesture_intro_seen {
            return;
        }
        let t = self.t;
        let full = if self.full_screen.is_finite() { self.full_screen } else { rect };
        let fade = ctx.animate_bool_with_time(Id::new("m_intro_fade"), true, 0.35);
        let time = ctx.input(|i| i.time) as f32;
        ctx.request_repaint();
        let mut done = false;
        egui::Area::new(Id::new("m_gesture_intro")).order(Order::Foreground).fixed_pos(full.min).constrain(false).show(ctx, |ui| {
            ui.set_min_size(full.size());
            let p = ui.painter().clone();
            p.rect_filled(full, CornerRadius::ZERO, Color32::from_black_alpha((200.0 * fade) as u8));
            let a = |c: Color32| c.gamma_multiply(fade);
            // Taps anywhere also close it.
            if ui.interact(full, Id::new("m_intro_bg"), Sense::click()).clicked() {
                done = true;
            }
            let portrait = rect.height() >= rect.width();
            let mid_y = rect.center().y - if portrait { 40.0 } else { 20.0 };
            let travel = (rect.height() * if portrait { 0.12 } else { 0.18 }).min(90.0);
            for (i, (kind, icon, label)) in [(GestureKind::Volume, ph::SPEAKER_HIGH, t!("Lautstärke")), (GestureKind::Brightness, ph::SUN, t!("Helligkeit"))].into_iter().enumerate() {
                let x = if kind == GestureKind::Volume { rect.left() + rect.width() * 0.25 } else { rect.left() + rect.width() * 0.75 };
                // Track with arrows
                p.line_segment([pos2(x, mid_y - travel - 10.0), pos2(x, mid_y + travel + 10.0)], egui::Stroke::new(2.0, a(Color32::from_white_alpha(50))));
                p.text(pos2(x, mid_y - travel - 22.0), Align2::CENTER_CENTER, ph::CARET_UP, theme::icon(18.0), a(Color32::from_white_alpha(160)));
                p.text(pos2(x, mid_y + travel + 22.0), Align2::CENTER_CENTER, ph::CARET_DOWN, theme::icon(18.0), a(Color32::from_white_alpha(160)));
                // Finger moving up and down (the two sides out of phase)
                let ph_ = time * 2.2 + i as f32 * 1.6;
                let y = mid_y + ph_.sin() * travel;
                p.circle_filled(pos2(x, y), 20.0, a(Color32::from_white_alpha(60)));
                p.circle_filled(pos2(x, y), 11.0, a(Color32::WHITE));
                // Level pill next to it, following the finger
                let lvl = 0.5 - ph_.sin() * 0.5;
                let bar = Rect::from_center_size(pos2(x + 54.0, mid_y), vec2(8.0, travel * 2.0));
                p.rect_filled(bar, CornerRadius::same(4), a(Color32::from_white_alpha(40)));
                p.rect_filled(Rect::from_min_max(pos2(bar.left(), bar.bottom() - bar.height() * lvl), bar.max), CornerRadius::same(4), a(t.accent));
                p.text(pos2(x, mid_y + travel + 52.0), Align2::CENTER_CENTER, icon, theme::icon(22.0), a(Color32::WHITE));
                p.text(pos2(x, mid_y + travel + 76.0), Align2::CENTER_CENTER, label, theme::semibold(14.0), a(Color32::WHITE));
            }
            // Title and text
            let tw = (rect.width() - 48.0).min(460.0);
            let top = rect.top() + if portrait { 70.0 } else { 18.0 };
            p.text(pos2(rect.center().x, top), Align2::CENTER_TOP, t!("Neu: Wischgesten"), theme::semibold(20.0), a(Color32::WHITE));
            let text_rect = Rect::from_min_size(pos2(rect.center().x - tw / 2.0, rect.bottom() - if portrait { 210.0 } else { 120.0 }), vec2(tw, 200.0));
            ui.scope_builder(UiBuilder::new().max_rect(text_rect).layout(egui::Layout::top_down(egui::Align::Center)), |ui| {
                ui.multiply_opacity(fade);
                ui.label(egui::RichText::new(t!("Links nach oben oder unten wischen ändert die Lautstärke, rechts die Helligkeit.")).color(Color32::WHITE).size(14.5));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(t!("Ausschalten kannst du das unter Einstellungen → Wiedergabe.")).color(Color32::from_white_alpha(170)).size(13.0));
                ui.add_space(12.0);
                if widgets::accent_button(ui, &t, t!("Verstanden")).clicked() {
                    done = true;
                }
            });
        });
        if done {
            self.data.settings.gesture_intro_seen = true;
            self.mark_dirty();
        }
    }

    fn mobile_player_overlay(&mut self, ui: &mut Ui, rect: Rect) {
        let t = self.t;
        let mm = m();
        let ctx = ui.ctx().clone();
        let Some(st) = self.player.as_ref().map(|p| p.st.clone()) else { return };
        let p = ui.painter().clone();
        let shade = |r: Rect, top_dark: bool| {
            let mut mesh = egui::Mesh::default();
            let (a, b) = if top_dark { (Color32::from_black_alpha(190), Color32::TRANSPARENT) } else { (Color32::TRANSPARENT, Color32::from_black_alpha(200)) };
            mesh.colored_vertex(r.left_top(), a);
            mesh.colored_vertex(r.right_top(), a);
            mesh.colored_vertex(r.right_bottom(), b);
            mesh.colored_vertex(r.left_bottom(), b);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            mesh
        };
        // Shading spans the whole display, the controls only the safe `rect`.
        let full = ui.max_rect();
        if self.mob.locked {
            self.locked_overlay(ui, rect);
            return;
        }
        // Bottom geometry: button row at the bottom (if any buttons are
        // chosen), the seek row above it.
        let tc = self.touch_items();
        let n = (tc.left.len() + tc.right.len()) as f32;
        let bb = mm.icon_btn.min((rect.width() - 16.0) / (n + 1.0));
        let btn_row = Rect::from_min_max(pos2(rect.left() + 8.0, rect.bottom() - 4.0 - bb), pos2(rect.right() - 8.0, rect.bottom() - 4.0));
        let seek_bottom = if n > 0.0 { btn_row.top() + 4.0 } else { rect.bottom() - 10.0 };
        let seek_row = Rect::from_min_max(pos2(rect.left() + 14.0, seek_bottom - mm.seek_h), pos2(rect.right() - 14.0, seek_bottom));
        let top = Rect::from_min_max(full.min, pos2(full.right(), rect.top() + mm.header_h + 30.0));
        let audio = self.mobile_audio_bar();
        if audio {
            // A solid bar instead of shading over the now-playing page.
            let bar = Rect::from_min_max(pos2(full.left(), rect.bottom() - self.mobile_audio_bar_h()), full.max);
            p.rect_filled(bar, CornerRadius::ZERO, t.panel);
            p.hline(bar.x_range(), bar.top(), Stroke::new(1.0, t.border));
        } else {
            let bottom = Rect::from_min_max(pos2(full.left(), seek_row.top() - 50.0), full.max);
            p.add(egui::Shape::mesh(shade(top, true)));
            p.add(egui::Shape::mesh(shade(bottom, false)));
            p.rect_filled(full, CornerRadius::ZERO, Color32::from_black_alpha(60));
        }
        let mut touched = false;

        // Top: back, title, panels
        let b = mm.icon_btn;
        let back = Rect::from_center_size(pos2(rect.left() + 8.0 + b / 2.0, rect.top() + mm.header_h / 2.0), vec2(b, b));
        if touch_icon(ui, back, ph::CARET_DOWN, &t, true).clicked() {
            self.view = View::Library;
            return;
        }
        let queue = Rect::from_center_size(pos2(rect.right() - 8.0 - b / 2.0, back.center().y), vec2(b, b));
        let adjust = queue.translate(vec2(-b - 4.0, 0.0));
        if touch_icon(ui, adjust, ph::SLIDERS_HORIZONTAL, &t, true).clicked() {
            self.right = if self.right == RightPanel::Adjust { RightPanel::None } else { RightPanel::Adjust };
            touched = true;
        }
        if touch_icon(ui, queue, ph::QUEUE, &t, true).clicked() {
            self.right = if self.right == RightPanel::Queue { RightPanel::None } else { RightPanel::Queue };
            touched = true;
        }
        // Sleep timer: moon, with the time left underneath when active.
        let moon = adjust.translate(vec2(-b - 4.0, 0.0));
        let sleeping = self.sleep_active();
        let moon_resp = touch_icon(ui, moon, if sleeping { ph::MOON_STARS } else { ph::MOON }, &t, true);
        if sleeping {
            p.circle_filled(moon.center() + vec2(b * 0.28, -b * 0.28), 4.0, t.accent);
            p.text(pos2(moon.center().x, moon.bottom() + 2.0), Align2::CENTER_TOP, self.sleep_label(), theme::semibold(11.0), Color32::WHITE);
        }
        egui::Popup::menu(&moon_resp).align(RectAlign::BOTTOM_END).width(250.0).show(|ui| {
            // Keep the controls (and so this menu) up while it is open.
            self.bars_shown_at = Instant::now();
            self.sleep_menu(ui);
        });
        if moon_resp.clicked() {
            touched = true;
        }
        widgets::painter_text_trunc(&p, pos2(back.right() + 6.0, back.center().y - 10.0), &st.media_title, theme::semibold(16.0), Color32::WHITE, moon.left() - back.right() - 14.0);

        // Center: previous, play/pause, next. For music in the bar, above the seek row.
        let c = if audio { pos2(rect.center().x, seek_row.top() - 8.0 - mm.play_btn / 2.0) } else { rect.center() };
        let pb = Rect::from_center_size(c, vec2(mm.play_btn, mm.play_btn));
        let gap = mm.play_btn * 0.5 + mm.ctl_btn * 0.5 + 26.0;
        let prev = Rect::from_center_size(c - vec2(gap, 0.0), vec2(mm.ctl_btn, mm.ctl_btn));
        let next = Rect::from_center_size(c + vec2(gap, 0.0), vec2(mm.ctl_btn, mm.ctl_btn));
        let pr = ui.interact(pb, Id::new("m_play"), Sense::click());
        p.circle_filled(pb.center(), mm.play_btn / 2.0, if pr.is_pointer_button_down_on() { t.accent.gamma_multiply(0.8) } else { t.accent });
        p.text(pb.center(), Align2::CENTER_CENTER, if st.paused { ph::PLAY } else { ph::PAUSE }, theme::fill_icon(mm.play_btn * 0.42), Color32::WHITE);
        if pr.clicked() {
            if let Some(pl) = &self.player {
                pl.ctl.toggle_pause();
            }
            touched = true;
        }
        for (r, icon, action) in [(prev, ph::SKIP_BACK, Action::Prev), (next, ph::SKIP_FORWARD, Action::Next)] {
            let resp = ui.interact(r, Id::new(("m_ctl", icon)), Sense::click());
            p.circle_filled(r.center(), mm.ctl_btn / 2.0, Color32::from_black_alpha(if resp.is_pointer_button_down_on() { 150 } else { 90 }));
            p.text(r.center(), Align2::CENTER_CENTER, icon, theme::fill_icon(mm.ctl_icon), Color32::WHITE);
            if resp.clicked() {
                self.run_action(&ctx, action, None);
                touched = true;
            }
        }

        // Bottom, two rows: time · seek bar · duration, and below it the
        // buttons chosen in the settings, in the left and right corner.
        let tc = self.touch_items();
        let mut x = btn_row.left();
        for item in &tc.left {
            let r = Rect::from_min_size(pos2(x, btn_row.top()), vec2(bb, bb));
            x += bb + 4.0;
            touched |= self.touch_item(ui, *item, r, false);
        }
        let mut x = btn_row.right();
        for item in tc.right.iter().rev() {
            let r = Rect::from_min_max(pos2(x - bb, btn_row.top()), pos2(x, btn_row.bottom()));
            x -= bb + 4.0;
            touched |= self.touch_item(ui, *item, r, true);
        }
        let font = FontId::proportional(12.5);
        // Phone style: 4:07, or 1:04:07 for long files.
        let long = st.duration >= 3600.0;
        let clock = |v: f64| {
            let s = if v.is_finite() { v.max(0.0) as u64 } else { 0 };
            if long { format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
        };
        let cur = clock(st.time);
        let dur = clock(st.duration);
        // Both time slots as wide as the duration, so the bar does not jump.
        let tw = p.layout_no_wrap(dur.clone(), font.clone(), Color32::WHITE).size().x.max(28.0);
        p.text(pos2(seek_row.left() + tw / 2.0, seek_row.center().y), Align2::CENTER_CENTER, cur, font.clone(), Color32::WHITE);
        p.text(pos2(seek_row.right() - tw / 2.0, seek_row.center().y), Align2::CENTER_CENTER, dur, font, Color32::from_white_alpha(200));
        let bar = Rect::from_min_max(pos2(seek_row.left() + tw + 10.0, seek_row.top()), pos2(seek_row.right() - tw - 10.0, seek_row.bottom()));
        ui.scope_builder(UiBuilder::new().max_rect(bar), |ui| {
            let before = self.seek_drag.is_some();
            self.seek_bar(ui, bar.width());
            if before || self.seek_drag.is_some() {
                touched = true;
            }
        });
        if touched {
            self.bars_shown_at = Instant::now();
        }
    }

    /// Locked screen: only an unlock pill, shown when tapping the video.
    fn locked_overlay(&mut self, ui: &mut Ui, rect: Rect) {
        let t = self.t;
        let p = ui.painter().clone();
        let label = format!("{}  {}", ph::LOCK_SIMPLE, t!("Entsperren"));
        let font = theme::semibold(14.0);
        let w = p.layout_no_wrap(label.clone(), font.clone(), Color32::WHITE).size().x + 40.0;
        let pill = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 44.0), vec2(w, 46.0));
        let resp = ui.interact(pill, Id::new("m_unlock"), Sense::click());
        p.rect_filled(pill, CornerRadius::same(23), Color32::from_black_alpha(if resp.is_pointer_button_down_on() { 220 } else { 160 }));
        p.rect_stroke(pill, CornerRadius::same(23), Stroke::new(1.0, Color32::from_white_alpha(40)), egui::StrokeKind::Inside);
        p.text(pill.center(), Align2::CENTER_CENTER, label, font, Color32::WHITE);
        if resp.clicked() {
            self.mob.locked = false;
            self.bars_shown_at = Instant::now();
            self.osd(t!("Entsperrt"));
        }
        let _ = t;
    }

    /// One button of the bottom row. Returns true when it was used.
    fn touch_item(&mut self, ui: &mut Ui, item: TouchItem, r: Rect, right_side: bool) -> bool {
        let t = self.t;
        let ctx = ui.ctx().clone();
        let Some(st) = self.player.as_ref().map(|p| p.st.clone()) else { return false };
        let align = if right_side { RectAlign::TOP_END } else { RectAlign::TOP_START };
        let speed_txt;
        let sleeping = self.sleep_active();
        let (label, is_text, active) = match item {
            TouchItem::Repeat => match self.data.settings.repeat {
                RepeatMode::Off => (ph::REPEAT, false, false),
                RepeatMode::All => (ph::REPEAT, false, true),
                RepeatMode::One => (ph::REPEAT_ONCE, false, true),
            },
            TouchItem::Tracks | TouchItem::Subtitles => (touch_item_icon(item), false, st.selected("sub").is_some()),
            TouchItem::Scale => (if self.mob.scale == VideoScale::Fill { ph::ARROWS_IN } else { ph::ARROWS_OUT }, false, self.mob.scale != VideoScale::Fit),
            TouchItem::Speed if (st.speed - 1.0).abs() > 0.01 => {
                speed_txt = format!("{}×", (st.speed * 100.0).round() / 100.0);
                (speed_txt.as_str(), true, true)
            }
            TouchItem::Sleep => (if sleeping { ph::MOON_STARS } else { ph::MOON }, false, sleeping),
            TouchItem::Mute => (if st.muted { ph::SPEAKER_X } else { ph::SPEAKER_HIGH }, false, st.muted),
            TouchItem::Queue => (ph::QUEUE, false, self.right == RightPanel::Queue),
            TouchItem::Adjust => (ph::SLIDERS_HORIZONTAL, false, self.right == RightPanel::Adjust),
            _ => (touch_item_icon(item), false, false),
        };
        let resp = touch_btn(ui, r, Id::new(("m_item", item)), label, is_text, active, &t);
        let clicked = resp.clicked();
        let step = m().seek_step_s as f64;
        match item {
            TouchItem::Lock if clicked => {
                self.mob.locked = true;
                self.mob.gesture = None;
                self.right = RightPanel::None;
                self.osd(t!("Bildschirm gesperrt"));
            }
            TouchItem::Repeat if clicked => {
                let (n, msg) = match self.data.settings.repeat {
                    RepeatMode::Off => (RepeatMode::All, t!("Wiederholen: alle")),
                    RepeatMode::All => (RepeatMode::One, t!("Wiederholen: eine Datei")),
                    RepeatMode::One => (RepeatMode::Off, t!("Wiederholen: aus")),
                };
                self.set_repeat(n);
                self.mark_dirty();
                self.osd(msg);
            }
            TouchItem::Tracks | TouchItem::Audio | TouchItem::Subtitles => {
                let (audio, subs) = (item != TouchItem::Subtitles, item != TouchItem::Audio);
                egui::Popup::menu(&resp).align(align).width(280.0).show(|ui| {
                    self.bars_shown_at = Instant::now();
                    self.mobile_tracks_menu(ui, audio, subs);
                });
            }
            TouchItem::Scale => {
                egui::Popup::menu(&resp).align(align).width(230.0).show(|ui| {
                    self.bars_shown_at = Instant::now();
                    ui.label(egui::RichText::new(t!("Bildgröße")).font(theme::semibold(14.0)).color(t.text));
                    ui.add_space(4.0);
                    for (v, icon, name) in [
                        (VideoScale::Fit, ph::ARROWS_OUT, t!("Einpassen")),
                        (VideoScale::Fill, ph::ARROWS_IN, t!("Bild füllen")),
                        (VideoScale::Stretch, ph::ARROWS_OUT_LINE_HORIZONTAL, t!("Strecken")),
                        (VideoScale::Original, ph::FRAME_CORNERS, t!("Originalgröße")),
                    ] {
                        if widgets::menu_item(ui, &t, icon, name, "", self.mob.scale == v).clicked() {
                            self.mob.scale = v;
                            self.apply_video_scale();
                            self.osd(name);
                            ui.close();
                        }
                    }
                });
            }
            TouchItem::Rotate if clicked => mobile::rotate(&ctx),
            TouchItem::Speed => {
                egui::Popup::menu(&resp).align(align).width(150.0).show(|ui| {
                    self.bars_shown_at = Instant::now();
                    for sp in [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0] {
                        if widgets::menu_item(ui, &t, "", &format!("{sp}×"), "", (st.speed - sp).abs() < 0.01).clicked() {
                            if let Some(pl) = &self.player {
                                pl.set_speed(sp);
                            }
                            ui.close();
                        }
                    }
                });
            }
            TouchItem::Shuffle if clicked => {
                if let Some(pl) = &self.player {
                    pl.playlist_shuffle();
                }
                self.osd(t!("Warteschlange gemischt"));
            }
            TouchItem::Bookmark if clicked => self.add_bookmark(None),
            TouchItem::SeekBack | TouchItem::SeekForward if clicked => {
                let back = item == TouchItem::SeekBack;
                if let Some(pl) = &self.player {
                    pl.ctl.seek_rel(if back { -step } else { step });
                }
                self.osd(format!("{} {}{:.0} s", if back { ph::REWIND } else { ph::FAST_FORWARD }, if back { "−" } else { "+" }, step));
            }
            TouchItem::Prev if clicked => self.run_action(&ctx, Action::Prev, None),
            TouchItem::Next if clicked => self.run_action(&ctx, Action::Next, None),
            TouchItem::Sleep => {
                egui::Popup::menu(&resp).align(align).width(250.0).show(|ui| {
                    self.bars_shown_at = Instant::now();
                    self.sleep_menu(ui);
                });
            }
            TouchItem::Mute if clicked => {
                if let Some(pl) = &self.player {
                    pl.ctl.set_mute(!st.muted);
                }
            }
            TouchItem::Chapters => {
                egui::Popup::menu(&resp).align(align).width(280.0).show(|ui| {
                    self.bars_shown_at = Instant::now();
                    ui.label(egui::RichText::new(t!("Kapitel")).font(theme::semibold(14.0)).color(t.text));
                    ui.add_space(4.0);
                    if st.chapters.is_empty() {
                        ui.label(egui::RichText::new(t!("Keine Kapitel")).color(t.text_dim));
                    }
                    egui::ScrollArea::vertical().max_height(ctx.screen_rect().height() * 0.6).show(ui, |ui| {
                        let cur = st.chapters.iter().rposition(|c| c.time <= st.time + 0.5);
                        for (i, c) in st.chapters.iter().enumerate() {
                            let name = if c.title.is_empty() { format!("{} {}", t!("Kapitel"), i + 1) } else { c.title.clone() };
                            if widgets::menu_item(ui, &t, "", &name, &crate::media::fmt_time(c.time), cur == Some(i)).clicked() {
                                if let Some(pl) = &self.player {
                                    pl.seek_abs(c.time, true);
                                    // a chosen chapter plays
                                    pl.set_pause(false);
                                }
                                ui.close();
                            }
                        }
                    });
                });
            }
            TouchItem::Queue if clicked => {
                self.right = if self.right == RightPanel::Queue { RightPanel::None } else { RightPanel::Queue };
            }
            TouchItem::Adjust if clicked => {
                self.right = if self.right == RightPanel::Adjust { RightPanel::None } else { RightPanel::Adjust };
            }
            _ => {}
        }
        clicked
    }

    /// Applies the phone player's picture size (mpv panscan / keepaspect /
    /// video-unscaled).
    pub fn apply_video_scale(&self) {
        let Some(pl) = &self.player else { return };
        let keep = self.data.settings.keep_aspect;
        let (pan, keep, unscaled) = match self.mob.scale {
            VideoScale::Fit => (0.0, keep, "no"),
            VideoScale::Fill => (1.0, true, "no"),
            VideoScale::Stretch => (0.0, false, "no"),
            VideoScale::Original => (0.0, true, "yes"),
        };
        pl.set("panscan", pan);
        pl.set_str("keepaspect", if keep { "yes" } else { "no" });
        pl.set_str("video-unscaled", unscaled);
    }

    /// Adjustments / queue as a sheet over the video.
    fn mobile_sheet(&mut self, ctx: &egui::Context, sheet: Rect, portrait: bool) {
        let t = self.t;
        let corner = if portrait { CornerRadius { nw: 14, ne: 14, sw: 0, se: 0 } } else { CornerRadius { nw: 14, sw: 14, ne: 0, se: 0 } };
        egui::Area::new(Id::new("m_sheet")).order(Order::Foreground).fixed_pos(sheet.min).show(ctx, |ui| {
            Frame::new().fill(t.panel).corner_radius(corner).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                ui.set_min_size(sheet.size() - vec2(24.0, 16.0));
                ui.set_max_size(sheet.size() - vec2(24.0, 16.0));
                match self.right {
                    RightPanel::Adjust => self.adjust_panel(ui),
                    RightPanel::Queue => self.queue_panel(ui),
                    RightPanel::None => {}
                }
            });
        });
    }
}

/// Where the Adjust/Queue sheet goes inside the safe area: bottom part in
/// portrait, right side in landscape.
fn sheet_rect(rect: Rect) -> Rect {
    if rect.height() >= rect.width() {
        Rect::from_min_max(pos2(rect.left(), rect.top() + rect.height() * 0.38), rect.max)
    } else {
        Rect::from_min_max(pos2(rect.right() - 380.0_f32.min(rect.width() * 0.55), rect.top()), rect.max)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GestureKind {
    Volume,
    Brightness,
}

/// A vertical swipe on the video in progress.
#[derive(Clone, Copy, Debug)]
pub struct Gesture {
    kind: GestureKind,
    start: egui::Pos2,
    start_val: f32,
    /// None until the swipe is clearly vertical (Some(true)) or not.
    vertical: Option<bool>,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum VideoScale {
    #[default]
    Fit,
    Fill,
    Stretch,
    Original,
}

/// Icon of a bottom-row button (settings preview).
pub fn touch_item_icon(item: TouchItem) -> &'static str {
    match item {
        TouchItem::Lock => ph::LOCK_SIMPLE_OPEN,
        TouchItem::Repeat => ph::REPEAT,
        TouchItem::Tracks => ph::CHAT_CENTERED_TEXT,
        TouchItem::Scale => ph::ARROWS_OUT,
        TouchItem::Rotate => ph::DEVICE_ROTATE,
        TouchItem::Speed => ph::SPEEDOMETER,
        TouchItem::Shuffle => ph::SHUFFLE,
        TouchItem::Bookmark => ph::BOOKMARK_SIMPLE,
        TouchItem::SeekBack => ph::CLOCK_COUNTER_CLOCKWISE,
        TouchItem::SeekForward => ph::CLOCK_CLOCKWISE,
        TouchItem::Prev => ph::SKIP_BACK,
        TouchItem::Next => ph::SKIP_FORWARD,
        TouchItem::Sleep => ph::MOON,
        TouchItem::Audio => ph::WAVEFORM,
        TouchItem::Subtitles => ph::SUBTITLES,
        TouchItem::Mute => ph::SPEAKER_HIGH,
        TouchItem::Chapters => ph::LIST_NUMBERS,
        TouchItem::Queue => ph::QUEUE,
        TouchItem::Adjust => ph::SLIDERS_HORIZONTAL,
    }
}

/// Bottom-row button: icon (or short text), accent colored when active.
fn touch_btn(ui: &mut Ui, r: Rect, id: Id, label: &str, is_text: bool, active: bool, t: &theme::Theme) -> egui::Response {
    let resp = ui.interact(r, id, Sense::click());
    if resp.is_pointer_button_down_on() || resp.hovered() && !mobile::touch() {
        ui.painter().circle_filled(r.center(), r.width() / 2.0, Color32::from_white_alpha(40));
    }
    let c = if active { t.accent } else { Color32::WHITE };
    let font = if is_text { theme::semibold(r.width() * 0.32) } else { theme::icon(r.width() * 0.5) };
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, font, c);
    resp
}

/// Phone player state kept in `App`.
#[derive(Default)]
pub struct MobileState {
    pub gesture: Option<Gesture>,
    /// Level indicator: kind, value 0..1, last change.
    pub hud: Option<(GestureKind, f32, Instant)>,
    /// Window brightness set by swiping (None = system).
    pub brightness: Option<f32>,
    /// Picture size (fit, fill, stretch, original).
    pub scale: VideoScale,
    /// Screen lock: touches only bring up the unlock button.
    pub locked: bool,
    /// Plex libraries drawer.
    pub drawer: crate::ui_plex_drawer::Drawer,
    /// Live editor for the bottom row (open while Some).
    pub editor: Option<crate::ui_touch_editor::Editor>,
    /// When playback stopped having media (switch to the next file).
    pub media_lost_at: Option<Instant>,
    /// Last next/previous title, to wait longer for the new one.
    pub skip_at: Option<Instant>,
    /// Playlist position seen last frame.
    pub last_pos: i64,
}

fn egui_popup_open(app: &App) -> bool {
    app.seek_drag.is_some()
}

/// Round icon button with a generous touch target.
fn touch_icon(ui: &mut Ui, r: Rect, icon: &str, t: &theme::Theme, on_video: bool) -> egui::Response {
    let resp = ui.interact(r, Id::new(("m_icon", icon, r.min.x as i32, r.min.y as i32)), Sense::click());
    if resp.is_pointer_button_down_on() || resp.hovered() && !mobile::touch() {
        ui.painter().circle_filled(r.center(), r.width() / 2.0, if on_video { Color32::from_white_alpha(40) } else { t.hover });
    }
    let c = if on_video { Color32::WHITE } else { t.text };
    ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, theme::icon(r.width() * 0.5), c);
    resp
}

fn current_brightness() -> Option<f32> {
    #[cfg(target_os = "android")]
    return crate::platform::android::brightness();
    #[allow(unreachable_code)]
    Some(1.0)
}
