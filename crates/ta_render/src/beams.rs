//! Additive beam ribbons for lasers, lightning segments and tracers.

use bytemuck::{Pod, Zeroable};

use crate::util::depth_state;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct BeamInstance {
    /// Start (xyz) and width (w).
    pub a: [f32; 4],
    /// End (xyz) and core fraction of the width (w, 0..1).
    pub b: [f32; 4],
    /// HDR colour (rgb) and intensity/alpha (a).
    pub color: [f32; 4],
    /// How its ends are drawn: x = world units over which the head (the start) rounds off, y =
    /// the share of its length over which the tail (the end) narrows and fades out; both 0 for
    /// square ends (a lightning bolt's segments, which join). z = how much of what lies behind
    /// its glow it hides (0: plain light, which on bright ground clips to white whatever its
    /// colour).
    pub shape: [f32; 4],
}

impl BeamInstance {
    pub fn new(a: [f32; 3], b: [f32; 3], width: f32, color: [f32; 3], alpha: f32) -> Self {
        BeamInstance { a: [a[0], a[1], a[2], width], b: [b[0], b[1], b[2], 0.25], color: [color[0], color[1], color[2], alpha], shape: [0.0; 4] }
    }

    /// A bolt: its head rounded off over `head` world units, its tail narrowing and fading out
    /// over the share `tail` of its length.
    pub fn tapered(mut self, head: f32, tail: f32) -> Self {
        self.shape[0] = head;
        self.shape[1] = tail;
        self
    }

    /// Hides `cover` (0..1) of what lies behind its glow, so its colour shows on bright ground.
    pub fn cover(mut self, cover: f32) -> Self {
        self.shape[2] = cover;
        self
    }

    /// The share of its width the white-hot core takes (a quarter unless set).
    pub fn core(mut self, core: f32) -> Self {
        self.b[3] = core;
        self
    }
}

pub struct BeamRenderer {
    pipeline: wgpu::RenderPipeline,
    /// Beams mirrored in the water (reflection.rs).
    reflection: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    count: u32,
}

impl BeamRenderer {
    pub fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout, hdr: wgpu::TextureFormat, samples: u32) -> Self {
        let shader = crate::util::shader(device, "beams", include_str!("shaders/beam.wgsl"));
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("beams"),
            bind_group_layouts: &[Some(frame_layout)],
            immediate_size: 0,
        });
        // The scene pass's pipeline, or with `mirrored` the reflection pass's.
        let make = |label: &str, mirrored: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_beam"),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<BeamInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4],
                    })],
                    compilation_options: Default::default(),
                },
                primitive: if mirrored { crate::reflection::primitive() } else { wgpu::PrimitiveState { cull_mode: None, ..Default::default() } },
                depth_stencil: Some(wgpu::DepthStencilState { depth_write_enabled: Some(false), ..depth_state(false) }),
                multisample: wgpu::MultisampleState { count: if mirrored { 1 } else { samples }, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if mirrored { "fs_beam_reflection" } else { "fs_beam" }),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: hdr,
                        // (Light added, less what its glow hides of what lies behind: see
                        // `BeamInstance::shape`.)
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
            })
        };
        let buffer = Self::make_buffer(device, 1024);
        BeamRenderer { pipeline: make("beams", false), reflection: make("beams reflection", true), buffer, count: 0 }
    }

    fn make_buffer(device: &wgpu::Device, n: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("beams"),
            size: (n * std::mem::size_of::<BeamInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, beams: &[BeamInstance]) {
        self.count = beams.len() as u32;
        if beams.is_empty() {
            return;
        }
        let bytes = std::mem::size_of_val(beams) as u64;
        if bytes > self.buffer.size() {
            self.buffer = Self::make_buffer(device, beams.len().next_power_of_two());
        }
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(beams));
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.draw_with(pass, &self.pipeline);
    }

    /// Draws the beams mirrored in the water (inside the reflection pass).
    pub(crate) fn draw_reflection<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        self.draw_with(pass, &self.reflection);
    }

    fn draw_with<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, pipeline: &'a wgpu::RenderPipeline) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}
