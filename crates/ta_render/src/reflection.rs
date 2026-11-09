//! Planar reflections: the world above the water mirrored in it (perspective camera; see
//! [`Lighting::water_reflections`](crate::Lighting::water_reflections)).
//!
//! TA has one sea level per map, so every sea is the same horizontal plane and a mirrored
//! camera gives an exact reflection. Before the scene pass, the ground (on the map and beyond
//! it), units and 3D features, the 3D trees and rocks (their middle level of detail at the
//! finest), tree billboards, effect sprites, smoke, particles and beams are drawn once
//! more into an HDR target of their own (no MSAA; half or full resolution, with its own depth
//! buffer), seen through the view mirrored about the plane `y = sea level` ([`mirror`]). The
//! pipelines of that pass use `*_reflection` fragment entry points that discard everything
//! below the water, so only what stands above it is mirrored (and, as a 3D tree takes over from
//! its painting, split the pixels between the two by an ordered dither: one sample a pixel has no
//! coverage to crossfade with), and count clockwise triangles as
//! front faces ([`primitive`]): the mirror turns every triangle over, and models light their
//! back faces by `front_facing`. The pass reuses everything the scene pass draws with (terrain
//! chunks, model instances, sprite batches, point lights, the shadow, cloud and occlusion maps,
//! fog of war); only the frame uniform differs: a second one with the mirrored view-projection,
//! the eye mirrored below the water (so highlights and haze follow the reflected rays) and the
//! billboards' up vector turned over (trees hang upside down from their mirrored foot).
//!
//! The target is cleared to transparent black: its alpha says where something was mirrored,
//! and elsewhere the water keeps reflecting the sky gradient. A filter chain then halves it
//! level by level into its mip chain and builds the *reach* beside it: how far beyond the water
//! surface the mirrored world lies along the eye's rays, each level keeping the nearest of the
//! overlapping source texels (`shaders/reflection_filter.wgsl`). Odd mip dimensions cover the
//! whole source with area weights, keeping colour and reach aligned at every level. The water
//! shader (`water.wgsl`) looks the reflection up through the waves: from the reach, smoothed
//! over a dozen pixels or so, it knows how far the mirrored point lies from the surface, bends
//! the reflected ray by the wave normal and looks up where the bent ray meets the mirrored world
//! at that distance, filtered by how fast that look-up moves across the screen (and by the
//! ripples too fine for the pixels). So reflections stay attached where things stand in the
//! water and waver the more the higher they reach above it, smearing into soft streaks where the
//! waves fold them instead of tearing into slices; where a bent ray leaves the screen they fade
//! back to the sky.
//!
//! Lava and painted voids mirror nothing (the pass is skipped, as it is when no water is in
//! view), and neither does the classic camera, which renders exactly as before.

use glam::{Mat4, Vec3};

use crate::util::{DEPTH_FORMAT, HDR_FORMAT, Target};

/// Format of the reach (distance beyond the water, world units) beside the reflection.
const REACH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// Levels the filter chain adds below the full reflection (fewer on a tiny target).
const FILTER_LEVELS: u32 = 5;

/// Levels below the full reflection a `width` x `height` target gets (its smallest at least
/// 4 texels a side).
fn filter_levels(width: u32, height: u32) -> u32 {
    let side = width.min(height).max(1);
    FILTER_LEVELS.min((31 - side.leading_zeros()).saturating_sub(2))
}

/// The reflection filter shader (common.wgsl is prepended by [`crate::util::shader`]).
pub(crate) fn filter_source() -> String {
    format!("{}\n{}", include_str!("shaders/fullscreen.wgsl"), include_str!("shaders/reflection_filter.wgsl"))
}

/// Planar reflections of the world in the water (perspective camera only; see
/// [`reflection`](crate::reflection)): their resolution, or off. Part of
/// [`Lighting`](crate::Lighting).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaterReflections {
    /// The water reflects the sky only.
    Off,
    /// Rendered at half the screen resolution on each axis (a quarter of the pixels).
    #[default]
    Half,
    /// Rendered at the full screen resolution.
    Full,
}

impl WaterReflections {
    /// Size of the reflection target for a `width` x `height` scene (`None` when off).
    pub fn target_size(self, width: u32, height: u32) -> Option<(u32, u32)> {
        match self {
            WaterReflections::Off => None,
            WaterReflections::Half => Some((width.div_ceil(2).max(1), height.div_ceil(2).max(1))),
            WaterReflections::Full => Some((width.max(1), height.max(1))),
        }
    }
}

/// The mirror about the water plane `y = sea`: (x, y, z) -> (x, 2 sea - y, z). Its determinant
/// is -1: it turns triangles over.
pub(crate) fn mirror(sea: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(0.0, 2.0 * sea, 0.0)) * Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0))
}

/// The billboards' up vector in the reflection pass for the scene's `up`: sprites are built
/// upright in the world and then mirrored, so they must lean the other way to face the camera
/// after the mirror, and come out upside down (see `vs_billboard` in `sprite.wgsl`).
pub(crate) fn mirrored_billboard_up(up: Vec3) -> Vec3 {
    Vec3::new(-up.x, up.y, -up.z)
}

/// Rasterisation of the pipelines that draw into the reflection: nothing is culled (as in the
/// scene pass), and since the mirror turns every triangle over, clockwise ones face the camera.
pub(crate) fn primitive() -> wgpu::PrimitiveState {
    wgpu::PrimitiveState { front_face: wgpu::FrontFace::Cw, cull_mode: None, ..Default::default() }
}

/// One pass of the filter chain: the colour level it writes, the reach level beside it, and
/// the level above (and, for the first, the depth) it reads.
struct FilterPass {
    color: wgpu::TextureView,
    reach: wgpu::TextureView,
    sources: wgpu::BindGroup,
}

/// The reflection target (colour with its mip chain, depth, the reach) and the bind group the
/// water reads it through.
struct ReflectionTarget {
    /// Level 0 of the colour alone, the reflection pass's attachment (the filter chain fills
    /// the levels below).
    attachment: wgpu::TextureView,
    depth: Target,
    passes: Vec<FilterPass>,
    group: wgpu::BindGroup,
    size: (u32, u32),
}

/// The reflection's render target, its filter chain and its bindings for the water (group 2 of
/// the water pipelines: colour and reach with their mips, a trilinear, anisotropic sampler).
pub(crate) struct Reflections {
    pub layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The filter chain: its first pass reads the reflection and its depth, the others the
    /// level above (group 1; group 0 is the scene's frame).
    first_layout: wgpu::BindGroupLayout,
    down_layout: wgpu::BindGroupLayout,
    first: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    /// Stand-ins bound while no reflection is drawn (the water then never reads them).
    none: wgpu::BindGroup,
    target: Option<ReflectionTarget>,
}

impl Reflections {
    pub fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout) -> Self {
        let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let filterable = wgpu::TextureSampleType::Float { filterable: true };
        let exact = wgpu::TextureSampleType::Float { filterable: false };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reflection"),
            entries: &[
                texture(0, filterable),
                texture(1, filterable),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("reflection"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let first_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reflection filter (first)"),
            entries: &[texture(0, exact), texture(1, wgpu::TextureSampleType::Depth)],
        });
        let down_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reflection filter"),
            entries: &[texture(0, exact), texture(2, exact)],
        });
        let shader = crate::util::shader(device, "reflection filter", &filter_source());
        let pipeline = |label: &str, sources: &wgpu::BindGroupLayout, fs: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(frame_layout), Some(sources)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_fullscreen"), buffers: &[], compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[
                        Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                        Some(wgpu::ColorTargetState { format: REACH_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                    ],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let first = pipeline("reflection filter (first)", &first_layout, "fs_reflection_first");
        let down = pipeline("reflection filter", &down_layout, "fs_reflection_down");
        let color = Target::new(device, "no reflection", 1, 1, HDR_FORMAT, 1, wgpu::TextureUsages::TEXTURE_BINDING);
        let reach = Target::new(device, "no reflection reach", 1, 1, REACH_FORMAT, 1, wgpu::TextureUsages::TEXTURE_BINDING);
        let none = Self::group(device, &layout, &sampler, &color.view, &reach.view);
        Reflections { layout, sampler, first_layout, down_layout, first, down, none, target: None }
    }

    fn group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, color: &wgpu::TextureView, reach: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reflection"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(color) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(reach) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    }

    /// Makes sure the target is `size` (width, height), recreating it when that changed.
    pub fn prepare(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        use wgpu::TextureUsages as U;
        let levels = filter_levels(size.0, size.1);
        let texture = |label: &str, (width, height): (u32, u32), format, mips| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: U::RENDER_ATTACHMENT | U::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let level = |t: &wgpu::Texture, l: u32| t.create_view(&wgpu::TextureViewDescriptor { base_mip_level: l, mip_level_count: Some(1), ..Default::default() });
        let color = texture("reflection", size, HDR_FORMAT, levels + 1);
        // (The reach starts at half the reflection's size: the water reads it smoothed.)
        let reach = texture("reflection reach", (size.0 / 2, size.1 / 2), REACH_FORMAT, levels.max(1));
        let depth = Target::new(device, "reflection depth", size.0, size.1, DEPTH_FORMAT, 1, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING);
        let passes = (1..=levels)
            .map(|l| {
                let above = level(&color, l - 1);
                let sources = if l == 1 {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("reflection filter (first)"),
                        layout: &self.first_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&above) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&depth.view) },
                        ],
                    })
                } else {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("reflection filter"),
                        layout: &self.down_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&above) },
                            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&level(&reach, l - 2)) },
                        ],
                    })
                };
                FilterPass { color: level(&color, l), reach: level(&reach, l - 1), sources }
            })
            .collect();
        let all = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let group = Self::group(device, &self.layout, &self.sampler, &all(&color), &all(&reach));
        let attachment = level(&color, 0);
        self.target = Some(ReflectionTarget { attachment, depth, passes, group, size });
    }

    /// Frees the target (reflections turned off).
    pub fn release(&mut self) {
        self.target = None;
    }

    /// The colour and depth attachments of the reflection pass (after [`Reflections::prepare`]).
    pub fn attachments(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView)> {
        self.target.as_ref().map(|t| (&t.attachment, &t.depth.view))
    }

    /// Encodes the filter chain after the reflection pass: the colour's mip chain and the reach
    /// beside it (`frame` = the scene's frame bind group: its camera unprojects the depth).
    pub fn filter(&self, encoder: &mut wgpu::CommandEncoder, frame: &wgpu::BindGroup) {
        let Some(t) = &self.target else { return };
        for (i, p) in t.passes.iter().enumerate() {
            let attachment = |view| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    depth_slice: None,
                })
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reflection filter"),
                color_attachments: &[attachment(&p.color), attachment(&p.reach)],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(if i == 0 { &self.first } else { &self.down });
            pass.set_bind_group(0, frame, &[]);
            pass.set_bind_group(1, &p.sources, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// The water's group 2: the reflection when it is drawn this frame, else the stand-ins.
    pub fn water_group(&self, drawn: bool) -> &wgpu::BindGroup {
        match &self.target {
            Some(t) if drawn => &t.group,
            _ => &self.none,
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec4Swizzles};

    use super::*;
    use crate::{Camera, CameraMode};

    /// Runs the production reduction on even and odd targets. Every source row and column
    /// contributes, a gradient stays aligned, and a close object on the final edge survives.
    #[test]
    #[ignore = "requires a GPU; run explicitly for rendering changes"]
    fn gpu_reflection_mips_preserve_odd_edges() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
        println!("reflection reduction GPU: {}", adapter.get_info().name);
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let source = format!(r#"
struct Frame {{ inv_view_proj: mat4x4<f32>, camera_pos: vec4<f32>, world: vec4<f32>, }};
var<private> frame: Frame;
{}
@group(0) @binding(0) var<storage, read_write> results: array<vec4<f32>>;
@compute @workgroup_size(8, 8)
fn check(@builtin(global_invocation_id) id: vec3<u32>) {{
    let size = textureDimensions(source_color) / 2u;
    if (any(id.xy >= size)) {{ return; }}
    let v = halve_down(vec2<i32>(id.xy));
    results[id.y * size.x + id.x] = vec4<f32>(v.color.xy, v.reach, v.color.a);
}}
"#, filter_source());
        let module = crate::util::shader_standalone(&device, "reflection reduction regression", &source);
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: None, module: &module, entry_point: Some("check"), compilation_options: Default::default(), cache: None });
        for (width, height) in [(8u32, 8u32), (9, 7), (240, 135), (961, 540)] {
            let pixels: Vec<[f32; 4]> = (0..width * height).map(|i| {
                let (x, y) = (i % width, i / width);
                [(x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32, 0.0, 1.0]
            }).collect();
            let reach: Vec<f32> = (0..width * height).map(|i| if i % width == width - 1 || i / width == height - 1 { 0.0 } else { 400.0 }).collect();
            let texture = |format, bytes: &[u8], texel_bytes| {
                let t = device.create_texture(&wgpu::TextureDescriptor { label: None, size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
                crate::util::write_texture_level(&queue, &t, 0, width, height, bytes, texel_bytes);
                t.create_view(&Default::default())
            };
            let color = texture(wgpu::TextureFormat::Rgba32Float, bytemuck::cast_slice(&pixels), 16);
            let reach = texture(wgpu::TextureFormat::R32Float, bytemuck::cast_slice(&reach), 4);
            let (w, h) = (width / 2, height / 2);
            let bytes = u64::from(w * h) * 16;
            let result = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
            let read = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
            let output = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[wgpu::BindGroupEntry { binding: 0, resource: result.as_entire_binding() }] });
            let input = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(1), entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&color) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&reach) },
            ] });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &output, &[]);
                pass.set_bind_group(1, &input, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            }
            encoder.copy_buffer_to_buffer(&result, 0, &read, 0, bytes);
            queue.submit([encoder.finish()]);
            read.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
            device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
            let data = read.get_mapped_range(..).unwrap();
            let values: &[[f32; 4]] = bytemuck::cast_slice(&data);
            let mut average = [0.0f64; 2];
            for (i, v) in values.iter().enumerate() {
                let (x, y) = (i as u32 % w, i as u32 / w);
                // Area averages of a sampled ramp can differ slightly from its continuous
                // centre, but never by half a source texel. Cropping accumulates a larger shift.
                for (axis, (p, dst, src)) in [(x, w, width), (y, h, height)].into_iter().enumerate() {
                    let center = (p as f32 + 0.5) / dst as f32;
                    assert!((v[axis] - center).abs() <= 0.25 / src as f32 + 1e-5, "{width}x{height} at {x},{y}: {v:?}");
                    average[axis] += v[axis] as f64 / values.len() as f64;
                }
                if x == w - 1 || y == h - 1 {
                    assert_eq!(v[2], 0.0, "closest reach at the last source edge must survive");
                }
                assert!((v[3] - 1.0).abs() < 1e-5);
            }
            assert!((average[0] - 0.5).abs() < 1e-5 && (average[1] - 0.5).abs() < 1e-5, "all source texels contribute equally: {average:?}");
        }
    }

    fn screen(view_proj: Mat4, p: Vec3) -> (Vec2, f32) {
        let c = view_proj * p.extend(1.0);
        (c.xy() / c.w, c.z / c.w)
    }

    fn camera() -> Camera {
        let mut cam = Camera::new(Vec2::new(1600.0, 900.0));
        cam.mode = CameraMode::Perspective;
        cam.focus = Vec3::new(2000.0, 40.0, 1500.0);
        cam.pitch = 35f32.to_radians();
        cam.yaw = 0.6;
        cam.zoom = 1.3;
        cam
    }

    #[test]
    fn the_mirror_flips_heights_about_the_sea() {
        let m = mirror(40.0);
        assert_eq!(m.transform_point3(Vec3::new(3.0, 70.0, -8.0)), Vec3::new(3.0, 10.0, -8.0));
        assert_eq!(m.transform_point3(Vec3::new(3.0, 40.0, 5.0)), Vec3::new(3.0, 40.0, 5.0));
        assert!(m.determinant() < 0.0, "the mirror turns triangles over");
        assert!((m * m).abs_diff_eq(Mat4::IDENTITY, 1e-6));
    }

    /// A point above the water drawn with the mirrored view lands where the camera sees the
    /// water reflect it: on the water point where the ray to its mirror image crosses the sea,
    /// and behind that point (so the reflection never covers what is in front of the water).
    #[test]
    fn the_mirrored_view_draws_reflections_where_the_water_shows_them() {
        let cam = camera();
        let sea = 40.0;
        let view_proj = cam.view_proj();
        let mirrored = view_proj * mirror(sea);
        let eye = cam.eye();
        for p in [Vec3::new(2050.0, 90.0, 1400.0), Vec3::new(1900.0, 41.0, 1350.0), Vec3::new(2300.0, 160.0, 1200.0)] {
            let image = Vec3::new(p.x, 2.0 * sea - p.y, p.z);
            // Where the ray from the eye to the image crosses the water.
            let t = (eye.y - sea) / (eye.y - image.y);
            let surface = eye + (image - eye) * t;
            let (drawn, depth) = screen(mirrored, p);
            let (water, water_depth) = screen(view_proj, surface);
            assert!((drawn - water).length() < 1e-4, "{p}: drawn at {drawn}, water shows it at {water}");
            assert!(depth > water_depth, "{p}: the reflection lies behind the water surface");
            // The same path, seen from the mirrored eye: what the reflection pass's frame
            // uniform uses for view directions and haze.
            let eye_m = mirror(sea).transform_point3(eye);
            assert!((eye_m.distance(p) - eye.distance(image)).abs() < 1e-2);
        }
    }

    /// Billboards: built in the world with the mirrored up vector, then drawn through the
    /// mirrored view, a sprite comes out upside down and facing the camera, exactly as tall on
    /// screen as the upright one (as `vs_billboard` places corners: `pos + right * x - up * y`,
    /// with y growing down the sprite).
    #[test]
    fn mirrored_billboards_hang_upside_down_facing_the_camera() {
        let cam = camera();
        let sea = 40.0;
        let (right, up) = cam.billboard_basis();
        let foot = Vec3::new(2020.0, sea, 1450.0);
        let corner = |view_proj: Mat4, up: Vec3, y: f32| screen(view_proj, foot - up * y).0;
        let view_proj = cam.view_proj();
        let mirrored = view_proj * mirror(sea);
        let up_m = mirrored_billboard_up(up);
        // 40 px above the foot, upright and mirrored.
        let top = corner(view_proj, up, -40.0) - corner(view_proj, up, 0.0);
        let hanging = corner(mirrored, up_m, -40.0) - corner(mirrored, up_m, 0.0);
        assert!(top.y > 0.0 && hanging.y < 0.0, "upright {top}, mirrored {hanging}");
        assert!((top + hanging).length() < top.length() * 0.02, "mirror image of the sprite: {top} vs {hanging}");
        // The mirrored sprite faces the camera: perpendicular to the view once mirrored.
        let plane_normal = right.cross(Vec3::new(up_m.x, -up_m.y, up_m.z)).normalize();
        let view = (cam.focus - cam.eye()).normalize();
        assert!(plane_normal.dot(view).abs() > 0.999);
        // Everything of an upright sprite standing on the water is above the water (kept).
        let top_world = foot - up_m * -40.0;
        assert!(top_world.y > sea);
    }

    #[test]
    fn targets_follow_the_setting() {
        assert_eq!(WaterReflections::default(), WaterReflections::Half);
        assert_eq!(WaterReflections::Off.target_size(1920, 1080), None);
        assert_eq!(WaterReflections::Half.target_size(1921, 1080), Some((961, 540)));
        assert_eq!(WaterReflections::Full.target_size(1920, 1080), Some((1920, 1080)));
    }

    /// The filter chain halves the reflection a few times, never below 4 texels a side.
    #[test]
    fn filter_levels_fit_the_target() {
        assert_eq!(filter_levels(1280, 720), FILTER_LEVELS);
        assert_eq!(filter_levels(2560, 1440), FILTER_LEVELS);
        assert_eq!(filter_levels(64, 32), 3);
        assert_eq!(filter_levels(1, 1), 0);
        for (w, h) in [(961, 540), (100, 7), (5, 5)] {
            let l = filter_levels(w, h);
            assert!(w.min(h) >> l >= 4 || l == 0, "{w}x{h}: {l} levels");
        }
    }
}
