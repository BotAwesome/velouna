//! A visualizer's `score.lua`: turns the song analysis into events for the
//! shaders. Runs in a sandboxed Lua (no io/os/package, memory and time
//! limits). Velouna lends it fast helpers over the analysis and the same
//! seeded random numbers SunoViz uses, so presets stay deterministic.
//!
//! Script contract (API 1):
//! - `build(song, ctx)` once per song / seed / settings / aspect.
//!   `ctx = { seed, aw, ah, params = { id = value }, react = { … } }`.
//! - `step(t0, dt, first)` per simulation step (60 Hz). Returns a flat list
//!   of 12 numbers per operation (head, a, b as in the shader) and,
//!   optionally, a second value: the decay factor of this step.
//! - `frame(t)` (optional) per drawn frame. May return `{ rip = { {x, y, age, s}, … } }`.

use super::analysis::{Analysis, Hit, env_at};
use mlua::{Function, Lua, LuaOptions, StdLib, Table, UserData, UserDataMethods, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// splitmix64, identical to SunoViz's `Rng`.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u32) -> Self {
        let mut r = Rng((seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03);
        r.next();
        r.next();
        r
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
    pub fn below(&mut self, n: u32) -> u32 {
        (self.next() % n.max(1) as u64) as u32
    }
}

pub fn hash2(seed: u32, i: u32) -> u32 {
    let mut x = seed.wrapping_mul(0x27d4_eb2d) ^ i.wrapping_mul(0x1656_67b1);
    x ^= x >> 15;
    x = x.wrapping_mul(0x85eb_ca6b);
    x ^= x >> 13;
    x = x.wrapping_mul(0xc2b2_ae35);
    x ^ (x >> 16)
}

/// Decaying sum of hits before `t` (time constant `tau`), at most 1.25.
pub fn pulse(hits: &[Hit], t: f32, tau: f32) -> f32 {
    let i = hits.partition_point(|h| h.t <= t);
    let mut s = 0f32;
    for h in hits[..i].iter().rev() {
        let d = t - h.t;
        if d > tau * 6.0 {
            break;
        }
        s += h.s * (-d / tau).exp();
    }
    s.min(1.25)
}

impl UserData for Rng {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method_mut("f", |_, r, ()| Ok(r.f()));
        m.add_method_mut("range", |_, r, (a, b): (f32, f32)| Ok(r.range(a, b)));
        m.add_method_mut("below", |_, r, n: u32| Ok(r.below(n)));
        m.add_method_mut("chance", |_, r, p: f32| Ok(r.f() < p));
    }
}

/// The analysis as seen from Lua.
#[derive(Clone)]
pub struct Song(pub Arc<Analysis>);

impl Song {
    fn hits(&self, name: &str) -> mlua::Result<&[Hit]> {
        let a = &self.0;
        Ok(match name {
            "kick" => &a.kick,
            "snare" => &a.snare,
            "hat" => &a.hat,
            "tonal" | "keys" => &a.tonal,
            _ => return Err(mlua::Error::runtime(format!("unknown hits '{name}'"))),
        })
    }
    fn env(&self, name: &str) -> mlua::Result<&[f32]> {
        let e = &self.0.env;
        Ok(match name {
            "mix" => &e.mix,
            "sub" => &e.sub,
            "bass" => &e.bass,
            "drums" => &e.drums,
            "mids" | "keys" => &e.mids,
            "presence" | "pads" => &e.presence,
            "high" => &e.high,
            "intensity" => &e.intensity,
            "wave" => &self.0.wave,
            // Without stems there is no voice curve.
            "voc" => &[],
            _ => return Err(mlua::Error::runtime(format!("unknown curve '{name}'"))),
        })
    }
}

impl UserData for Song {
    fn add_fields<F: mlua::UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("dur", |_, s| Ok(s.0.dur));
        f.add_field_method_get("bpm", |_, s| Ok(s.0.bpm));
        f.add_field_method_get("beat_period", |_, s| Ok(s.0.beat_period()));
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("env", |_, s, (name, t): (String, f32)| Ok(env_at(s.env(&name)?, t)));
        m.add_method("pulse", |_, s, (name, t, tau): (String, f32, f32)| Ok(pulse(s.hits(&name)?, t, tau)));
        // List of hits as { t = …, s = … } tables.
        m.add_method("hits", |lua, s, name: String| {
            let h = s.hits(&name)?;
            let out = lua.create_table_with_capacity(h.len(), 0)?;
            for x in h {
                let e = lua.create_table_with_capacity(0, 2)?;
                e.raw_set("t", x.t)?;
                e.raw_set("s", x.s)?;
                out.raw_push(e)?;
            }
            Ok(out)
        });
        m.add_method("count", |_, s, name: String| Ok(s.hits(&name)?.len()));
        m.add_method("downbeats", |lua, s, ()| lua.create_sequence_from(s.0.downbeats.iter().copied()));
        m.add_method("beats", |lua, s, ()| lua.create_sequence_from(s.0.beats.iter().copied()));
        // Number of downbeats at or before t (the bar index).
        m.add_method("bar_index", |_, s, t: f32| Ok(s.0.downbeats.partition_point(|&d| d <= t)));
        m.add_method("phases", |_, s, t: f32| Ok(s.0.phases(t)));
        // kind, start, end, index (1-based) of the section at t.
        m.add_method("section", |_, s, t: f32| {
            let a = &s.0;
            match a.sections.iter().position(|x| t >= x.start && t < x.end).or(a.sections.len().checked_sub(1)) {
                Some(i) => {
                    let x = a.sections[i];
                    Ok((Some(x.kind.name()), x.start, x.end, i + 1))
                }
                None => Ok((None, 0.0, 0.0, 0)),
            }
        });
        m.add_method("sections", |lua, s, ()| {
            let out = lua.create_table()?;
            for x in &s.0.sections {
                let e = lua.create_table()?;
                e.raw_set("kind", x.kind.name())?;
                e.raw_set("start", x.start)?;
                e.raw_set("stop", x.end)?;
                out.raw_push(e)?;
            }
            Ok(out)
        });
    }
}

pub struct Score {
    lua: Lua,
    step_fn: Option<Function>,
    frame_fn: Option<Function>,
    deadline: Arc<Mutex<Instant>>,
}

/// Time a script may take per call; build gets more.
const STEP_LIMIT: Duration = Duration::from_millis(50);
const BUILD_LIMIT: Duration = Duration::from_millis(3000);

fn sandbox() -> mlua::Result<(Lua, Arc<Mutex<Instant>>)> {
    let lua = Lua::new_with(StdLib::MATH | StdLib::STRING | StdLib::TABLE | StdLib::UTF8, LuaOptions::default())?;
    lua.set_memory_limit(128 << 20)?;
    let g = lua.globals();
    for name in ["dofile", "loadfile", "load", "require", "collectgarbage", "rawequal"] {
        g.raw_set(name, Value::Nil)?;
    }
    g.raw_set(
        "print",
        lua.create_function(|_, args: mlua::Variadic<Value>| {
            let s: Vec<String> = args.iter().map(|v| v.to_string().unwrap_or_default()).collect();
            log::info!("plugin: {}", s.join(" "));
            Ok(())
        })?,
    )?;
    let v = lua.create_table()?;
    v.raw_set("rng", lua.create_function(|_, seed: u32| Ok(Rng::new(seed)))?)?;
    v.raw_set("hash2", lua.create_function(|_, (a, b): (i64, i64)| Ok(hash2(a as u32, b as u32)))?)?;
    g.raw_set("velouna", v)?;
    let deadline = Arc::new(Mutex::new(Instant::now() + BUILD_LIMIT));
    let d = deadline.clone();
    lua.set_hook(mlua::HookTriggers::new().every_nth_instruction(20_000), move |_, _| {
        if Instant::now() > *d.lock().unwrap() {
            Err(mlua::Error::runtime("took too long"))
        } else {
            Ok(mlua::VmState::Continue)
        }
    });
    Ok((lua, deadline))
}

pub struct BuildCtx<'a> {
    pub seed: u32,
    pub aw: f32,
    pub ah: f32,
    pub params: &'a [(String, f32)],
    pub react: &'a [(&'static str, f32)],
}

impl Score {
    /// Loads `source` (named `chunk` in errors) and runs its `build`.
    pub fn new(source: &str, chunk: &str, song: Arc<Analysis>, ctx: &BuildCtx) -> Result<Score, String> {
        let r = (|| -> mlua::Result<Score> {
            let (lua, deadline) = sandbox()?;
            lua.load(source).set_name(format!("@{chunk}")).set_mode(mlua::ChunkMode::Text).exec()?;
            let g = lua.globals();
            let build: Option<Function> = g.get("build")?;
            let c = lua.create_table()?;
            c.raw_set("seed", ctx.seed)?;
            c.raw_set("aw", ctx.aw)?;
            c.raw_set("ah", ctx.ah)?;
            let p = lua.create_table()?;
            for (k, v) in ctx.params {
                p.raw_set(k.as_str(), *v)?;
            }
            c.raw_set("params", p)?;
            let r = lua.create_table()?;
            for (k, v) in ctx.react {
                r.raw_set(*k, *v)?;
            }
            c.raw_set("react", r)?;
            if let Some(b) = build {
                *deadline.lock().unwrap() = Instant::now() + BUILD_LIMIT;
                b.call::<()>((Song(song), c))?;
            }
            let step_fn: Option<Function> = g.get("step")?;
            let frame_fn: Option<Function> = g.get("frame")?;
            Ok(Score { lua, step_fn, frame_fn, deadline })
        })();
        r.map_err(|e| e.to_string())
    }

    pub fn has_steps(&self) -> bool {
        self.step_fn.is_some()
    }

    /// Operations of one step, appended to `out` (12 floats each, at most
    /// `cap` operations), and the step's decay factor.
    pub fn step(&self, t0: f32, dt: f32, first: bool, out: &mut Vec<[f32; 12]>, cap: usize) -> Result<f32, String> {
        let Some(f) = &self.step_fn else { return Ok(1.0) };
        *self.deadline.lock().unwrap() = Instant::now() + STEP_LIMIT;
        let (ops, decay): (Option<Table>, Option<f32>) = f.call((t0, dt, first)).map_err(|e| e.to_string())?;
        if let Some(ops) = ops {
            let n = ops.raw_len() / 12;
            let start = out.len();
            for i in 0..n.min(cap) {
                let mut op = [0f32; 12];
                for (k, v) in op.iter_mut().enumerate() {
                    *v = ops.raw_get::<f32>(i * 12 + k + 1).map_err(|e| e.to_string())?;
                }
                out.push(op);
            }
            debug_assert!(out.len() - start <= cap);
        }
        Ok(decay.unwrap_or(1.0))
    }

    /// Ripples (x, y, age, strength) for this frame, if the script draws any.
    pub fn frame(&self, t: f32, rip: &mut Vec<[f32; 4]>) -> Result<(), String> {
        let Some(f) = &self.frame_fn else { return Ok(()) };
        *self.deadline.lock().unwrap() = Instant::now() + STEP_LIMIT;
        let r: Option<Table> = f.call(t).map_err(|e| e.to_string())?;
        if let Some(list) = r.and_then(|r| r.raw_get::<Option<Table>>("rip").ok().flatten()) {
            for e in list.sequence_values::<Table>().take(16) {
                let e = e.map_err(|e| e.to_string())?;
                let g = |i: usize| e.raw_get::<f32>(i).unwrap_or(0.0);
                rip.push([g(1), g(2), g(3), g(4)]);
            }
        }
        let _ = &self.lua;
        Ok(())
    }
}
