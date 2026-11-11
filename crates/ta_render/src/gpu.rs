//! Device, queue and window surface management, and what the card does when its memory runs out.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result};

/// How many times the graphics card has refused memory since the game started (an allocation it
/// could not make: the buffer or texture comes back invalid, and whatever is drawn with it fails).
/// A battlefield's loading compares it before and after: run short, the load fails cleanly instead
/// of leaving a half-made battlefield (or, before `CheckedBufferInit`, a crash).
static OUT_OF_MEMORY: AtomicU32 = AtomicU32::new(0);

/// The graphics device was lost (its callback, `Gpu::new`): nothing made with it works again.
static DEVICE_LOST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the graphics device was lost (to the driver, or to a card out of memory).
pub fn device_lost() -> bool {
    DEVICE_LOST.load(Ordering::Relaxed)
}

/// The count of the card's refusals so far (see `OUT_OF_MEMORY`).
pub fn out_of_memory_events() -> u32 {
    OUT_OF_MEMORY.load(Ordering::Relaxed)
}

/// What the game does with an error the card reports outside any error scope: written to the log
/// (with its causes), and a refusal of memory counted. Repeats of one message are written once a
/// second at most with how many there were: an invalid resource drawn every frame would otherwise
/// write thousands of lines a second and slow the game down with them.
pub fn report_uncaptured(e: &wgpu::Error) {
    if matches!(e, wgpu::Error::OutOfMemory { .. }) {
        OUT_OF_MEMORY.fetch_add(1, Ordering::Relaxed);
    }
    let mut msg = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        msg.push_str(&format!("\n  caused by: {s}"));
        src = s.source();
    }
    static LAST: std::sync::Mutex<Option<(String, std::time::Instant, u32)>> = std::sync::Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|p| p.into_inner());
    let now = std::time::Instant::now();
    match last.as_mut() {
        Some((m, at, repeats)) if *m == msg && now.duration_since(*at).as_secs_f32() < 1.0 => *repeats += 1,
        _ => {
            if let Some((m, _, repeats)) = last.take().filter(|l| l.2 > 0) {
                log::error!("wgpu: the last error again {repeats} more times: {}", m.lines().next().unwrap_or_default());
            }
            log::error!("wgpu: {msg}");
            *last = Some((msg, now, 0));
        }
    }
}

thread_local! {
    static SHORT_GUARD: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// `draw`, unless the card refuses memory to what a library panics for (egui-wgpu's
/// `update_buffers`, when the staging for an interface's vertices is refused): `None` then, and the
/// frame goes without what it would have drawn; the refusal is counted as the card's are. The
/// game's panic hook only logs a panic [`short_of_memory_panic`] says is one of these.
pub fn unless_short_of_memory<T>(draw: impl FnOnce() -> T) -> Option<T> {
    SHORT_GUARD.with(|g| g.set(g.get() + 1));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(draw));
    SHORT_GUARD.with(|g| g.set(g.get().saturating_sub(1)));
    if result.is_err() {
        OUT_OF_MEMORY.fetch_add(1, Ordering::Relaxed);
    }
    result.ok()
}

/// Whether the panic under way is inside [`unless_short_of_memory`] (caught: the card short of
/// memory): a panic hook logs it and says nothing more.
pub fn short_of_memory_panic() -> bool {
    SHORT_GUARD.with(|g| g.get() > 0)
}

/// Whether the card can still make a small buffer mapped for writing (a probe, made and dropped at
/// once). Asked before what makes buffers that way and cannot be told otherwise: egui-wgpu's
/// renderer makes its uniform buffer as wgpu's `create_buffer_init` does, and panics as it does
/// when the card refuses it. A refusal here is the probe's own (in its error scope): not counted.
pub fn can_allocate(device: &wgpu::Device) -> bool {
    // (Every kind of error caught: a refusal can come as more than OutOfMemory, a budget's as a
    // device error; and the probe is no good unless it maps.)
    let scopes = [wgpu::ErrorFilter::OutOfMemory, wgpu::ErrorFilter::Validation, wgpu::ErrorFilter::Internal].map(|f| device.push_error_scope(f));
    let probe = device.create_buffer(&wgpu::BufferDescriptor { label: Some("memory probe"), size: 256, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: true });
    let mapped = probe.get_mapped_range_mut(..).map(drop).map_err(|e| e.to_string());
    // (Popped innermost first.)
    let refused = scopes.into_iter().rev().filter_map(|s| pollster::block_on(s.pop())).next();
    match (&refused, mapped) {
        (None, Ok(())) => {
            probe.unmap();
            true
        }
        (Some(e), _) => {
            log::warn!("graphics memory: the card has none left ({e})");
            false
        }
        (None, Err(e)) => {
            log::warn!("graphics memory: the card has none left ({e})");
            false
        }
    }
}

/// Up to `mb` MB taken from the card in buffers written to (so that it really keeps them), 64 MB
/// at a time and then 1 MB at a time, stopping where it refuses: memory held so that running short
/// of it can be tried (the game's `TA_GPU_BALLAST_MB`).
pub fn take_ballast(device: &wgpu::Device, queue: &wgpu::Queue, mb: u64) -> Vec<wgpu::Buffer> {
    let mut held = Vec::new();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ballast") });
    let mut left = mb;
    for chunk in [64u64, 1] {
        while left >= chunk {
            let scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("ballast"), size: chunk << 20, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            if pollster::block_on(scope.pop()).is_some() {
                break;
            }
            encoder.clear_buffer(&buffer, 0, None);
            held.push(buffer);
            left -= chunk;
        }
    }
    queue.submit([encoder.finish()]);
    held
}

/// A buffer made with its contents, as `wgpu::util::DeviceExt::create_buffer_init` makes it, but
/// without its panic: when the card refuses the memory the buffer comes back invalid, and wgpu's
/// own `expect`s the mapping of it (a battlefield loading on a full card crashed the game). Here
/// the contents are not written then, the failure is logged (the refusal itself is counted where
/// the card reports it, `report_uncaptured`) and the invalid buffer is returned: what draws with it
/// fails as anything else made then does, and the loading that made it fails cleanly.
pub trait CheckedBufferInit {
    fn create_buffer_checked(&self, descriptor: &wgpu::util::BufferInitDescriptor<'_>) -> wgpu::Buffer;
}

impl CheckedBufferInit for wgpu::Device {
    fn create_buffer_checked(&self, descriptor: &wgpu::util::BufferInitDescriptor<'_>) -> wgpu::Buffer {
        if descriptor.contents.is_empty() {
            return self.create_buffer(&wgpu::BufferDescriptor { label: descriptor.label, size: 0, usage: descriptor.usage, mapped_at_creation: false });
        }
        // (As wgpu pads it: a multiple of COPY_BUFFER_ALIGNMENT, and never empty.)
        let unpadded = descriptor.contents.len() as wgpu::BufferAddress;
        let align = wgpu::COPY_BUFFER_ALIGNMENT - 1;
        let padded = ((unpadded + align) & !align).max(wgpu::COPY_BUFFER_ALIGNMENT);
        let buffer = self.create_buffer(&wgpu::BufferDescriptor { label: descriptor.label, size: padded, usage: descriptor.usage, mapped_at_creation: true });
        match buffer.get_mapped_range_mut(..) {
            Ok(mut range) => {
                range.slice(..unpadded as usize).copy_from_slice(descriptor.contents);
                drop(range);
                buffer.unmap();
            }
            Err(e) => log::error!("graphics memory: {} ({padded} bytes) was not made: {e}", descriptor.label.unwrap_or("a buffer")),
        }
        buffer
    }
}

/// Native APIs used by the game, launcher previews and GPU validation. Metal is the
/// native backend on both Intel and Apple Silicon Macs; Vulkan is not required there.
pub(crate) fn native_backends() -> wgpu::Backends {
    wgpu::Backends::DX12 | wgpu::Backends::VULKAN | wgpu::Backends::METAL
}

/// The options wgpu's backends open with: the `WGPU_*` environment's, and on Direct3D 12 without
/// `WGPU_DX12_COMPILER`, the shader compiler [`dx12_compiler`] picks.
pub(crate) fn backend_options() -> wgpu::BackendOptions {
    let mut options = wgpu::BackendOptions::from_env_or_default();
    if std::env::var_os("WGPU_DX12_COMPILER").is_none() {
        options.dx12.shader_compiler = dx12_compiler(shipped_dxc());
    }
    options
}

/// Direct3D 12's shader compiler: DXC from `shipped`, the dxcompiler.dll the Windows packages put
/// beside the game's programs (Microsoft's, `installer/windows_package.ps1`), else FXC
/// (d3dcompiler_47.dll, which every Windows has: several times slower over the game's shaders).
/// Never a dxcompiler.dll found along the PATH, as wgpu's own `Auto` would take: one older than
/// 1.8.2502 signs no shaders without a dxil.dll, and Direct3D refuses unsigned ones.
fn dx12_compiler(shipped: Option<std::path::PathBuf>) -> wgpu::Dx12Compiler {
    match shipped {
        Some(path) => wgpu::Dx12Compiler::DynamicDxc { dxc_path: path.to_string_lossy().into_owned() },
        None => wgpu::Dx12Compiler::Fxc,
    }
}

/// The dxcompiler.dll beside the running program, when it is there and loads.
#[cfg(windows)]
fn shipped_dxc() -> Option<std::path::PathBuf> {
    let path = std::env::current_exe().ok()?.parent()?.join("dxcompiler.dll");
    // SAFETY: loading the library runs its initialisation only, as wgpu's own loading of it does.
    (path.is_file() && unsafe { libloading::Library::new(&path) }.is_ok()).then_some(path)
}

#[cfg(not(windows))]
fn shipped_dxc() -> Option<std::path::PathBuf> {
    None
}

/// Whether wgpu's Direct3D 12 backend compiles shaders with FXC given `compiler` (with `Auto`,
/// from `WGPU_DX12_COMPILER=auto`, as wgpu decides: DXC where a dxcompiler.dll loads).
fn dx12_compiles_with_fxc(compiler: &wgpu::Dx12Compiler) -> bool {
    match compiler {
        wgpu::Dx12Compiler::Fxc => true,
        wgpu::Dx12Compiler::Auto => !dxcompiler_loads(),
        _ => false,
    }
}

#[cfg(windows)]
fn dxcompiler_loads() -> bool {
    // SAFETY: loading the library runs its initialisation only, as wgpu's own check does.
    unsafe { libloading::Library::new("dxcompiler.dll") }.is_ok()
}

#[cfg(not(windows))]
fn dxcompiler_loads() -> bool {
    false
}

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: Arc<wgpu::Surface<'static>>,
    pub config: wgpu::SurfaceConfiguration,
    /// The swapchain format viewed as sRGB so shaders can write linear values.
    pub view_format: wgpu::TextureFormat,
    /// The device traces rays (hardware ray tracing, asked for and supported; see raytrace.rs).
    pub ray_tracing: bool,
    /// A real swapchain frame has been queued for presentation (not an offscreen photo).
    presented: bool,
    /// A suboptimal frame remains valid until presentation. Reconfigure only before
    /// acquiring the next frame, after that texture has been presented or dropped.
    needs_reconfigure: bool,
}

impl Gpu {
    /// Opens the graphics card for `window`. With `ray_tracing`, and when the card traces rays
    /// in hardware, the device can trace them (see raytrace.rs).
    pub fn new(window: impl Into<wgpu::SurfaceTarget<'static>>, width: u32, height: u32, vsync: bool, ray_tracing: bool) -> Result<Self> {
        // (`TA_GPU_BUDGET=<percent>`: the card refuses what would take the game past that share of
        // the memory the system gives it, as a full card does; for trying the game short of memory.
        // Without it the system lends the card ordinary memory when its own runs out, slowly.)
        let budget = std::env::var("TA_GPU_BUDGET").ok().and_then(|v| v.parse::<u8>().ok());
        let backend_options = backend_options();
        let dx12_fxc = dx12_compiles_with_fxc(&backend_options.dx12.shader_compiler);
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_else(native_backends),
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options,
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds { for_resource_creation: budget, for_device_loss: None },
            display: None,
        });
        let surface = instance.create_surface(window).context("creating window surface")?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .context("no suitable GPU adapter found")?;
        let info = adapter.get_info();
        log::info!("GPU: {} ({:?}, driver {})", info.name, info.backend, info.driver_info);
        let fxc = info.backend == wgpu::Backend::Dx12 && dx12_fxc;
        if info.backend == wgpu::Backend::Dx12 {
            log::info!("Direct3D 12 compiles shaders with {}", if fxc { "FXC (d3dcompiler_47.dll)" } else { "DXC (dxcompiler.dll)" });
        }
        crate::util::set_fxc(fxc);

        let supported = adapter.limits();
        let required_limits = wgpu::Limits {
            max_texture_dimension_2d: supported.max_texture_dimension_2d.min(16384),
            max_texture_array_layers: supported.max_texture_array_layers.min(2048),
            max_storage_buffer_binding_size: supported.max_storage_buffer_binding_size,
            max_buffer_size: supported.max_buffer_size,
            ..wgpu::Limits::default()
        };
        // (Adapter-specific format features allow 2x and 8x antialiasing where supported.)
        let wanted = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::FLOAT32_FILTERABLE | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        let features = adapter.features() & wanted;
        // Ray tracing: ray queries (still experimental in wgpu), their limits, and more textures
        // per shader stage than the default 16 (the traced results join the frame's bindings,
        // which brings the water's pipeline, reading the scene behind it too, to 18).
        let can_trace = adapter.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) && supported.max_sampled_textures_per_shader_stage >= 18;
        if ray_tracing && !can_trace {
            log::info!("ray tracing: not supported by this graphics card");
        }
        let traced = ray_tracing && can_trace;
        let request = |traced: bool| wgpu::DeviceDescriptor {
            label: Some("ta device"),
            required_features: if traced { features | wgpu::Features::EXPERIMENTAL_RAY_QUERY } else { features },
            required_limits: if traced {
                wgpu::Limits { max_sampled_textures_per_shader_stage: supported.max_sampled_textures_per_shader_stage.min(32), ..required_limits.clone() }
                    .using_acceleration_structure_values(supported.clone())
            } else {
                required_limits.clone()
            },
            // SAFETY: ray queries are the one experimental feature asked for; the renderer uses
            // them as wgpu's own ray query examples do (see raytrace.rs).
            experimental_features: if traced { unsafe { wgpu::ExperimentalFeatures::enabled() } } else { wgpu::ExperimentalFeatures::disabled() },
            ..Default::default()
        };
        // (Should the card refuse the device with ray tracing, it opens without.)
        let (device, queue, traced) = match pollster::block_on(adapter.request_device(&request(traced))) {
            Ok((d, q)) => (d, q, traced),
            Err(e) if traced => {
                log::warn!("ray tracing: the graphics card refused it ({e}); starting without");
                let (d, q) = pollster::block_on(adapter.request_device(&request(false))).context("creating GPU device")?;
                (d, q, false)
            }
            Err(e) => return Err(e).context("creating GPU device"),
        };
        if traced {
            log::info!("ray tracing: on");
        }
        device.on_uncaptured_error(Arc::new(|e| report_uncaptured(&e)));
        // (A device lost, to the driver or a full card: everything made with it after is invalid.)
        device.set_device_lost_callback(|reason, message| {
            DEVICE_LOST.store(true, Ordering::Relaxed);
            log::error!("wgpu: the graphics device was lost ({reason:?}): {message}");
        });

        let mut config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .context("surface is not supported by the adapter")?;
        let caps = surface.get_capabilities(&adapter);
        // Prefer a non-sRGB swapchain format and render through an sRGB view.
        let base = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(config.format);
        config.format = base;
        let view_format = base.add_srgb_suffix();
        if view_format != base {
            config.view_formats = vec![view_format];
        }
        config.present_mode = if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        config.desired_maximum_frame_latency = 2;
        config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        surface.configure(&device, &config);

        Ok(Gpu { instance, adapter, device, queue, surface: Arc::new(surface), config, view_format, ray_tracing: traced, presented: false, needs_reconfigure: false })
    }

    /// A lightweight presenter for the same window while the renderer is constructed
    /// on a worker. GPU handles and the surface are shared; configuration and presentation
    /// tracking are independent. Only one owner may acquire, configure or present the
    /// surface at a time. During loading that is the main-thread loading screen: the
    /// worker may create resources and upload data, but must not touch the swapchain.
    /// Resize the renderer to the window's current dimensions when handing it back.
    pub fn loading_clone(&self) -> Self {
        Self {
            instance: self.instance.clone(),
            adapter: self.adapter.clone(),
            device: self.device.clone(),
            queue: self.queue.clone(),
            surface: self.surface.clone(),
            config: self.config.clone(),
            view_format: self.view_format,
            ray_tracing: self.ray_tracing,
            presented: false,
            needs_reconfigure: false,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.needs_reconfigure = false;
    }

    pub fn set_vsync(&mut self, vsync: bool) {
        self.config.present_mode = if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        self.surface.configure(&self.device, &self.config);
        self.needs_reconfigure = false;
    }

    /// Whether a window frame has been rendered and queued for presentation. A hidden
    /// startup window can be shown now; skipped frames and offscreen renders do not count.
    pub fn has_presented(&self) -> bool {
        self.presented
    }

    /// Queues a completed swapchain frame and records that this presenter has drawn.
    pub fn present(&mut self, frame: wgpu::SurfaceTexture) {
        self.queue.present(frame);
        self.presented = true;
    }

    /// Acquires the next swapchain image, reconfiguring on recoverable errors.
    pub fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        if std::mem::take(&mut self.needs_reconfigure) {
            self.surface.configure(&self.device, &self.config);
        }
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => Some(t),
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                // wgpu forbids configuring while an acquired texture is alive. This frame
                // can still be drawn; rebuild on the next acquire after it has been released.
                self.needs_reconfigure = true;
                Some(t)
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                None
            }
            other => {
                log::debug!("skipping frame: {other:?}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Direct3D 12 compiles with the dxcompiler.dll shipped beside the game, by its own path (never
    /// one found along the PATH), and with FXC without it; side-by-side pipeline builds follow FXC.
    #[test]
    fn direct3d_12_compiles_with_the_shipped_dxc_else_fxc() {
        let shipped = std::path::PathBuf::from(r"C:\Games\TA Reborn\dxcompiler.dll");
        match dx12_compiler(Some(shipped.clone())) {
            wgpu::Dx12Compiler::DynamicDxc { dxc_path } => assert_eq!(std::path::Path::new(&dxc_path), shipped),
            other => panic!("expected the shipped DXC, got {other:?}"),
        }
        assert!(matches!(dx12_compiler(None), wgpu::Dx12Compiler::Fxc));
        assert!(!dx12_compiles_with_fxc(&dx12_compiler(Some(shipped))));
        assert!(dx12_compiles_with_fxc(&dx12_compiler(None)));
    }

    /// The card's refusals of memory reported outside an error scope are counted (a battlefield's
    /// loading fails cleanly on them), its other errors not.
    #[test]
    fn refused_memory_is_counted() {
        let before = out_of_memory_events();
        report_uncaptured(&wgpu::Error::OutOfMemory { source: Box::new(std::io::Error::other("no room")) });
        report_uncaptured(&wgpu::Error::OutOfMemory { source: Box::new(std::io::Error::other("no room")) });
        report_uncaptured(&wgpu::Error::Internal { source: Box::new(std::io::Error::other("other")), description: "other".into() });
        // (Other tests may report too: at least these two.)
        assert!(out_of_memory_events() >= before + 2);
    }

    /// A buffer the card cannot make is returned unmade, without the panic of wgpu's own
    /// `create_buffer_init` (which crashed the game when a battlefield loaded on a full card); one it
    /// can make holds its contents.
    #[test]
    #[ignore = "requires a GPU; run with --ignored"]
    fn a_buffer_the_card_cannot_make_is_no_crash() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_else(native_backends),
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: crate::gpu::backend_options(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        // Larger than the device makes buffers: invalid from the start, as one refused for want of
        // memory is (its error caught here, where a test has no handler to log it).
        let too_big = vec![7u8; (device.limits().max_buffer_size + 4) as usize];
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let _unmade = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("too big"), contents: &too_big, usage: wgpu::BufferUsages::VERTEX });
        assert!(pollster::block_on(scope.pop()).is_some(), "the card refused it");
        drop(too_big);
        // One it can make: its contents, read back.
        let contents: Vec<u8> = (0..10u8).collect();
        let made = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("made"), contents: &contents, usage: wgpu::BufferUsages::COPY_SRC });
        let read = device.create_buffer(&wgpu::BufferDescriptor { label: Some("read"), size: 12, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_buffer_to_buffer(&made, 0, &read, 0, 12);
        queue.submit([encoder.finish()]);
        read.map_async(wgpu::MapMode::Read, .., |r| r.expect("readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        assert_eq!(&read.get_mapped_range(..).expect("mapped")[..10], &contents[..]);
    }

    /// The probe says yes on a card with room, and leaves nothing behind.
    #[test]
    #[ignore = "requires a GPU; run with --ignored"]
    fn a_card_with_room_can_allocate() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_else(native_backends),
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: crate::gpu::backend_options(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
        let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        assert!(can_allocate(&device));
        assert!(can_allocate(&device), "twice");
    }
}
