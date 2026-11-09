//! Shadows of the sky's clouds on the ground, water, units and effects (perspective camera
//! with the sky on; see [`Lighting::cloud_shadows`](crate::Lighting::cloud_shadows)).
//!
//! The clouds' shape lives in `shaders/clouds.wgsl`, shared with the sky, so the shadows are
//! those of the clouds overhead and drift with them. Each frame a small pass evaluates the
//! cloud layer's coverage along the sun for the ground around the view into a 512² map with a
//! compressed mapping (texel `t` in -1..1 holds the point `centre + K t / (1 - |t|)` of the plane
//! y = 0): about 20 units per texel under the camera, reaching the horizon at the map's edges,
//! so a single texture covers everything visible, however low the camera. Shaders look up a
//! point at height `y` where the sun ray through it meets that plane (`cloud_light` in
//! `common.wgsl`), so hills and units get the shadow of the right cloud.
//!
//! The layer casting the shadows sits [`SHADOW_LAYER`] above the ground: the sky draws its
//! clouds 5000 units above the eye, which is 500 to 2000 units above the ground in play.

use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};

use crate::util::Target;

/// Texels per side of the cloud shadow map.
pub(crate) const TEXELS: u32 = 512;
/// Scale K of the map's compressed mapping (world units).
pub(crate) const SCALE: f32 = 2500.0;
/// Height of the shadow-casting cloud layer above y = 0 (world units).
pub(crate) const SHADOW_LAYER: f32 = 6000.0;
/// Lowest sun elevation (as the y of its unit direction) the shadows are projected with.
pub(crate) const MIN_SUN_Y: f32 = 0.1;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CloudShadowUniform {
    map: [f32; 4],
    layer: [f32; 4],
    cover: [f32; 4],
}

/// Map texel coordinate (0..1 per axis) of a point `d` away from the map's centre (the lookup
/// in `cloud_light`, `common.wgsl`).
#[cfg(test)]
fn map_uv(d: Vec2) -> Vec2 {
    Vec2::splat(0.5) + 0.5 * d / (d.abs() + SCALE)
}

/// The point (relative to the map's centre) that map texel coordinate `uv` holds (as
/// `cloud_shadow.wgsl` computes it).
#[cfg(test)]
fn map_point(uv: Vec2) -> Vec2 {
    let t = uv * 2.0 - 1.0;
    SCALE * t / (Vec2::ONE - t.abs()).max(Vec2::splat(1e-4))
}

/// The map's centre for a view focused on `focus`: snapped to the size of the texels there, so
/// the pattern does not crawl as the camera pans.
pub(crate) fn snapped_centre(focus: Vec2) -> Vec2 {
    let texel = SCALE * 2.0 / TEXELS as f32;
    (focus / texel).round() * texel
}

/// The cloud shadow map and the pass that draws it.
pub(crate) struct CloudShadows {
    pub target: Target,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Centre of the map drawn this frame (world xz).
    pub centre: Vec2,
}

impl CloudShadows {
    pub fn new(device: &wgpu::Device) -> Self {
        let target = Target::new(
            device,
            "cloud shadows",
            TEXELS,
            TEXELS,
            wgpu::TextureFormat::R8Unorm,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cloud shadows"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cloud shadows"),
            size: std::mem::size_of::<CloudShadowUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cloud shadows"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        let shader = crate::util::shader_standalone(device, "cloud shadows", &shader_source());
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cloud shadows"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cloud shadows"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_fullscreen"), buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_cloud_shadow"),
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        CloudShadows { target, pipeline, uniform, bind_group, centre: Vec2::ZERO }
    }

    /// Sets up this frame's map around `focus` (world xz) for the sun direction `sun` (unit),
    /// the wind's drift so far (`cloud speed * time`), the cloud cover (0..1) and the map's offset
    /// of the cloud field (as the sky's, `Frame::ground_beyond.zw`).
    pub fn prepare(&mut self, queue: &wgpu::Queue, focus: Vec2, sun: Vec3, drift: f32, cover: f32, field: Vec2) {
        self.centre = snapped_centre(focus);
        let offset = Vec2::new(sun.x, sun.z) / sun.y.max(MIN_SUN_Y) * SHADOW_LAYER;
        let u = CloudShadowUniform {
            map: [self.centre.x, self.centre.y, SCALE, TEXELS as f32],
            layer: [offset.x, offset.y, drift, 0.0],
            cover: [cover.clamp(0.0, 1.0), field.x, field.y, 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
    }

    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cloud shadows"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.target.view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The cloud shadow pass: full-screen triangle, noise, the cloud layer, the pass itself.
pub(crate) fn shader_source() -> String {
    format!(
        "{}\n{}\n{}\n{}",
        include_str!("shaders/fullscreen.wgsl"),
        include_str!("shaders/noise.wgsl"),
        include_str!("shaders/clouds.wgsl"),
        include_str!("shaders/cloud_shadow.wgsl")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compressed_mapping_round_trips_and_reaches_the_horizon() {
        for d in [Vec2::ZERO, Vec2::new(100.0, -40.0), Vec2::new(-3000.0, 8000.0), Vec2::new(45000.0, -60000.0)] {
            let back = map_point(map_uv(d));
            assert!((back - d).length() < d.length() * 1e-4 + 1e-2, "{d} -> {back}");
        }
        // Fine under the camera, coarse far away, and the edge texels are beyond the horizon.
        let texel = |d: f32| map_point(map_uv(Vec2::new(d, 0.0)) + Vec2::new(1.0 / TEXELS as f32, 0.0)).x - d;
        assert!(texel(0.0) < 25.0, "{}", texel(0.0));
        assert!(texel(8000.0) > texel(1000.0));
        let last = map_point(Vec2::splat(1.0 - 0.5 / TEXELS as f32));
        assert!(last.x > 60000.0, "{last}");
    }

    #[test]
    fn the_centre_moves_in_whole_texels() {
        let texel = SCALE * 2.0 / TEXELS as f32;
        let a = snapped_centre(Vec2::new(1234.5, -77.0));
        assert_eq!((a / texel).fract(), Vec2::ZERO);
        assert!((a - Vec2::new(1234.5, -77.0)).abs().max_element() <= texel * 0.5 + 1e-3);
    }
}
