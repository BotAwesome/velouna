//! Plex libraries as a side drawer on phones (instead of chips on the Plex
//! home page): swiped in from the left edge or opened with the button next
//! to the search field. The first time, a short animation shows the swipe.

use crate::app::App;
use crate::library::{LibLoc, PlexLoc, PlexTab};
use crate::mobile;
use crate::theme;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Id, Order, Pos2, Rect, Sense, Stroke, pos2, vec2};
use egui_phosphor::regular as ph;
use std::time::Instant;

/// Touches starting this close to the left edge pull the drawer in.
const EDGE: f32 = 30.0;
const HINT_CYCLE: f32 = 1.3;
const HINT_CYCLES: f32 = 2.0;
/// After the hint the burger button lights up once for this long.
const PULSE: f32 = 1.1;

#[derive(Default)]
pub struct Drawer {
    /// 0 closed … 1 open (animated towards `target` unless dragging).
    open: f32,
    target: bool,
    /// Finger drag in progress: open value and x when it started.
    drag: Option<(f32, f32)>,
    /// First-time hint running since.
    hint_at: Option<Instant>,
    /// Burger button highlight after the hint.
    pulse_at: Option<Instant>,
    /// Burger button rect this frame (set by the search bar).
    pub button: Option<Rect>,
}

impl Drawer {
    pub fn is_open(&self) -> bool {
        self.target || self.open > 0.01
    }
    pub fn close(&mut self) {
        self.target = false;
    }
    pub fn toggle(&mut self) {
        self.target = !self.target;
        self.pulse_at = None;
    }
    /// 0..1 progress of the one-time burger highlight, if running.
    pub fn pulse(&self) -> Option<f32> {
        self.pulse_at.map(|at| at.elapsed().as_secs_f32() / PULSE).filter(|k| *k < 1.0)
    }
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

impl App {
    /// The drawer over the Plex pages; `screen` is the safe area, the edge
    /// strip ends at `content_bottom` (above the tab bar and mini player).
    pub fn plex_drawer(&mut self, ctx: &egui::Context, screen: Rect, content_bottom: f32, on_home: bool) {
        let t = self.t;
        let w = (screen.width() * 0.8).min(300.0);
        let dt = ctx.input(|i| i.stable_dt).min(0.05);
        let mut d = std::mem::take(&mut self.mob.drawer);

        // ------------------------------------------------ first-time hint
        let mut hint = None;
        if on_home && !self.data.settings.plex_drawer_hint_seen && !d.is_open() && d.drag.is_none() && self.dialog.is_none() {
            let at = *d.hint_at.get_or_insert_with(Instant::now);
            let el = at.elapsed().as_secs_f32();
            let pressed = ctx.input(|i| i.pointer.any_pressed());
            if el > HINT_CYCLE * HINT_CYCLES || (pressed && el > 0.4) {
                self.data.settings.plex_drawer_hint_seen = true;
                self.mark_dirty();
                d.hint_at = None;
                d.pulse_at = Some(Instant::now());
            } else {
                hint = Some(el);
                ctx.request_repaint();
            }
        }

        // ------------------------------------------------ edge swipe
        // With gesture navigation the system owns the screen edge for "back":
        // take a 200 dp band in the middle back for the drawer (the most
        // Android allows), as wide as the system's strip, and mark it with a
        // small grip. Above and below it "back" works as usual.
        let gesture = mobile::gesture_edge_left(ctx);
        let win_left = if self.full_screen.is_finite() { self.full_screen.left() } else { screen.left() };
        // Below the header and the search row (burger button), so taps there
        // still reach them.
        let bottom = content_bottom.clamp(screen.top() + 100.0, screen.bottom());
        let top = d.button.take().map(|b| b.bottom() + 8.0).unwrap_or(screen.top()).clamp(screen.top(), bottom - 100.0);
        let strip = if gesture > 0.0 {
            let h = 200.0_f32.min(bottom - top);
            let cy = screen.center().y.clamp(top + h / 2.0, bottom - h / 2.0);
            Rect::from_min_max(pos2(win_left, cy - h / 2.0), pos2(win_left + (gesture + 10.0).max(EDGE), cy + h / 2.0))
        } else {
            Rect::from_min_max(pos2(win_left, top), pos2(screen.left() + EDGE, bottom))
        };
        if !d.is_open() && hint.is_none() {
            egui::Area::new(Id::new("plex_drawer_edge")).order(Order::Foreground).fixed_pos(strip.min).show(ctx, |ui| {
                let r = ui.allocate_rect(strip, Sense::drag());
                if r.drag_started() {
                    d.drag = Some((0.0, ui.input(|i| i.pointer.press_origin()).map(|p| p.x).unwrap_or(strip.left())));
                }
                if gesture > 0.0 {
                    let grip = Rect::from_center_size(pos2(win_left + 4.0, strip.center().y), vec2(4.0, 44.0));
                    ui.painter().rect_filled(grip, CornerRadius::same(2), t.text_faint.gamma_multiply(0.7));
                }
            });
        }
        if gesture > 0.0 && !d.is_open() {
            mobile::exclude_from_back_gesture(ctx, strip);
        }
        // Following the finger (from the edge, the panel or the backdrop).
        if let Some((start_open, start_x)) = d.drag {
            let (pos, down, vx) = ctx.input(|i| (i.pointer.interact_pos(), i.pointer.primary_down(), i.pointer.velocity().x));
            if let Some(p) = pos {
                d.open = (start_open + (p.x - start_x) / w).clamp(0.0, 1.0);
            }
            if !down {
                d.target = if vx.abs() > 300.0 { vx > 0.0 } else { d.open > 0.4 };
                d.drag = None;
                if d.target {
                    self.data.settings.plex_drawer_hint_seen = true;
                }
            }
            ctx.request_repaint();
        } else {
            let goal = if d.target { 1.0 } else { 0.0 };
            if (d.open - goal).abs() > 0.001 {
                d.open += (goal - d.open) * (dt * 14.0).min(1.0);
                if (d.open - goal).abs() < 0.005 {
                    d.open = goal;
                }
                ctx.request_repaint();
            }
        }

        // Hint: the drawer peeks out while a finger slides from the edge.
        let mut finger: Option<(Pos2, f32)> = None;
        let mut shown = d.open;
        if let Some(el) = hint {
            let ph_ = (el % HINT_CYCLE) / HINT_CYCLE;
            let fade_in = ease(ph_ / 0.12);
            let slide = ease((ph_ - 0.15) / 0.45);
            let back = ease((ph_ - 0.72) / 0.2);
            shown = 0.32 * slide * (1.0 - back);
            // In the band that really opens the drawer.
            let y = strip.center().y;
            let alpha = fade_in * (1.0 - ease((ph_ - 0.8) / 0.15));
            finger = Some((pos2(screen.left() + 14.0 + slide * w * 0.32, y), alpha));
        }

        if shown > 0.001 {
            let x0 = screen.left() - w + w * shown;
            let panel = Rect::from_min_size(pos2(x0, screen.top()), vec2(w, screen.height()));
            // Backdrop and panel in one layer, so the panel is always on top
            // (two areas get reordered when the backdrop is tapped, and the
            // backdrop then covered the panel the next time).
            egui::Area::new(Id::new("plex_drawer")).order(Order::Foreground).fixed_pos(screen.min).interactable(hint.is_none()).show(ctx, |ui| {
                ctx.move_to_top(ui.layer_id());
                // Backdrop: tap closes, drag left closes with the finger.
                let r = ui.allocate_rect(screen, Sense::click_and_drag());
                ui.painter().rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha((150.0 * shown) as u8));
                if r.clicked() {
                    d.target = false;
                }
                if r.drag_started() && d.drag.is_none() {
                    d.drag = Some((d.open, ui.input(|i| i.pointer.press_origin()).map(|p| p.x).unwrap_or(0.0)));
                }
                let p = ui.painter().clone();
                // Click too, so taps on the panel's empty parts do not reach
                // the backdrop underneath (which would close the drawer).
                let bg = ui.allocate_rect(panel, Sense::click_and_drag());
                if bg.drag_started() && d.drag.is_none() {
                    d.drag = Some((d.open, ui.input(|i| i.pointer.press_origin()).map(|p| p.x).unwrap_or(0.0)));
                }
                p.rect_filled(panel, CornerRadius { ne: 16, se: 16, nw: 0, sw: 0 }, t.panel);
                p.line_segment([panel.right_top(), panel.right_bottom()], Stroke::new(1.0, t.border));
                // Header
                p.text(pos2(panel.left() + 20.0, panel.top() + 30.0), Align2::LEFT_CENTER, "Plex", theme::semibold(20.0), t.text);
                let server = self.plex.server.as_ref().map(|s| s.name.clone()).unwrap_or_default();
                p.text(pos2(panel.left() + 20.0, panel.top() + 54.0), Align2::LEFT_CENTER, server, FontId::proportional(12.5), t.text_dim);
                let current = match &self.lib.loc {
                    LibLoc::Plex(PlexLoc::Section { key, .. }) => Some(key.clone()),
                    LibLoc::Plex(PlexLoc::Home) => Some(String::new()),
                    _ => None,
                };
                let mut y = panel.top() + 78.0;
                p.text(pos2(panel.left() + 20.0, y + 10.0), Align2::LEFT_CENTER, t!("Mediatheken"), FontId::proportional(12.0), t.text_faint);
                y += 22.0;
                let mut rows: Vec<(&str, String, Option<LibLoc>, bool)> = vec![(ph::HOUSE, t!("Startseite").to_string(), Some(LibLoc::Plex(PlexLoc::Home)), current.as_deref() == Some(""))];
                for s in &self.plex.sections {
                    let loc = LibLoc::Plex(PlexLoc::Section { key: s.key.clone(), title: s.title.clone(), tab: PlexTab::Recommended });
                    rows.push((s.icon(), s.title.clone(), Some(loc), current.as_deref() == Some(s.key.as_str())));
                }
                let row_h = ((panel.bottom() - y - 12.0) / rows.len().max(1) as f32).clamp(36.0, 48.0);
                let mut go = None;
                for (icon, name, loc, sel) in rows {
                    let r = Rect::from_min_size(pos2(panel.left() + 8.0, y), vec2(w - 16.0, row_h - 2.0));
                    y += row_h;
                    if r.top() > panel.bottom() {
                        break;
                    }
                    let resp = ui.interact(r, Id::new(("plex_drawer_row", name.as_str())), Sense::click());
                    if sel {
                        p.rect_filled(r, CornerRadius::same(12), t.accent_soft);
                    } else if resp.is_pointer_button_down_on() {
                        p.rect_filled(r, CornerRadius::same(12), t.hover);
                    }
                    let col = if sel { t.accent } else { t.text };
                    p.text(pos2(r.left() + 22.0, r.center().y), Align2::CENTER_CENTER, icon, theme::icon(19.0), if sel { t.accent } else { t.text_dim });
                    crate::widgets::painter_text_trunc(&p, pos2(r.left() + 46.0, r.center().y - 9.5), &name, FontId::proportional(14.5), col, r.width() - 56.0);
                    if resp.clicked() {
                        go = loc;
                    }
                }
                if let Some(loc) = go {
                    if self.lib.loc != loc {
                        self.lib.go(loc);
                    }
                    d.target = false;
                }
            });
        }

        // Hint finger and caption on top.
        if let (Some((pos, alpha)), Some(_)) = (finger, hint) {
            let p = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, Id::new("plex_drawer_hint")));
            p.circle_filled(pos, 22.0, Color32::from_white_alpha((70.0 * alpha) as u8));
            p.circle_filled(pos, 12.0, Color32::from_white_alpha((235.0 * alpha) as u8));
            let text = t!("Vom linken Rand wischen: deine Mediatheken");
            let font = theme::semibold(14.0);
            let galley = p.layout(text.to_string(), font, Color32::WHITE, (screen.width() - 120.0).min(260.0));
            let bubble = Rect::from_min_size(pos2(screen.left() + w * 0.32 + 48.0, pos.y - galley.size().y / 2.0 - 12.0), galley.size() + vec2(28.0, 24.0));
            p.rect_filled(bubble, CornerRadius::same(12), t.accent);
            p.galley(bubble.min + vec2(14.0, 12.0), galley, Color32::WHITE);
        }

        self.mob.drawer = d;
    }
}
