//! Custom widgets in the redesign style.

use crate::theme::{self, Theme};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind,
    Ui, Vec2, pos2, vec2,
};

/// Square icon button with subtle hover background.
pub fn icon_button(ui: &mut Ui, t: &Theme, icon: &str, tip: &str, active: bool) -> Response {
    icon_button_sized(ui, t, icon, tip, active, 30.0, 17.0)
}

pub fn icon_button_sized(ui: &mut Ui, t: &Theme, icon: &str, tip: &str, active: bool, box_size: f32, icon_size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(box_size), Sense::click());
    let hovered = resp.hovered();
    if hovered || resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, CornerRadius::same(theme::R), t.hover);
    }
    let color = if active {
        t.accent
    } else if hovered {
        t.text
    } else {
        t.text_dim
    };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, theme::icon(icon_size), color);
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// The big orange play/pause button.
pub fn play_button(ui: &mut Ui, t: &Theme, playing: bool, h: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(h * 1.55, h), Sense::click());
    let fill = if resp.hovered() { t.accent.gamma_multiply(1.12) } else { t.accent };
    ui.painter().rect_filled(rect, CornerRadius::same(theme::R), fill);
    let icon = if playing { egui_phosphor::regular::PAUSE } else { egui_phosphor::regular::PLAY };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, theme::fill_icon(h * 0.55), t.on_accent);
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(if playing { "Pause" } else { t!("Wiedergabe") })
}

pub fn accent_button(ui: &mut Ui, t: &Theme, text: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::semibold(13.5), t.on_accent);
    let size = vec2(galley.size().x + 28.0, ui.spacing().interact_size.y.max(30.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() { t.accent.gamma_multiply(1.1) } else { t.accent };
    ui.painter().rect_filled(rect, CornerRadius::same(theme::R), fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.on_accent);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn ghost_button(ui: &mut Ui, t: &Theme, text: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::semibold(13.0), t.text);
    let size = vec2(galley.size().x + 24.0, ui.spacing().interact_size.y.max(30.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() { t.hover } else { t.elevated };
    ui.painter().rect_filled(rect, CornerRadius::same(theme::R), fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.text);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Icon + label button used in empty states and lists.
pub fn icon_text_button(ui: &mut Ui, t: &Theme, icon: &str, text: &str) -> Response {
    let font = FontId::proportional(13.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, t.text);
    let size = vec2(galley.size().x + 44.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() { t.hover } else { t.elevated };
    ui.painter().rect_filled(rect, CornerRadius::same(theme::R), fill);
    ui.painter().text(pos2(rect.left() + 18.0, rect.center().y), Align2::CENTER_CENTER, icon, theme::icon(16.0), t.accent);
    ui.painter().galley(pos2(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0), galley, t.text);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Full-width menu row with icon, label and optional shortcut / check mark.
pub fn menu_item(ui: &mut Ui, t: &Theme, icon: &str, label: &str, right: &str, checked: bool) -> Response {
    let w = ui.available_width().clamp(200.0, 340.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(theme::R), t.hover);
    }
    let p = ui.painter();
    if !icon.is_empty() {
        p.text(pos2(rect.left() + 16.0, rect.center().y), Align2::CENTER_CENTER, icon, theme::icon(16.0), t.text_dim);
    }
    p.text(pos2(rect.left() + 34.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.5), t.text);
    if checked {
        p.text(pos2(rect.right() - 14.0, rect.center().y), Align2::CENTER_CENTER, egui_phosphor::regular::CHECK, theme::icon(15.0), t.accent);
    } else if !right.is_empty() {
        p.text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, right, FontId::proportional(12.0), t.text_faint);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Selectable row with a check mark on the left (track lists).
pub fn check_row(ui: &mut Ui, t: &Theme, label: &str, checked: bool) -> Response {
    let w = ui.available_width().clamp(160.0, 340.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(theme::R), t.hover);
    }
    if checked {
        ui.painter().text(pos2(rect.left() + 14.0, rect.center().y), Align2::CENTER_CENTER, egui_phosphor::regular::CHECK, theme::icon(15.0), t.accent);
    }
    let galley = ui.painter().layout(label.to_string(), FontId::proportional(13.0), if checked { t.text } else { t.text_dim }, w - 40.0);
    let galley_h = galley.size().y;
    ui.painter().galley(pos2(rect.left() + 30.0, rect.center().y - galley_h / 2.0), galley, t.text);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn section_label(ui: &mut Ui, t: &Theme, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text).size(12.0).color(t.text_faint));
    ui.add_space(2.0);
}

pub fn heading(ui: &mut Ui, t: &Theme, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::semibold(15.0)).color(t.text));
}

/// Tabs with orange underline. Returns true if selection changed.
pub fn tabs<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, current: &mut T, items: &[(T, &str, &str)]) -> bool {
    let mut changed = false;
    let avail = ui.available_width();
    let n = items.len().max(1) as f32;
    let font = FontId::proportional(12.5);
    let content: Vec<f32> = items
        .iter()
        .map(|(_, icon, label)| {
            let tw = ui.painter().layout_no_wrap(label.to_string(), font.clone(), t.text).size().x;
            tw + if icon.is_empty() { 0.0 } else { 20.0 } + 14.0
        })
        .collect();
    let total: f32 = content.iter().sum();
    let icon_only = total > avail;
    let widths: Vec<f32> = if icon_only {
        vec![avail / n; items.len()]
    } else {
        content.iter().map(|c| c + (avail - total) / n).collect()
    };
    let (rect, _) = ui.allocate_exact_size(vec2(avail, 38.0), Sense::hover());
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.border));
    let mut x_acc = rect.left();
    for (i, (val, icon, label)) in items.iter().enumerate() {
        let w = widths[i];
        let r = Rect::from_min_size(pos2(x_acc, rect.top()), vec2(w, rect.height()));
        x_acc += w;
        let full: &str = label;
        let label: &str = if icon_only && !icon.is_empty() { "" } else { label };
        let mut resp = ui.interact(r, ui.id().with(("tab", i)), Sense::click());
        if icon_only {
            resp = resp.on_hover_text(full);
        }
        let icon_w_full = if icon.is_empty() || label.is_empty() { 0.0 } else { 20.0 };
        let _ = icon_w_full;
        let sel = *current == *val;
        let col = if sel { t.text } else if resp.hovered() { t.text } else { t.text_dim };
        let icon_col = if sel { t.accent } else { col };
        let font = FontId::proportional(12.5);
        let galley = ui.painter().layout_no_wrap(label.to_string(), font, col);
        let icon_w = if icon.is_empty() { 0.0 } else if label.is_empty() { 14.0 } else { 20.0 };
        let total = icon_w + galley.size().x;
        let x0 = r.center().x - total / 2.0;
        if !icon.is_empty() {
            ui.painter().text(pos2(x0 + 7.0, r.center().y), Align2::CENTER_CENTER, *icon, theme::icon(15.0), icon_col);
        }
        ui.painter().galley(pos2(x0 + icon_w, r.center().y - galley.size().y / 2.0), galley, col);
        if sel {
            let ul = Rect::from_min_max(pos2(r.left() + 6.0, r.bottom() - 2.0), pos2(r.right() - 6.0, r.bottom()));
            ui.painter().rect_filled(ul, CornerRadius::ZERO, t.accent);
        }
        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() && !sel {
            *current = *val;
            changed = true;
        }
    }
    changed
}

/// Segmented control (e.g. Relative | Pixels).
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, current: &mut T, items: &[(T, &str)]) -> bool {
    let mut changed = false;
    let font = FontId::proportional(12.5);
    let widths: Vec<f32> = items
        .iter()
        .map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), t.text).size().x + 22.0)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (rect, base) = ui.allocate_exact_size(vec2(total, 28.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(theme::R), t.track);
    let mut x = rect.left() + 2.0;
    for (i, (val, label)) in items.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(widths[i], rect.height() - 4.0));
        x += widths[i];
        let resp = ui.interact(r, base.id.with(("seg", i)), Sense::click());
        let sel = *current == *val;
        if sel {
            ui.painter().rect_filled(r, CornerRadius::same(3), t.accent_soft);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(3), t.hover);
        }
        let col = if sel { t.accent } else { t.text_dim };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *label, font.clone(), col);
        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() && !sel {
            *current = *val;
            changed = true;
        }
    }
    changed
}

/// iOS-ish toggle switch in accent color.
pub fn toggle(ui: &mut Ui, t: &Theme, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let k = ui.ctx().animate_bool_with_time(resp.id, *on, 0.12);
    let bg = if *on { t.accent } else { t.hover };
    ui.painter().rect_filled(rect, CornerRadius::same(10), bg);
    let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, k);
    let knob = if *on { t.on_accent } else { t.text_dim };
    ui.painter().circle_filled(pos2(x, rect.center().y), 7.0, knob);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Thick orange slider with a pill handle (matches the seek bar look).
pub fn slider(ui: &mut Ui, t: &Theme, value: &mut f64, min: f64, max: f64, width: f32) -> Response {
    slider_ex(ui, t, value, min, max, width, None, &[])
}

pub fn slider_ex(
    ui: &mut Ui,
    t: &Theme,
    value: &mut f64,
    min: f64,
    max: f64,
    width: f32,
    origin: Option<f64>,
    ticks: &[f64],
) -> Response {
    let h = 22.0;
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, h), Sense::click_and_drag());
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 8.0, 10.0));
    let to_x = |v: f64| track.left() + ((v - min) / (max - min)).clamp(0.0, 1.0) as f32 * track.width();
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() || resp.drag_started() {
            let f = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0) as f64;
            let mut nv = min + f * (max - min);
            // Snap to ticks when close.
            for &tk in ticks {
                if ((to_x(tk) - p.x).abs()) < 5.0 {
                    nv = tk;
                }
            }
            if (nv - *value).abs() > f64::EPSILON {
                *value = nv;
                resp.mark_changed();
            }
        }
    }
    if resp.double_clicked() {
        if let Some(o) = origin {
            *value = o;
            resp.mark_changed();
        }
    }
    let p = ui.painter();
    p.rect_filled(track, CornerRadius::same(3), t.track);
    let x = to_x(*value);
    let from = origin.map(to_x).unwrap_or(track.left());
    let fill = Rect::from_min_max(pos2(from.min(x), track.top()), pos2(from.max(x), track.bottom()));
    p.rect_filled(fill, CornerRadius::same(3), t.accent_dim);
    for &tk in ticks {
        let tx = to_x(tk);
        p.line_segment([pos2(tx, track.top() + 3.0), pos2(tx, track.bottom() - 3.0)], Stroke::new(1.0, t.text_faint.gamma_multiply(0.6)));
    }
    let handle = Rect::from_center_size(pos2(x, track.center().y), vec2(6.0, 18.0));
    let hc = if resp.hovered() || resp.dragged() { t.accent.gamma_multiply(1.15) } else { t.accent };
    p.rect_filled(handle, CornerRadius::same(3), hc);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Vertical slider for the equalizer.
pub fn vslider(ui: &mut Ui, t: &Theme, value: &mut f64, min: f64, max: f64, height: f32) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(22.0, height), Sense::click_and_drag());
    let track = Rect::from_center_size(rect.center(), vec2(8.0, rect.height() - 8.0));
    let to_y = |v: f64| track.bottom() - ((v - min) / (max - min)).clamp(0.0, 1.0) as f32 * track.height();
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() {
            let f = ((track.bottom() - p.y) / track.height()).clamp(0.0, 1.0) as f64;
            *value = ((min + f * (max - min)) * 2.0).round() / 2.0;
            resp.mark_changed();
        }
    }
    if resp.double_clicked() {
        *value = 0.0;
        resp.mark_changed();
    }
    let p = ui.painter();
    p.rect_filled(track, CornerRadius::same(3), t.track);
    let y = to_y(*value);
    let y0 = to_y(0.0);
    p.rect_filled(Rect::from_min_max(pos2(track.left(), y.min(y0)), pos2(track.right(), y.max(y0))), CornerRadius::same(3), t.accent_dim);
    p.rect_filled(Rect::from_center_size(pos2(track.center().x, y), vec2(16.0, 6.0)), CornerRadius::same(3), t.accent);
    resp
}

/// Small rounded value box like "-2.000 s".
pub fn value_box(ui: &mut Ui, t: &Theme, text: &str, width: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    ui.painter().rect_filled(rect, CornerRadius::same(theme::R), t.track);
    ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.5), t.text);
    resp
}

pub fn key_chip(ui: &mut Ui, t: &Theme, text: &str) {
    let font = theme::semibold(12.0);
    let g = ui.painter().layout_no_wrap(text.to_string(), font, t.text);
    let (rect, _) = ui.allocate_exact_size(vec2(g.size().x.max(14.0) + 14.0, 22.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(4), t.elevated);
    ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, t.border), StrokeKind::Inside);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, t.text);
}

/// The Velour mark in `color`, fitted into `rect`. The drapes are curved
/// (not convex), so the mark is rasterized once into a mipmapped mask
/// texture and tinted per call.
pub fn draw_logo(p: &egui::Painter, rect: Rect, color: Color32) {
    let id = egui::Id::new("velour-logo-mask");
    let ctx = p.ctx();
    let tex = match ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        Some(t) => t,
        None => {
            let n = 256;
            let img = egui::ColorImage::from_rgba_unmultiplied([n, n], &crate::logo::logo_mask(n));
            let opts = egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
            let t = ctx.load_texture("velour-logo", img, opts);
            ctx.data_mut(|d| d.insert_temp(id, t.clone()));
            t
        }
    };
    let s = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center(), vec2(s, s));
    p.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), color);
}

/// Volume control drawn as a filled cone/wedge with a green→orange→red gradient.
pub fn volume_cone(ui: &mut Ui, t: &Theme, volume: &mut f64, max: f64, muted: bool) -> Response {
    let size = vec2(64.0, 20.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() {
            let f = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
            *volume = (f * max).round();
            resp.mark_changed();
        }
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            *volume = (*volume + if scroll > 0.0 { 5.0 } else { -5.0 }).clamp(0.0, max);
            resp.mark_changed();
        }
    }
    let p = ui.painter();
    let bottom = rect.bottom() - 3.0;
    let height_at = |x: f32| 3.0 + (x - rect.left()) / rect.width() * (rect.height() - 6.0);
    // background wedge
    p.add(Shape::convex_polygon(
        vec![pos2(rect.left(), bottom), pos2(rect.right(), bottom), pos2(rect.right(), bottom - height_at(rect.right())), pos2(rect.left(), bottom - height_at(rect.left()))],
        t.track,
        Stroke::NONE,
    ));
    let frac = (*volume / max).clamp(0.0, 1.0) as f32;
    let fill_to = rect.left() + rect.width() * frac;
    let steps = 24;
    let mut mesh = egui::Mesh::default();
    let color_at = |v: f32| -> Color32 {
        // v is volume percentage 0..max
        let stops = [(0.0, Color32::from_rgb(0x5c, 0xb8, 0x2e)), (70.0, Color32::from_rgb(0xc9, 0xc0, 0x2a)), (100.0, t.accent), (150.0, Color32::from_rgb(0xe0, 0x3a, 0x2a))];
        let mut c = stops[0].1;
        for w in stops.windows(2) {
            if v >= w[0].0 && v <= w[1].0 {
                let k = (v - w[0].0) / (w[1].0 - w[0].0);
                let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k) as u8;
                c = Color32::from_rgb(l(w[0].1.r(), w[1].1.r()), l(w[0].1.g(), w[1].1.g()), l(w[0].1.b(), w[1].1.b()));
            } else if v > w[1].0 {
                c = w[1].1;
            }
        }
        if muted { Color32::from_gray(90) } else { c }
    };
    for i in 0..=steps {
        let x = rect.left() + (fill_to - rect.left()) * i as f32 / steps as f32;
        let v = (x - rect.left()) / rect.width() * max as f32;
        let c = color_at(v);
        let idx = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(x, bottom), c);
        mesh.colored_vertex(pos2(x, bottom - height_at(x)), c);
        if i > 0 {
            mesh.add_triangle(idx - 2, idx - 1, idx);
            mesh.add_triangle(idx - 1, idx, idx + 1);
        }
    }
    if frac > 0.0 {
        p.add(Shape::mesh(mesh));
    }
    // 100% marker
    if max > 100.0 {
        let x = rect.left() + rect.width() * (100.0 / max as f32);
        p.line_segment([pos2(x, bottom + 1.0), pos2(x, bottom - height_at(x) - 1.0)], Stroke::new(1.0, t.text_faint));
    }
    resp.on_hover_text(tf!("Lautstärke {:.0}%", volume))
}

/// Settings row: label (+ info tooltip) on the left, control on the right.
pub fn setting_row(ui: &mut Ui, t: &Theme, label: &str, info: &str, add: impl FnOnce(&mut Ui)) {
    if crate::mobile::touch() {
        // Narrow screen, no hover: label and help text on top, the control
        // below, right aligned.
        ui.add_space(4.0);
        ui.label(egui::RichText::new(label).color(t.text).size(14.5));
        if !info.is_empty() {
            ui.label(egui::RichText::new(info).color(t.text_faint).size(12.0));
        }
        let w = ui.available_width();
        ui.allocate_ui_with_layout(vec2(w, 36.0), egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true), add);
        ui.add_space(4.0);
        let r = ui.available_rect_before_wrap();
        ui.painter().line_segment([pos2(r.left(), r.top()), pos2(r.right(), r.top())], Stroke::new(1.0, t.border.gamma_multiply(0.6)));
        ui.add_space(4.0);
        return;
    }
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        ui.label(egui::RichText::new(label).color(t.text));
        if !info.is_empty() {
            let r = ui.label(egui::RichText::new(egui_phosphor::regular::INFO).color(t.text_faint).size(14.0));
            r.on_hover_text(info);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| add(ui));
    });
}

pub fn separator(ui: &mut Ui, t: &Theme) {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 9.0), Sense::hover());
    ui.painter().line_segment([pos2(r.left(), r.center().y), pos2(r.right(), r.center().y)], Stroke::new(1.0, t.border));
}

/// Card frame used for popups.
pub fn popup_frame(t: &Theme) -> egui::Frame {
    egui::Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(theme::R_LG))
        .inner_margin(egui::Margin::same(8))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(120) })
}

pub fn progress_line(p: &egui::Painter, rect: Rect, frac: f32, t: &Theme) {
    p.rect_filled(rect, CornerRadius::ZERO, t.track.gamma_multiply(0.8));
    let f = Rect::from_min_max(rect.min, pos2(rect.left() + rect.width() * frac.clamp(0.0, 1.0), rect.bottom()));
    p.rect_filled(f, CornerRadius::ZERO, t.accent);
}

pub fn color_swatch(ui: &mut Ui, t: &Theme, c: Color32, selected: bool, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let r = rect.shrink(3.0);
    ui.painter().circle_filled(r.center(), r.width() / 2.0, c);
    if selected {
        ui.painter().circle_stroke(r.center(), r.width() / 2.0 + 2.0, Stroke::new(2.0, t.text));
    } else if resp.hovered() {
        ui.painter().circle_stroke(r.center(), r.width() / 2.0 + 2.0, Stroke::new(1.0, t.text_dim));
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn painter_text_trunc(p: &egui::Painter, pos: Pos2, text: &str, font: FontId, color: Color32, max_w: f32) {
    let mut s = text.to_string();
    let mut g = p.layout_no_wrap(s.clone(), font.clone(), color);
    if g.size().x > max_w {
        while !s.is_empty() && g.size().x > max_w {
            s.pop();
            g = p.layout_no_wrap(format!("{s}…"), font.clone(), color);
        }
    }
    p.galley(pos, g, color);
}

/// What happened in a [`search_field`] this frame.
pub struct SearchOut {
    pub changed: bool,
    pub submitted: bool,
    pub cleared: bool,
    pub focused: bool,
}

/// Pill-shaped search field: magnifier, frameless text, clear button and an
/// accent ring while focused. Enter submits; Esc clears, or drops focus when
/// already empty. The whole pill is a click target for focusing.
pub fn search_field(ui: &mut Ui, t: &Theme, id: egui::Id, text: &mut String, hint: &str, width: f32) -> SearchOut {
    use egui_phosphor::regular as ph;
    let h = 34.0;
    let (rect, bg_resp) = ui.allocate_exact_size(vec2(width.max(120.0), h), Sense::click());
    let focused_before = ui.memory(|m| m.has_focus(id));
    let hovered = ui.rect_contains_pointer(rect);
    let bg = if focused_before { t.surface } else if hovered { t.hover } else { t.elevated };
    let ring = if focused_before { t.accent } else { t.border };
    ui.painter().rect(rect, CornerRadius::same((h / 2.0) as u8), bg, Stroke::new(1.0, ring), StrokeKind::Inside);
    let icon_col = if focused_before { t.accent } else { t.text_faint };
    ui.painter().text(pos2(rect.left() + 18.0, rect.center().y), Align2::CENTER_CENTER, ph::MAGNIFYING_GLASS, theme::icon(15.0), icon_col);

    let has_text = !text.is_empty();
    let right_pad = if has_text { 34.0 } else { 14.0 };
    let inner = Rect::from_min_max(pos2(rect.left() + 34.0, rect.top()), pos2(rect.right() - right_pad, rect.bottom()));
    let te = ui.put(
        inner,
        egui::TextEdit::singleline(text)
            .id(id)
            .frame(false)
            .margin(egui::Margin::ZERO)
            .vertical_align(egui::Align::Center)
            .font(FontId::proportional(13.5))
            .text_color(t.text)
            .hint_text(egui::RichText::new(hint).color(t.text_faint))
            .desired_width(inner.width()),
    );
    let mut out = SearchOut { changed: te.changed(), submitted: false, cleared: false, focused: te.has_focus() };
    if bg_resp.clicked() {
        te.request_focus();
    }
    if bg_resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }
    if te.lost_focus() {
        let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        if enter {
            out.submitted = true;
        } else if esc && !text.is_empty() {
            text.clear();
            out.cleared = true;
            out.changed = true;
            te.request_focus();
        }
    }
    if !text.is_empty() {
        let c = pos2(rect.right() - 18.0, rect.center().y);
        let xr = ui.interact(Rect::from_center_size(c, Vec2::splat(22.0)), id.with("clear"), Sense::click());
        if xr.hovered() {
            ui.painter().circle_filled(c, 10.0, t.hover);
        }
        ui.painter().text(c, Align2::CENTER_CENTER, ph::X, theme::icon(13.0), if xr.hovered() { t.text } else { t.text_dim });
        if xr.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            text.clear();
            out.cleared = true;
            out.changed = true;
            te.request_focus();
        }
    }
    out
}
