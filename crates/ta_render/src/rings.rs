//! Rings on the ground round selected units (see `shaders/ring.wgsl`): drawn with the opaque
//! scene, once the units are, so that what stands in front of a ring (its own unit, its
//! neighbours, a hill) hides it, as nothing painted over the finished picture can; their depth
//! kept, so that the water drawn over the scene leaves a ship's ring on its surface.

use bytemuck::{Pod, Zeroable};

use crate::util::depth_state;

/// A ring: its middle and radius, the lowest it lies (a ship's ring on the water's surface), its
/// colour and its lines' widths.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct RingInstance {
    /// The middle (x, z; y unused) and the radius (w), world units.
    pub centre: [f32; 4],
    /// Linear colour (rgb) and opacity (a).
    pub color: [f32; 4],
    /// x: the lowest the ring lies (world y), y: the coloured line's width, z: its dark edge's
    /// (pixels), w: unused.
    pub shape: [f32; 4],
}

impl RingInstance {
    pub fn new(centre: [f32; 2], radius: f32, lowest: f32, color: [f32; 3], opacity: f32, width: f32, edge: f32) -> Self {
        RingInstance { centre: [centre[0], 0.0, centre[1], radius], color: [color[0], color[1], color[2], opacity], shape: [lowest, width, edge, 0.0] }
    }
}

/// Its sides (as in the shader).
const SEGMENTS: u32 = 96;

pub struct RingRenderer {
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    count: u32,
}

impl RingRenderer {
    pub fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout, hdr: wgpu::TextureFormat, samples: u32) -> Self {
        let shader = crate::util::shader(device, "rings", include_str!("shaders/ring.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("rings"), bind_group_layouts: &[Some(frame_layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rings"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_ring"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<RingInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(depth_state(true)),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_ring"),
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
        RingRenderer { pipeline, buffer: Self::make_buffer(device, 64), count: 0 }
    }

    fn make_buffer(device: &wgpu::Device, n: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor { label: Some("rings"), size: (n * std::mem::size_of::<RingInstance>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
    }

    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, rings: &[RingInstance]) {
        self.count = rings.len() as u32;
        if rings.is_empty() {
            return;
        }
        let bytes = std::mem::size_of_val(rings) as u64;
        if bytes > self.buffer.size() {
            self.buffer = Self::make_buffer(device, rings.len().next_power_of_two());
        }
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(rings));
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.draw(0..SEGMENTS * 6, 0..self.count);
    }
}
