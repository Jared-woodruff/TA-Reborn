//! GPU time of the renderer's passes, measured with timestamp queries when the adapter
//! supports them: see [`Renderer::set_gpu_timing`](crate::Renderer::set_gpu_timing) and
//! [`Renderer::gpu_timings`](crate::Renderer::gpu_timings). Results arrive a few frames late
//! (the readback never stalls the pipeline).

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Passes timed per frame at most.
const MAX_PASSES: u32 = 12;
/// Frames whose timestamps can be in flight at once.
const RING: usize = 4;

const IDLE: u8 = 0;
const MAPPING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;

/// GPU time of one pass of the last measured frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassTime {
    pub pass: &'static str,
    pub ms: f32,
}

struct Slot {
    buffer: wgpu::Buffer,
    labels: Vec<&'static str>,
    frame: u64,
    state: Arc<AtomicU8>,
}

/// Where one pass writes its timestamps (owns a handle to the query set, so the pass
/// descriptor can borrow it without borrowing the renderer).
pub(crate) struct PassTimestamps {
    query_set: wgpu::QuerySet,
    index: u32,
}

impl PassTimestamps {
    pub fn writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(self.index),
            end_of_pass_write_index: Some(self.index + 1),
        }
    }

    /// The same for a compute pass.
    pub fn compute_writes(&self) -> wgpu::ComputePassTimestampWrites<'_> {
        wgpu::ComputePassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(self.index),
            end_of_pass_write_index: Some(self.index + 1),
        }
    }
}

pub(crate) struct GpuTimer {
    query_set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    period_ns: f32,
    /// This frame's slot, when recording.
    current: Option<usize>,
    labels: Vec<&'static str>,
    next: usize,
    frame: u64,
    latest_frame: u64,
    latest: Vec<PassTime>,
}

impl GpuTimer {
    /// `None` when the device cannot write timestamps.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<GpuTimer> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let count = MAX_PASSES * 2;
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("pass timestamps"), ty: wgpu::QueryType::Timestamp, count });
        let size = count as u64 * wgpu::QUERY_SIZE as u64;
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pass timestamps"),
            size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..RING)
            .map(|_| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("pass timestamps readback"),
                    size,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                labels: Vec::new(),
                frame: 0,
                state: Arc::new(AtomicU8::new(IDLE)),
            })
            .collect();
        Some(GpuTimer {
            query_set,
            resolve,
            slots,
            period_ns: queue.get_timestamp_period(),
            current: None,
            labels: Vec::new(),
            next: 0,
            frame: 0,
            latest_frame: 0,
            latest: Vec::new(),
        })
    }

    /// Starts a frame; it is skipped when its readback buffer is still in flight.
    pub fn begin_frame(&mut self) {
        self.labels.clear();
        self.frame += 1;
        self.current = (self.slots[self.next].state.load(Ordering::Acquire) == IDLE).then_some(self.next);
    }

    /// Timestamp writes for this frame's next pass, named `label`.
    pub fn pass(&mut self, label: &'static str) -> Option<PassTimestamps> {
        self.current?;
        let i = self.labels.len() as u32;
        if i >= MAX_PASSES {
            return None;
        }
        self.labels.push(label);
        Some(PassTimestamps { query_set: self.query_set.clone(), index: 2 * i })
    }

    /// Resolves this frame's timestamps into its readback buffer (before finishing `encoder`).
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(slot) = self.current else { return };
        let n = self.labels.len() as u32 * 2;
        if n == 0 {
            self.current = None;
            return;
        }
        encoder.resolve_query_set(&self.query_set, 0..n, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.slots[slot].buffer, 0, n as u64 * wgpu::QUERY_SIZE as u64);
        let s = &mut self.slots[slot];
        s.labels.clear();
        s.labels.extend_from_slice(&self.labels);
        s.frame = self.frame;
    }

    /// After the frame was submitted: starts reading its timestamps back.
    pub fn after_submit(&mut self) {
        let Some(slot) = self.current.take() else { return };
        let s = &self.slots[slot];
        s.state.store(MAPPING, Ordering::Release);
        let state = s.state.clone();
        s.buffer.map_async(wgpu::MapMode::Read, .., move |r| state.store(if r.is_ok() { READY } else { FAILED }, Ordering::Release));
        self.next = (slot + 1) % RING;
    }

    /// Picks up timestamps that have arrived (never blocks).
    pub fn collect(&mut self, device: &wgpu::Device) {
        let _ = device.poll(wgpu::PollType::Poll);
        for s in &mut self.slots {
            match s.state.load(Ordering::Acquire) {
                READY => {
                    if s.frame > self.latest_frame && let Ok(data) = s.buffer.slice(..).get_mapped_range() {
                        let ticks: Vec<u64> = data[..s.labels.len() * 16].as_chunks::<8>().0.iter().map(|&b| u64::from_le_bytes(b)).collect();
                        let ms = |from: u64, to: u64| to.saturating_sub(from) as f32 * self.period_ns * 1e-6;
                        self.latest = s.labels.iter().zip(ticks.as_chunks::<2>().0).map(|(&pass, t)| PassTime { pass, ms: ms(t[0], t[1]) }).collect();
                        // The frame from its first pass to the end of bloom (the composite may
                        // wait for the swapchain image): includes what happens between passes
                        // (the MSAA resolve lands in either neighbour or in neither).
                        let end = s.labels.iter().position(|&l| l == "bloom").map_or(ticks.len() - 1, |i| 2 * i + 1);
                        if let (Some(&first), Some(&last)) = (ticks.first(), ticks.get(end)) {
                            self.latest.push(PassTime { pass: "frame", ms: ms(first, last) });
                        }
                        // Everything, composite included (it may also wait for the swapchain).
                        if let (Some(&first), Some(&last)) = (ticks.first(), ticks.last()) {
                            self.latest.push(PassTime { pass: "full", ms: ms(first, last) });
                        }
                        self.latest_frame = s.frame;
                    }
                    s.buffer.unmap();
                    s.state.store(IDLE, Ordering::Release);
                }
                FAILED => s.state.store(IDLE, Ordering::Release),
                _ => {}
            }
        }
    }

    /// The newest frame's pass times.
    pub fn latest(&self) -> &[PassTime] {
        &self.latest
    }

    /// Which frame they are of (0: none yet).
    pub fn latest_frame(&self) -> u64 {
        self.latest_frame
    }
}
