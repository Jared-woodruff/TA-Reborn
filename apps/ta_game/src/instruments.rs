//! A modelled cockpit's instruments, painted onto its glass (Perspective): an egui context of
//! their own draws each instrument into its region of the cockpit's atlas
//! (`ta_render::cockpit::InstrumentAtlas`) every frame, before the frame is drawn, and the
//! glass shows the picture as a lit display. What is drawn is `perspective_hud`'s.

use egui::{Context, Pos2, Rect, pos2, vec2};
use rustc_hash::FxHashMap;
use ta_render::cockpit::InstrumentAtlas;

pub struct InstrumentPainter {
    ctx: Context,
    renderer: egui_wgpu::Renderer,
    /// Units' pictures for the instruments, loaded into this context (an egui texture belongs to
    /// the context that made it).
    pics: FxHashMap<usize, Option<egui::TextureHandle>>,
    /// Units' portraits drawn from their models (the HUD's, see `Hud::model_portrait`), registered
    /// with this renderer; none while asked for and not given yet.
    portraits: Portraits,
}

/// The instruments' portraits of units, by the HUD's key.
pub type Portraits = FxHashMap<crate::hud::PortraitKey, Option<egui::TextureId>>;

/// Paints the instruments: given the context, the unit pictures, the portraits (a picture not
/// there yet is asked for by adding its key) and each screen's region (by name, in the atlas's
/// pixels).
pub type PaintInstruments<'a> = dyn FnMut(&Context, &mut FxHashMap<usize, Option<egui::TextureHandle>>, &mut Portraits, &[(String, Rect)]) + 'a;

impl InstrumentPainter {
    /// None while the graphics card refuses its interface the memory (the screens stay dark).
    pub fn new(device: &wgpu::Device) -> Option<InstrumentPainter> {
        let renderer = crate::loading::ui_renderer(device, InstrumentAtlas::FORMAT, egui_wgpu::RendererOptions { msaa_samples: 1, depth_stencil_format: None, dithering: false, ..Default::default() })?;
        let ctx = Context::default();
        crate::theme::install_compact(&ctx);
        ctx.set_zoom_factor(1.0);
        Some(InstrumentPainter { ctx, renderer, pics: FxHashMap::default(), portraits: FxHashMap::default() })
    }

    /// The portraits the instruments asked for and do not have yet.
    pub fn wanted_portraits(&self) -> Vec<crate::hud::PortraitKey> {
        self.portraits.iter().filter(|(_, id)| id.is_none()).map(|(key, _)| *key).collect()
    }

    /// A portrait drawn (the picture the HUD shows), for the instruments too.
    pub fn add_portrait(&mut self, device: &wgpu::Device, key: crate::hud::PortraitKey, view: &wgpu::TextureView) {
        let id = self.renderer.register_native_texture(device, view, wgpu::FilterMode::Linear);
        self.portraits.insert(key, Some(id));
    }

    /// Paints the atlas afresh (it starts clear) and hands it to the GPU.
    pub fn paint(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, atlas: &InstrumentAtlas, now: f64, paint: &mut PaintInstruments) {
        let (w, h) = atlas.size;
        let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32))), time: Some(now), ..Default::default() };
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(1.0);
        let regions: Vec<(String, Rect)> = atlas.regions.iter().map(|(name, r)| (name.clone(), Rect::from_min_size(pos2(r[0] as f32, r[1] as f32), vec2(r[2] as f32, r[3] as f32)))).collect();
        self.ctx.begin_pass(raw);
        paint(&self.ctx, &mut self.pics, &mut self.portraits, &regions);
        let out = self.ctx.end_pass();
        let prims = self.ctx.tessellate(out.shapes, 1.0);
        // egui's deltas are commands: consume them as they are applied, just as the main UI
        // does. Leaving already-applied commands in the output panics when it is dropped.
        let mut textures_delta = out.textures_delta;
        for (id, deltas) in textures_delta.set.drain() {
            for delta in deltas {
                self.renderer.update_texture(device, queue, id, &delta);
            }
        }
        let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [w, h], pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("cockpit instruments") });
        // (Not when the card refuses its staging memory, as egui-wgpu panics then: the screens keep
        // what they showed.)
        let Some(extra) = ta_render::gpu::unless_short_of_memory(|| self.renderer.update_buffers(device, queue, &mut encoder, &prims, &sd)) else {
            for id in textures_delta.free.drain() {
                self.renderer.free_texture(&id);
            }
            return;
        };
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("cockpit instruments"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &atlas.view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &prims, &sd);
        }
        queue.submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
        for id in textures_delta.free.drain() {
            self.renderer.free_texture(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the real offscreen renderer: texture creation, updates on a later frame,
    /// and freeing on a third. This needs a GPU but no window or original game installation.
    #[test]
    #[ignore = "requires a GPU adapter; run with --ignored"]
    fn instruments_consume_texture_updates_and_releases() {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("instrument test atlas"),
            size: wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: InstrumentAtlas::FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let atlas = InstrumentAtlas { texture, view, size: (64, 64), regions: vec![("test".into(), [0, 0, 64, 64])] };
        let mut instruments = InstrumentPainter::new(&device).expect("the instruments' interface");
        instruments.paint(&device, &queue, &atlas, 0.0, &mut |ctx, pics, _, regions| {
            let texture = ctx.load_texture("test instrument", egui::ColorImage::filled([2, 2], egui::Color32::RED), egui::TextureOptions::LINEAR);
            ctx.debug_painter().image(texture.id(), regions[0].1, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), egui::Color32::WHITE);
            pics.insert(0, Some(texture));
        });
        let id = instruments.pics[&0].as_ref().unwrap().id();
        assert!(instruments.renderer.texture(&id).is_some(), "the first frame uploads its image");

        instruments.paint(&device, &queue, &atlas, 1.0, &mut |_, pics, _, _| {
            pics.get_mut(&0).unwrap().as_mut().unwrap().set(egui::ColorImage::filled([2, 2], egui::Color32::BLUE), egui::TextureOptions::LINEAR);
        });
        assert!(instruments.renderer.texture(&id).is_some(), "an update keeps the texture alive");

        instruments.pics.clear();
        instruments.paint(&device, &queue, &atlas, 2.0, &mut |_, _, _, _| {});
        assert!(instruments.renderer.texture(&id).is_none(), "released images leave the GPU renderer");
    }
}
