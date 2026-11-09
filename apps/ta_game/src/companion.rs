//! The companion screen: a second window beside the battle (filling a second display when there
//! is one) with the battle's map at its full size and the intel a commander keeps glancing at;
//! `hud/companion.rs` draws its page. It is a window of its own on the same graphics card: a
//! surface of its own, an interface of its own (fonts and pictures loaded there) and a copy of
//! the HUD's panels, drawn some thirty times a second after the battle's frame (never more often
//! than every other refresh of its display, and handed over without waiting for it, so that it
//! never holds the battle up). On Windows it never takes the keyboard from the battle (a window
//! that does not activate): a click on it looks or orders, and the keys stay the game's. Its key
//! (Alt+C) or the pause menu opens and closes it, and the settings can have every battle open it
//! on a second display.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Fullscreen, Window, WindowId};

use crate::hud::Hud;
use crate::minimap::Minimap;

/// How often the companion is drawn at most.
const EVERY: Duration = Duration::from_millis(33);
/// How seldom it is drawn at the least, however slowly its display takes its frames.
const SLOWEST: Duration = Duration::from_millis(133);
/// Handing a frame over taking longer than this holds the battle up (the companion is drawn on
/// the battle's thread): three in a row and it is drawn half as often.
const HOLDS_UP: Duration = Duration::from_millis(4);

pub struct Companion {
    pub window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    ctx: egui::Context,
    input: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    /// Its own HUD (its pictures and skin are its interface's) and map.
    pub hud: Hud,
    pub minimap: Minimap,
    drawn: Option<Instant>,
    /// How often it is drawn (`pace`), and the frames in a row that held the battle up.
    every: Duration,
    slow: u8,
    resized: bool,
    /// Captures: the next frame's picture is saved there (`TA_COMPANION_SHOT`).
    pub snapshot: Option<std::path::PathBuf>,
}

/// The display a companion opens on: one other than the battle's (`main`), if there is one.
pub fn other_display(event_loop: &ActiveEventLoop, main: &Window) -> Option<winit::monitor::MonitorHandle> {
    let current = main.current_monitor();
    event_loop.available_monitors().find(|m| Some(m) != current.as_ref())
}

impl Companion {
    /// Opens the companion: filling another display than the battle's when there is one, else a
    /// window beside it; in a capture (`capture`), hidden and `TA_COMPANION_SIZE=<w>x<h>`
    /// (1920x1080) large.
    pub fn open(event_loop: &ActiveEventLoop, gpu: &ta_render::Gpu, main: &Window, capture: bool) -> Result<Companion> {
        let icon =winit::window::Icon::from_rgba(ta_ui::icon::emblem_rgba(64), 64, 64).ok();
        let mut attrs = Window::default_attributes().with_title(ta_ui::i18n::t("TA Reborn · Companion screen")).with_window_icon(icon).with_active(false);
        attrs = match other_display(event_loop, main) {
            _ if capture => {
                let size = std::env::var("TA_COMPANION_SIZE").ok().and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?))));
                let (w, h) = size.unwrap_or((1920, 1080));
                attrs.with_visible(false).with_inner_size(winit::dpi::PhysicalSize::new(w, h))
            }
            Some(display) => attrs.with_fullscreen(Some(Fullscreen::Borderless(Some(display)))),
            None => attrs.with_inner_size(winit::dpi::LogicalSize::new(1280.0, 760.0)),
        };
        let window = Arc::new(event_loop.create_window(attrs).context("opening the companion screen")?);
        keep_the_keyboard_away(&window);
        let surface = gpu.instance.create_surface(window.clone()).context("the companion screen's surface")?;
        let size = window.inner_size();
        let mut config = surface.get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1)).context("the graphics card cannot draw the companion screen")?;
        let caps = surface.get_capabilities(&gpu.adapter);
        if let Some(format) = caps.formats.iter().copied().find(|f| !f.is_srgb()) {
            config.format = format;
        }
        // In turn with its display (Fifo): drawn at most every other refresh, each frame is shown
        // at the next one and handing it over never waits. (Without waiting for the display,
        // Mailbox or Immediate, some drivers wait for it all the same, a whole refresh in handing
        // each frame over: the battle held up thirty times a second.)
        config.present_mode = wgpu::PresentMode::Fifo;
        config.desired_maximum_frame_latency = 2;
        let format = config.format;
        surface.configure(&gpu.device, &config);
        let ctx = egui::Context::default();
        crate::theme::install_compact(&ctx);
        ta_ui::sound::enable(&ctx, true);
        let input = egui_winit::State::new(ctx.clone(), egui::ViewportId::ROOT, &window, Some(window.scale_factor() as f32), None, Some(gpu.device.limits().max_texture_dimension_2d as usize));
        let renderer = crate::loading::ui_renderer(&gpu.device, format, egui_wgpu::RendererOptions { msaa_samples: 1, depth_stencil_format: None, dithering: true, ..Default::default() })
            .context("not enough graphics memory for the companion screen")?;
        log::info!("companion screen: {} x {}{}", size.width, size.height, if window.fullscreen().is_some() { " on a second display" } else { "" });
        let every = pace(&window);
        Ok(Companion { window, surface, config, ctx, input, renderer, hud: Hud::default(), minimap: Minimap::default(), drawn: None, every, slow: 0, resized: false, snapshot: None })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    /// One of its window's events; true when it was closed.
    pub fn event(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => return true,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => self.resized = true,
            _ => {}
        }
        let _ = self.input.on_window_event(&self.window, event);
        false
    }

    /// A frame is due (some thirty a second).
    pub fn due(&self) -> bool {
        self.drawn.is_none_or(|t| t.elapsed() >= self.every)
    }

    /// Draws a frame, its page laid out by `page` (its HUD and map handed to it); returns the
    /// interface's sound for what was clicked, if any.
    pub fn frame(&mut self, gpu: &ta_render::Gpu, mut page: impl FnMut(&mut egui::Ui, &mut Hud, &mut Minimap)) -> Option<ta_ui::sound::UiSound> {
        self.drawn = Some(Instant::now());
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return None;
        }
        if std::mem::take(&mut self.resized) || (self.config.width, self.config.height) != (size.width, size.height) {
            (self.config.width, self.config.height) = (size.width, size.height);
            self.surface.configure(&gpu.device, &self.config);
            // (Perhaps on another display now.)
            self.every = pace(&self.window);
        }
        // Laid out on 600 points high (or 1,024 wide), as the battle's interface is.
        let native = (self.window.scale_factor() as f32).max(0.5);
        let zoom = (size.width as f32 / native / 1024.0).min(size.height as f32 / native / 600.0).clamp(0.6, 4.0);
        if (self.ctx.zoom_factor() - zoom).abs() > 0.001 {
            self.ctx.set_zoom_factor(zoom);
        }
        let raw = self.input.take_egui_input(&self.window);
        let (hud, minimap) = (&mut self.hud, &mut self.minimap);
        let output = self.ctx.run_ui(raw, |ui| page(ui, hud, minimap));
        self.input.handle_platform_output(&self.window, output.platform_output);
        let ppp = output.pixels_per_point;
        let prims = self.ctx.tessellate(output.shapes, ppp);
        let mut textures = output.textures_delta;
        for (id, deltas) in textures.set.drain() {
            for delta in deltas {
                self.renderer.update_texture(&gpu.device, &gpu.queue, id, &delta);
            }
        }
        let handed = Instant::now();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&gpu.device, &self.config);
                None
            }
            other => {
                log::debug!("companion screen: skipping a frame: {other:?}");
                None
            }
        };
        let mut waited = handed.elapsed();
        let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [self.config.width, self.config.height], pixels_per_point: ppp };
        if let Some(frame) = frame {
            let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("companion screen") });
            // (Not when the card refuses its staging memory, as egui-wgpu panics then: the screen keeps
            // its last picture.)
            if ta_render::gpu::unless_short_of_memory(|| self.renderer.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &prims, &sd)).is_some() {
                self.draw(&mut encoder, &view, &prims, &sd);
                gpu.queue.submit(Some(encoder.finish()));
                let presented = Instant::now();
                gpu.queue.present(frame);
                waited += presented.elapsed();
            }
        }
        self.slow = if waited > HOLDS_UP { self.slow + 1 } else { 0 };
        if self.slow >= 3 && self.every < SLOWEST {
            self.every = (self.every * 2).min(SLOWEST);
            self.slow = 0;
            log::info!("companion screen: its display took {:.1} ms a frame; drawn every {} ms", waited.as_secs_f32() * 1000.0, self.every.as_millis());
        }
        if let Some(path) = self.snapshot.take() {
            self.save(gpu, &prims, &sd, &path);
        }
        for id in textures.free.drain() {
            self.renderer.free_texture(&id);
        }
        ta_ui::sound::take(&self.ctx)
    }

    /// The interface's shapes (`prims`) drawn into `view`, over black.
    fn draw(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, prims: &[egui::ClippedPrimitive], sd: &egui_wgpu::ScreenDescriptor) {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("companion screen"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        self.renderer.render(&mut pass, prims, sd);
    }

    /// Captures: the frame just laid out drawn again into a picture of its own and saved at
    /// `path` (the window's own surface cannot be read back).
    fn save(&mut self, gpu: &ta_render::Gpu, prims: &[egui::ClippedPrimitive], sd: &egui_wgpu::ScreenDescriptor, path: &std::path::Path) {
        let (width, height) = (self.config.width, self.config.height);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("companion picture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let padded = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("companion picture"), size: (padded * height) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("companion picture") });
        self.draw(&mut encoder, &view, prims, sd);
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(height) } },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        gpu.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        if let Err(e) = gpu.device.poll(wgpu::PollType::wait_indefinitely()) {
            log::warn!("companion picture: {e}");
            return;
        }
        let Ok(data) = slice.get_mapped_range() else { return };
        let bgra = matches!(self.config.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            for px in data[(row * padded) as usize..(row * padded + width * 4) as usize].as_chunks::<4>().0 {
                rgba.extend_from_slice(&if bgra { [px[2], px[1], px[0], 255] } else { [px[0], px[1], px[2], 255] });
            }
        }
        match image::save_buffer(path, &rgba, width, height, image::ColorType::Rgba8) {
            Ok(()) => log::info!("saved the companion screen's picture {}", path.display()),
            Err(e) => log::warn!("companion picture {}: {e}", path.display()),
        }
    }

    /// A unit's portrait drawn for the battle's interface (`view`), registered with this one's.
    pub fn register_portrait(&mut self, gpu: &ta_render::Gpu, view: &wgpu::TextureView) -> egui::TextureId {
        self.renderer.register_native_texture(&gpu.device, view, wgpu::FilterMode::Linear)
    }
}

/// How often a companion on `window`'s display is drawn (`pace_for` its refresh).
fn pace(window: &Window) -> Duration {
    pace_for(window.current_monitor().and_then(|m| m.refresh_rate_millihertz()))
}

/// How often a companion is drawn on a display refreshing `refresh` times a thousand seconds:
/// some thirty times a second, but never more often than every other refresh (a 30 Hz
/// television's: fifteen), so that its frames never queue up for the display.
fn pace_for(refresh: Option<u32>) -> Duration {
    refresh.filter(|&mhz| mhz >= 1000).map_or(EVERY, |mhz| EVERY.max(Duration::from_secs_f64(2000.0 / f64::from(mhz))).min(SLOWEST))
}

/// On Windows: a window that does not activate (a click on it leaves the keyboard with the
/// battle), still on the taskbar.
fn keep_the_keyboard_away(window: &Window) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE};
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = window.window_handle() else { return };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else { return };
        let hwnd = HWND(handle.hwnd.get() as *mut core::ffi::c_void);
        // SAFETY: the window is alive (we hold it) and its extended style is only added to.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | (WS_EX_NOACTIVATE.0 | WS_EX_APPWINDOW.0) as isize);
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_drawn_at_most_every_other_refresh() {
        let ms = |refresh| pace_for(refresh).as_secs_f32() * 1000.0;
        // Thirty a second on the usual displays, and when the display does not say.
        assert_eq!(pace_for(Some(60_000)).as_millis(), 33);
        assert_eq!(pace_for(Some(144_000)), EVERY);
        assert_eq!(pace_for(None), EVERY);
        assert_eq!(pace_for(Some(0)), EVERY);
        // Slower displays: every other refresh.
        assert!((ms(Some(30_000)) - 66.7).abs() < 0.1);
        assert!((ms(Some(24_000)) - 83.3).abs() < 0.1);
        assert_eq!(pace_for(Some(59_940)).as_millis(), 33);
        // And never more seldom than that.
        assert_eq!(pace_for(Some(5_000)), SLOWEST);
    }
}
