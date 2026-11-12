//! Colors, fonts and egui visuals for the redesigned look:
//! neutral grays/black, a velvet-red accent, slightly rounded rectangles.

use crate::config::{Settings, ThemeMode};
use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

pub const ACCENTS: [[u8; 3]; 9] = [
    [0xD8, 0x3F, 0x5C],
    [0xF0, 0x86, 0x00],
    [0xC9, 0xA2, 0x27],
    [0x3F, 0xA3, 0x4D],
    [0x22, 0x9E, 0xA8],
    [0x3B, 0x6F, 0xE0],
    [0x8A, 0x4F, 0xD8],
    [0xC8, 0x48, 0x9B],
    [0x8C, 0x8C, 0x8C],
];

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub video_bg: Color32,
    pub bg: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub elevated: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_dim: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub track: Color32,
    pub danger: Color32,
}

pub const R: u8 = 4;
pub const R_LG: u8 = 6;

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

impl Theme {
    pub fn from_settings(s: &Settings, system_dark: bool) -> Theme {
        let dark = match s.theme {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => system_dark,
        };
        let accent = Color32::from_rgb(s.accent[0], s.accent[1], s.accent[2]);
        if dark {
            let bg = Color32::from_rgb(0x12, 0x12, 0x12);
            Theme {
                dark,
                video_bg: Color32::BLACK,
                bg,
                panel: Color32::from_rgb(0x17, 0x17, 0x17),
                surface: Color32::from_rgb(0x1f, 0x1f, 0x1f),
                elevated: Color32::from_rgb(0x26, 0x26, 0x26),
                hover: Color32::from_rgb(0x2e, 0x2e, 0x2e),
                border: Color32::from_rgb(0x2c, 0x2c, 0x2c),
                text: Color32::from_rgb(0xe8, 0xe8, 0xe8),
                text_dim: Color32::from_rgb(0x9a, 0x9a, 0x9a),
                text_faint: Color32::from_rgb(0x66, 0x66, 0x66),
                accent,
                accent_dim: mix(Color32::from_rgb(0x0e, 0x0e, 0x0e), accent, 0.28),
                accent_soft: mix(Color32::from_rgb(0x1a, 0x1a, 0x1a), accent, 0.16),
                on_accent: Color32::from_rgb(0x14, 0x0c, 0x02),
                track: Color32::from_rgb(0x0b, 0x0b, 0x0b),
                danger: Color32::from_rgb(0xe5, 0x48, 0x48),
            }
        } else {
            let bg = Color32::from_rgb(0xf2, 0xf2, 0xf2);
            Theme {
                dark,
                video_bg: Color32::from_rgb(0x10, 0x10, 0x10),
                bg,
                panel: Color32::from_rgb(0xfa, 0xfa, 0xfa),
                surface: Color32::from_rgb(0xff, 0xff, 0xff),
                elevated: Color32::from_rgb(0xff, 0xff, 0xff),
                hover: Color32::from_rgb(0xe9, 0xe9, 0xe9),
                border: Color32::from_rgb(0xdd, 0xdd, 0xdd),
                text: Color32::from_rgb(0x1c, 0x1c, 0x1c),
                text_dim: Color32::from_rgb(0x5e, 0x5e, 0x5e),
                text_faint: Color32::from_rgb(0x9a, 0x9a, 0x9a),
                accent,
                accent_dim: mix(Color32::WHITE, accent, 0.35),
                accent_soft: mix(Color32::WHITE, accent, 0.14),
                on_accent: Color32::WHITE,
                track: Color32::from_rgb(0xe2, 0xe2, 0xe2),
                danger: Color32::from_rgb(0xd0, 0x30, 0x30),
            }
        }
    }

    pub fn apply(&self, ctx: &egui::Context, scale: f32) {
        let mut v = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = self.panel;
        v.window_fill = self.surface;
        v.extreme_bg_color = if self.dark { Color32::from_rgb(0x0c, 0x0c, 0x0c) } else { Color32::WHITE };
        v.faint_bg_color = self.surface;
        v.code_bg_color = self.elevated;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_corner_radius = CornerRadius::same(R_LG);
        v.menu_corner_radius = CornerRadius::same(R_LG);
        v.window_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(110) };
        v.popup_shadow = egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(90) };
        v.selection.bg_fill = self.accent_dim;
        v.selection.stroke = Stroke::new(1.0, self.accent);
        v.hyperlink_color = self.accent;
        v.override_text_color = Some(self.text);
        v.slider_trailing_fill = true;
        v.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.5 };
        let w = &mut v.widgets;
        let r = CornerRadius::same(R);
        for (st, bg, stroke) in [
            (&mut w.noninteractive, self.surface, self.border),
            (&mut w.inactive, self.elevated, self.border),
            (&mut w.hovered, self.hover, self.text_faint),
            (&mut w.active, self.hover, self.accent),
            (&mut w.open, self.elevated, self.border),
        ] {
            st.bg_fill = bg;
            st.weak_bg_fill = bg;
            st.bg_stroke = Stroke::new(1.0, stroke);
            st.corner_radius = r;
        }
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.hovered.fg_stroke = Stroke::new(1.0, self.text);
        w.active.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.bg_stroke = Stroke::new(1.0, self.border);
        ctx.set_visuals(v);

        ctx.style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            s.spacing.button_padding = egui::vec2(10.0, 5.0);
            s.spacing.interact_size.y = 26.0;
            s.spacing.slider_width = 160.0;
            s.spacing.scroll = egui::style::ScrollStyle::thin();
            s.text_styles = [
                (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Heading, FontId::new(19.0, FontFamily::Name("semibold".into()))),
                (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
            ]
            .into();
            s.interaction.tooltip_delay = 0.35;
            if crate::mobile::touch() {
                // Fingers: bars only while scrolling, bigger hit targets.
                s.spacing.scroll = egui::style::ScrollStyle::floating();
                s.spacing.interact_size.y = 36.0;
                s.spacing.button_padding = egui::vec2(12.0, 8.0);
                s.spacing.item_spacing = egui::vec2(10.0, 8.0);
            }
        });
        ctx.set_zoom_factor(scale.clamp(0.75, 2.0));
    }
}

pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let read = crate::platform::font;
    let mut prop_first: Vec<String> = Vec::new();
    if let Some(b) = read("segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), egui::FontData::from_owned(b).into());
        prop_first.push("segoe".into());
    }
    let semibold = read("seguisb.ttf");
    if let Some(b) = semibold {
        fonts.font_data.insert("segoe-sb".into(), egui::FontData::from_owned(b).into());
    }
    // Symbols like ←, ⌫ and key glyphs.
    if let Some(b) = read("seguisym.ttf") {
        fonts.font_data.insert("segoe-sym".into(), egui::FontData::from_owned(b).into());
    }
    fonts.font_data.insert("phosphor".into(), egui_phosphor::Variant::Regular.font_data().into());
    fonts.font_data.insert("phosphor-fill".into(), egui_phosphor::Variant::Fill.font_data().into());

    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut prop = prop_first.clone();
    prop.push("phosphor".into());
    prop.extend(defaults.iter().cloned());
    if fonts.font_data.contains_key("segoe-sym") {
        prop.push("segoe-sym".into());
    }
    fonts.families.insert(FontFamily::Proportional, prop.clone());

    let mut sb = Vec::new();
    if fonts.font_data.contains_key("segoe-sb") {
        sb.push("segoe-sb".to_string());
    }
    sb.extend(prop.iter().cloned());
    fonts.families.insert(FontFamily::Name("semibold".into()), sb);

    let mut fill = vec!["phosphor-fill".to_string()];
    fill.extend(prop.iter().cloned());
    fonts.families.insert(FontFamily::Name("fill".into()), fill);

    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        mono.push("phosphor".into());
    }
    ctx.set_fonts(fonts);
    load_cjk_fallbacks(ctx);
}

/// Japanese, Chinese and Korean titles (anime, Plex metadata, file names) need
/// system fonts Segoe UI lacks. They are 10–20 MB each, so they load off the UI
/// thread and join every family as lowest-priority fallbacks once read.
fn load_cjk_fallbacks(ctx: &egui::Context) {
    use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        // Japanese first so shared Han characters get Japanese glyph shapes.
        let candidates: [(&str, &[&str]); 3] = [
            ("cjk-ja", &["YuGothM.ttc", "YuGothR.ttc", "meiryo.ttc", "msgothic.ttc"]),
            ("cjk-zh", &["msyh.ttc", "simsun.ttc"]),
            ("cjk-ko", &["malgun.ttf"]),
        ];
        let families = || {
            [FontFamily::Proportional, FontFamily::Monospace, FontFamily::Name("semibold".into()), FontFamily::Name("fill".into())]
                .into_iter()
                .map(|family| InsertFontFamily { family, priority: FontPriority::Lowest })
                .collect::<Vec<_>>()
        };
        let mut added = false;
        for (name, files) in candidates {
            let Some(bytes) = files.iter().find_map(|f| crate::platform::font(f)) else {
                continue;
            };
            ctx.add_font(FontInsert::new(name, egui::FontData::from_owned(bytes), families()));
            added = true;
        }
        if added {
            ctx.request_repaint();
        }
    });
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}
pub fn fill_icon(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("fill".into()))
}
pub fn icon(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
