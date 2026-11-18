//! Runs a visualizer preset with OpenGL (desktop GL 3.3 / GLES 3.0): the
//! manifest's passes (simulations at 60 steps per second fed by the Lua
//! score, frame passes), then Velouna's shared finish into the player area.
//! Lives inside an egui paint callback; egui restores its own GL state after.

use super::analysis::{Analysis, SectionKind, env_at, FPS};
use super::preset::{PassDef, Preset};
use super::score::{BuildCtx, Score, pulse};
use eframe::egui;
use eframe::glow::{self, HasContext};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SIM_HZ: f32 = 60.0;
const SIM_DT: f32 = 1.0 / SIM_HZ;
const OPS_PER_STEP: usize = 64;
const MAX_BATCH: usize = 128;
/// After a seek the simulation restarts this many seconds before the target.
const WARM_SECS: f32 = 20.0;
/// Largest side of the drawn picture (bigger areas are scaled up by the finish).
const MAX_SIDE: i32 = 1920;

const PRELUDE: &str = include_str!("prelude.glsl");
const POST: &str = include_str!("post.glsl");
const VERT: &str = "void main() {\n    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));\n    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);\n}\n";

/// How strongly the picture follows the music (SunoViz defaults).
#[derive(Clone, Copy)]
pub struct React {
    pub kick: f32,
    pub snare: f32,
    pub hats: f32,
    pub bass: f32,
    pub voice: f32,
    pub keys: f32,
    pub dynamics: f32,
    pub camera: f32,
}

impl Default for React {
    fn default() -> Self {
        React { kick: 1.0, snare: 1.0, hats: 0.7, bass: 1.0, voice: 1.0, keys: 0.8, dynamics: 0.8, camera: 0.6 }
    }
}

/// Everything a simulation depends on; a change restarts it.
#[derive(Clone)]
pub struct Setup {
    pub plugin_dir: PathBuf,
    pub preset: Preset,
    pub song: Arc<Analysis>,
    pub seed: u32,
    pub params: Vec<(String, f32)>,
    pub palette: [[f32; 3]; 7],
    pub react: React,
}

impl Setup {
    fn key(&self) -> String {
        format!(
            "{}|{}|{:p}|{}|{:?}",
            self.plugin_dir.display(),
            self.preset.id,
            Arc::as_ptr(&self.song),
            self.seed,
            self.params
        )
    }
}

struct Target {
    tex: [Option<glow::Texture>; 2],
    fbo: [Option<glow::Framebuffer>; 2],
    cur: usize,
    w: i32,
    h: i32,
}

struct Pass {
    def: PassDef,
    prog: glow::Program,
    target: Option<Target>,
}

pub struct Renderer {
    inited: bool,
    float_ok: bool,
    embedded: bool,
    vao: Option<glow::VertexArray>,
    ops_tex: Option<glow::Texture>,
    post: Option<glow::Program>,
    passes: Vec<Pass>,
    prog_key: String,
    setup: Option<Setup>,
    built_key: String,
    score: Option<Score>,
    flow: Vec<f32>,
    steps_done: Option<u64>,
    size: (i32, i32),
    want: ((i32, i32), Instant),
    pub time: f32,
    frame_no: u64,
    pub error: Option<String>,
    /// Still catching up after a seek or a start (true: draw anyway, ask for frames).
    pub busy: bool,
}

impl Default for Renderer {
    fn default() -> Self {
        Renderer {
            inited: false,
            float_ok: true,
            embedded: false,
            vao: None,
            ops_tex: None,
            post: None,
            passes: vec![],
            prog_key: String::new(),
            setup: None,
            built_key: String::new(),
            score: None,
            flow: vec![],
            steps_done: None,
            size: (0, 0),
            want: ((0, 0), Instant::now()),
            time: 0.0,
            frame_no: 0,
            error: None,
            busy: false,
        }
    }
}

impl Renderer {
    /// Reads shaders and score.lua from disk again on the next frame (after
    /// "Rescan", while writing a plugin).
    pub fn reload(&mut self) {
        self.prog_key.clear();
        self.built_key.clear();
        self.error = None;
    }

    pub fn set_setup(&mut self, s: Setup) {
        if self.setup.as_ref().is_none_or(|o| o.key() != s.key() || o.palette != s.palette) {
            if self.setup.as_ref().is_none_or(|o| !Arc::ptr_eq(&o.song, &s.song) || o.react.bass != s.react.bass) {
                self.flow = flow_curve(&s.song, &s.react);
            }
            self.setup = Some(s);
        }
    }

    /// Draws into `vp`. Called from the paint callback.
    pub fn paint(&mut self, gl: &glow::Context, vp: egui::epaint::ViewportInPixels) {
        if self.setup.is_none() {
            return;
        }
        unsafe {
            if !self.inited {
                if let Err(e) = self.init(gl) {
                    self.error = Some(e);
                }
                self.inited = true;
            }
            if self.post.is_none() {
                return;
            }
            if let Err(e) = self.paint_inner(gl, vp) {
                log::warn!("visualizer: {e}");
                self.error = Some(e);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }

    unsafe fn init(&mut self, gl: &glow::Context) -> Result<(), String> {
        unsafe {
            self.embedded = gl.version().is_embedded;
            if self.embedded {
                let ext = gl.supported_extensions();
                self.float_ok = ext.contains("GL_EXT_color_buffer_float") || ext.contains("GL_EXT_color_buffer_half_float");
            }
            self.vao = Some(gl.create_vertex_array()?);
            let t = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA32F as i32,
                (OPS_PER_STEP * 3) as i32,
                MAX_BATCH as i32,
                0,
                glow::RGBA,
                glow::FLOAT,
                glow::PixelUnpackData::Slice(None),
            );
            for (k, v) in [(glow::TEXTURE_MIN_FILTER, glow::NEAREST), (glow::TEXTURE_MAG_FILTER, glow::NEAREST)] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            self.ops_tex = Some(t);
            self.post = Some(self.program(gl, POST, "post")?);
            Ok(())
        }
    }

    unsafe fn program(&self, gl: &glow::Context, frag: &str, name: &str) -> Result<glow::Program, String> {
        let head = if self.embedded {
            "#version 300 es\nprecision highp float;\nprecision highp int;\nprecision highp sampler2D;\n"
        } else {
            "#version 330 core\n"
        };
        unsafe {
            let prog = gl.create_program()?;
            let mut shaders = vec![];
            for (kind, src) in [(glow::VERTEX_SHADER, format!("{head}{VERT}")), (glow::FRAGMENT_SHADER, format!("{head}{PRELUDE}\n#line 1 1\n{frag}"))] {
                let s = gl.create_shader(kind)?;
                gl.shader_source(s, &src);
                gl.compile_shader(s);
                if !gl.get_shader_compile_status(s) {
                    let log = gl.get_shader_info_log(s);
                    gl.delete_shader(s);
                    gl.delete_program(prog);
                    return Err(format!("{name}: {}", log.trim()));
                }
                gl.attach_shader(prog, s);
                shaders.push(s);
            }
            gl.link_program(prog);
            for s in shaders {
                gl.detach_shader(prog, s);
                gl.delete_shader(s);
            }
            if !gl.get_program_link_status(prog) {
                let log = gl.get_program_info_log(prog);
                gl.delete_program(prog);
                return Err(format!("{name}: {}", log.trim()));
            }
            Ok(prog)
        }
    }

    unsafe fn free_targets(&mut self, gl: &glow::Context) {
        for p in &mut self.passes {
            if let Some(t) = p.target.take() {
                unsafe {
                    for x in t.tex.into_iter().flatten() {
                        gl.delete_texture(x);
                    }
                    for x in t.fbo.into_iter().flatten() {
                        gl.delete_framebuffer(x);
                    }
                }
            }
        }
    }

    unsafe fn make_target(&self, gl: &glow::Context, def: &PassDef, w: i32, h: i32) -> Result<Target, String> {
        let float = def.format != "rgba8" && self.float_ok;
        let n = if def.feedback { 2 } else { 1 };
        let mut t = Target { tex: [None, None], fbo: [None, None], cur: 0, w, h };
        unsafe {
            for i in 0..n {
                let tex = gl.create_texture()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                let (ifmt, ty) = if float { (glow::RGBA16F, glow::HALF_FLOAT) } else { (glow::RGBA8, glow::UNSIGNED_BYTE) };
                gl.tex_image_2d(glow::TEXTURE_2D, 0, ifmt as i32, w, h, 0, glow::RGBA, ty, glow::PixelUnpackData::Slice(None));
                for (k, v) in [
                    (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                    (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                    (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                    (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                ] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
                }
                let fbo = gl.create_framebuffer()?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
                if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                    return Err(format!("{}: render target not supported", def.id));
                }
                gl.viewport(0, 0, w, h);
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                t.tex[i] = Some(tex);
                t.fbo[i] = Some(fbo);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        Ok(t)
    }

    unsafe fn paint_inner(&mut self, gl: &glow::Context, vp: egui::epaint::ViewportInPixels) -> Result<(), String> {
        let setup = self.setup.clone().unwrap();
        // Programs of the preset.
        let pk = format!("{}|{}", setup.plugin_dir.display(), setup.preset.id);
        if self.prog_key != pk {
            unsafe {
                self.free_targets(gl);
                for p in self.passes.drain(..) {
                    gl.delete_program(p.prog);
                }
            }
            self.prog_key = pk;
            self.error = None;
            for def in &setup.preset.passes {
                let src = std::fs::read_to_string(setup.plugin_dir.join(&def.shader)).map_err(|e| format!("{}: {e}", def.shader))?;
                let prog = unsafe { self.program(gl, &src, &def.shader)? };
                self.passes.push(Pass { def: def.clone(), prog, target: None });
            }
            self.size = (0, 0);
        }
        if self.passes.is_empty() {
            return Ok(());
        }
        // Output size, capped; resizes settle before the targets are rebuilt.
        let (vw, vh) = (vp.width_px.max(1), vp.height_px.max(1));
        let k = (MAX_SIDE as f32 / vw.max(vh) as f32).min(1.0);
        let want = (((vw as f32) * k).round() as i32, ((vh as f32) * k).round() as i32);
        if want != self.want.0 {
            self.want = (want, Instant::now());
        }
        if self.size != want && (self.size == (0, 0) || self.want.1.elapsed() > Duration::from_millis(250)) {
            unsafe { self.free_targets(gl) };
            for i in 0..self.passes.len() {
                let def = self.passes[i].def.clone();
                let (w, h) = (((want.0 as f32) * def.scale).round().max(1.0) as i32, ((want.1 as f32) * def.scale).round().max(1.0) as i32);
                let t = unsafe { self.make_target(gl, &def, w, h)? };
                self.passes[i].target = Some(t);
            }
            self.size = want;
            self.built_key.clear();
        }
        // Score for this song, seed, settings and aspect.
        let key = format!("{}|{:?}", setup.key(), self.size);
        if self.built_key != key {
            self.built_key = key;
            self.steps_done = None;
            self.score = None;
            if let Some(path) = &setup.preset.score {
                let src = std::fs::read_to_string(setup.plugin_dir.join(path)).map_err(|e| format!("{path}: {e}"))?;
                let m = self.size.0.min(self.size.1).max(1) as f32;
                let r = &setup.react;
                let react = [("kick", r.kick), ("snare", r.snare), ("hats", r.hats), ("bass", r.bass), ("voice", r.voice), ("keys", r.keys), ("dynamics", r.dynamics), ("camera", r.camera)];
                let ctx = BuildCtx { seed: setup.seed, aw: self.size.0 as f32 / m, ah: self.size.1 as f32 / m, params: &setup.params, react: &react };
                let t0 = Instant::now();
                self.score = Some(Score::new(&src, path, setup.song.clone(), &ctx)?);
                log::info!("visualizer: {} score built in {} ms", setup.preset.id, t0.elapsed().as_millis());
            }
        }
        let (w, h) = self.size;
        let t = self.time.max(0.0);
        // Rings for this frame; an error in frame() stops the preset like one in step().
        let mut rips = vec![];
        if let Some(sc) = &self.score {
            sc.frame(t, &mut rips)?;
        }
        let u = self.uniforms(&setup, t, w as f32, h as f32, &rips);
        let flat: &[f32] = bytemuck_flat(&u);
        unsafe {
            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.color_mask(true, true, true, true);
            gl.bind_vertex_array(self.vao);
            self.run_steps(gl, &setup, t, flat)?;
            // Frame passes.
            for i in 0..self.passes.len() {
                if self.passes[i].def.rate == "step" {
                    continue;
                }
                self.draw_pass(gl, i, flat, [0.0; 4]);
            }
            // Finish into the player area.
            let post = self.post.unwrap();
            let scene = self.passes.last().and_then(|p| p.target.as_ref()).and_then(|t| t.tex[t.cur]);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.viewport(vp.left_px, vp.from_bottom_px, vw, vh);
            gl.use_program(Some(post));
            set_common(gl, post, flat, (vw as f32, vh as f32), (vp.left_px as f32, vp.from_bottom_px as f32), [0.0; 4]);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, scene);
            gl.uniform_1_i32(gl.get_uniform_location(post, "u_scene").as_ref(), 0);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        self.frame_no += 1;
        Ok(())
    }

    /// Advances the step passes to `t` (60 per second), within a budget per frame.
    unsafe fn run_steps(&mut self, gl: &glow::Context, setup: &Setup, t: f32, flat: &[f32]) -> Result<(), String> {
        let Some(score) = self.score.take() else { return Ok(()) };
        let r = unsafe { self.run_steps_with(gl, &score, setup, t, flat) };
        self.score = Some(score);
        r
    }

    unsafe fn run_steps_with(&mut self, gl: &glow::Context, score: &Score, setup: &Setup, t: f32, flat: &[f32]) -> Result<(), String> {
        if !score.has_steps() || !self.passes.iter().any(|p| p.def.rate == "step") {
            return Ok(());
        }
        let target = (t * SIM_HZ).floor() as u64 + 1;
        let warm = (WARM_SECS * SIM_HZ) as u64;
        let mut first = false;
        let restart = match self.steps_done {
            None => true,
            Some(d) => target < d || target - d > warm,
        };
        if restart {
            for p in &mut self.passes {
                if p.def.rate == "step" {
                    if let Some(tg) = &mut p.target {
                        for f in tg.fbo.iter().flatten() {
                            unsafe {
                                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(*f));
                                gl.viewport(0, 0, tg.w, tg.h);
                                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                                gl.clear(glow::COLOR_BUFFER_BIT);
                            }
                        }
                        tg.cur = 0;
                    }
                }
            }
            self.steps_done = Some(target.saturating_sub(warm));
            first = true;
        }
        let budget: u64 = if crate::platform::MOBILE { 48 } else { 240 };
        let mut done = self.steps_done.unwrap();
        let end = target.min(done + budget);
        let mut ops: Vec<[f32; 12]> = Vec::with_capacity(OPS_PER_STEP * MAX_BATCH);
        let mut rows: Vec<(usize, f32)> = Vec::with_capacity(MAX_BATCH);
        let mut buf = vec![0f32; OPS_PER_STEP * 12 * MAX_BATCH];
        while done < end {
            let n = ((end - done) as usize).min(MAX_BATCH);
            rows.clear();
            buf.iter_mut().for_each(|x| *x = 0.0);
            for i in 0..n {
                ops.clear();
                let t0 = (done + i as u64) as f32 * SIM_DT;
                let decay = score.step(t0, SIM_DT, first && i == 0, &mut ops, OPS_PER_STEP)?;
                let row = &mut buf[i * OPS_PER_STEP * 12..(i + 1) * OPS_PER_STEP * 12];
                for (k, op) in ops.iter().enumerate() {
                    row[k * 12..k * 12 + 12].copy_from_slice(op);
                }
                rows.push((ops.len(), decay));
            }
            first = false;
            unsafe {
                gl.bind_texture(glow::TEXTURE_2D, self.ops_tex);
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
                let bytes: &[u8] = std::slice::from_raw_parts(buf.as_ptr() as *const u8, n * OPS_PER_STEP * 12 * 4);
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    0,
                    (OPS_PER_STEP * 3) as i32,
                    n as i32,
                    glow::RGBA,
                    glow::FLOAT,
                    glow::PixelUnpackData::Slice(Some(bytes)),
                );
                gl.bind_texture(glow::TEXTURE_2D, None);
                for (i, (cnt, decay)) in rows.iter().enumerate() {
                    for p in 0..self.passes.len() {
                        if self.passes[p].def.rate == "step" {
                            self.draw_pass(gl, p, flat, [*cnt as f32, *decay, i as f32, 0.0]);
                        }
                    }
                }
            }
            done += n as u64;
        }
        self.steps_done = Some(done);
        self.busy = done < target;
        let _ = setup;
        Ok(())
    }

    /// One pass into its target (flipping a feedback pair).
    unsafe fn draw_pass(&mut self, gl: &glow::Context, i: usize, flat: &[f32], step: [f32; 4]) {
        let inputs: Vec<(String, Option<glow::Texture>)> = self.passes[i]
            .def
            .inputs
            .iter()
            .map(|id| (id.clone(), self.passes.iter().find(|p| &p.def.id == id).and_then(|p| p.target.as_ref()).and_then(|t| t.tex[t.cur])))
            .collect();
        let ops = self.ops_tex;
        let p = &mut self.passes[i];
        let prog = p.prog;
        let feedback = p.def.feedback;
        let Some(tg) = &mut p.target else { return };
        let (read, write) = if feedback { (tg.tex[tg.cur], 1 - tg.cur) } else { (None, 0) };
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, tg.fbo[write]);
            gl.viewport(0, 0, tg.w, tg.h);
            gl.use_program(Some(prog));
            set_common(gl, prog, flat, (tg.w as f32, tg.h as f32), (0.0, 0.0), step);
            let mut unit = 0u32;
            let mut bind = |name: &str, tex: Option<glow::Texture>| {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, tex);
                gl.uniform_1_i32(gl.get_uniform_location(prog, name).as_ref(), unit as i32);
                unit += 1;
            };
            bind("u_ops", ops);
            bind("u_prev", read);
            for (id, tex) in &inputs {
                bind(&format!("u_{id}"), *tex);
            }
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            for u in 0..unit {
                gl.active_texture(glow::TEXTURE0 + u);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            gl.active_texture(glow::TEXTURE0);
        }
        tg.cur = write;
    }

    fn uniforms(&self, s: &Setup, t: f32, w: f32, h: f32, rips: &[[f32; 4]]) -> [[f32; 4]; 38] {
        let an = &s.song;
        let e = &an.env;
        let r = &s.react;
        let m = w.min(h);
        let (beat, bar) = an.phases(t);
        let (build, flash) = build_drop(an, t);
        let sg = section_gain(an, r, t);
        let kick = pulse(&an.kick, t, 0.14);
        let bass = env_at(&e.bass, t);
        let cam = r.camera;
        let seedf = (s.seed % 1000) as f32 + (s.seed / 1000 % 997) as f32 * 0.001;
        let zoom = 1.0 + cam * (0.016 * kick * r.kick * sg + 0.006 * bass);
        let sm = |x: f32| x * x * (3.0 - 2.0 * x);
        let fade = sm((t / 1.5).clamp(0.0, 1.0)) * sm(((an.dur - t) / 2.5).clamp(0.0, 1.0));
        let ph = s.seed as f32 * 0.37;
        let p = |i: usize| s.params.get(i).map(|x| x.1).unwrap_or(0.0);
        let mut u = [[0f32; 4]; 38];
        u[0] = [w, h, 1.0 / w, 1.0 / h];
        u[1] = [t, an.dur, seedf, m];
        u[2] = [env_at(&e.sub, t), bass, env_at(&e.drums, t), env_at(&e.mix, t)];
        u[3] = [0.0, env_at(&e.mids, t) * r.keys, env_at(&e.presence, t), env_at(&e.high, t)];
        u[4] = [env_at(&e.intensity, t), sg, build, flash];
        u[5] = [kick * sg, pulse(&an.snare, t, 0.11) * sg, pulse(&an.hat, t, 0.06) * sg, pulse(&an.tonal, t, 0.25)];
        u[6] = [beat, bar, env_at(&self.flow, t), self.frame_no as f32];
        for (i, c) in s.palette.iter().enumerate() {
            u[7 + i] = [c[0], c[1], c[2], 1.0];
        }
        u[14] = [p(0), p(1), p(2), p(3)];
        u[15] = [p(4), p(5), p(6), p(7)];
        u[16] = [r.kick, r.snare, r.hats, r.bass];
        u[17] = [r.voice, r.keys, r.dynamics, cam];
        u[18] = [0.55, 0.2, fade, 0.0];
        u[19] = [1.0, 1.0, zoom, 0.0];
        u[20] = [cam * 0.004 * (t * 0.13 + ph).sin(), cam * 0.004 * (t * 0.11 + ph * 1.3).cos(), 0.0, rips.len().min(16) as f32];
        for (i, rp) in rips.iter().take(16).enumerate() {
            u[22 + i] = *rp;
        }
        u
    }
}

fn bytemuck_flat(u: &[[f32; 4]; 38]) -> &[f32] {
    // [[f32; 4]; 38] is laid out as 152 consecutive f32.
    unsafe { std::slice::from_raw_parts(u.as_ptr() as *const f32, 38 * 4) }
}

unsafe fn set_common(gl: &glow::Context, prog: glow::Program, flat: &[f32], size: (f32, f32), origin: (f32, f32), step: [f32; 4]) {
    unsafe {
        gl.uniform_4_f32_slice(gl.get_uniform_location(prog, "u_f").as_ref(), flat);
        gl.uniform_2_f32(gl.get_uniform_location(prog, "u_size").as_ref(), size.0, size.1);
        gl.uniform_2_f32(gl.get_uniform_location(prog, "u_origin").as_ref(), origin.0, origin.1);
        gl.uniform_4_f32(gl.get_uniform_location(prog, "u_step").as_ref(), step[0], step[1], step[2], step[3]);
    }
}

/// Integral of the bass drive: a smooth, seekable "flow" time.
fn flow_curve(an: &Analysis, r: &React) -> Vec<f32> {
    let n = an.env.mix.len();
    let mut flow = Vec::with_capacity(n + 1);
    let mut acc = 0f32;
    flow.push(0.0);
    for i in 0..n {
        let b = 0.6 * an.env.bass.get(i).copied().unwrap_or(0.0) + 0.4 * an.env.intensity.get(i).copied().unwrap_or(0.0);
        acc += (0.35 + 0.9 * b * r.bass) / FPS;
        flow.push(acc);
    }
    flow
}

/// Section energy with a soft cross-fade (±0.6 s).
fn section_energy(an: &Analysis, t: f32) -> f32 {
    let e = |t: f32| an.section_at(t).map(|s| s.kind.energy()).unwrap_or(0.6);
    (e(t - 0.6) + 2.0 * e(t) + e(t + 0.6)) * 0.25
}

fn section_gain(an: &Analysis, r: &React, t: f32) -> f32 {
    1.0 - r.dynamics + r.dynamics * section_energy(an, t)
}

/// (progress in a build-up 0..1, drop flash 0..1)
fn build_drop(an: &Analysis, t: f32) -> (f32, f32) {
    let Some(s) = an.section_at(t) else { return (0.0, 0.0) };
    let build = if s.kind == SectionKind::Build { ((t - s.start) / (s.end - s.start).max(0.1)).clamp(0.0, 1.0) } else { 0.0 };
    let mut flash = 0.0;
    if s.kind == SectionKind::Drop {
        let idx = an.sections.iter().position(|x| x.start == s.start).unwrap_or(0);
        if !(idx > 0 && an.sections[idx - 1].kind == SectionKind::Drop) {
            flash = (-(t - s.start) / 0.7).exp();
        }
    }
    (build, flash)
}
