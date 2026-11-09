//! Visual check and benchmark of the lighting on real maps: a small base among each map's own
//! features, seen from TA's default pitch, a low pitch, far out, towards the horizon, over the
//! sea and in the classic projection.
//!
//! Loads maps from the installed game, places buildings in a row (so there are gaps between
//! them), tanks and kbots, a burning wreck with smoke, an explosion and an aircraft overhead on
//! the flattest dry ground near the map centre, freezes the animation clock and saves one PNG
//! per view (`<out>/<map>_<view><tag>.png`):
//!
//! ```text
//! cargo run -p ta_render --example showcase --release -- \
//!     [--maps "Coast To Coast,Sherwood"] [--out scratch/render2] [--tag _after] \
//!     [--frames 3] [--time 20] [--only ta,low] [--size 1600x900] [--site x,z] \
//!     [--particles [--sequence] [--step 0.0167]] \
//!     [--set key=value]... [--bench [--rounds N] [--vsync] [--config "label:key=value,key=value"]...]
//! ```
//!
//! `--particles` sets off GPU particles at the start of every view, with the game's own recipes:
//! a unit blowing up at the explosion (sparks, flames, embers, burning chunks, grit, dust),
//! embers and flames over the burning wreck, in the `sea` view a shell bursting on the water
//! (spray and mist) and in the `nuke` view the nuclear blast's storm; each frame advances them by
//! `--step` seconds, so `--frames` picks the moment captured. `--sequence` saves every frame of
//! each view (`..._NNN.png`), for an animation. `--set particles=N` (or in a `--config`) keeps
//! about N particles alive around the base instead (a steady mix of sparks, embers, flames, dust
//! and grit; `mix=sparks|embers|fire|dust|dirt` emits one kind only), to measure what they cost:
//! compare the `frame` column, and see `particles` for the compute pass alone.
//!
//! Views: `ta` (TA's 63.4° pitch), `close` (45°, zoomed in), `low` (24°), `wide` (far out),
//! `horizon` (7°, towards the sun), `sea` (over the nearest sea or lava, if any), `blast`
//! (close to the explosion: heat haze), `nuke` (only with `--particles`: further out, over a
//! nuclear blast), `classic` and `classic_wide` (TA's projection, which must render exactly as it
//! always did without particles). On maps with a sea of water, `ships` (a small fleet off the
//! coast, 30°) and `mirror` (15°, across the water towards the coast) show the water's
//! reflections; the fleet is only in those two views. The scene is identical from run to run, so
//! images of two builds can be compared pixel by pixel.
//!
//! `--set` changes a setting for every view (see `apply_setting` for the keys, e.g. `ao=0`,
//! `ao_contact=0`, `cloud_shadows=0`, `heat=0`, `sky_light=0`, `soft_smoke=0`,
//! `reflections=off|half|full`, `reflectivity=0.35`, `pool=32768` and `density=0.35` for the GPU
//! particles, or `effects=0` for all of the perspective camera's lighting effects, which then
//! renders exactly as before they existed). `--dump-ao` also saves each map's baked terrain
//! occlusion as a PNG.
//!
//! `--bench` disables vsync (unless `--vsync`) and prints, for every perspective view under
//! each `--config` (a label and settings applied on top of `--set`; the first one is the
//! reference the others are compared with), the frame time and the GPU time of each pass
//! (timestamp queries). The configurations take turns for `--rounds` rounds (default 5): frame
//! times are the median of the rounds; GPU times the median frame of the best round, since a run
//! sometimes settles into waiting ~0.3 ms on the MSAA resolve or the swapchain inside the
//! measured window. Compare the `frame` column: all rendering up to the end of bloom.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use glam::{Mat4, Vec2, Vec3};
use ta_formats::gaf::Gaf;
use ta_formats::image::Palette;
use ta_formats::model3do::{FIXED_ONE, Model3do};
use ta_formats::tnt::TntMap;
use ta_render::terrain::HeightField;
use ta_render::{Camera, CameraMode, DrawInstance, ModelId, ParticleBurst, ParticleKind, PointLight, Renderer, SpriteId, SpriteInstance, WaterReflections};
use ta_vfs::Vfs;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

struct Options {
    maps: Vec<String>,
    out: PathBuf,
    tag: String,
    frames: u32,
    time: f32,
    only: Option<Vec<String>>,
    size: (u32, u32),
    site: Option<Vec2>,
    sets: Vec<(String, String)>,
    bench: bool,
    /// Benchmark rounds (each configuration once per round).
    rounds: u32,
    /// Keep vsync on while benchmarking (GPU times only; the GPU then never waits on presents).
    vsync: bool,
    /// Also save each map's baked terrain occlusion as a greyscale PNG and print its range.
    dump_ao: bool,
    /// Benchmark configurations: label and settings.
    configs: Vec<(String, Vec<(String, String)>)>,
    /// Set off GPU particles at the start of every view.
    particles: bool,
    /// Save every frame of each view.
    sequence: bool,
    /// Seconds the particles advance per frame.
    step: f32,
}

fn parse_pairs(s: &str) -> Result<Vec<(String, String)>> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| p.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).with_context(|| format!("expected key=value, got {p}")))
        .collect()
}

impl Options {
    fn parse() -> Result<Options> {
        let mut o = Options {
            maps: vec!["Coast To Coast".into()],
            out: PathBuf::from("scratch/render2"),
            tag: String::new(),
            frames: 3,
            time: 20.0,
            only: None,
            size: (1600, 900),
            site: None,
            sets: Vec::new(),
            bench: false,
            rounds: 5,
            vsync: false,
            dump_ao: false,
            configs: Vec::new(),
            particles: false,
            sequence: false,
            step: 1.0 / 60.0,
        };
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            let mut value = || args.next().with_context(|| format!("{a} needs a value"));
            match a.as_str() {
                "--maps" | "--map" => o.maps = value()?.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                "--out" => o.out = PathBuf::from(value()?),
                "--tag" => o.tag = value()?,
                "--frames" => o.frames = value()?.parse::<u32>().context("--frames")?.max(1),
                "--time" => o.time = value()?.parse().context("--time")?,
                "--only" => o.only = Some(value()?.split(',').map(|s| s.trim().to_string()).collect()),
                "--site" => {
                    let v = value()?;
                    let (x, z) = v.split_once(',').context("--site x,z")?;
                    o.site = Some(Vec2::new(x.trim().parse()?, z.trim().parse()?));
                }
                "--set" => o.sets.extend(parse_pairs(&value()?)?),
                "--bench" => o.bench = true,
                "--dump-ao" => o.dump_ao = true,
                "--particles" => o.particles = true,
                "--sequence" => o.sequence = true,
                "--step" => o.step = value()?.parse().context("--step")?,
                "--vsync" => o.vsync = true,
                "--rounds" => o.rounds = value()?.parse::<u32>().context("--rounds")?.max(1),
                "--config" => {
                    let v = value()?;
                    let (label, pairs) = v.split_once(':').unwrap_or((&v, ""));
                    o.configs.push((label.to_string(), parse_pairs(pairs)?));
                }
                "--size" => {
                    let v = value()?;
                    let (w, h) = v.split_once('x').context("--size WxH")?;
                    o.size = (w.parse()?, h.parse()?);
                }
                other => anyhow::bail!("unknown argument {other}"),
            }
        }
        if o.configs.is_empty() {
            o.configs.push(("default".into(), Vec::new()));
        }
        Ok(o)
    }
}

/// Applies one `--set` / `--config` setting.
fn apply_setting(r: &mut Renderer, key: &str, value: &str) -> Result<()> {
    let number = || value.parse::<f32>().with_context(|| format!("{key}={value}: expected a number"));
    let on = matches!(value, "1" | "on" | "true" | "yes");
    match key {
        "sky" => r.lighting.sky.enabled = on,
        "clouds" => r.lighting.sky.cloud_cover = number()?,
        "shadows" => r.lighting.shadow_strength = number()?,
        "bloom" => r.post.settings.bloom = number()?,
        "water" => r.lighting.water.enabled = on,
        "ao" => r.lighting.ambient_occlusion.enabled = on,
        "ao_terrain" => r.lighting.ambient_occlusion.terrain = number()?,
        "ao_contact" => r.lighting.ambient_occlusion.contact = number()?,
        "ao_models" => r.lighting.ambient_occlusion.models = number()?,
        "cloud_shadows" => r.lighting.cloud_shadows = number()?,
        "heat" => r.post.settings.heat_haze = number()?,
        "sky_light" => r.lighting.sky_light = number()?,
        "soft_smoke" => r.lighting.soft_smoke = number()?,
        "reflections" => {
            r.lighting.water_reflections = match value {
                "off" | "0" => WaterReflections::Off,
                "half" | "1" | "on" => WaterReflections::Half,
                "full" | "2" => WaterReflections::Full,
                other => anyhow::bail!("reflections={other}: expected off, half or full"),
            }
        }
        "reflectivity" => r.lighting.water.reflectivity = number()?,
        // GPU particles: pool size and density (the game's effects quality: Low = 32768, 0.35).
        "pool" => r.set_particle_settings(ta_render::ParticleSettings { capacity: number()? as u32, ..r.particle_settings() }),
        "density" => r.set_particle_settings(ta_render::ParticleSettings { density: number()?, ..r.particle_settings() }),
        // Every lighting effect of the perspective camera on or off at once.
        "effects" => {
            let defaults = (ta_render::Lighting::default(), ta_render::post::PostSettings::default());
            let k = if on { 1.0 } else { 0.0 };
            r.lighting.ambient_occlusion.enabled = on;
            r.lighting.cloud_shadows = defaults.0.cloud_shadows * k;
            r.lighting.sky_light = defaults.0.sky_light * k;
            r.lighting.soft_smoke = defaults.0.soft_smoke * k;
            r.lighting.water_reflections = if on { defaults.0.water_reflections } else { WaterReflections::Off };
            r.post.settings.heat_haze = defaults.1.heat_haze * k;
        }
        other => anyhow::bail!("unknown setting {other}"),
    }
    Ok(())
}

/// One scripted view.
#[derive(Clone, Debug)]
struct Shot {
    name: &'static str,
    mode: CameraMode,
    /// Ground focus (x, z).
    focus: Vec2,
    zoom: f32,
    yaw_deg: f32,
    pitch_deg: f32,
    /// The fleet is on the water.
    ships: bool,
}

fn shot(name: &'static str, mode: CameraMode, focus: Vec2, zoom: f32, yaw_deg: f32, pitch_deg: f32) -> Shot {
    Shot { name, mode, focus, zoom, yaw_deg, pitch_deg, ships: false }
}

/// Where the explosion is, relative to the site.
const BLAST: Vec2 = Vec2::new(-190.0, 150.0);

/// The views of one map: around the base at several pitches, over the sea (and with `fleet`, the
/// fleet and its reflections), and classic.
fn plan(site: Vec2, sea: Option<Vec2>, fleet: bool, particles: bool) -> Vec<Shot> {
    use CameraMode::{Classic, Perspective};
    let mut shots = vec![
        shot("ta", Perspective, site, 1.6, 0.0, 63.4),
        shot("close", Perspective, site + Vec2::new(-40.0, 30.0), 3.2, -20.0, 45.0),
        shot("low", Perspective, site + Vec2::new(10.0, 40.0), 1.6, -35.0, 24.0),
        shot("wide", Perspective, site, 0.45, 0.0, 50.0),
        // Towards the north-west, where the default sun is: clouds and their shadows.
        shot("horizon", Perspective, site + Vec2::new(0.0, 60.0), 0.8, -40.0, 7.0),
    ];
    if let Some(sea) = sea {
        shots.push(shot("sea", Perspective, sea, 1.0, -20.0, 35.0));
        if fleet {
            // Out at sea, looking back towards the coast past the fleet.
            let to_sea = (sea - site).normalize_or(Vec2::X);
            let yaw = (-to_sea.x).atan2(to_sea.y).to_degrees();
            shots.push(Shot { ships: true, ..shot("ships", Perspective, sea - to_sea * 40.0, 1.9, yaw + 25.0, 30.0) });
            shots.push(Shot { ships: true, ..shot("mirror", Perspective, sea - to_sea * 90.0, 1.3, yaw - 10.0, 15.0) });
        }
    }
    // Close to the explosion (heat haze).
    shots.push(shot("blast", Perspective, site + BLAST + Vec2::new(0.0, 10.0), 3.0, -15.0, 38.0));
    if particles {
        // Further out over the explosion, for the nuclear blast.
        shots.push(shot("nuke", Perspective, site + BLAST + Vec2::new(0.0, -60.0), 0.55, -15.0, 32.0));
    }
    shots.push(shot("classic", Classic, site, 1.6, 0.0, 63.4));
    shots.push(shot("classic_wide", Classic, site, 0.6, 0.0, 63.4));
    shots
}

/// A feature type's definition, as far as drawing it needs.
struct FeatureDef {
    filename: String,
    seq: String,
    seq_shadow: String,
    object: String,
    height: f32,
    metal_deposit: bool,
    translucent: bool,
    footprint: (u32, u32),
}

/// How one feature type is drawn (mirrors the game's `features.rs`).
#[derive(Default)]
struct FeatureVisual {
    frames: Vec<SpriteId>,
    shadow: Option<SpriteId>,
    upright: bool,
    translucent: bool,
    model: Option<ModelId>,
}

fn load_feature_defs(vfs: &Vfs) -> HashMap<String, FeatureDef> {
    let mut defs = HashMap::new();
    for path in vfs.list("features", Some("tdf"), true) {
        let Some(bytes) = vfs.read_opt(&path) else { continue };
        for s in ta_formats::tdf::parse_bytes(&bytes).sections {
            let st = |k: &str| s.get(k).unwrap_or("").trim().to_string();
            let height = s.get_f32_or("height", 0.0);
            let metal = s.get_f32_or("metal", 0.0);
            let def = FeatureDef {
                filename: st("filename"),
                seq: st("seqname"),
                seq_shadow: st("seqnameshad"),
                object: st("object"),
                height,
                metal_deposit: st("category").eq_ignore_ascii_case("metal") || metal > 0.0 && s.get_bool_or("indestructible", false) && height <= 8.0,
                translucent: s.get_bool_or("animtrans", false),
                footprint: (s.get_i32_or("footprintx", 1).max(1) as u32, s.get_i32_or("footprintz", 1).max(1) as u32),
            };
            defs.insert(s.name.trim().to_ascii_lowercase(), def);
        }
    }
    defs
}

/// A model placed in a fixed pose.
struct Placed {
    model: ModelId,
    matrices: Vec<Mat4>,
    team: u32,
}

/// Everything drawn on one map.
struct MapScene {
    placed: Vec<Placed>,
    /// Ships off the coast (the `ships` and `mirror` views only).
    fleet: Vec<Placed>,
    decals: Vec<SpriteInstance>,
    billboards: Vec<SpriteInstance>,
    /// Burning wreck and explosion (world positions on the ground).
    fire: Vec3,
    blast: Vec3,
}

/// The flattest dry ground near the map centre (world xz).
fn find_site(field: &HeightField) -> Vec2 {
    let (w, h) = (field.width as i32, field.height as i32);
    let r = 12.min(w / 4).min(h / 4).max(1);
    let centre = Vec2::new(w as f32, h as f32) * 8.0;
    let mut best: Option<(f32, Vec2)> = None;
    for cz in (r..h - r).step_by(3) {
        for cx in (r..w - r).step_by(3) {
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for z in cz - r..=cz + r {
                for x in cx - r..=cx + r {
                    let v = field.vertex(x, z);
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            if lo < field.sea_level + 0.5 {
                continue;
            }
            let p = Vec2::new(cx as f32, cz as f32) * 16.0;
            let score = (hi - lo) * 20.0 + p.distance(centre) * 0.05;
            if best.is_none_or(|b| score < b.0) {
                best = Some((score, p));
            }
        }
    }
    best.map_or(centre, |b| b.1)
}

/// A point over open water (or lava) near `site`, if the map has a sea.
fn find_sea(field: &HeightField, site: Vec2) -> Option<Vec2> {
    if field.sea_level <= 0.0 {
        return None;
    }
    let mut best: Option<(f32, Vec2)> = None;
    for z in 0..field.height as i32 {
        for x in 0..field.width as i32 {
            if field.vertex(x, z) < field.sea_level - 10.0 {
                let p = Vec2::new(x as f32, z as f32) * 16.0;
                let d = p.distance(site);
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, p));
                }
            }
        }
    }
    let (_, p) = best?;
    // A little further out, so the view is over water rather than the beach.
    let out = (p - site).normalize_or(Vec2::X);
    let size = field.size_px();
    Some((p + out * 160.0).clamp(Vec2::splat(100.0), size - 100.0))
}

/// Model-space (z negated, as the renderer loads it) xz bounds of a 3DO in its rest pose.
fn model_bounds(model: &Model3do) -> (Vec2, Vec2) {
    let mut origin = Vec::with_capacity(model.pieces.len());
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for p in &model.pieces {
        let off = Vec3::new(p.offset[0] as f32, p.offset[1] as f32, -(p.offset[2] as f32)) / FIXED_ONE;
        let o = p.parent.map_or(off, |parent: usize| origin[parent] + off);
        origin.push(o);
        for v in &p.vertices {
            let q = o + Vec3::new(v[0] as f32, v[1] as f32, -(v[2] as f32)) / FIXED_ONE;
            lo = lo.min(Vec2::new(q.x, q.z));
            hi = hi.max(Vec2::new(q.x, q.z));
        }
    }
    if lo.x > hi.x { (Vec2::ZERO, Vec2::ZERO) } else { (lo, hi) }
}

struct State {
    window: Arc<Window>,
    renderer: Renderer,
    camera: Camera,
    palette: Palette,
    feature_defs: HashMap<String, FeatureDef>,
    feature_visuals: HashMap<String, FeatureVisual>,
    gaf_cache: HashMap<String, Option<Gaf>>,
    glow: Option<SpriteId>,
    puff: Option<SpriteId>,
    map_index: usize,
    scene: Option<MapScene>,
    shots: Vec<Shot>,
    current: usize,
    frame: u32,
    /// Lighting after `--set`, restored before every view and configuration.
    base: (ta_render::Lighting, ta_render::post::PostSettings),
    bench_config: usize,
    bench_round: u32,
    bench_start: Option<Instant>,
    /// (map, view, config, ms per frame).
    bench_results: Vec<(String, String, String, f64)>,
    /// GPU pass times of every timed frame of the current view and configuration.
    pass_samples: Vec<(&'static str, f64)>,
    /// (map, view, config, round, pass, GPU ms of one frame); pass "frame" = the whole GPU
    /// frame.
    pass_results: Vec<(String, String, String, u32, &'static str, f64)>,
    /// Particles kept alive by the steady emitter (`particles=N`; 0 = off), and its random
    /// state.
    steady: u32,
    /// `--particles` (the burning wreck keeps emitting).
    options_particles: bool,
    /// What the steady emitter emits (`mix=`): `all` (a battle's mix), or only `sparks`,
    /// `embers`, `dirt`, `fire` or `dust`.
    mix: String,
    rng: u32,
    /// Live particles measured during the current benchmark (sum, frames).
    live_samples: (u64, u32),
    /// (map, view, config, mean live particles).
    live_results: Vec<(String, String, String, f64)>,
}

struct App {
    options: Options,
    vfs: Arc<Vfs>,
    state: Option<State>,
    error: Option<anyhow::Error>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop, &self.options, &self.vfs) {
            Ok(s) => self.state = Some(s),
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.renderer.resize(size.width, size.height);
                state.camera.viewport = Vec2::new(size.width.max(1) as f32, size.height.max(1) as f32);
            }
            WindowEvent::RedrawRequested => match state.frame(&self.options, &self.vfs) {
                Ok(true) => {}
                Ok(false) => event_loop.exit(),
                Err(e) => {
                    self.error = Some(e);
                    event_loop.exit();
                }
            },
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(s) = &self.state {
            s.window.request_redraw();
        }
    }
}

/// The game's particle recipes, copied verbatim from `apps/ta_game/src/effects_particles.rs`, so
/// the views show exactly what the game does.
#[allow(dead_code)]
mod recipes {
    use std::f32::consts::PI;

    use glam::Vec3;
    use ta_render::{ParticleBurst as Burst, ParticleKind as Kind};

    const UP: [f32; 3] = [0.0, 1.0, 0.0];

    /// What set a blast off, which decides its look.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(super) enum Blast {
        /// A shell, rocket or bomb: sparks, flames, grit and dust.
        Shell,
        /// A laser, lightning or paralyzer hit: a spray of sparks in the beam's colour.
        Beam([f32; 3]),
        /// A dying unit or building: a bigger shower, burning chunks and embers.
        Death,
        /// The flame thrower: flames and embers.
        Flame,
    }

    /// What a blast went off in.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Ground {
        /// On (or above) dry ground.
        Land,
        Sea,
        Lava,
    }

    /// The bursts of a fireball `radius` wide at `p` that burns for about `life` seconds (the
    /// perspective camera's blast, in place of TA's painting of one): a ball of flame swelling and
    /// rising, and from a big one, smaller ones boiling out around it a moment later, so its outline
    /// breaks into lobes.
    pub(super) fn fireball_bursts(p: Vec3, radius: f32, life: f32) -> Vec<Burst> {
        let r = radius.max(3.0);
        let d = r * 2.0;
        // (A blast's fireball outlasts TA's painting of it a little: it cools into soot.)
        let life = (life * 1.15).clamp(0.4, 2.6);
        let mut out = vec![
            Burst::new(Kind::Fireball, [p.x, p.y + r * 0.2, p.z], 1)
                .size(d * 0.55, d * 1.15)
                .life(life * 0.92, life * 1.08)
                .toward(UP, 0.5)
                .speed(0.0, r * 0.15)
                .gravity(-r * 0.9)
                .drag(1.6),
        ];
        if r >= 16.0 {
            let n = ((r / 10.0) as u32).clamp(2, 6);
            out.push(
                Burst::new(Kind::Fireball, [p.x, p.y + r * 0.25, p.z], n)
                    .radius(r * 0.4)
                    .vertical(0.6)
                    .outward(0.9)
                    .speed(r * 0.5, r * 1.2)
                    .size(d * 0.3, d * 0.75)
                    .life(life * 0.6, life * 0.9)
                    .gravity(-r * 0.8)
                    .drag(3.0)
                    .intensity(0.95)
                    .over(0.06),
            );
        }
        out
    }

    /// The bursts of a blast `radius` wide (the weapon's area of effect) at `p`, `ground` being the
    /// height of the ground under it. `fireball`: a fireball burns at its heart (the perspective
    /// camera's, see `fireball_bursts`), so fewer flames lick out of it.
    pub(super) fn blast_bursts(p: Vec3, radius: f32, ground: f32, under: Ground, blast: Blast, fireball: bool) -> Vec<Burst> {
        let s = radius.max(4.0);
        let at = |y: f32| [p.x, y, p.z];
        let mut out = Vec::with_capacity(8);
        // On the ground or a unit standing on it, debris flies up and out and the ground throws up
        // grit and dust; in the air, sparks fly every way.
        let low = p.y - ground < 30.0;
        let (tint, shower) = match blast {
            Blast::Beam(c) => (c, 0.6),
            Blast::Death => ([1.0; 3], 1.7),
            Blast::Flame => ([1.0, 0.8, 0.6], 0.5),
            Blast::Shell => ([1.0; 3], 1.0),
        };
        // (Dozens rather than hundreds, and not white-hot for long: a battle's showers overlap, and
        // a dense one blooms into a white fog. Thin, and flung at speeds over a wide range, so they
        // streak out unevenly rather than in a neat ring.)
        let sparks = ((10.0 + s * 2.2) * shower).min(1400.0) as u32;
        let shower = Burst::new(Kind::Spark, at(p.y + 3.0), sparks)
            .radius(s * 0.12)
            .toward(UP, if low { 1.2 } else { PI })
            .speed(30.0 + s * 0.5, (160.0 + s * 3.2).min(650.0))
            .life(0.6, 1.6 + (s / 80.0).min(1.6))
            .size(1.8 + s * 0.008, 1.0)
            .intensity(if blast == Blast::Death { 3.2 } else { 2.8 })
            .color(tint);
        // Energy beams throw sparks of their own colour, not hot metal; the sea quenches hot metal.
        if under != Ground::Sea {
            out.push(if matches!(blast, Blast::Beam(_)) { shower.cold() } else { shower });
        }
        if let Blast::Beam(c) = blast {
            // A flash where the beam burns in; beams kick up little else.
            out.push(Burst::new(Kind::Glow, at(p.y + 2.0), 1).size(12.0, 20.0).life(0.12, 0.16).color(c).intensity(4.0));
            return out;
        }
        match under {
            Ground::Lava => {
                // Molten rock thrown up, glowing as it falls back, and the melt's glare. (Fewer, lower
                // and dimmer than they were: over a lava lake under fire they filled the view.)
                let n = (8.0 + s * 0.7).min(300.0) as u32;
                out.push(Burst::new(Kind::Ember, at(p.y + 1.0), n).radius(s * 0.2).toward(UP, 0.55).speed(40.0 + s * 0.6, 90.0 + s * 1.6).life(0.5, 1.1).size(2.4, 1.2).gravity(380.0).drag(0.4).intensity(4.5).speck());
                out.push(Burst::new(Kind::Glow, at(p.y + 2.0), 1).size(s * 0.6, s * 1.0).life(0.2, 0.3).color([1.0, 0.42, 0.1]).intensity(1.8));
                return out;
            }
            Ground::Sea => {
                // A column of spray thrown up and falling back, a lower crown of droplets round its
                // foot, and mist drifting off the water.
                let n = (14.0 + s * 1.8).min(700.0) as u32;
                out.push(
                    Burst::new(Kind::Spray, at(p.y + 1.0), n)
                        .radius(s * 0.12)
                        .toward(UP, 0.2)
                        .speed(60.0 + s * 1.2, 150.0 + s * 3.4)
                        .life(0.7, 1.2 + s / 120.0)
                        .size(2.2 + s * 0.02, 1.6 + s * 0.015)
                        .intensity(0.75),
                );
                let crown = (10.0 + s * 1.0).min(400.0) as u32;
                out.push(Burst::new(Kind::Spray, at(p.y + 1.0), crown).radius(s * 0.25).toward(UP, 0.95).speed(25.0 + s * 0.5, 70.0 + s * 1.5).life(0.4, 0.9).size(1.8 + s * 0.015, 1.2).intensity(0.6));
                let mist = (6.0 + s * 0.3).min(90.0) as u32;
                out.push(
                    Burst::new(Kind::Dust, at(p.y + 2.0), mist)
                        .color([0.9, 0.95, 1.0])
                        .intensity(0.4)
                        .radius(s * 0.25)
                        .vertical(0.4)
                        .speed(s * 0.3, s * 0.9)
                        .life(1.0, 2.4)
                        .size(s * 0.5, s * 1.4)
                        .after(0.12),
                );
                return out;
            }
            Ground::Land => {}
        }
        if s >= 12.0 {
            // Flames boiling out of the blast and rising.
            let n = ((s * if fireball { 0.15 } else { 0.3 }).min(90.0) * if blast == Blast::Flame { 2.0 } else { 1.0 }) as u32;
            let size = s.min(120.0);
            out.push(Burst::new(Kind::Fire, at(p.y + s * 0.15), n).radius(s * 0.3).outward(0.9).speed(s * 0.3, s).life(0.35, 0.7 + s / 300.0).size(size * 0.35, size * 0.7).gravity(-70.0).drag(3.0).intensity(1.2));
        }
        if s >= 20.0 || blast != Blast::Shell {
            // Embers drifting up from the blast.
            let n = ((s * 0.45).min(220.0) * if blast == Blast::Death { 1.8 } else { 1.0 }) as u32;
            out.push(Burst::new(Kind::Ember, at(p.y + 4.0), n).radius(s * 0.3).toward(UP, 1.1).speed(20.0, 40.0 + s * 0.9).life(1.2, 2.5 + s / 80.0).size(3.0, 1.2).intensity(6.0).over(0.3));
        }
        if blast == Blast::Death || s >= 30.0 {
            // Burning chunks flung out in arcs, glowing on where they land.
            let n = ((s * 0.25).min(90.0) * if blast == Blast::Death { 1.5 } else { 1.0 }) as u32;
            out.push(Burst::new(Kind::Ember, at(p.y + 4.0), n).radius(s * 0.15).toward(UP, 1.0).speed(60.0 + s, (150.0 + s * 2.5).min(560.0)).life(1.6, 3.2).size(4.6, 2.6).gravity(300.0).drag(0.3).intensity(5.5));
        }
        if low && s >= 8.0 {
            let g = [p.x, ground + 1.0, p.z];
            // Grit and clods thrown up, falling back and bouncing.
            let grit = (10.0 + s * 1.2).min(500.0) as u32;
            out.push(
                Burst::new(Kind::Dirt, g, grit)
                    .ground_color()
                    .radius(s * 0.2)
                    .toward(UP, 0.75)
                    .speed(40.0 + s, (100.0 + s * 2.8).min(520.0))
                    .life(0.7, 1.4 + s / 150.0)
                    .size(2.6 + s * 0.016, 2.3 + s * 0.016),
            );
            // Dust rolling out over the ground, in the ground's own colour (thin: on pale sand a
            // thick one washes a whole battle out).
            let dust = (4.0 + s * 0.28).min(80.0) as u32;
            let size = s.min(140.0);
            out.push(
                Burst::new(Kind::Dust, [p.x, ground + 2.0, p.z], dust)
                    .ground_color()
                    .radius(s * 0.25)
                    .vertical(0.15)
                    .outward(0.35)
                    .speed(s * 0.5, s * 1.5)
                    .life(1.2, 2.4 + s / 120.0)
                    .size(size * 0.4, size * 1.15)
                    .intensity(0.55),
            );
        }
        if s >= 10.0 && blast != Blast::Flame {
            // Smoke: a dark plume boiling up out of the fireball and drifting off, blacker where
            // a unit or building burned.
            let n = ((3.0 + s * 0.22) * if blast == Blast::Death { 1.6 } else { 1.0 }).min(70.0) as u32;
            let size = s.min(130.0);
            let color = if blast == Blast::Death { [0.12, 0.11, 0.1] } else { [0.24, 0.225, 0.21] };
            out.push(
                Burst::new(Kind::Smoke, at(p.y + s * 0.2), n)
                    .radius(s * 0.25)
                    .vertical(0.8)
                    .toward(UP, 0.5)
                    .speed(8.0, 20.0 + s * 0.35)
                    .life(2.2, 3.8 + s / 60.0)
                    .size(size * 0.3, size * 1.0)
                    .color(color)
                    .intensity(0.7)
                    .gravity(-18.0)
                    .drag(0.8)
                    .over(0.35)
                    .after(0.08),
            );
        }
        out
    }

    /// The storm of a nuclear blast `radius` wide at `p` (`ground` = height of the ground under it),
    /// the biggest show there is: white-hot sparks flung out from the fireball in every direction,
    /// raining back over the whole blast; the fireball boiling up into a rising column of flame and
    /// then of smoke; burning chunks arcing out; tens of thousands of embers climbing into the cloud
    /// and later drifting down from it; a wall of dust racing out along the ground with the
    /// shockwave; and grit thrown high.
    pub(super) fn nuke_bursts(p: Vec3, radius: f32, ground: f32) -> Vec<Burst> {
        let r = radius.clamp(200.0, 480.0);
        let at = |y: f32| [p.x, y, p.z];
        let g = [p.x, ground + 2.0, p.z];
        vec![
            // Sparks from all over the fireball (not its centre, which would burn white).
            Burst::new(Kind::Spark, at(p.y + r * 0.2), 14_000).radius(r * 0.25).vertical(0.7).outward(0.6).speed(150.0, r * 2.6).life(1.2, 3.5).size(3.6, 1.8).intensity(4.5).drag(0.45).gravity(300.0).over(0.3),
            // The fireball, and the column of flame it becomes.
            Burst::new(Kind::Fire, at(p.y + r * 0.15), 300).radius(r * 0.3).vertical(0.6).outward(1.0).speed(20.0, r * 0.6).life(1.0, 2.4).size(r * 0.16, r * 0.4).gravity(-100.0).drag(1.5).intensity(0.8).over(0.5),
            Burst::new(Kind::Fire, at(p.y + r * 0.4), 300).radius(r * 0.18).vertical(2.0).toward(UP, 0.25).speed(60.0, 180.0).life(1.2, 2.6).size(r * 0.08, r * 0.22).gravity(-60.0).drag(0.9).intensity(0.9).over(2.5).after(0.3),
            // Smoke boiling up the column and spreading into the cap.
            Burst::new(Kind::Smoke, at(p.y + r * 0.5), 700).radius(r * 0.2).vertical(2.5).toward(UP, 0.3).speed(40.0, 120.0).life(5.0, 9.0).size(r * 0.15, r * 0.5).color([0.2, 0.19, 0.18]).intensity(0.6).gravity(-30.0).drag(0.6).over(3.0).after(0.5),
            // Burning chunks flung out in arcs.
            Burst::new(Kind::Ember, at(p.y + r * 0.1), 500).radius(r * 0.2).toward(UP, 1.2).speed(150.0, 450.0).life(2.0, 4.0).size(5.5, 3.0).gravity(280.0).drag(0.25).intensity(6.0),
            // Embers climbing into the cloud, then drifting down from its cap.
            Burst::new(Kind::Ember, at(p.y + r * 0.3), 10_000).radius(r * 0.7).vertical(0.8).toward(UP, 1.1).speed(30.0, 240.0).life(3.0, 9.0).size(4.0, 1.6).gravity(-14.0).drag(0.7).intensity(3.5).over(3.0),
            Burst::new(Kind::Ember, at(p.y + r * 0.95), 6000).radius(r * 0.7).vertical(0.35).toward([0.0, -1.0, 0.0], PI * 0.6).speed(0.0, 30.0).life(4.0, 8.0).size(2.8, 1.2).gravity(10.0).drag(0.5).intensity(4.5).over(4.0).after(1.5),
            // The dust wall and the grit.
            Burst::new(Kind::Dust, g, 1200).ground_color().radius(r * 0.2).vertical(0.05).outward(0.12).speed(r * 1.1, r * 2.3).life(2.5, 5.0).size(r * 0.1, r * 0.35).gravity(-3.0).drag(1.0).intensity(0.8).over(0.25),
            Burst::new(Kind::Dirt, g, 5000).ground_color().radius(r * 0.3).toward(UP, 0.9).speed(150.0, 520.0).life(1.6, 3.6).size(5.0, 4.0).drag(0.25).gravity(380.0),
        ]
    }
}

/// A soft round sprite (`power` shapes the falloff), white.
fn radial_sprite(renderer: &mut Renderer, n: u32, power: f32) -> Option<SpriteId> {
    let mut rgba = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(Vec2::splat(n as f32 * 0.5)) / (n as f32 * 0.5);
            let a = (1.0 - d).clamp(0.0, 1.0).powf(power);
            rgba.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    renderer.sprites.atlas.add_rgba(n, n, &rgba, n as f32 * 0.5, n as f32 * 0.5)
}

impl State {
    fn new(event_loop: &ActiveEventLoop, o: &Options, vfs: &Vfs) -> Result<State> {
        let window = Arc::new(event_loop.create_window(
            Window::default_attributes()
                .with_title("ta_render: showcase")
                .with_inner_size(PhysicalSize::new(o.size.0, o.size.1))
                .with_resizable(false),
        )?);
        let size = window.inner_size();
        let mut renderer = Renderer::new(window.clone(), size.width, size.height)?;
        renderer.set_animation_time(Some(o.time));
        if o.bench {
            renderer.gpu.set_vsync(o.vsync);
            if !renderer.set_gpu_timing(true) {
                log::warn!("no timestamp queries: GPU pass times unavailable");
            }
        }
        let palette = Palette::parse(&vfs.read("palettes/palette.pal")?)?;
        let gafs: Vec<Gaf> = vfs
            .list("textures", Some("gaf"), false)
            .iter()
            .filter_map(|p| vfs.read_opt(p).and_then(|d| Gaf::parse(&d).ok()))
            .collect();
        renderer.init_models(&gafs, &palette);
        let mut steady = 0;
        for (k, v) in &o.sets {
            if k == "particles" {
                steady = v.parse().context("particles=N")?;
            } else {
                apply_setting(&mut renderer, k, v)?;
            }
        }
        let glow = radial_sprite(&mut renderer, 64, 2.2);
        let puff = radial_sprite(&mut renderer, 64, 1.3);
        let camera = Camera::new(Vec2::new(size.width as f32, size.height as f32));
        let base = (renderer.lighting.clone(), renderer.post.settings);
        let mut state = State {
            window,
            renderer,
            camera,
            palette,
            feature_defs: load_feature_defs(vfs),
            feature_visuals: HashMap::new(),
            gaf_cache: HashMap::new(),
            glow,
            puff,
            map_index: 0,
            scene: None,
            shots: Vec::new(),
            current: 0,
            frame: 0,
            base,
            bench_config: 0,
            bench_round: 0,
            bench_start: None,
            bench_results: Vec::new(),
            pass_samples: Vec::new(),
            pass_results: Vec::new(),
            steady,
            options_particles: o.particles,
            mix: "all".to_string(),
            rng: 0x2545_F491,
            live_samples: (0, 0),
            live_results: Vec::new(),
        };
        state.load_map(o, vfs, 0)?;
        Ok(state)
    }

    /// Loads (once) and returns the visuals of a feature type.
    fn feature_visual(&mut self, vfs: &Vfs, name: &str) -> Option<&FeatureVisual> {
        let key = name.to_ascii_lowercase();
        if !self.feature_visuals.contains_key(&key) {
            let def = self.feature_defs.get(&key)?;
            let mut v = FeatureVisual {
                upright: def.height > 4.0 && !def.metal_deposit,
                translucent: def.translucent,
                ..Default::default()
            };
            if !def.object.is_empty() {
                if let Some(models) = self.renderer.models.as_mut() {
                    let name = def.object.to_ascii_lowercase();
                    v.model = vfs
                        .read_opt(&format!("objects3d/{name}.3do"))
                        .and_then(|b| Model3do::parse(&b).ok())
                        .map(|m| models.library.add(&name, &m, &models.textures, &self.palette));
                }
            } else if !def.filename.is_empty() {
                let file = def.filename.to_ascii_lowercase();
                let gaf = self
                    .gaf_cache
                    .entry(file.clone())
                    .or_insert_with(|| vfs.read_opt(&format!("anims/{file}.gaf")).and_then(|b| Gaf::parse(&b).ok()));
                if let Some(gaf) = gaf {
                    if let Some(entry) = gaf.entry(&def.seq) {
                        for f in &entry.frames {
                            v.frames.extend(self.renderer.sprites.atlas.add_gaf(f, &self.palette, false));
                        }
                    }
                    if !def.seq_shadow.is_empty() && let Some(entry) = gaf.entry(&def.seq_shadow) {
                        v.shadow = entry.frames.first().and_then(|f| self.renderer.sprites.atlas.add_gaf(f, &self.palette, true));
                    }
                }
            }
            self.feature_visuals.insert(key.clone(), v);
        }
        self.feature_visuals.get(&key)
    }

    /// Places a model at `pos` (world) turned by `heading` (radians) in its rest pose.
    fn place(&mut self, vfs: &Vfs, name: &str, pos: Vec3, heading: f32, team: u32) -> Option<Placed> {
        let models = self.renderer.models.as_mut()?;
        let model = Model3do::parse(&vfs.read_opt(&format!("objects3d/{name}.3do"))?).ok()?;
        let id = models.library.add(name, &model, &models.textures, &self.palette);
        let info = models.library.info(id);
        let root = Mat4::from_translation(pos) * Mat4::from_rotation_y(heading);
        let matrices = info
            .rest_pose()
            .iter()
            .zip(&info.pieces)
            // Muzzle flashes and wakes are hidden unless a script shows them.
            .map(|(m, p)| {
                let n = p.name.to_ascii_lowercase();
                if n.starts_with("flare") || n.starts_with("wake") { Mat4::ZERO } else { root * *m }
            })
            .collect();
        Some(Placed { model: id, matrices, team })
    }

    fn load_map(&mut self, o: &Options, vfs: &Vfs, index: usize) -> Result<()> {
        let name = o.maps[index].clone();
        let map_path = format!("maps/{name}.tnt");
        let map = TntMap::parse(&vfs.read(&map_path).with_context(|| map_path.clone())?)?;
        log::info!("map {name}: {}x{} px, sea level {}", map.width_px(), map.height_px(), map.sea_level);
        self.renderer.load_map(&map, &self.palette);
        let field = self.renderer.terrain.as_ref().context("terrain")?.field.clone();
        if o.dump_ao {
            let ao = ta_render::occlusion::terrain_visibility(map.width, map.height, &map.heights);
            let mut sorted = ao.clone();
            sorted.sort_unstable();
            let pct = |q: f32| sorted[((sorted.len() - 1) as f32 * q) as usize];
            let open = ao.iter().filter(|&&v| v == 255).count() as f32 / ao.len().max(1) as f32;
            println!("{name}: terrain occlusion min {} p1 {} p10 {} p50 {} mean {:.1}, {:.0}% open sky", pct(0.0), pct(0.01), pct(0.1), pct(0.5), ao.iter().map(|&v| v as f32).sum::<f32>() / ao.len().max(1) as f32, open * 100.0);
            std::fs::create_dir_all(&o.out)?;
            let prefix = name.to_ascii_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "_");
            image::save_buffer(o.out.join(format!("{prefix}_terrain_ao.png")), &ao, map.width, map.height, image::ColorType::L8)?;
        }
        let site = o.site.unwrap_or_else(|| find_site(&field));
        let sea = find_sea(&field, site);
        log::info!("site {site}, sea {sea:?}");
        let ground = |p: Vec2| Vec3::new(p.x, field.sample(p.x, p.y), p.y);

        // The map's own features, as the game draws them.
        let mut decals = Vec::new();
        let mut billboards = Vec::new();
        let mut placed = Vec::new();
        let time = o.time;
        for (i, &f) in map.features.iter().enumerate() {
            let Some(fname) = map.feature_names.get(f as usize) else { continue };
            let Some(footprint) = self.feature_defs.get(&fname.to_ascii_lowercase()).map(|d| d.footprint) else { continue };
            let origin = ((i as u32 % map.width) as f32, (i as u32 / map.width) as f32);
            let x = origin.0 * 16.0 + footprint.0 as f32 * 8.0;
            let z = origin.1 * 16.0 + footprint.1 as f32 * 8.0;
            let p = ground(Vec2::new(x, z));
            let Some(v) = self.feature_visual(vfs, fname) else { continue };
            if let Some(shadow) = v.shadow {
                decals.push(SpriteInstance::new([p.x, p.y + 0.3, p.z], shadow).alpha(0.5));
            }
            if v.frames.is_empty() {
                if let Some(model) = v.model {
                    placed.push((model, p));
                }
                continue;
            }
            let frame = v.frames[((time * 10.0) as usize + origin.0 as usize) % v.frames.len()];
            let inst = SpriteInstance::new([p.x, p.y + 0.2, p.z], frame).alpha(if v.translucent { 0.6 } else { 1.0 });
            if v.upright {
                billboards.push(inst);
            } else {
                decals.push(inst);
            }
        }
        let mut placed: Vec<Placed> = placed
            .into_iter()
            .filter_map(|(model, p)| {
                let info = self.renderer.models.as_ref()?.library.info(model);
                let root = Mat4::from_translation(p);
                Some(Placed { model, matrices: info.rest_pose().into_iter().map(|m| root * m).collect(), team: 0 })
            })
            .collect();

        // A row of buildings north of the site, a few units apart.
        let buildings = ["armlab", "armvp", "armsolar", "armsolar", "armllt", "armrad", "cormex"];
        let mut row = Vec::new();
        for b in buildings {
            if let Some(m) = vfs.read_opt(&format!("objects3d/{b}.3do")).and_then(|d| Model3do::parse(&d).ok()) {
                row.push((b, model_bounds(&m)));
            }
        }
        let gap = 14.0;
        let total: f32 = row.iter().map(|(_, (lo, hi))| hi.x - lo.x + gap).sum::<f32>() - gap;
        let mut x = site.x - total * 0.5;
        for (b, (lo, hi)) in row {
            let pos = ground(Vec2::new(x - lo.x, site.y - 70.0 - hi.y));
            placed.extend(self.place(vfs, b, pos, 0.0, 0));
            x += hi.x - lo.x + gap;
        }
        // Tanks and kbots south of it, and an aircraft above them.
        let units: [(&str, f32, f32, f32, u32); 12] = [
            ("armcom", -150.0, 40.0, 0.3, 0),
            ("armstump", -95.0, 36.0, 0.6, 0),
            ("armstump", -55.0, 44.0, 0.5, 0),
            ("armpw", -20.0, 30.0, 0.2, 0),
            ("armpw", 5.0, 42.0, 0.1, 0),
            ("armflash", 40.0, 36.0, -0.4, 0),
            ("corcom", 20.0, 120.0, 3.0, 1),
            ("corak", -30.0, 118.0, 2.8, 1),
            ("corak", -55.0, 128.0, 3.3, 1),
            ("corraid", 70.0, 112.0, 3.4, 1),
            ("corthud", 105.0, 126.0, 2.9, 1),
            ("armham", -110.0, 96.0, 1.0, 0),
        ];
        for (name, dx, dz, heading, team) in units {
            placed.extend(self.place(vfs, name, ground(site + Vec2::new(dx, dz)), heading, team));
        }
        let fighter = ground(site + Vec2::new(-10.0, 80.0)) + Vec3::Y * 110.0;
        placed.extend(self.place(vfs, "armfig", fighter, 0.8, 0));
        let fire = ground(site + Vec2::new(150.0, 60.0));
        placed.extend(self.place(vfs, "armstump_dead", fire, 1.1, 0));
        let blast = ground(site + BLAST);
        // On seas of water, a fleet off the coast: a battleship, destroyers of both sides and a
        // patrol boat, each where the water is deep enough.
        let mut fleet = Vec::new();
        if let (Some(sea), Some(ta_render::Fluid::Water | ta_render::Fluid::Acid)) = (sea, self.renderer.fluid()) {
            let out = (sea - site).normalize_or(Vec2::X);
            let across = out.perp();
            let heading = across.x.atan2(across.y);
            let ships = [
                ("armbats", out * 30.0, heading, 0),
                ("armroy", -out * 55.0 + across * 110.0, heading + 2.6, 0),
                ("corroy", out * 10.0 - across * 125.0, heading - 0.5, 1),
                ("armpt", -out * 100.0 - across * 30.0, heading + 0.9, 0),
            ];
            for (name, d, heading, team) in ships {
                let p = sea + d;
                if field.sample(p.x, p.y) < field.sea_level - 6.0 {
                    fleet.extend(self.place(vfs, name, Vec3::new(p.x, field.sea_level, p.y), heading, team));
                }
            }
        }

        let has_fleet = !fleet.is_empty();
        self.scene = Some(MapScene { placed, fleet, decals, billboards, fire, blast });
        let mut shots = plan(site, sea, has_fleet, o.particles);
        if let Some(only) = &o.only {
            shots.retain(|s| only.iter().any(|n| n == s.name));
        }
        if o.bench {
            shots.retain(|s| s.mode == CameraMode::Perspective);
        }
        self.map_index = index;
        self.shots = shots;
        self.current = 0;
        self.frame = 0;
        Ok(())
    }

    fn setup_shot(&mut self) {
        let s = self.shots[self.current].clone();
        let field = &self.renderer.terrain.as_ref().expect("terrain").field;
        let cam = &mut self.camera;
        cam.mode = s.mode;
        cam.zoom = s.zoom;
        cam.yaw = s.yaw_deg.to_radians();
        cam.pitch = s.pitch_deg.to_radians();
        let ground = field.sample(s.focus.x, s.focus.y).max(field.sea_level);
        cam.focus = Vec3::new(s.focus.x, if s.mode == CameraMode::Classic { 0.0 } else { ground }, s.focus.y);
        log::info!("view {}: {:?}", s.name, s);
    }

    /// Submits this frame's scene: models, features, the fire and the explosion with their
    /// lights, glows and smoke. Everything is a function of the frozen clock `t`, and with
    /// `--particles` of the blast's age (s) and whether it is the nuclear one: its light and flash
    /// then fade out as the game's do.
    fn submit(&mut self, t: f32, blast: Option<(f32, bool)>, ships: bool) {
        let Some(scene) = &self.scene else { return };
        let flicker = |f: f32, phase: f32| 0.75 + 0.15 * (t * f + phase).sin() + 0.1 * (t * f * 2.7 + phase * 1.3).sin();
        let fire_light = scene.fire + Vec3::Y * 12.0;
        let blast_light = scene.blast + Vec3::Y * 25.0;
        let k = flicker(13.0, 2.0);
        let mut lights = vec![PointLight::new(fire_light.to_array(), [3.0 * k, 1.3 * k, 0.35 * k], 90.0)];
        let mut effects = Vec::new();
        if let Some(glow) = self.glow {
            effects.push(SpriteInstance::new(fire_light.to_array(), glow).scale(0.55).tint([1.0, 0.45, 0.12]).brightness(2.4));
        }
        match blast {
            None => {
                lights.push(PointLight::new(blast_light.to_array(), [4.2, 2.1, 0.7], 170.0));
                if let Some(glow) = self.glow {
                    effects.push(SpriteInstance::new(blast_light.to_array(), glow).scale(1.4).tint([1.0, 0.6, 0.25]).brightness(3.0));
                }
            }
            Some((age, nuke)) => {
                // As the game's effects.rs: timed lights fading with the square of the time left,
                // and a flash sprite growing and fading.
                let fade = |start: f32, life: f32| if age < start { 0.0 } else { (1.0 - ((age - start) / life).min(1.0)).powi(2) };
                let b = scene.blast;
                let (flash_size, flash_life, flash_brightness) = if nuke {
                    let r = 240.0;
                    lights.push(PointLight::new((b + Vec3::Y * 120.0).to_array(), [18.0, 14.0, 10.0].map(|c| c * fade(0.0, 1.2)), r * 3.5));
                    lights.push(PointLight::new((b + Vec3::Y * 160.0).to_array(), [6.0, 2.4, 0.6].map(|c| c * fade(0.3, 7.0)), r * 2.4));
                    (r * 1.1, 0.35, 7.0)
                } else {
                    lights.push(PointLight::new((b + Vec3::Y * 24.0).to_array(), [7.0, 3.4, 1.1].map(|c| c * fade(0.0, 1.0)), 360.0));
                    (220.0, 0.4, 5.0)
                };
                let f = age / flash_life;
                if let (Some(glow), true) = (self.glow, f < 1.0) {
                    effects.push(SpriteInstance::new((b + Vec3::Y * 10.0).to_array(), glow).scale(flash_size / 64.0 * (1.0 + 0.4 * f)).alpha(1.0 - f).tint([1.0, 0.7, 0.4]).brightness(flash_brightness));
                }
            }
        }
        let mut smoke = Vec::new();
        if let Some(puff) = self.puff {
            for i in 0..7 {
                let age = ((t * 0.35 + i as f32 / 7.0).fract()) * 3.5;
                let pos = scene.fire + Vec3::new(age * 3.0, 8.0 + age * 24.0, -age * 2.0);
                let life = age / 3.5;
                smoke.push(
                    SpriteInstance::new(pos.to_array(), puff)
                        .scale((14.0 + 45.0 * life) / 32.0)
                        .alpha(0.55 * (1.0 - life))
                        .tint([0.32, 0.3, 0.28])
                        .rotation(i as f32 * 1.7),
                );
            }
        }
        self.renderer.set_lights(&lights);
        // Hot air over the fire and the explosion.
        self.renderer.add_heat_source(scene.fire.to_array(), 28.0, 0.8);
        self.renderer.add_heat_source(scene.blast.to_array(), 70.0, 1.0);
        self.renderer.prepare_beams(&[]);
        self.renderer.prepare_sprites(&scene.decals, &scene.billboards, &effects, &smoke, &[]);
        let fleet = if ships { scene.fleet.as_slice() } else { &[] };
        let instances = scene.placed.iter().chain(fleet).map(|u| DrawInstance { model: u.model, pieces: &u.matrices, team: u.team, flags: 0, build: 1.0, damage: 0.0, travel: [0.0; 2], piece: None, playing: ta_render::model::PLAYING_ALL });
        self.renderer.prepare_models(instances);
    }

    /// Renders one frame; returns false when every view of every map is done.
    fn frame(&mut self, o: &Options, vfs: &Vfs) -> Result<bool> {
        while self.current >= self.shots.len() {
            if self.map_index + 1 >= o.maps.len() {
                if o.bench {
                    self.print_bench(o);
                }
                return Ok(false);
            }
            self.load_map(o, vfs, self.map_index + 1)?;
        }
        if self.frame == 0 {
            self.setup_shot();
            (self.renderer.lighting, self.renderer.post.settings) = self.base.clone();
            let mut steady = o.sets.iter().find(|(k, _)| k == "particles").map_or(Ok(0), |(_, v)| v.parse::<u32>()).context("particles=N")?;
            let mut mix = o.sets.iter().find(|(k, _)| k == "mix").map_or("all".to_string(), |(_, v)| v.clone());
            if o.bench {
                for (k, v) in &o.configs[self.bench_config].1 {
                    match k.as_str() {
                        "particles" => steady = v.parse().context("particles=N")?,
                        "mix" => mix = v.clone(),
                        _ => apply_setting(&mut self.renderer, k, v)?,
                    }
                }
            }
            self.steady = steady;
            self.mix = mix;
            // Every view starts without particles; `--particles` sets its own off.
            self.renderer.clear_particles();
            if o.particles {
                let shot = self.shots[self.current].clone();
                self.emit_scene(&shot);
            }
        }
        let shot = self.shots[self.current].clone();
        self.renderer.step_particles(o.step);
        self.emit_frame(o.step);
        let blast = o.particles.then(|| (self.frame as f32 * o.step, shot.name == "nuke"));
        self.submit(o.time, blast, shot.ships);
        if o.bench {
            return Ok(self.bench_frame(o, &shot));
        }
        let last = self.frame + 1 >= o.frames;
        let save = last || o.sequence;
        if save {
            self.renderer.request_screenshot();
        }
        self.renderer.render(&self.camera, |_, _, _, _| {});
        if save {
            let img = self.renderer.take_screenshot().context("screenshot was not captured")?;
            std::fs::create_dir_all(&o.out)?;
            let prefix = o.maps[self.map_index].to_ascii_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "_");
            let name = if o.sequence { format!("{prefix}_{}{}_{:03}.png", shot.name, o.tag, self.frame) } else { format!("{prefix}_{}{}.png", shot.name, o.tag) };
            let path = o.out.join(name);
            image::save_buffer(&path, &img.rgba, img.width, img.height, image::ColorType::Rgba8)?;
            log::info!("saved {} ({:?})", path.display(), self.renderer.particle_stats());
        }
        if last {
            self.current += 1;
            self.frame = 0;
        } else {
            self.frame += 1;
        }
        Ok(true)
    }

    fn bench_frame(&mut self, o: &Options, shot: &Shot) -> bool {
        // Long enough for the steady particle emitter to reach its level.
        const WARMUP: u32 = 240;
        const FRAMES: u32 = 150;
        self.renderer.render(&self.camera, |_, _, _, _| {});
        self.frame += 1;
        if self.frame > WARMUP {
            let timings = self.renderer.gpu_timings();
            self.pass_samples.extend(timings.iter().map(|t| (t.pass, t.ms as f64)));
            self.live_samples.0 += self.renderer.particle_stats().live as u64;
            self.live_samples.1 += 1;
        }
        if self.frame == WARMUP {
            self.bench_start = Some(Instant::now());
            self.pass_samples.clear();
        } else if self.frame == WARMUP + FRAMES {
            let ms = self.bench_start.map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0 / FRAMES as f64);
            let config = &o.configs[self.bench_config].0;
            log::info!("bench {} {} / {config}: {ms:.3} ms per frame", o.maps[self.map_index], shot.name);
            self.bench_results.push((o.maps[self.map_index].clone(), shot.name.to_string(), config.clone(), ms));
            for (pass, ms) in self.pass_samples.drain(..) {
                self.pass_results.push((o.maps[self.map_index].clone(), shot.name.to_string(), config.clone(), self.bench_round, pass, ms));
            }
            let (sum, n) = std::mem::take(&mut self.live_samples);
            self.live_results.push((o.maps[self.map_index].clone(), shot.name.to_string(), config.clone(), sum as f64 / n.max(1) as f64));
            self.frame = 0;
            self.bench_config += 1;
            if self.bench_config == o.configs.len() {
                self.bench_config = 0;
                self.bench_round += 1;
                if self.bench_round == o.rounds {
                    self.bench_round = 0;
                    self.current += 1;
                }
            }
        }
        true
    }

    /// Frame time per view and configuration (median of the rounds), and each configuration's
    /// difference from the first one.
    fn print_bench(&self, o: &Options) {
        let median = |mut v: Vec<f64>| -> f64 {
            if v.is_empty() {
                return f64::NAN;
            }
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let size = self.window.inner_size();
        println!("frame time (ms) at {}x{}, 4x MSAA, vsync off, median of {} rounds", size.width, size.height, o.rounds);
        let mut header = format!("{:<28}", "map / view");
        for (label, _) in &o.configs {
            header.push_str(&format!(" {:>12}", label));
        }
        println!("{header}");
        let mut keys: Vec<(String, String)> = Vec::new();
        for r in &self.bench_results {
            if !keys.iter().any(|k| k.0 == r.0 && k.1 == r.1) {
                keys.push((r.0.clone(), r.1.clone()));
            }
        }
        for (map, view) in keys {
            let get = |c: &str| median(self.bench_results.iter().filter(|r| r.0 == map && r.1 == view && r.2 == c).map(|r| r.3).collect());
            let reference = get(&o.configs[0].0);
            let mut line = format!("{:<28}", format!("{map} / {view}"));
            for (i, (label, _)) in o.configs.iter().enumerate() {
                let ms = get(label);
                if i == 0 {
                    line.push_str(&format!(" {:>12.3}", ms));
                } else {
                    line.push_str(&format!(" {:>12}", format!("{:.3} ({:+.3})", ms, ms - reference)));
                }
            }
            println!("{line}");
        }
        if self.pass_results.is_empty() {
            return;
        }
        println!();
        println!("GPU time per pass (ms, timestamp queries; per round the median frame, then the best round)");
        let mut passes: Vec<&'static str> = Vec::new();
        for r in &self.pass_results {
            if !passes.contains(&r.4) && r.4 != "frame" && r.4 != "full" {
                passes.push(r.4);
            }
        }
        passes.push("full");
        passes.push("frame");
        let mut header = format!("{:<40}", "map / view / config");
        for p in &passes {
            header.push_str(&format!(" {:>10}", p));
        }
        header.push_str(&format!(" {:>10}", "vs first"));
        println!("{header}");
        let mut keys: Vec<(String, String, String)> = Vec::new();
        for r in &self.pass_results {
            if !keys.iter().any(|k| k.0 == r.0 && k.1 == r.1 && k.2 == r.2) {
                keys.push((r.0.clone(), r.1.clone(), r.2.clone()));
            }
        }
        // A run sometimes settles into waiting ~0.3 ms on the swapchain inside the measured
        // window: per round take the median frame, then the round that waited least.
        let best = |map: &str, view: &str, config: &str, pass: &str| {
            (0..o.rounds)
                .map(|round| median(self.pass_results.iter().filter(|r| r.0 == map && r.1 == view && r.2 == config && r.3 == round && r.4 == pass).map(|r| r.5).collect()))
                .filter(|v| !v.is_nan())
                .fold(f64::NAN, f64::min)
        };
        let total = |map: &str, view: &str, config: &str| best(map, view, config, "frame");
        for (map, view, config) in keys {
            let mut line = format!("{:<40}", format!("{map} / {view} / {config}"));
            for p in &passes {
                let ms = best(&map, &view, &config, p);
                if ms.is_nan() {
                    line.push_str(&format!(" {:>10}", "-"));
                } else {
                    line.push_str(&format!(" {:>10.3}", ms));
                }
            }
            let reference = total(&map, &view, &o.configs[0].0);
            line.push_str(&format!(" {:>+10.3}", total(&map, &view, &config) - reference));
            let live: Vec<f64> = self.live_results.iter().filter(|r| r.0 == map && r.1 == view && r.2 == config).map(|r| r.3).collect();
            if live.iter().any(|&l| l > 0.0) {
                line.push_str(&format!("  ({:.0} live particles)", live.iter().sum::<f64>() / live.len() as f64));
            }
            println!("{line}");
        }
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// `--particles`: a unit blowing up at the explosion; in the `nuke` view a nuclear blast
    /// instead, and in the `sea` view a shell bursting on the water.
    fn emit_scene(&mut self, shot: &Shot) {
        let (Some(scene), Some(terrain)) = (&self.scene, &self.renderer.terrain) else { return };
        let (p, ground) = (scene.blast, scene.blast.y);
        let bursts = match shot.name {
            "nuke" => recipes::nuke_bursts(p, 240.0, ground),
            "sea" => {
                let field = &terrain.field;
                let water = Vec3::new(shot.focus.x, field.sea_level, shot.focus.y);
                recipes::blast_bursts(water, 40.0, field.sample(water.x, water.z), recipes::Ground::Sea, recipes::Blast::Shell, false)
            }
            _ => {
                // (The perspective camera's fireball in place of TA's painted blast, explode3 at
                // the dying unit's scale.)
                let mut b = recipes::fireball_bursts(p + Vec3::Y * 8.0, 24.6, 23.0 / 20.0);
                b.extend(recipes::blast_bursts(p, 55.0, ground, recipes::Ground::Land, recipes::Blast::Death, true));
                b
            }
        };
        self.renderer.emit_particles(&bursts);
    }

    /// Particles that keep coming: embers and flames over the burning wreck (`--particles`),
    /// and the steady emitter of `particles=N`.
    fn emit_frame(&mut self, step: f32) {
        let Some(scene) = &self.scene else { return };
        let fire = scene.fire;
        let site = scene.blast + Vec3::new(190.0, 0.0, -150.0);
        let mut bursts = Vec::new();
        if self.options_particles {
            bursts.push(ParticleBurst::new(ParticleKind::Ember, fire.to_array(), 1).count((step * 30.0).ceil() as u32).radius(8.0).toward([0.0, 1.0, 0.0], 0.5).speed(15.0, 55.0).life(1.2, 3.2).size(2.4, 1.0).over(step));
            bursts.push(ParticleBurst::new(ParticleKind::Fire, (fire + Vec3::Y * 3.0).to_array(), 1).count((step * 16.0).ceil() as u32).radius(6.0).toward([0.0, 1.0, 0.0], 0.35).speed(8.0, 25.0).life(0.4, 0.8).size(10.0, 3.0).gravity(-50.0).over(step));
        }
        if self.steady > 0 {
            // A battle's worth of blasts around the base, each 1-2 s of particles: the rate
            // that keeps about `steady` alive (mean lives ~1.6 s).
            let per_frame = self.steady as f32 * step / 1.6;
            let blasts = (per_frame / 400.0).ceil().max(1.0) as u32;
            for _ in 0..blasts {
                let n = per_frame / blasts as f32;
                let p = site + Vec3::new(self.rand() * 700.0 - 350.0, 0.0, self.rand() * 500.0 - 250.0);
                let at = [p.x, p.y + 3.0, p.z];
                let share = |kind: &str, all: f32| if self.mix == "all" { all } else if self.mix == kind { 1.0 } else { 0.0 };
                let count = |share: f32| (n * share).round() as u32;
                bursts.push(ParticleBurst::new(ParticleKind::Spark, at, count(share("sparks", 0.6))).life(1.1, 2.1).speed(40.0, 240.0).size(2.8, 1.4));
                bursts.push(ParticleBurst::new(ParticleKind::Ember, at, count(share("embers", 0.2))).life(1.0, 2.2).size(3.0, 1.2));
                bursts.push(ParticleBurst::new(ParticleKind::Fire, at, count(share("fire", 0.04))).life(1.0, 2.2).size(8.0, 16.0));
                bursts.push(ParticleBurst::new(ParticleKind::Dirt, at, count(share("dirt", 0.12))).ground_color().life(1.0, 2.2).size(3.2, 2.8));
                bursts.push(ParticleBurst::new(ParticleKind::Dust, at, count(share("dust", 0.04))).ground_color().life(1.0, 2.2).size(10.0, 30.0));
            }
        }
        self.renderer.emit_particles(&bursts);
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn")).init();
    let options = Options::parse()?;
    let install = ta_vfs::install::discover(None).map_err(|e| anyhow::anyhow!("{e} (this example needs the original game data)"))?;
    let mut vfs = Vfs::new();
    vfs.mount_game_dir(&install.root, "base")?;
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App { options, vfs: Arc::new(vfs), state: None, error: None };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
