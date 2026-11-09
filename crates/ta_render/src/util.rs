//! Small shared helpers.

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Creates a shader module with the shared declarations prepended.
pub fn shader(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(with_common(source).into()),
    })
}

/// Whether the renderer's shaders read the ray-traced shadows and ambient occlusion (see
/// raytrace.rs): set once, before the renderer makes its pipelines, when the device can trace.
static RAY_TRACED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn set_ray_traced(on: bool) {
    RAY_TRACED.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn ray_traced() -> bool {
    RAY_TRACED.load(std::sync::atomic::Ordering::Relaxed)
}

/// `source` with the shared frame declarations (`common.wgsl`) prepended.
pub fn with_common(source: &str) -> String {
    with_common_for(source, ray_traced())
}

/// `source` with the shared frame declarations prepended, reading the ray-traced results or not
/// (`rt_visibility`, see raytrace.rs).
pub fn with_common_for(source: &str, ray_traced: bool) -> String {
    let rt = if ray_traced { include_str!("shaders/rt_on.wgsl") } else { include_str!("shaders/rt_off.wgsl") };
    format!("{}\n{}\n{}", include_str!("shaders/common.wgsl"), rt, source)
}

/// Creates a shader module without the shared frame declarations.
pub fn shader_standalone(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

pub fn depth_state(write: bool) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(wgpu::CompareFunction::LessEqual),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

pub fn write_texture_level(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    level: u32,
    width: u32,
    height: u32,
    data: &[u8],
    bytes_per_pixel: u32,
) {
    write_texture_layer(queue, texture, level, 0, width, height, data, bytes_per_pixel);
}

#[allow(clippy::too_many_arguments)]
pub fn write_texture_layer(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    level: u32,
    layer: u32,
    width: u32,
    height: u32,
    data: &[u8],
    bytes_per_pixel: u32,
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * bytes_per_pixel), rows_per_image: Some(height) },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
}

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

pub fn srgb_to_linear_lut() -> [f32; 256] {
    std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0))
}

pub fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// A render target texture plus its default view.
pub struct Target {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl Target {
    pub fn new(
        device: &wgpu::Device,
        label: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        samples: u32,
        usage: wgpu::TextureUsages,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Target { texture, view }
    }
}
