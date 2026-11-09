//! Visual check and benchmark for the sky and the world beyond the map edges.
//!
//! Loads one or more maps from the installed game, renders a list of scripted views of each
//! (map edges and corners at low and at TA's default pitch, the horizon, a low sun, the
//! whole map, and classic-projection corners), saves one PNG per view and exits:
//!
//! ```text
//! cargo run -p ta_render --example sky --release -- \
//!     [--maps "Coast To Coast,Painted Desert"] [--out scratch/sky] [--frames 12] \
//!     [--only NAME[,NAME]] [--size 1600x900] [--time SECONDS] \
//!     [--view NAME:x,z,zoom,yaw,pitch[,sun_azimuth,sun_elevation]]... [--bench]
//! ```
//!
//! Files are named `<map>_<view>.png`. `--time` freezes the animation clock so captures are
//! comparable; `--view` adds a view (degrees; a negative pitch uses the classic projection).
//!
//! `--bench` disables vsync and, instead of saving screenshots, prints the average frame time
//! of every perspective view with everything on, with the sky and extended ground off, with the
//! sky but no extended ground (so the sky pass shades all the background), and without clouds.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use glam::{Vec2, Vec3};
use ta_formats::image::Palette;
use ta_formats::tnt::TntMap;
use ta_render::terrain::HeightField;
use ta_render::{Camera, CameraMode, Renderer};
use ta_vfs::Vfs;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// A view to capture: name, (x, z, zoom, yaw, pitch), optional sun azimuth and elevation.
type View = (String, [f32; 5], Option<(f32, f32)>);

struct Options {
    maps: Vec<String>,
    out: PathBuf,
    frames: u32,
    only: Option<Vec<String>>,
    size: (u32, u32),
    /// Extra views.
    views: Vec<View>,
    /// Frozen animation clock (seconds), for comparable captures.
    time: Option<f32>,
    bench: bool,
}

impl Options {
    fn parse() -> Result<Options> {
        let mut o = Options {
            maps: vec!["Coast To Coast".into()],
            out: PathBuf::from("scratch/sky"),
            frames: 12,
            only: None,
            size: (1600, 900),
            views: Vec::new(),
            time: None,
            bench: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            let mut value = || args.next().with_context(|| format!("{a} needs a value"));
            match a.as_str() {
                "--maps" | "--map" => o.maps = value()?.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                "--out" => o.out = PathBuf::from(value()?),
                "--frames" => o.frames = value()?.parse().context("--frames")?,
                "--only" => o.only = Some(value()?.split(',').map(|s| s.trim().to_string()).collect()),
                "--time" => o.time = Some(value()?.parse().context("--time")?),
                "--bench" => o.bench = true,
                // --view NAME:x,z,zoom,yaw,pitch[,sun_azimuth,sun_elevation] (degrees); a
                // negative pitch selects the classic projection.
                "--view" => {
                    let v = value()?;
                    let (name, nums) = v.split_once(':').context("--view NAME:x,z,zoom,yaw,pitch")?;
                    let n: Vec<f32> = nums.split(',').map(|s| s.trim().parse()).collect::<Result<_, _>>().context("--view numbers")?;
                    anyhow::ensure!(n.len() == 5 || n.len() == 7, "--view needs 5 or 7 numbers");
                    o.views.push((name.to_string(), [n[0], n[1], n[2], n[3], n[4]], (n.len() == 7).then(|| (n[5], n[6]))));
                }
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
    name: String,
    mode: CameraMode,
    /// Ground focus (x, z).
    focus: Vec2,
    zoom: f32,
    yaw_deg: f32,
    pitch_deg: f32,
    /// Sun (azimuth, elevation) in degrees, azimuth 0 = north, 90 = east; `None` keeps the default.
    sun: Option<(f32, f32)>,
}

fn shot(name: &str, mode: CameraMode, focus: Vec2, zoom: f32, yaw_deg: f32, pitch_deg: f32) -> Shot {
    Shot { name: name.to_string(), mode, focus, zoom, yaw_deg, pitch_deg, sun: None }
}

/// Views of a map: edges, a corner, the horizon, a low sun, the whole map, classic corners.
fn plan(field: &HeightField, viewport: Vec2) -> Vec<Shot> {
    use CameraMode::{Classic, Perspective};
    let size = field.size_px();
    let (w, h) = (size.x, size.y);
    // Zoom at which the whole map fits the screen height at a 32° pitch (roughly).
    let fit = (viewport.y / (w.max(h) * 0.9)).clamp(0.05, 1.0);
    vec![
        // Looking north over the north edge at a low pitch.
        shot("edge_north", Perspective, Vec2::new(w * 0.5, 320.0), 1.0, 0.0, 24.0),
        // Looking east over the east edge.
        shot("edge_east", Perspective, Vec2::new(w - 320.0, h * 0.5), 1.0, 90.0, 28.0),
        // Looking south-west over the south-west corner.
        shot("edge_sw", Perspective, Vec2::new(360.0, h - 360.0), 0.8, 225.0, 22.0),
        // TA's default pitch at the north-east corner, zoomed out a little.
        shot("corner_ta", Perspective, Vec2::new(w - 200.0, 220.0), 0.6, 0.0, 63.4),
        // Grazing view towards the horizon (north-west, where the default sun is).
        shot("horizon", Perspective, Vec2::new(w * 0.5, h * 0.55), 0.9, -40.0, 6.0),
        // The same with a low sun ahead.
        Shot { sun: Some((-40.0, 7.0)), ..shot("sunset", Perspective, Vec2::new(w * 0.5, h * 0.55), 0.9, -40.0, 4.0) },
        // The whole map in its surroundings.
        shot("overview", Perspective, Vec2::new(w * 0.5, h * 0.5), fit, -25.0, 32.0),
        // Classic projection at two corners: must look exactly as before.
        shot("classic_nw", Classic, Vec2::new(240.0, 200.0), 1.0, 0.0, 63.4),
        shot("classic_se", Classic, Vec2::new(w - 240.0, h - 200.0), 1.0, 0.0, 63.4),
    ]
}

/// Benchmark configurations: label and what to change from the defaults.
const BENCH_CONFIGS: [&str; 4] = ["all on", "sky off", "no ground", "no clouds"];
/// Frames rendered before timing a configuration, and frames timed.
const BENCH_WARMUP: u32 = 60;
const BENCH_FRAMES: u32 = 300;

struct State {
    window: Arc<Window>,
    renderer: Renderer,
    camera: Camera,
    palette: Palette,
    map_index: usize,
    shots: Vec<Shot>,
    current: usize,
    frame: u32,
    day: ta_render::Lighting,
    /// Benchmark: index into BENCH_CONFIGS, timing start, and (map, view, config, ms) results.
    bench_config: usize,
    bench_start: Option<Instant>,
    bench_results: Vec<(String, String, &'static str, f64)>,
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

impl State {
    fn new(event_loop: &ActiveEventLoop, o: &Options, vfs: &Vfs) -> Result<State> {
        let window = Arc::new(event_loop.create_window(
            Window::default_attributes()
                .with_title("ta_render: sky and beyond the map")
                .with_inner_size(PhysicalSize::new(o.size.0, o.size.1))
                .with_resizable(false),
        )?);
        let size = window.inner_size();
        let mut renderer = Renderer::new(window.clone(), size.width, size.height)?;
        renderer.set_animation_time(o.time);
        if o.bench {
            renderer.gpu.set_vsync(false);
        }
        let palette = Palette::parse(&vfs.read("palettes/palette.pal")?)?;
        let camera = Camera::new(Vec2::new(size.width as f32, size.height as f32));
        let day = renderer.lighting.clone();
        let mut state = State {
            window,
            renderer,
            camera,
            palette,
            map_index: 0,
            shots: Vec::new(),
            current: 0,
            frame: 0,
            day,
            bench_config: 0,
            bench_start: None,
            bench_results: Vec::new(),
        };
        state.load_map(o, vfs, 0)?;
        Ok(state)
    }

    fn load_map(&mut self, o: &Options, vfs: &Vfs, index: usize) -> Result<()> {
        let name = &o.maps[index];
        let map_path = format!("maps/{name}.tnt");
        let map = TntMap::parse(&vfs.read(&map_path).with_context(|| map_path.clone())?)?;
        log::info!("map {name}: {}x{} px, sea level {}", map.width_px(), map.height_px(), map.sea_level);
        self.renderer.load_map(&map, &self.palette);
        let field = &self.renderer.terrain.as_ref().context("terrain")?.field;
        let mut shots = plan(field, self.camera.viewport);
        if let Some(only) = &o.only {
            shots.retain(|s| only.contains(&s.name));
        }
        for (name, [x, z, zoom, yaw, pitch], sun) in &o.views {
            let mode = if *pitch < 0.0 { CameraMode::Classic } else { CameraMode::Perspective };
            shots.push(Shot { sun: *sun, ..shot(name, mode, Vec2::new(*x, *z), *zoom, *yaw, pitch.abs()) });
        }
        if o.bench {
            // The classic view draws none of this.
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
        let mut l = self.day.clone();
        if let Some((azimuth, elevation)) = s.sun {
            let (az, el) = (azimuth.to_radians(), elevation.to_radians());
            // Azimuth 0 = north (-z), 90 = east (+x).
            l.sun_dir = Vec3::new(az.sin() * el.cos(), el.sin(), -az.cos() * el.cos());
        }
        self.renderer.lighting = l;
        log::info!("view {}: {:?}", s.name, s);
    }

    /// Renders one frame; returns false when every view of every map has been captured.
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
        }
        let shot = self.shots[self.current].clone();
        if o.bench {
            return self.bench_frame(o, &shot);
        }
        let last = self.frame + 1 >= o.frames;
        if last {
            self.renderer.request_screenshot();
        }
        self.renderer.render(&self.camera, |_, _, _, _| {});
        if last {
            let img = self.renderer.take_screenshot().context("screenshot was not captured")?;
            std::fs::create_dir_all(&o.out)?;
            let prefix = o.maps[self.map_index].to_ascii_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "_");
            let path = o.out.join(format!("{prefix}_{}.png", shot.name));
            image::save_buffer(&path, &img.rgba, img.width, img.height, image::ColorType::Rgba8)?;
            log::info!("saved {}", path.display());
            self.current += 1;
            self.frame = 0;
        } else {
            self.frame += 1;
        }
        Ok(true)
    }

    fn bench_frame(&mut self, o: &Options, shot: &Shot) -> Result<bool> {
        let config = BENCH_CONFIGS[self.bench_config];
        let sky = &mut self.renderer.lighting.sky;
        let defaults = ta_render::SkySettings::default();
        sky.enabled = config != "sky off";
        sky.horizon_extent = if config == "no ground" { 0.0 } else { defaults.horizon_extent };
        sky.cloud_cover = if config == "no clouds" { 0.0 } else { defaults.cloud_cover };
        self.renderer.render(&self.camera, |_, _, _, _| {});
        self.frame += 1;
        if self.frame == BENCH_WARMUP {
            self.bench_start = Some(Instant::now());
        } else if self.frame == BENCH_WARMUP + BENCH_FRAMES {
            let ms = self.bench_start.map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0 / BENCH_FRAMES as f64);
            log::info!("bench {} {} / {config}: {ms:.3} ms per frame", o.maps[self.map_index], shot.name);
            self.bench_results.push((o.maps[self.map_index].clone(), shot.name.clone(), config, ms));
            self.frame = 0;
            self.bench_config += 1;
            if self.bench_config == BENCH_CONFIGS.len() {
                self.bench_config = 0;
                self.current += 1;
            }
        }
        Ok(true)
    }

    /// Prints the benchmark table: frame time per view and configuration, and the cost of the
    /// whole feature, of the sky pass (sky shading the whole background) and of the clouds.
    fn print_bench(&self, o: &Options) {
        let size = self.window.inner_size();
        println!("frame time (ms) at {}x{}, 4x MSAA, vsync off, {} frames each", size.width, size.height, BENCH_FRAMES);
        println!(
            "{:<30} {:>8} {:>8} {:>9} {:>9} {:>8} {:>8} {:>8}",
            "map / view", "all on", "sky off", "no ground", "no clouds", "feature", "sky", "clouds"
        );
        for map in &o.maps {
            let views: Vec<&String> = {
                let mut v: Vec<&String> = self.bench_results.iter().filter(|r| &r.0 == map).map(|r| &r.1).collect();
                v.dedup();
                v
            };
            for view in views {
                let get = |c: &str| self.bench_results.iter().find(|r| &r.0 == map && &r.1 == view && r.2 == c).map_or(f64::NAN, |r| r.3);
                let (all, off, no_ground, no_clouds) = (get("all on"), get("sky off"), get("no ground"), get("no clouds"));
                println!(
                    "{:<30} {:>8.3} {:>8.3} {:>9.3} {:>9.3} {:>+8.3} {:>+8.3} {:>+8.3}",
                    format!("{map} / {view}"),
                    all,
                    off,
                    no_ground,
                    no_clouds,
                    all - off,
                    no_ground - off,
                    all - no_clouds
                );
            }
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
