//! TA Reborn renderer.
//!
//! Frame structure:
//! 1. shadow pass (sun shadow map for units and features),
//! 2. (perspective camera) the cloud shadow map and the contact occlusion map around the view,
//! 3. (while any are alive) the GPU particles' compute pass: emission and simulation,
//! 4. (perspective camera, water in view) the reflection pass: the world above the water seen
//!    through the mirrored view, for the water's reflections (see [`reflection`]),
//! 5. HDR scene pass (MSAA): terrain and (perspective camera) the ground beyond the map,
//!    decals, units, feature billboards, the sky on the remaining background, then the
//!    translucent water surface, smoke and alpha-blended particles, additive effects and
//!    particles, beams and the heat haze's hot air,
//! 6. post: bloom + (heat haze distortion) + tone mapping into the swapchain,
//! 7. UI (egui) on top, supplied by the caller.
//!
//! Dynamic point lights ([`PointLight`], submitted each frame with [`Renderer::set_lights`])
//! are binned into a world-space grid and available to every shader through
//! `point_lighting()` in `common.wgsl`; terrain, units, sprites and water use them.
//!
//! The sky and the world beyond the map edges ([`SkySettings`] in [`Lighting::sky`]) are
//! described in [`sky`]; the classic camera draws neither.
//!
//! GPU particles ([`ParticleBurst`]s queued with [`Renderer::emit_particles`], moved by
//! [`Renderer::step_particles`]) are created, simulated and drawn by the GPU: sparks, embers,
//! flames, dust, smoke, grit and spray, hundreds of thousands at a time. Both cameras draw
//! them. See [`gpu_particles`].
//!
//! The perspective camera adds lighting effects, each with a setting so the game can offer
//! quality options (the classic camera renders exactly as it always did):
//! * ambient occlusion ([`AmbientOcclusion`] in [`Lighting::ambient_occlusion`], see
//!   [`occlusion`]): terrain creases baked from the heightmap, and a soft contact darkening
//!   under and between units, buildings, wrecks and trees;
//! * shadows of the sky's clouds ([`Lighting::cloud_shadows`], see [`clouds`]);
//! * heat haze over explosions, fires and lava seas ([`post::PostSettings::heat_haze`], sources
//!   from [`Renderer::add_heat_source`], see [`heat`]);
//! * light on units from the sky above and the painted ground below ([`Lighting::sky_light`]);
//! * smoke fading out softly where it meets the ground ([`Lighting::soft_smoke`]);
//! * the world above the water mirrored in it, rippled by the waves: shores and cliffs, units,
//!   ships and 3D features, trees, explosions, smoke and beams ([`WaterReflections`] in
//!   [`Lighting::water_reflections`], see [`reflection`]; off on lava and void seas).
//!
//! [`Renderer::set_gpu_timing`] measures the GPU time of every pass.

pub mod beams;
pub mod rings;
pub mod camera;
pub mod clouds;
pub mod cockpit;
pub mod terrain_detail;
pub mod daylight;
pub mod foliage;
pub mod glb;
pub mod gpu;
pub mod gpu_particles;
pub mod heat;
pub mod icons;
pub mod lights;
pub mod map_preview;
pub mod model;
pub mod occlusion;
pub mod portrait;
pub mod preview;
pub mod post;
pub mod raytrace;
pub mod reflection;
pub mod room;
pub mod sky;
pub mod space;
pub mod sprites;
mod surroundings;
pub mod terrain;
mod terrain_pads;
pub mod terrain_shadow;
mod terrain_strands;
pub mod terrain_surface;
pub mod timing;
pub mod util;
pub mod wakes;
pub mod water;
pub mod world_text;

use std::time::Instant;

/// What the renderer is building as the game starts ("" once it is built), for a crash report:
/// a graphics driver that fails while compiling a shader takes the game down with it (the game's
/// crash handler names the step from here).
static BUILDING: std::sync::Mutex<(&'static str, Option<Instant>)> = std::sync::Mutex::new(("", None));

/// What the renderer was building when last asked: empty once it is built (see [`Building`]).
pub fn startup_step() -> &'static str {
    BUILDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0
}

/// Seconds after which a step still being built is said in the log (a graphics driver's shader
/// compiler that never finishes would otherwise leave no trace).
const STILL_BUILDING: [u64; 3] = [10, 60, 300];

/// The renderer's start step by step: what is being built now ([`startup_step`]), and what each
/// step took (a graphics driver's shader compiler that takes long shows in the log).
struct Building {
    started: Instant,
    steps: Vec<(&'static str, f32)>,
    /// Set when the building is over: the watch (see [`STILL_BUILDING`]) stops.
    over: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Building {
    fn new() -> Building {
        let over = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watch = over.clone();
        let _ = std::thread::Builder::new().name("renderer-watch".into()).spawn(move || {
            let mut told: (&'static str, usize) = ("", 0);
            while !watch.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(250));
                let (step, since) = *BUILDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let Some(since) = since.filter(|_| !step.is_empty()) else { continue };
                if told.0 != step {
                    told = (step, 0);
                }
                if let Some(&secs) = STILL_BUILDING.get(told.1)
                    && since.elapsed().as_secs() >= secs
                {
                    log::warn!("renderer: still building {step} after {secs} s (the graphics driver compiling their shaders)");
                    told.1 += 1;
                }
            }
        });
        Building { started: Instant::now(), steps: Vec::new(), over }
    }

    fn step<T>(&mut self, what: &'static str, make: impl FnOnce() -> T) -> T {
        *BUILDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = (what, Some(Instant::now()));
        // `TA_CRASH_IN=<words>`: an access violation in the step whose name has them, as a
        // graphics driver's would be, to try the game's crash report.
        if std::env::var("TA_CRASH_IN").is_ok_and(|words| !words.is_empty() && what.contains(words.as_str())) {
            log::warn!("TA_CRASH_IN: crashing in {what} on purpose");
            // SAFETY: none meant: the crash is the point.
            unsafe { std::ptr::write_volatile(std::ptr::without_provenance_mut::<u8>(16), 1) };
        }
        let t0 = Instant::now();
        let made = make();
        let secs = t0.elapsed().as_secs_f32();
        if secs >= 5.0 {
            log::warn!("renderer: {what} took {secs:.1} s (the graphics driver compiling their shaders)");
        }
        self.steps.push((what, secs));
        made
    }

    /// The end: logged, the slowest steps first.
    fn done(mut self, what: &str) {
        *BUILDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = ("", None);
        self.over.store(true, std::sync::atomic::Ordering::Relaxed);
        self.steps.sort_by(|a, b| b.1.total_cmp(&a.1));
        let slowest: Vec<String> = self.steps.iter().take(4).map(|(step, secs)| format!("{step} {secs:.2} s")).collect();
        log::info!("{what} built in {:.2} s (the slowest: {})", self.started.elapsed().as_secs_f32(), slowest.join(", "));
    }
}

impl Drop for Building {
    fn drop(&mut self) {
        self.over.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};

pub use camera::{Camera, CameraMode};

/// The cockpits Perspective puts round the view (GLB, made by `tools/cockpits/build.py`), by
/// the kind of unit: (name, file). `TA_COCKPITS=<folder>` reads `<name>.glb` from that folder
/// where it has one (and the textures and shader, see [`cockpit_file`]): the builder's output
/// tried without building the game again.
pub fn cockpit_models() -> &'static [(&'static str, &'static [u8])] {
    static MODELS: std::sync::OnceLock<Vec<(&'static str, &'static [u8])>> = std::sync::OnceLock::new();
    MODELS.get_or_init(|| {
        let built_in: [(&'static str, &'static [u8]); 8] = [
            ("vehicle", include_bytes!("../cockpits/vehicle.glb")),
            ("walker", include_bytes!("../cockpits/walker.glb")),
            ("titan", include_bytes!("../cockpits/titan.glb")),
            ("commander", include_bytes!("../cockpits/commander.glb")),
            ("bridge", include_bytes!("../cockpits/bridge.glb")),
            ("canopy", include_bytes!("../cockpits/canopy.glb")),
            ("gunship", include_bytes!("../cockpits/gunship.glb")),
            ("emplacement", include_bytes!("../cockpits/emplacement.glb")),
        ];
        built_in.into_iter().map(|(name, bytes)| (name, cockpit_file(&format!("{name}.glb"), bytes))).collect()
    })
}

/// The textures the cockpits share (`tools/cockpits/art.py`): the wear (chips, scratches,
/// grime, grain; linear) and the decals (stencils, plates, labels; sRGB), as PNG.
pub fn cockpit_textures() -> (&'static [u8], &'static [u8]) {
    static TEXTURES: std::sync::OnceLock<(&'static [u8], &'static [u8])> = std::sync::OnceLock::new();
    *TEXTURES.get_or_init(|| (cockpit_file("wear.png", include_bytes!("../cockpits/wear.png")), cockpit_file("decals.png", include_bytes!("../cockpits/decals.png"))))
}

/// A cockpit's file: `TA_COCKPITS`'s copy where that folder has one, else the built-in one.
/// (Read once by its callers, who keep it.)
pub fn cockpit_file(name: &str, built_in: &'static [u8]) -> &'static [u8] {
    match std::env::var_os("TA_COCKPITS").map(|dir| std::path::PathBuf::from(dir).join(name)) {
        Some(path) if path.is_file() => match std::fs::read(&path) {
            Ok(bytes) => {
                log::info!("TA_COCKPITS: {}", path.display());
                Box::leak(bytes.into_boxed_slice())
            }
            Err(e) => {
                log::warn!("TA_COCKPITS: {}: {e}; using the built-in one", path.display());
                built_in
            }
        },
        _ => built_in,
    }
}
pub use gpu::{CheckedBufferInit, Gpu, can_allocate, out_of_memory_events};
pub use gpu_particles::{ParticleBurst, ParticleKind, ParticleSettings, ParticleStats};
pub use heat::HeatSource;
pub use lights::{MAX_LIGHTS, PointLight};
pub use model::{DrawInstance, ModelId, ModelRenderer, TextureLibrary};
pub use beams::{BeamInstance, BeamRenderer};
pub use rings::RingInstance;
pub use world_text::{GlyphInstance, WorldTextRenderer};
pub use reflection::WaterReflections;
pub use sky::SkySettings;
pub use sprites::{SpriteId, SpriteInstance, SpriteLayer, SpriteRenderer};
pub use terrain_pads::GroundPad;
pub use timing::PassTime;
pub use wakes::{WakePoint, WakeTrail};
pub use water::{Fluid, PaintedSea, WaterSettings, WaterSurface};
use clouds::CloudShadows;
use gpu_particles::{GpuParticles, ParticleLayer};
use heat::HeatRenderer;
use lights::{GpuLight, LightGrid};
use occlusion::{Occluder, OcclusionMap, Region};
use post::Post;
use reflection::Reflections;
use sky::{MapAtmosphere, SkirtMesh, SkyPipeline};
use terrain::{TerrainGpu, TerrainPipeline};
use raytrace::RayTracing;
use terrain_shadow::TerrainShadow;
use timing::GpuTimer;
use water::{WaterPipeline, WaterScene};

/// The animation clock (waves, lava, clouds) starts again every this many seconds (about 18
/// hours: one jump then, and never coarser steps than an f32 has at 65,536).
const ANIMATION_PERIOD: f64 = 65536.0;

/// Mirrors `Frame` in `shaders/common.wgsl` (every field is 16-byte aligned).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    shadow_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    world: [f32; 4],
    screen: [f32; 4],
    fog: [f32; 4],
    billboard_right: [f32; 4],
    billboard_up: [f32; 4],
    water_color: [f32; 4],
    water: [f32; 4],
    water_extra: [f32; 4],
    shore_water: [f32; 4],
    shore_land: [f32; 4],
    light_grid_origin: [f32; 4],
    light_grid_dims: [u32; 4],
    los: [f32; 4],
    sky_zenith: [f32; 4],
    sky: [f32; 4],
    ground_beyond: [f32; 4],
    beyond_sea: [f32; 4],
    beyond_shore: [f32; 4],
    inv_view_proj: [[f32; 4]; 4],
    ao: [f32; 4],
    ao_map: [f32; 4],
    clouds: [f32; 4],
    heat: [f32; 4],
    ground_bounce: [f32; 4],
    detail: [f32; 4],
    daylight: [f32; 4],
    night: [f32; 4],
    celestial: [f32; 4],
    terrain_shadow: [f32; 4],
    wind: [f32; 4],
    rain: [f32; 4],
    shadow_soft: [f32; 4],
    rt: [f32; 4],
    liquid: [f32; 4],
    world_grade: [[f32; 4]; 3],
    drift: [f32; 4],
}

/// What the ground shows over its art (a player's map view).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapOverlay {
    None,
    /// The colours uploaded with [`TerrainGpu::set_overlay`] (metal, movement, sight).
    Cells,
    /// Heights: a colour ramp from the lowest to the highest ground, with contour lines
    /// (drawn per pixel from the ground's own height).
    Height,
}

/// The viewer's sight over the map, a texel a cell, for [`Renderer::set_fog_of_war`].
pub struct FogOfWar<'a> {
    /// Cells per axis.
    pub width: u32,
    pub height: u32,
    /// World units per cell.
    pub cell: f32,
    /// `width × height` cells of four bytes, row-major: how far the cell is in sight, explored,
    /// in radar or sonar range (each 0..255; the game eases them from one state to the next, so
    /// what comes into sight or leaves it fades), and a fourth the shader leaves alone.
    pub texels: &'a [u8],
}

/// How bright TA's own fog leaves a place by its state in sight, explored and radar range (as
/// `fog_brightness` in common.wgsl; the radar changes nothing): 1 in sight, a half explored, a
/// fifth never seen.
pub fn fog_brightness(sight: f32, explored: f32) -> f32 {
    let known = 0.2 + (0.5 - 0.2) * explored.max(sight);
    known + (1.0 - known) * sight
}

/// How much the ground moves in the wind this frame (0..1): the setting
/// ([`Lighting::ground_life`]), none for those who asked for reduced motion.
fn ground_life_strength(setting: f32, reduced_motion: bool) -> f32 {
    if reduced_motion { 0.0 } else { setting.clamp(0.0, 1.0) }
}

/// Everything the frame bind group (group 0) holds.
struct FrameResources<'a> {
    frame_buffer: &'a wgpu::Buffer,
    shadow_view: &'a wgpu::TextureView,
    shadow_sampler: &'a wgpu::Sampler,
    light_buffer: &'a wgpu::Buffer,
    light_grid_buffer: &'a wgpu::Buffer,
    los_view: &'a wgpu::TextureView,
    los_sampler: &'a wgpu::Sampler,
    occlusion_view: &'a wgpu::TextureView,
    cloud_view: &'a wgpu::TextureView,
    ground_view: &'a wgpu::TextureView,
    /// Ray tracing's results: shade and reflections (only with a device that traces, see
    /// raytrace.rs), and the 3D trees' and rocks' own shadow map (`feature_shadow_map`).
    rt_views: Option<(&'a wgpu::TextureView, &'a wgpu::TextureView, &'a wgpu::TextureView)>,
}

/// The frame bind group (group 0): shared by every pass, with the shadow pass getting a
/// dummy in place of the shadow map it renders into.
fn frame_group(device: &wgpu::Device, label: &str, layout: &wgpu::BindGroupLayout, r: &FrameResources<'_>) -> wgpu::BindGroup {
    let view = wgpu::BindingResource::TextureView;
    let mut entries = vec![
            wgpu::BindGroupEntry { binding: 0, resource: r.frame_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: view(r.shadow_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(r.shadow_sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: r.light_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: r.light_grid_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 5, resource: view(r.los_view) },
            wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(r.los_sampler) },
            wgpu::BindGroupEntry { binding: 7, resource: view(r.occlusion_view) },
            wgpu::BindGroupEntry { binding: 8, resource: view(r.cloud_view) },
            wgpu::BindGroupEntry { binding: 9, resource: view(r.ground_view) },
    ];
    if let Some((shade, reflections, features)) = r.rt_views {
        entries.push(wgpu::BindGroupEntry { binding: 10, resource: view(shade) });
        entries.push(wgpu::BindGroupEntry { binding: 11, resource: view(reflections) });
        entries.push(wgpu::BindGroupEntry { binding: 12, resource: view(features) });
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout, entries: &entries })
}

/// Layout entry for a filterable 2D texture read by fragment shaders (and vertex shaders: the
/// particles work out their light, fog of war and height above the ground per vertex).
fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn los_texture(device: &wgpu::Device, width: u32, height: u32) -> util::Target {
    util::Target::new(
        device,
        "fog of war",
        width,
        height,
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    )
}

/// Layout entry for a read-only storage buffer used by fragment (and vertex) shaders.
fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Global lighting and atmosphere settings.
#[derive(Clone, Debug)]
pub struct Lighting {
    /// Direction towards the sun (normalised on use).
    pub sun_dir: Vec3,
    pub sun_color: Vec3,
    pub sun_intensity: f32,
    pub ambient: Vec3,
    /// 0 = terrain exactly as painted, 1 = full dynamic relighting of slopes.
    pub terrain_relight: f32,
    pub shadow_strength: f32,
    /// Distance haze colour; also the horizon colour of the sky and of water reflections
    /// (with the perspective sky, tinted per map by [`SkySettings::map_tint`]).
    pub fog_color: Vec3,
    pub fog_density: f32,
    /// Clear colour behind the map (classic view, or with the sky disabled).
    pub sky_color: Vec3,
    /// Water surface appearance (maps with a sea level).
    pub water: WaterSettings,
    /// Sky and the ground beyond the map edges (perspective camera).
    pub sky: SkySettings,
    /// Ambient occlusion: terrain creases and contact darkening (perspective camera).
    pub ambient_occlusion: AmbientOcclusion,
    /// Strength of the shadows the sky's drifting clouds cast on the ground, water, units and
    /// effects: 0 = off, 1 = a thick cloud blocks all direct sunlight. Perspective camera with
    /// the sky on only; the pattern and its drift are those of the clouds in the sky
    /// ([`SkySettings::cloud_cover`], [`SkySettings::cloud_speed`]). Costs one small pass
    /// (512² texels) and a texture lookup per lit pixel.
    pub cloud_shadows: f32,
    /// How much units take the colours of the sky above them and of the painted ground below
    /// (light bounced off it) instead of the plain ambient colour (0..1; perspective camera).
    pub sky_light: f32,
    /// Soft smoke: height (world units) over which smoke fades out as it meets the ground,
    /// instead of being cut off where the ground passes in front of it (perspective camera;
    /// 0 = off).
    pub soft_smoke: f32,
    /// Reflections of the world above the water (shores, units, ships, trees, explosions) in
    /// seas of water, and their resolution (perspective camera; see [`reflection`]). Costs one
    /// more pass drawing the ground, models, billboards and effects mirrored into a target of
    /// that resolution, in frames that show water.
    pub water_reflections: WaterReflections,
    /// Rain clouds over the battlefield (0 = the map's own light, 1 = a downpour): the sun
    /// dims, shadows soften, the sky clouds over and the haze darkens (perspective camera).
    pub overcast: f32,
    /// Strength (0..1) of the shadows the relief casts: ridges shade the valleys and what
    /// stands in them, long under a low sun (perspective camera; see [`terrain_shadow`]).
    /// Costs a compute pass whenever the light moves, and a lookup per lit pixel.
    pub terrain_shadows: f32,
    /// The wind over the battlefield: world xz, its length 0 (calm) to about 1.5 (a gale).
    /// Trees bend and flutter in it (see [`Lighting::foliage_sway`]).
    pub wind: Vec2,
    /// How much (0..1) trees sway in the wind (perspective camera).
    pub foliage_sway: f32,
    /// How much (0..1) the ground moves in the wind (perspective camera): grass shimmers in waves
    /// that roll downwind, sand and snow smoke with blown grains and powder (see `ground_life` in
    /// terrain.wgsl). 0 leaves the painted ground still, as reduced motion does.
    pub ground_life: f32,
    /// How wet the world is (0..1; perspective camera): the ground darkens and gleams, water
    /// stands in its low, flat places, units darken and shine.
    pub wetness: f32,
    /// How hard it rains now (0..1): rings on the water and the puddles.
    pub rain: f32,
    /// Contact-hardening soft shadows of models: the shadow blurs as far as its caster stands
    /// above what it falls on (sharp at a tank's tracks, soft under a high aircraft), as the
    /// sun's disc makes it. The spread's slope (penumbra width per unit of distance); 0 = the
    /// plain 3×3 filter. Costs a blocker search and a wider filter per shadowed pixel.
    pub soft_shadows: f32,
    /// Ray-traced shadows and ambient occlusion of the models, where the device traces rays
    /// (perspective camera; see [`raytrace`] and [`Renderer::ray_tracing_available`]).
    pub ray_tracing: bool,
    /// HD surfaces (0..1; perspective camera): as the camera comes close, the ground's
    /// dithered pixels give way to smooth colour with fine grain and relief lighting, and
    /// models gain worn edges and creases, battle damage, reflections on metal and glowing
    /// lights.
    /// 0 keeps TA's art exactly as painted.
    pub hd_surfaces: f32,
    /// The time of day, in hours (perspective camera; see [`daylight`]): `None` is TA's own
    /// light, which is also what noon ([`daylight::NOON`]) looks like. The sun, the moon, the
    /// sky and the light on the painted ground follow the clock.
    pub time_of_day: Option<f32>,
    /// Where the moon is (a direction towards it), with a time of day; `None`: high in TA's
    /// north-west. It lights the night from there.
    pub moon_dir: Option<Vec3>,
}

/// Ambient occlusion settings (perspective camera only; see [`occlusion`]). Part of
/// [`Lighting`].
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientOcclusion {
    /// Master switch.
    pub enabled: bool,
    /// Strength (0..1) of the terrain's occlusion (creases, valley floors, the foot of cliffs),
    /// baked from the heightmap when the map loads, so it costs one texture lookup per terrain
    /// pixel. Acts less on art that is painted dark already.
    pub terrain: f32,
    /// Strength (0..1) of the soft darkening of the ground under and around units, buildings,
    /// wrecks and trees (overlapping ones darken more, e.g. between buildings). Costs one small
    /// pass (a quad per occluder into a 1024² map) and a lookup per pixel.
    pub contact: f32,
    /// How much (0..1) the lower parts of models and trees darken near the ground (from the
    /// contact occlusion around them, so only while `contact` is above 0).
    pub models: f32,
}

impl Default for AmbientOcclusion {
    fn default() -> Self {
        AmbientOcclusion { enabled: true, terrain: 0.7, contact: 0.55, models: 0.5 }
    }
}

impl Default for Lighting {
    fn default() -> Self {
        Lighting {
            // TA's painted art is lit from the upper left (north-west).
            sun_dir: Vec3::new(-0.45, 0.8, -0.4),
            sun_color: Vec3::new(1.0, 0.97, 0.92),
            sun_intensity: 1.0,
            ambient: Vec3::new(0.42, 0.45, 0.5),
            terrain_relight: 0.35,
            shadow_strength: 0.85,
            fog_color: Vec3::new(0.55, 0.62, 0.7),
            fog_density: 0.00012,
            sky_color: Vec3::new(0.018, 0.022, 0.03),
            water: WaterSettings::default(),
            sky: SkySettings::default(),
            ambient_occlusion: AmbientOcclusion::default(),
            cloud_shadows: 0.4,
            sky_light: 1.0,
            soft_smoke: 12.0,
            water_reflections: WaterReflections::Half,
            overcast: 0.0,
            terrain_shadows: 1.0,
            wind: Vec2::new(0.5, 0.2),
            foliage_sway: 1.0,
            ground_life: 1.0,
            wetness: 0.0,
            rain: 0.0,
            soft_shadows: 0.035,
            ray_tracing: true,
            hd_surfaces: 1.0,
            time_of_day: None,
            moon_dir: None,
        }
    }
}

/// Strength of the ray-traced ambient occlusion (see raytrace.rs).
const RT_AO_STRENGTH: f32 = 0.9;

/// How strongly a lava world's melt glows on the rock standing over it from afar (see
/// `melt_glow` in common.wgsl), on a map with plenty of melt.
const MELT_GLOW: f32 = 1.0;

/// How much of the sun's and the sky's light rain clouds take away at their thickest: the
/// rain's mood is in its flat light, grey haze and wet sheen; the land keeps enough light to
/// keep its colour and its relief.
const RAIN_DIM: f32 = 0.35;

/// How deep (world units) the sun's shadow map reaches, from its near plane, at the least: as
/// deep as the ground across it rises and falls along the sun's way (`shadow_depth`).
const SHADOW_DEPTH_RANGE: f32 = 6400.0;
/// Room the shadow map keeps along the sun's way beyond the ground it covers (world units): for
/// hills over the slab, tall units and aircraft towards the sun, valleys away from it.
const SHADOW_DEPTH_ROOM: f32 = 1500.0;
/// The tallest that casts a shadow into the map from beyond it, towards the sun (world units).
const SHADOW_CASTER_HEIGHT: f32 = 400.0;

/// How far (world units) the shadow map's eye stands back from its middle along the sun's way,
/// and how deep from its near plane the map reaches, for a map of half-size `extent` (the
/// ground it covers lies within that of its middle), the sun's height over the horizon being
/// `sun_y` (its direction's y). The ground across the map comes nearer the sun by its half-size
/// on the sun's side and goes further on the other (times the cosine of the sun's height), and
/// what casts a shadow into it can stand further out still towards a low sun. (A fixed 3000 put
/// the far side of a big map beyond its depth: unshadowed, and what stood towards the sun cast
/// nothing.) Never less deep than `SHADOW_DEPTH_RANGE`.
fn shadow_depth(extent: f32, sun_y: f32) -> (f32, f32) {
    let sun_y = sun_y.clamp(0.12, 1.0);
    let flat = (1.0 - sun_y * sun_y).sqrt();
    let caster = (SHADOW_CASTER_HEIGHT * flat / sun_y).min(4000.0);
    let back = ((extent + caster) * flat + SHADOW_DEPTH_ROOM).max(SHADOW_DEPTH_RANGE * 0.5);
    (back, (back + extent * flat + SHADOW_DEPTH_ROOM).max(SHADOW_DEPTH_RANGE))
}
/// How far (shadow map texels) soft shadows look for casters around a point: as far as their
/// widest penumbra reaches.
const SHADOW_SEARCH_TEXELS: f32 = 9.0;
/// The steps the shadow map's size is taken in (an eighth of an octave).
const SHADOW_EXTENT_STEP: f32 = 1.090_507_7;
/// The shadow map's half-size at most (world units).
const SHADOW_EXTENT_MAX: f32 = 6000.0;
/// Shadows fade out over the shadow map's outer tenth (of its half-size; also in the shaders,
/// `shadow_edge_keep`): the view is fitted inside the rest.
const SHADOW_FADE_FROM: f32 = 0.9;
/// How far from the eye (world units, across the ground) the shadow map covers the view at
/// most: a view out to the horizon is cut short nearer still, as far as the map holds.
const SHADOW_DISTANCE: f32 = 24_000.0;

/// The middle of the shadow map and how far the view reaches from it (world units, across the
/// ground), for a view whose outline on the ground is `points` seen from above `eye`: the
/// ground within some distance of the eye (as far as the map holds, `SHADOW_EXTENT_MAX`, its
/// faded rim left over; the outline's points further off brought in to it). A view running out
/// towards the horizon meets the ground tens of thousands of units off: fitted to all of it,
/// the map's middle was far away, and cut down to the largest map it left the ground nearest
/// the camera without a shadow (the title menu's camera, swinging low and up again, had its
/// trees' shadows come and go).
fn shadow_fit(eye: Vec2, points: &[Vec2]) -> (Vec2, f32) {
    let fit = |distance: f32| {
        let near: Vec<Vec2> = points
            .iter()
            .map(|&p| {
                let d = p - eye;
                let l = d.length();
                if l > distance { eye + d * (distance / l) } else { p }
            })
            .collect();
        let centre = near.iter().copied().sum::<Vec2>() / near.len().max(1) as f32;
        let reach = near.iter().map(|p| p.distance(centre)).fold(0.0, f32::max);
        (centre, reach)
    };
    let mut distance = SHADOW_DISTANCE;
    let mut best = fit(distance);
    while best.1 + 150.0 > SHADOW_EXTENT_MAX * SHADOW_FADE_FROM && distance > 500.0 {
        distance *= 0.85;
        best = fit(distance);
    }
    best
}

/// The shadow map's half-size for a view needing `needed` world units: `current` while that
/// still holds the view and is no more than two steps too big, else the step at or just above
/// `needed`. (Between steps a texel keeps its size, so shadows hold still as the camera moves.)
fn steady_shadow_extent(current: f32, needed: f32) -> f32 {
    if current >= needed && current <= needed * SHADOW_EXTENT_STEP * SHADOW_EXTENT_STEP {
        return current;
    }
    SHADOW_EXTENT_STEP.powf(needed.max(1.0).log(SHADOW_EXTENT_STEP).ceil())
}

/// Strength of the contact occlusion of upright sprites (trees, rocks) relative to models.
const SPRITE_OCCLUSION: f32 = 0.6;
/// How strongly the ground darkens round a 3D rock's foot (see `occlusion::contact_occluder`).
const ROCK_OCCLUSION: f32 = 1.0;

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Which of the perspective camera's effects this frame draws.
#[derive(Clone, Copy, Debug, Default)]
struct FrameEffects {
    /// The contact occlusion map holds occluders.
    contact: bool,
    /// The cloud shadow map is drawn.
    clouds: bool,
    /// Heat haze sources and lava heat are drawn, and the composite distorts.
    heat: bool,
    /// The lava surface adds heat.
    lava_heat: bool,
    /// The reflection pass draws the world above the water for its reflections.
    reflections: bool,
    /// Shadows and ambient occlusion are ray traced (see raytrace.rs).
    ray_traced: bool,
}

pub struct Renderer {
    pub gpu: Gpu,
    pub post: Post,
    pub lighting: Lighting,
    /// This frame's light: [`Renderer::lighting`] at the time of day (see
    /// [`Renderer::daylight`]), which the shadows, the clouds and the frame uniform read.
    frame_lighting: Lighting,
    /// The time of day's light this frame, if one is set and shown.
    frame_daylight: Option<daylight::Daylight>,
    frame_layout: wgpu::BindGroupLayout,
    frame_buffer: wgpu::Buffer,
    frame_bind_group: wgpu::BindGroup,
    /// Same as `frame_bind_group` but with a dummy shadow texture, for the shadow pass itself.
    shadow_pass_bind_group: wgpu::BindGroup,
    /// The frame uniform of the mirrored camera, and a frame bind group with it in place of
    /// the scene's, for the reflection pass (see reflection.rs).
    reflection_frame_buffer: wgpu::Buffer,
    reflection_bind_group: wgpu::BindGroup,
    /// The reflection's target and its bindings for the water.
    reflections: Reflections,
    shadow_dummy: util::Target,
    shadow_sampler: wgpu::Sampler,
    /// Fog-of-war brightness texture (1×1 white while inactive).
    los: util::Target,
    los_sampler: wgpu::Sampler,
    los_size: (u32, u32),
    /// 1 / world extent of the fog texture, whether fog of war is active, and its mean
    /// brightness (0..1; what the ground beyond the map fades to).
    los_params: [f32; 4],
    shadow_map: util::Target,
    /// With ray tracing, the 3D trees' and rocks' shadows in a map of their own: the traced
    /// scene holds the models alone, and surfaces whose sun is traced read this for theirs.
    feature_shadow_map: Option<util::Target>,
    shadow_size: u32,
    shadow_view_proj: Mat4,
    /// World units per texel of the shadow map this frame.
    shadow_texel: f32,
    /// How deep the shadow map reaches this frame (world units, see `shadow_depth`).
    shadow_depth: f32,
    /// The shadow map's half-size in world units, kept in steps (see `update_shadow_matrix`).
    shadow_extent: f32,
    terrain_pipeline: TerrainPipeline,
    /// The ground's detail materials (shared by every map; its world set per map).
    terrain_detail: terrain_detail::TerrainDetail,
    pub terrain: Option<TerrainGpu>,
    /// The map overlay shown on the ground (see [`Renderer::set_overlay`]).
    overlay: MapOverlay,
    water_pipeline: WaterPipeline,
    /// What the water reads of the opaque scene behind it (made when the water is first drawn).
    water_scene: WaterScene,
    /// Water surface of the current map (`None` when the map has no water).
    pub water: Option<WaterSurface>,
    sky_pipeline: SkyPipeline,
    /// The ground beyond the current map's edges (rebuilt when its reach changes).
    skirt: Option<SkirtMesh>,
    /// Mean painted colour of the current map (tints its sky), and the gains its world's grade
    /// balances its colours by (see `terrain_detail::WorldGrade::balance_gains`).
    map_average: Option<Vec3>,
    map_balance_gains: Vec3,
    /// The world's colour grade as the shaders take it (the frame's `world_grade`, see
    /// `terrain_detail::TerrainDetail::set_world`): the ground, the features and their 3D
    /// stand-ins are graded alike.
    world_grade: [[f32; 4]; 3],
    /// The key of the map's land and its colour balance (see `terrain::land_key`), for its
    /// world's colour grade.
    map_key: f32,
    map_balance: [f32; 3],
    /// The share of the map's ground under the sea (a lava world's melt).
    sea_share: f32,
    /// Whether the map's world has TA's hair-like strands painted over (Reborn style; see
    /// `terrain_detail::world_strands`), and whether the last frame was the perspective camera's:
    /// only it draws them larger, into scribbles; from TA's own camera and distance they read as
    /// the shading they were drawn as, which painted over left the hills soft smudges.
    ground_strands: bool,
    strands_view: bool,
    /// The sky of the map's world (see [`sky::world_sky`]), and where its clouds lie (an offset
    /// of the cloud field, world units, from the map's own seed: maps seen from the same place
    /// at the same time do not share a skyline).
    world_sky: sky::WorldSky,
    cloud_offset: Vec2,
    pub models: Option<ModelRenderer>,
    /// TA Reborn's own trees, bushes and rocks (see [`foliage`]); none if they failed to load.
    pub foliage: Option<foliage::FoliageRenderer>,
    pub sprites: SpriteRenderer,
    pub beams: BeamRenderer,
    /// Rings on the ground round selected units (see [`rings`]).
    rings: rings::RingRenderer,
    /// Text and signs in the world, drawn over the ground under the decals (see [`world_text`]).
    pub world_text: WorldTextRenderer,
    /// The wakes of ships and hovercraft, drawn into the water's wake map (see [`wakes`]).
    wakes: wakes::WakeRenderer,
    /// Strategic icons over the finished image (see [`icons`]).
    icons: icons::IconRenderer,
    icon_list: Vec<icons::IconInstance>,
    /// GPU particles (see [`Renderer::emit_particles`]).
    particles: GpuParticles,
    /// This frame's point lights (see [`Renderer::set_lights`]).
    lights: Vec<PointLight>,
    light_buffer: wgpu::Buffer,
    light_grid_buffer: wgpu::Buffer,
    /// Contact occlusion map around the view (perspective camera).
    occlusion: OcclusionMap,
    /// This frame's occluders (scratch).
    occluders: Vec<Occluder>,
    /// Occluders of the upright sprites (trees, rocks) last prepared, at full strength.
    sprite_occluders: Vec<Occluder>,
    /// And of the 3D rocks where they meet the ground (see `prepare_foliage`).
    foliage_occluders: Vec<Occluder>,
    /// Cloud shadow map (perspective camera with the sky).
    cloud_shadows: CloudShadows,
    /// The shadows the relief casts (see [`terrain_shadow`]).
    terrain_shadow: TerrainShadow,
    /// Ray-traced shadows and ambient occlusion, where the device traces (see [`raytrace`]).
    ray_tracing: Option<RayTracing>,
    /// Report a model-library storage limit once; raster rendering stays available.
    rt_storage_warning: bool,
    /// The room the intro plays in, while it is up (see [`room`]).
    room: Option<room::RoomRenderer>,
    /// A space scene, while one is up (see [`space`]).
    space: Option<space::SpaceRenderer>,
    /// Perspective: the cockpit round the view, and how it is this frame (none: not drawn).
    cockpit: Option<cockpit::CockpitRenderer>,
    cockpit_frame: Option<cockpit::CockpitFrame>,
    /// This frame's heat haze sources (see [`Renderer::add_heat_source`]).
    heat_sources: Vec<HeatSource>,
    heat: HeatRenderer,
    /// GPU pass timing, when enabled and supported.
    timer: Option<GpuTimer>,
    start: Instant,
    /// Frozen animation clock (see [`Renderer::set_animation_time`]).
    frozen_time: Option<f32>,
    /// How far the wind has carried what drifts on it (see [`Renderer::advance_wind_drift`]),
    /// and the animation clock's time it was last moved on at.
    wind_drift: Vec2,
    drift_time: Option<f32>,
    screenshot_request: bool,
    last_screenshot: Option<Screenshot>,
    /// The window's size, and the fraction of it the scene is drawn at (see
    /// [`Renderer::set_render_scale`]).
    window_size: (u32, u32),
    render_scale: f32,
    /// While a replay is made into a film, its pictures' size: the scene's targets stay that
    /// size between its pictures (see [`Renderer::set_film_size`]).
    film_size: Option<(u32, u32)>,
    /// Units in colour-blind-safe player colours (see [`Renderer::set_colour_blind`]).
    colour_blind: bool,
    /// Reborn or Enhanced Classic art, independent of projection and performance settings.
    art_style_reborn: bool,
}

/// Choices fixed when the renderer is made.
#[derive(Clone, Copy, Debug)]
pub struct RendererOptions {
    /// Antialiasing samples wanted for the scene (1, 2, 4 or 8); the largest count up to it
    /// that the device supports is used.
    pub msaa: u32,
}

impl Default for RendererOptions {
    fn default() -> Self {
        RendererOptions { msaa: 4 }
    }
}

/// The largest sample count up to `wanted` that the scene's colour and depth formats both
/// support on this device: 1 and 4 always, 2 and 8 when the adapter has them and the device
/// was opened with its specific format features.
fn supported_samples(gpu: &Gpu, wanted: u32) -> u32 {
    let specific = gpu.device.features().contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
    let ok = |n: u32| {
        matches!(n, 1 | 4)
            || specific && [util::HDR_FORMAT, util::DEPTH_FORMAT].iter().all(|&f| gpu.adapter.get_texture_format_features(f).flags.sample_count_supported(n))
    };
    [8, 4, 2, 1].into_iter().filter(|&n| n <= wanted.max(1)).find(|&n| ok(n)).unwrap_or(1)
}

/// Captured frame pixels (RGBA8, sRGB).
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn ray_trace_buffer_sizes_fit(vertices: u64, indices: u64, binding_limit: u64) -> bool {
    vertices <= binding_limit && indices <= binding_limit
}

impl Renderer {
    pub fn new(window: impl Into<wgpu::SurfaceTarget<'static>>, width: u32, height: u32) -> anyhow::Result<Self> {
        Self::with_gpu(Gpu::new(window, width, height, true, false)?, width, height, RendererOptions::default())
    }

    /// A renderer on a device already opened (so its kind can pick the options first).
    pub fn with_gpu(gpu: Gpu, width: u32, height: u32, options: RendererOptions) -> anyhow::Result<Self> {
        let mut building = Building::new();
        // (Before any pipeline: the shaders read the ray-traced result, or not.)
        util::set_ray_traced(gpu.ray_tracing);
        let device = &gpu.device;
        let samples = supported_samples(&gpu, options.msaa);
        if samples != options.msaa.max(1) {
            log::info!("{}x antialiasing is not supported here: using {samples}x", options.msaa);
        }

        let mut frame_entries = vec![
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                // Point lights and their world-space grid (see lights.rs).
                storage_entry(3),
                storage_entry(4),
                // Fog of war, and the linear clamped sampler every map below shares.
                texture_entry(5),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Contact occlusion map, cloud shadow map, the ground's heights and the relief's
                // shadows (terrain_shadow.rs).
                texture_entry(7),
                texture_entry(8),
                texture_entry(9),
        ];
        // Ray tracing's results (raytrace.rs), read with textureLoad: shade and reflections.
        if gpu.ray_tracing {
            for binding in [10, 11] {
                frame_entries.push(wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                });
            }
        }
        // The 3D trees' and rocks' own shadow map, which surfaces whose sun is ray traced read for
        // their shadows (the traced scene holds the models only): see `feature_shadow_map`.
        if gpu.ray_tracing {
            frame_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 12,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("frame layout"), entries: &frame_entries });
        let frame_uniform = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<FrameUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let frame_buffer = frame_uniform("frame uniform");
        let reflection_frame_buffer = frame_uniform("frame uniform (reflection)");
        let light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point lights"),
            size: (lights::MAX_LIGHTS * std::mem::size_of::<GpuLight>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let light_grid_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point light grid"),
            size: (lights::GRID_WORDS * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_size = 4096;
        let shadow_map = util::Target::new(
            device,
            "shadow map",
            shadow_size,
            shadow_size,
            util::DEPTH_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow compare"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        // The shadow pass cannot bind the shadow map it renders into, so it gets a twin bind
        // group with a dummy depth texture; every other binding must be identical.
        let shadow_dummy = util::Target::new(device, "shadow dummy", 1, 1, util::DEPTH_FORMAT, 1, wgpu::TextureUsages::TEXTURE_BINDING);
        // (Ray tracing only: the 3D features cast into a map of their own, see `render`.)
        let feature_shadow_map = gpu.ray_tracing.then(|| {
            util::Target::new(device, "feature shadow map", shadow_size, shadow_size, util::DEPTH_FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING)
        });
        let los = los_texture(device, 1, 1);
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &los.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &[255; 4],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let los_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fog of war"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let occlusion = building.step("the contact occlusion's pipelines", || OcclusionMap::new(device));
        let cloud_shadows = building.step("the cloud shadows' pipelines", || CloudShadows::new(device));
        let ground_dummy = util::Target::new(device, "ground dummy", 1, 1, wgpu::TextureFormat::Rg8Unorm, 1, wgpu::TextureUsages::TEXTURE_BINDING);
        let terrain_shadow = building.step("the relief's shadow pipelines", || TerrainShadow::new(device));
        let ray_tracing = gpu.ray_tracing.then(|| building.step("the ray tracing's pipelines", || RayTracing::new(device)));
        let resources = |frame_buffer, shadow_view| FrameResources {
            frame_buffer,
            shadow_view,
            shadow_sampler: &shadow_sampler,
            light_buffer: &light_buffer,
            light_grid_buffer: &light_grid_buffer,
            los_view: &los.view,
            los_sampler: &los_sampler,
            occlusion_view: &occlusion.target.view,
            cloud_view: &cloud_shadows.target.view,
            ground_view: &terrain_shadow.target.view,
            rt_views: None,
        };
        // (The shadow passes bind dummies in place of the maps they render into.)
        let features_view = feature_shadow_map.as_ref().map_or(&shadow_dummy.view, |m| &m.view);
        let rt_scene = ray_tracing.as_ref().map(|r| (r.result_view(), r.reflection_view(), features_view));
        let rt_shadow = ray_tracing.as_ref().map(|r| (r.result_view(), r.reflection_view(), &shadow_dummy.view));
        let frame_bind_group = frame_group(device, "frame", &frame_layout, &FrameResources { rt_views: rt_scene, ..resources(&frame_buffer, &shadow_map.view) });
        let shadow_pass_bind_group = frame_group(device, "frame (shadow pass)", &frame_layout, &FrameResources { rt_views: rt_shadow, ..resources(&frame_buffer, &shadow_dummy.view) });
        let reflection_bind_group = frame_group(device, "frame (reflection)", &frame_layout, &FrameResources { rt_views: rt_scene, ..resources(&reflection_frame_buffer, &shadow_map.view) });
        let reflections = building.step("the reflections' pipelines", || Reflections::new(device, &frame_layout));
        let post = building.step("the post-processing pipelines", || Post::new(device, gpu.view_format, width, height, samples));
        let terrain_detail = building.step("the ground's detail materials", || terrain_detail::TerrainDetail::new(device, &gpu.queue));
        let terrain_layout = TerrainPipeline::bind_group_layout(device);
        let water_scene = WaterScene::new(device, samples);
        // (Where their shaders take long to compile, the water's pipelines are built while the
        // terrain's are: see `util::side_by_side`.)
        let (terrain_pipeline, water_pipeline) = std::thread::scope(|scope| {
            let water = || WaterPipeline::new(device, &frame_layout, &terrain_layout, &reflections.layout, &water_scene.layout, util::HDR_FORMAT, samples);
            let early = util::builds_side_by_side().then(|| std::thread::Builder::new().name("pipeline builder".into()).spawn_scoped(scope, water).ok()).flatten();
            let terrain = building.step("the terrain's pipelines", || TerrainPipeline::new(device, &frame_layout, terrain_layout.clone(), &terrain_detail, util::HDR_FORMAT, samples));
            let water = building.step("the water's pipelines", || match early {
                Some(thread) => thread.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                None => water(),
            });
            (terrain, water)
        });
        let sky_pipeline = building.step("the sky's pipelines", || SkyPipeline::new(device, &frame_layout, util::HDR_FORMAT, samples));
        let sprites = building.step("the sprites' pipelines", || SpriteRenderer::new(device, &frame_layout, util::HDR_FORMAT, samples));
        let foliage = building.step("the trees' and rocks' pipelines", || foliage::FoliageRenderer::new(device, &gpu.queue, &frame_layout, util::HDR_FORMAT, samples).map_err(|e| log::error!("foliage: {e:#}")).ok());
        let beams = building.step("the beams' pipelines", || BeamRenderer::new(device, &frame_layout, util::HDR_FORMAT, samples));
        let rings = building.step("the rings' pipelines", || rings::RingRenderer::new(device, &frame_layout, util::HDR_FORMAT, samples));
        let world_text = building.step("the world text's pipelines", || WorldTextRenderer::new(device, &gpu.queue, &frame_layout, util::HDR_FORMAT, samples));
        let wakes = building.step("the wakes' pipelines", || wakes::WakeRenderer::new(device, &frame_layout));
        let icons = building.step("the icons' pipelines", || icons::IconRenderer::new(device, gpu.view_format));
        let heat = building.step("the heat haze's pipelines", || HeatRenderer::new(device, &frame_layout, util::HDR_FORMAT, samples));
        let particles = building.step("the particles' pipelines", || GpuParticles::new(device, &gpu.queue, &frame_layout, util::HDR_FORMAT, samples, &ground_dummy.view));
        building.done("renderer");

        Ok(Renderer {
            gpu,
            post,
            lighting: Lighting::default(),
            frame_lighting: Lighting::default(),
            frame_daylight: None,
            frame_layout,
            frame_buffer,
            frame_bind_group,
            shadow_pass_bind_group,
            reflection_frame_buffer,
            reflection_bind_group,
            reflections,
            shadow_dummy,
            shadow_sampler,
            los,
            los_sampler,
            los_size: (1, 1),
            los_params: [0.0; 4],
            shadow_map,
            feature_shadow_map,
            shadow_size,
            shadow_view_proj: Mat4::IDENTITY,
            shadow_texel: 1.0,
            shadow_depth: SHADOW_DEPTH_RANGE,
            shadow_extent: 0.0,
            terrain_pipeline,
            terrain_detail,
            terrain: None,
            overlay: MapOverlay::None,
            water_pipeline,
            water_scene,
            water: None,
            sky_pipeline,
            skirt: None,
            map_average: None,
            map_balance_gains: Vec3::ONE,
            world_grade: [[0.0; 4]; 3],
            map_key: terrain_detail::WorldGrade::NONE.key,
            map_balance: [1.0; 3],
            sea_share: 0.0,
            ground_strands: false,
            strands_view: true,
            world_sky: sky::world_sky(""),
            cloud_offset: Vec2::ZERO,
            models: None,
            foliage,
            sprites,
            beams,
            rings,
            world_text,
            wakes,
            icons,
            icon_list: Vec::new(),
            particles,
            lights: Vec::new(),
            light_buffer,
            light_grid_buffer,
            occlusion,
            occluders: Vec::new(),
            sprite_occluders: Vec::new(),
            foliage_occluders: Vec::new(),
            cloud_shadows,
            terrain_shadow,
            ray_tracing,
            rt_storage_warning: false,
            room: None,
            space: None,
            cockpit: None,
            cockpit_frame: None,
            heat_sources: Vec::new(),
            heat,
            timer: None,
            start: Instant::now(),
            frozen_time: None,
            wind_drift: Vec2::ZERO,
            drift_time: None,
            screenshot_request: false,
            last_screenshot: None,
            window_size: (width.max(1), height.max(1)),
            render_scale: 1.0,
            film_size: None,
            colour_blind: false,
            art_style_reborn: true,
        })
    }

    pub fn frame_layout(&self) -> &wgpu::BindGroupLayout {
        &self.frame_layout
    }

    /// Recreates the frame bind groups (after the fog-of-war texture or the map changed).
    fn rebuild_frame_groups(&mut self) {
        let ground_view = &self.terrain_shadow.target.view;
        let resources = |frame_buffer, shadow_view| FrameResources {
            frame_buffer,
            shadow_view,
            shadow_sampler: &self.shadow_sampler,
            light_buffer: &self.light_buffer,
            light_grid_buffer: &self.light_grid_buffer,
            los_view: &self.los.view,
            los_sampler: &self.los_sampler,
            occlusion_view: &self.occlusion.target.view,
            cloud_view: &self.cloud_shadows.target.view,
            ground_view,
            rt_views: None,
        };
        let device = &self.gpu.device;
        let features_view = self.feature_shadow_map.as_ref().map_or(&self.shadow_dummy.view, |m| &m.view);
        let rt_scene = self.ray_tracing.as_ref().map(|r| (r.result_view(), r.reflection_view(), features_view));
        let rt_shadow = self.ray_tracing.as_ref().map(|r| (r.result_view(), r.reflection_view(), &self.shadow_dummy.view));
        let frame = frame_group(device, "frame", &self.frame_layout, &FrameResources { rt_views: rt_scene, ..resources(&self.frame_buffer, &self.shadow_map.view) });
        let shadow = frame_group(device, "frame (shadow pass)", &self.frame_layout, &FrameResources { rt_views: rt_shadow, ..resources(&self.frame_buffer, &self.shadow_dummy.view) });
        let reflection = frame_group(device, "frame (reflection)", &self.frame_layout, &FrameResources { rt_views: rt_scene, ..resources(&self.reflection_frame_buffer, &self.shadow_map.view) });
        self.frame_bind_group = frame;
        self.shadow_pass_bind_group = shadow;
        self.reflection_bind_group = reflection;
    }

    /// Whether this device can trace the loaded model library. Ray-traced shadows and
    /// ambient occlusion are drawn while [`Lighting::ray_tracing`] is on (see [`raytrace`]).
    pub fn ray_tracing_available(&self) -> bool {
        self.ray_tracing.is_some() && self.ray_trace_model_storage_fits()
    }

    fn ray_trace_model_storage_fits(&self) -> bool {
        self.models.as_ref().and_then(|m| m.library.buffers()).is_none_or(|(vertices, indices)| {
            ray_trace_buffer_sizes_fit(vertices.size(), indices.size(), self.gpu.device.limits().max_storage_buffer_binding_size)
        })
    }

    /// Whether this frame's shadows and ambient occlusion are ray traced (perspective camera,
    /// ray tracing on and available, models to trace), with its targets at the scene's size.
    fn prepare_ray_tracing(&mut self, camera: &Camera) -> bool {
        let wanted = camera.mode == CameraMode::Perspective && self.lighting.ray_tracing && self.models.as_ref().is_some_and(|m| m.traced_pieces().next().is_some());
        if wanted && self.ray_tracing.is_some() && !self.ray_trace_model_storage_fits() {
            if !self.rt_storage_warning {
                log::warn!("ray tracing: loaded model buffers exceed this GPU's storage binding limit; using raster shadows and reflections with the same artwork");
                self.rt_storage_warning = true;
            }
            return false;
        }
        let size = self.post.size();
        let Some(rt) = self.ray_tracing.as_mut().filter(|_| wanted) else { return false };
        if rt.ensure_targets(&self.gpu.device, size) {
            self.rebuild_frame_groups();
        }
        true
    }

    /// Shows a map overlay on the ground (or none).
    pub fn set_overlay(&mut self, overlay: MapOverlay) {
        self.overlay = overlay;
    }

    /// Units in colour-blind-safe player colours: TA's team textures give way to the white
    /// team's shading tinted with the colour-blind table (`unit.wgsl`), and the build and
    /// placement glows turn blue and orange.
    pub fn set_colour_blind(&mut self, on: bool) {
        self.colour_blind = on;
    }

    /// Selects Reborn artwork (`true`) or Enhanced Classic (`false`) without changing
    /// the simulation, camera, or graphics quality. Applies to models and world surfaces.
    pub fn set_art_style(&mut self, reborn: bool) {
        self.art_style_reborn = reborn;
        if let Some(models) = &mut self.models {
            models.set_art_style(reborn);
        }
        if let Some(t) = self.terrain.as_mut() {
            t.set_strands(&self.gpu.queue, reborn && self.ground_strands && self.strands_view);
        }
    }

    /// Sets (or with `None` clears) the fog of war: every lit surface is shown by the state of
    /// its visibility cell (in sight, explored, in radar range), filtered bilinearly and shaped
    /// into soft edges. Call whenever the viewer's visibility changes, or the fog's fades move on;
    /// uploads are small (four bytes a cell).
    pub fn set_fog_of_war(&mut self, fog: Option<FogOfWar<'_>>) {
        let Some(fog) = fog.filter(|f| f.width > 0 && f.height > 0 && f.texels.len() >= (f.width * f.height * 4) as usize) else {
            self.los_params = [0.0; 4];
            return;
        };
        if self.los_size != (fog.width, fog.height) {
            self.los = los_texture(&self.gpu.device, fog.width, fog.height);
            self.los_size = (fog.width, fog.height);
            self.rebuild_frame_groups();
        }
        let texels = &fog.texels[..(fog.width * fog.height * 4) as usize];
        util::write_texture_level(&self.gpu.queue, &self.los.texture, 0, fog.width, fog.height, texels, 4);
        let extent = (fog.width as f32 * fog.cell, fog.height as f32 * fog.cell);
        let sum: f32 = texels.chunks_exact(4).map(|c| fog_brightness(c[0] as f32 / 255.0, c[1] as f32 / 255.0)).sum();
        let mean = sum / (texels.len() / 4) as f32;
        self.los_params = [1.0 / extent.0.max(1.0), 1.0 / extent.1.max(1.0), 1.0, mean];
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.gpu.resize(width, height);
        self.window_size = (width, height);
        self.resize_scene();
    }

    /// Draws the scene at `scale` (0.5 to 1) of the window's resolution, scaled up when it
    /// is shown; cheaper on weak graphics. What is drawn over the frame afterwards (the
    /// interface) keeps the full resolution.
    pub fn set_render_scale(&mut self, scale: f32) {
        let scale = scale.clamp(0.5, 1.0);
        if (scale - self.render_scale).abs() > 1e-3 {
            self.render_scale = scale;
            self.resize_scene();
        }
    }

    pub fn render_scale(&self) -> f32 {
        self.render_scale
    }

    /// Antialiasing samples the scene is drawn with.
    pub fn msaa_samples(&self) -> u32 {
        self.post.samples
    }

    fn resize_scene(&mut self) {
        if let Some((w, h)) = self.film_size {
            self.post.resize(&self.gpu.device, w, h);
            return;
        }
        let (w, h) = self.window_size;
        let scaled = |v: u32| ((v as f32 * self.render_scale).round() as u32).max(1);
        self.post.resize(&self.gpu.device, scaled(w), scaled(h));
    }

    /// Uploads a map's terrain. Maps with a sea level above 0 and ground below it also get a
    /// water surface (see [`WaterSettings`] in [`Lighting::water`] for its appearance). The
    /// ground beyond the map edges and the sky's tint are derived here too (see [`sky`]), and
    /// the terrain's ambient occlusion is baked (see [`occlusion`]). The GPU particles of the
    /// previous map are cleared.
    /// Whether this graphics card can draw `map`'s ground (see [`TerrainGpu::fits`]); the
    /// error says what is too big for it.
    pub fn map_fits(&self, map: &ta_formats::tnt::TntMap) -> Result<(), String> {
        TerrainGpu::fits(map, self.gpu.device.limits().max_texture_dimension_2d)
    }

    /// The terrain's heights were changed in place (see [`TerrainGpu::update_heights`]): the
    /// ground map every pass reads (the heights decals and lying features are draped over, the
    /// relief's shadows) is filled again.
    pub fn heights_changed(&mut self) {
        if let Some(terrain) = &self.terrain {
            self.terrain_shadow.heights_changed(terrain.field.highest());
        }
    }

    /// Levels the drawn ground under buildings as they are placed (see `terrain_pads.rs`): each
    /// pad's footprint drawn at its building's base, the ground round it banked away to its own,
    /// for good (a pad stays after its building is gone). Only the ground the new pads reach
    /// changes, everywhere it is drawn and read.
    pub fn add_ground_pads(&mut self, pads: &[GroundPad]) {
        if pads.is_empty() {
            return;
        }
        let Some(t) = self.terrain.as_mut() else { return };
        if let Some(change) = t.add_pads(&self.gpu.queue, pads) {
            self.terrain_shadow.heights_changed_in(change.area, change.top);
        }
    }

    /// The game went back to `tick` (the battle's highlights): the pads laid since are taken up.
    pub fn forget_ground_pads_after(&mut self, tick: u64) {
        let Some(t) = self.terrain.as_mut() else { return };
        if let Some(change) = t.forget_pads_after(&self.gpu.queue, tick) {
            self.terrain_shadow.heights_changed_in(change.area, change.top);
        }
    }

    pub fn load_map(&mut self, map: &ta_formats::tnt::TntMap, palette: &ta_formats::image::Palette) {
        // (Loaded all the same: the textures that don't fit are left empty, which the log says.)
        if let Err(e) = self.map_fits(map) {
            log::error!("{e}");
        }
        let terrain = TerrainGpu::new(&self.gpu.device, &self.gpu.queue, &self.terrain_pipeline, map, palette);
        self.water = WaterSurface::new(&self.gpu.device, &terrain.field, map, palette);
        // (No wakes of the last map's ships until the game gives this one's.)
        self.wakes.prepare(&self.gpu.device, &self.gpu.queue, &[], &[]);
        self.map_average = Some(Vec3::from(terrain.average_color));
        (self.map_key, self.map_balance) = (terrain.land_key, terrain.land_balance);
        // (FNV-1a over the map's size and heights.)
        let seed = map.heights.iter().step_by(97).fold(0xcbf2_9ce4_8422_2325u64 ^ ((map.width as u64) << 32 | map.height as u64), |h, &v| (h ^ v as u64).wrapping_mul(0x0100_0000_01b3));
        self.cloud_offset = Vec2::new((seed % 100_003) as f32, ((seed >> 24) % 100_019) as f32);
        self.particles.set_ground(&self.gpu.device, &terrain.height_view, Some(&terrain.tile_colors));
        let highest = terrain.field.highest();
        self.terrain_shadow.set_map(&self.gpu.device, &terrain.height_view, (terrain.field.width, terrain.field.height), highest);
        // How much sea lies around each cell: on lava worlds the melt's glow (`lava_light`).
        let field = &terrain.field;
        self.sea_share = field.heights.iter().filter(|&&h| (h as f32) < field.sea_level - 1.0).count() as f32 / field.heights.len().max(1) as f32;
        if field.sea_level > 0.0 {
            let near = terrain_shadow::lava_nearness(field.width, field.height, &field.heights, field.sea_level);
            self.terrain_shadow.set_sea_near(&self.gpu.device, &self.gpu.queue, field.width, field.height, &near);
        }
        self.skirt = None;
        self.terrain = Some(terrain);
        self.rebuild_frame_groups();
        self.update_skirt();
    }

    /// The map's metal deposits: how rich each heightmap cell is (0 = plain ground, 255 = the
    /// richest), `width` × `height` cells row by row. With the perspective camera they glint in
    /// the rock. Call after [`Renderer::load_map`].
    pub fn set_ore(&mut self, width: u32, height: u32, ore: &[u8]) {
        self.terrain_shadow.set_ore(&self.gpu.device, &self.gpu.queue, width, height, ore);
    }

    /// (Re)builds the ground beyond the map edges when the map or its reach changed.
    fn update_skirt(&mut self) {
        let Some(t) = &self.terrain else {
            self.skirt = None;
            return;
        };
        let size = t.field.size_px();
        // (In space nothing lies beyond the map.)
        let reach = if self.world_sky.space > 0.0 { 0.0 } else { self.lighting.sky.horizon_extent.clamp(0.0, 1.0e6) };
        if self.skirt.as_ref().is_some_and(|s| s.map_size == size && s.reach == reach) {
            return;
        }
        self.skirt = (reach > 0.0).then(|| SkirtMesh::new(&self.gpu.device, size, reach));
    }

    /// Whether this frame draws the sky and the ground beyond the map.
    fn sky_visible(&self, camera: &Camera) -> bool {
        camera.mode == CameraMode::Perspective && self.lighting.sky.enabled
    }

    /// The map's own light at noon: the lighting, and the sky's colours for it.
    fn noon(&self, camera: &Camera) -> daylight::Noon {
        let l = &self.lighting;
        let (horizon, zenith) = if self.sky_visible(camera) {
            let atmosphere = self.ground_average().map(|average| MapAtmosphere { average, fluid: self.fluid() });
            sky::sky_colors(&l.sky, l.fog_color, l.ambient, atmosphere, &self.world_sky)
        } else {
            (l.fog_color, Vec3::ZERO)
        };
        daylight::Noon {
            sun_dir: l.sun_dir,
            sun_color: l.sun_color,
            sun_intensity: l.sun_intensity,
            ambient: l.ambient,
            horizon,
            zenith,
            shadow: l.shadow_strength,
            moon_dir: l.moon_dir,
        }
    }

    /// The light of the time of day for this frame: set (see [`Lighting::time_of_day`]) and
    /// seen through the perspective camera; the classic view keeps TA's own light.
    pub fn daylight(&self, camera: &Camera) -> Option<daylight::Daylight> {
        let hours = self.lighting.time_of_day?;
        (camera.mode == CameraMode::Perspective).then(|| daylight::at(hours, &self.noon(camera)))
    }

    /// Sets this frame's light (see [`Renderer::frame_lighting`]).
    fn begin_frame_lighting(&mut self, camera: &Camera) {
        let day = self.daylight(camera);
        let mut l = self.lighting.clone();
        if let Some(d) = &day {
            l.sun_dir = d.light_dir;
            l.sun_color = d.light_color;
            l.sun_intensity = d.light_intensity;
            l.ambient = d.ambient;
            l.shadow_strength = d.shadow;
        }
        self.frame_lighting = l;
        self.frame_daylight = day;
    }

    /// Replaces this frame's dynamic point lights (explosions, muzzle flashes, laser glow,
    /// burning wrecks). Call once per frame before [`Renderer::render`]; the lights persist
    /// until the next call, so pass an empty slice to clear them.
    ///
    /// Lights that cannot reach the visible ground are culled at render time; of the rest at
    /// most [`MAX_LIGHTS`] are used (the brightest and largest win). Colours are linear HDR
    /// and may exceed 1; the contribution falls smoothly to zero at each light's radius.
    /// Animate brightness on the caller's side (fade explosions out over their lifetime,
    /// flicker fires) by scaling `color`.
    ///
    /// ```ignore
    /// // An explosion 0.3 s into its 0.6 s life, 20 units above the ground.
    /// let fade = 1.0 - 0.3 / 0.6;
    /// let fireball = PointLight::new([x, ground + 20.0, z], [6.0 * fade, 3.0 * fade, 1.0 * fade], 150.0);
    /// renderer.set_lights(&[fireball]);
    /// ```
    ///
    /// Suggested starting points (colour, radius): explosion `[6.0, 3.0, 1.0]`, 120-250
    /// (scale with the weapon's area of effect); muzzle flash `[5.0, 3.5, 1.5]`, 60-120 for
    /// 2-3 frames; laser glow `beam colour × 1.5`, 50-80 at a few points along the beam;
    /// burning wreck `[2.5, 1.0, 0.25]` with flicker, 60-100.
    pub fn set_lights(&mut self, lights: &[PointLight]) {
        self.lights.clear();
        self.lights.extend_from_slice(lights);
    }

    /// This frame's point lights as last passed to [`Renderer::set_lights`].
    pub fn lights(&self) -> &[PointLight] {
        &self.lights
    }

    /// The liquid below sea level on the current map (`None` without water), honouring
    /// [`WaterSettings::fluid`].
    pub fn fluid(&self) -> Option<Fluid> {
        self.water.as_ref().map(|w| w.fluid(&self.lighting.water))
    }

    /// Whether a water (or lava) surface is drawn this frame.
    fn water_visible(&self) -> bool {
        self.lighting.water.enabled && self.fluid().is_some_and(|f| f != Fluid::Void)
    }

    /// Creates the model renderer from the game's texture GAFs (`textures/*.gaf`).
    pub fn init_models(&mut self, gafs: &[ta_formats::gaf::Gaf], palette: &ta_formats::image::Palette) {
        self.init_models_with_skin(gafs, palette, None);
    }

    /// Loads optional local cosmetic paint once alongside the unit material libraries.
    pub fn init_models_with_skin(&mut self, gafs: &[ta_formats::gaf::Gaf], palette: &ta_formats::image::Palette, skin: Option<&ta_skins::Pack>) {
        self.init_models_with_skins(gafs, palette, skin, skin, None);
    }

    /// Both viewer-relative cosmetic choices are uploaded once. They never affect game data.
    /// With `used`, only those textures (lower case: [`model::drawn_textures`] of every model
    /// that may be drawn) are loaded.
    pub fn init_models_with_skins(&mut self, gafs: &[ta_formats::gaf::Gaf], palette: &ta_formats::image::Palette, skin: Option<&ta_skins::Pack>, enemy_skin: Option<&ta_skins::Pack>, used: Option<&std::collections::HashSet<String>>) {
        let textures = TextureLibrary::new_with_skins(&self.gpu.device, &self.gpu.queue, gafs, palette, skin, enemy_skin, used);
        let mut building = Building::new();
        self.models = Some(building.step("the units' pipelines", || ModelRenderer::new(&self.gpu.device, &self.frame_layout, util::HDR_FORMAT, self.post.samples, textures)));
        building.done("the units' pipelines");
    }

    /// Submits this frame's model instances. Their contact occlusion (perspective camera) is
    /// derived from them: every whole model that is not cloaked occludes the ground around its
    /// footprint, units under construction in proportion to their progress (placement ghosts,
    /// at zero progress, not at all), flying ones not at all; debris pieces never.
    pub fn prepare_models<'a>(&mut self, instances: impl IntoIterator<Item = DrawInstance<'a>>) {
        if let Some(m) = &mut self.models {
            m.set_art_style(self.art_style_reborn);
            m.prepare(&self.gpu.device, &self.gpu.queue, instances, None, self.lighting.ray_tracing);
        }
    }

    /// Submits models in the selected art style. Projected size controls geometry detail;
    /// changing camera or performance settings does not change the chosen art direction.
    pub fn prepare_models_for_view<'a>(&mut self, camera: &Camera, instances: impl IntoIterator<Item = DrawInstance<'a>>) {
        if let Some(m) = &mut self.models {
            m.set_art_style(self.art_style_reborn);
            let traced = camera.mode == CameraMode::Perspective && self.lighting.ray_tracing;
            m.prepare(&self.gpu.device, &self.gpu.queue, instances, Some((camera, self.lighting.hd_surfaces > 0.0)), traced);
        }
    }

    /// Submits this frame's 3D trees, bushes and rocks (see [`foliage`]): model, level of detail
    /// and instance; and with `sight` (a point and a radius), they give way round the line from
    /// the eye to that point, keeping a unit followed from behind in sight.
    pub fn prepare_foliage(&mut self, list: &[(foliage::FoliageModelId, f32, foliage::FoliageInstance)], sight: Option<(Vec3, f32)>) {
        self.foliage_occluders.clear();
        if let Some(f) = &mut self.foliage {
            f.prepare(&self.gpu.device, &self.gpu.queue, list, sight, self.lighting.soft_shadows > 0.0);
            // The ground darkens where a rock meets it (a boulder filling its painting's width
            // hides the sprite's contact blob under itself; it looked set down on the sand).
            for (model, _, inst) in list {
                let Some(m) = f.library.models.get(*model as usize).filter(|m| m.contact > 0.0 && inst.fade > 0.0) else { continue };
                // (A vehicle's along its length and breadth, a rock's round.)
                let half = if m.length > 0.0 { [m.length * 0.5 * inst.scale[0], m.breadth * 0.5 * inst.scale[2]] } else { [m.contact * inst.scale[0]; 2] };
                self.foliage_occluders.extend(occlusion::contact_occluder(inst.pos, half, inst.yaw, ROCK_OCCLUSION * inst.fade));
            }
        }
    }

    /// Submits this frame's sprites: `lying`, the features painted lying (see
    /// [`sprites::painted_lying`]) laid on the ground under the perspective camera. Billboards
    /// (trees, rocks) also occlude the ground around their base a little (perspective camera).
    pub fn prepare_sprites(&mut self, decals: &[SpriteInstance], billboards: &[SpriteInstance], effects: &[SpriteInstance], smoke: &[SpriteInstance], lying: &[SpriteInstance]) {
        self.sprites.prepare(&self.gpu.device, &self.gpu.queue, decals, billboards, effects, smoke, lying);
        self.sprite_occluders.clear();
        for b in billboards {
            let Some(width) = self.sprites.atlas.frame_width(b.frame) else { continue };
            self.sprite_occluders.extend(occlusion::sprite_occluder(b.pos, width * b.params[1], SPRITE_OCCLUSION * b.params[0].clamp(0.0, 1.0)));
        }
    }

    /// Adds a source of hot air to this frame's heat haze (perspective camera; strength set by
    /// [`post::PostSettings::heat_haze`]): a soft column rising from `position`, `radius` wide
    /// and about twice as high, at `strength` (1 = a big explosion's fireball, 0.3 = a
    /// smouldering wreck). Sources last one frame: add them every frame before
    /// [`Renderer::render`], fading them out on the caller's side. Lava seas get their haze
    /// automatically. See [`heat`].
    pub fn add_heat_source(&mut self, position: [f32; 3], radius: f32, strength: f32) {
        self.heat_sources.push(HeatSource { position, radius, strength });
    }

    /// The heat sources added for the coming frame.
    pub fn heat_sources(&self) -> &[HeatSource] {
        &self.heat_sources
    }

    /// Turns GPU timing of the render passes on or off; returns whether the device supports
    /// it (timestamp queries). Read the results with [`Renderer::gpu_timings`].
    pub fn set_gpu_timing(&mut self, enabled: bool) -> bool {
        if !enabled {
            self.timer = None;
            return true;
        }
        if self.timer.is_none() {
            self.timer = GpuTimer::new(&self.gpu.device, &self.gpu.queue);
        }
        self.timer.is_some()
    }

    /// GPU time of each pass of the most recently measured frame (a few frames behind; empty
    /// until [`Renderer::set_gpu_timing`] is on and results arrived). Passes: `shadow`,
    /// `scene` (everything in the HDR scene pass, the particles' draws included), `bloom`,
    /// `composite` (heat haze distortion and tone mapping into the swapchain image; may include
    /// waiting for that image), `particles` (the GPU particles' emission and simulation, in
    /// frames where any are alive), `reflection` (the world mirrored for the water's
    /// reflections, in frames that draw them), `cockpit` (a Perspective view's modelled
    /// cockpit, in frames that draw it), and `frame`: from the start of the shadow pass to the end of
    /// bloom, i.e. all of the frame's rendering except the composite and the UI, including the
    /// small cloud shadow and contact occlusion passes and the MSAA resolve (which the
    /// neighbouring passes may or may not account for). Compare `frame` between settings to
    /// measure what an effect costs.
    pub fn gpu_timings(&self) -> &[PassTime] {
        self.timer.as_ref().map_or(&[], |t| t.latest())
    }

    /// Which frame [`Renderer::gpu_timings`] are of (0: none yet): it changes as new ones come.
    pub fn gpu_timings_frame(&self) -> u64 {
        self.timer.as_ref().map_or(0, |t| t.latest_frame())
    }

    /// Submits this frame's beams (lasers, lightning).
    pub fn prepare_beams(&mut self, beams: &[BeamInstance]) {
        self.beams.prepare(&self.gpu.device, &self.gpu.queue, beams);
    }

    /// This frame's rings on the ground round selected units (see [`rings`]).
    pub fn prepare_rings(&mut self, rings: &[RingInstance]) {
        self.rings.prepare(&self.gpu.device, &self.gpu.queue, rings);
    }

    /// This frame's text and signs in the world (see [`world_text`]).
    pub fn prepare_world_text(&mut self, glyphs: &[GlyphInstance]) {
        self.world_text.prepare(&self.gpu.device, &self.gpu.queue, glyphs);
    }

    /// This frame's wakes behind ships and hovercraft (see [`wakes`]): `trails` over `points`.
    pub fn prepare_wakes(&mut self, points: &[WakePoint], trails: &[WakeTrail]) {
        self.wakes.prepare(&self.gpu.device, &self.gpu.queue, points, trails);
    }

    /// Replaces the strategic icons drawn over the finished image (see [`icons`]); they stay
    /// until the next call (an empty slice clears them).
    pub fn set_icons(&mut self, icons: &[icons::IconInstance]) {
        self.icon_list.clear();
        self.icon_list.extend_from_slice(icons);
    }

    /// Queues bursts of GPU particles, emitted by the GPU at the start of the next rendered
    /// frame (see [`gpu_particles`]). Only the burst descriptors are uploaded, however many
    /// particles they ask for; counts are scaled by [`ParticleSettings::density`], and bursts
    /// with non-finite values are dropped. Particles are presentation only.
    ///
    /// ```ignore
    /// // A unit blows up: a shower of sparks, embers drifting up, flames, grit and dust.
    /// renderer.emit_particles(&[
    ///     ParticleBurst::new(ParticleKind::Spark, pos, 300).speed(80.0, 340.0),
    ///     ParticleBurst::new(ParticleKind::Ember, pos, 60).over(0.5),
    ///     ParticleBurst::new(ParticleKind::Fire, pos, 20).radius(12.0),
    ///     ParticleBurst::new(ParticleKind::Dirt, pos, 40),
    ///     ParticleBurst::new(ParticleKind::Dust, ground, 16).ground_color(),
    /// ]);
    /// ```
    pub fn emit_particles(&mut self, bursts: &[ParticleBurst]) {
        self.particles.queue(bursts);
    }

    /// How far (seconds) the GPU particles advance in each coming frame: the frame time times
    /// the game speed, 0 while paused (they then hang still, and are still drawn). Call every
    /// frame.
    pub fn step_particles(&mut self, dt: f32) {
        self.particles.set_step(dt);
    }

    /// Pool size, density and wind of the GPU particles (e.g. from an effects quality setting).
    /// Changing the pool size empties it.
    pub fn set_particle_settings(&mut self, settings: ParticleSettings) {
        self.particles.set_settings(settings);
    }

    pub fn particle_settings(&self) -> ParticleSettings {
        self.particles.settings()
    }

    /// Live GPU particles and those emitted, as measured a few frames ago.
    pub fn particle_stats(&self) -> ParticleStats {
        self.particles.stats()
    }

    /// Removes every GPU particle (loading a map does this too).
    pub fn clear_particles(&mut self) {
        self.particles.clear();
    }

    /// Fits an orthographic sun frustum around the visible ground nearest the camera
    /// (`shadow_fit`), steady while the camera moves: its size is the circle round that ground
    /// (the same however the camera turns), with room for the faded rim, taken in steps of an
    /// eighth of an octave (and made smaller only once two steps too big), so between steps a
    /// shadow texel keeps its size and, its origin snapped to whole texels, its place in the
    /// world. (A size following the view exactly changed the texels every frame the camera
    /// zoomed, turned or followed the ground, and every shadow's edge crawled.)
    fn update_shadow_matrix(&mut self, camera: &Camera) {
        let sun = self.frame_lighting.sun_dir.normalize();
        let seen_from = camera.eye();
        let (center2, reach) = shadow_fit(Vec2::new(seen_from.x, seen_from.z), &camera.outline_points([0.0, 200.0]));
        let needed = (reach + 150.0).max(reach / SHADOW_FADE_FROM).min(SHADOW_EXTENT_MAX);
        self.shadow_extent = steady_shadow_extent(self.shadow_extent, needed);
        let extent = self.shadow_extent;
        let texel = 2.0 * extent / self.shadow_size as f32;
        self.shadow_texel = texel;
        let (back, depth) = shadow_depth(extent, sun.y);
        self.shadow_depth = depth;
        let center = Vec3::new(center2.x, 60.0, center2.y);
        let up = if sun.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        let eye = center + sun * back;
        let view = glam::camera::rh::view::look_at_mat4(eye, center, up);
        // Snap the view origin to whole shadow texels.
        let origin = view.transform_point3(Vec3::ZERO);
        let snapped = Vec3::new((origin.x / texel).round() * texel, (origin.y / texel).round() * texel, origin.z);
        let view = Mat4::from_translation(snapped - origin) * view;
        let proj = glam::camera::rh::proj::directx::orthographic(-extent, extent, -extent, extent, 100.0, 100.0 + self.shadow_depth);
        self.shadow_view_proj = proj * view;
    }

    /// Freezes the animation clock (waves, lava, clouds, construction effects) at `seconds`,
    /// or with `None` lets it run in real time again. For deterministic screenshots.
    pub fn set_animation_time(&mut self, seconds: Option<f32>) {
        self.frozen_time = seconds;
    }

    /// Seconds on the animation clock: real time since the renderer started, taken in f64 and
    /// wrapped at [`ANIMATION_PERIOD`], so the f32 the shaders get stays fine-grained in a
    /// session of any length.
    fn animation_time(&self) -> f32 {
        self.frozen_time.unwrap_or_else(|| (self.start.elapsed().as_secs_f64() % ANIMATION_PERIOD) as f32)
    }

    /// Moves the wind's drift on to `time` on the animation clock: by the wind blowing now
    /// ([`Lighting::wind`]) for the time since it was last moved (a clock that went back, as it
    /// does when it starts again, moves it nothing). What drifts on the wind (the scum on acid,
    /// the gusts over the trees) is laid out from where the drift has got to, so a wind that
    /// changes changes how fast they go, not where they are: laid out along the wind times the
    /// clock, they jumped (the further from the map's corner, the further) at every change.
    fn advance_wind_drift(&mut self, time: f32) {
        let dt = self.drift_time.map_or(0.0, |last| time - last);
        if dt > 0.0 {
            self.wind_drift += self.lighting.wind * dt.min(0.25);
            // (Wound back as the clock is, never coarser than an f32 has at 65,536: one jump in
            // many hours of a gale.)
            if self.wind_drift.abs().max_element() > ANIMATION_PERIOD as f32 {
                self.wind_drift = Vec2::ZERO;
            }
        }
        self.drift_time = Some(time);
    }

    /// Captures the next rendered frame; retrieve it with [`Renderer::take_screenshot`].
    pub fn request_screenshot(&mut self) {
        self.screenshot_request = true;
    }

    pub fn take_screenshot(&mut self) -> Option<Screenshot> {
        self.last_screenshot.take()
    }

    /// Bins this frame's lights for the visible ground and uploads them.
    fn upload_lights(&self, camera: &Camera) -> LightGrid {
        // Ground seen by the camera, widened for tall units and effects near the view edge.
        let (min, max) = camera.ground_bounds(300.0);
        let margin = Vec2::splat(96.0);
        let grid = LightGrid::build(&self.lights, min - margin, max + margin);
        if !grid.lights.is_empty() {
            self.gpu.queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&grid.lights));
            self.gpu.queue.write_buffer(&self.light_grid_buffer, 0, bytemuck::cast_slice(&grid.words));
        }
        grid
    }

    /// Sets up this frame's contact occlusion map, cloud shadow map and heat sources
    /// (perspective camera), and says which of them are drawn.
    fn prepare_effects(&mut self, camera: &Camera) -> FrameEffects {
        let mut fx = FrameEffects::default();
        if camera.mode != CameraMode::Perspective {
            self.heat_sources.clear();
            return fx;
        }
        let l = &self.frame_lighting;
        let ao = &l.ambient_occlusion;
        let focus = Vec2::new(camera.focus.x, camera.focus.z);
        if ao.enabled && ao.contact > 0.0 {
            let strength = ao.contact.min(1.0);
            let field = self.terrain.as_ref().map(|t| &t.field);
            let ground = |p: Vec2| field.map_or(0.0, |f| f.sample(p.x, p.y));
            self.occluders.clear();
            if let Some(models) = &self.models {
                self.occlusion.upload_masks(&self.gpu.queue, models.library.models.iter().map(|m| &m.footprint));
                for &(model, root, build) in &models.occluder_roots {
                    if let Some(info) = models.library.models.get(model as usize) {
                        self.occluders.extend(info.footprint.occluder(&root, ground, strength * build, self.occlusion.mask_uv(model)));
                    }
                }
            }
            self.occluders.extend(self.sprite_occluders.iter().chain(&self.foliage_occluders).map(|o| Occluder { shape: [o.shape[0], o.shape[1] * strength], ..*o }));
            let region = Region::fit(focus, camera.ground_bounds_with(200.0, false));
            fx.contact = self.occlusion.prepare(&self.gpu.device, &self.gpu.queue, region, &mut self.occluders) > 0;
        }
        let sun = l.sun_dir.normalize();
        let cover = self.cloud_cover(camera);
        if self.sky_visible(camera) && l.cloud_shadows > 0.0 && cover > 0.0 && sun.y > 0.02 {
            let drift = l.sky.cloud_speed * self.animation_time();
            self.cloud_shadows.prepare(&self.gpu.queue, focus, sun, drift, cover, self.cloud_offset);
            fx.clouds = true;
        }
        if self.post.settings.effective_heat_haze() > 0.0 {
            fx.heat = true;
            fx.lava_heat = self.water_visible() && self.fluid() == Some(Fluid::Lava);
            self.heat.prepare(&self.gpu.device, &self.gpu.queue, &self.heat_sources);
        }
        self.heat_sources.clear();
        fx
    }

    /// Whether this frame draws the water's reflections (see [`reflection`]); if so, sizes their
    /// target and picks the terrain chunks the mirrored view needs. `view_min`..`view_max` is
    /// the ground the camera sees (as the terrain and water were culled with).
    fn prepare_reflections(&mut self, camera: &Camera, view_min: Vec2, view_max: Vec2) -> bool {
        let (width, height) = self.post.size();
        let Some(size) = self.lighting.water_reflections.target_size(width, height) else {
            self.reflections.release();
            return false;
        };
        // Water only: lava and painted voids mirror nothing.
        if camera.mode != CameraMode::Perspective || !self.water_visible() || !matches!(self.fluid(), Some(Fluid::Water | Fluid::Acid)) {
            return false;
        }
        let sky_on = self.sky_visible(camera);
        let (Some(terrain), Some(water)) = (&mut self.terrain, &self.water) else { return false };
        // Water in view: wet chunks on the map, or a sea that continues beyond it.
        let map = terrain.field.size_px();
        let beyond = view_min.x < 0.0 || view_min.y < 0.0 || view_max.x > map.x || view_max.y > map.y;
        if water.visible_chunks() == 0 && !(beyond && sky_on && self.skirt.is_some() && terrain.sea_beyond) {
            return false;
        }
        // Everything mirrored lies between the sea and as far below it as the tallest ground and
        // units reach above it.
        let sea = terrain.field.sea_level;
        let (min, max) = camera.slab_bounds([2.0 * sea - 260.0, sea], true);
        terrain.cull_reflection(&self.gpu.queue, min, max);
        self.reflections.prepare(&self.gpu.device, size);
        true
    }

    /// How far rain clouds cover the scene (the perspective camera only: the classic view
    /// keeps the map's light).
    fn overcast(&self, camera: &Camera) -> f32 {
        if camera.mode == CameraMode::Perspective { self.lighting.overcast.clamp(0.0, 1.0) } else { 0.0 }
    }

    /// The sky's cloud cover, thickened by rain clouds.
    fn cloud_cover(&self, camera: &Camera) -> f32 {
        // (A world without air has none.)
        if self.world_sky.clouds <= 0.0 {
            return 0.0;
        }
        let cover = (self.lighting.sky.cloud_cover * self.world_sky.clouds).clamp(0.0, 0.85);
        cover + (0.95 - cover).max(0.0) * self.overcast(camera)
    }

    fn write_frame_uniform(&self, camera: &Camera, grid: &LightGrid, fx: FrameEffects, time: f32) {
        let l = &self.frame_lighting;
        // Rain clouds: less sun, softer shadows, a darker haze.
        let overcast = self.overcast(camera);
        let (sun_dim, shadow_dim, haze_dim) = (1.0 - RAIN_DIM * overcast, 1.0 - 0.5 * overcast, 1.0 - 0.3 * overcast);
        let sun = l.sun_dir.normalize();
        let (map_w, map_h, sea) = match &self.terrain {
            Some(t) => {
                let s = t.field.size_px();
                (s.x, s.y, t.field.sea_level)
            }
            None => (0.0, 0.0, 0.0),
        };
        let w = &l.water;
        let water_on = self.water_visible();
        let fluid = self.water.as_ref().map_or(Fluid::Water, |s| s.fluid(w));
        let body = self.water.as_ref().map_or([0.0; 3], |s| s.body_color(w));
        let (shore_water, shore_land) = self.water.as_ref().and_then(|s| s.painted).map_or(([0.0; 4], [0.0; 4]), |p| p.shore_uniforms());
        let lava = fluid == Fluid::Lava;
        let opacity = self.water.as_ref().map_or(w.opacity, |s| s.opacity(w));
        // Acid: murkier and calmer than water (see `Fluid::Acid`).
        let acid = fluid == Fluid::Acid;
        let clarity = if acid { w.clarity.min(20.0) } else { w.clarity };
        let eye = camera.eye();
        let view_proj = camera.view_proj();
        // The perspective sky: horizon haze (the fog colour, tinted per map) and zenith. The
        // classic view keeps the plain fog colour and never reads the sky fields.
        let sky_on = self.sky_visible(camera);
        let s = &l.sky;
        let (fog_color, zenith) = if let Some(d) = &self.frame_daylight {
            (d.horizon * haze_dim, if sky_on { d.zenith * haze_dim } else { Vec3::ZERO })
        } else if sky_on {
            let atmosphere = self.ground_average().map(|average| MapAtmosphere { average, fluid: self.fluid() });
            let (fog, zenith) = sky::sky_colors(s, l.fog_color, l.ambient, atmosphere, &self.world_sky);
            (fog * haze_dim, zenith * haze_dim)
        } else {
            (l.fog_color, Vec3::ZERO)
        };
        // Under rain clouds the haze and the sky above it go a cool, colourless grey (a desert's
        // warm haze, only darkened, hung over the land as a sepia wash; see also `fog` below).
        let rainy = |c: Vec3| c.lerp(Vec3::splat(c.dot(Vec3::new(0.2126, 0.7152, 0.0722))) * Vec3::new(0.94, 0.99, 1.06), 0.65 * overcast);
        let (fog_color, zenith) = (rainy(fog_color), rainy(zenith));
        // (And the light they let through is a little cooler: the land under them is not the
        // sunlit land, darkened, but it keeps its own colour.)
        let cool = Vec3::ONE.lerp(Vec3::new(0.95, 0.985, 1.04), overcast);
        let reach = if self.skirt.is_some() { s.horizon_extent } else { 0.0 };
        let painted = self.water.as_ref().and_then(|w| w.painted);
        let beyond_sea = painted.map_or([0.0; 4], |p| [p.sea[0], p.sea[1], p.sea[2], 1.0]);
        let beyond_shore = painted.and_then(|p| p.shore).map_or([0.0; 4], |c| [c[0], c[1], c[2], 1.0]);
        // The perspective camera's effects (all zero in the classic view).
        let perspective = camera.mode == CameraMode::Perspective;
        let ao = &l.ambient_occlusion;
        let ao_on = perspective && ao.enabled;
        let region = self.occlusion.region;
        let cloud_strength = if fx.clouds { l.cloud_shadows.clamp(0.0, 1.0) * smoothstep(0.02, 0.2, sun.y) } else { 0.0 };
        let centre = self.cloud_shadows.centre;
        let bounce = if perspective { self.ground_bounce(sun) } else { [0.0; 4] };
        let u = FrameUniform {
            view_proj: view_proj.to_cols_array_2d(),
            shadow_view_proj: self.shadow_view_proj.to_cols_array_2d(),
            camera_pos: [eye.x, eye.y, eye.z, if camera.mode == CameraMode::Perspective { 1.0 } else { 0.0 }],
            sun_dir: [sun.x, sun.y, sun.z, l.sun_intensity * sun_dim],
            sun_color: [l.sun_color.x * cool.x, l.sun_color.y * cool.y, l.sun_color.z * cool.z, l.terrain_relight],
            // (An overcast sky dims the sky's light as much as the sun's: the painted ground,
            // lit by the sun alone, darkens by that much, and so must what stands on it.)
            ambient: [l.ambient.x * sun_dim * cool.x, l.ambient.y * sun_dim * cool.y, l.ambient.z * sun_dim * cool.z, 1.0],
            world: [time, sea, map_w, map_h],
            // The scene's own resolution (smaller than the window's at a render scale).
            screen: [self.post.size().0 as f32, self.post.size().1 as f32, camera.zoom, if self.models.is_some() { l.shadow_strength * shadow_dim } else { 0.0 }],
            // (Rain thickens the haze: the land further off greys into it.)
            fog: [fog_color.x, fog_color.y, fog_color.z, l.fog_density * if sky_on { self.world_sky.haze } else { 1.0 } * (1.0 + overcast)],
            billboard_right: camera.billboard_basis().0.extend(0.0).to_array(),
            billboard_up: camera.billboard_basis().1.extend(0.0).to_array(),
            water_color: [body[0], body[1], body[2], 1.0 / clarity.max(1.0)],
            water: [
                w.wave_strength.max(0.0) * if acid { 0.7 } else { 1.0 },
                // (Acid froths thinly: a little scum where it laps at the shore.)
                if lava { 0.0 } else { w.foam.max(0.0) * if acid { 0.55 } else { 1.0 } },
                if acid { opacity.max(0.65) } else { opacity }.clamp(0.0, 1.0),
                if water_on { 1.0 } else { 0.0 },
            ],
            water_extra: [
                if lava { 0.0 } else { w.caustics.max(0.0) },
                if lava { 1.0 } else { 0.0 },
                if fx.reflections { 1.0 } else { 0.0 },
                w.reflectivity.clamp(0.0, 1.0),
            ],
            shore_water,
            shore_land,
            light_grid_origin: [grid.origin.x, grid.origin.y, if grid.cell > 0.0 { 1.0 / grid.cell } else { 0.0 }, 0.0],
            light_grid_dims: [grid.dims.x, grid.dims.y, grid.lights.len() as u32, 0],
            los: [self.los_params[0], self.los_params[1], self.los_params[2], camera.distance() + 900.0],
            sky_zenith: [zenith.x, zenith.y, zenith.z, self.cloud_cover(camera)],
            sky: [
                s.sun_size.clamp(0.0, 20.0).to_radians(),
                s.cloud_speed,
                camera.clip_range().1,
                if sky_on { 1.0 } else { 0.0 },
            ],
            ground_beyond: [reach, self.los_params[3], self.cloud_offset.x, self.cloud_offset.y],
            beyond_sea,
            beyond_shore,
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            ao: [
                if ao_on { ao.terrain.clamp(0.0, 1.0) } else { 0.0 },
                if fx.contact { 1.0 } else { 0.0 },
                if ao_on { ao.models.clamp(0.0, 1.0) } else { 0.0 },
                if self.art_style_reborn { 1.0 } else { 0.0 },
            ],
            ao_map: [region.origin.x, region.origin.y, 1.0 / region.size, 0.0],
            clouds: [cloud_strength, clouds::SCALE, centre.x, centre.y],
            heat: [
                if fx.heat { self.post.settings.effective_heat_haze() } else { 0.0 },
                if perspective { l.soft_smoke.max(0.0) } else { 0.0 },
                // (The melt's glow from afar: as much as the map has melt, a thin river's
                // already lighting the pall over it.)
                if perspective && lava { MELT_GLOW * smoothstep(0.0, 0.06, self.sea_share) } else { 0.0 },
                0.0,
            ],
            ground_bounce: bounce,
            // Surface quality is independent of the camera and selected art style.
            detail: {
                let hd = l.hd_surfaces.clamp(0.0, 1.0);
                let overlay = match self.overlay {
                    MapOverlay::None => 0.0,
                    MapOverlay::Cells => 1.0,
                    MapOverlay::Height => 2.0,
                };
                [hd, hd, overlay, if self.colour_blind { 1.0 } else { 0.0 }]
            },
            // The time of day on the painted ground and in the sky (TA's own light without).
            daylight: self.frame_daylight.map_or([cool.x, cool.y, cool.z, 0.0], |d| [d.ground_light.x * cool.x, d.ground_light.y * cool.y, d.ground_light.z * cool.z, d.dusk]),
            night: self.frame_daylight.map_or([0.0; 4], |d| [d.ground_skylight.x, d.ground_skylight.y, d.ground_skylight.z, d.night]),
            celestial: {
                let s = self.frame_daylight.map_or(sun, |d| d.sun_dir);
                [s.x, s.y, s.z, if sky_on { self.world_sky.airless } else { 0.0 }]
            },
            terrain_shadow: self.terrain_shadow.uniform(if perspective && self.terrain.is_some() { l.terrain_shadows.clamp(0.0, 1.0) * shadow_dim } else { 0.0 }),
            wind: [l.wind.x, l.wind.y, if perspective { l.foliage_sway.clamp(0.0, 2.0) } else { 0.0 }, 0.0],
            rain: if perspective {
                [l.wetness.clamp(0.0, 1.0), l.rain.clamp(0.0, 1.0), ground_life_strength(l.ground_life, self.post.settings.reduced_motion), 0.0]
            } else {
                [0.0; 4]
            },
            shadow_soft: [
                if perspective { l.soft_shadows.clamp(0.0, 0.5) } else { 0.0 },
                SHADOW_SEARCH_TEXELS,
                self.shadow_depth,
                self.shadow_texel,
            ],
            rt: if fx.ray_traced {
                let (near, far) = camera.clip_range();
                [1.0, near, far, 0.0]
            } else {
                [0.0; 4]
            },
            liquid: [if acid { 1.0 } else { 0.0 }, if sky_on && self.world_sky.space > 0.0 { 1.0 } else { 0.0 }, 0.0, 0.0],
            world_grade: self.world_grade,
            drift: [self.wind_drift.x, self.wind_drift.y, 0.0, 0.0],
        };
        self.gpu.queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&u));
        if fx.reflections {
            // The reflection pass sees the world through the mirror: the mirrored view, the eye
            // below the water (view directions, highlights and haze follow the reflected rays),
            // and billboards turned over (see reflection.rs).
            let mirror = reflection::mirror(sea);
            let view_proj = view_proj * mirror;
            let eye = mirror.transform_point3(eye);
            let up = reflection::mirrored_billboard_up(camera.billboard_basis().1);
            let mirrored = FrameUniform {
                view_proj: view_proj.to_cols_array_2d(),
                inv_view_proj: view_proj.inverse().to_cols_array_2d(),
                camera_pos: [eye.x, eye.y, eye.z, 1.0],
                billboard_up: up.extend(0.0).to_array(),
                // (Ray tracing traced the camera's view, not the mirror's.)
                rt: [0.0; 4],
                ..u
            };
            self.gpu.queue.write_buffer(&self.reflection_frame_buffer, 0, bytemuck::bytes_of(&mirrored));
        }
    }

    /// The reflection pass: the world above the water seen through the mirror, for the water's
    /// reflections (see [`reflection`]).
    fn draw_reflection(&self, encoder: &mut wgpu::CommandEncoder, sky_on: bool, timestamps: Option<&timing::PassTimestamps>) {
        let Some((color, depth)) = self.reflections.attachments() else { return };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("reflection"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                resolve_target: None,
                // Transparent: where nothing is mirrored the water reflects the sky.
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: timestamps.map(|t| t.writes()),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.reflection_bind_group, &[]);
        // (What stands on the ground first, the ground after it, as in the scene pass.)
        if let Some(m) = &self.models {
            m.draw_reflection(&mut pass);
        }
        if let Some(f) = &self.foliage {
            f.draw_reflection(&mut pass);
        }
        self.sprites.draw_reflection(&mut pass, SpriteLayer::Billboard);
        if let Some(t) = &self.terrain {
            t.draw_reflection(&mut pass, &self.terrain_pipeline);
            if let (Some(skirt), true) = (&self.skirt, sky_on) {
                pass.set_bind_group(2, &self.terrain_pipeline.detail, &[]);
                skirt.draw(&mut pass, &self.terrain_pipeline.skirt_reflection, t.bind_group());
            }
        }
        self.sprites.draw_reflection(&mut pass, SpriteLayer::Smoke);
        self.particles.draw_reflection(&mut pass, ParticleLayer::Alpha);
        self.sprites.draw_reflection(&mut pass, SpriteLayer::Effect);
        self.particles.draw_reflection(&mut pass, ParticleLayer::Additive);
        self.beams.draw_reflection(&mut pass);
    }

    /// The map's mean colour, in the balance its world's grade gives the ground.
    fn ground_average(&self) -> Option<Vec3> {
        self.map_average.map(|a| a * self.map_balance_gains)
    }

    /// Light bounced onto units from the painted ground below them (rgb) and how much the sky
    /// and ground colours replace the plain ambient (a), for [`Lighting::sky_light`].
    fn ground_bounce(&self, sun: Vec3) -> [f32; 4] {
        let l = &self.frame_lighting;
        let k = l.sky_light.clamp(0.0, 1.0);
        let Some(average) = self.ground_average().filter(|_| k > 0.0) else { return [0.0; 4] };
        // The ground's mean albedo (its painted brightness) lit by the sun and the sky.
        let irradiance = l.sun_color * l.sun_intensity * sun.y.max(0.0) + l.ambient;
        let c = average * irradiance;
        [c.x, c.y, c.z, k]
    }

    /// Renders one frame. `ui` records overlay passes onto the final swapchain image (given as
    /// a non-sRGB view).
    pub fn render(
        &mut self,
        camera: &Camera,
        ui: impl FnOnce(&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder, &wgpu::TextureView),
    ) {
        let Some(frame) = self.gpu.acquire() else {
            self.skip_frame();
            return;
        };
        let shot = std::mem::take(&mut self.screenshot_request);
        // Where ray tracing may build new pieces (they are first seen, and built, in the frame),
        // or the relief's shadows are filled, the frame is checked: refused (out of memory),
        // what it made is made again with the next.
        let filling = self.terrain_shadow.will_fill();
        let checked = self.ray_tracing.is_some() || filling;
        let scopes = checked.then(|| (self.gpu.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory), self.gpu.device.push_error_scope(wgpu::ErrorFilter::Validation)));
        let pixels = self.draw(camera, &frame.texture, Some(ui), shot, true);
        if let Some((memory, invalid)) = scopes {
            let failed = [pollster::block_on(invalid.pop()), pollster::block_on(memory.pop())];
            if let Some(e) = failed.iter().flatten().next() {
                log::error!("frame failed: {e}");
                self.forget_frame(filling);
            }
        }
        if shot {
            self.last_screenshot = pixels;
        }
        self.gpu.present(frame);
    }

    /// A frame failed (its submission refused): what it built (and, `filled`, the relief's
    /// shadows it filled) is made again.
    fn forget_frame(&mut self, filled: bool) {
        if let Some(rt) = &mut self.ray_tracing {
            rt.forget_builds();
        }
        if filled {
            self.terrain_shadow.forget();
        }
    }

    /// While a replay is made into a film (`Some` of its pictures' size, which every picture is
    /// drawn at with [`Renderer::render_image_with`]), the scene's targets are kept that size
    /// instead of being made again for the window between pictures; `None` gives them back to
    /// the window.
    pub fn set_film_size(&mut self, size: Option<(u32, u32)>) {
        self.film_size = size;
        self.resize_scene();
    }

    /// A frame of interface alone (`ui`, as for [`Renderer::render`]) over black, without the
    /// scene: what the window shows while a replay is made into a film, whose pictures are drawn
    /// on their own ([`Renderer::render_image_with`]).
    pub fn render_ui(&mut self, ui: impl FnOnce(&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder, &wgpu::TextureView)) {
        let Some(frame) = self.gpu.acquire() else {
            self.skip_frame();
            return;
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("interface alone") });
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("black"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        ui(&self.gpu.device, &self.gpu.queue, &mut encoder, &view);
        let pending = std::mem::take(&mut self.screenshot_request).then(|| self.encode_readback(&mut encoder, &frame.texture));
        self.gpu.queue.submit(Some(encoder.finish()));
        if let Some((buffer, w, h, padded)) = pending {
            self.last_screenshot = self.finish_readback(buffer, w, h, padded);
        }
        self.gpu.present(frame);
    }

    /// A screenshot was asked for and not yet taken (the frame that takes it must be drawn).
    pub fn screenshot_pending(&self) -> bool {
        self.screenshot_request
    }

    /// For a frame that is not drawn (no swapchain image to draw into, a capture running on
    /// ahead of its shot): lets go of what was prepared for it. The `prepare_*` calls upload as
    /// they go, and each upload waits on the queue, in staging memory of its own, until
    /// something is submitted: undrawn, a battle of 20,000 units held 20 GB of them after half
    /// a minute, the frame drawn after could not make its ray-traced scene and every frame came
    /// out black.
    pub fn skip_frame(&mut self) {
        self.heat_sources.clear();
        self.particles.drop_queued();
        // (Nothing to draw: the submission carries the uploads alone, and frees the memory of
        // those the card is done with.)
        self.gpu.queue.submit(std::iter::empty());
    }

    /// Puts up the cockpit a Perspective view is seen from (see [`cockpit`]), or takes it down.
    pub fn set_cockpit(&mut self, model: Option<&cockpit::CockpitModel>) {
        self.cockpit = model.map(|m| cockpit::CockpitRenderer::new(&self.gpu.device, &self.gpu.queue, m, self.post.samples));
        self.cockpit_frame = None;
    }

    /// How the cockpit is this frame; none hides it.
    pub fn set_cockpit_frame(&mut self, frame: Option<cockpit::CockpitFrame>) {
        self.cockpit_frame = frame;
    }

    /// The map's world (its OTA's `planet`): which detail materials its ground may be, its look,
    /// its grade and its sky; `painted_by_ta`: whether its ground is TA's painting, whose
    /// hatching may be painted over (not TA Reborn's own maps, nor the editor's tiles).
    pub fn set_ground_world(&mut self, planet: &str, painted_by_ta: bool) {
        // (The sky is tinted, and units lit from below, by the ground as its grade balances it.)
        let (gains, grade) = self.terrain_detail.set_world(&self.gpu.queue, planet, self.map_key, self.map_balance);
        (self.map_balance_gains, self.world_grade) = (Vec3::from(gains), grade);
        // (Metal and urban worlds' walls and terraces are built square: drawn less smoothed.)
        let (_, built) = terrain_detail::world_weights(planet);
        // (`TA_STRANDS=0` leaves them as painted, to compare.)
        self.ground_strands = painted_by_ta && terrain_detail::world_strands(planet) && !std::env::var("TA_STRANDS").is_ok_and(|v| v == "0");
        self.world_sky = sky::world_sky(planet);
        // (A world in space: its void cells drawn as nothing, the sky showing through.)
        let space = self.world_sky.space > 0.0;
        if let Some(t) = self.terrain.as_mut() {
            t.set_built(&self.gpu.queue, built);
            t.set_space(&self.gpu.queue, space);
            // (TA's hair-like strands over its hills painted over: Reborn style, on the worlds
            // where they are hatching.)
            t.set_strands(&self.gpu.queue, self.art_style_reborn && self.ground_strands && self.strands_view);
        }
        // (Nor is there ground beyond its edges.)
        self.update_skirt();
    }

    /// The cockpit's instruments, for the game to paint before the frame is drawn.
    pub fn cockpit_instruments(&self) -> Option<&cockpit::InstrumentAtlas> {
        self.cockpit.as_ref().map(|c| &c.instruments)
    }

    /// Builds the room the intro plays in (see [`room`]), or takes it down (`None`).
    pub fn set_room(&mut self, scene: Option<&room::RoomScene>) {
        self.room = scene.map(|s| room::RoomRenderer::new(&self.gpu.device, &self.gpu.queue, s, self.post.samples));
    }

    /// Whether the room is up.
    pub fn has_room(&self) -> bool {
        self.room.is_some()
    }

    /// The movie's newest frame for the room's tube, RGBA (sRGB).
    pub fn room_picture(&mut self, width: u32, height: u32, rgba: &[u8]) {
        if let Some(r) = &mut self.room {
            r.set_picture(&self.gpu.device, &self.gpu.queue, width, height, rgba);
        }
    }

    /// Renders the room (in place of the battlefield) with the interface over it, and presents
    /// it. Does nothing while the room is down.
    pub fn render_room(&mut self, frame: &room::RoomFrame, ui: impl FnOnce(&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder, &wgpu::TextureView)) {
        if self.room.is_none() {
            return;
        }
        // (No battlefield drawn: the heat and the particles queued for it are let go, or they
        // pile up while the room is up.)
        self.heat_sources.clear();
        self.particles.drop_queued();
        let Some(output) = self.gpu.acquire() else {
            self.skip_frame();
            return;
        };
        let device = &self.gpu.device;
        let size = self.post.size();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("room") });
        let mut focus = 1.0;
        let (near, far) = if let Some(room) = &mut self.room {
            let near_far = room.prepare(&self.gpu.queue, frame, size, self.post.settings.exposure * post::ROOM_EXPOSURE);
            let (color, resolve) = self.post.scene_attachment();
            room.draw(device, &mut encoder, (color, resolve), resolve.unwrap_or(color), self.post.depth_view(), size);
            focus = room.focus(frame);
            near_far
        } else {
            (0.02, 20.0)
        };
        // The room's lens, focused on the tube, in place of photo mode's.
        let photo_dof = self.post.settings.dof;
        self.post.settings.dof = (frame.dof > 0.0).then_some(post::DepthOfField { mode: post::DofMode::Lens, focus, blur: frame.dof, band: 0.0 });
        let texture = &output.texture;
        let view = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(self.gpu.view_format), ..Default::default() });
        let post_frame = post::PostFrame { time: frame.time, heat: 0.0, depth: [near, far, 0.0], modern: false, room: true, space: None, output: (texture.width(), texture.height()) };
        self.post.run(device, &self.gpu.queue, &mut encoder, &view, post_frame, (None, None));
        self.post.settings.dof = photo_dof;
        let ui_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        ui(device, &self.gpu.queue, &mut encoder, &ui_view);
        let shot = std::mem::take(&mut self.screenshot_request);
        let pending = shot.then(|| self.encode_readback(&mut encoder, texture));
        self.gpu.queue.submit(Some(encoder.finish()));
        if let Some((buffer, w, h, padded)) = pending {
            self.last_screenshot = self.finish_readback(buffer, w, h, padded);
        }
        self.gpu.present(output);
    }

    /// Builds a space scene (see [`space`]), or takes it down (`None`). Fails if its asteroids'
    /// rocks cannot be read.
    pub fn set_space(&mut self, scene: Option<&space::SpaceScene>) -> anyhow::Result<()> {
        self.space = None;
        if let Some(scene) = scene {
            self.space = Some(space::SpaceRenderer::new(&self.gpu.device, &self.gpu.queue, scene, self.post.samples)?);
        }
        Ok(())
    }

    /// Whether a space scene is up.
    pub fn has_space(&self) -> bool {
        self.space.is_some()
    }

    /// Renders the space scene (in place of the battlefield) as `frame` has it, with the interface
    /// over it, and presents it. Does nothing while no scene is up.
    pub fn render_space(&mut self, frame: &space::SpaceFrame, ui: impl FnOnce(&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder, &wgpu::TextureView)) {
        if self.space.is_none() {
            return;
        }
        self.heat_sources.clear();
        self.particles.drop_queued();
        let Some(output) = self.gpu.acquire() else {
            self.skip_frame();
            return;
        };
        let device = &self.gpu.device;
        let size = self.post.size();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("space") });
        let mut look = None;
        if let Some(space) = &mut self.space {
            space.prepare(device, &self.gpu.queue, frame, size);
            let (color, resolve) = self.post.scene_attachment();
            space.draw(device, &mut encoder, (color, resolve), resolve.unwrap_or(color), self.post.depth_view());
            look = Some(space.look(frame));
        }
        let texture = &output.texture;
        let view = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(self.gpu.view_format), ..Default::default() });
        // (No depth of field in space: photo mode's is put back after.)
        let photo_dof = self.post.settings.dof.take();
        let post_frame = post::PostFrame { time: frame.time, heat: 0.0, depth: [0.0, 1.0, 0.0], modern: false, room: false, space: look, output: (texture.width(), texture.height()) };
        self.post.run(device, &self.gpu.queue, &mut encoder, &view, post_frame, (None, None));
        self.post.settings.dof = photo_dof;
        let ui_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        ui(device, &self.gpu.queue, &mut encoder, &ui_view);
        let shot = std::mem::take(&mut self.screenshot_request);
        let pending = shot.then(|| self.encode_readback(&mut encoder, texture));
        self.gpu.queue.submit(Some(encoder.finish()));
        if let Some((buffer, w, h, padded)) = pending {
            self.last_screenshot = self.finish_readback(buffer, w, h, padded);
        }
        self.gpu.present(output);
    }

    /// Renders a photo: the frame as `camera` shows it in the window, into a picture `width` ×
    /// `height` pixels (the window's shape), without the interface or the strategic icons and
    /// whatever the render scale. Call it before [`Renderer::render`] in a frame (it takes the
    /// frame's heat haze). None if the device cannot draw that large.
    pub fn render_image(&mut self, camera: &Camera, width: u32, height: u32) -> Option<Screenshot> {
        self.render_image_inner(camera, width, height, None::<fn(&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder, &wgpu::TextureView)>)
    }

    /// As [`Renderer::render_image`], with `ui` drawn over the picture (the scene's
    /// post-processing done) into its target.
    pub fn render_image_with(&mut self, camera: &Camera, width: u32, height: u32, ui: impl FnOnce(&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder, &wgpu::TextureView)) -> Option<Screenshot> {
        self.render_image_inner(camera, width, height, Some(ui))
    }

    fn render_image_inner(&mut self, camera: &Camera, width: u32, height: u32, ui: Option<impl FnOnce(&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder, &wgpu::TextureView)>) -> Option<Screenshot> {
        let device = &self.gpu.device;
        let max = device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > max || height > max {
            return None;
        }
        // Too large for the memory there is: the draw fails instead of the game.
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let invalid = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let filling = self.terrain_shadow.will_fill();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("photo"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.gpu.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[self.gpu.view_format],
        });
        let mut camera = camera.clone();
        camera.zoom *= height as f32 / camera.viewport.y.max(1.0);
        camera.viewport = Vec2::new(width as f32, height as f32);
        self.post.resize(&self.gpu.device, width, height);
        let icons = std::mem::take(&mut self.icon_list);
        let pixels = self.draw(&camera, &texture, ui, true, false);
        self.icon_list = icons;
        self.resize_scene();
        let failed = [pollster::block_on(invalid.pop()), pollster::block_on(memory.pop())];
        if let Some(e) = failed.iter().flatten().next() {
            log::error!("photo {width} x {height} failed: {e}");
            self.forget_frame(filling);
            return None;
        }
        pixels
    }

    /// Draws a frame into `texture` (of the window's format): the scene, its post-processing,
    /// the strategic icons and `ui`; with `readback`, returns its pixels. `timed`: the GPU timer
    /// measures it (the window's frames).
    fn draw(
        &mut self,
        camera: &Camera,
        texture: &wgpu::Texture,
        ui: Option<impl FnOnce(&wgpu::Device, &wgpu::Queue, &mut wgpu::CommandEncoder, &wgpu::TextureView)>,
        readback: bool,
        timed: bool,
    ) -> Option<Screenshot> {
        if let (Some(t), true) = (&mut self.timer, timed) {
            t.collect(&self.gpu.device);
            t.begin_frame();
        }
        self.begin_frame_lighting(camera);
        self.update_shadow_matrix(camera);
        self.update_skirt();
        let grid = self.upload_lights(camera);
        let (min, max) = camera.ground_bounds(260.0);
        self.strands_view = camera.mode == CameraMode::Perspective;
        if let Some(t) = &mut self.terrain {
            t.set_strands(&self.gpu.queue, self.art_style_reborn && self.ground_strands && self.strands_view);
            t.cull(&self.gpu.queue, min, max);
        }
        if let Some(w) = &mut self.water {
            w.cull(&self.gpu.queue, min, max);
        }
        let mut fx = self.prepare_effects(camera);
        fx.reflections = self.prepare_reflections(camera, min, max);
        fx.ray_traced = self.prepare_ray_tracing(camera);
        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        // Only advertise traced lighting after every caster made it into this frame's scene.
        // A partial/failed build must use the complete raster shadows, not an incomplete TLAS.
        if fx.ray_traced {
            fx.ray_traced = match (&mut self.ray_tracing, &self.models) {
                (Some(rt), Some(m)) => rt.build_scene(&self.gpu.device, &self.gpu.queue, &mut encoder, m),
                _ => false,
            };
        }
        // Sample once so raster and ray-traced material frames change together.
        let material_time = self.animation_time();
        self.advance_wind_drift(material_time);
        self.write_frame_uniform(camera, &grid, fx, material_time);
        let sky_on = self.sky_visible(camera);
        let water_on = self.water_visible();
        // Particles die in the sea or lava where its surface is drawn.
        let liquid = self.terrain.as_ref().filter(|_| water_on).map(|t| t.field.sea_level);
        let view_depth = if camera.mode == CameraMode::Perspective { -camera.view().row(2) } else { camera.view_proj().row(2) };
        let reflected_depth = fx.reflections.then(|| {
            let sea = self.terrain.as_ref().map_or(0.0, |t| t.field.sea_level);
            (reflection::mirror(sea).transpose() * view_depth).to_array()
        });
        self.sprites.sort_transparency(&self.gpu.device, &self.gpu.queue, view_depth.to_array(), reflected_depth);
        let particles_on = self.particles.prepare(&self.gpu.device, &self.gpu.queue, liquid, view_depth.to_array(), reflected_depth);
        let cockpit_on = camera.mode == CameraMode::Perspective && self.cockpit.is_some() && self.cockpit_frame.is_some();
        let mut timestamps = |label| self.timer.as_mut().filter(|_| timed).and_then(|t| t.pass(label));
        // Passes are timed in the order the GPU runs them (the timer's `full` span runs from the
        // first to the last). The cloud and contact occlusion passes are not timed on their own:
        // they take a few hundredths of a millisecond, and timestamps around a pass that small
        // serialize the GPU and cost far more than the pass (their cost shows in `frame`).
        let rt_ts = if fx.ray_traced { timestamps("ray tracing") } else { None };
        let shadow_ts = timestamps("shadow");
        let particles_ts = if particles_on { timestamps("particles") } else { None };
        let reflection_ts = if fx.reflections { timestamps("reflection") } else { None };
        let scene_ts = timestamps("scene");
        // (The water and what is drawn after it, in a pass of their own while the water is in
        // view: see below.)
        let water_drawn = water_on && self.water.is_some() && self.terrain.is_some();
        // Names in the world drawn again over what stands on them (world_text.rs) read the
        // scene's depth: so does the water, and the opaque scene then ends in a pass of its own.
        let text_over = self.world_text.wants_over();
        let translucent_ts = if water_drawn || text_over { timestamps("water and effects") } else { None };
        let cockpit_ts = if cockpit_on { timestamps("cockpit") } else { None };
        let bloom_ts = timestamps("bloom");
        let composite_ts = timestamps("composite");

        let device = &self.gpu.device;
        // Both the tracing pass and raster shading read this map. Refresh it before either
        // consumes it, including the first frame and frames following terrain edits.
        let follow_light = camera.mode == CameraMode::Perspective && self.frame_lighting.terrain_shadows > 0.0;
        self.terrain_shadow.update(&self.gpu.queue, &mut encoder, self.frame_lighting.sun_dir, follow_light);
        // Ray tracing: the scene of this frame's models, the depth of what the camera sees, then
        // the rays (raytrace.rs).
        let sun_dim = 1.0 - RAIN_DIM * self.overcast(camera);
        if let (true, Some(rt)) = (fx.ray_traced, &mut self.ray_tracing) {
            if let Some(depth) = rt.depth_view() {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ray tracing depth"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: depth,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_bind_group(0, &self.frame_bind_group, &[]);
                if let Some(t) = &self.terrain {
                    t.draw_prepass(&mut pass, &self.terrain_pipeline);
                }
                if let Some(m) = &self.models {
                    m.draw_prepass(&mut pass);
                }
            }
            let l = &self.frame_lighting;
            let view = raytrace::TraceView {
                view_proj: camera.view_proj(),
                eye: camera.eye(),
                time: material_time,
                light: l.sun_dir,
                sun_color: l.sun_color * l.sun_intensity * sun_dim,
                ambient: l.ambient * sun_dim,
                clip: camera.clip_range(),
                ao: if self.lighting.ambient_occlusion.enabled { RT_AO_STRENGTH } else { 0.0 },
                grid: [grid.origin.x, grid.origin.y, if grid.cell > 0.0 && !grid.lights.is_empty() { 1.0 / grid.cell } else { 0.0 }, 0.0],
                grid_dims: [grid.dims.x, grid.dims.y, grid.lights.len() as u32, self.colour_blind as u32 | ((self.art_style_reborn as u32) << 1)],
                lights: &self.light_buffer,
                light_grid: &self.light_grid_buffer,
                ground: &self.terrain_shadow.target.view,
                ground_scale: {
                    let u = self.terrain_shadow.uniform(1.0);
                    [u[1], u[2]]
                },
                wetness: self.lighting.wetness,
                model_reflections: self.lighting.hd_surfaces > 0.0,
            };
            if let Some(m) = &self.models {
                rt.trace(device, &self.gpu.queue, &mut encoder, m, &view, rt_ts.as_ref());
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_map.view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: shadow_ts.as_ref().map(|t| t.writes()),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.shadow_pass_bind_group, &[]);
            if let Some(m) = &self.models {
                m.draw_shadow(&mut pass);
            }
            if let Some(f) = &self.foliage {
                f.draw_shadow(&mut pass);
            }
        }
        // Ray traced: the 3D features' shadows again, alone, for the traced surfaces.
        if let (Some(map), Some(f), true) = (&self.feature_shadow_map, &self.foliage, fx.ray_traced) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("feature shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &map.view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.shadow_pass_bind_group, &[]);
            f.draw_shadow(&mut pass);
        }
        if fx.clouds {
            self.cloud_shadows.draw(&mut encoder);
        }
        if fx.contact {
            self.occlusion.draw(&mut encoder);
        }
        self.particles.encode(&mut encoder, particles_ts.as_ref());
        if fx.reflections {
            self.draw_reflection(&mut encoder, sky_on, reflection_ts.as_ref());
            self.reflections.filter(&mut encoder, &self.frame_bind_group);
        }
        // With the water in view the opaque scene is finished in a pass of its own, resolved into
        // (without antialiasing: copied into) the water's copy of it, and the water and what
        // follows it are drawn in a second pass that keeps its depth: the water reads both (see
        // `WaterScene`).
        if water_drawn {
            self.water_scene.prepare(device, self.post.depth_view(), self.post.size());
        }
        if text_over {
            self.world_text.prepare_over(device, self.post.depth_view());
        }
        let water_scene = self.water_scene.prepared().filter(|_| water_drawn);
        // What is kept of the scene's targets once a pass ends, on tiled graphics (Apple's own
        // chips), where a target left unkept never leaves the chip, and storing a multisampled
        // one writes every sample of it out to memory: the multisampled colour only for a pass
        // after it (the water's, the cockpit's; with antialiasing the picture itself is the
        // resolved copy, written at the end of the pass), the depth only for what reads it (the
        // water's pass, the names' second pass, the depth of field). (Elsewhere all is kept: a
        // dedicated card gains nothing measurable by it.)
        let antialiased = self.post.scene_attachment().1.is_some();
        let second_pass = water_scene.is_some() || text_over;
        let tiled = self.gpu.tiled;
        let keep = |kept: bool| if kept || !tiled { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard };
        {
            let (color, resolve) = self.post.scene_attachment();
            let sky = self.lighting.sky_color;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    resolve_target: match water_scene {
                        Some((copy, ..)) => resolve.map(|_| &copy.view),
                        // (Resolved at the end of the pass after it.)
                        None if text_over => None,
                        None => resolve,
                    },
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: sky.x as f64, g: sky.y as f64, b: sky.z as f64, a: 1.0 }),
                        store: keep(!antialiased || second_pass || cockpit_on),
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.post.depth_view(),
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: keep(second_pass || self.post.reads_depth()) }),
                    stencil_ops: None,
                }),
                timestamp_writes: scene_ts.as_ref().map(|t| t.writes()),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            // What stands on the ground first, the ground after it: the ground's is the costliest
            // shading of the frame, and wherever a unit, a tree or a painting stands in front of
            // it the depth test now skips it (a forest's ground is mostly hidden: a fifth of the
            // frame's time saved over Mossvale's woods). Nothing that stands is blended, so the
            // picture is the same: what lies on the ground (the names, the scorch marks, the
            // lying paintings) is still drawn after it, kept off what stands in front by the depth.
            if let Some(m) = &self.models {
                m.draw(&mut pass);
            }
            if let Some(f) = &self.foliage {
                f.draw(&mut pass);
            }
            self.sprites.draw(&mut pass, SpriteLayer::Billboard);
            if let Some(t) = &self.terrain {
                t.draw(&mut pass, &self.terrain_pipeline);
                // The ground beyond the map, after the terrain so it is mostly depth-rejected.
                if let (Some(skirt), true) = (&self.skirt, sky_on) {
                    pass.set_bind_group(2, &self.terrain_pipeline.detail, &[]);
                    skirt.draw(&mut pass, &self.terrain_pipeline.skirt, t.bind_group());
                }
            }
            // Text and signs lying on the ground, under the decals (the blasts' scorch marks) and
            // everything that stands on it.
            self.world_text.draw(&mut pass);
            self.sprites.draw(&mut pass, SpriteLayer::Decal);
            self.sprites.draw(&mut pass, SpriteLayer::Lying);
            // Selected units' rings on the ground (and on the water), under what stands on them.
            self.rings.draw(&mut pass);
            // The sky fills whatever background is left after the opaque scene.
            if sky_on {
                self.sky_pipeline.draw(&mut pass);
            }
            if water_scene.is_none() && !text_over {
                self.draw_translucent(&mut pass, &fx, sky_on, None);
            }
        }
        if water_scene.is_some() || text_over {
            let (color, resolve) = self.post.scene_attachment();
            if let Some((copy, _, wake_map)) = water_scene {
                if resolve.is_none() {
                    encoder.copy_texture_to_texture(self.post.scene_texture().as_image_copy(), copy.texture.as_image_copy(), copy.texture.size());
                }
                // The wakes ships and hovercraft leave, into the wake map the water reads.
                self.wakes.draw(&mut encoder, &wake_map.view, &self.frame_bind_group);
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("water and effects"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    resolve_target: resolve,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: keep(!antialiased || cockpit_on) },
                    depth_slice: None,
                })],
                // (Read-only: the water's group reads it too, and the names' second pass.)
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: self.post.depth_view(), depth_ops: None, stencil_ops: None }),
                timestamp_writes: translucent_ts.as_ref().map(|t| t.writes()),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            // The names over the units that stand on them, under the water and the effects.
            if text_over {
                self.world_text.draw_over(&mut pass);
            }
            self.draw_translucent(&mut pass, &fx, sky_on, water_scene.map(|(_, group, _)| group));
        }

        // Perspective: the cockpit over the scene, before the picture is finished (its lamps
        // bloom like the world's).
        if cockpit_on && let (Some(c), Some(f)) = (self.cockpit.as_mut(), self.cockpit_frame.as_ref()) {
            c.draw(device, &self.gpu.queue, &mut encoder, self.post.scene_attachment(), self.post.size(), camera, &self.frame_lighting, f, cockpit_ts.as_ref());
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.gpu.view_format),
            ..Default::default()
        });
        let post_frame = post::PostFrame {
            time: self.animation_time(),
            heat: if fx.heat { self.post.settings.effective_heat_haze() } else { 0.0 },
            depth: camera.depth_mapping(),
            modern: camera.mode == CameraMode::Perspective,
            room: false,
            space: None,
            output: (texture.width(), texture.height()),
        };
        self.post.run(device, &self.gpu.queue, &mut encoder, &view, post_frame, (bloom_ts.as_ref(), composite_ts.as_ref()));
        // Strategic icons over the finished image, under the interface.
        self.icons.prepare(device, &self.gpu.queue, &self.icon_list, camera.view_proj(), texture.width(), texture.height());
        self.icons.draw(&mut encoder, &view);
        if let Some(ui) = ui {
            // UI renderers (egui) expect a non-sRGB view and do their own gamma handling.
            let ui_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            ui(device, &self.gpu.queue, &mut encoder, &ui_view);
        }

        let pending = readback.then(|| self.encode_readback(&mut encoder, texture));
        if let (Some(t), true) = (&mut self.timer, timed) {
            t.resolve(&mut encoder);
        }
        self.gpu.queue.submit(Some(encoder.finish()));
        if let (Some(t), true) = (&mut self.timer, timed) {
            t.after_submit();
        }
        self.particles.after_submit();
        pending.and_then(|(buffer, width, height, padded)| self.finish_readback(buffer, width, height, padded))
    }

    /// What is drawn over the opaque scene: the water (given `water_scene`, the group it reads
    /// the opaque scene through, see [`WaterScene`]), then smoke, particles, effects, beams and
    /// hot air.
    fn draw_translucent<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, fx: &FrameEffects, sky_on: bool, water_scene: Option<&'a wgpu::BindGroup>) {
        // Translucent water over the finished opaque scene, below smoke and effects. Its
        // pipelines read the reflection as group 2 (stand-ins when none was drawn) and the scene
        // behind the water as group 3.
        let water = |pass: &mut wgpu::RenderPass<'a>, pipeline: &'a wgpu::RenderPipeline, outer: &'a wgpu::RenderPipeline| {
            if let (Some(w), Some(t), Some(scene)) = (&self.water, &self.terrain, water_scene) {
                pass.set_bind_group(2, self.reflections.water_group(fx.reflections), &[]);
                pass.set_bind_group(3, scene, &[]);
                w.draw_with(pass, pipeline, t);
                if let (Some(skirt), true) = (&self.skirt, sky_on) {
                    skirt.draw(pass, outer, t.bind_group());
                }
            }
        };
        water(pass, &self.water_pipeline.pipeline, &self.water_pipeline.outer);
        self.sprites.draw(pass, SpriteLayer::Smoke);
        self.particles.draw(pass, ParticleLayer::Alpha);
        self.sprites.draw(pass, SpriteLayer::Effect);
        self.particles.draw(pass, ParticleLayer::Additive);
        self.beams.draw(pass);
        // Hot air, last: it only adds to the target's alpha (see heat.rs).
        if fx.heat {
            self.heat.draw(pass);
            if fx.lava_heat {
                water(pass, &self.water_pipeline.heat, &self.water_pipeline.heat_outer);
            }
        }
    }

    fn encode_readback(&self, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) -> (wgpu::Buffer, u32, u32, u32) {
        let (width, height) = (texture.width(), texture.height());
        let padded = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot"),
            size: (padded * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        (buffer, width, height, padded)
    }

    fn finish_readback(&self, buffer: wgpu::Buffer, width: u32, height: u32, padded: u32) -> Option<Screenshot> {
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        if let Err(e) = self.gpu.device.poll(wgpu::PollType::wait_indefinitely()) {
            log::error!("screenshot readback failed: {e}");
            return None;
        }
        let data = match slice.get_mapped_range() {
            Ok(d) => d,
            Err(e) => {
                log::error!("screenshot mapping failed: {e}");
                return None;
            }
        };
        let bgra = matches!(self.gpu.config.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let line = &data[(row * padded) as usize..(row * padded + width * 4) as usize];
            for px in line.as_chunks::<4>().0 {
                if bgra {
                    rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                } else {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
        }
        Some(Screenshot { width, height, rgba })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ray_trace_storage_limit_checks_each_entire_binding_without_overflow() {
        let limit = 128 * 1024 * 1024;
        assert!(super::ray_trace_buffer_sizes_fit(0, 0, limit));
        assert!(super::ray_trace_buffer_sizes_fit(limit, limit, limit));
        assert!(!super::ray_trace_buffer_sizes_fit(limit + 1, 4, limit));
        assert!(!super::ray_trace_buffer_sizes_fit(52, limit + 1, limit));
        assert!(!super::ray_trace_buffer_sizes_fit(u64::MAX, 0, u64::from(u32::MAX)));
    }
    use super::*;
    use wgpu::naga;

    /// The shadow map keeps its size (so its texels hold still on the ground) while the view
    /// needs a little more or less of it, and steps to the next size when it needs much less or
    /// more than it holds.
    #[test]
    fn the_shadow_maps_size_holds_while_the_view_changes_a_little() {
        let first = steady_shadow_extent(0.0, 1000.0);
        assert!((1000.0..1000.0 * SHADOW_EXTENT_STEP).contains(&first), "{first}");
        for needed in [first, first * 0.99, first / SHADOW_EXTENT_STEP, first / (SHADOW_EXTENT_STEP * SHADOW_EXTENT_STEP) * 1.001] {
            assert_eq!(steady_shadow_extent(first, needed), first, "needing {needed}");
        }
        let smaller = steady_shadow_extent(first, first / 1.3);
        assert!(smaller < first && smaller >= first / 1.3, "{smaller}");
        let bigger = steady_shadow_extent(first, first * 1.01);
        assert!((bigger / first - SHADOW_EXTENT_STEP).abs() < 1e-3, "{bigger}");
        // (Its steps are the same ones however it got there.)
        assert!((steady_shadow_extent(0.0, first * 1.01) - bigger).abs() < 1e-3);
    }

    /// A camera low over the ground looking out to the horizon (the title menu's, at 4K): the
    /// shadow map covers the ground nearest the camera, inside its unfaded part, not ground far
    /// off; a camera looking down has its whole view covered.
    #[test]
    fn the_shadow_map_covers_the_ground_nearest_the_camera() {
        // (The menu's camera swings between 0.16 and 0.5; at a little over half the field of view,
        // 0.26, the top of the screen looks just under the horizon, at ground far off.)
        let pitches = (0..23).map(|k| 0.16 + 0.02 * k as f32).chain([1.2]);
        for (pitch, zoom) in pitches.map(|p| (p, 1.1f32)) {
            let mut cam = Camera::new(Vec2::new(3840.0, 2160.0));
            cam.mode = CameraMode::Perspective;
            cam.focus = Vec3::new(9000.0, 40.0, 9000.0);
            cam.zoom = zoom;
            cam.pitch = pitch;
            cam.yaw = 0.7;
            let eye = cam.eye();
            let points = cam.outline_points([0.0, 200.0]);
            let (centre, reach) = shadow_fit(Vec2::new(eye.x, eye.z), &points);
            let extent = steady_shadow_extent(0.0, (reach + 150.0).max(reach / SHADOW_FADE_FROM).min(SHADOW_EXTENT_MAX));
            assert!(extent <= SHADOW_EXTENT_MAX * SHADOW_EXTENT_STEP, "pitch {pitch}: extent {extent}");
            // The ground under the bottom of the screen (its corners and middle) and under its
            // middle lower down: in the map, before its rim (a square round the middle: the
            // farther axis counts).
            for screen in [Vec2::new(0.0, 2160.0), Vec2::new(1920.0, 2160.0), Vec2::new(3840.0, 2160.0), Vec2::new(1920.0, 1620.0)] {
                let (o, d) = cam.screen_ray(screen);
                let p = o + d * ((0.0 - o.y) / d.y);
                let off = (Vec2::new(p.x, p.z) - centre).abs().max_element();
                assert!(off <= extent * SHADOW_FADE_FROM + 1.0, "pitch {pitch}: the ground at {screen} is {off:.0} from the map's middle, its unfaded half-size {:.0}", extent * SHADOW_FADE_FROM);
            }
            if pitch > 1.0 {
                let all = cam.outline_points([0.0, 0.0]).iter().map(|p| (*p - centre).abs().max_element()).fold(0.0, f32::max);
                assert!(all <= extent * SHADOW_FADE_FROM + 1.0, "looking down, the whole view is covered ({all:.0} of {extent:.0})");
            }
        }
    }

    /// The ground across the biggest shadow map, and what towers over it from beyond towards the
    /// sun, lies within the map's depth, whatever the sun's height (and the map is never less
    /// deep than it was).
    #[test]
    fn the_shadow_maps_depth_holds_the_ground_across_it() {
        for el in [8.0f32, 15.0, 30.0, 45.0, 70.0, 89.0] {
            let sun = Vec3::new(el.to_radians().cos() * 0.6, el.to_radians().sin(), el.to_radians().cos() * 0.8).normalize();
            let (back, depth) = shadow_depth(SHADOW_EXTENT_MAX, sun.y);
            assert!(depth >= SHADOW_DEPTH_RANGE);
            let center = Vec3::new(9000.0, 60.0, 9000.0);
            let up = if sun.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
            let view = glam::camera::rh::view::look_at_mat4(center + sun * back, center, up);
            let deep = |p: Vec3| -view.transform_point3(p).z;
            let toward = Vec2::new(sun.x, sun.z).normalize();
            for k in 0..16 {
                let a = k as f32 * std::f32::consts::TAU / 16.0;
                let g = Vec2::new(a.cos(), a.sin()) * SHADOW_EXTENT_MAX;
                for h in [0.0f32, 200.0] {
                    let d = deep(Vec3::new(center.x + g.x, h, center.z + g.y));
                    assert!(d > 100.0 + 300.0 && d < 100.0 + depth - 300.0, "sun at {el} degrees: ground {g} at {h} is {d:.0} deep, the map {depth:.0}");
                }
            }
            // A tall caster beyond the map's edge towards the sun, whose shadow falls into it.
            let reach = SHADOW_CASTER_HEIGHT / el.to_radians().tan();
            let edge = toward * (SHADOW_EXTENT_MAX + reach.min(4000.0) * 0.9);
            let d = deep(Vec3::new(center.x + edge.x, SHADOW_CASTER_HEIGHT, center.z + edge.y));
            assert!(d > 100.0, "sun at {el} degrees: a caster towards the sun is {d:.0} deep");
        }
    }

    /// Parses and validates a shader exactly as the renderer assembles it (no GPU needed).
    fn validated(label: &str, source: &str) -> naga::Module {
        validated_for(label, source, false)
    }

    /// As [`validated`], reading the ray-traced results or not (see raytrace.rs).
    fn validated_for(label: &str, source: &str, ray_traced: bool) -> naga::Module {
        let full = util::with_common_for(source, ray_traced);
        let module = naga::front::wgsl::parse_str(&full).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&full)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        module
    }

    /// The terrain's and the water's shaders as wgpu's DirectX 12 backend hands them to its shader
    /// compiler (HLSL, an entry point a file, its constants settled), with wgpu's loop bounding
    /// and without, into `TA_HLSL_OUT`: `tools/shaders/hlsl_times.ps1` times fxc and dxc on them.
    /// (Windows only, as the compilers are: naga's HLSL writer comes with wgpu's Direct3D 12
    /// backend, which macOS builds leave out.)
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn dump_hlsl() {
        let out = std::path::PathBuf::from(std::env::var("TA_HLSL_OUT").expect("TA_HLSL_OUT=<folder>"));
        std::fs::create_dir_all(&out).unwrap();
        // (`TA_WGSL_IN=<file.wgsl>`: that shader, whole as written, in place of the four; each of
        // the four is otherwise also written whole as WGSL, to be changed and given back so.)
        let shaders: Vec<(String, String)> = match std::env::var("TA_WGSL_IN") {
            Ok(file) => vec![(std::path::Path::new(&file).file_stem().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&file).unwrap())],
            Err(_) => [
                ("terrain", terrain::shader_source_for(false)),
                ("terrain_far", terrain::shader_source_for(true)),
                ("water", water::shader_source_for(false)),
                ("water_far", water::shader_source_for(true)),
            ]
            .into_iter()
            .map(|(name, source)| {
                let full = util::with_common_for(&source, false);
                std::fs::write(out.join(format!("{name}.wgsl")), &full).unwrap();
                (name.to_string(), full)
            })
            .collect(),
        };
        for (name, full) in shaders {
            let name = name.as_str();
            let module = naga::front::wgsl::parse_str(&full).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&full)));
            let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
            for ep in &module.entry_points {
                let stage = match ep.stage {
                    naga::ShaderStage::Vertex => "vs",
                    naga::ShaderStage::Fragment => "ps",
                    _ => "cs",
                };
                let (settled, settled_info) = naga::back::pipeline_constants::process_overrides(&module, &info, Some((ep.stage, ep.name.as_str())), &Default::default()).unwrap();
                for bounded in [true, false] {
                    let options = naga::back::hlsl::Options {
                        shader_model: naga::back::hlsl::ShaderModel::V5_1,
                        fake_missing_bindings: true,
                        zero_initialize_workgroup_memory: true,
                        restrict_indexing: true,
                        force_loop_bounding: bounded,
                        ..Default::default()
                    };
                    let pipeline = naga::back::hlsl::PipelineOptions { entry_point: Some((ep.stage, ep.name.clone())) };
                    let mut text = String::new();
                    naga::back::hlsl::Writer::new(&mut text, &options, &pipeline).write(&settled, &settled_info, None).unwrap_or_else(|e| panic!("{name} {}: {e}", ep.name));
                    std::fs::write(out.join(format!("{name}.{}.{stage}.{}.hlsl", ep.name, if bounded { "bounded" } else { "plain" })), text).unwrap();
                }
            }
        }
    }

    /// Vulkan requires the variable decorated `SampleMask` to be an array of 32-bit integers
    /// (VUID-SampleMask-SampleMask-04359). naga 30 wrote a `u32`, the suspected cause of AMD's
    /// Vulkan driver crashing as it built the sprites' pipelines; the patched naga in
    /// third_party/naga writes `array<u32, 1>`. Every entry point writing a sample mask, compiled
    /// as a pipeline compiles it (eight samples), declares the array and stores through it.
    /// (Not on macOS, whose wgpu has no SPIR-V writer.)
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn sample_masks_are_arrays_in_spirv() {
        let unit = model::unit_source();
        let shaders = [
            ("sprites", include_str!("shaders/sprite.wgsl"), "fs_cutout"),
            ("units", unit.as_str(), "fs_main"),
            ("foliage", include_str!("shaders/foliage.wgsl"), "fs_cutout"),
            ("foliage", include_str!("shaders/foliage.wgsl"), "fs_opaque"),
        ];
        for (label, source, entry) in shaders {
            let module = validated(label, source);
            let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
            let mut constants = naga::back::PipelineConstants::default();
            constants.insert("SCENE_SAMPLES".to_string(), 8.0);
            let (settled, settled_info) = naga::back::pipeline_constants::process_overrides(&module, &info, Some((naga::ShaderStage::Fragment, entry)), &constants).unwrap();
            let pipeline = naga::back::spv::PipelineOptions { shader_stage: naga::ShaderStage::Fragment, entry_point: entry.into() };
            let words = naga::back::spv::write_vec(&settled, &settled_info, &naga::back::spv::Options::default(), Some(&pipeline)).unwrap();
            // The instructions (opcode, operands), after the five words of the header.
            let mut ops: Vec<(u32, &[u32])> = Vec::new();
            let mut at = 5;
            while at < words.len() {
                let count = (words[at] >> 16) as usize;
                ops.push((words[at] & 0xffff, &words[at + 1..at + count]));
                at += count;
            }
            let defining = |opcode: u32, result_at: usize, id: u32| ops.iter().find(|(op, args)| *op == opcode && args.get(result_at) == Some(&id)).map(|&(_, args)| args);
            const OP_DECORATE: u32 = 71;
            const BUILT_IN: u32 = 11;
            const SAMPLE_MASK: u32 = 20;
            let masks: Vec<u32> = ops.iter().filter(|(op, args)| *op == OP_DECORATE && args.get(1) == Some(&BUILT_IN) && args.get(2) == Some(&SAMPLE_MASK)).map(|(_, args)| args[0]).collect();
            assert_eq!(masks.len(), 1, "{label} {entry}: one sample mask written");
            let variable = defining(59, 1, masks[0]).expect("its OpVariable");
            let pointer = defining(32, 0, variable[0]).expect("its OpTypePointer");
            let array = defining(28, 0, pointer[2]).unwrap_or_else(|| panic!("{label} {entry}: the SampleMask variable is not an array"));
            assert_eq!(defining(21, 0, array[1]), Some(&[array[1], 32, 0][..]), "{label} {entry}: an array of u32");
            assert_eq!(defining(43, 1, array[2]).map(|c| c[2]), Some(1), "{label} {entry}: of one element");
            // (Written through an access chain to its element, never stored to whole.)
            assert!(ops.iter().any(|(op, args)| *op == 65 && args.get(2) == Some(&masks[0])), "{label} {entry}: stored through its element");
            assert!(!ops.iter().any(|(op, args)| *op == 62 && args.first() == Some(&masks[0])), "{label} {entry}: never stored to whole");
        }
    }

    /// How much code each function of the terrain's and the water's shaders makes in each entry
    /// point once every call is inlined, as HLSL's compilers inline them: its own expressions
    /// times the copies of it the entry point's calls make, the largest first (`TA_INLINE_TOP`,
    /// 25; `TA_WGSL_IN=<file.wgsl>` that shader instead).
    #[test]
    #[ignore]
    fn inlined_sizes() {
        use std::collections::HashMap;
        fn calls(block: &naga::Block, out: &mut Vec<naga::Handle<naga::Function>>) {
            for s in block.iter() {
                match s {
                    naga::Statement::Call { function, .. } => out.push(*function),
                    naga::Statement::Block(b) => calls(b, out),
                    naga::Statement::If { accept, reject, .. } => {
                        calls(accept, out);
                        calls(reject, out);
                    }
                    naga::Statement::Switch { cases, .. } => cases.iter().for_each(|c| calls(&c.body, out)),
                    naga::Statement::Loop { body, continuing, .. } => {
                        calls(body, out);
                        calls(continuing, out);
                    }
                    _ => {}
                }
            }
        }
        // The copies of every function one copy of `f` makes, itself among them.
        fn copies(module: &naga::Module, f: naga::Handle<naga::Function>, memo: &mut HashMap<naga::Handle<naga::Function>, HashMap<naga::Handle<naga::Function>, u64>>) -> HashMap<naga::Handle<naga::Function>, u64> {
            if let Some(m) = memo.get(&f) {
                return m.clone();
            }
            let mut out = HashMap::from([(f, 1u64)]);
            let mut callees = Vec::new();
            calls(&module.functions[f].body, &mut callees);
            for c in callees {
                for (g, n) in copies(module, c, memo) {
                    *out.entry(g).or_default() += n;
                }
            }
            memo.insert(f, out.clone());
            out
        }
        let top: usize = std::env::var("TA_INLINE_TOP").ok().and_then(|v| v.parse().ok()).unwrap_or(25);
        let shaders: Vec<(String, String)> = match std::env::var("TA_WGSL_IN") {
            Ok(file) => vec![(std::path::Path::new(&file).file_stem().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&file).unwrap())],
            Err(_) => [
                ("terrain", terrain::shader_source_for(false)),
                ("terrain_far", terrain::shader_source_for(true)),
                ("water", water::shader_source_for(false)),
                ("water_far", water::shader_source_for(true)),
            ]
            .into_iter()
            .map(|(name, source)| (name.to_string(), util::with_common_for(&source, false)))
            .collect(),
        };
        for (name, full) in shaders {
            let module = naga::front::wgsl::parse_str(&full).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&full)));
            let mut memo = HashMap::new();
            for ep in &module.entry_points {
                let mut total: HashMap<naga::Handle<naga::Function>, u64> = HashMap::new();
                let mut callees = Vec::new();
                calls(&ep.function.body, &mut callees);
                for c in callees {
                    for (g, n) in copies(&module, c, &mut memo) {
                        *total.entry(g).or_default() += n;
                    }
                }
                let own = |f: naga::Handle<naga::Function>| module.functions[f].expressions.len() as u64;
                let mut rows: Vec<_> = total.iter().map(|(&f, &n)| (n * own(f), n, own(f), module.functions[f].name.clone().unwrap_or_default())).collect();
                rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.3.cmp(&b.3)));
                let sum: u64 = rows.iter().map(|r| r.0).sum::<u64>() + ep.function.expressions.len() as u64;
                println!("{name} {}: {sum} expressions inlined ({} own)", ep.name, ep.function.expressions.len());
                for (size, n, own, f) in rows.iter().take(top) {
                    println!("  {size:>7} {n:>5} x {own:>5}  {f}");
                }
                // (`TA_INLINE_WHO=<function>`: where its copies come from, by the functions
                // calling it: their copies times their calls to it.)
                if let Ok(who) = std::env::var("TA_INLINE_WHO") {
                    let mut by: Vec<(u64, String)> = total
                        .iter()
                        .filter_map(|(&g, &n)| {
                            let mut direct = Vec::new();
                            calls(&module.functions[g].body, &mut direct);
                            let k = direct.iter().filter(|&&c| module.functions[c].name.as_deref() == Some(who.as_str())).count() as u64;
                            (k > 0).then(|| (n * k, format!("{} ({n} x {k})", module.functions[g].name.clone().unwrap_or_default())))
                        })
                        .collect();
                    let mut direct = Vec::new();
                    calls(&ep.function.body, &mut direct);
                    let k = direct.iter().filter(|&&c| module.functions[c].name.as_deref() == Some(who.as_str())).count() as u64;
                    if k > 0 {
                        by.push((k, format!("{} (entry)", ep.name)));
                    }
                    by.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
                    println!("  {who}'s copies:");
                    for (n, f) in by {
                        println!("    {n:>6}  {f}");
                    }
                }
            }
        }
    }

    /// The unit shader through what a pipeline does to it before it is built (its constants
    /// settled, each entry point kept alone): a module plain validation passes can still fail
    /// there (a constant made from another inside a function, emitted twice).
    #[test]
    fn unit_shader_survives_pipeline_constants() {
        let full = util::with_common_for(&model::unit_source(), false);
        let module = naga::front::wgsl::parse_str(&full).unwrap_or_else(|e| panic!("unit: {}", e.emit_to_string(&full)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
        for ep in &module.entry_points {
            if let Err(e) = naga::back::pipeline_constants::process_overrides(&module, &info, Some((ep.stage, ep.name.as_str())), &Default::default()) {
                panic!("{}: {e:#?}", ep.name);
            }
        }
    }

    #[test]
    fn ray_tracer_reads_model_vertices_whole() {
        // raytrace.wgsl walks the model vertex buffer in words of this many.
        let words = std::mem::size_of::<model::ModelVertex>() / 4;
        assert!(include_str!("shaders/raytrace.wgsl").contains(&format!("const VERTEX_WORDS: u32 = {words}u;")), "VERTEX_WORDS must be {words}");
    }

    #[test]
    fn room_shader_validates() {
        for samples in [1, 4] {
            let source = room::shader_source(samples);
            let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("room: {}", e.emit_to_string(&source)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e| panic!("room: {e:?}"));
            // The uniforms as the shader lays them out match the ones the renderer writes.
            assert_eq!(type_size(&module, "Room"), room::UNIFORM_SIZE);
            assert_eq!(type_size(&module, "Material"), room::MATERIAL_SIZE);
        }
    }

    #[test]
    fn cockpit_shader_validates() {
        let module = validated_standalone("cockpit", include_str!("shaders/cockpit.wgsl"));
        // The buffers as the shader lays them out match the ones the renderer writes.
        assert_eq!(type_size(&module, "Uniform"), cockpit::UNIFORM_SIZE);
        assert_eq!(type_size(&module, "Material"), cockpit::MATERIAL_SIZE);
        assert_eq!(type_size(&module, "Screen"), cockpit::SCREEN_SIZE);
    }

    #[test]
    fn space_shader_validates() {
        let has = |module: &naga::Module, name: &str| module.entry_points.iter().any(|e| e.name == name);
        for samples in [1, 4] {
            let module = validated_standalone("space", &space::shader_source(samples));
            // The buffers as the shader lays them out match the ones the renderer writes.
            assert_eq!(type_size(&module, "SpaceFrame"), space::FRAME_SIZE);
            assert_eq!(type_size(&module, "Sphere"), space::SPHERE_SIZE);
            assert_eq!(type_size(&module, "Nebula"), space::NEBULA_SIZE);
            assert_eq!(type_size(&module, "Galaxy"), space::GALAXY_SIZE);
            for name in ["vs_fullscreen", "fs_sky", "vs_star", "fs_star", "vs_rock", "fs_rock", "vs_rock_shadow", "vs_sphere", "fs_sphere", "fs_halo_tint", "fs_halo_glow", "vs_glare", "fs_glare"] {
                assert!(has(&module, name), "space: {name}");
            }
        }
    }

    fn type_size(module: &naga::Module, name: &str) -> usize {
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx()).expect("layout");
        let (handle, _) = module.types.iter().find(|(_, t)| t.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("no type {name}"));
        layouter[handle].size as usize
    }

    /// Parses and validates a shader that is built without the shared frame declarations.
    fn validated_standalone(label: &str, source: &str) -> naga::Module {
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        module
    }

    #[test]
    fn tas_own_fog_leaves_a_half_explored_and_a_fifth_never_seen() {
        // (As `fog_brightness` in common.wgsl: the radar changes nothing here.)
        assert_eq!(fog_brightness(1.0, 1.0), 1.0, "in sight, as lit");
        assert_eq!(fog_brightness(0.0, 1.0), 0.5, "explored: a half");
        assert!((fog_brightness(0.0, 0.0) - 0.2).abs() < 1e-6, "never seen: a fifth");
        // Between the states it goes smoothly, and what is in sight is explored too.
        assert!((fog_brightness(0.5, 0.0) - (0.35 + 0.65 * 0.5)).abs() < 1e-6);
        assert_eq!(fog_brightness(1.0, 0.0), 1.0);
        assert!(shader_source_has("fn fog_brightness(s: vec3<f32>)", "mix(mix(FOG_UNEXPLORED, FOG_EXPLORED, max(s.y, s.x)), 1.0, s.x)"), "the shader's own");
    }

    #[test]
    fn the_ground_moves_unless_motion_is_unwanted() {
        assert_eq!(ground_life_strength(1.0, false), 1.0);
        assert_eq!(ground_life_strength(0.4, false), 0.4);
        assert_eq!(ground_life_strength(0.0, false), 0.0, "the setting off");
        assert_eq!(ground_life_strength(1.0, true), 0.0, "reduced motion keeps the ground still");
        assert_eq!(ground_life_strength(5.0, false), 1.0, "never more than the whole");
        assert_eq!(Lighting::default().ground_life, 1.0);
    }

    /// Whether the shared shader source holds `function` and, in the text after it, `body`.
    fn shader_source_has(function: &str, body: &str) -> bool {
        let source = include_str!("shaders/common.wgsl");
        source.find(function).is_some_and(|at| source[at..].contains(body))
    }

    #[test]
    fn all_shaders_validate() {
        validated("terrain", &terrain::shader_source());
        validated("terrain (the map's)", &terrain::shader_source_for(false));
        validated("water", &water::shader_source());
        validated("water (the map's)", &water::shader_source_for(false));
        validated("water (no antialiasing)", &water::scene_source(&water::shader_source(), 1));
        validated("sky", &sky::shader_source());
        validated("unit", &model::unit_source());
        validated("sprites", include_str!("shaders/sprite.wgsl"));
        validated("foliage", include_str!("shaders/foliage.wgsl"));
        validated("beams", include_str!("shaders/beam.wgsl"));
        validated("rings", include_str!("shaders/ring.wgsl"));
        validated("world text", include_str!("shaders/world_text.wgsl"));
        validated("world text (no antialiasing)", &world_text::scene_source(1));
        validated("wakes", &wakes::shader_source());
        validated_standalone("icons", include_str!("shaders/icon.wgsl"));
        validated("heat", include_str!("shaders/heat.wgsl"));
        validated("reflection filter", &reflection::filter_source());
        validated_standalone("contact occlusion", include_str!("shaders/occlusion.wgsl"));
        validated_standalone("cloud shadows", &clouds::shader_source());
        validated_standalone("post", &post::shader_source());
        validated_standalone("depth of field", &post::dof_source(4));
        validated_standalone("depth of field (no antialiasing)", &post::dof_source(1));
        validated("particle draw", &gpu_particles::draw_source());
        validated_standalone("particle simulation", &gpu_particles::sim_source());
        validated_standalone("terrain shadow", include_str!("shaders/terrain_shadow.wgsl"));
    }

    /// With ray tracing the scene's shaders read its result, and its own shader traces.
    #[test]
    fn ray_traced_shaders_validate() {
        let has = |module: &naga::Module, name: &str| module.entry_points.iter().any(|e| e.name == name);
        validated_for("terrain (ray traced)", &terrain::shader_source(), true);
        validated_for("terrain (the map's, ray traced)", &terrain::shader_source_for(false), true);
        validated_for("water (ray traced)", &water::shader_source(), true);
        validated_for("water (the map's, ray traced)", &water::shader_source_for(false), true);
        validated_for("sky (ray traced)", &sky::shader_source(), true);
        validated_for("unit (ray traced)", &model::unit_source(), true);
        validated_for("sprites (ray traced)", include_str!("shaders/sprite.wgsl"), true);
        validated_for("foliage (ray traced)", include_str!("shaders/foliage.wgsl"), true);
        validated_for("world text (ray traced)", include_str!("shaders/world_text.wgsl"), true);
        validated_for("world text (no antialiasing, ray traced)", &world_text::scene_source(1), true);
        validated_for("particle draw (ray traced)", &gpu_particles::draw_source(), true);
        let rt = validated_standalone("ray tracing", &raytrace::shader_source());
        for name in ["trace", "blur_across", "blur_down"] {
            assert!(has(&rt, name), "ray tracing: {name}");
        }
    }

    /// Every entry point the pipelines use exists in its module.
    #[test]
    fn pipeline_entry_points_exist() {
        let has = |module: &naga::Module, name: &str| module.entry_points.iter().any(|e| e.name == name);
        let terrain = validated("terrain", &terrain::shader_source());
        for name in ["vs_main", "vs_skirt", "fs_main", "fs_reflection"] {
            assert!(has(&terrain, name), "terrain: {name}");
        }
        // The reflection pass's own fragment entry points (reflection.rs).
        let unit = validated("unit", &model::unit_source());
        assert!(has(&unit, "fs_main") && has(&unit, "fs_reflection"));
        let sprites = validated("sprites", include_str!("shaders/sprite.wgsl"));
        for name in ["fs_cutout_reflection", "fs_additive_reflection", "fs_smoke_reflection"] {
            assert!(has(&sprites, name), "sprites: {name}");
        }
        let foliage = validated("foliage", include_str!("shaders/foliage.wgsl"));
        for name in ["vs_main", "vs_shadow", "fs_cutout", "fs_opaque", "fs_shadow_cutout", "fs_cutout_reflection", "fs_opaque_reflection"] {
            assert!(has(&foliage, name), "foliage: {name}");
        }
        let beams = validated("beams", include_str!("shaders/beam.wgsl"));
        assert!(has(&beams, "fs_beam") && has(&beams, "fs_beam_reflection"));
        let text = validated("world text", include_str!("shaders/world_text.wgsl"));
        assert!(has(&text, "vs_text") && has(&text, "fs_text") && has(&text, "fs_text_over"));
        let filter = validated("reflection filter", &reflection::filter_source());
        for name in ["vs_fullscreen", "fs_reflection_first", "fs_reflection_down"] {
            assert!(has(&filter, name), "reflection filter: {name}");
        }
        let water = validated("water", &water::shader_source());
        for name in ["vs_water", "fs_water", "vs_water_outer", "fs_water_outer"] {
            assert!(has(&water, name), "water: {name}");
        }
        let sky = validated("sky", &sky::shader_source());
        for name in ["vs_sky", "fs_sky"] {
            assert!(has(&sky, name), "sky: {name}");
        }
        for name in ["fs_heat_water", "fs_heat_outer"] {
            assert!(has(&water, name), "water: {name}");
        }
        let heat = validated("heat", include_str!("shaders/heat.wgsl"));
        assert!(has(&heat, "vs_heat") && has(&heat, "fs_heat"));
        let occlusion = validated_standalone("contact occlusion", include_str!("shaders/occlusion.wgsl"));
        assert!(has(&occlusion, "vs_occluder") && has(&occlusion, "fs_occluder"));
        let clouds = validated_standalone("cloud shadows", &clouds::shader_source());
        assert!(has(&clouds, "vs_fullscreen") && has(&clouds, "fs_cloud_shadow"));
        let sim = validated_standalone("particle simulation", &gpu_particles::sim_source());
        for name in ["cs_emit", "cs_plan", "cs_simulate", "cs_finalize"] {
            assert!(has(&sim, name), "particle simulation: {name}");
        }
        let draw = validated("particle draw", &gpu_particles::draw_source());
        for name in ["vs_particle_additive", "fs_particle_additive", "vs_particle_alpha", "fs_particle_alpha", "fs_particle_additive_reflection", "fs_particle_alpha_reflection"] {
            assert!(has(&draw, name), "particle draw: {name}");
        }
    }

    #[test]
    fn gpu_structs_match_wgsl_layouts() {
        let module = validated("common", "");
        assert_eq!(type_size(&module, "Frame"), std::mem::size_of::<FrameUniform>());
        assert_eq!(type_size(&module, "PointLightGpu"), std::mem::size_of::<GpuLight>());
        let terrain = validated("terrain", &terrain::shader_source());
        assert_eq!(type_size(&terrain, "TerrainParams"), std::mem::size_of::<terrain::TerrainParams>());
        assert_eq!(type_size(&terrain, "PlainTiles"), std::mem::size_of::<terrain::PlainTiles>());
        assert_eq!(type_size(&terrain, "Surroundings"), std::mem::size_of::<surroundings::Surroundings>());
        let water = validated("water", &water::shader_source());
        assert_eq!(type_size(&water, "Surroundings"), std::mem::size_of::<surroundings::Surroundings>());
        assert_eq!(type_size(&terrain, "DetailWorld"), std::mem::size_of::<terrain_detail::WorldUniform>());
    }

    /// The sky and the cloud shadows draw the same clouds: one shared definition, used by both.
    #[test]
    fn cloud_shadows_use_the_sky_clouds() {
        let clouds = include_str!("shaders/clouds.wgsl");
        assert!(sky::shader_source().contains(clouds) && water::shader_source().contains(clouds));
        assert!(clouds::shader_source().contains(clouds));
        let sky_model = include_str!("shaders/sky_common.wgsl");
        for f in ["cloud_warp(p)", "cloud_fbm(p + warp, footprint, 5)", "cloud_threshold(cover)"] {
            assert!(sky_model.contains(f), "sky clouds use {f}");
        }
        let shadow = include_str!("shaders/cloud_shadow.wgsl");
        for f in ["cloud_warp(p)", "cloud_fbm(p + cloud_warp(p), footprint, 5)", "cloud_threshold(params.cover.x)", "CLOUD_WIND"] {
            assert!(shadow.contains(f), "cloud shadows use {f}");
        }
    }

    /// Every new effect is on by default (the perspective camera; the classic one never draws
    /// them, see `write_frame_uniform` and `prepare_effects`) and has a setting to turn it off.
    #[test]
    fn effect_settings_default_on() {
        let l = Lighting::default();
        assert!(l.ambient_occlusion.enabled && l.ambient_occlusion.terrain > 0.0 && l.ambient_occlusion.contact > 0.0);
        assert!(l.cloud_shadows > 0.0 && l.sky_light > 0.0 && l.soft_smoke > 0.0);
        assert!(post::PostSettings::default().heat_haze > 0.0);
        assert_eq!(l.water_reflections, WaterReflections::Half);
    }
}
