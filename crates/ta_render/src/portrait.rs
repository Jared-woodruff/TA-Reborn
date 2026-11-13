//! Portraits: a unit's model drawn on its own, as the interface's picture of the unit (the build
//! menu, the selection's card, the information page), in place of TA's build pictures (palette
//! art 64 or 96 pixels across, each on a backdrop of its own, soft and speckled once enlarged).
//!
//! The model is the one the battle draws, Reborn's kit and paint and all, with the battle's own
//! shader, posed as the game gives it ([`PortraitRequest::pieces`]); lit as in a studio: a key
//! light over the viewer's left shoulder and the sky's light, and nothing of the battle (no
//! shadows, haze, weather, fog of war or point lights). It is framed from the front and to its
//! left, a little from above, as TA's pictures are, drawn at twice the size asked for (whatever
//! the multisampling) and brought down to it, on a clear background (the interface sets it on a
//! backdrop of its own), tone mapped as the battle is. The picture holds sRGB-encoded colour
//! with its alpha premultiplied, in a plain (not sRGB-aware) texture, as egui's own textures
//! are: egui samples the values as they are.

use bytemuck::Zeroable;
use glam::{Mat4, Vec3};

use crate::model::{DrawInstance, ModelId, ModelRenderer};
use crate::util::{self, Target};
use crate::{FrameResources, FrameUniform, Renderer, frame_group};

/// A model to draw: its pieces' matrices as posed (the model's origin at the world's), the
/// player's colour it wears, and the picture's size in pixels.
pub struct PortraitRequest<'a> {
    pub model: ModelId,
    pub pieces: &'a [Mat4],
    pub team: u32,
    pub size: u32,
    /// It stands on land (`UnitDef::stands_on_land`): what its model reaches under the ground
    /// (below the world's y 0) is left out of the picture and of its framing, as a battle's
    /// ground hides it.
    pub ground: bool,
}

/// The picture's format.
pub const PORTRAIT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The view's vertical field of view (degrees): long, so that the unit is not distorted.
const FOV: f32 = 24.0;
/// Where the viewer stands: round to the unit's left from its front, and up (degrees).
const YAW: f32 = -38.0;
const PITCH: f32 = 26.0;
/// The share of the picture the unit's outline keeps clear of at its edges, and how far over
/// the middle it stands (both of half the picture's height): the interface letters a build
/// picture's costs along its foot.
const MARGIN: f32 = 0.07;
const LIFT: f32 = 0.16;

/// What makes portraits: the targets they are drawn in, the studio's frame, and the pass that
/// brings the picture down to its size.
pub struct PortraitStudio {
    frame_buffer: wgpu::Buffer,
    /// The finishing pass's exposure.
    params: wgpu::Buffer,
    finish: wgpu::RenderPipeline,
    finish_layout: wgpu::BindGroupLayout,
    /// Scratch targets of the last size drawn (twice the picture's): the scene's colour
    /// (multisampled as the battle's), its depth, and the colour resolved.
    targets: Option<(u32, Target, Target, Option<Target>)>,
}

impl PortraitStudio {
    pub fn new(renderer: &Renderer) -> PortraitStudio {
        Self::with_device(&renderer.gpu.device)
    }

    pub(crate) fn with_device(device: &wgpu::Device) -> PortraitStudio {
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame uniform (portrait)"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor { label: Some("portrait finish params"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let finish_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("portrait finish"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("portrait finish"), source: wgpu::ShaderSource::Wgsl(FINISH.into()) });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("portrait finish"), bind_group_layouts: &[Some(&finish_layout)], immediate_size: 0 });
        let finish = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("portrait finish"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format: PORTRAIT_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        PortraitStudio { frame_buffer, params, finish, finish_layout, targets: None }
    }

    /// Draws a portrait; none without the models, or for a model with nothing to see. (Call it
    /// between frames, before the frame's models are prepared: it draws with their buffers.)
    pub fn render(&mut self, renderer: &mut Renderer, request: &PortraitRequest) -> Option<wgpu::Texture> {
        let settings = StudioSettings { samples: renderer.msaa_samples(), art_reborn: renderer.art_style_reborn,
            colour_blind: renderer.colour_blind, hd: renderer.lighting.hd_surfaces.clamp(0.0, 1.0), exposure: renderer.post.settings.exposure, frame_scale: 1.0 };
        let Renderer { gpu, models, frame_layout, shadow_dummy, shadow_sampler, light_buffer, light_grid_buffer, los, los_sampler, occlusion, cloud_shadows, terrain_shadow, ray_tracing, .. } = renderer;
        let models = models.as_mut()?;
        let (device, queue) = (&gpu.device, &gpu.queue);
        let resources = FrameResources {
            frame_buffer: &self.frame_buffer,
            shadow_view: &shadow_dummy.view,
            shadow_sampler,
            light_buffer,
            light_grid_buffer,
            los_view: &los.view,
            los_sampler,
            occlusion_view: &occlusion.target.view,
            cloud_view: &cloud_shadows.target.view,
            ground_view: &terrain_shadow.target.view,
            // (No trees or rocks in the studio: the features' shadow map is a dummy.)
            rt_views: ray_tracing.as_ref().map(|r| (r.result_view(), r.reflection_view(), &shadow_dummy.view)),
        };
        let frame = frame_group(device, "frame (portrait)", frame_layout, &resources);
        self.render_model(device, queue, models, &frame, request, settings)
    }

    pub(crate) fn frame_buffer(&self) -> &wgpu::Buffer { &self.frame_buffer }

    /// Both the game's portraits and the launcher's headless previews use this exact pass.
    pub(crate) fn render_model(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, models: &mut ModelRenderer,
        frame: &wgpu::BindGroup, request: &PortraitRequest, settings: StudioSettings) -> Option<wgpu::Texture> {
        let size = request.size.clamp(8, 1024);
        let inner = size * 2;
        let StudioSettings { samples, art_reborn, colour_blind, hd, exposure, frame_scale } = settings;
        // (Framed by what is above the ground of a unit standing on land.)
        let points: Vec<Vec3> = models.library.corners(request.model)
            .filter_map(|(piece, p)| request.pieces.get(piece as usize).filter(|m| **m != Mat4::ZERO).map(|m| m.transform_point3(p)))
            .map(|p| if request.ground { Vec3::new(p.x, p.y.max(0.0), p.z) } else { p })
            .collect();
        let (view_proj, eye, light) = frame_model(&points)?;
        // Rotating launcher previews leave room for Reborn's added barrel/joint geometry,
        // which can extend beyond the original piece corners used to fit a portrait.
        let scale = frame_scale.clamp(0.5, 1.0);
        let view_proj = Mat4::from_scale(Vec3::new(scale, scale, 1.0)) * view_proj;
        let u = studio_uniform(view_proj, eye, light, inner, art_reborn, colour_blind, hd);
        queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&u));
        models.set_art_style(art_reborn);
        models.prepare(
            device,
            queue,
            [DrawInstance {
                model: request.model,
                pieces: request.pieces,
                team: request.team,
                flags: if request.ground { crate::model::FLAG_ABOVE_GROUND } else { 0 },
                build: 1.0,
                damage: 0.0,
                piece: None,
                travel: [0.0; 2],
                playing: crate::model::PLAYING_ALL,
            }],
            None,
            false,
        );
        if self.targets.as_ref().is_none_or(|t| t.0 != inner) {
            let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let color = Target::new(device, "portrait colour", inner, inner, util::HDR_FORMAT, samples, usage);
            let depth = Target::new(device, "portrait depth", inner, inner, util::DEPTH_FORMAT, samples, wgpu::TextureUsages::RENDER_ATTACHMENT);
            let resolve = (samples > 1).then(|| Target::new(device, "portrait resolve", inner, inner, util::HDR_FORMAT, 1, usage));
            self.targets = Some((inner, color, depth, resolve));
        }
        let (_, color, depth, resolve) = self.targets.as_ref()?;
        let picture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("portrait"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PORTRAIT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let picture_view = picture.create_view(&wgpu::TextureViewDescriptor::default());
        let scene = resolve.as_ref().unwrap_or(color);
        let finish = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("portrait finish"),
            layout: &self.finish_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&scene.view) },
                wgpu::BindGroupEntry { binding: 1, resource: self.params.as_entire_binding() },
            ],
        });
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&[exposure, 0.0f32, 0.0, 0.0]));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("portrait") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("portrait"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color.view,
                    resolve_target: resolve.as_ref().map(|r| &r.view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth.view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, frame, &[]);
            models.draw(&mut pass);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("portrait finish"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &picture_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.finish);
            pass.set_bind_group(0, &finish, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit(Some(encoder.finish()));
        Some(picture)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct StudioSettings {
    pub samples: u32,
    pub art_reborn: bool,
    pub colour_blind: bool,
    pub hd: f32,
    pub exposure: f32,
    pub frame_scale: f32,
}

/// Frames a model whose corners are `corners` (posed): the view's matrix, the eye, and the
/// direction towards the key light. None for a model with nothing to see.
fn frame_model(corners: &[Vec3]) -> Option<(Mat4, Vec3, Vec3)> {
    let (lo, hi) = corners.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    if corners.is_empty() || (hi - lo).max_element() <= 0.0 {
        return None;
    }
    let (yaw, pitch) = (YAW.to_radians(), PITCH.to_radians());
    // Units face +z: the viewer stands out in front, round to the left.
    let to_eye = Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()).normalize();
    let forward = -to_eye;
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let tan = (FOV.to_radians() * 0.5).tan();
    let (half_x, half_y) = (tan * (1.0 - MARGIN), tan * (1.0 - MARGIN - LIFT));
    // Far enough back that every corner is in the picture, then the picture centred on them.
    let mut target = (lo + hi) * 0.5;
    let mut distance = 1.0f32;
    for _ in 0..2 {
        distance = corners
            .iter()
            .map(|&c| {
                let p = c - target;
                let depth = p.dot(forward);
                (p.dot(right).abs() / half_x - depth).max(p.dot(up).abs() / half_y - depth)
            })
            .fold(1.0f32, f32::max);
        // Centre the corners' extent on the view.
        let eye = target - forward * distance;
        let (mut lo, mut hi) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
        for &c in corners {
            let p = c - eye;
            let s = glam::Vec2::new(p.dot(right), p.dot(up)) / p.dot(forward).max(1e-3);
            lo = lo.min(s);
            hi = hi.max(s);
        }
        let mid = (lo + hi) * 0.5;
        target += (right * mid.x + up * mid.y) * distance;
    }
    // Standing over the middle of the picture.
    target -= up * (LIFT * tan * distance);
    let eye = target - forward * distance;
    let reach = corners.iter().map(|&c| (c - eye).length()).fold(0.0f32, f32::max);
    let near = (distance - reach * 1.2).max(distance * 0.05).max(0.5);
    let far = distance + reach * 1.2;
    let view = glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Y);
    let proj = glam::camera::rh::proj::directx::perspective(FOV.to_radians(), 1.0, near, far);
    // The key light over the viewer's left shoulder, high.
    let light = (up * 0.9 - right * 0.75 + to_eye * 0.55).normalize();
    (distance.is_finite()).then_some((proj * view, eye, light))
}

/// The studio's frame: the view, the key light and the sky's light; nothing of the battle.
fn studio_uniform(view_proj: Mat4, eye: Vec3, light: Vec3, size: u32, art_reborn: bool, colour_blind: bool, hd: f32) -> FrameUniform {
    let mut u = FrameUniform::zeroed();
    u.view_proj = view_proj.to_cols_array_2d();
    u.inv_view_proj = view_proj.inverse().to_cols_array_2d();
    u.camera_pos = [eye.x, eye.y, eye.z, 1.0];
    u.sun_dir = [light.x, light.y, light.z, 1.15];
    u.sun_color = [1.0, 0.97, 0.92, 0.0];
    u.ambient = [0.46, 0.48, 0.53, 1.0];
    // (No sea: nothing is under water.)
    u.world = [0.0, -1.0e6, 0.0, 0.0];
    // No shadows.
    u.screen = [size as f32, size as f32, 1.0, 0.0];
    u.fog = [0.62, 0.64, 0.67, 0.0];
    u.sky_zenith = [0.42, 0.53, 0.68, 0.0];
    // The sky in what mirrors it (no sun's disc, still clouds), its far plane far.
    u.sky = [0.0, 0.0, 1.0e6, 1.0];
    u.ao = [0.0, 0.0, 0.0, if art_reborn { 1.0 } else { 0.0 }];
    u.detail = [hd, hd, 0.0, if colour_blind { 1.0 } else { 0.0 }];
    u.daylight = [1.0, 1.0, 1.0, 0.0];
    u.celestial = [light.x, light.y, light.z, 0.0];
    u.shadow_soft = [0.0, 1.0, 1.0, 1.0];
    u
}

/// The finishing pass: the scene's colour brought down to the picture's size (each pixel the
/// mean of four, their alpha the unit's coverage), tone mapped as the battle's picture is,
/// encoded as sRGB and its alpha premultiplied (egui's convention).
const FINISH: &str = r#"
@group(0) @binding(0) var scene: texture_2d<f32>;
// x: the exposure.
@group(0) @binding(1) var<uniform> params: vec4<f32>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VOut;
    out.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

// As the battle's picture: identity below 0.8, a smooth shoulder above it (post.wgsl).
fn shoulder(x: f32) -> f32 {
    let k = 0.8;
    if (x <= k) {
        return x;
    }
    return k + (1.0 - k) * (1.0 - exp(-(x - k) / (1.0 - k)));
}

fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let peak = max(max(c.r, c.g), c.b);
    if (peak <= 0.8) {
        return c;
    }
    let mapped = shoulder(peak);
    let whiteness = clamp((peak - 1.0) / 6.0, 0.0, 1.0);
    return mix(c * (mapped / peak), vec3<f32>(mapped), whiteness * 0.6);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy) * 2;
    var sum = vec4<f32>(0.0);
    for (var k = 0; k < 4; k++) {
        sum += max(textureLoad(scene, p + vec2<i32>(k & 1, k >> 1u), 0), vec4<f32>(0.0));
    }
    let c = sum * 0.25;
    let a = clamp(c.a, 0.0, 1.0);
    if (a <= 0.0) {
        return vec4<f32>(0.0);
    }
    let rgb = clamp(tonemap(c.rgb / a * params.x), vec3<f32>(0.0), vec3<f32>(1.0));
    let encoded = select(1.055 * pow(rgb, vec3<f32>(1.0 / 2.4)) - 0.055, rgb * 12.92, rgb <= vec3<f32>(0.0031308));
    return vec4<f32>(encoded * a, a);
}
"#;
