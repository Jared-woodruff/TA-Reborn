//! HDR scene targets, bloom and the final composite to the swapchain; photo mode's depth of
//! field before them (`shaders/dof.wgsl`).

use bytemuck::{Pod, Zeroable};
use crate::gpu::CheckedBufferInit;

use crate::timing::PassTimestamps;
use crate::util::{DEPTH_FORMAT, HDR_FORMAT, Target};

const BLOOM_LEVELS: usize = 6;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostUniform {
    grade: [f32; 4],
    info: [f32; 4],
    /// x = heat haze distortion in pixels per unit of heat (0 = off), y = sharpening, z = the
    /// intro room's tone curve's toe (0: not the room), w = its shoulder.
    heat: [f32; 4],
    /// x = 1 for space's look (see [`SpaceLook`]), y = the dither's strength (8-bit steps), z =
    /// its fade (the picture's values times it, as a video fade).
    look: [f32; 4],
    /// Photo mode's [`Look`]: x = contrast, y = warmth, z = grain, w = colour fringe.
    photo: [f32; 4],
    /// x = monochrome, y = the black level lifted, z = its special (1 thermal, 2 night sight),
    /// w = 1 when a look is on.
    photo_tone: [f32; 4],
    /// The tints of the shadows and of the highlights (rgb, and how much).
    shadows: [f32; 4],
    highlights: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BloomUniform {
    texel: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DofUniform {
    /// x = mode (1 lens, 2 tilt-shift), y = focus, z = the largest blur's radius in pixels, w =
    /// the tilt-shift band's half height.
    params: [f32; 4],
    /// xyz = [`crate::Camera::depth_mapping`], w = the tiles a blur reaches across.
    clip: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct PostSettings {
    pub exposure: f32,
    pub bloom: f32,
    pub bloom_threshold: f32,
    pub vignette: f32,
    pub saturation: f32,
    /// Strength of the heat haze over explosions, fires and lava seas (perspective camera;
    /// 0 = off, 1 = default, 2 = strong). See [`crate::heat`].
    pub heat_haze: f32,
    /// The perspective camera's cinematic colour grade (0 = TA's colours exactly, 1 = full):
    /// a gentle filmic contrast, vibrance, cool shadows and warm highlights.
    pub grade: f32,
    /// Sharpening of the perspective camera's picture (0 = none).
    pub sharpen: f32,
    /// Depth of field (photo mode), or none.
    pub dof: Option<DepthOfField>,
    /// Photo mode's look (a colour grade, grain, a fringe of colour; or a special sight), or
    /// none: the picture as it is.
    pub look: Option<Look>,
    /// Presentation preferences; kept apart from the player's graphics settings.
    pub reduced_motion: bool,
    pub reduced_flashes: bool,
}

impl Default for PostSettings {
    fn default() -> Self {
        PostSettings { exposure: 1.0, bloom: 0.35, bloom_threshold: 1.4, vignette: 0.18, saturation: 1.05, heat_haze: 1.0, grade: 1.0, sharpen: 0.35, dof: None, look: None, reduced_motion: false, reduced_flashes: false }
    }
}

impl PostSettings {
    pub fn effective_heat_haze(&self) -> f32 {
        if self.reduced_motion || self.reduced_flashes { 0.0 } else { self.heat_haze }
    }
}

#[cfg(test)]
mod accessibility_tests {
    use super::*;

    #[test]
    fn motion_preferences_preserve_the_players_graphics_values() {
        let mut post = PostSettings { heat_haze: 0.7, bloom: 0.6, ..Default::default() };
        assert_eq!(post.effective_heat_haze(), 0.7);
        post.reduced_motion = true;
        assert_eq!(post.effective_heat_haze(), 0.0);
        post.reduced_motion = false;
        post.reduced_flashes = true;
        assert_eq!(post.effective_heat_haze(), 0.0);
        post.reduced_flashes = false;
        assert_eq!(post.effective_heat_haze(), 0.7);
        assert_eq!(post.bloom, 0.6);
    }
}

/// Depth of field: the picture blurs away from what is in focus, as through a real lens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthOfField {
    pub mode: DofMode,
    /// [`DofMode::Lens`]: the distance in focus, from the eye along the view (see
    /// [`crate::Camera::view_distance`]); [`DofMode::TiltShift`]: the sharp band's centre on
    /// the picture (0 top, 1 bottom).
    pub focus: f32,
    /// The largest blur's radius, as a fraction of the picture's height (so a larger photo
    /// blurs alike).
    pub blur: f32,
    /// [`DofMode::TiltShift`]: the sharp band's half height (fraction of the picture's height);
    /// [`DofMode::Lens`]: how far either side of the focus stays sharp, as a share of the
    /// distance (the blur then grows over a longer stretch).
    pub band: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DofMode {
    /// Sharp at one distance, blurring nearer and farther.
    Lens,
    /// A sharp band across the picture, blurring above and below it: the world as a miniature.
    TiltShift,
    /// All of it blurred alike (the battle behind a window, as through frosted glass).
    Uniform,
}

/// Photo mode's look: a grade over the finished picture (in its display values), its grain and a
/// fringe of colour towards the corners, as a lens's; or, in place of the grade, a special sight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Contrast round the middle (1 as it is).
    pub contrast: f32,
    /// Cold (-1) to warm (1).
    pub warmth: f32,
    /// Film grain (0 none to 1 heavy).
    pub grain: f32,
    /// How far the colours fringe towards the corners (0 none to 1 strong).
    pub fringe: f32,
    /// Black and white (0 colour to 1).
    pub mono: f32,
    /// The black level raised (a faded print's: 0 to 0.2).
    pub lift: f32,
    /// The tints of the shadows and of the highlights (rgb, and how much: 0 to 1).
    pub shadows: [f32; 4],
    pub highlights: [f32; 4],
    pub special: Sight,
}

impl Default for Look {
    fn default() -> Self {
        Look { contrast: 1.0, warmth: 0.0, grain: 0.0, fringe: 0.0, mono: 0.0, lift: 0.0, shadows: [0.5, 0.5, 0.5, 0.0], highlights: [0.5, 0.5, 0.5, 0.0], special: Sight::Eye }
    }
}

/// A look in place of the grade: the eye's, a thermal camera's or a night sight's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sight {
    #[default]
    Eye,
    /// Brightness as heat: cold blues and purples to white-hot.
    Thermal,
    /// A green phosphor's picture, scan lines and noise.
    Night,
}

/// The menus' backdrop grade at full strength (see [`Post::backdrop`]): how much darker the
/// battle is, how much of its colour goes, and the vignette added round it.
const BACKDROP_DIM: f32 = 0.14;
const BACKDROP_GREY: f32 = 0.3;
const BACKDROP_CORNERS: f32 = 0.22;

/// Depth of field tiles: each holds the largest blur in its square of pixels (`TILE` in
/// dof.wgsl).
const DOF_TILE: u32 = 16;
/// How many tiles a blur can reach across; the blur's radius is capped to fit.
const DOF_REACH: u32 = 4;
/// The format of the depth of field's distances and tiles.
const DOF_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// Screen pixels the heat haze shifts the image by per unit of heat (at 1080 lines; scaled
/// with the output height) at strength 1.
const HEAT_PIXELS: f32 = 7.0;

/// Per-size resources, recreated on resize.
struct Sized {
    color_msaa: Option<Target>,
    color: Target,
    depth: Target,
    bloom: Vec<Target>,
    bloom_down: Vec<wgpu::BindGroup>,
    bloom_up: Vec<wgpu::BindGroup>,
    composite: wgpu::BindGroup,
    /// Made while the depth of field is on.
    dof: Option<DofTargets>,
    width: u32,
    height: u32,
}

/// The depth of field's targets and bind groups.
struct DofTargets {
    /// Each pixel's distance from the eye.
    distances: Target,
    /// The largest blur radius in each tile.
    tiles: Target,
    /// The blurred picture.
    image: Target,
    distance_group: wgpu::BindGroup,
    tiles_group: wgpu::BindGroup,
    blur_group: wgpu::BindGroup,
    /// Bloom's first pass and the composite, reading the blurred picture.
    bloom_down: wgpu::BindGroup,
    composite: wgpu::BindGroup,
}

/// The depth of field's pipelines.
struct DofPipelines {
    uniform: wgpu::Buffer,
    distance_layout: wgpu::BindGroupLayout,
    tiles_layout: wgpu::BindGroupLayout,
    blur_layout: wgpu::BindGroupLayout,
    distance: wgpu::RenderPipeline,
    tiles: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
}

/// What [`Post::run`] needs to know about the frame.
#[derive(Clone, Copy)]
pub(crate) struct PostFrame {
    /// Animation time (seconds).
    pub time: f32,
    /// Heat haze strength (0 when no hot air was drawn).
    pub heat: f32,
    /// [`crate::Camera::depth_mapping`].
    pub depth: [f32; 3],
    /// The perspective camera's picture (the cinematic grade and sharpening apply).
    pub modern: bool,
    /// The intro room's picture: as through a camera at night (a filmic curve, the lens's
    /// fringes and vignette, grain; a stronger glow).
    pub room: bool,
    /// Space (see [`crate::space`]): its own exposure and glow, a filmic curve, dithered.
    pub space: Option<SpaceLook>,
}

/// Space's picture: its exposure (times the light), its glow's strength and its corners'
/// darkening; the composite then takes it through a filmic curve (a soft toe, a long shoulder that
/// burns the brightest to white) and dithers it, so its dark gradients do not band. `fade` scales
/// the finished picture's values (sRGB) as a video fade does (1: as it is, 0: black).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpaceLook {
    pub exposure: f32,
    pub bloom: f32,
    pub vignette: f32,
    pub fade: f32,
}

/// Space's glow threshold (the sun and the brightest stars glow; lit ground barely).
const SPACE_BLOOM_THRESHOLD: f32 = 1.6;
/// Space's dither: this many 8-bit steps, peak to peak (triangular).
const SPACE_DITHER: f32 = 1.0;

/// The intro room's exposure, its glow's strength (times the setting's) and threshold, and its
/// tone curve, x^toe / (x^toe + shoulder) (the room's glass undoes it to show the movies as
/// they are).
pub(crate) const ROOM_EXPOSURE: f32 = 1.35;
const ROOM_BLOOM: f32 = 0.8;
const ROOM_BLOOM_THRESHOLD: f32 = 2.0;
pub(crate) const ROOM_TOE: f32 = 1.25;
pub(crate) const ROOM_SHOULDER: f32 = 0.35;

pub struct Post {
    pub samples: u32,
    pub settings: PostSettings,
    /// The menus' backdrop grade (0 none, 1 full): the battle behind the title menu and its
    /// pages steps back, darker, its colours quieter and its corners deeper, so they read
    /// over it. (Not one of the [`PostSettings`], which photo mode and the perspective
    /// camera keep and put back.)
    pub backdrop: f32,
    uniform: wgpu::Buffer,
    bloom_uniforms: Vec<wgpu::Buffer>,
    composite_layout: wgpu::BindGroupLayout,
    bloom_layout: wgpu::BindGroupLayout,
    composite_pipeline: wgpu::RenderPipeline,
    down_pipeline: wgpu::RenderPipeline,
    up_pipeline: wgpu::RenderPipeline,
    dof: DofPipelines,
    sampler: wgpu::Sampler,
    sized: Sized,
}

impl Post {
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat, width: u32, height: u32, samples: u32) -> Self {
        use wgpu::{BindingType as B, ShaderStages as S};
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: S::FRAGMENT,
            ty: B::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: S::FRAGMENT,
            ty: B::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: S::FRAGMENT,
            ty: B::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite"),
            entries: &[uniform_entry, tex_entry(1), tex_entry(2), sampler_entry(3)],
        });
        let bloom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom"),
            entries: &[uniform_entry, tex_entry(1), sampler_entry(2)],
        });
        let fullscreen = include_str!("shaders/fullscreen.wgsl");
        let post_src = shader_source();
        let bloom_src = format!("{fullscreen}\n{}", include_str!("shaders/bloom.wgsl"));
        let post_mod = crate::util::shader_standalone(device, "post", &post_src);
        let bloom_mod = crate::util::shader_standalone(device, "bloom", &bloom_src);

        let make = |label: &str, module: &wgpu::ShaderModule, layout: &wgpu::BindGroupLayout, fs: &str, format, blend| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs_fullscreen"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let additive = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        });
        let composite_pipeline = make("composite", &post_mod, &composite_layout, "fs_composite", output_format, None);
        let down_pipeline = make("bloom down", &bloom_mod, &bloom_layout, "fs_bloom_down", HDR_FORMAT, None);
        let up_pipeline = make("bloom up", &bloom_mod, &bloom_layout, "fs_bloom_up", HDR_FORMAT, additive);

        // Depth of field: its passes read textures only by texel.
        let exact = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: S::FRAGMENT,
            ty: B::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let depth_entry = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: S::FRAGMENT,
            ty: B::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: samples > 1,
            },
            count: None,
        };
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries });
        let distance_layout = layout("dof distances", &[uniform_entry, depth_entry]);
        let tiles_layout = layout("dof tiles", &[uniform_entry, exact(2)]);
        let blur_layout = layout("dof blur", &[uniform_entry, exact(2), exact(3), exact(4)]);
        let dof_mod = crate::util::shader_standalone(device, "dof", &dof_source(samples));
        let dof = DofPipelines {
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dof uniform"),
                size: std::mem::size_of::<DofUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            distance: make("dof distances", &dof_mod, &distance_layout, "fs_distance", DOF_FORMAT, None),
            tiles: make("dof tiles", &dof_mod, &tiles_layout, "fs_tiles", DOF_FORMAT, None),
            blur: make("dof blur", &dof_mod, &blur_layout, "fs_dof", HDR_FORMAT, None),
            distance_layout,
            tiles_layout,
            blur_layout,
        };

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post uniform"),
            size: std::mem::size_of::<PostUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bloom_uniforms: Vec<wgpu::Buffer> = (0..BLOOM_LEVELS * 2)
            .map(|_| {
                device.create_buffer_checked(&wgpu::util::BufferInitDescriptor {
                    label: Some("bloom uniform"),
                    contents: bytemuck::bytes_of(&BloomUniform { texel: [0.0; 4] }),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            })
            .collect();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let sized = Self::make_sized(
            device,
            &composite_layout,
            &bloom_layout,
            &uniform,
            &bloom_uniforms,
            &sampler,
            width,
            height,
            samples,
        );
        Post {
            samples,
            settings: PostSettings::default(),
            backdrop: 0.0,
            uniform,
            bloom_uniforms,
            composite_layout,
            bloom_layout,
            composite_pipeline,
            down_pipeline,
            up_pipeline,
            dof,
            sampler,
            sized,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_sized(
        device: &wgpu::Device,
        composite_layout: &wgpu::BindGroupLayout,
        bloom_layout: &wgpu::BindGroupLayout,
        uniform: &wgpu::Buffer,
        bloom_uniforms: &[wgpu::Buffer],
        sampler: &wgpu::Sampler,
        width: u32,
        height: u32,
        samples: u32,
    ) -> Sized {
        use wgpu::TextureUsages as U;
        let color_msaa = (samples > 1)
            .then(|| Target::new(device, "hdr msaa", width, height, HDR_FORMAT, samples, U::RENDER_ATTACHMENT));
        // (Copied from by the water without antialiasing, see `water::WaterScene`.)
        let color = Target::new(device, "hdr", width, height, HDR_FORMAT, 1, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC);
        // (Read by the depth of field.)
        let depth = Target::new(device, "depth", width, height, DEPTH_FORMAT, samples, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING);
        let bloom: Vec<Target> = (0..BLOOM_LEVELS)
            .map(|i| {
                let (w, h) = ((width >> (i + 1)).max(1), (height >> (i + 1)).max(1));
                Target::new(device, "bloom", w, h, HDR_FORMAT, 1, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING)
            })
            .collect();
        let bg = |buf: &wgpu::Buffer, view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom"),
                layout: bloom_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
                ],
            })
        };
        // down[i] reads the previous level (level 0 reads the HDR image) and writes bloom[i].
        let bloom_down = (0..BLOOM_LEVELS)
            .map(|i| bg(&bloom_uniforms[i], if i == 0 { &color.view } else { &bloom[i - 1].view }))
            .collect();
        // up[i] reads bloom[i + 1] and adds into bloom[i].
        let bloom_up = (0..BLOOM_LEVELS - 1).map(|i| bg(&bloom_uniforms[BLOOM_LEVELS + i], &bloom[i + 1].view)).collect();
        let composite = composite_group(device, composite_layout, uniform, &color.view, &bloom[0].view, sampler);
        Sized { color_msaa, color, depth, bloom, bloom_down, bloom_up, composite, dof: None, width, height }
    }

    /// The depth of field's targets for the current size.
    fn make_dof(&self, device: &wgpu::Device) -> DofTargets {
        use wgpu::TextureUsages as U;
        let s = &self.sized;
        let (w, h) = (s.width, s.height);
        let usage = U::RENDER_ATTACHMENT | U::TEXTURE_BINDING;
        let distances = Target::new(device, "dof distances", w, h, DOF_FORMAT, 1, usage);
        let tiles = Target::new(device, "dof tiles", w.div_ceil(DOF_TILE), h.div_ceil(DOF_TILE), DOF_FORMAT, 1, usage);
        let image = Target::new(device, "dof image", w, h, HDR_FORMAT, 1, usage);
        let group = |label, layout, views: &[(u32, &wgpu::TextureView)]| {
            let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: self.dof.uniform.as_entire_binding() }];
            entries.extend(views.iter().map(|&(binding, view)| wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(view) }));
            device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout, entries: &entries })
        };
        let distance_group = group("dof distances", &self.dof.distance_layout, &[(1, &s.depth.view)]);
        let tiles_group = group("dof tiles", &self.dof.tiles_layout, &[(2, &distances.view)]);
        let blur_group = group("dof blur", &self.dof.blur_layout, &[(2, &distances.view), (3, &s.color.view), (4, &tiles.view)]);
        let bloom_down = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom"),
            layout: &self.bloom_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.bloom_uniforms[0].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&image.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let composite = composite_group(device, &self.composite_layout, &self.uniform, &image.view, &s.bloom[0].view, &self.sampler);
        DofTargets { distances, tiles, image, distance_group, tiles_group, blur_group, bloom_down, composite }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == self.sized.width && height == self.sized.height {
            return;
        }
        self.sized = Self::make_sized(
            device,
            &self.composite_layout,
            &self.bloom_layout,
            &self.uniform,
            &self.bloom_uniforms,
            &self.sampler,
            width,
            height,
            self.samples,
        );
    }

    /// The scene colour attachment (MSAA if enabled) and its resolve target.
    pub fn scene_attachment(&self) -> (&wgpu::TextureView, Option<&wgpu::TextureView>) {
        match &self.sized.color_msaa {
            Some(msaa) => (&msaa.view, Some(&self.sized.color.view)),
            None => (&self.sized.color.view, None),
        }
    }

    pub fn depth_view(&self) -> &wgpu::TextureView {
        &self.sized.depth.view
    }

    /// The scene's colour as resolved (as drawn, without antialiasing).
    pub fn scene_texture(&self) -> &wgpu::Texture {
        &self.sized.color.texture
    }

    pub fn size(&self) -> (u32, u32) {
        (self.sized.width, self.sized.height)
    }

    /// Runs the depth of field (when on) and bloom, and composites into `output`; `timestamps`
    /// times the bloom passes together and the composite.
    pub(crate) fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        frame: PostFrame,
        timestamps: (Option<&PassTimestamps>, Option<&PassTimestamps>),
    ) {
        let mut st = self.settings;
        if frame.room {
            st.exposure *= ROOM_EXPOSURE;
            st.bloom *= ROOM_BLOOM;
            st.bloom_threshold = ROOM_BLOOM_THRESHOLD;
        }
        if let Some(look) = frame.space {
            st.exposure = look.exposure;
            st.bloom = look.bloom;
            st.bloom_threshold = SPACE_BLOOM_THRESHOLD;
            st.vignette = look.vignette;
            st.saturation = 1.0;
        }
        let backdrop = self.backdrop.clamp(0.0, 1.0);
        if st.reduced_flashes { st.bloom = 0.0; }
        st.exposure *= 1.0 - BACKDROP_DIM * backdrop;
        st.saturation *= 1.0 - BACKDROP_GREY * backdrop;
        st.vignette += BACKDROP_CORNERS * backdrop;
        // The depth of field's targets live while it is on.
        let dof = st.dof.filter(|d| d.blur > 0.0);
        if dof.is_none() {
            self.sized.dof = None;
        } else if self.sized.dof.is_none() {
            self.sized.dof = Some(self.make_dof(device));
        }
        let s = &self.sized;
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&PostUniform {
                grade: [st.exposure, st.bloom, st.vignette, st.saturation],
                info: [1.0 / s.width as f32, 1.0 / s.height as f32, frame.time, if frame.modern { st.grade.clamp(0.0, 2.0) } else { 0.0 }],
                heat: [frame.heat.max(0.0) * HEAT_PIXELS * s.height as f32 / 1080.0, if frame.modern { st.sharpen.clamp(0.0, 2.0) } else { 0.0 }, if frame.room { ROOM_TOE } else { 0.0 }, ROOM_SHOULDER],
                look: [if frame.space.is_some() { 1.0 } else { 0.0 }, SPACE_DITHER, frame.space.map_or(1.0, |l| l.fade.clamp(0.0, 1.0)), 0.0],
                photo: st.look.map_or([1.0, 0.0, 0.0, 0.0], |l| [l.contrast.clamp(0.3, 2.5), l.warmth.clamp(-1.0, 1.0), l.grain.clamp(0.0, 1.0), l.fringe.clamp(0.0, 1.0)]),
                photo_tone: st.look.map_or([0.0; 4], |l| {
                    let special = match l.special {
                        Sight::Eye => 0.0,
                        Sight::Thermal => 1.0,
                        Sight::Night => 2.0,
                    };
                    [l.mono.clamp(0.0, 1.0), l.lift.clamp(0.0, 0.3), special, 1.0]
                }),
                shadows: st.look.map_or([0.0; 4], |l| l.shadows),
                highlights: st.look.map_or([0.0; 4], |l| l.highlights),
            }),
        );
        // Bloom: its first pass writes the start timestamp, its last the end one. The composite
        // (which may wait for the swapchain image) is timed on its own.
        let (bloom_ts, composite_ts) = timestamps;
        let first = std::cell::Cell::new(true);
        let fullscreen_pass = |encoder: &mut wgpu::CommandEncoder,
                               view: &wgpu::TextureView,
                               pipeline: &wgpu::RenderPipeline,
                               bind: &wgpu::BindGroup,
                               load: wgpu::LoadOp<wgpu::Color>,
                               stage: Stage| {
            let writes = match stage {
                Stage::Untimed => None,
                Stage::Composite => composite_ts.map(|t| t.writes()),
                Stage::Bloom { last } => bloom_ts.map(|t| t.writes()).and_then(|mut w| {
                    if !first.replace(false) {
                        w.beginning_of_pass_write_index = None;
                    }
                    if !last {
                        w.end_of_pass_write_index = None;
                    }
                    (w.beginning_of_pass_write_index.is_some() || w.end_of_pass_write_index.is_some()).then_some(w)
                }),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        };

        // The depth of field replaces the picture bloom and the composite read.
        let (bloom_first, composite) = match (dof, &s.dof) {
            (Some(d), Some(t)) => {
                let blur = (d.blur * s.height as f32).min((DOF_TILE * DOF_REACH) as f32);
                let mode = match d.mode {
                    DofMode::Lens => 1.0,
                    DofMode::TiltShift => 2.0,
                    DofMode::Uniform => 3.0,
                };
                let [a, b, linear] = frame.depth;
                queue.write_buffer(
                    &self.dof.uniform,
                    0,
                    bytemuck::bytes_of(&DofUniform {
                        params: [mode, d.focus, blur, d.band],
                        clip: [a, b, linear, (blur / DOF_TILE as f32).ceil()],
                    }),
                );
                let clear = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
                fullscreen_pass(encoder, &t.distances.view, &self.dof.distance, &t.distance_group, clear, Stage::Untimed);
                fullscreen_pass(encoder, &t.tiles.view, &self.dof.tiles, &t.tiles_group, clear, Stage::Untimed);
                fullscreen_pass(encoder, &t.image.view, &self.dof.blur, &t.blur_group, clear, Stage::Untimed);
                (&t.bloom_down, &t.composite)
            }
            _ => (&s.bloom_down[0], &s.composite),
        };
        if st.bloom > 0.0 {
            for i in 0..BLOOM_LEVELS {
                let (sw, sh) = if i == 0 { (s.width, s.height) } else { (s.width >> i, s.height >> i) };
                let threshold = if i == 0 { st.bloom_threshold } else { 0.0 };
                queue.write_buffer(
                    &self.bloom_uniforms[i],
                    0,
                    bytemuck::bytes_of(&BloomUniform {
                        texel: [1.0 / sw.max(1) as f32, 1.0 / sh.max(1) as f32, threshold, 0.0],
                    }),
                );
                let group = if i == 0 { bloom_first } else { &s.bloom_down[i] };
                fullscreen_pass(encoder, &s.bloom[i].view, &self.down_pipeline, group, wgpu::LoadOp::Clear(wgpu::Color::BLACK), Stage::Bloom { last: false });
            }
            for i in (0..BLOOM_LEVELS - 1).rev() {
                let (sw, sh) = ((s.width >> (i + 2)).max(1), (s.height >> (i + 2)).max(1));
                queue.write_buffer(
                    &self.bloom_uniforms[BLOOM_LEVELS + i],
                    0,
                    bytemuck::bytes_of(&BloomUniform { texel: [1.0 / sw as f32, 1.0 / sh as f32, 0.0, 0.0] }),
                );
                fullscreen_pass(encoder, &s.bloom[i].view, &self.up_pipeline, &s.bloom_up[i], wgpu::LoadOp::Load, Stage::Bloom { last: i == 0 });
            }
        } else {
            fullscreen_pass(encoder, &s.bloom[0].view, &self.down_pipeline, bloom_first, wgpu::LoadOp::Clear(wgpu::Color::BLACK), Stage::Bloom { last: true });
        }
        fullscreen_pass(encoder, output, &self.composite_pipeline, composite, wgpu::LoadOp::Clear(wgpu::Color::BLACK), Stage::Composite);
    }
}

fn composite_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    picture: &wgpu::TextureView,
    bloom: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("composite"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(picture) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(bloom) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

/// Which timestamps a post pass writes.
#[derive(Clone, Copy)]
enum Stage {
    Untimed,
    Bloom { last: bool },
    Composite,
}

/// The depth of field shader, reading a depth buffer of `samples` samples.
pub(crate) fn dof_source(samples: u32) -> String {
    let source = format!("{}\n{}", include_str!("shaders/fullscreen.wgsl"), include_str!("shaders/dof.wgsl"));
    if samples > 1 { source } else { source.replace("texture_depth_multisampled_2d", "texture_depth_2d") }
}

/// The composite shader: full-screen triangle, noise (heat haze), the composite itself.
pub(crate) fn shader_source() -> String {
    format!(
        "{}\n{}\n{}",
        include_str!("shaders/fullscreen.wgsl"),
        include_str!("shaders/noise.wgsl"),
        include_str!("shaders/post.wgsl")
    )
}
