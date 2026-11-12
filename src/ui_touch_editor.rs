//! Live editor for the phone player's bottom row: the player as a preview
//! on the right, every available element in a sidebar on the left. Elements
//! are dragged into the row (left or right corner), moved around in it, or
//! dragged out / tapped to remove them.

use crate::app::App;
use crate::config::{TOUCH_CATEGORIES, TouchControls, TouchItem};
use crate::mobile::{self, m};
use crate::theme;
use crate::ui_mobile::touch_item_icon;
use crate::widgets;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Id, LayerId, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as ph;

/// Most elements per corner.
const MAX_SIDE: usize = 5;
const ROW_H: f32 = 46.0;
const HEAD_H: f32 = 30.0;

#[derive(Default)]
pub struct Editor {
    /// Sidebar scroll offset.
    scroll: f32,
    /// The element being dragged and where the finger is.
    drag: Option<(TouchItem, Pos2)>,
}

/// Bottom-area geometry, the same as in the real player overlay.
struct Geo {
    btn_row: Rect,
    seek_row: Rect,
    bb: f32,
}

fn geo(rect: Rect, tc: &TouchControls) -> Geo {
    let mm = m();
    let n = (tc.left.len() + tc.right.len()).max(1) as f32;
    let bb = mm.icon_btn.min((rect.width() - 16.0) / (n + 1.0));
    let btn_row = Rect::from_min_max(pos2(rect.left() + 8.0, rect.bottom() - 4.0 - mm.icon_btn), pos2(rect.right() - 8.0, rect.bottom() - 4.0));
    let seek_bottom = btn_row.top() + 4.0;
    let seek_row = Rect::from_min_max(pos2(rect.left() + 14.0, seek_bottom - mm.seek_h), pos2(rect.right() - 14.0, seek_bottom));
    Geo { btn_row, seek_row, bb }
}

/// Where each placed element sits: (item, right side, index, rect).
fn item_rects(g: &Geo, tc: &TouchControls) -> Vec<(TouchItem, bool, usize, Rect)> {
    let mut out = vec![];
    let y = g.btn_row.center().y;
    for (i, it) in tc.left.iter().enumerate() {
        let x = g.btn_row.left() + i as f32 * (g.bb + 4.0) + g.bb / 2.0;
        out.push((*it, false, i, Rect::from_center_size(pos2(x, y), vec2(g.bb, g.bb))));
    }
    let n = tc.right.len();
    for (i, it) in tc.right.iter().enumerate() {
        let k = (n - 1 - i) as f32;
        let x = g.btn_row.right() - k * (g.bb + 4.0) - g.bb / 2.0;
        out.push((*it, true, i, Rect::from_center_size(pos2(x, y), vec2(g.bb, g.bb))));
    }
    out
}

fn without(tc: &TouchControls, item: TouchItem) -> TouchControls {
    let mut r = tc.clone();
    r.left.retain(|x| *x != item);
    r.right.retain(|x| *x != item);
    r
}

/// Drop target for a finger at `p`: Some((right side, index)), or None when
/// outside the row. `Err` when that corner is full.
fn slot(prev: Rect, g: &Geo, rest: &TouchControls, p: Pos2) -> Option<Result<(bool, usize), bool>> {
    if !prev.contains(p) || p.y < g.seek_row.top() - 24.0 {
        return None;
    }
    let right = p.x > g.btn_row.center().x;
    let len = if right { rest.right.len() } else { rest.left.len() };
    if len >= MAX_SIDE {
        return Some(Err(right));
    }
    let idx = item_rects(g, rest).iter().filter(|(_, r, _, rc)| *r == right && rc.center().x < p.x).count();
    Some(Ok((right, idx)))
}

fn insert(tc: &mut TouchControls, item: TouchItem, right: bool, idx: usize) {
    let v = if right { &mut tc.right } else { &mut tc.left };
    let idx = idx.min(v.len());
    v.insert(idx, item);
}

impl App {
    pub fn open_touch_editor(&mut self, ctx: &egui::Context) {
        self.mob.editor = Some(Editor::default());
        mobile::force_landscape(ctx);
    }

    pub fn close_touch_editor(&mut self) {
        self.mob.editor = None;
        mobile::release_rotation();
    }

    pub fn touch_editor(&mut self, ui: &mut Ui) {
        let t = self.t;
        let Some(mut ed) = self.mob.editor.take() else { return };
        let full = ui.max_rect();
        let side_w = (full.width() * 0.34).clamp(150.0, 250.0);
        let side = Rect::from_min_size(full.min, vec2(side_w, full.height()));
        let prev = Rect::from_min_max(pos2(side.right(), full.top()), full.max);
        let mut tc = self.data.settings.touch_controls.clone();
        let (pointer, down, released) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_down(), i.pointer.any_released()));
        let mut close = false;

        // Finger moved while dragging; dropped when released.
        if let Some((item, pos)) = &mut ed.drag {
            if let Some(p) = pointer {
                *pos = p;
            }
            if released || !down {
                let item = *item;
                let at = *pos;
                let rest = without(&tc, item);
                let g = geo(prev, &rest);
                match slot(prev, &g, &rest, at) {
                    Some(Ok((right, idx))) => {
                        tc = rest;
                        insert(&mut tc, item, right, idx);
                    }
                    Some(Err(_)) => self.osd(t!("Kein Platz mehr in dieser Ecke")),
                    // Dragged out of the row: removed.
                    None => tc = rest,
                }
                ed.drag = None;
            }
        }

        // ---------------------------------------------------------- preview
        let p = ui.painter().clone();
        p.rect_filled(prev, CornerRadius::ZERO, t.video_bg);
        let has_video = self.player.as_ref().is_some_and(|pl| pl.st.has_video());
        let vid = Rect::from_center_size(prev.center(), {
            let w = prev.width().min(prev.height() * 16.0 / 9.0);
            vec2(w, w * 9.0 / 16.0)
        });
        match (&self.video, has_video) {
            (Some(v), true) => v.paint(&p, vid),
            _ => {
                let mut mesh = egui::Mesh::default();
                let (a, b) = (Color32::from_rgb(0x24, 0x22, 0x33), Color32::from_rgb(0x3a, 0x26, 0x2c));
                mesh.colored_vertex(vid.left_top(), a);
                mesh.colored_vertex(vid.right_top(), a);
                mesh.colored_vertex(vid.right_bottom(), b);
                mesh.colored_vertex(vid.left_bottom(), b);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                p.add(egui::Shape::mesh(mesh));
                // Keep mpv's render context alive (see draw_mobile).
                if let Some(v) = &self.video {
                    v.paint(&p, Rect::from_min_size(full.min, vec2(1.0, 1.0)));
                }
            }
        }
        p.rect_filled(prev, CornerRadius::ZERO, Color32::from_black_alpha(110));
        let mm = m();
        let dim = Color32::from_white_alpha(90);
        // Top bar (decoration) and the editor's own buttons
        p.text(pos2(prev.left() + 8.0 + mm.icon_btn / 2.0, prev.top() + mm.header_h / 2.0), Align2::CENTER_CENTER, ph::CARET_DOWN, theme::icon(mm.icon_btn * 0.5), dim);
        let title = self.player.as_ref().map(|pl| pl.st.media_title.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| t!("Vorschau").into());
        widgets::painter_text_trunc(&p, pos2(prev.left() + 14.0 + mm.icon_btn, prev.top() + mm.header_h / 2.0 - 10.0), &title, theme::semibold(15.0), dim, prev.width() * 0.35);
        let done = Rect::from_min_size(pos2(prev.right() - 12.0 - 104.0, prev.top() + 9.0), vec2(104.0, 36.0));
        let reset = Rect::from_min_size(pos2(done.left() - 8.0 - 44.0, done.top()), vec2(44.0, 36.0));
        let done_r = ui.interact(done, Id::new("ed_done"), Sense::click());
        p.rect_filled(done, CornerRadius::same(18), if done_r.is_pointer_button_down_on() { t.accent.gamma_multiply(0.8) } else { t.accent });
        p.text(done.center(), Align2::CENTER_CENTER, format!("{}  {}", ph::CHECK, t!("Fertig")), theme::semibold(14.0), Color32::WHITE);
        if done_r.clicked() {
            close = true;
        }
        let reset_r = ui.interact(reset, Id::new("ed_reset"), Sense::click()).on_hover_text(t!("Zurücksetzen"));
        p.rect_filled(reset, CornerRadius::same(18), Color32::from_white_alpha(if reset_r.is_pointer_button_down_on() { 70 } else { 35 }));
        p.text(reset.center(), Align2::CENTER_CENTER, ph::ARROW_COUNTER_CLOCKWISE, theme::icon(18.0), Color32::WHITE);
        if reset_r.clicked() {
            tc = TouchControls::default();
            self.osd(t!("Standard wiederhergestellt"));
        }
        // Center controls (decoration)
        let c = pos2(prev.center().x, prev.top() + prev.height() * 0.42);
        p.circle_filled(c, mm.play_btn / 2.0, t.accent.gamma_multiply(0.45));
        p.text(c, Align2::CENTER_CENTER, ph::PAUSE, theme::fill_icon(mm.play_btn * 0.42), dim);
        let gap = mm.play_btn * 0.5 + mm.ctl_btn * 0.5 + 26.0;
        for (dx, icon) in [(-gap, ph::SKIP_BACK), (gap, ph::SKIP_FORWARD)] {
            p.circle_filled(c + vec2(dx, 0.0), mm.ctl_btn / 2.0, Color32::from_black_alpha(70));
            p.text(c + vec2(dx, 0.0), Align2::CENTER_CENTER, icon, theme::fill_icon(mm.ctl_icon), dim);
        }
        p.text(
            pos2(prev.center().x, c.y + mm.play_btn / 2.0 + 22.0),
            Align2::CENTER_TOP,
            t!("Elemente aus der Liste in die Leiste ziehen. Antippen entfernt sie."),
            FontId::proportional(13.0),
            Color32::from_white_alpha(200),
        );

        // Live layout: while dragging, the others make room at the target.
        let dragging = ed.drag.map(|(it, _)| it);
        let target = ed.drag.and_then(|(it, at)| {
            let rest = without(&tc, it);
            slot(prev, &geo(prev, &rest), &rest, at)
        });
        let mut shown = match dragging {
            Some(it) => without(&tc, it),
            None => tc.clone(),
        };
        let ghost_at = match (dragging, target) {
            (Some(it), Some(Ok((right, idx)))) => {
                insert(&mut shown, it, right, idx);
                Some(it)
            }
            _ => None,
        };
        let g = geo(prev, &shown);
        // Seek row
        let font = FontId::proportional(12.5);
        p.text(pos2(g.seek_row.left() + 16.0, g.seek_row.center().y), Align2::CENTER_CENTER, "9:18", font.clone(), Color32::WHITE);
        p.text(pos2(g.seek_row.right() - 16.0, g.seek_row.center().y), Align2::CENTER_CENTER, "23:40", font, Color32::from_white_alpha(200));
        let track = Rect::from_center_size(g.seek_row.center(), vec2(g.seek_row.width() - 84.0, 6.0));
        p.rect_filled(track, CornerRadius::same(3), Color32::from_white_alpha(50));
        p.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * 0.38, track.height())), CornerRadius::same(3), t.accent);
        for f in [0.12, 0.55, 0.8] {
            let x = track.left() + track.width() * f;
            p.line_segment([pos2(x, track.top()), pos2(x, track.bottom())], Stroke::new(2.0, Color32::WHITE));
        }
        // Drop zones: left and right half of the button row
        let mid = g.btn_row.center().x;
        for right in [false, true] {
            let zone = if right {
                Rect::from_min_max(pos2(mid + 10.0, g.btn_row.top() - 2.0), pos2(g.btn_row.right() + 4.0, g.btn_row.bottom() + 2.0))
            } else {
                Rect::from_min_max(pos2(g.btn_row.left() - 4.0, g.btn_row.top() - 2.0), pos2(mid - 10.0, g.btn_row.bottom() + 2.0))
            };
            let (fill, stroke) = match target {
                Some(Ok((r, _))) if r == right => (t.accent.gamma_multiply(0.22), Stroke::new(1.5, t.accent)),
                Some(Err(r)) if r == right => (Color32::from_rgba_unmultiplied(200, 40, 40, 50), Stroke::new(1.5, Color32::from_rgb(220, 70, 70))),
                _ if dragging.is_some() => (Color32::from_white_alpha(14), Stroke::new(1.0, Color32::from_white_alpha(90))),
                _ => (Color32::TRANSPARENT, Stroke::new(1.0, Color32::from_white_alpha(45))),
            };
            p.rect_filled(zone, CornerRadius::same(10), fill);
            p.rect_stroke(zone, CornerRadius::same(10), stroke, StrokeKind::Inside);
            let empty = if right { shown.right.is_empty() } else { shown.left.is_empty() };
            if empty {
                p.text(zone.center(), Align2::CENTER_CENTER, if right { t!("Rechts") } else { t!("Links") }, FontId::proportional(12.0), Color32::from_white_alpha(110));
            }
        }
        // Placed elements: drag to move or out to remove, tap to remove.
        for (item, _, _, r) in item_rects(&g, &shown) {
            if Some(item) == ghost_at {
                p.circle_stroke(r.center(), g.bb / 2.0 - 2.0, Stroke::new(1.5, t.accent));
                p.text(r.center(), Align2::CENTER_CENTER, touch_item_icon(item), theme::icon(g.bb * 0.5), t.accent.gamma_multiply(0.6));
                continue;
            }
            let resp = ui.interact(r, Id::new(("ed_placed", item)), Sense::click_and_drag());
            if resp.drag_started() && ed.drag.is_none() {
                ed.drag = Some((item, pointer.unwrap_or(r.center())));
            } else if resp.clicked() {
                tc = without(&tc, item);
            }
            p.circle_filled(r.center(), g.bb / 2.0 - 1.0, Color32::from_white_alpha(if resp.is_pointer_button_down_on() { 60 } else { 26 }));
            p.text(r.center(), Align2::CENTER_CENTER, touch_item_icon(item), theme::icon(g.bb * 0.5), Color32::WHITE);
            let badge = r.right_top() + vec2(-5.0, 5.0);
            p.circle_filled(badge, 8.0, t.accent);
            p.text(badge, Align2::CENTER_CENTER, ph::X, theme::icon(10.0), Color32::WHITE);
        }

        // ---------------------------------------------------------- sidebar
        p.rect_filled(side, CornerRadius::ZERO, t.panel);
        p.line_segment([side.right_top(), side.right_bottom()], Stroke::new(1.0, t.border));
        p.text(pos2(side.left() + 16.0, side.top() + 26.0), Align2::LEFT_CENTER, t!("Elemente"), theme::semibold(16.0), t.text);
        let list = Rect::from_min_max(pos2(side.left(), side.top() + 50.0), side.max);
        let content_h: f32 = TOUCH_CATEGORIES.iter().map(|(_, items)| HEAD_H + items.len() as f32 * ROW_H).sum::<f32>() + 16.0;
        let max_scroll = (content_h - list.height()).max(0.0);
        // Scrolling: wheel, or a vertical drag anywhere in the list.
        let bg = ui.interact(list, Id::new("ed_list"), Sense::drag());
        if bg.dragged() {
            ed.scroll -= bg.drag_delta().y;
        }
        if ui.rect_contains_pointer(list) {
            ed.scroll -= ui.input(|i| i.smooth_scroll_delta.y);
        }
        ed.scroll = ed.scroll.clamp(0.0, max_scroll);
        let lp = p.with_clip_rect(list);
        let mut y = list.top() - ed.scroll;
        for (cat, items) in TOUCH_CATEGORIES {
            lp.text(pos2(list.left() + 16.0, y + HEAD_H - 8.0), Align2::LEFT_BOTTOM, crate::i18n::tr(cat), FontId::proportional(12.0), t.text_faint);
            y += HEAD_H;
            for item in items.iter().copied() {
                let r = Rect::from_min_size(pos2(list.left() + 6.0, y), vec2(list.width() - 12.0, ROW_H - 4.0));
                y += ROW_H;
                if r.bottom() < list.top() || r.top() > list.bottom() {
                    continue;
                }
                let placed = tc.left.contains(&item) || tc.right.contains(&item);
                let resp = ui.interact(r.intersect(list), Id::new(("ed_side", item)), Sense::click_and_drag());
                if resp.drag_started() && ed.drag.is_none() {
                    // Mostly sideways: pick the element up; up/down: scroll.
                    let d = ui.input(|i| i.pointer.press_origin().zip(i.pointer.interact_pos()).map(|(a, b)| b - a)).unwrap_or_default();
                    if d.x.abs() >= d.y.abs() && !placed {
                        ed.drag = Some((item, pointer.unwrap_or(r.center())));
                    }
                }
                if resp.dragged() && ed.drag.is_none() {
                    ed.scroll = (ed.scroll - resp.drag_delta().y).clamp(0.0, max_scroll);
                }
                if resp.clicked() {
                    if placed {
                        tc = without(&tc, item);
                    } else {
                        // Tap: into the emptier corner, at its inner end.
                        let right = tc.right.len() < tc.left.len();
                        let (right, full) = match (right, tc.left.len() >= MAX_SIDE, tc.right.len() >= MAX_SIDE) {
                            (false, true, false) => (true, false),
                            (true, false, true) => (false, false),
                            (r, a, b) => (r, if r { b } else { a }),
                        };
                        if full {
                            self.osd(t!("Kein Platz mehr in dieser Ecke"));
                        } else if right {
                            tc.right.insert(0, item);
                        } else {
                            tc.left.push(item);
                        }
                    }
                }
                let pressed = resp.is_pointer_button_down_on() || dragging == Some(item);
                if pressed {
                    lp.rect_filled(r, CornerRadius::same(theme::R), t.hover);
                }
                let (ic, tx) = if placed { (t.text_faint, t.text_faint) } else { (t.accent, t.text) };
                lp.text(pos2(r.left() + 20.0, r.center().y), Align2::CENTER_CENTER, touch_item_icon(item), theme::icon(19.0), ic);
                let lw = r.width() - 64.0;
                let galley = lp.layout(item.label().to_string(), FontId::proportional(13.5), tx, lw);
                lp.galley(pos2(r.left() + 42.0, r.center().y - galley.size().y / 2.0), galley, tx);
                if placed {
                    lp.text(pos2(r.right() - 14.0, r.center().y), Align2::CENTER_CENTER, ph::CHECK, theme::icon(15.0), t.accent);
                } else {
                    lp.text(pos2(r.right() - 14.0, r.center().y), Align2::CENTER_CENTER, ph::DOTS_SIX_VERTICAL, theme::icon(15.0), t.text_faint);
                }
            }
        }
        // Scroll hint at the bottom edge
        if ed.scroll < max_scroll - 1.0 {
            let fade = Rect::from_min_max(pos2(list.left(), list.bottom() - 28.0), list.max);
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(fade.left_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(fade.right_top(), Color32::TRANSPARENT);
            mesh.colored_vertex(fade.right_bottom(), t.panel);
            mesh.colored_vertex(fade.left_bottom(), t.panel);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            p.add(egui::Shape::mesh(mesh));
        }

        // The picked-up element follows the finger (a bit above it, so the
        // finger does not hide it).
        if let Some((item, at)) = ed.drag {
            let fp = ui.ctx().layer_painter(LayerId::new(Order::Tooltip, Id::new("ed_ghost")));
            let c = at - vec2(0.0, 34.0);
            let col = match target {
                Some(Ok(_)) => t.accent,
                Some(Err(_)) => Color32::from_rgb(200, 60, 60),
                None if self.data.settings.touch_controls.left.contains(&item) || self.data.settings.touch_controls.right.contains(&item) => Color32::from_rgb(120, 120, 120),
                None => t.accent.gamma_multiply(0.7),
            };
            fp.circle_filled(c, 26.0, col);
            fp.text(c, Align2::CENTER_CENTER, touch_item_icon(item), theme::icon(24.0), Color32::WHITE);
            fp.text(c + vec2(0.0, 34.0), Align2::CENTER_TOP, item.label(), theme::semibold(12.0), Color32::WHITE);
            ui.ctx().request_repaint();
        }

        if tc != self.data.settings.touch_controls {
            self.data.settings.touch_controls = tc;
            self.mark_dirty();
        }
        if close {
            self.close_touch_editor();
        } else {
            self.mob.editor = Some(ed);
        }
    }
}
