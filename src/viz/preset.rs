//! The `visualizer` part of a plugin manifest: presets with their passes,
//! parameters and palettes.

use crate::plugins::Text;
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct VizManifest {
    // "needs" ("song") is accepted in plugin.json but not read yet: every
    // visualizer gets the song analysis.
    pub presets: Vec<Preset>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Preset {
    pub id: String,
    pub name: Text,
    /// Lua score (optional), path inside the plugin.
    #[serde(default)]
    pub score: Option<String>,
    pub passes: Vec<PassDef>,
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub palettes: Vec<Palette>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct PassDef {
    pub id: String,
    pub shader: String,
    /// Size relative to the output (simulations often run smaller).
    #[serde(default = "one")]
    pub scale: f32,
    /// "rgba16f" (falls back to "rgba8" where float targets are missing) or "rgba8".
    #[serde(default)]
    pub format: String,
    /// Reads its own previous result as `u_prev`.
    #[serde(default)]
    pub feedback: bool,
    /// "frame" (once per drawn frame) or "step" (once per score step, 60 Hz).
    #[serde(default)]
    pub rate: String,
    /// Earlier passes it samples, as `u_<id>`.
    #[serde(default)]
    pub inputs: Vec<String>,
}

fn one() -> f32 {
    1.0
}

#[derive(Deserialize, Clone, Debug)]
pub struct Param {
    pub id: String,
    pub label: Text,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Palette {
    pub name: Text,
    /// Seven colours "#rrggbb": background, background 2, a, b, c, accent, text.
    pub colors: Vec<String>,
}

impl Palette {
    pub fn rgb(&self) -> [[f32; 3]; 7] {
        let mut out = [[0f32; 3]; 7];
        for (i, c) in self.colors.iter().take(7).enumerate() {
            let h = c.trim_start_matches('#');
            let v = u32::from_str_radix(h, 16).unwrap_or(0);
            out[i] = [((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0];
        }
        out
    }
}
