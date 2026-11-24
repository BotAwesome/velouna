//! Right side panels: adjustments sidebar (playback/video/audio/subtitles/marks) and the queue.

use crate::app::{AdjustTab, App, Dialog, EQ_FREQS, EQ_PRESETS, RightPanel, VIDEO_PRESETS};
use crate::config::RepeatMode;
use crate::hotkeys::Action;
use crate::media;
use crate::theme;
use crate::widgets::{self, icon_button, setting_row};
use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as ph;
use std::time::Instant;

#[derive(Clone)]
pub struct LibDrag(pub Vec<String>);

/// While files from the library are dragged, a small label follows the
/// pointer: what is being carried, and how many.
pub fn paint_drag_ghost(ctx: &egui::Context, t: &theme::Theme) {
    let Some(drag) = egui::DragAndDrop::payload::<LibDrag>(ctx) else { return };
    let Some(pos) = ctx.pointer_latest_pos() else { return };
    let Some(first) = drag.0.first() else { return };
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let icon = if media::is_audio(first) { ph::MUSIC_NOTES } else { ph::FILM_STRIP };
    let mut name = media::display_name(first);
    if name.chars().count() > 40 {
        name = name.chars().take(39).collect::<String>() + "…";
    }
    let more = drag.0.len().saturating_sub(1);
    egui::Area::new(Id::new("lib_drag_ghost"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + vec2(14.0, 12.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(t.surface)
                .stroke(Stroke::new(1.0, t.accent.gamma_multiply(0.6)))
                .corner_radius(CornerRadius::same(theme::R_LG))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(120) })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new(icon).font(theme::icon(15.0)).color(t.accent));
                        ui.label(RichText::new(name).size(13.0).color(t.text));
                        if more > 0 {
                            ui.label(RichText::new(format!("+{more}")).size(12.0).color(t.text_dim));
                        }
                    });
                });
        });
    ctx.request_repaint();
}

const FONTS: &[&str] = &[
    "Segoe UI",
    "Segoe UI Semibold",
    "Arial",
    "Calibri",
    "Verdana",
    "Tahoma",
    "Trebuchet MS",
    "Georgia",
    "Times New Roman",
    "Consolas",
    "Comic Sans MS",
    "Impact",
];

const SUB_COLORS: &[[u8; 4]] = &[
    [255, 255, 255, 255],
    [255, 235, 59, 255],
    [255, 193, 7, 255],
    [129, 212, 250, 255],
    [165, 214, 167, 255],
    [12, 12, 12, 255],
];

pub fn show_in_explorer(path: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.replace('/', "\\"))).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        // Selects the file in Files/Dolphin/... (FileManager1), else opens its folder.
        let uri = format!("file://{}", path.replace('%', "%25").replace(' ', "%20").replace('#', "%23"));
        let shown = std::process::Command::new("gdbus")
            .args(["call", "--session", "--dest", "org.freedesktop.FileManager1", "--object-path", "/org/freedesktop/FileManager1", "--method", "org.freedesktop.FileManager1.ShowItems", &format!("['{uri}']"), ""])
            .status()
            .is_ok_and(|s| s.success());
        if !shown {
            if let Some(dir) = std::path::Path::new(path).parent() {
                let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
            }
        }
    }
    // Android has no file manager to point at.
    #[cfg(target_os = "android")]
    let _ = path;
}

impl App {
    fn panel_header(&mut self, ui: &mut Ui, title: &str) {
        let t = self.t;
        ui.horizontal(|ui| {
            ui.set_min_height(34.0);
            ui.label(RichText::new(title).font(theme::semibold(15.0)).color(t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, &t, ph::X, t!("Schließen"), false).clicked() {
                    self.right = RightPanel::None;
                }
            });
        });
    }

    // ===================================================== adjustments

    pub fn adjust_panel(&mut self, ui: &mut Ui) {
        let t = self.t;
        self.panel_header(ui, t!("Anpassungen"));
        let mut tab = self.adjust_tab;
        widgets::tabs(
            ui,
            &t,
            &mut tab,
            &[
                (AdjustTab::Playback, ph::PLAY, t!("Wiedergabe")),
                (AdjustTab::Video, ph::FILM_STRIP, "Video"),
                (AdjustTab::Audio, ph::WAVEFORM, "Audio"),
                (AdjustTab::Subtitles, ph::SUBTITLES, t!("Untertitel")),
                (AdjustTab::Marks, ph::BOOKMARKS, t!("Marken")),
            ],
        );
        self.adjust_tab = tab;
        ui.add_space(6.0);
        if self.player.is_none() {
            ui.label(t!("Wiedergabe-Engine nicht verfügbar."));
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width() - 4.0);
            match tab {
                AdjustTab::Playback => self.tab_playback(ui),
                AdjustTab::Video => self.tab_video(ui),
                AdjustTab::Audio => self.tab_audio(ui),
                AdjustTab::Subtitles => self.tab_subtitles(ui),
                AdjustTab::Marks => self.tab_marks(ui),
            }
            ui.add_space(20.0);
        });
    }

    fn card<R>(&self, ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
        let t = self.t;
        ui.add_space(4.0);
        let r = egui::Frame::new()
            .fill(t.surface)
            .corner_radius(CornerRadius::same(theme::R_LG))
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(title).font(theme::semibold(13.5)).color(t.text));
                ui.add_space(4.0);
                add(ui)
            })
            .inner;
        ui.add_space(4.0);
        r
    }

    fn tab_playback(&mut self, ui: &mut Ui) {
        let t = self.t;
        let p = self.player.as_ref().unwrap();
        let (c, st) = (p.ctl.clone(), p.st.clone());
        self.card(ui, t!("Geschwindigkeit"), |ui| {
            setting_row(ui, &t, t!("Geschwindigkeit"), "", |ui| {
                widgets::value_box(ui, &t, &format!("{:.2}×", st.speed), 80.0);
            });
            let mut sp = st.speed;
            let w = ui.available_width();
            let ticks = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
            if widgets::slider_ex(ui, &t, &mut sp, 0.25, 2.0, w, Some(1.0), &ticks).changed() {
                c.set_speed((sp * 20.0).round() / 20.0);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let seg = w / 7.0;
                for (i, l) in ["0.25", "0.5", "0.75", "1", "1.25", "1.5", "1.75", "2"].iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(vec2(if i == 0 || i == 7 { seg / 2.0 } else { seg }, 14.0), Sense::hover());
                    let x = if i == 0 { r.left() + 4.0 } else if i == 7 { r.right() - 4.0 } else { r.center().x };
                    let al = if i == 0 { Align2::LEFT_CENTER } else if i == 7 { Align2::RIGHT_CENTER } else { Align2::CENTER_CENTER };
                    ui.painter().text(pos2(x, r.center().y), al, *l, FontId::proportional(10.5), t.text_faint);
                }
            });
            setting_row(ui, &t, t!("Rückwärts abspielen"), t!("Experimentell: spielt die Datei rückwärts (benötigt viel Speicher)."), |ui| {
                let mut b = st.direction_backward;
                if widgets::toggle(ui, &t, &mut b).changed() {
                    c.set_str("play-direction", if b { "backward" } else { "forward" });
                }
            });
        });

        let sleep_label = match (self.sleep_at, self.sleep_eof) {
            (Some(at), _) => {
                let r = at.saturating_duration_since(Instant::now()).as_secs();
                tf!("noch {}:{:02}", r / 60, r % 60)
            }
            (None, true) => t!("am Ende der Folge / des Films").into(),
            _ => "aus".into(),
        };
        let mut sleep_choice: Option<Option<u64>> = None;
        let mut eof_choice = false;
        self.card(ui, t!("Sleep-Timer"), |ui| {
            ui.label(RichText::new(format!("Status: {sleep_label}")).color(t.text_dim));
            ui.horizontal_wrapped(|ui| {
                for m in [0u64, 15, 30, 45, 60, 90] {
                    let label = if m == 0 { t!("Aus").to_string() } else { format!("{m} min") };
                    if ui.button(label).clicked() {
                        sleep_choice = Some(if m == 0 { None } else { Some(m) });
                    }
                }
                if ui.button(t!("Ende der Folge / des Films")).clicked() {
                    eof_choice = true;
                }
            });
        });
        if let Some(ch) = sleep_choice {
            self.sleep_minutes(ch);
        }
        if eof_choice {
            self.sleep_end_of_title();
        }

        let mut ab = false;
        let mut repeat = self.data.settings.repeat;
        self.card(ui, t!("Schleifen"), |ui| {
            setting_row(ui, &t, t!("Wiederholen"), "", |ui| {
                widgets::segmented(ui, &t, &mut repeat, &[(RepeatMode::Off, t!("Aus")), (RepeatMode::All, t!("Alle")), (RepeatMode::One, t!("Eine"))]);
            });
            setting_row(ui, &t, t!("A-B-Schleife"), t!("Erster Klick setzt A, zweiter B, dritter entfernt die Schleife."), |ui| {
                let label = match (st.ab_a, st.ab_b) {
                    (Some(a), Some(b)) => format!("{} – {}  ✕", media::fmt_time(a), media::fmt_time(b)),
                    (Some(a), None) => tf!("A {} · B setzen", media::fmt_time(a)),
                    _ => t!("A setzen").into(),
                };
                if ui.button(label).clicked() {
                    ab = true;
                }
            });
        });
        if repeat != self.data.settings.repeat {
            self.set_repeat(repeat);
        }
        if ab {
            self.cycle_ab_loop(None);
        }

        let mut s = self.data.settings.clone();
        self.card(ui, t!("Anzeige"), |ui| {
            setting_row(ui, &t, t!("Untertitel-Zeitleiste"), t!("Zeigt die Untertitel als Blöcke über der Audio-Wellenform. Ziehen verschiebt die Untertitel."), |ui| {
                widgets::toggle(ui, &t, &mut s.show_sub_timeline);
            });
            setting_row(ui, &t, t!("Wellenform in der Suchleiste"), "", |ui| {
                widgets::toggle(ui, &t, &mut s.show_waveform);
            });
            setting_row(ui, &t, t!("Vorschaubilder beim Spulen"), "", |ui| {
                widgets::toggle(ui, &t, &mut s.show_seek_thumbnails);
            });
        });
        if s.show_sub_timeline != self.data.settings.show_sub_timeline
            || s.show_waveform != self.data.settings.show_waveform
            || s.show_seek_thumbnails != self.data.settings.show_seek_thumbnails
        {
            self.data.settings = s;
            self.mark_dirty();
        }

        let mut info_open = self.info_open;
        self.card(ui, t!("Medieninformationen"), |ui| {
            let r = ui.add(egui::Button::new(if info_open { t!("Ausblenden") } else { t!("Metadaten anzeigen") }));
            if r.clicked() {
                info_open = !info_open;
            }
            if info_open {
                egui::Grid::new("meta").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                    let mut row = |k: &str, v: String| {
                        if !v.is_empty() {
                            ui.label(RichText::new(k).color(t.text_dim));
                            ui.label(v);
                            ui.end_row();
                        }
                    };
                    row(t!("Datei"), st.path.clone());
                    row("Format", st.file_format.clone());
                    if st.file_size > 0 {
                        row(t!("Größe"), media::fmt_size(st.file_size));
                    }
                    row(t!("Dauer"), media::fmt_time(st.duration));
                    if st.video_w > 0 {
                        row(t!("Auflösung"), format!("{}×{}", st.video_w, st.video_h));
                    }
                    if st.fps > 0.0 {
                        row(t!("Bildrate"), format!("{:.3} fps", st.fps));
                    }
                    row(t!("Videocodec"), st.video_codec.clone());
                    row(t!("Audiocodec"), st.audio_codec.clone());
                    row(t!("Hardware-Dekodierung"), if st.hwdec.is_empty() || st.hwdec == "no" { "aus".into() } else { st.hwdec.clone() });
                    for (k, v) in &st.metadata {
                        row(k, v.clone());
                    }
                });
            }
        });
        self.info_open = info_open;
    }

    fn tab_video(&mut self, ui: &mut Ui) {
        let t = self.t;
        let p = self.player.as_ref().unwrap();
        let (c, st) = (p.ctl.clone(), p.st.clone());
        let mut preset_choice: Option<&str> = None;
        let current = self.video_preset.clone();
        self.card(ui, "Filter", |ui| {
            let w = ui.available_width();
            let cols = 4;
            let cw = (w - (cols as f32 - 1.0) * 6.0) / cols as f32;
            egui::Grid::new("vpresets").spacing([6.0, 6.0]).show(ui, |ui| {
                for (i, (name, _, _, c1, c2)) in VIDEO_PRESETS.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(cw, cw * 0.62 + 18.0), Sense::click());
                    let img = Rect::from_min_size(r.min, vec2(cw, cw * 0.62));
                    let mut mesh = egui::Mesh::default();
                    let a = Color32::from_rgb(c1[0], c1[1], c1[2]);
                    let b = Color32::from_rgb(c2[0], c2[1], c2[2]);
                    mesh.colored_vertex(img.left_top(), a);
                    mesh.colored_vertex(img.right_top(), b);
                    mesh.colored_vertex(img.right_bottom(), a);
                    mesh.colored_vertex(img.left_bottom(), b);
                    mesh.add_triangle(0, 1, 2);
                    mesh.add_triangle(0, 2, 3);
                    ui.painter().add(egui::Shape::mesh(mesh));
                    let sel = current == *name;
                    let stroke = if sel { Stroke::new(2.0, t.accent) } else if resp.hovered() { Stroke::new(1.0, t.text_dim) } else { Stroke::new(1.0, t.border) };
                    ui.painter().rect_stroke(img, CornerRadius::same(4), stroke, StrokeKind::Inside);
                    ui.painter().text(pos2(r.center().x, img.bottom() + 9.0), Align2::CENTER_CENTER, crate::i18n::tr(name), FontId::proportional(11.0), if sel { t.accent } else { t.text_dim });
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        preset_choice = Some(name);
                    }
                    if (i + 1) % cols == 0 {
                        ui.end_row();
                    }
                }
            });
        });
        if let Some(n) = preset_choice {
            self.set_video_preset(n);
        }

        self.card(ui, t!("Bild"), |ui| {
            let w = ui.available_width();
            for (label, prop, val) in [
                (t!("Helligkeit"), "brightness", st.brightness),
                (t!("Kontrast"), "contrast", st.contrast),
                (t!("Sättigung"), "saturation", st.saturation),
                ("Gamma", "gamma", st.gamma),
                (t!("Farbton"), "hue", st.hue),
            ] {
                setting_row(ui, &t, label, "", |ui| {
                    widgets::value_box(ui, &t, &format!("{val:+.0}"), 56.0);
                });
                let mut v = val;
                if widgets::slider_ex(ui, &t, &mut v, -100.0, 100.0, w, Some(0.0), &[0.0]).changed() {
                    c.set(prop, v.round());
                }
            }
            if ui.button(t!("Zurücksetzen")).clicked() {
                for prop in ["brightness", "contrast", "saturation", "gamma", "hue"] {
                    c.set(prop, 0.0);
                }
            }
        });

        let mut flip_h = self.flip_h;
        let mut flip_v = self.flip_v;
        self.card(ui, t!("Transformation"), |ui| {
            setting_row(ui, &t, t!("Drehen"), "", |ui| {
                let mut rot = st.rotate;
                if widgets::segmented(ui, &t, &mut rot, &[(0, "0°"), (90, "90°"), (180, "180°"), (270, "270°")]) {
                    c.set("video-rotate", rot as f64);
                }
            });
            setting_row(ui, &t, t!("Horizontal spiegeln"), "", |ui| {
                widgets::toggle(ui, &t, &mut flip_h);
            });
            setting_row(ui, &t, t!("Vertikal spiegeln"), "", |ui| {
                widgets::toggle(ui, &t, &mut flip_v);
            });
            setting_row(ui, &t, "Zoom", "", |ui| {
                widgets::value_box(ui, &t, &format!("{:.0} %", 2f64.powf(st.zoom) * 100.0), 70.0);
            });
            let mut z = st.zoom;
            let w = ui.available_width();
            if widgets::slider_ex(ui, &t, &mut z, -1.0, 1.5, w, Some(0.0), &[0.0]).changed() {
                c.set("video-zoom", z);
            }
            setting_row(ui, &t, t!("Seitenverhältnis"), "", |ui| {
                let opts = [("no", t!("Automatisch")), ("16:9", "16:9"), ("4:3", "4:3"), ("2.33", "21:9"), ("2.35", "2.35:1"), ("1:1", "1:1"), ("9:16", "9:16")];
                let cur = opts.iter().find(|(v, _)| *v == st.aspect).map(|x| x.1).unwrap_or(t!("Automatisch"));
                egui::ComboBox::from_id_salt("aspect").selected_text(cur).width(130.0).show_ui(ui, |ui| {
                    for (v, l) in opts {
                        if ui.selectable_label(st.aspect == v, l).clicked() {
                            c.set_str("video-aspect-override", v);
                        }
                    }
                });
            });
        });
        if flip_h != self.flip_h || flip_v != self.flip_v {
            self.flip_h = flip_h;
            self.flip_v = flip_v;
            self.apply_video_filters();
        }

        let mut deint = self.data.settings.deinterlace;
        self.card(ui, t!("Weiteres"), |ui| {
            setting_row(ui, &t, "Deinterlacing", "", |ui| {
                if widgets::toggle(ui, &t, &mut deint).changed() {
                    c.set_str("deinterlace", if deint { "yes" } else { "no" });
                }
            });
            widgets::section_label(ui, &t, t!("Videospur"));
            for tr in st.tracks_of("video") {
                let mut l = tr.label();
                if let (Some(w), Some(h)) = (tr.w, tr.h) {
                    l.push_str(&format!("  · {w}×{h}"));
                }
                if widgets::check_row(ui, &t, &l, tr.selected).clicked() {
                    c.set_track("video", Some(tr.id));
                }
            }
            ui.add_space(4.0);
            if widgets::icon_text_button(ui, &t, ph::CAMERA, t!("Screenshot speichern")).clicked() {
                c.screenshot();
            }
        });
        if deint != self.data.settings.deinterlace {
            self.data.settings.deinterlace = deint;
            self.mark_dirty();
        }
    }

    fn tab_audio(&mut self, ui: &mut Ui) {
        let t = self.t;
        let p = self.player.as_ref().unwrap();
        let (c, st) = (p.ctl.clone(), p.st.clone());
        let mut alang = self.data.settings.alang.clone();
        self.card(ui, t!("Standards"), |ui| {
            setting_row(ui, &t, t!("Audiospur"), "", |ui| {
                let cur = st.selected("audio").map(|t| t.label()).unwrap_or(t!("Keine").into());
                egui::ComboBox::from_id_salt("aud_track").selected_text(cur).width(170.0).show_ui(ui, |ui| {
                    for tr in st.tracks_of("audio") {
                        if ui.selectable_label(tr.selected, tr.label()).clicked() {
                            c.set_track("audio", Some(tr.id));
                        }
                    }
                });
            });
            setting_row(ui, &t, t!("Bevorzugte Sprache"), t!("Sprachcodes, z. B. „de,en“. Wird bei jeder Datei automatisch gewählt."), |ui| {
                ui.add(egui::TextEdit::singleline(&mut alang).desired_width(110.0).hint_text("de,en"));
            });
            setting_row(ui, &t, t!("Ausgabegerät"), "", |ui| {
                let cur = st
                    .audio_devices
                    .iter()
                    .find(|d| d.name == st.audio_device)
                    .map(|d| if d.name == "auto" { t!("Systemstandard").into() } else { d.description.clone() })
                    .unwrap_or(t!("Systemstandard").into());
                egui::ComboBox::from_id_salt("aud_dev").selected_text(cur).width(170.0).show_ui(ui, |ui| {
                    for d in &st.audio_devices {
                        let label = if d.name == "auto" { t!("Systemstandard").to_string() } else { d.description.clone() };
                        if ui.selectable_label(d.name == st.audio_device, label).clicked() {
                            c.set_str("audio-device", &d.name);
                        }
                    }
                });
            });
        });
        if alang != self.data.settings.alang {
            c.set_str("alang", &alang);
            self.data.settings.alang = alang;
            self.mark_dirty();
        }

        self.card(ui, t!("Synchronisation"), |ui| {
            setting_row(ui, &t, t!("Audioverzögerung"), "", |ui| {
                widgets::value_box(ui, &t, &format!("{:+.3} s", st.audio_delay), 90.0);
            });
            let mut d = st.audio_delay;
            let w = ui.available_width();
            if widgets::slider_ex(ui, &t, &mut d, -5.0, 5.0, w, Some(0.0), &[0.0]).changed() {
                c.set("audio-delay", (d * 1000.0).round() / 1000.0);
            }
            ui.horizontal(|ui| {
                if ui.button("−50 ms").clicked() {
                    c.set("audio-delay", st.audio_delay - 0.05);
                }
                if ui.button("+50 ms").clicked() {
                    c.set("audio-delay", st.audio_delay + 0.05);
                }
                if ui.button(t!("Zurücksetzen")).clicked() {
                    c.set("audio-delay", 0.0);
                }
            });
        });

        let mut eq_enabled = self.eq_enabled;
        let mut bands = self.eq_bands;
        let mut preamp = self.eq_preamp;
        let mut preset: Option<String> = None;
        let mut eq_open = self.eq_open;
        let cur_preset = self.eq_preset.clone();
        self.card(ui, "Equalizer", |ui| {
            setting_row(ui, &t, "Equalizer", "", |ui| {
                widgets::toggle(ui, &t, &mut eq_enabled);
            });
            setting_row(ui, &t, "Preset", "", |ui| {
                egui::ComboBox::from_id_salt("eq_preset").selected_text(eq_label(&cur_preset).to_string()).width(140.0).show_ui(ui, |ui| {
                    for (n, _) in EQ_PRESETS {
                        if ui.selectable_label(cur_preset == *n, eq_label(n)).clicked() {
                            preset = Some(n.to_string());
                        }
                    }
                });
            });
            let lbl = if eq_open { tf!("{}  Equalizer schließen", ph::CARET_UP) } else { tf!("{}  Equalizer öffnen", ph::CARET_DOWN) };
            if ui.add_sized([ui.available_width(), 26.0], egui::Button::new(lbl)).clicked() {
                eq_open = !eq_open;
            }
            if eq_open {
                ui.add_space(6.0);
                let w = ui.available_width();
                let n = 11.0;
                let col = w / n;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let col_ui = |ui: &mut Ui, v: &mut f64, label: &str| -> bool {
                        let mut ch = false;
                        ui.allocate_ui_with_layout(vec2(col, 170.0), Layout::top_down(Align::Center), |ui| {
                            ch = widgets::vslider(ui, &t, v, -12.0, 12.0, 130.0).changed();
                            ui.label(RichText::new(label).size(10.0).color(t.text_faint));
                            ui.label(RichText::new(format!("{:+.0}", v)).size(10.0).color(t.text_dim));
                        });
                        ch
                    };
                    if col_ui(ui, &mut preamp, "Pre") {
                        eq_enabled = true;
                    }
                    for (i, f) in EQ_FREQS.iter().enumerate() {
                        let l = if *f >= 1000 { format!("{}k", f / 1000) } else { f.to_string() };
                        if col_ui(ui, &mut bands[i], &l) {
                            eq_enabled = true;
                        }
                    }
                });
            }
        });
        self.eq_open = eq_open;
        if let Some(pn) = preset {
            self.eq_enabled = true;
            self.set_eq_preset(&pn);
        } else if eq_enabled != self.eq_enabled || bands != self.eq_bands || preamp != self.eq_preamp {
            if bands != self.eq_bands {
                self.eq_preset = "Benutzerdefiniert".into();
                self.data.eq_custom = bands.to_vec();
                self.mark_dirty();
            }
            self.eq_enabled = eq_enabled;
            self.eq_bands = bands;
            self.eq_preamp = preamp;
            self.apply_audio_filters();
        }

        let (mut n, mut ni, mut mo, mut wi) = (self.fx_normalize, self.fx_night, self.fx_mono, self.fx_widen);
        self.card(ui, t!("Effekte"), |ui| {
            setting_row(ui, &t, t!("Lautstärke normalisieren"), t!("Gleicht leise und laute Stellen automatisch an."), |ui| {
                widgets::toggle(ui, &t, &mut n);
            });
            setting_row(ui, &t, t!("Nachtmodus"), t!("Komprimiert die Dynamik – Explosionen leiser, Dialoge verständlich."), |ui| {
                widgets::toggle(ui, &t, &mut ni);
            });
            setting_row(ui, &t, "Mono", "", |ui| {
                widgets::toggle(ui, &t, &mut mo);
            });
            setting_row(ui, &t, t!("Stereo verbreitern"), "", |ui| {
                widgets::toggle(ui, &t, &mut wi);
            });
        });
        if (n, ni, mo, wi) != (self.fx_normalize, self.fx_night, self.fx_mono, self.fx_widen) {
            self.fx_normalize = n;
            self.fx_night = ni;
            self.fx_mono = mo;
            self.fx_widen = wi;
            self.apply_audio_filters();
        }
    }

    fn tab_subtitles(&mut self, ui: &mut Ui) {
        let t = self.t;
        let p = self.player.as_ref().unwrap();
        let (c, st) = (p.ctl.clone(), p.st.clone());
        ui.horizontal(|ui| {
            if widgets::icon_text_button(ui, &t, ph::PLUS, t!("Untertitel hinzufügen…")).clicked() {
                self.dialog = Some(Dialog::AddSubs);
            }
        });
        let mut slang = self.data.settings.slang.clone();
        self.card(ui, t!("Standards"), |ui| {
            setting_row(ui, &t, t!("Untertitelspur"), "", |ui| {
                let cur = st.selected("sub").map(|t| t.label()).unwrap_or(t!("Keine").into());
                egui::ComboBox::from_id_salt("sub_track").selected_text(cur).width(170.0).show_ui(ui, |ui| {
                    if ui.selectable_label(st.selected("sub").is_none(), t!("Keine")).clicked() {
                        c.set_track("sub", None);
                    }
                    for tr in st.tracks_of("sub") {
                        if ui.selectable_label(tr.selected, tr.label()).clicked() {
                            c.set_track("sub", Some(tr.id));
                        }
                    }
                });
            });
            setting_row(ui, &t, t!("Untertitelsprache"), t!("Wird bei jeder Datei automatisch gewählt, z. B. „de,en“."), |ui| {
                ui.add(egui::TextEdit::singleline(&mut slang).desired_width(110.0).hint_text("de,en"));
            });
        });
        if slang != self.data.settings.slang {
            c.set_str("slang", &slang);
            self.data.settings.slang = slang;
            self.mark_dirty();
        }

        let mut show_tl = self.data.settings.show_sub_timeline;
        self.card(ui, t!("Synchronisation"), |ui| {
            setting_row(ui, &t, t!("Untertitelverzögerung"), "", |ui| {
                widgets::value_box(ui, &t, &format!("{:+.3} s", st.sub_delay), 90.0);
            });
            let mut d = st.sub_delay;
            let w = ui.available_width();
            if widgets::slider_ex(ui, &t, &mut d, -10.0, 10.0, w, Some(0.0), &[0.0]).changed() {
                c.set("sub-delay", (d * 100.0).round() / 100.0);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("−10 s").size(10.5).color(t.text_faint));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new("+10 s").size(10.5).color(t.text_faint));
                });
            });
            ui.horizontal(|ui| {
                if ui.button("−100 ms").clicked() {
                    c.set("sub-delay", st.sub_delay - 0.1);
                }
                if ui.button("+100 ms").clicked() {
                    c.set("sub-delay", st.sub_delay + 0.1);
                }
                if ui.button(t!("Zurücksetzen")).clicked() {
                    c.set("sub-delay", 0.0);
                }
            });
            setting_row(ui, &t, t!("Untertitelgeschwindigkeit"), t!("Für Untertitel mit falscher Bildrate (z. B. 23.976 ↔ 25 fps)."), |ui| {
                widgets::value_box(ui, &t, &format!("{:.3}", st.sub_speed), 70.0);
            });
            let mut ss = st.sub_speed;
            if widgets::slider_ex(ui, &t, &mut ss, 0.8, 1.2, w, Some(1.0), &[1.0, 23.976 / 25.0, 25.0 / 23.976]).changed() {
                c.set("sub-speed", ss);
            }
            setting_row(ui, &t, t!("Zeitleiste anzeigen"), t!("Untertitel als Blöcke über der Audio-Wellenform – einfach verschieben, bis sie passen."), |ui| {
                widgets::toggle(ui, &t, &mut show_tl);
            });
        });
        if show_tl != self.data.settings.show_sub_timeline {
            self.data.settings.show_sub_timeline = show_tl;
            self.mark_dirty();
        }

        let mut s = self.data.settings.clone();
        let mut sub_scale = st.sub_scale;
        self.card(ui, t!("Darstellung"), |ui| {
            setting_row(ui, &t, t!("Größenwerte"), t!("Relativ skaliert mit dem Video, Pixel sind exakte Werte."), |ui| {
                widgets::segmented(ui, &t, &mut s.sub_size_pixels, &[(false, t!("Relativ")), (true, "Pixel")]);
            });
            setting_row(ui, &t, t!("Schriftart"), "", |ui| {
                egui::ComboBox::from_id_salt("sub_font").selected_text(s.sub_font.clone()).width(150.0).show_ui(ui, |ui| {
                    for f in FONTS {
                        ui.selectable_value(&mut s.sub_font, f.to_string(), *f);
                    }
                });
            });
            let w = ui.available_width();
            if s.sub_size_pixels {
                setting_row(ui, &t, t!("Schriftgröße"), "", |ui| {
                    widgets::value_box(ui, &t, &format!("{:.0} px", s.sub_font_size), 70.0);
                });
                widgets::slider_ex(ui, &t, &mut s.sub_font_size, 16.0, 110.0, w, Some(46.0), &[]);
                s.sub_font_size = s.sub_font_size.round();
            } else {
                setting_row(ui, &t, t!("Schriftgröße"), "", |ui| {
                    let label = match sub_scale {
                        x if x < 0.8 => t!("Klein"),
                        x if x < 1.15 => "Normal",
                        x if x < 1.6 => t!("Groß"),
                        _ => t!("Sehr groß"),
                    };
                    widgets::value_box(ui, &t, &format!("{label} ({:.0} %)", sub_scale * 100.0), 120.0);
                });
                widgets::slider_ex(ui, &t, &mut sub_scale, 0.5, 2.5, w, Some(1.0), &[1.0]);
            }
            setting_row(ui, &t, t!("Textfarbe"), "", |ui| {
                for col in SUB_COLORS.iter().rev() {
                    if widgets::color_swatch(ui, &t, Color32::from_rgb(col[0], col[1], col[2]), s.sub_color == *col, 22.0).clicked() {
                        s.sub_color = *col;
                    }
                }
            });
            setting_row(ui, &t, t!("Umrissfarbe"), "", |ui| {
                for col in [[12, 12, 12, 255], [60, 60, 60, 255], [255, 255, 255, 255], [0, 0, 120, 255]].iter().rev() {
                    if widgets::color_swatch(ui, &t, Color32::from_rgb(col[0], col[1], col[2]), s.sub_border_color == *col, 22.0).clicked() {
                        s.sub_border_color = *col;
                    }
                }
            });
            setting_row(ui, &t, t!("Umrissstärke"), "", |ui| {
                widgets::value_box(ui, &t, &format!("{:.1} px", s.sub_border_size), 70.0);
            });
            widgets::slider_ex(ui, &t, &mut s.sub_border_size, 0.0, 8.0, w, Some(2.5), &[]);
            s.sub_border_size = (s.sub_border_size * 2.0).round() / 2.0;
            setting_row(ui, &t, t!("Hintergrund-Deckkraft"), "", |ui| {
                widgets::value_box(ui, &t, &format!("{:.0} %", s.sub_back_opacity * 100.0), 60.0);
            });
            widgets::slider_ex(ui, &t, &mut s.sub_back_opacity, 0.0, 1.0, w, None, &[]);
            s.sub_back_opacity = (s.sub_back_opacity * 20.0).round() / 20.0;
            setting_row(ui, &t, "Position", t!("100 = unten, kleinere Werte schieben die Untertitel nach oben."), |ui| {
                widgets::value_box(ui, &t, &format!("{:.0}", s.sub_pos), 60.0);
            });
            widgets::slider_ex(ui, &t, &mut s.sub_pos, 0.0, 110.0, w, Some(100.0), &[100.0]);
            s.sub_pos = s.sub_pos.round();
            setting_row(ui, &t, t!("Fett"), "", |ui| {
                widgets::toggle(ui, &t, &mut s.sub_bold);
            });
        });
        let style_changed = s.sub_size_pixels != self.data.settings.sub_size_pixels
            || s.sub_font != self.data.settings.sub_font
            || s.sub_font_size != self.data.settings.sub_font_size
            || s.sub_color != self.data.settings.sub_color
            || s.sub_border_color != self.data.settings.sub_border_color
            || s.sub_border_size != self.data.settings.sub_border_size
            || s.sub_back_opacity != self.data.settings.sub_back_opacity
            || s.sub_pos != self.data.settings.sub_pos
            || s.sub_bold != self.data.settings.sub_bold;
        if style_changed {
            if s.sub_size_pixels && !self.data.settings.sub_size_pixels {
                c.set("sub-scale", 1.0);
            }
            self.data.settings = s;
            self.apply_sub_style_from_settings();
            self.mark_dirty();
        }
        if (sub_scale - st.sub_scale).abs() > 0.001 && !self.data.settings.sub_size_pixels {
            c.set("sub-scale", (sub_scale * 20.0).round() / 20.0);
        }

        // Presets per show (folder)
        let folder = self.current_folder().map(|f| f.to_string_lossy().to_string());
        let folder_preset = self.folder_preset_name();
        let names: Vec<String> = self.data.presets.iter().map(|p| p.name.clone()).collect();
        let mut apply: Option<String> = None;
        let mut delete: Option<String> = None;
        let mut unlink = false;
        let mut save = false;
        self.card(ui, "Presets", |ui| {
            ui.label(
                RichText::new(t!("Speichere Verzögerung, Darstellung und bevorzugte Spuren für eine Serie. Das Preset wird bei jeder Folge im Ordner automatisch angewendet."))
                    .size(12.0)
                    .color(t.text_dim),
            );
            ui.add_space(4.0);
            if let (Some(_), Some(fp)) = (&folder, &folder_preset) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(ph::FOLDER).color(t.accent));
                    ui.label(tf!("Dieser Ordner verwendet „{fp}“"));
                    if ui.small_button(t!("Lösen")).clicked() {
                        unlink = true;
                    }
                });
            }
            if widgets::icon_text_button(ui, &t, ph::FLOPPY_DISK, t!("Aktuelle Einstellungen als Preset speichern…")).clicked() {
                save = true;
            }
            for n in &names {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(ph::BOOKMARK_SIMPLE).color(t.text_faint));
                    ui.label(n);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, &t, ph::TRASH, t!("Löschen"), false).clicked() {
                            delete = Some(n.clone());
                        }
                        if ui.small_button(t!("Anwenden")).clicked() {
                            apply = Some(n.clone());
                        }
                    });
                });
            }
        });
        if save {
            self.dlg.preset_name = self
                .current_folder()
                .map(|f| crate::library::name_of(&f))
                .unwrap_or_else(|| t!("Mein Preset").into());
            self.dlg.preset_for_folder = folder.is_some();
            self.dialog = Some(Dialog::SavePreset);
        }
        if let Some(n) = apply {
            if let Some(pr) = self.data.preset(&n).cloned() {
                self.apply_preset(&pr, true);
                self.osd(tf!("Preset „{n}“ angewendet"));
            }
        }
        if let Some(n) = delete {
            self.data.presets.retain(|p| p.name != n);
            self.data.folder_presets.retain(|_, v| *v != n);
            self.mark_dirty();
        }
        if unlink {
            if let Some(f) = folder {
                self.data.folder_presets.remove(&f);
                self.mark_dirty();
            }
        }
    }

    fn tab_marks(&mut self, ui: &mut Ui) {
        let t = self.t;
        let p = self.player.as_ref().unwrap();
        let (c, st) = (p.ctl.clone(), p.st.clone());
        let combo = self.shortcut(Action::Bookmark);
        let mut add = false;
        let path = st.path.clone();
        let mut bms = self.data.bookmarks.get(&path).cloned().unwrap_or_default();
        let mut changed = false;
        self.card(ui, t!("Lesezeichen"), |ui| {
            if widgets::icon_text_button(ui, &t, ph::BOOKMARK_SIMPLE, &tf!("Lesezeichen hinzufügen  ({combo})")).clicked() {
                add = true;
            }
            ui.add_space(4.0);
            if bms.is_empty() {
                ui.label(RichText::new(t!("Noch keine Lesezeichen für diese Datei.")).color(t.text_faint));
            }
            let mut remove = None;
            for (i, b) in bms.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let r = ui.add(egui::Button::new(RichText::new(media::fmt_time(b.time)).color(t.accent)).frame(false));
                    if r.clicked() {
                        c.seek_abs(b.time, true);
                    }
                    let w = ui.available_width() - 36.0;
                    if ui.add(egui::TextEdit::singleline(&mut b.name).desired_width(w)).changed() {
                        changed = true;
                    }
                    if icon_button(ui, &t, ph::TRASH, t!("Entfernen"), false).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                bms.remove(i);
                changed = true;
            }
        });
        if changed {
            if bms.is_empty() {
                self.data.bookmarks.remove(&path);
            } else {
                self.data.bookmarks.insert(path.clone(), bms);
            }
            self.mark_dirty();
        }
        if add {
            self.add_bookmark(None);
        }
        self.card(ui, t!("Kapitel"), |ui| {
            if st.chapters.is_empty() {
                ui.label(RichText::new(t!("Diese Datei enthält keine Kapitel.")).color(t.text_faint));
            }
            for (i, ch) in st.chapters.iter().enumerate() {
                let label = format!("{}   {}", media::fmt_time(ch.time), if ch.title.is_empty() { tf!("Kapitel {}", i + 1) } else { ch.title.clone() });
                if widgets::check_row(ui, &t, &label, st.chapter == i as i64).clicked() {
                    c.seek_abs(ch.time, true);
                    // a chosen chapter plays
                    c.set_pause(false);
                }
            }
        });
    }

    // =========================================================== queue

    pub fn queue_panel(&mut self, ui: &mut Ui) {
        let t = self.t;
        let Some(p) = &self.player else { return };
        let (c, st) = (p.ctl.clone(), p.st.clone());
        let n = st.playlist.len();
        ui.horizontal(|ui| {
            ui.set_min_height(34.0);
            ui.label(RichText::new(t!("Warteschlange")).font(theme::semibold(15.0)).color(t.text));
            ui.label(RichText::new(format!("{n}")).color(t.text_faint));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, &t, ph::X, t!("Schließen"), false).clicked() {
                    self.right = RightPanel::None;
                }
            });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icon_button(ui, &t, ph::FILE_PLUS, t!("Dateien hinzufügen"), false).clicked() {
                self.dialog_add_to_queue();
            }
            if icon_button(ui, &t, ph::FOLDER_PLUS, t!("Ordner hinzufügen"), false).clicked() {
                self.pick(crate::pick::Pick::QueueFolder);
            }
            if icon_button(ui, &t, ph::SHUFFLE, t!("Mischen"), false).clicked() {
                c.playlist_shuffle();
            }
            let r = self.data.settings.repeat;
            if icon_button(ui, &t, if r == RepeatMode::One { ph::REPEAT_ONCE } else { ph::REPEAT }, t!("Wiederholen"), r != RepeatMode::Off).clicked() {
                self.set_repeat(match r {
                    RepeatMode::Off => RepeatMode::All,
                    RepeatMode::All => RepeatMode::One,
                    RepeatMode::One => RepeatMode::Off,
                });
            }
            if icon_button(ui, &t, ph::PLAYLIST, t!("Als Playlist speichern"), false).clicked() && n > 0 {
                let items: Vec<String> = st.playlist.iter().map(|e| e.filename.clone()).collect();
                self.dlg.playlist_name = String::new();
                self.dialog = Some(Dialog::NewPlaylist(items));
            }
            if icon_button(ui, &t, ph::TRASH, t!("Leeren (außer aktuellem Titel)"), false).clicked() {
                c.playlist_clear();
            }
        });
        ui.add_space(4.0);
        widgets::separator(ui, &t);

        let mut play_idx = None;
        let mut remove_idx = None;
        let mut move_op: Option<(usize, usize)> = None;
        let mut next_op: Option<usize> = None;
        let mut save_one: Option<String> = None;
        let frame = egui::Frame::new();
        let (_, dropped) = ui.dnd_drop_zone::<LibDrag, ()>(frame, |ui| {
            if n == 0 {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(ph::QUEUE).font(theme::icon(34.0)).color(t.text_faint));
                    ui.label(RichText::new(t!("Die Warteschlange ist leer")).color(t.text_dim));
                    ui.label(RichText::new(t!("Ziehe Dateien hierher oder aus der Bibliothek.")).size(12.0).color(t.text_faint));
                });
                ui.add_space(ui.available_height().max(0.0));
                return;
            }
            // Touch: a swipe on a row scrolls the list; rows move only by
            // their grip on the right, a tap plays, a long press opens the menu.
            let touch = crate::mobile::touch();
            let look = self.queue_look(&st.playlist);
            let row_h = look.row_h();
            let ctx = ui.ctx().clone();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, n, |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for i in range {
                    let e = &st.playlist[i];
                    let w = ui.available_width();
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, row_h), if touch { Sense::click() } else { Sense::click_and_drag() });
                    let grip = touch.then(|| {
                        let gr = Rect::from_min_max(pos2(rect.right() - 44.0, rect.top()), rect.max);
                        (gr, ui.interact(gr, Id::new(("qgrip", i)), Sense::drag()))
                    });
                    let current = e.current || i as i64 == st.playlist_pos;
                    let painter = ui.painter();
                    if current {
                        painter.rect_filled(rect, CornerRadius::same(theme::R), t.accent_soft);
                    } else if resp.hovered() {
                        painter.rect_filled(rect, CornerRadius::same(theme::R), t.hover);
                    }
                    let info = self.row_info(&ctx, e, true);
                    let dur = info.dur;
                    let right_w = if touch { 96.0 } else if resp.hovered() { 34.0 } else { 52.0 };
                    crate::ui_queue::paint_row(ui.painter(), &t, rect, &info, look, i, current, st.paused, right_w);
                    if let Some((gr, g)) = &grip {
                        let active = g.dragged() || g.hovered();
                        ui.painter().text(gr.center(), Align2::CENTER_CENTER, ph::DOTS_SIX_VERTICAL, theme::icon(18.0), if active { t.accent } else { t.text_faint });
                        if dur > 0.0 {
                            ui.painter().text(pos2(gr.left() - 4.0, rect.center().y), Align2::RIGHT_CENTER, media::fmt_time_short(dur), FontId::proportional(11.5), t.text_faint);
                        }
                    } else if resp.hovered() {
                        let xr = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(24.0, 24.0));
                        let xresp = ui.interact(xr, Id::new(("qx", i)), Sense::click());
                        ui.painter().text(xr.center(), Align2::CENTER_CENTER, ph::X, theme::icon(14.0), if xresp.hovered() { t.danger } else { t.text_dim });
                        if xresp.clicked() {
                            remove_idx = Some(i);
                        }
                    } else if dur > 0.0 {
                        ui.painter().text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, media::fmt_time_short(dur), FontId::proportional(11.5), t.text_faint);
                    }
                    if dur <= 0.0 {
                        let f = e.filename.clone();
                        self.tools.request_probe(&f);
                    }
                    match &grip {
                        Some((_, g)) => g.dnd_set_drag_payload(i),
                        None => resp.dnd_set_drag_payload(i),
                    }
                    if let Some(from) = resp.dnd_hover_payload::<usize>() {
                        if *from != i {
                            let y = if *from < i { rect.bottom() } else { rect.top() };
                            ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(2.0, t.accent));
                        }
                    }
                    if let Some(from) = resp.dnd_release_payload::<usize>() {
                        if *from != i {
                            let to = if *from < i { i + 1 } else { i };
                            move_op = Some((*from, to));
                        }
                    }
                    if if touch { resp.clicked() } else { resp.double_clicked() } {
                        play_idx = Some(i);
                    }
                    resp.context_menu(|ui| {
                        if widgets::menu_item(ui, &t, ph::PLAY, t!("Abspielen"), "", false).clicked() {
                            play_idx = Some(i);
                            ui.close();
                        }
                        if widgets::menu_item(ui, &t, ph::ARROW_BEND_DOWN_RIGHT, t!("Als Nächstes abspielen"), "", false).clicked() {
                            next_op = Some(i);
                            ui.close();
                        }
                        if widgets::menu_item(ui, &t, ph::PLAYLIST, t!("Zu Playlist hinzufügen…"), "", false).clicked() {
                            save_one = Some(e.filename.clone());
                            ui.close();
                        }
                        if !crate::platform::MOBILE && !media::is_url(&e.filename) && widgets::menu_item(ui, &t, ph::FOLDER_OPEN, crate::platform::show_in_folder_label(), "", false).clicked() {
                            show_in_explorer(&e.filename);
                            ui.close();
                        }
                        if widgets::menu_item(ui, &t, ph::TRASH, t!("Entfernen"), "", false).clicked() {
                            remove_idx = Some(i);
                            ui.close();
                        }
                    });
                }
            });
        });
        if let Some(d) = dropped {
            self.enqueue(&d.0);
        }
        if let Some(i) = play_idx {
            c.playlist_play(i);
            c.set_pause(false);
        }
        if let Some(i) = remove_idx {
            c.playlist_remove(i);
        }
        if let Some((a, b)) = move_op {
            c.playlist_move(a, b);
        }
        if let Some(i) = next_op {
            let pos = st.playlist_pos.max(0) as usize;
            let to = pos + 1;
            if i != pos {
                c.playlist_move(i, if i < to { to } else { to });
            }
        }
        if let Some(f) = save_one {
            self.dialog = Some(Dialog::AddToPlaylist(vec![f]));
        }
    }
}

#[allow(dead_code)]
fn _unused(_: Instant) {}

/// Display name of an equalizer preset ("Sprache" is "Speech" here, not "Language").
fn eq_label(n: &str) -> &str {
    if n == "Sprache" && crate::i18n::is_en() { "Speech" } else { crate::i18n::tr(n) }
}
