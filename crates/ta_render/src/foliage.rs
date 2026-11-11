//! TA Reborn's own trees, bushes and rocks (original models and textures, `features/`, made by
//! `tools/features/build.py` and `textures.py`): the game draws them in place of TA's painted
//! feature sprites when the camera comes close (the game's `features.rs` picks a model for each
//! feature, fits it to the painting's size and takes its colours), crossfading from the sprite.
//!
//! Every model comes in three levels of detail, its leaves cards cut out of a texture array by
//! alpha (anti-aliased by alpha to coverage, as the sprites are) and its bark and rock opaque. The
//! wind bends the trees as it sways the painted ones and their leaves flutter; they cast shadows
//! (the shadow map, leaves cut out), are mirrored in the water (reflection.rs), stand in the fog
//! of war, take the sky's and the ground's light, firelight, and darken at their foot. A burning
//! tree chars and glows.
//!
//! TA Reborn's own festive features (`tools/features/festive.py`: snowy and decorated firs,
//! cottages, lanterns...) are among the models, in colours of their own (`FoliageModel::own`):
//! their snow and the festive atlas never tinted, their lights glowing and twinkling, snow lying on
//! a snowy fir's needles where they turn up to the sky (`FoliageInstance::snow`).

use std::collections::HashMap;
use std::ops::Range;

use anyhow::{Context, Result, anyhow};
use bytemuck::{Pod, Zeroable};
use glam::{Vec3, Vec4Swizzles};

use crate::glb::Glb;
use crate::terrain::byte_mips;

/// The texture layers, in the order of the array (and of `tools/features/textures.py`); the
/// models' materials are named after them.
pub const LAYERS: [&str; 19] = ["broadleaf", "needles", "jungle", "tropical", "smallleaf", "twigs", "clump", "bark", "bark_smooth", "rock", "gasbag", "crystal", "tendril", "hull", "metal", "barrier", "trim", "snow", "festive"];
/// Layers before this one are cut out by their alpha (leaves, needles, twigs); the rest are
/// opaque (bark, rock).
pub const FIRST_OPAQUE: usize = 7;
/// Levels of detail per model (0 the finest).
pub const LODS: usize = 3;
/// How much brighter than its instance's colours the shader draws leaves and bark (foliage.wgsl's
/// `LEAF_LIGHT` and `BARK_LIGHT`): a model in its own colours is given them divided by these.
pub const LEAF_LIGHT: f32 = 0.95;
pub const BARK_LIGHT: f32 = 1.2;
const SIZE: u32 = 512;

fn pngs() -> [&'static [u8]; LAYERS.len()] {
    [
        include_bytes!("../features/broadleaf.png"),
        include_bytes!("../features/needles.png"),
        include_bytes!("../features/jungle.png"),
        include_bytes!("../features/tropical.png"),
        include_bytes!("../features/smallleaf.png"),
        include_bytes!("../features/twigs.png"),
        include_bytes!("../features/clump.png"),
        include_bytes!("../features/bark.png"),
        include_bytes!("../features/bark_smooth.png"),
        include_bytes!("../features/rock.png"),
        include_bytes!("../features/gasbag.png"),
        include_bytes!("../features/crystal.png"),
        include_bytes!("../features/tendril.png"),
        include_bytes!("../features/hull.png"),
        include_bytes!("../features/metal.png"),
        include_bytes!("../features/barrier.png"),
        include_bytes!("../features/trim.png"),
        include_bytes!("../features/snow.png"),
        include_bytes!("../features/festive.png"),
    ]
}

fn glb() -> &'static [u8] {
    include_bytes!("../features/features.glb")
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub(crate) struct Vertex {
    pos: [f32; 3],
    normal: [i8; 4],
    uv: [f32; 2],
    /// How open to the sky, how far the wind bends it, how much it flutters, a phase of its own.
    color: [u8; 4],
    layer: u32,
}

/// One tree, bush or rock to draw.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default)]
pub struct FoliageInstance {
    /// Its foot on the ground (world units).
    pub pos: [f32; 3],
    /// Turned about the vertical (radians).
    pub yaw: f32,
    /// The model stretched across (x, z) and up (y).
    pub scale: [f32; 3],
    /// How much of it is drawn (0..1): the rest is left to the painted sprite it stands in for.
    pub fade: f32,
    /// The colour of its leaves (linear): that of their painting's lit side; a: how charred it is
    /// (0..1).
    pub leaf: [f32; 4],
    /// The colour of its bark or rock (linear): its painting's mean, shade and all; a: how much
    /// embers glow in it (a fire, 0..1).
    pub bark: [f32; 4],
    /// x: its height in the world (how far the wind bends its top); y: 1 when it lies on the
    /// ground following its slope (a Mars plant's tendrils); z, w: the slope of the ground a rock
    /// stands on (its rise for a unit east and south), which its foot follows, upright still.
    pub extra: [f32; 4],
    /// A crystal's light within (linear): what shines through it, and from an energy node's tips.
    /// (Its own colour is `leaf`'s, the rock under it `bark`'s.) a: 1 for rock as dark as painted
    /// (an energy node's mound, darker than the ground round it), not lifted towards the ground's
    /// light as other dark rock is (see the shader's `vs_main`).
    pub light: [f32; 4],
    /// How much snow lies on its leaves (0..1, a snowy fir's): where they turn up to the sky
    /// they are white with it, in clumps.
    pub snow: f32,
}

pub type FoliageModelId = u16;

/// GPU copies of one feature share its coverage between adjacent levels of detail. Keeping
/// the copies here leaves contact occlusion and other feature bookkeeping counted just once.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
struct DrawInstance {
    feature: FoliageInstance,
    /// Interval within the feature's sample coverage, complementary to the other level's.
    lod_range: [f32; 2],
}

fn detail_levels(detail: f32) -> [(u8, [f32; 2]); 2] {
    let level = detail.clamp(0.0, (LODS - 1) as f32);
    let fine = level.floor() as u8;
    let share = 1.0 - level.fract();
    [(fine, [0.0, share]), (fine + 1, [share, 1.0])]
}

#[derive(Clone, Copy)]
enum DetailPass { Scene, Reflection, Shadow, NearShadow }

impl DetailPass {
    fn detail(self, detail: f32, fade: f32) -> (f32, f32) {
        let detail = detail.clamp(0.0, (LODS - 1) as f32);
        match self {
            Self::Scene => (detail, fade),
            // Both near meshes reflect as the middle one; every shadow uses the same far
            // mesh. Merge their coverage before batching rather than drawing it twice.
            Self::Reflection => (detail.max(1.0), fade),
            Self::Shadow => ((LODS - 1) as f32, fade),
            Self::NearShadow => ((LODS - 1) as f32, fade * (2.0 - detail).clamp(0.0, 1.0)),
        }
    }
}

type Batch = (FoliageModelId, u8, u32, u32);

#[derive(Clone, Debug, Default)]
struct Lod {
    base_vertex: i32,
    opaque: Range<u32>,
    cutout: Range<u32>,
}

/// A model of the library, in its own units (y up, its foot at the origin).
#[derive(Clone, Debug)]
pub struct FoliageModel {
    pub name: String,
    pub height: f32,
    /// Across, as it covers the ground seen from above (leaves and all).
    pub width: f32,
    /// The crown's centre and radius (where a burning tree's fire is).
    pub crown: [f32; 4],
    /// A rock's: how far from its foot it meets the ground (the ground darkens in a ring round
    /// that, see `contact_occluder`); 0 for the rest.
    pub contact: f32,
    /// A vehicle's length (along x, its front at +x) and breadth (along z); 0 for the rest.
    pub length: f32,
    pub breadth: f32,
    /// Some of its points (the middle level of detail's), to fit it to a painting's outline.
    pub points: Vec<Vec3>,
    /// TA Reborn's own feature models' colours (their extras' `own`): drawn as made, not stretched
    /// or tinted to a painting (theirs are drawn from them).
    pub own: Option<Own>,
    lods: [Lod; LODS],
}

/// The colours of one of TA Reborn's own models, as it was made: its needles and bark their
/// textures' own (see `FoliageLibrary::own_colours`; its bark that of another layer, `bark`, a
/// boulder's rock), its ice `leaf` (linear) with `light` within it, and `snow` lying on its
/// needles.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Own {
    pub leaf: Option<[f32; 3]>,
    pub bark: Option<usize>,
    pub light: [f32; 3],
    pub snow: f32,
}

impl FoliageModel {
    /// How far above its foot TA's oblique view would draw its top (world units, at scale 1 across
    /// and `up` up): TA lifts a point by half its height, and shows ground nearer the viewer
    /// (south, +z) lower on the screen. The painting of a feature this stands in for is that
    /// tall above its anchor.
    pub fn ta_height(&self, across: f32, up: f32) -> f32 {
        self.points.iter().map(|p| p.y * up * 0.5 - p.z * across).fold(0.0, f32::max)
    }
}

pub struct FoliageLibrary {
    pub models: Vec<FoliageModel>,
    by_name: HashMap<String, FoliageModelId>,
    /// Each layer's mean colour (linear, its covered texels): the shader takes each texture to
    /// its instance's colours by these.
    pub layer_mean: [[f32; 3]; LAYERS.len()],
}

impl FoliageLibrary {
    pub fn id(&self, name: &str) -> Option<FoliageModelId> {
        self.by_name.get(name).copied()
    }

    pub fn get(&self, id: FoliageModelId) -> &FoliageModel {
        &self.models[id as usize]
    }

    /// The library shipped, without a GPU (the game picks and fits models with it; tests).
    pub fn load() -> Result<FoliageLibrary> {
        let (models, _, _) = parse(glb()).context("features.glb")?;
        let mut layer_mean = [[0.5f32; 3]; LAYERS.len()];
        for (layer, png) in pngs().iter().enumerate() {
            layer_mean[layer] = mean_colour(&texels(png).with_context(|| LAYERS[layer])?);
        }
        Ok(FoliageLibrary::new(models, layer_mean))
    }

    fn new(models: Vec<FoliageModel>, layer_mean: [[f32; 3]; LAYERS.len()]) -> FoliageLibrary {
        let by_name = models.iter().enumerate().map(|(i, m)| (m.name.clone(), i as FoliageModelId)).collect();
        FoliageLibrary { models, by_name, layer_mean }
    }

    /// The colours to give an own model's instance (`leaf`, `bark`) so that the shader draws its
    /// needles and bark in their textures' own colours, or its crystal in its own (`Own::leaf`).
    pub fn own_colours(&self, own: &Own) -> ([f32; 3], [f32; 3]) {
        let mean = |name: &str| LAYERS.iter().position(|l| *l == name).map_or([0.5; 3], |i| self.layer_mean[i]);
        let leaf = own.leaf.unwrap_or_else(|| mean("needles").map(|c| c / LEAF_LIGHT));
        (leaf, own.bark.map_or_else(|| mean("bark"), |i| self.layer_mean[i]).map(|c| c / BARK_LIGHT))
    }

    /// The models whose names start with `prefix` (a kind's variants).
    pub fn variants(&self, prefix: &str) -> Vec<FoliageModelId> {
        let mut v: Vec<FoliageModelId> = self.models.iter().enumerate().filter(|(_, m)| m.name.starts_with(prefix)).map(|(i, _)| i as FoliageModelId).collect();
        v.sort_by(|&a, &b| self.models[a as usize].name.cmp(&self.models[b as usize].name));
        v
    }
}

/// Reads the library's models from its GLB: their vertices and indices for the GPU, and each
/// level's ranges (opaque triangles first, then the cut-out ones).
pub(crate) fn parse(bytes: &[u8]) -> Result<(Vec<FoliageModel>, Vec<Vertex>, Vec<u32>)> {
    let glb = Glb::parse(bytes)?;
    let materials: Vec<String> = glb.doc["materials"].as_array().map(|m| m.iter().map(|x| x["name"].as_str().unwrap_or("").to_string()).collect()).unwrap_or_default();
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut models: Vec<FoliageModel> = Vec::new();
    let mut index_of: HashMap<String, usize> = HashMap::new();
    for (node, matrix) in glb.walk() {
        let n = glb.node(node);
        let Some((name, lod)) = n["name"].as_str().and_then(|s| s.rsplit_once(".lod")) else { continue };
        let lod: usize = lod.parse().context("level of detail")?;
        anyhow::ensure!(lod < LODS, "{name}: level {lod}");
        let Some(mesh) = n["mesh"].as_u64() else { continue };
        let k = *index_of.entry(name.to_string()).or_insert_with(|| {
            models.push(FoliageModel { name: name.to_string(), height: 1.0, width: 1.0, crown: [0.0, 0.5, 0.0, 0.5], contact: 0.0, length: 0.0, breadth: 0.0, points: Vec::new(), own: None, lods: Default::default() });
            models.len() - 1
        });
        if lod == 0 {
            let e = &n["extras"];
            let num = |v: &serde_json::Value| v.as_f64().map(|x| x as f32);
            let m = &mut models[k];
            m.height = num(&e["height"]).ok_or_else(|| anyhow!("{name}: no height"))?;
            m.width = num(&e["width"]).ok_or_else(|| anyhow!("{name}: no width"))?;
            if let Some(c) = e["crown"].as_array().filter(|c| c.len() == 4) {
                m.crown = [0, 1, 2, 3].map(|i| num(&c[i]).unwrap_or(0.0));
            }
            m.contact = num(&e["contact"]).unwrap_or(0.0);
            m.length = num(&e["length"]).unwrap_or(0.0);
            m.breadth = num(&e["breadth"]).unwrap_or(0.0);
            if let Some(own) = e["own"].as_object() {
                let rgb = |v: &serde_json::Value| v.as_array().filter(|c| c.len() == 3).map(|c| [0, 1, 2].map(|i| num(&c[i]).unwrap_or(0.0)));
                let bark = own.get("bark").and_then(|b| b.as_str()).and_then(|b| LAYERS.iter().position(|l| *l == b));
                m.own = Some(Own { leaf: own.get("leaf").and_then(rgb), bark, light: own.get("light").and_then(rgb).unwrap_or([0.0; 3]), snow: own.get("snow").and_then(num).unwrap_or(0.0) });
            }
        }
        // The level's triangles: opaque layers first, then the cut-out ones.
        let base = vertices.len();
        let mut opaque = Vec::new();
        let mut cutout = Vec::new();
        let normal_matrix = matrix.inverse().transpose();
        for prim in glb.doc["meshes"][mesh as usize]["primitives"].as_array().into_iter().flatten() {
            let a = &prim["attributes"];
            let material = prim["material"].as_u64().and_then(|m| materials.get(m as usize)).ok_or_else(|| anyhow!("{name}: a primitive without a material"))?;
            let layer = LAYERS.iter().position(|l| l == material).ok_or_else(|| anyhow!("{name}: unknown layer {material}"))?;
            let read = |key: &str, width: usize| -> Result<Vec<f32>> { glb.read_vec(a[key].as_u64().ok_or_else(|| anyhow!("{name}: no {key}"))? as usize, width) };
            let pos = read("POSITION", 3)?;
            let nrm = read("NORMAL", 3)?;
            let uv = read("TEXCOORD_0", 2)?;
            let col = read("COLOR_0", 4)?;
            let count = pos.len() / 3;
            anyhow::ensure!(nrm.len() == count * 3 && uv.len() == count * 2 && col.len() == count * 4, "{name}: attribute counts differ");
            let first = (vertices.len() - base) as u32;
            for i in 0..count {
                let p = matrix.transform_point3(Vec3::new(pos[i * 3], pos[i * 3 + 1], pos[i * 3 + 2]));
                let q = normal_matrix.transform_vector3(Vec3::new(nrm[i * 3], nrm[i * 3 + 1], nrm[i * 3 + 2])).normalize_or(Vec3::Y);
                let snorm = |v: f32| (v.clamp(-1.0, 1.0) * 127.0).round() as i8;
                let unorm = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                vertices.push(Vertex {
                    pos: p.to_array(),
                    normal: [snorm(q.x), snorm(q.y), snorm(q.z), 0],
                    uv: [uv[i * 2], uv[i * 2 + 1]],
                    color: [unorm(col[i * 4]), unorm(col[i * 4 + 1]), unorm(col[i * 4 + 2]), unorm(col[i * 4 + 3])],
                    layer: layer as u32,
                });
                if lod == 1 && i % 3 == 0 {
                    models[k].points.push(p);
                }
            }
            let tris = glb.read_indices(prim["indices"].as_u64().ok_or_else(|| anyhow!("{name}: no indices"))? as usize)?;
            let into = if layer < FIRST_OPAQUE { &mut cutout } else { &mut opaque };
            into.extend(tris.into_iter().map(|t| t + first));
        }
        let start = indices.len() as u32;
        indices.extend_from_slice(&opaque);
        let mid = indices.len() as u32;
        indices.extend_from_slice(&cutout);
        models[k].lods[lod] = Lod { base_vertex: base as i32, opaque: start..mid, cutout: mid..indices.len() as u32 };
    }
    for m in &mut models {
        anyhow::ensure!(m.lods.iter().all(|l| !l.opaque.is_empty() || !l.cutout.is_empty()), "{}: a level of detail is missing", m.name);
        // Its width as it covers the ground seen from above (leaf sprays reach past the crown's
        // shape): the distance from its axis that all but the farthest twentieth of its points
        // keep within, twice.
        let mut r: Vec<f32> = m.points.iter().map(|p| (p.x * p.x + p.z * p.z).sqrt()).collect();
        if r.len() >= 20 {
            r.sort_by(f32::total_cmp);
            m.width = 2.0 * r[r.len() * 95 / 100];
        }
    }
    Ok((models, vertices, indices))
}

/// A model's finest level as a mesh of its own (in the model's units, y up, its foot at the
/// origin): its corners welded where they meet, and its triangles. Space's asteroids are the
/// rocks scaled up (see [`crate::space`]).
pub(crate) fn model_mesh(name: &str) -> Result<(Vec<Vec3>, Vec<[u32; 3]>)> {
    let (models, vertices, indices) = parse(glb()).context("features.glb")?;
    let model = models.iter().find(|m| m.name == name).ok_or_else(|| anyhow!("features.glb has no {name}"))?;
    let lod = &model.lods[0];
    let mut points: Vec<Vec3> = Vec::new();
    let mut welded: HashMap<[i32; 3], u32> = HashMap::new();
    let mut corner = |i: u32| {
        let v = &vertices[(lod.base_vertex + i as i32) as usize];
        let p = Vec3::from_array(v.pos);
        let key = [(p.x * 1e3).round() as i32, (p.y * 1e3).round() as i32, (p.z * 1e3).round() as i32];
        *welded.entry(key).or_insert_with(|| {
            points.push(p);
            points.len() as u32 - 1
        })
    };
    let mut triangles = Vec::new();
    for range in [lod.opaque.clone(), lod.cutout.clone()] {
        for t in indices[range.start as usize..range.end as usize].as_chunks::<3>().0 {
            let tri = [corner(t[0]), corner(t[1]), corner(t[2])];
            if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
                triangles.push(tri);
            }
        }
    }
    Ok((points, triangles))
}

/// The rock's texture (RGBA, sRGB, [`SIZE`] square) and its mean colour (linear).
pub(crate) fn rock_texture() -> Result<(Vec<u8>, [f32; 3])> {
    let rock = LAYERS.iter().position(|l| *l == "rock").expect("a rock layer");
    let level0 = texels(pngs()[rock]).context("rock")?;
    let mean = mean_colour(&level0);
    Ok((level0, mean))
}

/// The layers' textures' size (texels square).
pub(crate) const TEXTURE_SIZE: u32 = SIZE;

/// A layer's texels (RGBA, sRGB) from its PNG.
fn texels(png: &[u8]) -> Result<Vec<u8>> {
    let img = image::load_from_memory(png)?.to_rgba8();
    anyhow::ensure!(img.width() == SIZE && img.height() == SIZE, "{}x{}, not {SIZE}x{SIZE}", img.width(), img.height());
    Ok(img.into_raw())
}

/// A cut-out layer's mips with as much of each covered (alpha at least half) as of the full
/// texture: averaged alpha would thin the foliage out as it recedes, so each level's alpha is
/// scaled until its coverage matches.
fn cutout_mips(level0: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let covered = |data: &[u8], scale: f32| data.as_chunks::<4>().0.iter().filter(|p| p[3] as f32 * scale >= 127.5).count() as f32 / (data.len() / 4) as f32;
    let target = covered(level0, 1.0);
    let mut levels = byte_mips(SIZE, SIZE, 4, level0);
    for (_, _, data) in levels.iter_mut().skip(1) {
        let (mut lo, mut hi) = (0.25f32, 16.0f32);
        for _ in 0..16 {
            let mid = (lo * hi).sqrt();
            if covered(data, mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let scale = (lo * hi).sqrt();
        for p in data.as_chunks_mut::<4>().0 {
            p[3] = (p[3] as f32 * scale).round().min(255.0) as u8;
        }
    }
    levels
}

/// The mean linear colour of a layer's covered texels.
fn mean_colour(texels: &[u8]) -> [f32; 3] {
    let mut sum = [0.0f64; 3];
    let mut weight = 0.0f64;
    for p in texels.as_chunks::<4>().0 {
        let a = p[3] as f64 / 255.0;
        for c in 0..3 {
            sum[c] += crate::util::srgb_to_linear(p[c] as f32 / 255.0) as f64 * a;
        }
        weight += a;
    }
    sum.map(|s| (s / weight.max(1e-9)) as f32)
}

pub struct FoliageRenderer {
    pub library: FoliageLibrary,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    opaque: wgpu::RenderPipeline,
    cutout: wgpu::RenderPipeline,
    shadow_opaque: wgpu::RenderPipeline,
    shadow_cutout: wgpu::RenderPipeline,
    /// Mirrored in the water (reflection.rs): one sample a pixel, triangles turned over.
    opaque_reflection: wgpu::RenderPipeline,
    cutout_reflection: wgpu::RenderPipeline,
    instances: Vec<FoliageInstance>,
    draw_instances: Vec<DrawInstance>,
    instance_buffer: wgpu::Buffer,
    /// (model, level, first instance, count), this frame.
    batches: Vec<Batch>,
    reflection_batches: Vec<Batch>,
    shadow_batches: Vec<Batch>,
    /// The point the leaves give way to (xyz) and how far round the line to it (w; 0 none).
    sight: wgpu::Buffer,
}

impl FoliageRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, frame_layout: &wgpu::BindGroupLayout, hdr: wgpu::TextureFormat, samples: u32) -> Result<FoliageRenderer> {
        let (models, vertices, indices) = parse(glb()).context("features.glb")?;
        use crate::gpu::CheckedBufferInit;
        let vertex_buffer = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("foliage vertices"), contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::VERTEX });
        let index_buffer = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("foliage indices"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX });
        // The texture layers, with their mips (the cut-out ones keeping their coverage).
        let levels = SIZE.ilog2() + 1;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("foliage layers"),
            size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: LAYERS.len() as u32 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut layer_mean = [[0.5f32; 3]; LAYERS.len()];
        for (layer, png) in pngs().iter().enumerate() {
            let level0 = texels(png).with_context(|| LAYERS[layer])?;
            layer_mean[layer] = mean_colour(&level0);
            let mips = if layer < FIRST_OPAQUE { cutout_mips(&level0) } else { byte_mips(SIZE, SIZE, 4, &level0) };
            for (level, (w, h, data)) in mips.into_iter().enumerate().take(levels as usize) {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level as u32, origin: wgpu::Origin3d { x: 0, y: 0, z: layer as u32 }, aspect: wgpu::TextureAspect::All },
                    &data,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
            }
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("foliage"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("foliage"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let sight = device.create_buffer(&wgpu::BufferDescriptor { label: Some("foliage sight"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let means: Vec<[f32; 4]> = layer_mean.iter().map(|m| [m[0], m[1], m[2], 1.0]).collect();
        let means = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("foliage layer means"), contents: bytemuck::cast_slice(&means), usage: wgpu::BufferUsages::UNIFORM });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("foliage"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: means.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: sight.as_entire_binding() },
            ],
        });
        let shader = crate::util::shader(device, "foliage", include_str!("shaders/foliage.wgsl"));
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("foliage"), bind_group_layouts: &[Some(frame_layout), Some(&layout)], immediate_size: 0 });
        let buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Snorm8x4, 2 => Float32x2, 3 => Unorm8x4, 4 => Uint32],
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<DrawInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                // (In the order they lie: the snow after the light, then the level's range.)
                attributes: &wgpu::vertex_attr_array![5 => Float32x3, 6 => Float32, 7 => Float32x3, 8 => Float32, 9 => Float32x4, 10 => Float32x4, 11 => Float32x4, 12 => Float32x4, 14 => Float32, 13 => Float32x2],
            }),
        ];
        // Both scene pipelines turn alpha into coverage (multisampled): the leaves' edges; the
        // crossfade with the sprite (`fade`) shares each pixel's samples out (`handover_mask`).
        let scene = |label: &str, fs: &str, cull: Option<wgpu::Face>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &buffers, compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
                depth_stencil: Some(crate::util::depth_state(true)),
                multisample: wgpu::MultisampleState { count: samples, alpha_to_coverage_enabled: samples > 1, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    // (How many samples a model taking over from its painting shares them out of.)
                    targets: &[Some(wgpu::ColorTargetState { format: hdr, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &[("SCENE_SAMPLES", samples as f64)], ..Default::default() },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = scene("foliage opaque", "fs_opaque", Some(wgpu::Face::Back));
        let cutout = scene("foliage cutout", "fs_cutout", None);
        // The reflection pass's: clockwise triangles face the camera after the mirror.
        let mirrored = |label: &str, fs: &str, cull: Option<wgpu::Face>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &buffers, compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState { cull_mode: cull, ..crate::reflection::primitive() },
                depth_stencil: Some(crate::util::depth_state(true)),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format: hdr, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque_reflection = mirrored("foliage opaque reflection", "fs_opaque_reflection", Some(wgpu::Face::Back));
        let cutout_reflection = mirrored("foliage cutout reflection", "fs_cutout_reflection", None);
        let shadow = |label: &str, fs: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_shadow"), buffers: &buffers, compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState { bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 }, ..crate::util::depth_state(true) }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), targets: &[], compilation_options: Default::default() }),
                multiview_mask: None,
                cache: None,
            })
        };
        let shadow_opaque = shadow("foliage shadow", "fs_shadow_opaque");
        let shadow_cutout = shadow("foliage shadow cutout", "fs_shadow_cutout");
        let instance_buffer = Self::make_instances(device, 1024);
        log::info!("foliage: {} models, {} vertices, {} triangles", models.len(), vertices.len(), indices.len() / 3);
        Ok(FoliageRenderer {
            library: FoliageLibrary::new(models, layer_mean),
            vertex_buffer,
            index_buffer,
            bind_group,
            opaque,
            cutout,
            shadow_opaque,
            shadow_cutout,
            opaque_reflection,
            cutout_reflection,
            instances: Vec::new(),
            draw_instances: Vec::new(),
            instance_buffer,
            batches: Vec::new(),
            reflection_batches: Vec::new(),
            shadow_batches: Vec::new(),
            sight,
        })
    }

    fn make_instances(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("foliage instances"),
            size: (count * std::mem::size_of::<DrawInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// This frame's instances: each with its model and continuous level of detail; and the
    /// point they give way to (see `Renderer::prepare_foliage`). `far_shadows` includes distant
    /// trees in the shadow pass (soft shadows, High and Ultra).
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, list: &[(FoliageModelId, f32, FoliageInstance)], sight: Option<(Vec3, f32)>, far_shadows: bool) {
        let s = sight.map_or([0.0; 4], |(p, r)| [p.x, p.y, p.z, r]);
        queue.write_buffer(&self.sight, 0, bytemuck::cast_slice(&s));
        self.instances.clear();
        self.draw_instances.clear();
        let models = self.library.models.len();
        let valid: Vec<_> = (0..list.len()).filter(|&i| {
            let (model, detail, feature) = list[i];
            (model as usize) < models && detail.is_finite() && feature.fade > 0.0
        }).collect();
        self.instances.extend(valid.iter().map(|&i| list[i].2));
        let mut order = Vec::with_capacity(valid.len());
        // Model and detail IDs form a small, dense key space. Count into those buckets
        // instead of sorting the entire forest again for each pass.
        let mut counts = vec![0usize; models * LODS];
        let mut offsets = vec![0usize; models * LODS];
        let mut prepare = |batches: &mut Vec<Batch>, pass: DetailPass| {
            order.clear();
            counts.fill(0);
            batches.clear();
            for &i in &valid {
                let (model, detail, feature) = list[i];
                let (detail, fade) = pass.detail(detail, feature.fade);
                if fade <= 0.0 { continue; }
                for (lod, lod_range) in detail_levels(detail) {
                    if lod_range[1] > lod_range[0] {
                        counts[model as usize * LODS + lod as usize] += 1;
                        order.push((i, lod, lod_range, fade));
                    }
                }
            }
            let mut end = self.draw_instances.len();
            for (key, &count) in counts.iter().enumerate() {
                offsets[key] = end;
                if count > 0 {
                    batches.push(((key / LODS) as FoliageModelId, (key % LODS) as u8, end as u32, count as u32));
                    end += count;
                }
            }
            debug_assert_eq!(end, self.draw_instances.len() + order.len());
            self.draw_instances.resize(end, DrawInstance::zeroed());
            for &(i, lod, lod_range, fade) in &order {
                let (model, _, mut feature) = list[i];
                feature.fade = fade;
                let next = &mut offsets[model as usize * LODS + lod as usize];
                self.draw_instances[*next] = DrawInstance { feature, lod_range };
                *next += 1;
            }
        };
        prepare(&mut self.batches, DetailPass::Scene);
        prepare(&mut self.reflection_batches, DetailPass::Reflection);
        prepare(&mut self.shadow_batches, if far_shadows { DetailPass::Shadow } else { DetailPass::NearShadow });
        if self.instances.is_empty() {
            return;
        }
        let need = self.draw_instances.len() * std::mem::size_of::<DrawInstance>();
        if (self.instance_buffer.size() as usize) < need {
            self.instance_buffer = Self::make_instances(device, self.draw_instances.len().next_power_of_two());
        }
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&self.draw_instances));
    }

    /// This frame's instances (as prepared).
    pub fn instances(&self) -> &[FoliageInstance] {
        &self.instances
    }

    /// Draws a pass's batches, already merged where adjacent levels use the same mesh.
    fn draw_with<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, opaque: &'a wgpu::RenderPipeline, cutout: &'a wgpu::RenderPipeline, batches: &[Batch]) {
        if batches.is_empty() {
            return;
        }
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        for (pipeline, cut) in [(opaque, false), (cutout, true)] {
            pass.set_pipeline(pipeline);
            for &(model, lod, first, count) in batches {
                let l = &self.library.models[model as usize].lods[lod as usize];
                let range = if cut { l.cutout.clone() } else { l.opaque.clone() };
                if !range.is_empty() {
                    pass.draw_indexed(range, l.base_vertex, first..first + count);
                }
            }
        }
    }

    /// Draws them into the scene (after the models, which are in the depth buffer so that hidden
    /// leaves are skipped, and before the ground, which the leaves then hide from its shading).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.draw_with(pass, &self.opaque, &self.cutout, &self.batches);
    }

    /// Draws them mirrored in the water (inside the reflection pass): each no finer than the
    /// middle level (the reflection is drawn at half resolution or so, and rippled).
    pub(crate) fn draw_reflection<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.draw_with(pass, &self.opaque_reflection, &self.cutout_reflection, &self.reflection_batches);
    }

    /// Draws their shadows into the shadow map: those near enough for their finer levels of
    /// detail (far off a tree's shadow is a few pixels, and there are many trees), each cast by
    /// its coarsest level (seen from the sun nothing hides the leaves, so every card of every
    /// tree is drawn: its few big cards cast much the same shadow).
    /// Draws their prepared shadows, once each with the coarsest shape. Soft shadows include
    /// distant trees too: their paintings' shadows are gone once they have taken over.
    pub fn draw_shadow<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.draw_with(pass, &self.shadow_opaque, &self.shadow_cutout, &self.shadow_batches);
    }

    /// Where a model's crown is for an instance (world): its centre and radius.
    pub fn crown(&self, model: FoliageModelId, inst: &FoliageInstance) -> (Vec3, f32) {
        let m = self.library.get(model);
        let c = glam::Vec4::from_array(m.crown);
        let (s, cos) = inst.yaw.sin_cos();
        let local = c.xyz() * Vec3::from_array(inst.scale);
        let turned = Vec3::new(local.x * cos + local.z * s, local.y, -local.x * s + local.z * cos);
        (Vec3::from_array(inst.pos) + turned, c.w * inst.scale[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foliage_detail_levels_partition_coverage_and_keep_adjacent_meshes() {
        for step in 0..=200 {
            let detail = step as f32 * 0.01;
            let levels = detail_levels(detail);
            let active: Vec<_> = levels.iter().filter(|(_, r)| r[1] > r[0]).collect();
            assert!(!active.is_empty() && active.len() <= 2);
            assert_eq!(active.first().unwrap().1[0], 0.0);
            assert_eq!(active.last().unwrap().1[1], 1.0);
            assert!(active.iter().all(|(lod, _)| (*lod as usize) < LODS));
            if active.len() == 2 {
                assert_eq!(active[0].0 + 1, active[1].0);
                assert_eq!(active[0].1[1], active[1].1[0]);
            }
            if detail.fract() == 0.0 { assert_eq!(active.len(), 1); }
        }
    }

    #[test]
    fn foliage_passes_do_not_draw_identical_meshes_twice() {
        let active = |detail| detail_levels(detail).into_iter().filter(|(_, r)| r[1] > r[0]).count();
        for step in 0..=200 {
            let detail = step as f32 * 0.01;
            let (shadow, fade) = DetailPass::Shadow.detail(detail, 0.75);
            assert_eq!(active(shadow), 1);
            assert_eq!(shadow, 2.0);
            assert_eq!(fade, 0.75);
            let (reflection, fade) = DetailPass::Reflection.detail(detail, 0.75);
            assert_eq!(fade, 0.75);
            if detail <= 1.0 { assert_eq!(reflection, 1.0); assert_eq!(active(reflection), 1); }
            let (near_shadow, fade) = DetailPass::NearShadow.detail(detail, 0.75);
            assert_eq!(active(near_shadow), 1);
            assert!((fade - 0.75 * (2.0 - detail).clamp(0.0, 1.0)).abs() < 1e-6);
        }
    }

    #[test]
    #[ignore = "requires a graphics adapter"]
    fn gpu_foliage_fades_and_detail_levels_keep_exact_coverage() {
        let source = include_str!("shaders/foliage.wgsl");
        let common = include_str!("shaders/common.wgsl");
        let section = |text: &str, start: &str, end: &str| {
            let at = text.find(start).expect(start);
            text[at..at + text[at..].find(end).expect(end)].to_owned()
        };
        // Render the production fragment shader. Additive grey exposes both gaps and double
        // coverage when adjacent meshes divide a feature's samples, with and without MSAA.
        let shader = format!(r#"
{}
{}
{}
{}
{}
struct Frame {{ camera_pos: vec4<f32> }};
var<private> frame: Frame = Frame(vec4<f32>(0.0, 0.0, 0.0, 1.0));
var<private> sight: vec4<f32>;
override TEST_DISTANCE: f32 = 18.0;
override TEST_COARSE_DISTANCE: f32 = 18.0;
override TEST_FADE: f32 = 1.0;
override TEST_SPLIT: f32 = -1.0;
fn saturate(x: f32) -> f32 {{ return clamp(x, 0.0, 1.0); }}
fn opaque_colour(in: VOut) -> vec3<f32> {{ return vec3<f32>(0.25); }}
@vertex
fn test_vertex(@builtin(vertex_index) i: u32, @builtin(instance_index) level: u32) -> VOut {{
    let points = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: VOut;
    out.clip = vec4<f32>(points[i], 0.5, 1.0);
    out.world = vec3<f32>(select(TEST_DISTANCE, TEST_COARSE_DISTANCE, level == 1u), 0.0, 0.0);
    out.fade = TEST_FADE;
    out.lod_range = vec2<f32>(0.0, 1.0);
    if (TEST_SPLIT >= 0.0) {{
        out.lod_range = select(vec2<f32>(0.0, TEST_SPLIT), vec2<f32>(TEST_SPLIT, 1.0), level == 1u);
    }}
    return out;
}}
"#,
            section(common, "fn handover_dither", "// Points of a disc"),
            section(source, "struct VOut", "struct Placed"),
            section(source, "fn near_fade", "// The lit colour"),
            section(source, "fn detail_mask", "@fragment"),
            section(source, "@fragment\nfn fs_opaque(in", "// Bark and rock lit"),
        );
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let module = crate::util::shader_standalone(&device, "foliage coverage regression", &shader);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
        });
        for samples in [1, 4] {
            let multisampled = (samples > 1).then(|| device.create_texture(&wgpu::TextureDescriptor {
                label: None, size: target.size(), mip_level_count: 1, sample_count: samples,
                dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[],
            }));
            for (distance, coarse_distance, fade, expected) in [(10.0, 10.0, 1.0, 0), (18.0, 18.0, 1.0, 8), (30.0, 30.0, 1.0, 16), (18.0, 18.0, 0.5, 4), (30.0, 30.0, 0.5, 8), (30.0, 18.0, 1.0, 16), (18.0, 30.0, 0.5, 4)] {
                let mut baseline = Vec::new();
                for split in [-1.0, 0.0, 0.125, 0.5, 0.875, 1.0] {
                    let constants = [("TEST_DISTANCE", distance), ("TEST_COARSE_DISTANCE", coarse_distance), ("TEST_FADE", fade), ("TEST_SPLIT", split), ("SCENE_SAMPLES", samples as f64)];
                    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: None, layout: None,
                        vertex: wgpu::VertexState { module: &module, entry_point: Some("test_vertex"), buffers: &[], compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() } },
                        primitive: Default::default(), depth_stencil: None, multisample: wgpu::MultisampleState { count: samples, alpha_to_coverage_enabled: samples > 1, ..Default::default() },
                        fragment: Some(wgpu::FragmentState {
                            module: &module, entry_point: Some("fs_opaque"),
                            targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: Some(wgpu::BlendState {
                                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                                alpha: wgpu::BlendComponent::REPLACE,
                            }), write_mask: wgpu::ColorWrites::ALL })],
                            compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                        }),
                        multiview_mask: None, cache: None,
                    });
                    let read = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 1024, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
                    let mut encoder = device.create_command_encoder(&Default::default());
                    let view = target.create_view(&Default::default());
                    let msaa_view = multisampled.as_ref().map(|t| t.create_view(&Default::default()));
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: msaa_view.as_ref().unwrap_or(&view), depth_slice: None, resolve_target: msaa_view.as_ref().map(|_| &view), ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                            ..Default::default()
                        });
                        pass.set_pipeline(&pipeline);
                        pass.draw(0..3, 0..if split < 0.0 { 1 } else { 2 });
                    }
                    encoder.copy_texture_to_buffer(target.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &read, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: Some(4) } }, target.size());
                    queue.submit([encoder.finish()]);
                    read.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
                    device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                    let bytes = read.get_mapped_range(..).unwrap();
                    let pixels: Vec<u8> = (0..4).flat_map(|y| (0..4).map(move |x| y * 256 + x * 4)).map(|i| bytes[i]).collect();
                    if split < 0.0 {
                        if samples == 1 { assert_eq!(pixels.iter().filter(|&&p| p == 64).count(), expected, "distance {distance}, handover {fade}: single-sample covered pixels"); }
                        baseline = pixels;
                    } else if distance == 30.0 && coarse_distance == 30.0 {
                        assert_eq!(pixels, baseline, "handover {fade}, LOD split {split}, samples {samples}: opaque adjacent meshes must cover exactly what one mesh covered");
                    } else {
                        assert!(pixels.iter().all(|&p| p <= 64), "LOD intervals must never overlap, even with different coverage");
                        if samples == 1 {
                            let coverage = |d| if d <= 10.0 { 0.0 } else if d == 18.0 { 0.5 } else { 1.0 };
                            let want = (16.0 * fade * (split * coverage(distance) + (1.0 - split) * coverage(coarse_distance))).round() as i32;
                            let drawn = pixels.iter().filter(|&&p| p == 64).count() as i32;
                            assert!((drawn - want).abs() <= 1, "distance {distance}/{coarse_distance}, handover {fade}, split {split}: coverage {drawn}, expected {want}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_models_shipped_load() {
        let (models, vertices, indices) = parse(glb()).unwrap();
        assert!(!models.is_empty());
        for m in &models {
            // (Props fitted to their paintings, a ruin's blocks and stumps, are modelled at about a
            // unit and scaled where they stand.)
            assert!(m.height > 0.5 && m.width > 0.5, "{}: {} x {}", m.name, m.height, m.width);
            assert!(!m.points.is_empty(), "{}: no points", m.name);
            for (k, l) in m.lods.iter().enumerate() {
                let n = (l.opaque.len() + l.cutout.len()) / 3;
                assert!(n > 0, "{} lod {k}: no triangles", m.name);
                for r in [&l.opaque, &l.cutout] {
                    for &i in &indices[r.start as usize..r.end as usize] {
                        assert!(((l.base_vertex as u32) + i) < vertices.len() as u32, "{} lod {k}: index out of range", m.name);
                    }
                }
            }
            // Coarser levels have fewer triangles.
            let tris = |k: usize| m.lods[k].opaque.len() + m.lods[k].cutout.len();
            assert!(tris(0) >= tris(1) && tris(1) >= tris(2), "{}: levels of detail grow", m.name);
        }
        // Every vertex names a layer there is.
        assert!(vertices.iter().all(|v| (v.layer as usize) < LAYERS.len()));
    }

    /// The festive models are TA Reborn's own, in their own colours; the shader's light factors are
    /// those the game divides an own model's colours by.
    #[test]
    fn the_festive_models_keep_their_own_colours() {
        let library = FoliageLibrary::load().unwrap();
        for name in ["snowpine_a", "festive_fir_a", "cottage_a", "clock_tower_a", "ice_star_a", "snowrock_a"] {
            let id = library.id(name).unwrap_or_else(|| panic!("{name} missing"));
            let own = library.get(id).own.unwrap_or_else(|| panic!("{name}: not its own colours"));
            let (leaf, bark) = library.own_colours(&own);
            let needles = library.layer_mean[1];
            if own.leaf.is_none() {
                assert!((0..3).all(|i| (leaf[i] * LEAF_LIGHT - needles[i]).abs() < 1e-5), "{name}");
            }
            let barked = own.bark.unwrap_or(7);
            assert!((0..3).all(|i| (bark[i] * BARK_LIGHT - library.layer_mean[barked][i]).abs() < 1e-5), "{name}");
        }
        assert!(library.get(library.id("snowpine_a").unwrap()).own.unwrap().snow > 0.3);
        assert!(library.get(library.id("ice_star_a").unwrap()).own.unwrap().leaf.is_some());
        // (No model of TA's paintings has colours of its own.)
        assert!(library.get(library.id("conifer_a").unwrap()).own.is_none());
        let shader = include_str!("shaders/foliage.wgsl");
        assert!(shader.contains(&format!("const LEAF_LIGHT: f32 = {LEAF_LIGHT:?};")) && shader.contains(&format!("const BARK_LIGHT: f32 = {BARK_LIGHT:?};")));
        assert!(shader.contains(&format!("array<vec4<f32>, {}>", LAYERS.len())));
        for (i, l) in LAYERS.iter().enumerate() {
            let name = format!("const {}_LAYER: u32 = {i}u;", l.to_ascii_uppercase());
            if ["rock", "crystal", "hull", "metal", "trim", "twigs", "snow", "festive"].contains(l) {
                assert!(shader.contains(&name), "foliage.wgsl: {name}");
            }
        }
    }

    #[test]
    fn the_textures_shipped_load() {
        for (name, png) in LAYERS.iter().zip(pngs()) {
            let t = texels(png).unwrap_or_else(|e| panic!("{name}: {e}"));
            let covered = t.as_chunks::<4>().0.iter().filter(|p| p[3] >= 128).count() as f32 / (t.len() / 4) as f32;
            if LAYERS.iter().position(|l| l == name).unwrap() < FIRST_OPAQUE {
                assert!(covered > 0.05 && covered < 0.95, "{name}: a cut-out covering {covered}");
            } else {
                assert!(covered > 0.999, "{name}: opaque but covering {covered}");
            }
        }
    }

    /// The cut-out mips keep the full texture's coverage, so far trees do not thin out.
    #[test]
    fn cutout_mips_keep_their_coverage() {
        let t = texels(pngs()[0]).unwrap();
        let covered = |d: &[u8]| d.as_chunks::<4>().0.iter().filter(|p| p[3] >= 128).count() as f32 / (d.len() / 4) as f32;
        let target = covered(&t);
        for (w, _, data) in cutout_mips(&t).iter().take(7) {
            assert!((covered(data) - target).abs() < 0.06, "level {w}: {} against {target}", covered(data));
        }
    }

    #[test]
    fn ta_height_lifts_by_half() {
        let m = FoliageModel { name: "pole".into(), height: 100.0, width: 1.0, crown: [0.0; 4], contact: 0.0, length: 0.0, breadth: 0.0, points: vec![Vec3::ZERO, Vec3::new(0.0, 100.0, 0.0), Vec3::new(0.0, 0.0, -10.0)], own: None, lods: Default::default() };
        assert_eq!(m.ta_height(1.0, 1.0), 50.0);
        // Ground behind it (north, -z) shows above its foot.
        let m = FoliageModel { points: vec![Vec3::new(0.0, 0.0, -60.0)], ..m };
        assert_eq!(m.ta_height(1.0, 1.0), 60.0);
    }
}
