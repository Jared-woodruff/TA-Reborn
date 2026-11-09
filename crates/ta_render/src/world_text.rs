//! Text and signs in the world (see `shaders/world_text.wgsl`): the letters of a name laid on
//! the ground (lit and shadowed as the ground is, draped over its relief, under the scorch marks
//! of the blasts that fall on them and the units that walk across them), burning, projected as
//! light, built by a nanolathe, or printed on a banner waving in the air.
//!
//! The renderer holds two atlases and draws the glyphs the game lays out each frame:
//!
//! - the glyph atlas: signed distance fields (one byte a texel, 0.5 on the outline, [`SPREAD`]
//!   texels of distance either side), one cell a glyph, cut by the game from a font's outlines
//!   (`apps/ta_game/src/world_text.rs`); a field is drawn sharp however near the camera comes,
//!   and gives the edge, the glow and the dark round it their widths in world units;
//! - the picture atlas: one RGBA picture (the emblem) for a sign of its own.
//!
//! A glyph is a quad (`GlyphInstance`): its middle and its edge vectors in the world, so it lies
//! on the ground or stands in the air as the game puts it. Draped over the ground, it follows
//! the heights under it (a lift over them) and stays on the sea over water.
//!
//! The glyphs are drawn twice. First with the opaque scene, right after the terrain: what stands
//! in front of a letter hides it. Then once more, lighter, after the opaque scene
//! ([`WorldTextRenderer::draw_over`]): only where something was drawn in front of a letter, and
//! only as far as that something stands on the letter's ground (the shader reads where it is from
//! the scene's depth): so a name stays readable through the units walking over it, still the
//! ground's letter, lit as the ground under it is, while a ridge or a building far in front of it
//! hides it as before. (`TA_TEXT_OVER=0` leaves the second pass out, to compare.)

use std::sync::atomic::{AtomicU64, Ordering};

use bytemuck::{Pod, Zeroable};

use crate::util::depth_state;

/// Texels the distance field spreads either side of an outline (the shader's `SPREAD`).
pub const SPREAD: f32 = 8.0;
/// Texels an atlas is across (the glyph atlas) and the picture atlas is (room for the emblem's 512
/// with its padding, and more).
pub const GLYPH_ATLAS: u32 = 2048;
pub const PICTURE_ATLAS: u32 = 1024;
/// A texel's padding round each cell, so filtering never takes from its neighbour.
const PAD: u32 = 1;

/// The styles of a glyph (`GlyphInstance::centre[3]`; the shader's `STYLE_*`).
pub mod style {
    /// Paint on the ground, lit as it is; glowing as it heats; dark round its edge.
    pub const PAINT: f32 = 0.0;
    /// Projected light: lines and a glow, added to the ground.
    pub const LIGHT: f32 = 1.0;
    /// A nanolathe's work: solid as far as it is built, its frame above.
    pub const LATTICE: f32 = 2.0;
    /// A picture of the picture atlas, lit as the ground is.
    pub const PICTURE: f32 = 3.0;
    /// Cloth, printed, lit from both sides.
    pub const CLOTH: f32 = 4.0;
}

/// One glyph to draw: a quad of the world and how it looks.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default, PartialEq)]
pub struct GlyphInstance {
    /// xyz: the middle of the quad (when it drapes over the ground, y is a lift over it); w: the
    /// style ([`style`]).
    pub centre: [f32; 4],
    /// xyz: from the middle to the middle of the right edge; w: 1 to drape over the ground, 0 to
    /// stand free.
    pub right: [f32; 4],
    /// xyz: from the middle to the middle of the top edge; w: world units to a texel of its cell.
    pub up: [f32; 4],
    /// Its cell: u0, v0, u1, v1 (of the glyph atlas, or the picture atlas for a picture).
    pub uv: [f32; 4],
    /// Linear colour and opacity.
    pub color: [f32; 4],
    /// x: how hot it glows (0 cold, 1 white hot); y: how far it is revealed (0..1, a front
    /// crossing it from the left; 1 all of it); z: its age (seconds, for what moves in it); w: a
    /// seed of its own.
    pub state: [f32; 4],
    /// rgb: the light it gives of its own (linear, HDR); a: the least light its paint shows in
    /// (0..1).
    pub glow: [f32; 4],
    /// Cloth: x how far it waves, y the wave number along it, z the wave's speed, w its phase.
    pub wave: [f32; 4],
}

/// Where a cell sits in its atlas, as the uv rectangle of its texels (u0, v0, u1, v1).
pub type Cell = [f32; 4];

/// A shelf packer over a texture's texels held on the CPU, uploaded when it changes.
struct Atlas {
    side: u32,
    bytes_per_texel: u32,
    pixels: Vec<u8>,
    cursor: (u32, u32),
    shelf: u32,
    dirty: bool,
    full: bool,
}

impl Atlas {
    fn new(side: u32, bytes_per_texel: u32) -> Atlas {
        Atlas { side, bytes_per_texel, pixels: vec![0; (side * side * bytes_per_texel) as usize], cursor: (0, 0), shelf: 0, dirty: true, full: false }
    }

    /// A cell of `w` x `h` texels (the distance field's outside value, or transparent, round it).
    fn add(&mut self, w: u32, h: u32, data: &[u8], fill: u8) -> Option<Cell> {
        let bpt = self.bytes_per_texel;
        assert_eq!(data.len(), (w * h * bpt) as usize, "a cell of {w} x {h} texels");
        let (pw, ph) = (w + PAD * 2, h + PAD * 2);
        if pw > self.side || ph > self.side {
            return None;
        }
        if self.cursor.0 + pw > self.side {
            self.cursor = (0, self.cursor.1 + self.shelf);
            self.shelf = 0;
        }
        if self.cursor.1 + ph > self.side {
            if !self.full {
                self.full = true;
                log::warn!("world text: the atlas of {0} x {0} texels is full", self.side);
            }
            return None;
        }
        let (x0, y0) = (self.cursor.0 + PAD, self.cursor.1 + PAD);
        for y in 0..ph {
            let sy = (y as i32 - PAD as i32).clamp(0, h as i32 - 1) as u32;
            for x in 0..pw {
                let sx = (x as i32 - PAD as i32).clamp(0, w as i32 - 1) as u32;
                let edge = x < PAD || y < PAD || x >= w + PAD || y >= h + PAD;
                let dst = (((self.cursor.1 + y) * self.side + self.cursor.0 + x) * bpt) as usize;
                for c in 0..bpt {
                    let v = data[((sy * w + sx) * bpt + c) as usize];
                    self.pixels[dst + c as usize] = if edge && bpt == 1 { fill } else if edge && c + 1 == bpt { 0 } else { v };
                }
            }
        }
        self.cursor.0 += pw;
        self.shelf = self.shelf.max(ph);
        self.dirty = true;
        let s = self.side as f32;
        Some([x0 as f32 / s, y0 as f32 / s, (x0 + w) as f32 / s, (y0 + h) as f32 / s])
    }

    fn clear(&mut self) {
        self.pixels.fill(0);
        self.cursor = (0, 0);
        self.shelf = 0;
        self.dirty = true;
        self.full = false;
    }
}

/// The world text's passes: its two atlases and the glyphs of the frame.
pub struct WorldTextRenderer {
    pipeline: wgpu::RenderPipeline,
    /// The second pass, over what stands on the letters (see the module's notes), and what it
    /// reads of the scene: its depth, every sample.
    over_pipeline: wgpu::RenderPipeline,
    depth_layout: wgpu::BindGroupLayout,
    depth_group: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    over: bool,
    glyph_texture: wgpu::Texture,
    picture_texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    count: u32,
    glyphs: Atlas,
    pictures: Atlas,
    /// Told apart from the atlases another renderer made: the game keeps the cells it cut by this.
    serial: u64,
}

/// Cells (6 x 6 of two triangles) a draped glyph is cut into (the shader's `GRID`).
const VERTICES: u32 = 6 * 6 * 6;

static SERIALS: AtomicU64 = AtomicU64::new(1);

impl WorldTextRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, frame_layout: &wgpu::BindGroupLayout, hdr: wgpu::TextureFormat, samples: u32) -> Self {
        use wgpu::{BindingType as B, ShaderStages as S};
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: S::FRAGMENT,
            ty: B::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world text"),
            entries: &[texture_entry(0), texture_entry(1), wgpu::BindGroupLayoutEntry { binding: 2, visibility: S::FRAGMENT, ty: B::Sampler(wgpu::SamplerBindingType::Filtering), count: None }],
        });
        // (What the second pass reads of the scene: its depth, multisampled or not as it is drawn.)
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world text over"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: S::FRAGMENT,
                ty: B::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: samples > 1 },
                count: None,
            }],
        });
        let shader = crate::util::shader(device, "world text", &scene_source(samples));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("world text"), bind_group_layouts: &[Some(frame_layout), Some(&layout)], immediate_size: 0 });
        let over_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("world text over"), bind_group_layouts: &[Some(frame_layout), Some(&layout), Some(&depth_layout)], immediate_size: 0 });
        let over_constants = [("SCENE_SAMPLES", samples as f64)];
        let make = |label: &str, layout: &wgpu::PipelineLayout, fragment: &str, compare: wgpu::CompareFunction, constants: &[(&str, f64)]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_text"),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<GlyphInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4],
                    })],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                // Over the ground as a decal is: nearer to the eye by a little, written nowhere (the
                // blasts' scorch marks, drawn after it, lie over it). The second pass the other way
                // round with the same bias: only where something stands in front of the letter,
                // never where the first one drew it.
                depth_stencil: Some(wgpu::DepthStencilState {
                    depth_write_enabled: Some(false),
                    depth_compare: Some(compare),
                    bias: wgpu::DepthBiasState { constant: -16, slope_scale: -2.0, clamp: 0.0 },
                    ..depth_state(false)
                }),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: hdr,
                        // Over the picture (premultiplied: light adds with no alpha); the target's alpha
                        // (the heat's) kept.
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
                            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions { constants, ..Default::default() },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make("world text", &pipeline_layout, "fs_text", wgpu::CompareFunction::LessEqual, &[]);
        let over_pipeline = make("world text over", &over_layout, "fs_text_over", wgpu::CompareFunction::Greater, &over_constants);
        let make_texture = |label: &str, side: u32, format: wgpu::TextureFormat| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let glyph_texture = make_texture("world text glyphs", GLYPH_ATLAS, wgpu::TextureFormat::R8Unorm);
        let picture_texture = make_texture("world text pictures", PICTURE_ATLAS, wgpu::TextureFormat::Rgba8UnormSrgb);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("world text"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world text"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&glyph_texture.create_view(&Default::default())) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&picture_texture.create_view(&Default::default())) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let mut renderer = WorldTextRenderer {
            pipeline,
            over_pipeline,
            depth_layout,
            depth_group: None,
            over: !std::env::var("TA_TEXT_OVER").is_ok_and(|v| v == "0"),
            glyph_texture,
            picture_texture,
            bind_group,
            buffer: Self::make_buffer(device, 256),
            count: 0,
            glyphs: Atlas::new(GLYPH_ATLAS, 1),
            pictures: Atlas::new(PICTURE_ATLAS, 4),
            serial: SERIALS.fetch_add(1, Ordering::Relaxed),
        };
        renderer.upload(queue);
        renderer
    }

    fn make_buffer(device: &wgpu::Device, n: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor { label: Some("world text"), size: (n * std::mem::size_of::<GlyphInstance>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
    }

    /// Told apart from every other renderer's: the cells the game cut are this atlas's alone.
    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// Adds a glyph's distance field (`w` x `h` texels, 0.5 on the outline, 1 well inside, 0 well
    /// outside; [`SPREAD`] texels from the outline to 0 and 1): its cell, or none when the atlas
    /// is full.
    pub fn add_glyph(&mut self, w: u32, h: u32, field: &[u8]) -> Option<Cell> {
        self.glyphs.add(w, h, field, 0)
    }

    /// Adds a picture (RGBA, not premultiplied, sRGB): its cell.
    pub fn add_picture(&mut self, w: u32, h: u32, rgba: &[u8]) -> Option<Cell> {
        self.pictures.add(w, h, rgba, 0)
    }

    /// Forgets every glyph and picture (the game cuts the ones it needs again).
    pub fn clear_atlases(&mut self) {
        self.glyphs.clear();
        self.pictures.clear();
    }

    fn upload(&mut self, queue: &wgpu::Queue) {
        if self.glyphs.dirty {
            crate::util::write_texture_level(queue, &self.glyph_texture, 0, GLYPH_ATLAS, GLYPH_ATLAS, &self.glyphs.pixels, 1);
            self.glyphs.dirty = false;
        }
        if self.pictures.dirty {
            crate::util::write_texture_level(queue, &self.picture_texture, 0, PICTURE_ATLAS, PICTURE_ATLAS, &self.pictures.pixels, 4);
            self.pictures.dirty = false;
        }
    }

    /// This frame's glyphs (and the atlases' new cells).
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, glyphs: &[GlyphInstance]) {
        self.upload(queue);
        self.count = glyphs.len() as u32;
        if glyphs.is_empty() {
            return;
        }
        let bytes = std::mem::size_of_val(glyphs) as u64;
        if bytes > self.buffer.size() {
            self.buffer = Self::make_buffer(device, glyphs.len().next_power_of_two());
        }
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(glyphs));
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.draw(0..VERTICES, 0..self.count);
    }

    /// Whether this frame has glyphs to draw over what stands on them: the renderer then ends the
    /// opaque scene's pass before what is drawn over it, whose pass reads the scene's depth (as
    /// it does with the water in view), and draws [`Self::draw_over`] first in it.
    pub fn wants_over(&self) -> bool {
        self.over && self.count > 0
    }

    /// Readies the second pass for a scene whose depth target is `depth` (again when it changes).
    pub fn prepare_over(&mut self, device: &wgpu::Device, depth: &wgpu::TextureView) {
        if self.depth_group.as_ref().is_some_and(|(view, _)| view == depth) {
            return;
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world text over"),
            layout: &self.depth_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) }],
        });
        self.depth_group = Some((depth.clone(), group));
    }

    /// The glyphs again over what stands on them (see the module's notes), in a pass that keeps
    /// the scene's depth read-only, after the opaque scene and before what is drawn over it.
    pub fn draw_over<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let Some((_, depth)) = &self.depth_group else { return };
        if !self.wants_over() {
            return;
        }
        pass.set_pipeline(&self.over_pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_bind_group(2, depth, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.draw(0..VERTICES, 0..self.count);
    }
}

/// The world text's shader for a scene drawn with `samples` antialiasing samples: the scene's
/// depth its second pass reads is multisampled, or not (its pipeline sets `SCENE_SAMPLES`).
pub(crate) fn scene_source(samples: u32) -> String {
    let source = include_str!("shaders/world_text.wgsl");
    if samples > 1 { source.to_string() } else { source.replace("texture_depth_multisampled_2d", "texture_depth_2d") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_pack_in_shelves_and_keep_their_edges() {
        let mut atlas = Atlas::new(32, 1);
        let a = atlas.add(10, 6, &[200; 60], 0).expect("room");
        let b = atlas.add(10, 6, &[100; 60], 0).expect("room");
        // (Side by side on the first shelf, a texel of padding between.)
        assert!(b[0] > a[2], "{a:?} {b:?}");
        assert_eq!(a[1], b[1]);
        // The padding takes the outside value, the cell its own.
        assert_eq!(atlas.pixels[(1 * 32 + 1) as usize], 200);
        assert_eq!(atlas.pixels[0], 0);
        // A third no longer fits the shelf: it starts the next.
        let c = atlas.add(10, 6, &[50; 60], 0).expect("room");
        assert!(c[1] > a[3], "{a:?} {c:?}");
        // Too tall for what is left: none, and the atlas says it is full.
        assert!(atlas.add(30, 30, &vec![0; 900], 0).is_none());
        assert!(atlas.full);
    }

    #[test]
    fn a_picture_of_512_fits_its_atlas() {
        let mut atlas = Atlas::new(PICTURE_ATLAS, 4);
        assert!(atlas.add(512, 512, &vec![255; 512 * 512 * 4], 0).is_some());
    }

    #[test]
    fn a_glyph_instance_is_eight_vectors() {
        assert_eq!(std::mem::size_of::<GlyphInstance>(), 128);
    }
}
