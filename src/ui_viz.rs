//! Visualizer in the player: the drawing area for music, the button in the
//! player bar (click: on/off, arrow: choose preset, palette, seed, params).
//! The button only exists while a visualizer plugin is switched on.

use crate::app::App;
use crate::player::PlayerState;
use crate::plugins::Kind;
use crate::theme;
use crate::viz::preset::{Preset, VizManifest};
use crate::widgets::{self, icon_button};
use eframe::egui::{self, Align2, Color32, FontId, Rect, RectAlign, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as ph;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The chosen visualizer: plugin, its presets, the active one.
pub struct VizChoice {
    pub plugin_id: String,
    pub dir: PathBuf,
    pub manifest: Arc<VizManifest>,
    pub preset: usize,
}

/// Per-preset look, kept in the plugin's settings.
pub struct Look {
    pub palette: usize,
    pub seed: u32,
    pub params: Vec<(String, f32)>,
}

pub const BUTTON_W: f32 = 30.0 + 4.0 + 16.0;

impl App {
    /// Enabled visualizer plugins with their presets ("plugin/preset" ids).
    fn viz_all(&mut self) -> Vec<(String, PathBuf, Arc<VizManifest>)> {
        self.plugins.ensure();
        let mut out = vec![];
        let list: Vec<(String, String, PathBuf, Option<Value>)> = self
            .plugins
            .enabled(Kind::Visualizer, &self.data.plugins)
            .map(|p| (p.manifest.id.clone(), p.manifest.version.clone(), p.dir.clone(), p.manifest.visualizer.clone()))
            .collect();
        for (id, ver, dir, v) in list {
            let key = format!("{id}@{ver}");
            let m = self
                .viz_manifests
                .entry(key)
                .or_insert_with(|| match v.map(serde_json::from_value::<VizManifest>) {
                    Some(Ok(m)) if !m.presets.is_empty() => Some(Arc::new(m)),
                    Some(Err(e)) => {
                        log::warn!("visualizer {id}: {e}");
                        None
                    }
                    _ => None,
                })
                .clone();
            if let Some(m) = m {
                out.push((id, dir, m));
            }
        }
        out
    }

    pub(crate) fn viz_choice(&mut self) -> Option<VizChoice> {
        let all = self.viz_all();
        let want = self.data.settings.viz_preset.clone();
        let (pid, pre) = want.split_once('/').unwrap_or(("", ""));
        let pick = all.iter().find(|(id, _, _)| id == pid).or(all.first())?;
        let preset = pick.2.presets.iter().position(|p| p.id == pre).unwrap_or(0);
        Some(VizChoice { plugin_id: pick.0.clone(), dir: pick.1.clone(), manifest: pick.2.clone(), preset })
    }

    fn viz_look(&self, plugin: &str, p: &Preset) -> Look {
        let s = self.data.plugins.get(plugin).and_then(|s| s.settings.get("presets")).and_then(|v| v.get(&p.id));
        let palette = s.and_then(|v| v.get("palette")).and_then(Value::as_u64).unwrap_or(0) as usize;
        let seed = s.and_then(|v| v.get("seed")).and_then(Value::as_u64).unwrap_or(1) as u32;
        let params = p
            .params
            .iter()
            .map(|d| {
                let v = s.and_then(|v| v.get("params")).and_then(|v| v.get(&d.id)).and_then(Value::as_f64).map(|x| x as f32).unwrap_or(d.default);
                (d.id.clone(), v.clamp(d.min, d.max))
            })
            .collect();
        Look { palette: palette.min(p.palettes.len().saturating_sub(1)), seed, params }
    }

    fn viz_store(&mut self, plugin: &str, preset: &str, key: &str, value: Value) {
        let st = self.data.plugins.entry(plugin.to_string()).or_default();
        let presets = st.settings.entry("presets").or_insert_with(|| json!({}));
        if !presets.is_object() {
            *presets = json!({});
        }
        let p = presets.as_object_mut().unwrap().entry(preset.to_string()).or_insert_with(|| json!({}));
        if !p.is_object() {
            *p = json!({});
        }
        let obj = p.as_object_mut().unwrap();
        match key.split_once('.') {
            Some((a, b)) => {
                let inner = obj.entry(a.to_string()).or_insert_with(|| json!({}));
                if !inner.is_object() {
                    *inner = json!({});
                }
                inner.as_object_mut().unwrap().insert(b.to_string(), value);
            }
            None => {
                obj.insert(key.to_string(), value);
            }
        }
        self.mark_dirty();
    }

    /// Whether the player area shows a visualizer for what plays now.
    pub(crate) fn viz_shown(&mut self, st: &PlayerState) -> bool {
        st.is_audio_only() && self.data.settings.visualizer && self.viz_choice().is_some()
    }

    /// Draws the visualizer into `rect`; false when it can't (yet), so the
    /// caller draws the usual cover or logo.
    pub(crate) fn draw_visualizer(&mut self, ui: &Ui, painter: &egui::Painter, rect: Rect, st: &PlayerState) -> bool {
        let Some(choice) = self.viz_choice() else { return false };
        let Some(p) = self.player.as_ref() else { return false };
        let headers = if crate::media::is_url(&st.path) { p.mpv.get_string("file-local-options/http-header-fields") } else { None };
        let t = p.mpv.get_f64("audio-pts").or_else(|| p.mpv.get_f64("time-pos")).unwrap_or(st.time) as f32;
        let ctx = ui.ctx().clone();
        let status = self.songs.get(&st.path, headers.as_deref().filter(|h| !h.is_empty()), move || ctx.request_repaint());
        let song = match status {
            crate::viz::Status::Ready(a) => a,
            crate::viz::Status::Working => {
                self.viz_note(painter, rect, t!("Visualizer: Song wird analysiert …"));
                return false;
            }
            crate::viz::Status::Failed => return false,
        };
        let preset = choice.manifest.presets[choice.preset].clone();
        let look = self.viz_look(&choice.plugin_id, &preset);
        let palette = preset.palettes.get(look.palette).map(|p| p.rgb()).unwrap_or([[0.0; 3]; 7]);
        let setup = crate::viz::gl::Setup {
            plugin_dir: choice.dir.clone(),
            preset,
            song,
            seed: look.seed,
            params: look.params,
            palette,
            react: Default::default(),
        };
        let (busy, error) = {
            let mut r = self.viz.lock().unwrap();
            r.set_setup(setup);
            r.time = t;
            (r.busy, r.error.clone())
        };
        if let Some(e) = error {
            self.viz_note(painter, rect, &tf!("Visualizer angehalten: {}", e));
            return false;
        }
        let vz = self.viz.clone();
        painter.add(egui::PaintCallback {
            rect,
            callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, p| {
                if let Ok(mut r) = vz.lock() {
                    r.paint(p.gl(), info.viewport_in_pixels());
                }
            })),
        });
        if !st.paused || busy {
            ui.ctx().request_repaint();
        }
        true
    }

    fn viz_note(&self, painter: &egui::Painter, rect: Rect, text: &str) {
        painter.text(rect.center_bottom() - vec2(0.0, 18.0), Align2::CENTER_BOTTOM, text, FontId::proportional(12.5), self.t.text_faint);
    }

    /// Width the visualizer button takes in the player bar (0 when hidden).
    pub(crate) fn viz_button_width(&mut self) -> f32 {
        let show = self.player.as_ref().is_some_and(|p| p.st.is_audio_only());
        if show && self.viz_choice().is_some() { BUTTON_W } else { 0.0 }
    }

    /// Player bar: on/off and the choice popup.
    pub(crate) fn viz_button(&mut self, ui: &mut Ui) {
        if self.viz_button_width() == 0.0 {
            return;
        }
        let t = self.t;
        let Some(choice) = self.viz_choice() else { return };
        let on = self.data.settings.visualizer;
        let name = choice.manifest.presets[choice.preset].name.get();
        let tip = if on { tf!("Visualizer: {} (an)", name) } else { t!("Visualizer: aus").to_string() };
        if icon_button(ui, &t, ph::SPARKLE, &tip, on).clicked() {
            self.data.settings.visualizer = !on;
            self.mark_dirty();
        }
        let (r, resp) = ui.allocate_exact_size(vec2(16.0, 26.0), Sense::click());
        let resp = resp.on_hover_text(t!("Visualizer wählen"));
        if resp.hovered() {
            ui.painter().rect_filled(r, egui::CornerRadius::same(theme::R), t.hover);
        }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, ph::CARET_UP, theme::icon(11.0), t.text_dim);
        egui::Popup::menu(&resp).align(RectAlign::TOP_END).width(280.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            self.viz_menu(ui);
        });
    }

    fn viz_menu(&mut self, ui: &mut Ui) {
        let t = self.t;
        let all = self.viz_all();
        let Some(choice) = self.viz_choice() else { return };
        let mut on = self.data.settings.visualizer;
        ui.horizontal(|ui| {
            ui.label(RichText::new(t!("Visualizer")).font(theme::semibold(14.0)).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::toggle(ui, &t, &mut on).changed() {
                    self.data.settings.visualizer = on;
                    self.mark_dirty();
                }
            });
        });
        ui.add_space(4.0);
        // Presets of all enabled visualizer plugins.
        for (pid, _, m) in &all {
            for p in &m.presets {
                let id = format!("{pid}/{}", p.id);
                let active = *pid == choice.plugin_id && p.id == choice.manifest.presets[choice.preset].id;
                if widgets::menu_item(ui, &t, ph::SPARKLE, &p.name.get(), "", active).clicked() {
                    self.data.settings.viz_preset = id;
                    self.data.settings.visualizer = true;
                    self.mark_dirty();
                }
            }
        }
        let preset = choice.manifest.presets[choice.preset].clone();
        let look = self.viz_look(&choice.plugin_id, &preset);
        if !preset.palettes.is_empty() {
            widgets::separator(ui, &t);
            ui.label(RichText::new(t!("Palette")).size(12.0).color(t.text_dim));
            ui.horizontal_wrapped(|ui| {
                for (i, pal) in preset.palettes.iter().enumerate() {
                    let c = pal.rgb();
                    let (r, resp) = ui.allocate_exact_size(vec2(56.0, 22.0), Sense::click());
                    let resp = resp.on_hover_text(pal.name.get());
                    let col = |k: usize| Color32::from_rgb((c[k][0] * 255.0) as u8, (c[k][1] * 255.0) as u8, (c[k][2] * 255.0) as u8);
                    let w = r.width() / 4.0;
                    for (j, k) in [0usize, 2, 3, 5].iter().enumerate() {
                        let rr = Rect::from_min_size(r.min + vec2(w * j as f32, 0.0), vec2(w, r.height()));
                        ui.painter().rect_filled(rr, 0, col(*k));
                    }
                    let stroke = if i == look.palette { egui::Stroke::new(2.0, t.accent) } else { egui::Stroke::new(1.0, t.border) };
                    ui.painter().rect_stroke(r, egui::CornerRadius::same(3), stroke, egui::StrokeKind::Outside);
                    if resp.clicked() {
                        self.viz_store(&choice.plugin_id, &preset.id, "palette", json!(i));
                    }
                }
            });
        }
        widgets::separator(ui, &t);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tf!("Variante {}", look.seed)).size(12.5).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, &t, ph::DICE_FIVE, t!("Neue Variante würfeln"), false).clicked() {
                    let s = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(7) % 9973) as u32 + 1;
                    self.viz_store(&choice.plugin_id, &preset.id, "seed", json!(s));
                }
            });
        });
        if !preset.params.is_empty() {
            egui::CollapsingHeader::new(RichText::new(t!("Einstellungen")).size(12.5).color(t.text_dim)).id_salt("viz_params").show(ui, |ui| {
                for (d, (_, v)) in preset.params.iter().zip(look.params.iter()) {
                    let mut x = *v as f64;
                    ui.label(RichText::new(d.label.get()).size(12.0).color(t.text_dim));
                    let r = widgets::slider(ui, &t, &mut x, d.min as f64, d.max as f64, 240.0);
                    if r.changed() {
                        self.viz_store(&choice.plugin_id, &preset.id, &format!("params.{}", d.id), json!(x));
                    }
                }
                if widgets::ghost_button(ui, &t, t!("Zurücksetzen")).clicked() {
                    if let Some(st) = self.data.plugins.get_mut(&choice.plugin_id) {
                        if let Some(Value::Object(m)) = st.settings.get_mut("presets") {
                            if let Some(Value::Object(p)) = m.get_mut(&preset.id) {
                                p.remove("params");
                            }
                        }
                    }
                    self.mark_dirty();
                }
            });
        }
    }
}

/// Shared renderer handle (lives as long as the app).
pub type VizHandle = Arc<Mutex<crate::viz::gl::Renderer>>;
