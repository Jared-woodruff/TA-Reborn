//! Sky dome and the world beyond the map edges (perspective camera only).
//!
//! * **Sky** (`shaders/sky.wgsl`): one full-screen triangle on the far plane, drawn inside the
//!   MSAA scene pass after the opaque geometry with the depth test on and depth writes off, so
//!   only background pixels are shaded (and MSAA resolves the silhouettes against it). The sky
//!   is a gradient from the horizon haze (the colour distance fog tends to, including the sun's
//!   glow, so far ground meets the sky without a seam) to the zenith colour, a limb-darkened HDR
//!   sun disc that blooms, and a layer of soft drifting clouds (see `sky_common.wgsl`). Water
//!   reflects the same sky.
//! * **Beyond the map**: a ring mesh around the map rectangle (the "skirt", `SkirtMesh`)
//!   extends the ground towards the horizon. Its inner ring is exactly the terrain's boundary
//!   vertices (watertight seam); further rings grow geometrically and get sparser along the
//!   edges, stitched without T-junctions. Next to the map the ground there is the map's own
//!   mirror image (`terrain_common.wgsl`): each point shows the ground reflected into the map
//!   across the edges nearest to it, so whatever crosses an edge carries on (a hill cut by the
//!   edge closes as its own reflection, a coast bends back, open sea stays sea) and the seam is
//!   exact, its heights read from the heightmap's mips as coarsely as the rings are spaced.
//!   From a few hundred units out it is a far land of its own (`far_ground`), made after the
//!   map's ground round the nearest stretch of its edges (`crate::surroundings`): as much of it
//!   sea, the land as high and as rugged, painted in its colours, but its coasts and hills
//!   shaped by noise, so the map never sits among copies of itself (a metal world's far land is
//!   blocks of plating, the moon's craters). It gets a little darker (so the playable area
//!   stands out) and dissolves into the horizon haze. The skirt uses the terrain's fragment
//!   shader (built with the far land's code, `FAR_LAND`; the map's own without it), so the
//!   art's detail materials, HD surfaces, rock faces, lighting, caustics, foam and lava glow
//!   continue across the edge.
//!   Water and lava continue over it at sea level (the water pass draws the same mesh
//!   flattened, with the far shorelines resolved per pixel); painted voids stay empty (their
//!   mist continues, no surface is drawn), as on the map.
//! * **In a battle** ([`Beyond::Clouds`], which the game sets outside its films) none of that
//!   ground or water is drawn: below the horizon the sky pass draws a sea of dark clouds some
//!   way under the map instead (`beyond_void` in `sky.wgsl`, the frame uniform's `liquid.z`),
//!   drifting slowly, faintly lit from above, shaded along the map's edges, merging far out
//!   into an even dark that lies along the horizon as a bank. The battlefield stands out, as
//!   in Supreme Commander, and nothing drawn beyond it can shimmer or fight over depth.
//! * Everything above is off in the classic projection, which never shows the horizon: the
//!   frame uniform's `sky.w` flag is 0 there and every shader keeps its original path.
//!
//! Per map, the horizon haze and the derived zenith colour are tinted towards the map's mean
//! painted colour ([`SkySettings::map_tint`]), and each world has a sky of its own
//! ([`world_sky`] from its OTA's `planet`): a deep blue over deserts, smoke and ash over lava,
//! smog over metal and urban worlds, a sickly haze over acid, violet over crystal, no air at all
//! on the moon (a black sky with stars by day, no clouds, little haze, deep shade). Each map's
//! clouds lie where its own seed puts them.

use glam::{Vec2, Vec3};
use crate::gpu::CheckedBufferInit;

use crate::water::Fluid;

/// Appearance of the sky and of the ground beyond the map edges. Part of
/// [`Lighting`](crate::Lighting); used by the perspective camera only.
#[derive(Clone, Debug)]
pub struct SkySettings {
    /// Draw the sky and extend the ground (and any sea or lava) beyond the map to the horizon.
    /// When off, the map floats over [`Lighting::sky_color`](crate::Lighting::sky_color) as in
    /// the classic view.
    pub enabled: bool,
    /// Sky colour straight up (linear RGB). `None` derives it from the ambient colour, tinted
    /// like the haze by the map's painted colours.
    pub zenith: Option<[f32; 3]>,
    /// How strongly the map's mean painted colour tints the horizon haze (distance fog) and
    /// the derived zenith: 0 = exactly [`Lighting::fog_color`](crate::Lighting::fog_color).
    pub map_tint: f32,
    /// Cloud cover: 0 = clear sky, 1 = overcast.
    pub cloud_cover: f32,
    /// Cloud drift speed in world units per second.
    pub cloud_speed: f32,
    /// Angular radius of the sun disc in degrees (0 hides the disc; the glow remains).
    pub sun_size: f32,
    /// How far the ground continues beyond the map edges, in world units; it dissolves into
    /// the horizon haze over the outer half of this distance. 0 draws the sky only.
    pub horizon_extent: f32,
    /// What lies beyond the map's edges.
    pub beyond: Beyond,
}

/// What lies beyond the map's edges (perspective camera with the sky).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Beyond {
    /// The land and sea carried on to the horizon (see the module's notes): the films' world.
    #[default]
    Land,
    /// A sea of dark clouds under the map, the battlefield the focus: a battle's.
    Clouds,
}

impl Default for SkySettings {
    fn default() -> Self {
        SkySettings {
            enabled: true,
            zenith: None,
            map_tint: 0.3,
            cloud_cover: 0.4,
            cloud_speed: 30.0,
            sun_size: 0.7,
            horizon_extent: 60000.0,
            beyond: Beyond::Land,
        }
    }
}

/// Sky shader: noise, the cloud layer's shape and the sky model, then the sky pass entry
/// points. `common.wgsl` is prepended by [`crate::util::shader`].
pub(crate) fn shader_source() -> String {
    format!(
        "{}\n{}\n{}\n{}",
        include_str!("shaders/noise.wgsl"),
        include_str!("shaders/clouds.wgsl"),
        include_str!("shaders/sky_common.wgsl"),
        include_str!("shaders/sky.wgsl")
    )
}

/// The sky pass pipeline (frame bind group only).
pub(crate) struct SkyPipeline {
    pipeline: wgpu::RenderPipeline,
}

impl SkyPipeline {
    pub fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout, hdr_format: wgpu::TextureFormat, samples: u32) -> Self {
        let shader = crate::util::shader(device, "sky", &shader_source());
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky"),
            bind_group_layouts: &[Some(frame_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sky"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            // On the far plane: passes only where the depth buffer is still clear.
            depth_stencil: Some(crate::util::depth_state(false)),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_sky"),
                targets: &[Some(wgpu::ColorTargetState { format: hdr_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        SkyPipeline { pipeline }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.draw(0..3, 0..1);
    }
}

/// Vertex layout of the skirt mesh: world xz per vertex (heights come from the shaders).
pub(crate) const SKIRT_VERTEX_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: 8,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
};

/// The extended ground around one map: a ring mesh from the map rectangle out to
/// [`SkySettings::horizon_extent`]. Drawn with the terrain's skirt pipeline, and flattened to
/// sea level by the water's outer pipeline.
pub(crate) struct SkirtMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    /// The map size (world units) and reach it was built for.
    pub(crate) map_size: Vec2,
    pub(crate) reach: f32,
}

impl SkirtMesh {
    pub(crate) fn new(device: &wgpu::Device, map_size: Vec2, reach: f32) -> Self {
        let (vertices, indices) = skirt_geometry(map_size, reach);
        log::info!("skirt: {} vertices, {} triangles, reach {reach}", vertices.len(), indices.len() / 3);
        let vertex_buffer = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor {
            label: Some("skirt vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor {
            label: Some("skirt indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        SkirtMesh { vertex_buffer, index_buffer, index_count: indices.len() as u32, map_size, reach }
    }

    /// Draws the mesh with `pipeline` (the terrain's skirt or the water's outer pipeline) and
    /// the terrain bind group as group 1.
    pub(crate) fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, pipeline: &'a wgpu::RenderPipeline, terrain: &'a wgpu::BindGroup) {
        if self.index_count == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(1, terrain, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

/// Distances of the skirt's rings from the map edge: 0 (the map boundary itself), then steps
/// of one heightmap cell growing by 15% per ring (close enough to carry the mirrored relief,
/// which the shaders read about twice as coarsely), the last one exactly at `reach`.
fn ring_distances(reach: f32) -> Vec<f32> {
    let mut rings = vec![0.0f32];
    let mut d = 0.0f32;
    while d < reach {
        d = (d + (d * 0.15).max(16.0)).min(reach);
        rings.push(d);
    }
    rings
}

/// Spacing of the skirt's vertices along the map sides at ring distance `d`: one heightmap
/// cell (16 units) at the edge, doubling as the rings spread (about a tenth to a fifth of
/// `d`). Powers of two times 16, so every ring's columns include the next coarser ring's.
fn column_spacing(d: f32) -> f32 {
    let mut s = 16.0f32;
    while s * 2.0 <= (d * 0.2).max(16.0) {
        s *= 2.0;
    }
    s
}

/// One ring of the skirt: a closed loop at distance `d` around the `size` map rectangle
/// (straight sides, quarter-circle corners), as (loop parameter, position). The parameter runs
/// over [0, 8): the north side (going east) in [0, 1), the north-east corner in [1, 2), then
/// the east side, south-east corner, south side, south-west corner, west side and north-west
/// corner. Sides and arcs include their start and exclude their end; at `d` = 0 the arcs
/// collapse into the map corners, which the sides already start at.
fn skirt_ring(size: Vec2, d: f32) -> Vec<(f32, Vec2)> {
    use std::f32::consts::{FRAC_PI_2, PI};
    let s = column_spacing(d);
    let (w, h) = (size.x, size.y);
    // Positions along a side of length `len` at multiples of `s`, from its start (excluded end).
    let forward = |len: f32| (0..).map(move |k| k as f32 * s).take_while(move |&x| x < len);
    // The same positions walked from the far end (so the south and west sides, which run
    // backwards, reuse the other rings' columns).
    let backward = |len: f32| std::iter::once(len).chain((1..).map(move |k| (len / s).ceil() * s - k as f32 * s).take_while(|&x| x > 0.0));
    let segments = ((FRAC_PI_2 * d / s).ceil() as usize).clamp(2, 12);
    // Quarter circle around a map corner starting at angle `from` (in the xz plane, +x east,
    // +z south: north is -90°), as (fraction of the arc, position).
    let arc = |center: Vec2, from: f32| {
        (0..segments).filter(move |_| d > 0.0).map(move |k| {
            let f = k as f32 / segments as f32;
            let a = from + f * FRAC_PI_2;
            (f, center + Vec2::new(a.cos(), a.sin()) * d)
        })
    };
    let mut ring = Vec::new();
    ring.extend(forward(w).map(|x| (x / w, Vec2::new(x, -d))));
    ring.extend(arc(Vec2::new(w, 0.0), -FRAC_PI_2).map(|(f, p)| (1.0 + f, p)));
    ring.extend(forward(h).map(|z| (2.0 + z / h, Vec2::new(w + d, z))));
    ring.extend(arc(Vec2::new(w, h), 0.0).map(|(f, p)| (3.0 + f, p)));
    ring.extend(backward(w).map(|x| (4.0 + (w - x) / w, Vec2::new(x, h + d))));
    ring.extend(arc(Vec2::new(0.0, h), FRAC_PI_2).map(|(f, p)| (5.0 + f, p)));
    ring.extend(backward(h).map(|z| (6.0 + (h - z) / h, Vec2::new(-d, z))));
    ring.extend(arc(Vec2::ZERO, PI).map(|(f, p)| (7.0 + f, p)));
    ring
}

/// Triangulates the band between two closed rings (sorted by loop parameter, both starting
/// at 0) by walking them together, always advancing the ring whose next vertex comes first.
/// Shared columns give quads; where the outer ring is coarser, fans (no T-junctions).
fn zip_rings(indices: &mut Vec<u32>, inner: &[(f32, u32)], outer: &[(f32, u32)]) {
    let (na, nb) = (inner.len(), outer.len());
    let ta = |i: usize| if i >= na { 8.0 } else { inner[i].0 };
    let tb = |j: usize| if j >= nb { 8.0 } else { outer[j].0 };
    let va = |i: usize| inner[i % na].1;
    let vb = |j: usize| outer[j % nb].1;
    let (mut i, mut j) = (0, 0);
    while i < na || j < nb {
        if j >= nb || (i < na && ta(i + 1) <= tb(j + 1)) {
            indices.extend_from_slice(&[va(i), va(i + 1), vb(j)]);
            i += 1;
        } else {
            indices.extend_from_slice(&[va(i), vb(j + 1), vb(j)]);
            j += 1;
        }
    }
}

/// Vertices (world xz) and triangle indices of the skirt around a `size` map out to `reach`.
pub(crate) fn skirt_geometry(size: Vec2, reach: f32) -> (Vec<[f32; 2]>, Vec<u32>) {
    let mut vertices: Vec<[f32; 2]> = Vec::new();
    let mut indices = Vec::new();
    if reach <= 0.0 || size.x <= 0.0 || size.y <= 0.0 {
        return (vertices, indices);
    }
    let mut previous: Vec<(f32, u32)> = Vec::new();
    for d in ring_distances(reach) {
        let ring: Vec<(f32, u32)> = skirt_ring(size, d)
            .into_iter()
            .map(|(t, p)| {
                vertices.push(p.to_array());
                (t, vertices.len() as u32 - 1)
            })
            .collect();
        if !previous.is_empty() {
            zip_rings(&mut indices, &previous, &ring);
        }
        previous = ring;
    }
    (vertices, indices)
}

/// A world's sky (perspective camera): see [`world_sky`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldSky {
    /// Multipliers of the zenith and horizon haze colours derived from the map's light.
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    /// Smoke, ash or smog in the air: its colour (rgb, linear) and how far the haze (a) and the
    /// zenith (twice as dark) go towards it.
    pub smoke: [f32; 4],
    /// Multiplier of the cloud cover ([`SkySettings::cloud_cover`]).
    pub clouds: f32,
    /// Multiplier of the distance haze's density.
    pub haze: f32,
    /// No air (0..1): a black sky with stars by day, no clouds, little haze, deep shade.
    pub airless: f32,
    /// In space (0 or 1): no ground beyond the map, no horizon, the stars all round and below
    /// (and through the map's void cells).
    pub space: f32,
}

/// The sky of the world a map's OTA names as its `planet` (as the detail materials read it,
/// `terrain_detail::world_weights`); the plain sky for worlds it does not know.
pub fn world_sky(planet: &str) -> WorldSky {
    let p = planet.to_ascii_lowercase();
    let has = |w: &str| p.contains(w);
    let sky = |zenith: [f32; 3], horizon: [f32; 3], smoke: [f32; 4], clouds: f32, haze: f32| WorldSky { zenith, horizon, smoke, clouds, haze, airless: 0.0, space: 0.0 };
    let clear = [0.0; 4];
    if crate::terrain_detail::space_world(&p) {
        WorldSky { zenith: [0.01, 0.01, 0.015], horizon: [0.02, 0.02, 0.03], smoke: clear, clouds: 0.0, haze: 0.02, airless: 1.0, space: 1.0 }
    } else if has("lunar") || has("moon") || has("regolith") {
        WorldSky { zenith: [0.02, 0.02, 0.03], horizon: [0.06, 0.06, 0.07], smoke: clear, clouds: 0.0, haze: 0.05, airless: 1.0, space: 0.0 }
    } else if has("metal") || has("urban") {
        // A grey-brown smog under an overcast.
        sky([0.8, 0.78, 0.7], [0.96, 0.93, 0.88], [0.36, 0.34, 0.31, 0.45], 1.9, 1.35)
    } else if has("lava") || has("volcan") || has("basalt") {
        // Smoke and ash: a dim, dusky sky, an ash pall, the haze warm from below.
        sky([1.0, 0.9, 0.85], [1.0, 0.92, 0.86], [0.30, 0.16, 0.11, 0.75], 1.6, 1.5)
    } else if has("acid") {
        // A sickly yellow-green haze, murky cloud.
        sky([0.9, 0.95, 0.75], [1.0, 1.0, 0.85], [0.42, 0.45, 0.22, 0.4], 1.35, 1.4)
    } else if has("crystal") {
        // A violet twilight.
        sky([0.85, 0.7, 1.1], [0.98, 0.94, 1.04], clear, 0.5, 0.9)
    } else if has("slate") {
        // A cold grey overcast.
        sky([0.8, 0.84, 0.9], [0.95, 0.97, 1.0], [0.3, 0.32, 0.36, 0.2], 1.6, 1.15)
    } else if has("ice") || has("snow") || has("arctic") || has("frozen") || has("tundra") {
        sky([0.92, 0.97, 1.05], [1.0, 1.0, 1.03], clear, 0.8, 0.9)
    } else if has("red") || has("mars") || has("martian") {
        // Dust: a butterscotch sky.
        sky([1.05, 0.85, 0.75], [1.08, 0.96, 0.86], clear, 0.5, 1.15)
    } else if has("desert") || has("arid") || has("dune") {
        // A deep blue zenith over a hot, bleached horizon, hardly a cloud.
        sky([0.8, 0.9, 1.15], [1.04, 1.0, 0.93], clear, 0.25, 0.85)
    } else {
        // Green, lush, water and archipelago worlds, and the rest: a deeper blue and cumulus.
        sky([0.85, 0.92, 1.08], [0.97, 0.98, 1.0], clear, 1.1, 1.0)
    }
}

/// Colours of a map's atmosphere, derived from its painted art at load time.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MapAtmosphere {
    /// Mean painted colour of the whole map (linear RGB).
    pub average: Vec3,
    /// What fills the map below sea level, if anything.
    pub fluid: Option<Fluid>,
}

/// Horizon haze and zenith colours for the perspective sky (linear RGB), on `world`.
pub(crate) fn sky_colors(settings: &SkySettings, fog: Vec3, ambient: Vec3, atmosphere: Option<MapAtmosphere>, world: &WorldSky) -> (Vec3, Vec3) {
    let derived_zenith = ambient * Vec3::new(0.62, 0.85, 1.35);
    let Some(atm) = atmosphere else {
        return (fog, settings.zenith.map_or(derived_zenith, Vec3::from));
    };
    let (horizon, zenith) = map_sky(settings, fog, derived_zenith, atm);
    // The world's own sky.
    let mut horizon = horizon * Vec3::from(world.horizon);
    let mut zenith = zenith * Vec3::from(world.zenith);
    let smoke = Vec3::new(world.smoke[0], world.smoke[1], world.smoke[2]);
    horizon = horizon.lerp(smoke, world.smoke[3]);
    zenith = zenith.lerp(smoke * 0.5, (world.smoke[3] * 1.1).min(1.0));
    (horizon, settings.zenith.map_or(zenith, Vec3::from))
}

/// The sky's colours from the map's own colours: the haze and the zenith tinted by its painted
/// ground; over a lava sea on a world that does not say it is a lava world, smoke.
fn map_sky(settings: &SkySettings, fog: Vec3, derived_zenith: Vec3, atm: MapAtmosphere) -> (Vec3, Vec3) {
    let k = settings.map_tint.clamp(0.0, 1.0);
    // A dusty tint from the painted ground: its hue half desaturated, never greener than it
    // is red or blue (vegetation does not tint haze green), at unit luminance and with its
    // chroma limited.
    let luminance = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    let hue = atm.average / luminance(atm.average).max(1e-3);
    let mut dust = Vec3::ONE.lerp(hue, 0.5);
    dust.y = dust.y.min(dust.x.max(dust.z) * 1.02);
    let dust = (dust / luminance(dust).max(1e-3)).clamp(Vec3::splat(0.6), Vec3::splat(1.4));
    let mut horizon = fog * Vec3::ONE.lerp(dust, k);
    let mut zenith = derived_zenith * Vec3::ONE.lerp(dust, k * 0.6);
    if atm.fluid == Some(Fluid::Lava) {
        // Smoke and ash over the lava: a darker, warmer haze and a dim, dusky sky.
        let smoke = (k * 2.0).min(1.0);
        horizon = horizon.lerp(Vec3::new(0.34, 0.19, 0.13), smoke * 0.6);
        zenith = zenith.lerp(Vec3::new(0.11, 0.08, 0.09), smoke * 0.7);
    }
    (horizon, zenith)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Executes the production star field across cell and cube-face boundaries, then times
    /// its full-screen cost against the same shader restricted to the containing cell.
    #[test]
    #[ignore = "requires a GPU; run explicitly for rendering changes"]
    fn gpu_stars_cross_cell_and_face_edges() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
        println!("star field GPU: {}", adapter.get_info().name);
        let timestamp = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: if timestamp { wgpu::Features::TIMESTAMP_QUERY } else { wgpu::Features::empty() },
            ..Default::default()
        })).unwrap();
        let sky = include_str!("shaders/sky_common.wgsl");
        let start = sky.find("fn star_grid").unwrap();
        let end = sky.find("// Where a view ray meets").unwrap();
        let source = format!(r#"
{}
{}
struct Frame {{ night: vec4<f32>, celestial: vec4<f32>, world: vec4<f32>, liquid: vec4<f32>, }};
var<private> frame: Frame;
{}
@group(0) @binding(0) var<storage, read_write> results: array<vec4<f32>>;
@compute @workgroup_size(1)
fn check() {{
    frame.night.a = 1.0;
    let pixel = 0.002750675; // 720p, 70-degree lens at the centre of the image
    results[0] = vec4<f32>(star_grid(vec2<f32>(-24.0001, 85.3088235) / 260.0, 3u, pixel, 0.0), 1.0);
    results[1] = vec4<f32>(star_grid(vec2<f32>(-23.9999, 85.3088235) / 260.0, 3u, pixel, 0.0), 1.0);
    results[2] = vec4<f32>(star_field(normalize(vec3<f32>(1.0 - 1e-6, 1.0, 35.5127451 / 260.0)), pixel), 1.0);
    results[3] = vec4<f32>(star_field(normalize(vec3<f32>(1.0 + 1e-6, 1.0, 35.5127451 / 260.0)), pixel), 1.0);
    results[4] = vec4<f32>(star_grid(vec2<f32>(-24.0, 85.3088235) / 260.0, 3u, 0.02, 0.0), 1.0);
}}
@fragment
fn picture(in: VsOut) -> @location(0) vec4<f32> {{
    frame.night.a = 1.0;
    let p = (in.uv * 2.0 - 1.0) * vec2<f32>(1.7777778, 1.0) * 0.70020754;
    let dir = normalize(vec3<f32>(p.x, 1.0, p.y));
    let pixel = length(fwidth(dir));
    return vec4<f32>(star_field(dir, pixel), 1.0);
}}
"#, include_str!("shaders/fullscreen.wgsl"), include_str!("shaders/noise.wgsl"), &sky[start..end]);
        let module = crate::util::shader_standalone(&device, "star boundary regression", &source);
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: None, module: &module, entry_point: Some("check"), compilation_options: Default::default(), cache: None });
        let result = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 80, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 80, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &compute.get_bind_group_layout(0), entries: &[wgpu::BindGroupEntry { binding: 0, resource: result.as_entire_binding() }] });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&compute);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&result, 0, &read, 0, 80);
        queue.submit([encoder.finish()]);
        read.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
        let data = read.get_mapped_range(..).unwrap();
        let values: &[[f32; 4]] = bytemuck::cast_slice(&data);
        for pair in [0, 2] {
            assert!(values[pair][0] > 0.5, "a visible star at the boundary: {:?}", &values[pair..pair + 2]);
            for axis in 0..3 {
                assert!((values[pair][axis] - values[pair + 1][axis]).abs() < 0.02, "star jumps across boundary: {:?}", &values[pair..pair + 2]);
            }
        }
        assert_eq!(&values[4][..3], &[0.0; 3], "unresolved stars fade before escaping bounded support");
        drop(data);
        read.unmap();
        if !timestamp { return; }
        let containing = source.replace("let lo = vec2<i32>(ceil(p - radius - 0.75));", "let lo = vec2<i32>(floor(p));").replace("let hi = vec2<i32>(floor(p + radius - 0.25));", "let hi = lo;");
        let old_module = crate::util::shader_standalone(&device, "star containing-cell cost baseline", &containing);
        let pipeline = |shader: &wgpu::ShaderModule| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None, layout: None,
            vertex: wgpu::VertexState { module: shader, entry_point: Some("vs_fullscreen"), buffers: &[], compilation_options: Default::default() },
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: shader, entry_point: Some("picture"), targets: &[Some(wgpu::ColorTargetState { format: crate::util::HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            multiview_mask: None, cache: None,
        });
        let old = pipeline(&old_module);
        let new = pipeline(&module);
        let query = device.create_query_set(&wgpu::QuerySetDescriptor { label: None, ty: wgpu::QueryType::Timestamp, count: 2 });
        let times = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let timing_read = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        for (width, height) in [(1920, 1080), (7680, 4320)] {
            let target = crate::util::Target::new(&device, "star field cost", width, height, crate::util::HDR_FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT);
            for (name, pipeline) in [("containing-cell baseline", &old), ("neighbor support", &new)] {
                let mut measurements = Vec::new();
                for round in 0..7 {
                    let mut encoder = device.create_command_encoder(&Default::default());
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: None,
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &target.view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store }, depth_slice: None })],
                            depth_stencil_attachment: None, timestamp_writes: Some(wgpu::RenderPassTimestampWrites { query_set: &query, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1) }), occlusion_query_set: None, multiview_mask: None,
                        });
                        pass.set_pipeline(pipeline);
                        pass.draw(0..3, 0..1);
                    }
                    encoder.resolve_query_set(&query, 0..2, &times, 0);
                    encoder.copy_buffer_to_buffer(&times, 0, &timing_read, 0, 16);
                    queue.submit([encoder.finish()]);
                    timing_read.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
                    device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                    let data = timing_read.get_mapped_range(..).unwrap();
                    let t: &[u64] = bytemuck::cast_slice(&data);
                    if round > 1 { measurements.push((t[1] - t[0]) as f64 * queue.get_timestamp_period() as f64 / 1e6); }
                    drop(data);
                    timing_read.unmap();
                }
                measurements.sort_by(f64::total_cmp);
                println!("star field {width}x{height}, {name}: {:.3} ms GPU median", measurements[measurements.len() / 2]);
            }
        }
    }

    /// The sky's colours from the map alone (the world's own sky left out), as `sky_colors`
    /// starts from them.
    fn map_or_plain(s: &SkySettings, fog: Vec3, ambient: Vec3, atm: Option<MapAtmosphere>, _: &WorldSky) -> (Vec3, Vec3) {
        let derived = ambient * Vec3::new(0.62, 0.85, 1.35);
        match atm {
            None => (fog, s.zenith.map_or(derived, Vec3::from)),
            Some(atm) => {
                let (h, z) = map_sky(s, fog, derived, atm);
                (h, s.zenith.map_or(z, Vec3::from))
            }
        }
    }

    #[test]
    fn rings_start_at_the_map_edge_and_end_at_the_reach() {
        let rings = ring_distances(60000.0);
        assert_eq!(rings[0], 0.0);
        assert_eq!(rings[1], 16.0);
        assert_eq!(*rings.last().unwrap(), 60000.0);
        assert!(rings.windows(2).all(|w| w[1] > w[0]));
        assert!(rings.len() < 60, "{} rings", rings.len());
    }

    #[test]
    fn inner_ring_is_the_terrain_boundary() {
        // A 64x48-cell map: the inner ring holds every boundary vertex of the terrain grid
        // exactly once, in loop order.
        let size = Vec2::new(64.0 * 16.0, 48.0 * 16.0);
        let ring = skirt_ring(size, 0.0);
        let on_edge = |p: Vec2| p.x == 0.0 || p.y == 0.0 || p.x == size.x || p.y == size.y;
        assert!(ring.iter().all(|&(_, p)| on_edge(p) && p.x % 16.0 == 0.0 && p.y % 16.0 == 0.0));
        assert_eq!(ring.len(), 2 * (64 + 48));
        for x in 0..=64 {
            for z in [0.0, size.y] {
                let p = Vec2::new(x as f32 * 16.0, z);
                assert_eq!(ring.iter().filter(|&&(_, q)| q == p).count(), 1, "{p}");
            }
        }
        for z in 0..=48 {
            for x in [0.0, size.x] {
                let p = Vec2::new(x, z as f32 * 16.0);
                assert_eq!(ring.iter().filter(|&&(_, q)| q == p).count(), 1, "{p}");
            }
        }
        assert!(ring.windows(2).all(|w| w[1].0 > w[0].0));
    }

    #[test]
    fn coarser_rings_reuse_finer_columns() {
        let size = Vec2::new(5120.0, 3072.0);
        for d in ring_distances(60000.0).windows(2) {
            let (inner, outer) = (skirt_ring(size, d[0]), skirt_ring(size, d[1]));
            assert!(column_spacing(d[1]) >= column_spacing(d[0]));
            // Every side column of the outer ring has a matching column on the inner ring.
            for &(t, _) in outer.iter().filter(|(t, _)| (*t as i32) % 2 == 0) {
                assert!(inner.iter().any(|&(u, _)| (u - t).abs() < 1e-6), "column {t} at d = {} has no match", d[1]);
            }
        }
    }

    #[test]
    fn skirt_is_a_closed_band_without_t_junctions() {
        let size = Vec2::new(40.0 * 16.0, 24.0 * 16.0);
        let (vertices, indices) = skirt_geometry(size, 3000.0);
        assert_eq!(indices.len() % 3, 0);
        // Count how often each undirected edge is used: interior edges twice, the inner and
        // outer boundaries once, and nothing more (a T-junction or overlap would break this).
        let mut edges = std::collections::BTreeMap::new();
        for t in indices.as_chunks::<3>().0 {
            let degenerate = vertices[t[0] as usize] == vertices[t[1] as usize]
                || vertices[t[1] as usize] == vertices[t[2] as usize]
                || vertices[t[0] as usize] == vertices[t[2] as usize];
            if degenerate {
                continue;
            }
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let key = (a.min(b), a.max(b));
                *edges.entry(key).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|&n| n <= 2), "an edge is shared by more than two triangles");
        let boundary: Vec<_> = edges.iter().filter(|&(_, &n)| n == 1).map(|(k, _)| *k).collect();
        // Boundary edges lie on the map rectangle or on the outermost ring.
        let outer = ring_distances(3000.0).last().copied().unwrap();
        let rect_dist = |p: [f32; 2]| {
            let p = Vec2::from(p);
            p.distance(p.clamp(Vec2::ZERO, size))
        };
        for (a, b) in boundary {
            let (da, db) = (rect_dist(vertices[a as usize]), rect_dist(vertices[b as usize]));
            let inner = da < 1e-3 && db < 1e-3;
            let outer_edge = (da - outer).abs() < 0.5 && (db - outer).abs() < 0.5;
            assert!(inner || outer_edge, "open edge {:?} - {:?}", vertices[a as usize], vertices[b as usize]);
        }
        // Triangles all have the same orientation (no fold-overs), ignoring degenerate ones.
        let mut signs = [0usize; 2];
        for t in indices.as_chunks::<3>().0 {
            let [a, b, c] = [0, 1, 2].map(|k| Vec2::from(vertices[t[k] as usize]));
            let cross = (b - a).perp_dot(c - a);
            if cross.abs() > 1e-3 {
                signs[(cross > 0.0) as usize] += 1;
            }
        }
        assert!(signs[0] == 0 || signs[1] == 0, "mixed orientation {signs:?}");
    }

    #[test]
    fn worlds_have_their_own_skies() {
        let s = SkySettings::default();
        let fog = Vec3::new(0.55, 0.62, 0.7);
        let ambient = Vec3::new(0.42, 0.45, 0.5);
        let atm = MapAtmosphere { average: Vec3::new(0.3, 0.3, 0.3), fluid: None };
        let colors = |planet: &str| sky_colors(&s, fog, ambient, Some(atm), &world_sky(planet));
        let luminance = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        let (green_h, green_z) = colors("Green Planet");
        // The moon has no air: a near-black sky, no clouds, little haze, stars by day.
        let moon = world_sky("Lunar");
        let (h, z) = colors("Lunar");
        assert!(luminance(h) < 0.1 * luminance(green_h) && luminance(z) < 0.1 * luminance(green_z));
        assert!(moon.clouds == 0.0 && moon.haze < 0.1 && moon.airless == 1.0 && moon.space == 0.0);
        // A world in space: airless too, and nothing beyond the map, no horizon.
        let rocks = world_sky("Asteroid Belt");
        assert!(rocks.airless == 1.0 && rocks.space == 1.0 && rocks.clouds == 0.0);
        assert!(["Green", "Lava", "Metal", "Ice", "Lunar", ""].iter().all(|p| world_sky(p).space == 0.0));
        // Smoke over lava, smog over metal: darker, browner; a desert's zenith deeper blue.
        let (h, z) = colors("Lava");
        assert!(luminance(h) < 0.8 * luminance(green_h) && luminance(z) < 0.7 * luminance(green_z) && h.x > h.z);
        assert!(world_sky("Metal").clouds > 1.5 && world_sky("Urban") == world_sky("Metal"));
        let (_, z) = colors("Desert");
        assert!(z.z / z.x > green_z.z / green_z.x);
        // Everything finite and positive; unknown worlds have the plain sky.
        for planet in ["Green", "Desert", "Lava", "Metal", "Urban", "Ice", "Lunar", "Acid", "Crystal", "Slate", "Red Planet", "Alien", ""] {
            let (h, z) = colors(planet);
            assert!(h.is_finite() && z.is_finite() && h.min_element() >= 0.0 && z.min_element() >= 0.0, "{planet}");
        }
        assert_eq!(world_sky("Unspecified mod world"), world_sky(""));
    }

    #[test]
    fn tints_follow_the_painted_ground() {
        let s = SkySettings::default();
        let fog = Vec3::new(0.55, 0.62, 0.7);
        let ambient = Vec3::new(0.42, 0.45, 0.5);
        let plain = world_sky("");
        let sky_colors = |s: &SkySettings, fog, ambient, atm| map_or_plain(s, fog, ambient, atm, &plain);
        // No map: the plain fog colour and the ambient-derived zenith.
        let (h, z) = sky_colors(&s, fog, ambient, None);
        assert_eq!(h, fog);
        assert!((z - ambient * Vec3::new(0.62, 0.85, 1.35)).length() < 1e-6);
        // A desert warms the haze; a green world does not turn it green; no tint leaves it
        // alone; an explicit zenith wins.
        let desert = MapAtmosphere { average: Vec3::new(0.45, 0.3, 0.15), fluid: None };
        let (h, _) = sky_colors(&s, fog, ambient, Some(desert));
        assert!(h.x / h.z > fog.x / fog.z);
        let forest = MapAtmosphere { average: Vec3::new(0.05, 0.2, 0.02), fluid: None };
        let (h, _) = sky_colors(&s, fog, ambient, Some(forest));
        assert!(h.y / h.x <= fog.y / fog.x * 1.02 && h.y / h.z <= fog.y / fog.z * 1.08, "{h}");
        let plain = SkySettings { map_tint: 0.0, zenith: Some([0.1, 0.2, 0.3]), ..s.clone() };
        let (h, z) = sky_colors(&plain, fog, ambient, Some(desert));
        assert_eq!(h, fog);
        assert_eq!(z, Vec3::new(0.1, 0.2, 0.3));
        // Lava darkens the sky.
        let lava = MapAtmosphere { average: Vec3::new(0.2, 0.08, 0.04), fluid: Some(Fluid::Lava) };
        let (h, z) = sky_colors(&s, fog, ambient, Some(lava));
        assert!(h.element_sum() < fog.element_sum() && z.element_sum() < (ambient * Vec3::new(0.62, 0.85, 1.35)).element_sum());
    }
}
