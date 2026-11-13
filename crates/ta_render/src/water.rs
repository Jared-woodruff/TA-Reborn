//! Water surface at sea level.
//!
//! TA has one global sea level per map (`TntMap::sea_level`); all terrain below it is under
//! water. The painted tile art already depicts the sea bed and the water colour, so the
//! surface is drawn as a translucent layer over the finished opaque scene: per-fragment
//! depth from the heightmap, procedural band-limited waves, Fresnel reflection of the sky and
//! (perspective camera) of the world above the water (see [`crate::reflection`]), sun
//! glitter, point light sparkle, a depth-based tint that keeps shallow water clear, and foam
//! along the waterline (see `shaders/water.wgsl`). The terrain shader adds the matching
//! caustics on the sea bed.
//!
//! Geometry: one quad per terrain chunk that has any ground below sea level, culled with the
//! terrain's visible chunk range. Quads share the terrain's vertex grid and all shading is a
//! function of world position only, so chunk borders are seamless.
//!
//! The water is drawn after the opaque geometry (terrain, decals, units, billboards) and
//! before smoke and additive effects, with the depth test on and depth writes off, in a pass
//! of its own that reads the finished opaque scene ([`WaterScene`]): the water column's depth
//! comes from the heightmap, and what stands in the water (a hull, a pontoon, a wading unit)
//! from the scene's depth, which works with MSAA and in both camera modes; what lies under the
//! surface is shown bent by the waves.
//!
//! Not every TA "sea" is water. At load time the renderer measures the painted colour of the
//! sea and of the beach next to it ([`PaintedSea`]): the beach colour lets the foam follow the
//! painted shoreline (where it agrees with the heightmap's waterline), and the sea colour
//! classifies the [`Fluid`]. Red-orange seas are lava (TA's lava worlds) and get an emissive
//! crust-and-cracks surface; near-white ones are the cloud/mist voids of slate worlds and get
//! no surface; every other colour is water whose body colour follows the painting (blue seas,
//! green acid, purple crystal pools...). The game turns the water of acid worlds into
//! [`Fluid::Acid`], from their map's flag (the paint cannot tell acid from a dark green sea).

use glam::{UVec2, Vec2, Vec3};
use ta_formats::image::Palette;
use ta_formats::tnt::{TILE_SIZE, TntMap};

use crate::terrain::{CHUNK_CELLS, HeightField, TerrainGpu};
use crate::util::{Target, srgb_to_linear_lut};

/// What fills a map below its sea level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fluid {
    /// Water of any painted hue.
    Water,
    /// The acid of TA's acid worlds (the OTA's `waterdoesdamage`): shaded like water, but its
    /// own colour tints what it mirrors, it is murkier, its waves are slower and heavier, and
    /// rafts of scum drift on it. Never classified from the paint.
    Acid,
    /// Molten rock (TA's lava worlds): emissive crust and cracks, glow on the nearby rock.
    Lava,
    /// A painted void such as the mist seas of slate worlds: no surface is drawn.
    Void,
}

impl Fluid {
    /// Classifies a painted sea colour (linear RGB).
    pub fn classify(sea: [f32; 3]) -> Fluid {
        // Hue / saturation / value of the displayed (sRGB) colour.
        let s = sea.map(|c| crate::util::linear_to_srgb(c) as f32 / 255.0);
        let max = s[0].max(s[1]).max(s[2]);
        let min = s[0].min(s[1]).min(s[2]);
        let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
        let hue = if max == min {
            0.0
        } else if max == s[0] {
            (60.0 * (s[1] - s[2]) / (max - min)).rem_euclid(360.0)
        } else if max == s[1] {
            60.0 * (s[2] - s[0]) / (max - min) + 120.0
        } else {
            60.0 * (s[0] - s[1]) / (max - min) + 240.0
        };
        if max > 0.35 && sat > 0.6 && !(45.0..=340.0).contains(&hue) {
            Fluid::Lava
        } else if max > 0.8 && sat < 0.3 {
            Fluid::Void
        } else {
            Fluid::Water
        }
    }
}

/// Appearance of the water surface. Part of [`Lighting`](crate::Lighting).
#[derive(Clone, Debug)]
pub struct WaterSettings {
    /// Draw the water surface (and the sea bed effects) on maps that have water.
    pub enabled: bool,
    /// What fills the map below sea level. `None` classifies it from the painted art (see
    /// [`Fluid::classify`]); the game can force it, e.g. from the OTA's `lavaworld=1`.
    pub fluid: Option<Fluid>,
    /// Colour of light scattered back out of deep water, or the melt colour of lava (linear
    /// RGB). `None` derives it from the map's painted sea, so every world keeps its hue.
    pub color: Option<[f32; 3]>,
    /// View-path distance through the water (world units) over which ~63% of the light
    /// from the sea bed is absorbed. Smaller values give murkier water.
    pub clarity: f32,
    /// Upper bound on how much the water body covers the painted sea bed (0..1). Keeps the
    /// original art readable in deep water.
    pub opacity: f32,
    /// Wave normal strength: 0 = mirror-flat, 1 = default breeze, 2 = rough sea.
    pub wave_strength: f32,
    /// Shoreline foam intensity (0 disables foam).
    pub foam: f32,
    /// Strength of the refraction (what lies under the water shifted by the waves) and of the
    /// caustics on the sea bed (0 disables both).
    pub caustics: f32,
    /// How strongly the water mirrors the world above it (with
    /// [`Lighting::water_reflections`](crate::Lighting::water_reflections) on): reflectance added
    /// to the Fresnel term where something is mirrored, 0..1. At 0 the reflections are
    /// physically faint from TA's steep views (a few percent); 1 is a mirror. The sky's
    /// reflection keeps the Fresnel term alone, so the sea's colour does not change.
    pub reflectivity: f32,
}

impl Default for WaterSettings {
    fn default() -> Self {
        WaterSettings {
            enabled: true,
            fluid: None,
            color: None,
            clarity: 38.0,
            opacity: 0.5,
            wave_strength: 1.0,
            foam: 1.0,
            caustics: 1.0,
            reflectivity: 0.35,
        }
    }
}

/// Body colour used when a map's painted sea could not be measured.
const DEFAULT_WATER_COLOR: [f32; 3] = [0.012, 0.066, 0.16];

/// The water render pipelines (share the terrain bind group layout as group 1; group 2 holds
/// the reflection, see reflection.rs; group 3 the scene behind the water, see [`WaterScene`]).
pub struct WaterPipeline {
    pub(crate) pipeline: wgpu::RenderPipeline,
    /// Sea level over the extended ground beyond the map (sky.rs), drawn with the skirt mesh.
    pub(crate) outer: wgpu::RenderPipeline,
    /// The heat haze's hot air over lava, on the map and beyond it (alpha only, see heat.rs).
    pub(crate) heat: wgpu::RenderPipeline,
    pub(crate) heat_outer: wgpu::RenderPipeline,
}

/// Water shader: terrain bindings (heightmap for the water depth, art for the painted
/// shoreline), noise, procedural waves and foam, the sky model (reflections), then the water
/// entry points. `common.wgsl` is prepended by [`crate::util::shader`]. The water beyond the
/// map's edges shades as [`shader_source_for`] with `far_land` has it.
#[cfg(test)]
pub(crate) fn shader_source() -> String {
    shader_source_for(true)
}

/// The water shader for the water beyond the map's edges (`far_land`: the outer pipelines) or
/// for the map's own, without what only the water beyond it needs (as
/// [`crate::terrain::shader_source_for`]).
pub(crate) fn shader_source_for(far_land: bool) -> String {
    format!(
        "const FAR_LAND: bool = {far_land};\n{}\n{}\n{}\n{}\n{}\n{}",
        include_str!("shaders/terrain_common.wgsl"),
        include_str!("shaders/noise.wgsl"),
        include_str!("shaders/water_common.wgsl"),
        include_str!("shaders/clouds.wgsl"),
        include_str!("shaders/sky_common.wgsl"),
        include_str!("shaders/water.wgsl")
    )
}

/// `source` (the water shader) for a scene drawn with `samples` antialiasing samples: the
/// scene's depth it reads is multisampled, or not (its pipelines set `SCENE_SAMPLES`).
pub(crate) fn scene_source(source: &str, samples: u32) -> String {
    if samples > 1 { source.to_string() } else { source.replace("texture_depth_multisampled_2d", "texture_depth_2d") }
}

/// What the water reads of the scene behind it (group 3 of the water pipelines): the opaque
/// scene's depth as drawn, every sample of it, and its colour resolved into a copy. With the
/// water in view the renderer finishes the opaque scene in a pass of its own, resolved into the
/// copy (copied into it, without antialiasing), and draws the water and everything after it in
/// a second pass that keeps the depth read-only (`Renderer::draw`): so the water knows what
/// lies how far under it at each pixel, and shows it bent by the waves. Beside them the wake
/// map, half the scene's size, which the wakes of ships and hovercraft are drawn into before
/// the water (see [`crate::wakes`]).
pub(crate) struct WaterScene {
    pub layout: wgpu::BindGroupLayout,
    sized: Option<SceneTargets>,
}

/// What [`WaterScene`] made for a scene: the copy of its colour, the wake map, their bind group
/// with the depth, and the depth target and scene size they were made for.
struct SceneTargets {
    color: Target,
    wakes: Target,
    group: wgpu::BindGroup,
    depth: wgpu::TextureView,
    size: (u32, u32),
}

impl WaterScene {
    pub fn new(device: &wgpu::Device, samples: u32) -> Self {
        let texture = |binding, sample_type, multisampled| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water scene"),
            entries: &[
                texture(0, wgpu::TextureSampleType::Depth, samples > 1),
                texture(1, wgpu::TextureSampleType::Float { filterable: true }, false),
                texture(2, wgpu::TextureSampleType::Float { filterable: true }, false),
            ],
        });
        WaterScene { layout, sized: None }
    }

    /// Makes the copy of the colour and the wake map for a scene of `size` whose depth target
    /// is `depth` (again when either changed).
    pub fn prepare(&mut self, device: &wgpu::Device, depth: &wgpu::TextureView, size: (u32, u32)) {
        if self.sized.as_ref().is_some_and(|s| s.size == size && s.depth == *depth) {
            return;
        }
        use wgpu::TextureUsages as U;
        let color = Target::new(device, "water scene", size.0, size.1, crate::util::HDR_FORMAT, 1, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_DST);
        let wakes = Target::new(device, "wake map", size.0.div_ceil(2), size.1.div_ceil(2), crate::wakes::WAKE_FORMAT, 1, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water scene"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&color.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&wakes.view) },
            ],
        });
        self.sized = Some(SceneTargets { color, wakes, group, depth: depth.clone(), size });
    }

    /// The copy of the scene's colour, the bind group and the wake map (once prepared).
    pub fn prepared(&self) -> Option<(&Target, &wgpu::BindGroup, &Target)> {
        self.sized.as_ref().map(|s| (&s.color, &s.group, &s.wakes))
    }
}

/// Colours measured from a map's painted art at load time.
///
/// The shaders project art colours onto the axis between `sea` and `shore` (in a roughly
/// perceptual space) to find the *painted* shoreline, which the foam follows: the heightmap's
/// sea-level contour is quantised to 16 px cells and may sit a few pixels off the painting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintedSea {
    /// Mean painted colour of the sea well away from any shore, deep where the map has deep
    /// water (linear RGB).
    pub sea: [f32; 3],
    /// Mean painted colour of the beach 2-4 cells from the waterline (linear RGB), when it
    /// differs enough from the sea to locate the painted shoreline.
    pub shore: Option<[f32; 3]>,
    /// The liquid, classified from `sea`.
    pub fluid: Fluid,
}

/// Minimum distance between the sea and beach colours in sqrt-linear space for the painted
/// shoreline to be trusted (the darkest stock worlds, lush and acid, measure 0.16 to 0.3).
const MIN_SHORE_CONTRAST: f32 = 0.12;

impl PaintedSea {
    /// Measures the reference colours. `None` when too little open sea is visible in the art.
    pub fn measure(map: &TntMap, palette: &Palette, field: &HeightField) -> Option<PaintedSea> {
        let lut = srgb_to_linear_lut();
        let linear: Vec<Vec3> = (0..=255u8)
            .map(|i| {
                let [r, g, b] = palette.rgb(i);
                Vec3::new(lut[r as usize], lut[g as usize], lut[b as usize])
            })
            .collect();
        let (art_w, art_h) = ((map.width / 2) as i32 * TILE_SIZE as i32, (map.height / 2) as i32 * TILE_SIZE as i32);
        let tiles_x = (map.width / 2) as usize;
        // Mean colour of the 8x8 art block centred on art pixel (ax, ay).
        let block = |ax: i32, ay: i32| -> Option<Vec3> {
            if ax < 4 || ay < 4 || ax + 4 > art_w || ay + 4 > art_h {
                return None;
            }
            let mut sum = Vec3::ZERO;
            for y in ay - 4..ay + 4 {
                for x in ax - 4..ax + 4 {
                    let tile = map.tile_map[(y as usize / TILE_SIZE) * tiles_x + x as usize / TILE_SIZE] as usize;
                    let index = map.tiles.get(tile)?[(y as usize % TILE_SIZE) * TILE_SIZE + x as usize % TILE_SIZE];
                    sum += linear[index as usize];
                }
            }
            Some(sum / 64.0)
        };
        let sea_level = field.sea_level;
        let (w, h) = (field.width as usize, field.height as usize);
        let wet: Vec<bool> = field.heights.iter().map(|&v| (v as f32) < sea_level).collect();
        let dist = shore_distance(&wet, w, h, 6);
        // Deep sea well away from any shore, and beach 2-4 cells from the waterline (past the
        // mixed pixels of the painted edge itself). Every other vertex is plenty for a mean.
        // Shallow seas have no deep water (Cluster Freak and Temblorian Mist paint their mist on
        // a bed at height 0 under a sea level of 10): there the open sea away from the shore, at
        // any depth, stands in for it.
        let mut sea = Vec::new();
        let mut open = Vec::new();
        let mut shore = Vec::new();
        for z in (0..h).step_by(2) {
            for x in (0..w).step_by(2) {
                let i = z * w + x;
                let height = field.heights[i] as f32;
                let art = (x as i32 * 16, z as i32 * 16 - (height * 0.5) as i32);
                if wet[i] && dist[i] >= 3 {
                    let b = block(art.0, art.1);
                    if height < sea_level - 12.0 {
                        sea.extend(b);
                    }
                    open.extend(b);
                } else if !wet[i] && (2..=4).contains(&dist[i]) {
                    shore.extend(block(art.0, art.1));
                }
            }
        }
        let sea = if sea.len() >= 8 { sea } else { open };
        if sea.len() < 8 {
            return None;
        }
        let mean = |v: &[Vec3]| v.iter().sum::<Vec3>() / v.len() as f32;
        let sea = mean(&sea);
        let shore = (shore.len() >= 8)
            .then(|| mean(&shore))
            .filter(|&s| (sqrt3(sea) - sqrt3(s)).length() > MIN_SHORE_CONTRAST)
            .map(|s| s.to_array());
        Some(PaintedSea { sea: sea.to_array(), shore, fluid: Fluid::classify(sea.to_array()) })
    }

    /// Reference colours for the shader's painted-water mask: (sea, scale) and (shore, 1),
    /// both in sqrt-linear space; zeros when the shoreline cannot be located. The scale makes
    /// the mask reach 1 at 70% of the way from the beach to the deep-sea colour, so lighter
    /// painted shallows still count as water.
    pub(crate) fn shore_uniforms(&self) -> ([f32; 4], [f32; 4]) {
        match self.shore {
            Some(shore) => {
                let (a, b) = (sqrt3(Vec3::from(self.sea)), sqrt3(Vec3::from(shore)));
                let inv = 1.0 / (0.7 * (a - b).length_squared().max(1e-4));
                ([a.x, a.y, a.z, inv], [b.x, b.y, b.z, 1.0])
            }
            None => ([0.0; 4], [0.0; 4]),
        }
    }
}

fn sqrt3(v: Vec3) -> Vec3 {
    Vec3::new(v.x.max(0.0).sqrt(), v.y.max(0.0).sqrt(), v.z.max(0.0).sqrt())
}

impl WaterPipeline {
    /// The water's pipelines; `terrain_layout`: the terrain's bind group's (see
    /// [`crate::terrain::TerrainPipeline::bind_group_layout`]), which they share.
    pub fn new(
        device: &wgpu::Device,
        frame_layout: &wgpu::BindGroupLayout,
        terrain_layout: &wgpu::BindGroupLayout,
        reflection_layout: &wgpu::BindGroupLayout,
        scene_layout: &wgpu::BindGroupLayout,
        hdr_format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let shader = crate::util::shader(device, "water", &scene_source(&shader_source_for(false), samples));
        let outer_shader = crate::util::shader(device, "water beyond the map", &scene_source(&shader_source_for(true), samples));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water"),
            bind_group_layouts: &[Some(frame_layout), Some(terrain_layout), Some(reflection_layout), Some(scene_layout)],
            immediate_size: 0,
        });
        let chunk_instances = wgpu::VertexBufferLayout {
            array_stride: 8,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Uint32x2],
        };
        // (`shader`: the map's water's or the water's beyond it.)
        let make = |label: &str, shader: &wgpu::ShaderModule, vs: &str, fs: &str, buffers: &[Option<wgpu::VertexBufferLayout<'_>>], blend: wgpu::BlendState, write_mask: wgpu::ColorWrites| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: shader, entry_point: Some(vs), buffers, compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                // Depth test against the opaque scene, but never occlude what is drawn later.
                depth_stencil: Some(crate::util::depth_state(false)),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format: hdr_format, blend: Some(blend), write_mask })],
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &[("SCENE_SAMPLES", samples as f64)], ..Default::default() },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let (over, all) = (wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING, wgpu::ColorWrites::ALL);
        let (heat, alpha) = (crate::heat::HEAT_BLEND, wgpu::ColorWrites::ALPHA);
        let chunks = [Some(chunk_instances)];
        let skirt = [Some(crate::sky::SKIRT_VERTEX_LAYOUT)];
        // (Side by side where their shaders take long to compile: see `util::side_by_side`.)
        let builds: [&(dyn Fn() -> wgpu::RenderPipeline + Sync); 4] = [
            &|| make("water", &shader, "vs_water", "fs_water", &chunks, over, all),
            &|| make("water beyond the map", &outer_shader, "vs_water_outer", "fs_water_outer", &skirt, over, all),
            &|| make("lava heat", &shader, "vs_water", "fs_heat_water", &chunks, heat, alpha),
            &|| make("lava heat beyond the map", &outer_shader, "vs_water_outer", "fs_heat_outer", &skirt, heat, alpha),
        ];
        let [pipeline, outer, heat, heat_outer] = crate::util::side_by_side(builds);
        WaterPipeline { pipeline, outer, heat, heat_outer }
    }
}

/// Per-map water geometry: which terrain chunks are (partly) under water.
pub struct WaterSurface {
    pub sea_level: f32,
    /// Colours measured from the painted sea and beach (`None` if too little sea is painted).
    pub painted: Option<PaintedSea>,
    chunks: UVec2,
    wet: Vec<bool>,
    instance_buffer: wgpu::Buffer,
    visible: u32,
}

impl WaterSurface {
    /// Returns `None` when the map has no water (sea level ≤ 0 or no ground below it).
    pub fn new(device: &wgpu::Device, field: &HeightField, map: &TntMap, palette: &Palette) -> Option<Self> {
        let wet = wet_chunks(field)?;
        let chunks = UVec2::new(field.width.div_ceil(CHUNK_CELLS), field.height.div_ceil(CHUNK_CELLS));
        let count = wet.iter().filter(|&&w| w).count();
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water chunks"),
            size: (chunks.x * chunks.y * 8) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let painted = PaintedSea::measure(map, palette, field);
        log::info!("water: sea level {}, {count} of {} chunks under water, painted {:?}", field.sea_level, wet.len(), painted);
        Some(WaterSurface { sea_level: field.sea_level, painted, chunks, wet, instance_buffer, visible: 0 })
    }

    /// The liquid to render: the settings' override, else the painted classification.
    pub fn fluid(&self, settings: &WaterSettings) -> Fluid {
        settings.fluid.or(self.painted.map(|p| p.fluid)).unwrap_or(Fluid::Water)
    }

    /// How much the painted sea is a flat, bright, electric fill rather than a sea's colour
    /// (0..1; Brain Coral's and the ice worlds' royal blue 1, every other stock sea 0): such a
    /// sea's body is graded to a deep version of its hue and covers more of the painted bed.
    pub fn paint_fill(&self) -> f32 {
        self.painted.filter(|p| p.fluid != Fluid::Lava).map_or(0.0, |p| paint_fill(p.sea))
    }

    /// Upper bound on how much the water body covers the painted bed (see
    /// [`WaterSettings::opacity`]), raised for electric painted seas ([`Self::paint_fill`]).
    pub fn opacity(&self, settings: &WaterSettings) -> f32 {
        let o = settings.opacity.clamp(0.0, 1.0);
        o + (0.75 - o).max(0.0) * self.paint_fill()
    }

    /// Body colour (water) or melt colour normalised to a peak of 1 (lava), linear RGB.
    pub fn body_color(&self, settings: &WaterSettings) -> [f32; 3] {
        let painted = self.painted.map(|p| p.sea);
        match self.fluid(settings) {
            Fluid::Lava => {
                let c = Vec3::from(settings.color.or(painted).unwrap_or([0.46, 0.012, 0.0]));
                (c / c.max_element().max(1e-3)).to_array()
            }
            // Slightly deeper than the painted average: the body adds depth, not a new hue (an
            // electric fill is graded to a deep version of it).
            _ => settings.color.or(painted.map(|c| graded_sea(c).map(|v| v * 0.8))).unwrap_or(DEFAULT_WATER_COLOR),
        }
    }

    /// Number of chunks drawn this frame.
    pub fn visible_chunks(&self) -> u32 {
        self.visible
    }

    /// Selects wet chunks overlapping the visible ground rectangle (same rule as the terrain).
    pub fn cull(&mut self, queue: &wgpu::Queue, min: Vec2, max: Vec2) {
        let size = CHUNK_CELLS as f32 * 16.0;
        let x0 = ((min.x / size).floor() as i32).clamp(0, self.chunks.x as i32) as u32;
        let x1 = ((max.x / size).ceil() as i32).clamp(0, self.chunks.x as i32) as u32;
        let z0 = ((min.y / size).floor() as i32 - 1).clamp(0, self.chunks.y as i32) as u32;
        let z1 = ((max.y / size).ceil() as i32 + 1).clamp(0, self.chunks.y as i32) as u32;
        let mut list: Vec<[u32; 2]> = Vec::new();
        for z in z0..z1 {
            for x in x0..x1 {
                if self.wet[(z * self.chunks.x + x) as usize] {
                    list.push([x, z]);
                }
            }
        }
        if !list.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&list));
        }
        self.visible = list.len() as u32;
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, pipeline: &'a WaterPipeline, terrain: &'a TerrainGpu) {
        self.draw_with(pass, &pipeline.pipeline, terrain);
    }

    /// Draws the wet chunks with `pipeline` (the water's own, or the lava heat's).
    pub(crate) fn draw_with<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, pipeline: &'a wgpu::RenderPipeline, terrain: &'a TerrainGpu) {
        if self.visible == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(1, terrain.bind_group(), &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..6, 0..self.visible);
    }
}

/// The displayed (sRGB, 0..1) hue, saturation and value of a linear colour.
fn hsv(c: [f32; 3]) -> (f32, f32, f32) {
    let s = c.map(|v| crate::util::linear_to_srgb(v) as f32 / 255.0);
    let max = s[0].max(s[1]).max(s[2]);
    let min = s[0].min(s[1]).min(s[2]);
    let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
    let hue = if max == min {
        0.0
    } else if max == s[0] {
        (60.0 * (s[1] - s[2]) / (max - min)).rem_euclid(360.0)
    } else if max == s[1] {
        60.0 * (s[2] - s[0]) / (max - min) + 120.0
    } else {
        60.0 * (s[0] - s[1]) / (max - min) + 240.0
    };
    (hue, sat, max)
}

/// How much a painted sea colour (linear) is a flat, bright, saturated fill (see
/// [`WaterSurface::paint_fill`]): saturated and bright at once, as no deep sea is.
fn paint_fill(sea: [f32; 3]) -> f32 {
    let (_, sat, value) = hsv(sea);
    let step = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    step(0.65, 0.8, sat) * step(0.6, 0.85, value)
}

/// A painted sea colour (linear) as a sea's body: an electric fill becomes a deep, less
/// saturated version of its hue; any other colour is kept.
fn graded_sea(sea: [f32; 3]) -> [f32; 3] {
    let k = paint_fill(sea);
    if k <= 0.0 {
        return sea;
    }
    let (hue, sat, value) = hsv(sea);
    let (sat, value) = (sat + (0.72 - sat) * k, value + (0.45 - value) * k);
    // HSV to sRGB, then linear.
    let f = |n: f32| {
        let k = (n + hue / 60.0).rem_euclid(6.0);
        value - value * sat * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    let lut = srgb_to_linear_lut();
    [f(5.0), f(3.0), f(1.0)].map(|v| lut[(v.clamp(0.0, 1.0) * 255.0).round() as usize])
}

/// How far (world units) [`shore_distances`] measures: further from the waterline they read
/// as this (the surf and the water's fade at its edge never reach as far).
pub(crate) const SHORE_REACH: f32 = 32.0;

/// Signed distance from heightmap vertices to the waterline: the sea-level contour of the
/// terrain mesh (each cell split along its (0,0)-(1,1) diagonal, as the mesh and
/// `mesh_height` are), in world units, positive under water and negative on land, saturating
/// at ±[`SHORE_REACH`]. Encoded for the shaders as 128 + 4 × distance (a quarter unit a step).
/// The vertices of `rect` (x0, z0, x1, z1), row by row, of a `width` × `height` map whose
/// heights are `heights` (all of them: a vertex's distance depends on the cells around it).
///
/// The shaders filter this bilinearly: across a coast the distance is about linear, so the
/// filtered field keeps both its zero on the mesh's waterline and its unit slope, where the
/// water depth over the ground's slope (what the shaders had before) runs away on flat
/// shelves just under the water.
pub(crate) fn shore_distances(width: u32, height: u32, heights: &[f32], sea: f32, rect: [u32; 4]) -> Vec<u8> {
    let (w, h) = (width as i32, height as i32);
    let level = |x: i32, z: i32| heights[(z * w + x) as usize] - sea;
    // The contour's segment across a triangle (corners in cells), if the water meets it.
    let segment = |tri: [(i32, i32); 3]| -> Option<(Vec2, Vec2)> {
        let mut ends = [Vec2::ZERO; 2];
        let mut found = 0;
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            let (va, vb) = (level(a.0, a.1), level(b.0, b.1));
            if (va < 0.0) != (vb < 0.0) && found < 2 {
                let (pa, pb) = (Vec2::new(a.0 as f32, a.1 as f32), Vec2::new(b.0 as f32, b.1 as f32));
                ends[found] = pa + (pb - pa) * (va / (va - vb));
                found += 1;
            }
        }
        (found == 2).then_some((ends[0], ends[1]))
    };
    let mut out = Vec::with_capacity(((rect[2] - rect[0]) * (rect[3] - rect[1])) as usize);
    for z in rect[1] as i32..rect[3] as i32 {
        for x in rect[0] as i32..rect[2] as i32 {
            let wet = level(x, z) < 0.0;
            // Segments within reach lie in the 4 x 4 cells around the vertex, and only where
            // their corners are not all on one side.
            let (x0, x1, z0, z1) = ((x - 2).max(0), (x + 2).min(w - 1), (z - 2).max(0), (z + 2).min(h - 1));
            let mixed = (z0..=z1).any(|vz| (x0..=x1).any(|vx| (level(vx, vz) < 0.0) != wet));
            let mut best = SHORE_REACH;
            if mixed {
                let p = Vec2::new(x as f32, z as f32);
                for cz in z0..z1 {
                    for cx in x0..x1 {
                        let c = [(cx, cz), (cx + 1, cz), (cx + 1, cz + 1), (cx, cz + 1)];
                        for tri in [[c[0], c[1], c[2]], [c[0], c[2], c[3]]] {
                            if let Some((a, b)) = segment(tri) {
                                let ab = b - a;
                                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
                                best = best.min(p.distance(a + ab * t) * 16.0);
                            }
                        }
                    }
                }
            }
            let d = if wet { best } else { -best };
            out.push((128.0 + d * 4.0).round().clamp(0.0, 255.0) as u8);
        }
    }
    out
}

/// Chebyshev distance in cells (1 = next to it, capped at `cap`) from every heightmap vertex
/// to the nearest vertex on the other side of sea level. Two-pass distance transform.
fn shore_distance(wet: &[bool], w: usize, h: usize, cap: u8) -> Vec<u8> {
    let at = |x: i64, z: i64| (x >= 0 && z >= 0 && (x as usize) < w && (z as usize) < h).then(|| z as usize * w + x as usize);
    let mut d = vec![cap; w * h];
    for z in 0..h as i64 {
        for x in 0..w as i64 {
            let i = at(x, z).unwrap_or(0);
            let differs = (-1..=1).any(|dz| (-1..=1).any(|dx| at(x + dx, z + dz).is_some_and(|j| wet[j] != wet[i])));
            if differs {
                d[i] = 1;
            }
        }
    }
    let mut relax = |x: i64, z: i64, neighbours: [(i64, i64); 4]| {
        let i = at(x, z).unwrap_or(0);
        for (dx, dz) in neighbours {
            if let Some(j) = at(x + dx, z + dz) {
                d[i] = d[i].min(d[j].saturating_add(1));
            }
        }
    };
    for z in 0..h as i64 {
        for x in 0..w as i64 {
            relax(x, z, [(-1, 0), (-1, -1), (0, -1), (1, -1)]);
        }
    }
    for z in (0..h as i64).rev() {
        for x in (0..w as i64).rev() {
            relax(x, z, [(1, 0), (1, 1), (0, 1), (-1, 1)]);
        }
    }
    d
}

/// Per chunk (row-major, `CHUNK_CELLS` cells square): does any mesh vertex of the chunk lie
/// below sea level? `None` if no chunk does.
pub fn wet_chunks(field: &HeightField) -> Option<Vec<bool>> {
    if field.sea_level <= 0.0 || field.width == 0 || field.height == 0 {
        return None;
    }
    let (cx, cz) = (field.width.div_ceil(CHUNK_CELLS), field.height.div_ceil(CHUNK_CELLS));
    let mut wet = vec![false; (cx * cz) as usize];
    for z in 0..cz {
        for x in 0..cx {
            // The chunk's vertices span cells [c * CHUNK_CELLS, c * CHUNK_CELLS + CHUNK_CELLS].
            let below = (z * CHUNK_CELLS..=(z + 1) * CHUNK_CELLS).any(|vz| {
                (x * CHUNK_CELLS..=(x + 1) * CHUNK_CELLS).any(|vx| field.vertex(vx as i32, vz as i32) < field.sea_level)
            });
            wet[(z * cx + x) as usize] = below;
        }
    }
    wet.iter().any(|&w| w).then_some(wet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(width: u32, height: u32, sea: f32, f: impl Fn(u32, u32) -> u8) -> HeightField {
        let heights = (0..height).flat_map(|z| (0..width).map(move |x| (x, z))).map(|(x, z)| f(x, z)).collect();
        HeightField { width, height, heights, sea_level: sea, display: Vec::new() }
    }

    fn floats(heights: &[u8]) -> Vec<f32> {
        heights.iter().map(|&v| v as f32).collect()
    }

    /// A 64x64-cell map: land (height 90, palette index 1) west of x = 32 cells, sea bed
    /// (height 10, index 2) east of it, sea level 50.
    fn beach_map(land_rgb: [u8; 3], water_rgb: [u8; 3]) -> (TntMap, Palette, HeightField) {
        let (w, h) = (64u32, 64u32);
        let heights: Vec<u8> = (0..h).flat_map(|_| (0..w).map(|x| if x < 32 { 90 } else { 10 })).collect();
        let mut palette = Palette::default();
        palette.colors[1] = land_rgb;
        palette.colors[2] = water_rgb;
        let tiles = vec![Box::new([1u8; TILE_SIZE * TILE_SIZE]), Box::new([2u8; TILE_SIZE * TILE_SIZE])];
        // Tiles are 2x2 cells; ground at x < 32 cells is art x < 512 px = tile column < 16.
        let tile_map = (0..h / 2).flat_map(|_| (0..w / 2).map(|tx| if tx < 16 { 0u16 } else { 1 })).collect();
        let map = TntMap {
            width: w,
            height: h,
            sea_level: 50,
            tiles,
            tile_map,
            heights: heights.clone(),
            features: vec![ta_formats::tnt::FEATURE_NONE; (w * h) as usize],
            feature_names: Vec::new(),
            minimap: Default::default(),
            flags: Vec::new(),
        };
        let field = HeightField { width: w, height: h, heights, sea_level: 50.0, display: Vec::new() };
        (map, palette, field)
    }

    #[test]
    fn painted_sea_colours_are_measured_from_the_art() {
        let (map, palette, field) = beach_map([200, 190, 150], [20, 70, 120]);
        let painted = PaintedSea::measure(&map, &palette, &field).expect("sea is painted");
        let lut = srgb_to_linear_lut();
        let lin = |c: [u8; 3]| c.map(|v| lut[v as usize]);
        let shore = painted.shore.expect("distinct beach");
        for (got, want) in painted.sea.iter().zip(lin([20, 70, 120])).chain(shore.iter().zip(lin([200, 190, 150]))) {
            assert!((got - want).abs() < 1e-4, "{painted:?}");
        }
        assert_eq!(painted.fluid, Fluid::Water);
        // Water and beach painted alike: the painted edge cannot be located, the sea still can.
        let (map, palette, field) = beach_map([60, 70, 80], [62, 70, 84]);
        let painted = PaintedSea::measure(&map, &palette, &field).expect("sea is painted");
        assert!(painted.shore.is_none());
    }

    #[test]
    fn shallow_seas_are_still_measured() {
        // Cluster Freak: a mist sea painted on a bed at height 0 under a sea level of 10, so no
        // water is deep. Its mist must still be measured (unmeasured, the sea would be taken
        // for water, and the map's lava world flag would turn that into lava).
        let (map, palette, mut field) = beach_map([69, 73, 94], [220, 241, 254]);
        for h in field.heights.iter_mut().filter(|h| **h < 50) {
            *h = 0;
        }
        field.sea_level = 10.0;
        let painted = PaintedSea::measure(&map, &palette, &field).expect("a shallow sea is measured");
        assert_eq!(painted.fluid, Fluid::Void);
        assert!(painted.shore.is_some(), "the slate rock beside the mist is measured too");
    }

    #[test]
    fn electric_seas_are_graded_and_natural_ones_kept() {
        let lut = srgb_to_linear_lut();
        let lin = |c: [u8; 3]| c.map(|v| lut[v as usize]);
        // Brain Coral's royal blue: graded to a deep blue of the same hue.
        let brain = lin([42, 76, 225]);
        assert!(paint_fill(brain) > 0.99);
        let (hue, sat, value) = hsv(graded_sea(brain));
        let (hue0, _, _) = hsv(brain);
        assert!((hue - hue0).abs() < 3.0 && (sat - 0.72).abs() < 0.03 && (value - 0.45).abs() < 0.03, "{hue} {sat} {value}");
        // Every other stock sea is left as painted.
        for c in [[31, 81, 124], [13, 53, 90], [4, 50, 34], [23, 25, 45], [5, 17, 6], [57, 44, 105], [110, 67, 133]] {
            assert_eq!(paint_fill(lin(c)), 0.0, "{c:?}");
            assert_eq!(graded_sea(lin(c)), lin(c));
        }
    }

    #[test]
    fn fluids_are_classified_like_the_stock_worlds() {
        // Mean painted sea colours (sRGB) measured on the GOG maps.
        let lut = srgb_to_linear_lut();
        let lin = |c: [u8; 3]| c.map(|v| lut[v as usize]);
        let cases = [
            ([31, 81, 124], Fluid::Water),   // archipelago (Coast To Coast)
            ([42, 76, 225], Fluid::Water),   // green planet, ice
            ([13, 53, 90], Fluid::Water),    // lush
            ([4, 50, 34], Fluid::Water),     // acid
            ([23, 25, 45], Fluid::Water),    // crystal
            ([5, 17, 6], Fluid::Water),      // metal
            ([57, 44, 105], Fluid::Water),   // red planet
            ([110, 67, 133], Fluid::Water),  // red planet (Raid on Vetes)
            ([180, 29, 0], Fluid::Lava),     // lava worlds
            ([220, 241, 254], Fluid::Void),  // slate worlds' mist
        ];
        for (rgb, fluid) in cases {
            assert_eq!(Fluid::classify(lin(rgb)), fluid, "{rgb:?}");
        }
    }

    #[test]
    fn shore_distances_measure_to_the_mesh_waterline() {
        // Land at 90 west of vertex column 32, sea bed at 10 from it, sea level 50: the mesh
        // meets the water half way between columns 31 and 32.
        let f = field(64, 8, 50.0, |x, _| if x < 32 { 90 } else { 10 });
        let all = [0, 0, 64, 8];
        let d = shore_distances(64, 8, &floats(&f.heights), 50.0, all);
        let at = |x: usize| (d[4 * 64 + x] as f32 - 128.0) / 4.0;
        assert_eq!(at(32), 8.0, "half a cell out to sea");
        assert_eq!(at(31), -8.0, "half a cell up the beach");
        assert_eq!(at(33), 24.0);
        assert_eq!(at(36), 31.75, "saturated out at sea");
        assert_eq!(at(10), -32.0, "saturated inland");
        // A gentle shelf: the depth over the slope would put this vertex 200 units out.
        let f = field(64, 8, 50.0, |x, _| if x < 32 { 51 } else { 49 });
        let d = shore_distances(64, 8, &floats(&f.heights), 50.0, all);
        assert_eq!((d[4 * 64 + 33] as f32 - 128.0) / 4.0, 24.0);
        // A window of the map gives the same values as the whole map.
        let f = field(40, 40, 50.0, |x, z| if (x as i32 - 20).pow(2) + (z as i32 - 18).pow(2) < 60 { 80 } else { 20 });
        let whole = shore_distances(40, 40, &floats(&f.heights), 50.0, [0, 0, 40, 40]);
        let part = shore_distances(40, 40, &floats(&f.heights), 50.0, [10, 12, 30, 25]);
        for z in 12..25 {
            assert_eq!(&part[(z - 12) * 20..(z - 12) * 20 + 20], &whole[z * 40 + 10..z * 40 + 30]);
        }
    }

    #[test]
    fn shore_distance_counts_cells_to_the_other_side() {
        // One wet column at x = 3 in a 7x3 grid.
        let wet: Vec<bool> = (0..3).flat_map(|_| (0..7).map(|x| x == 3)).collect();
        let d = shore_distance(&wet, 7, 3, 6);
        assert_eq!(&d[0..7], &[3, 2, 1, 1, 1, 2, 3]);
    }

    #[test]
    fn dry_maps_have_no_water() {
        assert!(wet_chunks(&field(64, 64, 0.0, |_, _| 0)).is_none(), "sea level 0 means no water");
        assert!(wet_chunks(&field(64, 64, 40.0, |_, _| 50)).is_none(), "all ground above the sea");
    }

    #[test]
    fn only_chunks_touching_low_ground_are_wet() {
        // 96x64 cells = 3x2 chunks; a lake in the middle of chunk (1, 1).
        let f = field(96, 64, 40.0, |x, z| if (44..52).contains(&x) && (40..50).contains(&z) { 10 } else { 60 });
        let wet = wet_chunks(&f).unwrap();
        assert_eq!(wet, vec![false, false, false, false, true, false]);
        // A lake cell on a chunk border marks both neighbours (they share the vertex row).
        let f = field(96, 64, 40.0, |x, z| if x == 32 && z == 10 { 10 } else { 60 });
        let wet = wet_chunks(&f).unwrap();
        assert_eq!(wet, vec![true, true, false, false, false, false]);
    }
}
