//! Strategic icons (the game's strategic zoom): a team-coloured shape for each unit, the same
//! number of pixels across however far the camera is, drawn over the finished image (after
//! the tone map, before the interface) in one instanced draw. The shapes are signed distance
//! functions evaluated per pixel (`shaders/icon.wgsl`): crisp at any size, with a dark
//! outline so they read on any ground.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

/// The icons' shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum IconShape {
    Circle = 0,
    Square = 1,
    /// Pointing up the screen.
    Triangle = 2,
    Diamond = 3,
    /// Five points.
    Star = 4,
    Hexagon = 5,
    /// A circle's outline only (a selection mark).
    Ring = 6,
    /// A square's outline with a dot in it.
    HollowSquare = 7,
    /// A circle's outline with a dot in it.
    HollowCircle = 8,
}

/// One icon.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub struct IconInstance {
    /// World position it centres on (xyz) and its size in pixels (w).
    pub pos_size: [f32; 4],
    /// Colour (linear RGB) and opacity.
    pub color: [f32; 4],
    /// [`IconShape`]; the rest pads to 16 bytes.
    pub shape: [u32; 4],
}

impl IconInstance {
    pub fn new(pos: [f32; 3], size: f32, shape: IconShape, color: [f32; 3], opacity: f32) -> Self {
        IconInstance { pos_size: [pos[0], pos[1], pos[2], size], color: [color[0], color[1], color[2], opacity], shape: [shape as u32, 0, 0, 0] }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct IconUniform {
    view_proj: [[f32; 4]; 4],
    /// xy = the target's size in pixels.
    target: [f32; 4],
}

pub struct IconRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    count: u32,
}

impl IconRenderer {
    /// `format`: the final image's (the swapchain view's) format.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = crate::util::shader_standalone(device, "icons", include_str!("shaders/icon.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("icons"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("icons"),
            size: std::mem::size_of::<IconUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("icons"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("icons"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("icons"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_icon"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<IconInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Uint32x4],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_icon"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let buffer = Self::make_buffer(device, 1024);
        IconRenderer { pipeline, uniform, bind_group, buffer, count: 0 }
    }

    fn make_buffer(device: &wgpu::Device, n: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("icons"),
            size: (n * std::mem::size_of::<IconInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Uploads this frame's icons (seen through `view_proj`, on a target `width` × `height`
    /// pixels).
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, icons: &[IconInstance], view_proj: Mat4, width: u32, height: u32) {
        self.count = icons.len() as u32;
        if icons.is_empty() {
            return;
        }
        let u = IconUniform { view_proj: view_proj.to_cols_array_2d(), target: [width as f32, height as f32, 0.0, 0.0] };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        let bytes = std::mem::size_of_val(icons) as u64;
        if bytes > self.buffer.size() {
            self.buffer = Self::make_buffer(device, icons.len().next_power_of_two());
        }
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(icons));
    }

    /// Draws the icons over `target` (the final image).
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        if self.count == 0 {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("icons"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.draw(0..4, 0..self.count);
    }
}
