//! Main-thread loading presentation, using the game's own window and GPU. The full
//! renderer can compile pipelines and load assets on a worker while this small egui
//! renderer keeps showing progress, animating, resizing and receiving window events.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use ta_render::gpu::Gpu;
use ta_ui::loading::{LoadingKind, LoadingScreen};
use winit::event::WindowEvent;
use winit::window::Window;

pub struct LoadingHost {
    window: Arc<Window>,
    gpu: Gpu,
    ctx: egui::Context,
    input: egui_winit::State,
    /// None when the graphics card refused it (`ui_renderer`): the background alone is drawn.
    renderer: Option<egui_wgpu::Renderer>,
    screen: LoadingScreen,
    kind: LoadingKind,
    status: String,
    detail: String,
    since: Instant,
    occluded: bool,
    /// When the screen began to go (on its own clock), the loading done: the words and the
    /// emblem fade off the bay for [`LoadingHost::outro`] before what was loaded comes up.
    leaving_since: Option<f64>,
    /// The graphics card refused the interface its memory once: drawn without it since.
    short: bool,
    /// Explicit developer capture only; ordinary loading never reads the GPU back.
    captures: Option<LoadingCaptures>,
}

impl LoadingHost {
    pub fn init_accessibility(&mut self, event_loop: &winit::event_loop::ActiveEventLoop,
        proxy: winit::event_loop::EventLoopProxy<egui_winit::accesskit_winit::Event>,
        adapter: Option<egui_winit::accesskit_winit::Adapter>, active: bool)
    {
        self.input.accesskit = adapter;
        if self.input.accesskit.is_none() { self.input.init_accesskit(event_loop, &self.window, proxy); }
        if active { self.ctx.enable_accesskit(); }
    }

    pub fn take_accessibility(&mut self) -> Option<egui_winit::accesskit_winit::Adapter> {
        self.input.accesskit.take()
    }

    pub fn accessibility_event(&mut self, event: egui_winit::accesskit_winit::WindowEvent) {
        crate::accessibility::native_event(&mut self.input, event);
        self.window.request_redraw();
    }
    /// `gpu` is normally [`Gpu::loading_clone`]. While this host is drawing, the worker
    /// must not acquire, configure or present the shared surface. Creating GPU resources
    /// and uploading assets on the worker remains safe.
    pub fn new(window: Arc<Window>, gpu: Gpu, kind: LoadingKind, scale: f32) -> Self {
        let ctx = loading_context(scale);
        let captures = LoadingCaptures::from_env();
        // A transition can begin while the existing window is already unfocused. (Not when its
        // frames are being shot, `TA_LOADING_SHOTS`: a capture's window has no focus, and its
        // pictures are of the screen as the player sees it, moving.)
        ta_ui::motion::set_still(&ctx, !window.has_focus() && captures.is_none());
        let input = egui_winit::State::new(
            ctx.clone(), egui::ViewportId::ROOT, &window,
            Some(window.scale_factor() as f32), None,
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let renderer = ui_renderer(&gpu.device, gpu.config.format, egui_wgpu::RendererOptions {
            msaa_samples: 1, depth_stencil_format: None, dithering: true, ..Default::default()
        });
        let short = renderer.is_none();
        Self {
            window, gpu, ctx, input, renderer, screen: LoadingScreen::new(kind), kind, short,
            status: ta_ui::i18n::t("Starting TA Reborn").to_owned(), detail: String::new(),
            since: Instant::now(), occluded: false, leaving_since: None, captures,
        }
    }

    /// How long the screen takes to go once loading is done: none between a benchmark's scenes
    /// (its timings are the player's) or with reduced motion.
    fn outro(&self) -> f64 {
        if self.kind == LoadingKind::Benchmark || ta_ui::motion::reduced(&self.ctx) { 0.0 } else { 0.2 }
    }

    /// The loading is done: the screen begins to go (it has gone after [`Self::outro`]).
    pub fn leave(&mut self) {
        if self.leaving_since.is_none() {
            self.leaving_since = Some(self.elapsed());
            self.window.request_redraw();
        }
    }

    /// Whether the screen has gone (and what was loaded can come up).
    pub fn gone(&self) -> bool {
        self.leaving_since.is_some_and(|since| self.elapsed() - since >= self.outro())
    }

    /// How far the screen has gone (0 to 1).
    fn leaving(&self) -> f32 {
        match self.leaving_since {
            Some(since) if self.outro() > 0.0 => ((self.elapsed() - since) / self.outro()).clamp(0.0, 1.0) as f32,
            Some(_) => 1.0,
            None => 0.0,
        }
    }

    /// Actual loading work reported by the worker, already translated by the caller.
    pub fn set_stage(&mut self, status: impl Into<String>, detail: impl Into<String>) {
        self.status = status.into();
        self.detail = detail.into();
        self.window.request_redraw();
    }

    pub fn set_hint_offset(&mut self, offset: usize) {
        self.screen.set_hint_offset(offset);
    }

    pub fn set_seasonal(&mut self, settings: &ta_config::Launcher, reduced_flashes: bool) {
        self.screen.set_seasonal(settings, reduced_flashes);
    }

    pub fn set_reduced_motion(&self, reduced: bool) {
        ta_ui::motion::set_reduced(&self.ctx, reduced);
    }

    pub fn elapsed(&self) -> f64 {
        self.since.elapsed().as_secs_f64()
    }

    /// Forward window events here while loading. Close requests remain the application's
    /// responsibility; no game input is retained and played back after loading finishes.
    pub fn event(&mut self, event: &WindowEvent) {
        let _ = self.input.on_window_event(&self.window, event);
        match event {
            WindowEvent::Resized(size) => self.gpu.resize(size.width, size.height),
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = self.window.inner_size();
                self.gpu.resize(size.width, size.height);
            }
            WindowEvent::Focused(focused) => ta_ui::motion::set_still(&self.ctx, !focused && self.captures.is_none()),
            WindowEvent::Occluded(occluded) => self.occluded = *occluded,
            _ => {}
        }
    }

    /// Paints and presents one loading frame. The caller shows the initially hidden
    /// window only after `true`, and drives redraws at a bounded rate while loading.
    /// Zero-size, minimized, occluded or temporarily unavailable surfaces return `false`.
    pub fn frame(&mut self) -> Result<bool> {
        let size = self.window.inner_size();
        // A hidden startup window may already be reported as occluded. Its first frame
        // still has to be drawn so the caller can reveal it without a white client area.
        if size.width == 0 || size.height == 0 || self.window.is_minimized() == Some(true)
            || (self.occluded && self.gpu.has_presented())
        {
            return Ok(false);
        }
        if self.gpu.config.width != size.width || self.gpu.config.height != size.height {
            self.gpu.resize(size.width, size.height);
        }
        let Some(frame) = self.gpu.acquire() else { return Ok(false) };
        // egui's shader writes display-encoded colour, as in the game's existing UI pass.
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut raw = self.input.take_egui_input(&self.window);
        let elapsed = self.elapsed();
        raw.time = Some(elapsed);
        let leaving = self.leaving();
        let mut output = self.ctx.run_ui(raw, |ui| self.screen.show_leaving(ui.ctx(), &self.status, &self.detail, elapsed, leaving));
        // Publish the accessibility tree while keeping the benchmark pointer hidden.
        if matches!(self.kind, LoadingKind::Benchmark) { output.platform_output.cursor_icon = egui::CursorIcon::None; }
        self.input.handle_platform_output(&self.window, output.platform_output);
        let pixels_per_point = output.pixels_per_point;
        let primitives = self.ctx.tessellate(output.shapes, pixels_per_point);
        for (id, deltas) in output.textures_delta.set.drain() {
            for delta in deltas {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.update_texture(&self.gpu.device, &self.gpu.queue, id, &delta);
                }
            }
        }
        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("loading screen") });
        let descriptor = egui_wgpu::ScreenDescriptor { size_in_pixels: [size.width, size.height], pixels_per_point };
        // (When the card refuses the interface its staging memory, egui-wgpu panics, and its buffers
        // may be left unmade: the loading screen goes on with its background alone, a new one made
        // at the next load.)
        let updated = match self.renderer.as_mut() {
            Some(renderer) if !self.short => ta_render::gpu::unless_short_of_memory(|| renderer.update_buffers(&self.gpu.device, &self.gpu.queue, &mut encoder, &primitives, &descriptor)),
            _ => None,
        };
        self.short |= updated.is_none();
        let drawn = updated.is_some();
        let mut commands = updated.unwrap_or_default();
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("loading screen"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view, resolve_target: None, depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.055, g: 0.051, b: 0.043, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            }).forget_lifetime();
            if drawn && let Some(renderer) = self.renderer.as_ref() {
                renderer.render(&mut pass, &primitives, &descriptor);
            }
        }
        let capture = self.captures.as_mut().and_then(|shots| shots.encode(&self.gpu, &mut encoder, &frame.texture, elapsed));
        commands.push(encoder.finish());
        self.gpu.queue.submit(commands);
        self.window.pre_present_notify();
        self.gpu.present(frame);
        for id in output.textures_delta.free.drain() {
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.free_texture(&id);
            }
        }
        if let Some(capture) = capture {
            capture.save(&self.gpu)?;
        }
        Ok(true)
    }
}

/// An interface's renderer, unless the graphics card refuses it the memory: egui-wgpu panics
/// making its buffers then (the `expect` in wgpu's `create_buffer_init`), so the card is asked
/// first and the making is guarded (the panic only logged: `ta_render::gpu::unless_short_of_memory`).
pub(crate) fn ui_renderer(device: &wgpu::Device, format: wgpu::TextureFormat, options: egui_wgpu::RendererOptions) -> Option<egui_wgpu::Renderer> {
    if !ta_render::can_allocate(device) {
        return None;
    }
    ta_render::gpu::unless_short_of_memory(|| egui_wgpu::Renderer::new(device, format, options))
}

/// Initialize zoom before egui-winit converts framebuffer pixels to logical points.
/// `set_zoom_factor` is for an existing context: its deferred change rescales the
/// previous viewport and overrides the next input rectangle. A fresh context has no
/// previous window bounds, so use the initial option here to keep its very first
/// presented loading frame inside the real window, including at enlarged UI scales.
fn loading_context(scale: f32) -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_compact(&ctx);
    ctx.options_mut(|options| options.zoom_factor = if scale.is_finite() { scale.clamp(0.5, 3.0) } else { 1.0 });
    ctx
}

/// `TA_LOADING_SHOTS=<directory>` records three actual swapchain frames per loading
/// screen: at its start, after 0.35 seconds and after 1.2 seconds (or at the seconds
/// `TA_LOADING_SHOT_TIMES=0,0.8,2.5` lists, while the screen is still up). The explicit
/// capture mode waits for GPU readback; it is for visual QA, never performance measurements.
struct LoadingCaptures {
    dir: PathBuf,
    host: u64,
    next: usize,
    times: Vec<f64>,
}

impl LoadingCaptures {
    fn from_env() -> Option<Self> {
        static HOSTS: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::var_os("TA_LOADING_SHOTS").filter(|value| !value.is_empty())?;
        let mut times: Vec<f64> = std::env::var("TA_LOADING_SHOT_TIMES").ok()
            .map(|list| list.split(',').filter_map(|s| s.trim().parse().ok()).filter(|t: &f64| t.is_finite()).collect())
            .filter(|times: &Vec<f64>| !times.is_empty())
            .unwrap_or_else(|| vec![0.0, 0.35, 1.2]);
        times.sort_by(f64::total_cmp);
        Some(Self { dir: PathBuf::from(dir), host: HOSTS.fetch_add(1, Ordering::Relaxed), next: 0, times })
    }

    fn encode(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture, elapsed: f64) -> Option<LoadingCapture> {
        if self.next >= self.times.len() || elapsed < self.times[self.next] {
            return None;
        }
        let (width, height) = (texture.width(), texture.height());
        let padded = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("loading screenshot"), size: padded as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(texture.as_image_copy(), wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(height) },
        }, wgpu::Extent3d { width, height, depth_or_array_layers: 1 });
        let path = self.dir.join(format!("loading-{}-{:02}-{}.png", std::process::id(), self.host, self.next));
        self.next += 1;
        Some(LoadingCapture { buffer, width, height, padded, path })
    }
}

struct LoadingCapture {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    padded: u32,
    path: PathBuf,
}

impl LoadingCapture {
    fn save(self, gpu: &Gpu) -> Result<()> {
        let slice = self.buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).context("reading loading screenshot")?;
        let data = slice.get_mapped_range().context("mapping loading screenshot")?;
        let bgra = matches!(gpu.config.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut rgba = Vec::with_capacity(self.width as usize * self.height as usize * 4);
        for row in 0..self.height {
            let start = (row * self.padded) as usize;
            for px in data[start..start + self.width as usize * 4].as_chunks::<4>().0 {
                let pixel = if bgra { [px[2], px[1], px[0], 255] } else { [px[0], px[1], px[2], 255] };
                rgba.extend_from_slice(&pixel);
            }
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).context("creating loading screenshots directory")?;
        }
        image::save_buffer(&self.path, &rgba, self.width, self.height, image::ColorType::Rgba8).context("saving loading screenshot")?;
        log::info!("saved loading screenshot {}", self.path.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_loading_frame_uses_window_bounds_at_initial_ui_scale() {
        for (scale, native_dpi, pixels, expected_zoom) in [
            (1.0, 1.0, egui::vec2(1280.0, 720.0), 1.0),
            (1.5, 1.5, egui::vec2(1280.0, 720.0), 1.5),
            (0.1, 2.0, egui::vec2(1280.0, 720.0), 0.5),
            (9.0, 1.0, egui::vec2(1920.0, 1080.0), 3.0),
            (f32::NAN, 1.5, egui::vec2(1280.0, 720.0), 1.0),
            (f32::INFINITY, 1.0, egui::vec2(1280.0, 720.0), 1.0),
        ] {
            let ctx = loading_context(scale);
            let pixels_per_point = native_dpi * expected_zoom;
            let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, pixels / pixels_per_point);
            assert_eq!(ctx.zoom_factor(), expected_zoom, "zoom must be ready before egui-winit sizes its first input");
            let mut raw = egui::RawInput { screen_rect: Some(bounds), time: Some(0.0), ..Default::default() };
            raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(native_dpi);
            let mut output = ctx.run_ui(raw, |ui| {
                let actual = ui.ctx().viewport_rect();
                assert_eq!(actual.min, bounds.min);
                // egui rounds logical dimensions to physical pixels at fractional DPI.
                assert!((actual.width() - bounds.width()).abs() <= 1.0 / pixels_per_point
                    && (actual.height() - bounds.height()).abs() <= 1.0 / pixels_per_point,
                    "initial zoom must preserve the real window bounds: {actual:?} versus {bounds:?}");
                LoadingScreen::new(LoadingKind::Benchmark)
                    .show(ui.ctx(), "Preparing the next scene", "Combined arms on Sapphire Reach", 0.0);
            });
            output.textures_delta.clear();
            assert_eq!(output.pixels_per_point, pixels_per_point);
            let mut status_visible = false;
            for clipped in output.shapes {
                if let egui::Shape::Text(text) = clipped.shape {
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    assert!(bounds.contains_rect(rect), "first-frame text outside {pixels:?} at zoom {expected_zoom}/DPI {native_dpi}: {:?}", text.galley.job.text);
                    status_visible |= text.galley.job.text == "Preparing the next scene";
                }
            }
            assert!(status_visible, "the first presented frame must already show progress");
        }
    }
}
