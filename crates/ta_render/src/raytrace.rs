//! Ray tracing (optional: graphics cards with hardware ray tracing, see [`crate::Gpu::ray_tracing`]):
//! the shadows, ambient occlusion and reflections of the models, traced against the models
//! themselves. (The 3D trees and rocks are not traced: on traced surfaces their shadows come
//! from a shadow map of their own, `rt_feature_shadow`.)
//!
//! * Every piece of every model gets a bottom-level acceleration structure the first time it is
//!   drawn; every frame a top-level one places this frame's pieces with the matrices they are
//!   drawn with (hidden pieces, cloaked units and placement ghosts left out). Each instance
//!   carries its piece's entry in a table (where its model's vertices and its triangles are:
//!   the instance's 24 bits of custom data), and a record by instance of its player's colour
//!   and its treads' travel, for shading what reflections meet.
//! * A depth prepass draws the terrain and the models into a single-sampled depth target.
//! * A compute pass (raytrace.wgsl) rebuilds each pixel's position and normal from it and traces
//!   two rays through the disc of the sun (or the moon) for its shadow, two short ones over the
//!   hemisphere above it for its ambient occlusion, one towards the point light that lights it
//!   most (explosions, fires, lasers throw shadows), and on models and wet ground a mirror ray
//!   whose hit is shaded from the model's own texture. Two passes of a depth-aware blur make
//!   smooth shade of the shadows and occlusion, stored with the pixel's depth.
//! * The terrain and model shaders use the results where their fragment is the surface they were
//!   traced for (`rt_visibility` and `rt_reflection` in rt_on.wgsl); elsewhere (trees, water,
//!   effects, silhouette edges) the shadow map and the contact occlusion as before. The
//!   relief's shadows (terrain_shadow.rs) come on top, as ever.

use rustc_hash::FxHashMap;

use glam::{Mat4, Vec3};

use crate::model::{ModelId, ModelRenderer, ModelVertex};
use crate::timing::PassTimestamps;
use crate::util;

/// Top-level instances the scene starts with room for (it grows as needed).
const FIRST_CAPACITY: u32 = 1024;
/// New pieces' structures built in a frame at most (hundreds first seen at once took a frame
/// of 15 ms): the rest wait their turn, the scene using raster shadows until all are in it.
const BUILDS_PER_FRAME: usize = 64;
/// Most pieces traced in a frame (about 200,000 units' worth; scenes past that use raster
/// shadows, and the log says so once).
const MAX_INSTANCES: u32 = 1 << 21;
/// Most pieces with an entry in the table: all an instance's 24 bits of custom data hold.
const MAX_PIECES: usize = 1 << 24;
const _: () = assert!(MAX_PIECES <= 1 << 24, "a piece's entry must fit an instance's custom data");
/// Tangent of the radius of the light's disc as the shadows see it: about three times the sun's,
/// so a shadow is sharp where it meets what casts it and softens a little further off.
const LIGHT_SPREAD: f32 = 0.014;
/// How far (world units) the ambient occlusion's rays look for what hides the sky.
const AO_REACH: f32 = 30.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    inv_view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    light: [f32; 4],
    size: [u32; 4],
    depth: [f32; 4],
    grid: [f32; 4],
    grid_dims: [u32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    ground: [f32; 4],
}

/// This frame's view, light and the frame's resources, for [`RayTracing::trace`].
pub(crate) struct TraceView<'a> {
    pub view_proj: Mat4,
    pub eye: Vec3,
    /// The raster frame's animation clock, in seconds (GAF lights advance at 8 fps).
    pub time: f32,
    /// Towards the light (the sun, or the moon at night), its colour (times its strength) and
    /// the sky's.
    pub light: Vec3,
    pub sun_color: Vec3,
    pub ambient: Vec3,
    /// Near and far planes.
    pub clip: (f32, f32),
    /// Strength of the ambient occlusion (0 = none).
    pub ao: f32,
    /// The point lights' grid: corner (world xz), 1 / cell size (0: no lights), and cells per
    /// axis; its buffers are the frame's.
    pub grid: [f32; 4],
    pub grid_dims: [u32; 4],
    pub lights: &'a wgpu::Buffer,
    pub light_grid: &'a wgpu::Buffer,
    /// The ground's heights (terrain_shadow.rs) and 1 / its world extent.
    pub ground: &'a wgpu::TextureView,
    pub ground_scale: [f32; 2],
    /// How wet the ground is (0..1): wet ground is traced for its reflections.
    pub wetness: f32,
    /// Models are traced for their reflections (HD surfaces).
    pub model_reflections: bool,
}

/// The targets, at the scene's size.
struct Targets {
    size: (u32, u32),
    depth: util::Target,
    raw: util::Target,
    mid: util::Target,
    out: util::Target,
    reflections: util::Target,
}

/// What the shaders know of each top-level instance (a piece), by its index: the distance its
/// treads have driven and the angle turned (for their scrolling), its player's colour, and its
/// state: bit 0 its animated textures play (`DrawInstance::playing`), bits 8-15 how far its
/// unit's lights are out (`FLAG_LIGHTS_OUT_SHIFT`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TracedInstance {
    travel: [f32; 2],
    team: u32,
    state: u32,
}

/// A piece's acceleration structure, what it is built from, and its entry in the table.
struct PieceBlas {
    blas: wgpu::Blas,
    size: wgpu::BlasTriangleGeometrySizeDescriptor,
    first_vertex: u32,
    first_index: u32,
    id: u32,
    built: bool,
}

pub struct RayTracing {
    /// Per nonempty piece of a model: its acceleration structure (None: allocation failed).
    pieces: FxHashMap<(ModelId, u32), Option<PieceBlas>>,
    /// Per traced piece (its `id`): its model's first vertex and its first index.
    table: Vec<[u32; 2]>,
    table_buffer: wgpu::Buffer,
    table_dirty: bool,
    /// Per TLAS instance, in the same order: what the shaders know of it.
    instances: Vec<TracedInstance>,
    instance_buffer: wgpu::Buffer,
    tlas: wgpu::Tlas,
    capacity: u32,
    /// Most instances the device's top-level structures take (at most [`MAX_INSTANCES`]).
    max_instances: u32,
    /// Instances set in the top-level structure last frame.
    used: usize,
    /// Pieces were left out (too many in view, or the table full): said once in the log.
    warned: bool,
    /// The pieces whose structures the last frame built (built again if it failed: see
    /// [`RayTracing::forget_builds`]).
    built_now: Vec<(ModelId, u32)>,
    trace: wgpu::ComputePipeline,
    blur_across: wgpu::ComputePipeline,
    blur_down: wgpu::ComputePipeline,
    trace_layout: wgpu::BindGroupLayout,
    across_layout: wgpu::BindGroupLayout,
    down_layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    targets: Option<Targets>,
    /// Stand-ins for the results before the targets exist.
    dummy: util::Target,
    dummy_reflections: util::Target,
}

impl RayTracing {
    pub fn new(device: &wgpu::Device) -> Self {
        use wgpu::{BindGroupLayoutEntry as E, BindingType as B, ShaderStages as S};
        let entry = |binding, ty| E { binding, visibility: S::COMPUTE, ty, count: None };
        let uniform = |binding| entry(binding, B::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None });
        let buffer = |binding| entry(binding, B::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None });
        let texture = |binding, filterable, view_dimension| {
            entry(binding, B::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension, multisampled: false })
        };
        let depth = |binding| entry(binding, B::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false });
        let storage = |binding, format| {
            entry(binding, B::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format, view_dimension: wgpu::TextureViewDimension::D2 })
        };
        let layout = |label, entries: &[E]| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries });
        let d2 = wgpu::TextureViewDimension::D2;
        let trace_layout = layout(
            "ray tracing",
            &[
                uniform(0),
                depth(1),
                entry(2, B::AccelerationStructure { vertex_return: false }),
                storage(3, wgpu::TextureFormat::Rgba16Float),
                buffer(7),
                buffer(8),
                storage(9, wgpu::TextureFormat::Rgba16Float),
                buffer(10),
                buffer(11),
                buffer(12),
                texture(13, true, wgpu::TextureViewDimension::D2Array),
                entry(14, B::Sampler(wgpu::SamplerBindingType::Filtering)),
                texture(15, true, d2),
                buffer(19),
                texture(20, true, wgpu::TextureViewDimension::D2Array),
                texture(21, true, wgpu::TextureViewDimension::D2Array),
                texture(22, true, wgpu::TextureViewDimension::D2Array),
                texture(23, true, wgpu::TextureViewDimension::D2Array),
                uniform(24),
                // The paint's second bank (model.rs `TEXTURE_BANKS`).
                texture(25, true, wgpu::TextureViewDimension::D2Array),
                texture(26, true, wgpu::TextureViewDimension::D2Array),
                texture(27, true, wgpu::TextureViewDimension::D2Array),
            ],
        );
        let across_layout = layout("ray tracing blur across", &[uniform(0), depth(1), texture(4, false, d2), storage(5, wgpu::TextureFormat::Rgba16Float)]);
        let down_layout = layout("ray tracing blur down", &[uniform(0), depth(1), texture(4, false, d2), storage(6, wgpu::TextureFormat::Rgba32Float)]);
        let shader = util::shader_standalone(device, "ray tracing", &shader_source());
        let pipeline = |label: &str, layout: &wgpu::BindGroupLayout, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let trace = pipeline("ray tracing", &trace_layout, "trace");
        let blur_across = pipeline("ray tracing blur across", &across_layout, "blur_across");
        let blur_down = pipeline("ray tracing blur down", &down_layout, "blur_down");
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ray tracing params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ray tracing"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let stand_in = |label, format| util::Target::new(device, label, 1, 1, format, 1, wgpu::TextureUsages::TEXTURE_BINDING);
        RayTracing {
            pieces: FxHashMap::default(),
            table: Vec::new(),
            table_buffer: Self::make_table(device, 1024),
            table_dirty: false,
            instances: Vec::new(),
            instance_buffer: Self::make_instances(device, FIRST_CAPACITY),
            tlas: Self::make_tlas(device, FIRST_CAPACITY),
            capacity: FIRST_CAPACITY,
            max_instances: MAX_INSTANCES.min(device.limits().max_tlas_instance_count.max(FIRST_CAPACITY)),
            used: 0,
            warned: false,
            built_now: Vec::new(),
            trace,
            blur_across,
            blur_down,
            trace_layout,
            across_layout,
            down_layout,
            params,
            sampler,
            targets: None,
            dummy: stand_in("ray tracing stand-in", wgpu::TextureFormat::Rgba32Float),
            dummy_reflections: stand_in("ray tracing reflections stand-in", wgpu::TextureFormat::Rgba16Float),
        }
    }

    fn make_tlas(device: &wgpu::Device, capacity: u32) -> wgpu::Tlas {
        device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("ray traced scene"),
            max_instances: capacity,
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        })
    }

    fn make_table(device: &wgpu::Device, entries: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ray traced pieces"),
            size: (entries.max(1) * 8) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn make_instances(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ray traced instances"),
            size: capacity.clamp(1, MAX_INSTANCES) as u64 * std::mem::size_of::<TracedInstance>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Makes the targets for a scene `size` pixels wide and high; true when they were remade
    /// (the frame bind group then needs the new result views).
    pub(crate) fn ensure_targets(&mut self, device: &wgpu::Device, size: (u32, u32)) -> bool {
        let size = (size.0.max(1), size.1.max(1));
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return false;
        }
        use wgpu::TextureUsages as U;
        let target = |label, format, usage| util::Target::new(device, label, size.0, size.1, format, 1, usage);
        let rw = U::STORAGE_BINDING | U::TEXTURE_BINDING;
        self.targets = Some(Targets {
            size,
            depth: target("ray tracing depth", util::DEPTH_FORMAT, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING),
            raw: target("ray tracing rays", wgpu::TextureFormat::Rgba16Float, rw),
            mid: target("ray tracing blur", wgpu::TextureFormat::Rgba16Float, rw),
            out: target("ray tracing result", wgpu::TextureFormat::Rgba32Float, rw),
            reflections: target("ray tracing reflections", wgpu::TextureFormat::Rgba16Float, rw),
        });
        true
    }

    /// The depth prepass's target.
    pub(crate) fn depth_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.depth.view)
    }

    /// The results the scene's shaders read (group 0, bindings 10 and 11).
    pub(crate) fn result_view(&self) -> &wgpu::TextureView {
        self.targets.as_ref().map_or(&self.dummy.view, |t| &t.out.view)
    }

    pub(crate) fn reflection_view(&self) -> &wgpu::TextureView {
        self.targets.as_ref().map_or(&self.dummy_reflections.view, |t| &t.reflections.view)
    }

    /// Builds the acceleration structures for this frame's models: the pieces seen for the
    /// first time, then the scene placing every piece. Returns whether every nonempty piece
    /// is present. A partial scene cannot replace raster shadows: its missing casters would
    /// leave pixels falsely lit, even though the depth prepass still sees those models.
    pub(crate) fn build_scene(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, models: &ModelRenderer) -> bool {
        let Some((vertices, indices)) = models.library.buffers() else { return false };
        let mut count = 0u32;
        let mut table_full = false;
        for (model, piece, ..) in models.traced_pieces() {
            if models.library.models[model as usize].pieces[piece as usize].index_count < 3 {
                continue;
            }
            if !self.pieces.contains_key(&(model, piece)) {
                let id = self.table.len() as u32;
                table_full |= self.table.len() >= MAX_PIECES;
                let made = (self.table.len() < MAX_PIECES).then(|| unless_short(device, || piece_blas(device, models, model, piece, id))).flatten().flatten();
                if let Some(p) = &made {
                    self.table.push([p.first_vertex, p.first_index]);
                    self.table_dirty = true;
                }
                self.pieces.insert((model, piece), made);
            }
            count = count.saturating_add(1);
        }
        if (table_full || count > self.max_instances) && !self.warned {
            self.warned = true;
            log::warn!(
                "ray tracing: {count} pieces in view (at most {} traced), {} kinds of piece (at most {MAX_PIECES}); incomplete scenes use raster shadows",
                self.max_instances,
                self.table.len()
            );
        }
        if self.table_dirty {
            if (self.table_buffer.size() as usize) < self.table.len() * 8 {
                self.table_buffer = Self::make_table(device, self.table.len().next_power_of_two());
            }
            queue.write_buffer(&self.table_buffer, 0, bytemuck::cast_slice(&self.table));
            self.table_dirty = false;
        }
        let required = count;
        let count = count.min(self.max_instances);
        if count > self.capacity {
            let capacity = count.next_power_of_two().min(self.max_instances);
            match unless_short(device, || (Self::make_tlas(device, capacity), Self::make_instances(device, capacity))) {
                Some((tlas, instances)) => {
                    (self.capacity, self.tlas, self.instance_buffer, self.used) = (capacity, tlas, instances, 0);
                }
                // Short of memory, keep the allocation but use raster shadows for any scene
                // that no longer fits it.
                None => self.max_instances = self.capacity,
            }
        } else if count < self.capacity / 4 && self.capacity > FIRST_CAPACITY {
            // Far fewer in view than it was made for: a smaller one, built faster every frame.
            let capacity = count.next_power_of_two().max(FIRST_CAPACITY);
            if let Some((tlas, instances)) = unless_short(device, || (Self::make_tlas(device, capacity), Self::make_instances(device, capacity))) {
                (self.capacity, self.tlas, self.instance_buffer, self.used) = (capacity, tlas, instances, 0);
            }
        }
        // The new pieces built this frame (see `BUILDS_PER_FRAME`).
        self.built_now.clear();
        for (key, p) in &mut self.pieces {
            if self.built_now.len() >= BUILDS_PER_FRAME {
                break;
            }
            if let Some(p) = p && !p.built {
                p.built = true;
                self.built_now.push(*key);
            }
        }
        let count = count.min(self.capacity);
        self.instances.clear();
        let mut placed = 0usize;
        for (model, piece, team, transform, travel, state) in models.traced_pieces() {
            if placed >= count as usize {
                break;
            }
            if let Some(Some(p)) = self.pieces.get(&(model, piece)) && p.built {
                self.tlas[placed] = Some(wgpu::TlasInstance::new(&p.blas, transform, p.id, 0xFF));
                // Empty, hidden or untraceable pieces must not take an entry: the shader
                // addresses this with the intersection's TLAS instance_index.
                self.instances.push(TracedInstance { travel, team: team.min(9), state });
                placed += 1;
            }
        }
        for i in placed..self.used {
            self.tlas[i] = None;
        }
        self.used = placed;
        if !self.instances.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&self.instances));
        }
        let stride = std::mem::size_of::<ModelVertex>() as u64;
        let fresh: Vec<wgpu::BlasBuildEntry<'_>> = self
            .built_now
            .iter()
            .filter_map(|key| self.pieces.get(key).and_then(|p| p.as_ref()))
            .map(|p| wgpu::BlasBuildEntry {
                blas: &p.blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                    size: &p.size,
                    vertex_buffer: vertices,
                    first_vertex: p.first_vertex,
                    vertex_stride: stride,
                    index_buffer: Some(indices),
                    first_index: Some(p.first_index),
                    transform_buffer: None,
                    transform_buffer_offset: None,
                }]),
            })
            .collect();
        encoder.build_acceleration_structures(fresh.iter(), std::iter::once(&self.tlas));
        drop(fresh);
        scene_is_complete(required, placed)
    }

    /// The frame that built the last pieces failed (its submission refused, out of memory):
    /// they are built again with the next, and the table written again. (Taken for built,
    /// every later frame would trace pieces never built, and draw black.)
    pub(crate) fn forget_builds(&mut self) {
        for key in self.built_now.drain(..) {
            if let Some(Some(p)) = self.pieces.get_mut(&key) {
                p.built = false;
                self.table_dirty = true;
            }
        }
    }

    /// Traces the shadows, ambient occlusion and reflections of the prepass's pixels, and blurs
    /// the shade into the result.
    pub(crate) fn trace(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        models: &ModelRenderer,
        view: &TraceView<'_>,
        timestamps: Option<&PassTimestamps>,
    ) {
        let (Some(t), Some((vertices, indices))) = (&self.targets, models.library.buffers()) else { return };
        let (w, h) = t.size;
        let light = view.light.normalize_or(Vec3::Y);
        let params = Params {
            inv_view_proj: view.view_proj.inverse().to_cols_array_2d(),
            eye: view.eye.extend(view.time).to_array(),
            light: [light.x, light.y, light.z, LIGHT_SPREAD],
            size: [w, h, 0, 0],
            depth: [view.clip.0, view.clip.1, AO_REACH, view.ao],
            grid: view.grid,
            grid_dims: view.grid_dims,
            sun_color: view.sun_color.extend(0.0).to_array(),
            ambient: view.ambient.extend(0.0).to_array(),
            ground: [view.ground_scale[0], view.ground_scale[1], view.wetness, if view.model_reflections { 1.0 } else { 0.0 }],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        // (Remade every frame: the scene, the model buffers and the table change as models come.)
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let tv = wgpu::BindingResource::TextureView;
        let group = |label, layout, entries: &[wgpu::BindGroupEntry<'_>]| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout, entries });
        let params = || self.params.as_entire_binding();
        let groups = [
            group(
                "ray tracing",
                &self.trace_layout,
                &[
                    entry(0, params()),
                    entry(1, tv(&t.depth.view)),
                    entry(2, self.tlas.as_binding()),
                    entry(3, tv(&t.raw.view)),
                    entry(7, view.lights.as_entire_binding()),
                    entry(8, view.light_grid.as_entire_binding()),
                    entry(9, tv(&t.reflections.view)),
                    entry(10, self.table_buffer.as_entire_binding()),
                    entry(11, vertices.as_entire_binding()),
                    entry(12, indices.as_entire_binding()),
                    entry(13, tv(&models.textures.view)),
                    entry(14, wgpu::BindingResource::Sampler(&self.sampler)),
                    entry(15, tv(view.ground)),
                    entry(19, self.instance_buffer.as_entire_binding()),
                    entry(20, tv(&models.textures.reborn_view)),
                    entry(21, tv(&models.textures.reborn_physical)),
                    entry(22, tv(models.textures.skin_view())),
                    entry(23, tv(models.textures.enemy_skin_view())),
                    entry(24, models.textures.livery_buffer().as_entire_binding()),
                    entry(25, tv(&models.textures.second[0])),
                    entry(26, tv(&models.textures.second[1])),
                    entry(27, tv(&models.textures.second[2])),
                ],
            ),
            group("ray tracing blur across", &self.across_layout, &[entry(0, params()), entry(1, tv(&t.depth.view)), entry(4, tv(&t.raw.view)), entry(5, tv(&t.mid.view))]),
            group("ray tracing blur down", &self.down_layout, &[entry(0, params()), entry(1, tv(&t.depth.view)), entry(4, tv(&t.mid.view)), entry(6, tv(&t.out.view))]),
        ];
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("ray tracing"), timestamp_writes: timestamps.map(|t| t.compute_writes()) });
        let (gx, gy) = (w.div_ceil(8), h.div_ceil(8));
        for (pipeline, group) in [(&self.trace, &groups[0]), (&self.blur_across, &groups[1]), (&self.blur_down, &groups[2])] {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(gx, gy, 1);
        }
    }
}

/// The tracing shader's source: its liveries (`livery.wgsl`, shared with the unit shader) after
/// its `enable` directive, which must come first.
pub(crate) fn shader_source() -> String {
    let source = include_str!("shaders/raytrace.wgsl");
    let (enable, rest) = source.split_once('\n').expect("raytrace.wgsl opens with its enable directive");
    format!("{enable}\n{}\n{rest}", include_str!("shaders/livery.wgsl"))
}

/// Empty scenes and any omitted caster must leave the raster result active for the frame.
fn scene_is_complete(required: u32, placed: usize) -> bool {
    required > 0 && usize::try_from(required) == Ok(placed)
}

/// A piece's acceleration structure (not yet built), or None for a piece without triangles.
/// What `make` makes on the card, or `None` when there was no memory left for it: made so, it
/// is invalid, and every frame drawn with it fails (a battle that ran the card out of memory
/// came out black from then on).
fn unless_short<T>(device: &wgpu::Device, make: impl FnOnce() -> T) -> Option<T> {
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let made = make();
    match pollster::block_on(memory.pop()) {
        Some(e) => {
            log::warn!("ray tracing: {e}");
            None
        }
        None => Some(made),
    }
}

fn piece_blas(device: &wgpu::Device, models: &ModelRenderer, model: ModelId, piece: u32, id: u32) -> Option<PieceBlas> {
    let info = models.library.models.get(model as usize)?;
    let p = info.pieces.get(piece as usize)?;
    if p.index_count < 3 {
        return None;
    }
    let size = wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: models.library.piece_vertex_count(model, piece),
        index_format: Some(wgpu::IndexFormat::Uint32),
        index_count: Some(p.index_count),
        flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
    };
    let blas = device.create_blas(
        &wgpu::CreateBlasDescriptor {
            label: None,
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        },
        wgpu::BlasGeometrySizeDescriptors::Triangles { descriptors: vec![size.clone()] },
    );
    Some(PieceBlas { blas, size, first_vertex: info.base_vertex.max(0) as u32, first_index: p.first_index, id, built: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_ray_scenes_keep_raster_shadows() {
        assert!(!scene_is_complete(0, 0), "no scene to trace");
        assert!(scene_is_complete(1024, 1024), "all casters fit");
        assert!(!scene_is_complete(1025, 1024), "capacity leaves one caster out");
        assert!(!scene_is_complete(2, 1), "one failed BLAS must not remove its shadow");
        assert!(!scene_is_complete(1, 0), "the only BLAS failed");
        assert!(scene_is_complete(1, 1), "a later frame containing only available casters recovers");
    }

    #[test]
    fn ray_traced_instance_layout_fits_device_limits() {
        use wgpu::naga;
        let source = shader_source();
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
        let (_, instances) = module.global_variables.iter().find(|(_, g)| g.binding == Some(naga::ResourceBinding { group: 0, binding: 19 })).expect("TLAS instance binding");
        assert!(matches!(instances.space, naga::AddressSpace::Storage { access } if access == naga::StorageAccess::LOAD));
        let naga::TypeInner::Array { base, stride, .. } = module.types[instances.ty].inner else { panic!("the instances must be an array") };
        assert_eq!(stride as usize, std::mem::size_of::<TracedInstance>());
        let naga::TypeInner::Struct { ref members, .. } = module.types[base].inner else { panic!("an instance must be a struct") };
        let offset = |name: &str| members.iter().find(|m| m.name.as_deref() == Some(name)).map(|m| m.offset as usize);
        assert_eq!(offset("travel"), Some(std::mem::offset_of!(TracedInstance, travel)));
        assert_eq!(offset("team"), Some(std::mem::offset_of!(TracedInstance, team)));
        let storage_count = module.global_variables.iter().filter(|(_, g)| matches!(g.space, naga::AddressSpace::Storage { .. })).count();
        assert_eq!(storage_count, 6);
        let limits = wgpu::Limits::default();
        assert!(storage_count <= limits.max_storage_buffers_per_shader_stage as usize);
        assert!(MAX_INSTANCES as usize * stride as usize <= limits.max_storage_buffer_binding_size as usize);
        let (_, params) = module.global_variables.iter().find(|(_, g)| g.binding == Some(naga::ResourceBinding { group: 0, binding: 0 })).expect("trace parameters");
        let naga::TypeInner::Struct { ref members, span } = module.types[params.ty].inner else { panic!("trace parameters must be a struct") };
        assert_eq!(span as usize, std::mem::size_of::<Params>());
        assert_eq!(members.iter().find(|m| m.name.as_deref() == Some("eye")).unwrap().offset as usize, std::mem::offset_of!(Params, eye));
    }

    /// Execute the production shader's UV function without requiring ray-query hardware.
    /// The two sides of the same tank turn in opposite directions, reverse motion wraps,
    /// and a non-track face at u=1 must retain its edge rather than wrap to the other side.
    /// Also compare reflected team paint with the production raster palettes for every
    /// player slot, both accessibility modes and both art styles, and verify animated
    /// lights at frame boundaries and wraparound without advancing team-colour frames.
    #[test]
    #[ignore = "requires a GPU; run with --ignored"]
    fn ray_traced_tread_uv_matches_drive_and_turn_on_gpu() {
        use crate::gpu::CheckedBufferInit;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_else(crate::gpu::native_backends),
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let production = include_str!("shaders/raytrace.wgsl");
        let start = production.find("fn tread_uv(").unwrap();
        let end = production[start..].find("\nfn reflected(").unwrap() + start;
        let raster = include_str!("shaders/unit.wgsl");
        let palettes_start = raster.find("fn team_paint(").unwrap();
        let palettes_end = raster.find("\nfn xform(").unwrap();
        let conversion_start = raster.find("fn srgb_to_linear(").unwrap();
        let conversion_end = raster[conversion_start..].find("\n}").unwrap() + conversion_start + 2;
        // (With the liveries the tracing shader is made with, as `shader_source` prepends them:
        // the part taken uses them.)
        let source = format!(
            "{}\n@group(0) @binding(24) var<uniform> liveries: Liveries;\n{}\n{}\n{}\n{}",
            include_str!("shaders/livery.wgsl"),
            &production[start..end],
            &raster[palettes_start..palettes_end],
            &raster[conversion_start..conversion_end],
            r#"
struct Input { uv: vec2<f32>, tread: vec2<f32>, travel: vec2<f32> };
@group(0) @binding(0) var<storage, read> inputs: array<Input>;
@group(0) @binding(1) var<storage, read_write> results: array<vec2<f32>>;
fn paint_matches_raster() -> bool {
    let paint = vec3<f32>(0.79, 0.395, 0.1975);
    let team_word = (10u << 16u) | 100u;
    for (var flags = 0u; flags < 4u; flags++) {
        for (var team = 0u; team <= 10u; team++) {
            let accessible = (flags & 1u) != 0u;
            let colour = select(team_paint(team), team_cb(team), accessible);
            if (any(abs(reflected_team_colour(team, flags) - colour) > vec3<f32>(1e-6))) { return false; }
            if (reflected_team_layer(team_word, team, flags) != 100u + select(min(team, 9u), 2u, accessible)) { return false; }
            let tinted = select(paint, paint / 0.79 * team_cb(team), accessible);
            if (any(abs(reflected_team_tint(paint, team_word, team, flags) - tinted) > vec3<f32>(1e-6))) { return false; }
            // Solid faces and ordinary animated textures must never acquire a team tint.
            for (var frame_count = 0u; frame_count < 10u; frame_count++) {
                let word = (frame_count << 16u) | 100u;
                if (reflected_team_layer(word, team, flags) != 100u || any(reflected_team_tint(paint, word, team, flags) != paint)) { return false; }
            }
            let solid = (10u << 16u) | 0xFFFFu;
            if (reflected_team_layer(solid, team, flags) != 0xFFFFu || any(reflected_team_tint(paint, solid, team, flags) != paint)) { return false; }
        }
    }
    return true;
}
fn animation_matches_raster() -> bool {
    let times = array<f32, 8>(0.0, 0.124, 0.125, 0.375, 0.5, 1.0, 12.75, 31.875);
    let frames3 = array<u32, 8>(0u, 0u, 1u, 0u, 1u, 2u, 0u, 0u);
    let frames4 = array<u32, 8>(0u, 0u, 1u, 3u, 0u, 0u, 2u, 3u);
    let frames255 = array<u32, 8>(0u, 0u, 1u, 3u, 4u, 8u, 102u, 0u);
    for (var flags = 0u; flags < 4u; flags++) {
        for (var i = 0u; i < 8u; i++) {
            if (reflected_texture_layer((3u << 16u) | 100u, 7u, flags, times[i], true) != 100u + frames3[i]) { return false; }
            if (reflected_texture_layer((4u << 16u) | 100u, 7u, flags, times[i], true) != 100u + frames4[i]) { return false; }
            if (reflected_texture_layer((255u << 16u) | 100u, 7u, flags, times[i], true) != 100u + frames255[i]) { return false; }
            // A piece whose script holds its textures (TA's `cache`) shows their first frame.
            if (reflected_texture_layer((4u << 16u) | 100u, 7u, flags, times[i], false) != 100u) { return false; }
            // Frame zero/one and untextured geometry never animate; ten is a team palette.
            if (reflected_texture_layer(100u, 7u, flags, times[i], true) != 100u) { return false; }
            if (reflected_texture_layer((1u << 16u) | 100u, 7u, flags, times[i], true) != 100u) { return false; }
            if (reflected_texture_layer((3u << 16u) | 0xFFFFu, 7u, flags, times[i], true) != 0xFFFFu) { return false; }
            if (reflected_texture_layer((10u << 16u) | 100u, 7u, flags, times[i], true) != 100u + select(7u, 2u, (flags & 1u) != 0u)) { return false; }
            if (reflected_texture_layer((10u << 16u) | 100u, 7u, flags, times[i], false) != 100u + select(7u, 2u, (flags & 1u) != 0u)) { return false; }
        }
    }
    return true;
}
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let value = inputs[id.x];
    results[id.x] = tread_uv(value.uv, value.tread, value.travel);
    if (!paint_matches_raster() || !animation_matches_raster()) { results[id.x] = vec2<f32>(-100.0); }
}
"#
        );
        let input: [[f32; 6]; 6] = [
            [0.2, 0.7, 0.125, -8.0, 10.0, 0.25],
            [0.2, 0.7, 0.125, 8.0, 10.0, 0.25],
            [0.2, 0.7, 0.125, 0.0, -2.0, 0.0],
            [0.2, 0.7, 0.125, 0.0, 0.0, 0.0],
            [1.0, 0.7, 0.0, 8.0, 10.0, 0.25],
            [0.2, 0.7, 0.125, 0.0, 8.0, 0.0],
        ];
        let expected = [[0.7, 0.7], [0.2, 0.7], [0.95, 0.7], [0.2, 0.7], [1.0, 0.7], [0.2, 0.7]];
        let input = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("tread fixture"), contents: bytemuck::cast_slice(&input), usage: wgpu::BufferUsages::STORAGE });
        let bytes = std::mem::size_of_val(&expected) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("production tread UV fixture"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None, layout: None, module: &shader, entry_point: Some("main"), compilation_options: Default::default(), cache: None,
        });
        let layout = pipeline.get_bind_group_layout(0);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: input.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() }],
        });
        let mut previous = None;
        for _ in 0..2 {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(expected.len() as u32, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
            queue.submit([encoder.finish()]);
            readback.slice(..).map_async(wgpu::MapMode::Read, |result| result.expect("tread fixture readback"));
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let data = readback.slice(..).get_mapped_range().unwrap();
            let values: &[[f32; 2]] = bytemuck::cast_slice(&data);
            for (actual, expected) in values.iter().zip(expected) {
                for axis in 0..2 { assert!((actual[axis] - expected[axis]).abs() < 1e-6, "{actual:?} != {expected:?}"); }
            }
            if let Some(previous) = &previous { assert_eq!(&*data, previous, "travel must not depend on wall-clock time"); }
            previous = Some(data.to_vec());
            drop(data);
            readback.unmap();
        }
    }
}
