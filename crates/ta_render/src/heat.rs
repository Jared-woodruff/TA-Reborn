//! Heat haze: shimmering air above explosions, fires, burning wrecks and lava seas
//! (perspective camera only; [`PostSettings::heat_haze`](crate::post::PostSettings) sets its
//! strength, 0 turns it off).
//!
//! Hot air is marked in the alpha channel of the HDR scene target, which nothing else uses:
//! opaque geometry leaves 1 there and the sky pass writes a large negative value. At the end of
//! the scene pass (inside the MSAA pass, depth tested, so units in front of a fire are not
//! distorted) each [`HeatSource`] adds a soft rising column of heat to the alpha, and on lava
//! worlds the lava surface adds a thin layer of it. The final composite (`post.wgsl`) then
//! offsets its lookup of the scene by drifting noise scaled by the heat (`alpha - 1`), skipping
//! any sample that lands on the sky; the UI is drawn afterwards and is never distorted. The
//! amount fades with distance from the camera, so far fires shimmer less on screen.
//!
//! The game adds sources every frame, e.g. for an explosion `t` seconds into its `life`
//! seconds and a burning wreck:
//!
//! ```ignore
//! renderer.add_heat_source(blast_pos, blast_radius * 1.2, (1.0 - t / life).powi(2));
//! renderer.add_heat_source(wreck_pos, if big { 45.0 } else { 30.0 }, 0.8 * fire_fade);
//! ```

use bytemuck::{Pod, Zeroable};

use crate::util::depth_state;

/// A source of hot air for the heat haze.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeatSource {
    /// Ground point (or centre of a fireball) the hot air rises from (world units).
    pub position: [f32; 3],
    /// Radius of the column of hot air; it rises about twice as high.
    pub radius: f32,
    /// 0..1 (more is allowed): 1 = a big explosion's fireball, 0.3 = a smouldering wreck.
    pub strength: f32,
}

/// At most this many heat sources are drawn per frame (the strongest win).
pub const MAX_HEAT_SOURCES: usize = 512;

/// GPU instance (mirrors `heat.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
struct HeatInstance {
    /// xyz = position, w = radius.
    pos_radius: [f32; 4],
    /// x = strength, yzw = unused.
    params: [f32; 4],
}

/// Blend state that adds a fragment's alpha to the target's and leaves its colour alone: how
/// heat is accumulated in the scene target.
pub(crate) const HEAT_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
    alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
};

/// The heat source pass (frame bind group only).
pub(crate) struct HeatRenderer {
    pipeline: wgpu::RenderPipeline,
    instances: wgpu::Buffer,
    count: u32,
}

impl HeatRenderer {
    pub fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout, hdr_format: wgpu::TextureFormat, samples: u32) -> Self {
        let shader = crate::util::shader(device, "heat", include_str!("shaders/heat.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("heat"),
            bind_group_layouts: &[Some(frame_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("heat"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_heat"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<HeatInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { depth_write_enabled: Some(false), ..depth_state(false) }),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_heat"),
                targets: &[Some(wgpu::ColorTargetState { format: hdr_format, blend: Some(HEAT_BLEND), write_mask: wgpu::ColorWrites::ALPHA })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let instances = Self::make_instances(device, 64);
        HeatRenderer { pipeline, instances, count: 0 }
    }

    fn make_instances(device: &wgpu::Device, n: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("heat sources"),
            size: (n * std::mem::size_of::<HeatInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Uploads this frame's sources (`count` = 0 draws nothing).
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, sources: &[HeatSource]) {
        let mut list: Vec<HeatInstance> = sources
            .iter()
            .filter(|s| s.position.iter().all(|v| v.is_finite()) && s.radius.is_finite() && s.radius > 0.0 && s.strength.is_finite() && s.strength > 0.0)
            .map(|s| HeatInstance { pos_radius: [s.position[0], s.position[1], s.position[2], s.radius], params: [s.strength, 0.0, 0.0, 0.0] })
            .collect();
        if list.len() > MAX_HEAT_SOURCES {
            let weight = |h: &HeatInstance| h.params[0] * h.pos_radius[3];
            list.select_nth_unstable_by(MAX_HEAT_SOURCES - 1, |a, b| weight(b).total_cmp(&weight(a)));
            list.truncate(MAX_HEAT_SOURCES);
        }
        self.count = list.len() as u32;
        if list.is_empty() {
            return;
        }
        let bytes = std::mem::size_of_val(list.as_slice()) as u64;
        if bytes > self.instances.size() {
            self.instances = Self::make_instances(device, list.len().next_power_of_two());
        }
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&list));
    }

    /// Draws the sources (inside the scene pass, frame bind group set).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}
