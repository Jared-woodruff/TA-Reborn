//! Visual check for the water surface and dynamic point lights.
//!
//! Loads a map from the installed game, places a few units and animated point lights, renders
//! a list of scripted views, saves one PNG per view and exits:
//!
//! ```text
//! cargo run -p ta_render --example water_lights --release -- \
//!     [--map "Coast To Coast"] [--out scratch/water] [--frames 45] [--only NAME[,NAME]] [--size 1600x900]
//!     [--bench]
//! ```
//!
//! Views: `persp_coast` (low pitch towards the sun: reflections and glitter), `persp_ta`
//! (TA's default 63° pitch), `classic` (TA's oblique projection: the painted art must stay
//! readable), `close_foam` (shoreline detail), `persp_far` (grazing view over open water:
//! distance aliasing), `battle_day`, `night_lights` and `night_classic` (point lights on
//! terrain, units and water). Coast To Coast has hand-picked spots; other maps use the
//! shoreline nearest the map centre.
//!
//! `--bench` disables vsync and, instead of saving screenshots, prints the average frame
//! time of every view with everything on, with the water disabled and without point lights.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use glam::{Mat4, Vec2, Vec3};
use ta_formats::image::Palette;
use ta_formats::model3do::Model3do;
use ta_formats::tnt::TntMap;
use ta_render::terrain::HeightField;
use ta_render::{BeamInstance, Camera, CameraMode, DrawInstance, ModelId, PointLight, Renderer, SpriteId, SpriteInstance};
use ta_vfs::Vfs;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

struct Options {
    map: String,
    out: PathBuf,
    frames: u32,
    only: Option<Vec<String>>,
    size: (u32, u32),
    bench: bool,
}

/// Benchmark configurations: (label, water enabled, point lights enabled).
const BENCH_CONFIGS: [(&str, bool, bool); 3] = [("all on", true, true), ("no water", false, true), ("no lights", true, false)];
/// Frames rendered before timing a configuration, and frames timed.
const BENCH_WARMUP: u32 = 60;
const BENCH_FRAMES: u32 = 300;

impl Options {
    fn parse() -> Result<Options> {
        let mut o = Options {
            map: "Coast To Coast".into(),
            out: PathBuf::from("scratch/water"),
            frames: 45,
            only: None,
            size: (1600, 900),
            bench: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            let mut value = || args.next().with_context(|| format!("{a} needs a value"));
            match a.as_str() {
                "--map" => o.map = value()?,
                "--out" => o.out = PathBuf::from(value()?),
                "--frames" => o.frames = value()?.parse().context("--frames")?,
                "--bench" => o.bench = true,
                "--only" => o.only = Some(value()?.split(',').map(|s| s.trim().to_string()).collect()),
                "--size" => {
                    let v = value()?;
                    let (w, h) = v.split_once('x').context("--size WxH")?;
                    o.size = (w.parse()?, h.parse()?);
                }
                other => anyhow::bail!("unknown argument {other}"),
            }
        }
        Ok(o)
    }
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
    night: bool,
    lights: bool,
}

fn shot(name: &'static str, mode: CameraMode, focus: Vec2, zoom: f32, yaw_deg: f32, pitch_deg: f32) -> Shot {
    Shot { name, mode, focus, zoom, yaw_deg, pitch_deg, night: false, lights: false }
}

/// Views and the anchor (a beach point next to the water) for units and lights.
fn plan(map_name: &str, field: &HeightField) -> (Vec<Shot>, Vec2, Vec2) {
    use CameraMode::{Classic, Perspective};
    if map_name.eq_ignore_ascii_case("Coast To Coast") {
        // East shore of the central sea (land at x > ~2450) and the west shore near z = 1000.
        let east = Vec2::new(2440.0, 760.0);
        let west = Vec2::new(690.0, 1000.0);
        let shots = vec![
            shot("persp_coast", Perspective, Vec2::new(2400.0, 640.0), 1.5, -45.0, 33.0),
            shot("persp_ta", Perspective, west, 1.6, 0.0, 63.4),
            shot("classic", Classic, west, 1.6, 0.0, 63.4),
            shot("close_foam", Perspective, Vec2::new(2455.0, 560.0), 4.0, -30.0, 50.0),
            shot("persp_far", Perspective, Vec2::new(1500.0, 1000.0), 0.55, -45.0, 20.0),
            Shot { lights: true, ..shot("battle_day", Perspective, east, 1.7, -20.0, 48.0) },
            Shot { lights: true, night: true, ..shot("night_lights", Perspective, east, 1.7, -20.0, 48.0) },
            Shot { lights: true, night: true, ..shot("night_classic", Classic, east, 1.6, 0.0, 63.4) },
        ];
        return (shots, east, Vec2::new(-1.0, 0.0));
    }
    let (anchor, seaward) = find_shore(field);
    let shots = vec![
        shot("persp_coast", Perspective, anchor + seaward * 120.0, 1.5, -45.0, 33.0),
        shot("persp_ta", Perspective, anchor, 1.6, 0.0, 63.4),
        shot("classic", Classic, anchor, 1.6, 0.0, 63.4),
        shot("close_foam", Perspective, anchor, 4.0, -30.0, 50.0),
        shot("persp_far", Perspective, anchor + seaward * 600.0, 0.55, -45.0, 20.0),
        Shot { lights: true, ..shot("battle_day", Perspective, anchor, 1.7, -20.0, 48.0) },
        Shot { lights: true, night: true, ..shot("night_lights", Perspective, anchor, 1.7, -20.0, 48.0) },
        Shot { lights: true, night: true, ..shot("night_classic", Classic, anchor, 1.6, 0.0, 63.4) },
    ];
    (shots, anchor, seaward)
}

/// The waterline point nearest the map centre and the horizontal direction towards the sea.
fn find_shore(field: &HeightField) -> (Vec2, Vec2) {
    let centre = field.size_px() * 0.5;
    let mut best: Option<(f32, Vec2, Vec2)> = None;
    for z in 1..field.height as i32 - 1 {
        for x in 1..field.width as i32 - 1 {
            let h = field.vertex(x, z);
            if h < field.sea_level {
                continue;
            }
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                if field.vertex(x + dx, z + dz) < field.sea_level {
                    let p = Vec2::new(x as f32 * 16.0, z as f32 * 16.0);
                    let d = p.distance(centre);
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, p, Vec2::new(dx as f32, dz as f32)));
                    }
                }
            }
        }
    }
    best.map_or((centre, Vec2::X), |(_, p, dir)| (p, dir))
}

/// A model placed in its rest pose.
struct Placed {
    model: ModelId,
    matrices: Vec<Mat4>,
    team: u32,
}

struct State {
    window: Arc<Window>,
    renderer: Renderer,
    camera: Camera,
    shots: Vec<Shot>,
    current: usize,
    frame: u32,
    units: Vec<Placed>,
    anchor: Vec2,
    seaward: Vec2,
    glow: Option<SpriteId>,
    start: Instant,
    day: ta_render::Lighting,
    /// Benchmark: index into BENCH_CONFIGS, timing start, and the results so far.
    bench_config: usize,
    bench_start: Option<Instant>,
    bench_results: Vec<(String, &'static str, f64)>,
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
            WindowEvent::RedrawRequested => match state.frame(&self.options) {
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

impl State {
    fn new(event_loop: &ActiveEventLoop, o: &Options, vfs: &Vfs) -> Result<State> {
        let window = Arc::new(event_loop.create_window(
            Window::default_attributes()
                .with_title("ta_render: water + point lights")
                .with_inner_size(PhysicalSize::new(o.size.0, o.size.1))
                .with_resizable(false),
        )?);
        let size = window.inner_size();
        let mut renderer = Renderer::new(window.clone(), size.width, size.height)?;

        let palette = Palette::parse(&vfs.read("palettes/palette.pal")?)?;
        let map_path = format!("maps/{}.tnt", o.map);
        let map = TntMap::parse(&vfs.read(&map_path).with_context(|| map_path.clone())?)?;
        log::info!("map {}: {}x{} px, sea level {}", o.map, map.width_px(), map.height_px(), map.sea_level);
        renderer.load_map(&map, &palette);
        let field = renderer.terrain.as_ref().context("terrain")?.field.clone();
        if renderer.water.is_none() {
            log::warn!("{} has no water; the views will only show point lights", o.map);
        }

        // Model textures and a few units: a commander and tanks on the beach, ships offshore
        // and a submarine below the surface.
        let gafs: Vec<ta_formats::gaf::Gaf> = vfs
            .list("textures", Some("gaf"), false)
            .iter()
            .filter_map(|p| vfs.read_opt(p).and_then(|d| ta_formats::gaf::Gaf::parse(&d).ok()))
            .collect();
        renderer.init_models(&gafs, &palette);
        let (mut shots, anchor, seaward) = plan(&o.map, &field);
        if let Some(only) = &o.only {
            shots.retain(|s| only.iter().any(|n| n == s.name));
            anyhow::ensure!(!shots.is_empty(), "no view matches --only {only:?}");
        }
        let mut units = Vec::new();
        let sea = field.sea_level;
        let land = |d: Vec2| {
            let p = anchor + d;
            Vec3::new(p.x, field.sample(p.x, p.y), p.y)
        };
        let across = Vec2::new(-seaward.y, seaward.x);
        let wet = |d: Vec2, below: f32| {
            let p = anchor + d;
            Vec3::new(p.x, sea - below, p.y)
        };
        let heading_to_sea = seaward.x.atan2(seaward.y);
        let placements = [
            ("armcom", land(-seaward * 110.0), heading_to_sea, 0),
            ("armstump", land(-seaward * 70.0 - across * 60.0), heading_to_sea + 0.4, 0),
            ("armstump", land(-seaward * 150.0 + across * 70.0), heading_to_sea - 0.3, 0),
            ("armroy", wet(seaward * 120.0 + across * 10.0, 0.0), heading_to_sea + 1.4, 0),
            ("corroy", wet(seaward * 260.0 - across * 150.0, 0.0), heading_to_sea - 1.9, 1),
            ("armsub", wet(seaward * 200.0 + across * 140.0, 14.0), heading_to_sea + 2.0, 0),
        ];
        let models = renderer.models.as_mut().context("models")?;
        for (name, pos, heading, team) in placements {
            let Some(bytes) = vfs.read_opt(&format!("objects3d/{name}.3do")) else { continue };
            let model = Model3do::parse(&bytes)?;
            let id = models.library.add(name, &model, &models.textures, &palette);
            let info = models.library.info(id);
            let root = Mat4::from_translation(pos) * Mat4::from_rotation_y(heading);
            let matrices = info
                .rest_pose()
                .iter()
                .zip(&info.pieces)
                // Muzzle flash and wake pieces are hidden unless a script shows them.
                .map(|(m, p)| if p.name.starts_with("flare") || p.name.starts_with("wake") { Mat4::ZERO } else { root * *m })
                .collect();
            units.push(Placed { model: id, matrices, team });
        }

        // A soft round glow sprite to make the light sources visible.
        let n = 64u32;
        let mut rgba = Vec::with_capacity((n * n * 4) as usize);
        for y in 0..n {
            for x in 0..n {
                let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(Vec2::splat(n as f32 * 0.5)) / (n as f32 * 0.5);
                let a = (1.0 - d).clamp(0.0, 1.0).powf(2.2);
                rgba.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
            }
        }
        let glow = renderer.sprites.atlas.add_rgba(n, n, &rgba, n as f32 * 0.5, n as f32 * 0.5);

        let mut camera = Camera::new(Vec2::new(size.width as f32, size.height as f32));
        camera.zoom = 1.6;
        let day = renderer.lighting.clone();
        if o.bench {
            renderer.gpu.set_vsync(false);
        }
        Ok(State {
            window,
            renderer,
            camera,
            shots,
            current: 0,
            frame: 0,
            units,
            anchor,
            seaward,
            glow,
            start: Instant::now(),
            day,
            bench_config: 0,
            bench_start: None,
            bench_results: Vec::new(),
        })
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

        let mut l = self.day.clone();
        if s.night {
            // Moonlight: dim bluish sun, dark ambient and haze.
            l.sun_intensity = 0.22;
            l.sun_color = Vec3::new(0.75, 0.85, 1.0);
            l.ambient = Vec3::new(0.16, 0.19, 0.28);
            l.fog_color = Vec3::new(0.10, 0.13, 0.2);
        }
        self.renderer.lighting = l;
        log::info!("view {}: {:?}", s.name, s);
    }

    /// Animated lights around the anchor: a fireball and a burning wreck on the beach, an
    /// explosion over the water, a destroyer's muzzle flash and a laser with its glow.
    fn battle_lights(&self, t: f32, field: &HeightField) -> (Vec<PointLight>, Vec<BeamInstance>) {
        let a = self.anchor;
        let s = self.seaward;
        let across = Vec2::new(-s.y, s.x);
        let ground = |d: Vec2, lift: f32| {
            let p = a + d;
            [p.x, field.sample(p.x, p.y).max(field.sea_level) + lift, p.y]
        };
        let flicker = |f: f32, phase: f32| 0.75 + 0.15 * (t * f + phase).sin() + 0.1 * (t * f * 2.7 + phase * 1.3).sin();
        let mut lights = vec![
            // Fireball between the tanks.
            PointLight::new(ground(-s * 95.0 - across * 25.0, 24.0), [7.0 * flicker(9.0, 0.0), 3.3 * flicker(9.0, 0.0), 1.0], 170.0),
            // Burning wreck further up the beach.
            PointLight::new(ground(-s * 180.0 + across * 20.0, 14.0), [3.4 * flicker(13.0, 2.0), 1.3 * flicker(11.0, 1.0), 0.3], 95.0),
            // Explosion over the water, pulsing.
            PointLight::new(ground(s * 220.0 - across * 60.0, 26.0), [6.0, 4.6, 3.0].map(|c| c * (0.55 + 0.45 * (t * 2.3).sin().abs())), 230.0),
        ];
        // Destroyer muzzle flash: short bursts.
        let burst = ((t * 1.7).fract() < 0.18) as i32 as f32;
        if burst > 0.0 {
            lights.push(PointLight::new(ground(s * 120.0 + across * 10.0 - s * 18.0, 22.0), [5.0, 3.6, 1.6], 120.0));
        }
        // A green laser from the ship to the beach, with glow lights along it.
        let from = Vec3::from(ground(s * 110.0 + across * 12.0, 18.0));
        let to = Vec3::from(ground(-s * 60.0 + across * 50.0, 10.0));
        let beams = vec![BeamInstance::new(from.to_array(), to.to_array(), 3.0, [0.4, 2.6, 0.6], 1.0)];
        for k in 0..4 {
            let p = from.lerp(to, (k as f32 + 0.5) / 4.0);
            lights.push(PointLight::new(p.to_array(), [0.35, 1.9, 0.45], 70.0));
        }
        // A scatter of small fires to exercise the light grid.
        for k in 0..40 {
            let f = k as f32;
            let d = Vec2::new((f * 137.5).to_radians().cos(), (f * 137.5).to_radians().sin()) * (80.0 + f * 18.0);
            lights.push(PointLight::new(ground(d - s * 260.0, 10.0), [2.2 * flicker(7.0 + f, f), 0.9, 0.2], 45.0 + (f % 5.0) * 8.0));
        }
        (lights, beams)
    }

    /// Renders one frame; returns false when every view has been captured (or benchmarked).
    fn frame(&mut self, o: &Options) -> Result<bool> {
        if self.current >= self.shots.len() {
            if o.bench {
                self.print_bench();
            }
            return Ok(false);
        }
        if self.frame == 0 {
            self.setup_shot();
        }
        let shot = self.shots[self.current].clone();
        let (config, water_on, lights_on) = BENCH_CONFIGS[self.bench_config];
        if o.bench {
            self.renderer.lighting.water.enabled = water_on;
        }
        let t = self.start.elapsed().as_secs_f32();
        let field = self.renderer.terrain.as_ref().expect("terrain").field.clone();
        let (mut lights, beams) = if shot.lights { self.battle_lights(t, &field) } else { (Vec::new(), Vec::new()) };
        let effects: Vec<SpriteInstance> = match self.glow {
            Some(glow) => lights
                .iter()
                .filter(|l| l.radius > 60.0)
                .map(|l| {
                    let peak = l.color.iter().copied().fold(0.0f32, f32::max).max(1e-3);
                    SpriteInstance::new(l.position, glow)
                        .scale(l.radius / 180.0)
                        .tint([l.color[0] / peak, l.color[1] / peak, l.color[2] / peak])
                        .brightness(peak * 0.5)
                })
                .collect(),
            None => Vec::new(),
        };
        if o.bench && !lights_on {
            lights.clear();
        }
        self.renderer.set_lights(&lights);
        self.renderer.prepare_beams(&beams);
        self.renderer.prepare_sprites(&[], &[], &effects, &[], &[]);
        let instances = self.units.iter().map(|u| DrawInstance { model: u.model, pieces: &u.matrices, team: u.team, flags: 0, build: 1.0, damage: 0.0, travel: [0.0; 2], piece: None, playing: ta_render::model::PLAYING_ALL });
        self.renderer.prepare_models(instances);

        if o.bench {
            self.renderer.render(&self.camera, |_, _, _, _| {});
            self.frame += 1;
            if self.frame == BENCH_WARMUP {
                self.bench_start = Some(Instant::now());
            } else if self.frame == BENCH_WARMUP + BENCH_FRAMES {
                let ms = self.bench_start.map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0 / BENCH_FRAMES as f64);
                log::info!("bench {} / {config}: {ms:.3} ms per frame", shot.name);
                self.bench_results.push((shot.name.to_string(), config, ms));
                self.frame = 0;
                self.bench_config += 1;
                if self.bench_config == BENCH_CONFIGS.len() {
                    self.bench_config = 0;
                    self.current += 1;
                }
            }
            return Ok(true);
        }

        let last = self.frame + 1 >= o.frames;
        if last {
            self.renderer.request_screenshot();
        }
        self.renderer.render(&self.camera, |_, _, _, _| {});
        if last {
            let shot_img = self.renderer.take_screenshot().context("screenshot was not captured")?;
            std::fs::create_dir_all(&o.out)?;
            // Other maps get their name as a prefix so a survey does not overwrite the default set.
            let prefix = if o.map.eq_ignore_ascii_case("Coast To Coast") {
                String::new()
            } else {
                format!("{}_", o.map.to_ascii_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "_"))
            };
            let path = o.out.join(format!("{prefix}{}.png", shot.name));
            image::save_buffer(&path, &shot_img.rgba, shot_img.width, shot_img.height, image::ColorType::Rgba8)?;
            log::info!("saved {}", path.display());
            self.current += 1;
            self.frame = 0;
        } else {
            self.frame += 1;
        }
        Ok(true)
    }

    /// Prints the benchmark table: frame time per view and configuration, and the cost of the
    /// water and of the point lights relative to "all on".
    fn print_bench(&self) {
        let size = self.window.inner_size();
        println!("frame time (ms) at {}x{}, 4x MSAA, vsync off, {} frames each", size.width, size.height, BENCH_FRAMES);
        println!("{:<16} {:>9} {:>9} {:>9} {:>9} {:>9}", "view", "all on", "no water", "no lights", "water", "lights");
        for shot in &self.shots {
            let get = |c: &str| self.bench_results.iter().find(|r| r.0 == shot.name && r.1 == c).map_or(f64::NAN, |r| r.2);
            let (all, no_water, no_lights) = (get("all on"), get("no water"), get("no lights"));
            println!(
                "{:<16} {:>9.3} {:>9.3} {:>9.3} {:>+9.3} {:>+9.3}",
                shot.name,
                all,
                no_water,
                no_lights,
                all - no_water,
                all - no_lights
            );
        }
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
