//! Shields (TA Reborn's: a mod's units' bubbles and personal shields, see `shaders/shield.wgsl`):
//! each a dome round its unit, an ellipsoid of its radius across and its height, drawn over the
//! finished scene (after the water) in its owner's colour, faint, its rim brighter as glass is,
//! rippling out from where it is hit and flashing as it collapses and comes back; a personal shield
//! a thin shell round its unit. Drawn with the depth test and without writing depth: what stands
//! in front hides it, the ground its lower half. Nothing is drawn in a battle without shields.

use bytemuck::{Pod, Zeroable};

use crate::util::depth_state;

/// The hits a shield shows ripples of at once (its latest).
pub const HITS: usize = 4;

/// A shield: where, how large, its look and its latest hits.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default)]
pub struct ShieldInstance {
    /// xyz: its middle (world), w: its radius across.
    pub centre: [f32; 4],
    /// x: its height (up and down from its middle), y: how bright it flashes now (0..1: collapsing,
    /// coming up), z: its hit points' share (0..1), w: 1 for a personal shield's shell, 0 a bubble.
    pub shape: [f32; 4],
    /// Linear colour (rgb) and opacity (a).
    pub color: [f32; 4],
    /// Its latest hits: xyz the way from its middle to where it was hit (in the dome's round
    /// shape: its height stretched to its radius), w seconds since (negative: none).
    pub hits: [[f32; 4]; HITS],
}

/// Sides round and bands down a dome (as in the shader).
const AROUND: u32 = 32;
const DOWN: u32 = 16;

pub struct ShieldRenderer {
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    count: u32,
}

impl ShieldRenderer {
    pub fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout, hdr: wgpu::TextureFormat, samples: u32) -> Self {
        let shader = crate::util::shader(device, "shields", include_str!("shaders/shield.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shields"), bind_group_layouts: &[Some(frame_layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shields"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shield"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ShieldInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            // (Both its sides: the far one seen through the near.)
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(depth_state(false)),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_shield"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr,
                    // Over the picture (premultiplied); the target's alpha (the heat's) kept.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
                        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        ShieldRenderer { pipeline, buffer: Self::make_buffer(device, 16), count: 0 }
    }

    fn make_buffer(device: &wgpu::Device, n: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor { label: Some("shields"), size: (n * std::mem::size_of::<ShieldInstance>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
    }

    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, shields: &[ShieldInstance]) {
        self.count = shields.len() as u32;
        if shields.is_empty() {
            return;
        }
        let bytes = std::mem::size_of_val(shields) as u64;
        if bytes > self.buffer.size() {
            self.buffer = Self::make_buffer(device, shields.len().next_power_of_two());
        }
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(shields));
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.draw(0..AROUND * DOWN * 6, 0..self.count);
    }
}
