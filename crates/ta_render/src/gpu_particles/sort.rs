//! GPU draw-order sorting. Only indices and depth keys move; the simulation keeps its compact
//! particle buffers. The main view and its reflection each have their own final order.

use super::{Pool, entry};

pub(super) const SORT_STEPS: usize = 15; // 64-entry blocks, then merges up to MAX_CAPACITY.
const PARAM_STRIDE: u64 = 256;
pub(super) const SORT_INDIRECT: u64 = 80;

pub(super) struct Sorter {
    blocks: wgpu::ComputePipeline,
    merge: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    scratch: [wgpu::Buffer; 2],
    pub order: [wgpu::Buffer; 2],
    groups: Vec<wgpu::BindGroup>,
    reflected: bool,
}

impl Sorter {
    pub fn new(device: &wgpu::Device, pool: &Pool) -> Self {
        let uniform = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(32),
            },
            count: None,
        };
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle sorting"),
            entries: &[uniform, storage(1, true), storage(2, true), storage(3, true), storage(4, false), storage(5, false)],
        });
        let shader = crate::util::shader_standalone(device, "particle sorting", &source());
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("particle sorting"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = |entry_point| device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry_point), layout: Some(&pl), module: &shader, entry_point: Some(entry_point), compilation_options: Default::default(), cache: None,
        });
        let buffer = |label, size| device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false,
        });
        let mut result = Self {
            blocks: pipeline("cs_sort_blocks"),
            merge: pipeline("cs_merge"),
            layout,
            params: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("particle sort parameters"), size: 2 * SORT_STEPS as u64 * PARAM_STRIDE,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
            }),
            scratch: std::array::from_fn(|_| buffer("particle sort keys", u64::from(pool.capacity) * 8)),
            order: std::array::from_fn(|_| buffer("particle draw order", u64::from(pool.capacity) * 4)),
            groups: Vec::new(),
            reflected: false,
        };
        for list in 0..2 {
            for view in 0..2 {
                for output in 0..2 {
                    result.groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("particle sorting"), layout: &result.layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &result.params, offset: 0, size: wgpu::BufferSize::new(32) }) },
                            entry(1, &pool.particles[1 - list]), entry(2, &pool.counters),
                            entry(3, &result.scratch[1 - output]), entry(4, &result.scratch[output]), entry(5, &result.order[view]),
                        ],
                    }));
                }
            }
        }
        result
    }

    pub fn prepare(&mut self, queue: &wgpu::Queue, capacity: u32, list: u32, depth: [f32; 4], reflected: Option<[f32; 4]>) {
        self.reflected = reflected.is_some();
        let views = [depth, reflected.unwrap_or(depth)];
        let mut bytes = [0u8; 2 * SORT_STEPS * PARAM_STRIDE as usize];
        for (view, depth) in views.iter().enumerate().take(1 + usize::from(self.reflected)) {
            for step in 0..SORT_STEPS {
                let at = (view * SORT_STEPS + step) * PARAM_STRIDE as usize;
                bytes[at..at + 16].copy_from_slice(bytemuck::cast_slice(depth));
                bytes[at + 16..at + 32].copy_from_slice(bytemuck::cast_slice(&[capacity, list, 32u32 << step, 0]));
            }
        }
        let used = (1 + usize::from(self.reflected)) * SORT_STEPS * PARAM_STRIDE as usize;
        queue.write_buffer(&self.params, 0, &bytes[..used]);
    }

    pub fn encode(&self, pass: &mut wgpu::ComputePass<'_>, pool: &Pool, list: usize) {
        for view in 0..1 + usize::from(self.reflected) {
            for step in 0..SORT_STEPS {
                pass.set_pipeline(if step == 0 { &self.blocks } else { &self.merge });
                let group = list * 4 + view * 2 + step % 2;
                let offset = ((view * SORT_STEPS + step) as u64 * PARAM_STRIDE) as u32;
                pass.set_bind_group(0, &self.groups[group], &[offset]);
                pass.dispatch_workgroups_indirect(&pool.indirect, SORT_INDIRECT + step as u64 * 16);
            }
        }
    }
}

fn source() -> String {
    format!("{}\n{}", include_str!("../shaders/particle_common.wgsl"), include_str!("../shaders/particle_sort.wgsl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_particles::{GpuCounters, GpuParticle, INDIRECT_SIZE};
    use crate::gpu::CheckedBufferInit;

    #[test]
    fn particle_sort_shader_validates() {
        let source = source();
        let module = wgpu::naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all()).validate(&module).unwrap();
        assert_eq!(32u32 << (SORT_STEPS - 1), super::super::MAX_CAPACITY / 2);
        assert_eq!(std::mem::size_of::<GpuParticle>(), 48);
    }

    /// Runs the actual GPU shader, including workgroup boundaries, ties, a changed compaction
    /// order and the mirror's reversed depth order. Timings exclude readback and submission.
    #[test]
    #[ignore = "requires a GPU; run explicitly for rendering changes"]
    fn gpu_particle_depth_order_survives_compaction() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
        println!("particle sorting GPU: {}", adapter.get_info().name);
        let timestamp = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: if timestamp { wgpu::Features::TIMESTAMP_QUERY } else { wgpu::Features::empty() },
            required_limits: wgpu::Limits { max_storage_buffers_per_shader_stage: 5, ..wgpu::Limits::default() },
            ..Default::default()
        })).expect("GPU device");
        let query = timestamp.then(|| device.create_query_set(&wgpu::QuerySetDescriptor { label: None, ty: wgpu::QueryType::Timestamp, count: 2 }));
        let times = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let timing_read = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        for n in [1u32, 63, 64, 65, 127, 128, 129, 1001, 32_768, 262_144, super::super::MAX_CAPACITY] {
            let capacity = n.max(1024).next_power_of_two();
            let mut pool = Pool::new(&device, capacity);
            let particle = |id: u32| GpuParticle {
                pos: [(id % 13) as f32, (id % 31) as f32, (id % 97) as f32], age: 0.25, vel: [0.0; 3], life: 2.0,
                data: [id, 0, 0, 0],
            };
            let alpha = n / 3;
            let front = n - alpha;
            let mut expected_front: Vec<_> = (0..front).collect();
            let mut expected_back: Vec<_> = (front..n).collect();
            let expected = |ids: &mut Vec<u32>, reflected: bool| {
                ids.sort_by(|a, b| {
                    let pa = particle(*a);
                    let pb = particle(*b);
                    let da = pa.pos[2] - if reflected { pa.pos[1] * 3.0 } else { 0.0 };
                    let db = pb.pos[2] - if reflected { pb.pos[1] * 3.0 } else { 0.0 };
                    db.total_cmp(&da).then_with(|| a.cmp(b))
                });
            };
            for (permuted, covered) in [(false, true), (true, true), (true, false)] {
                let mut particles = vec![particle(0); capacity as usize];
                for i in 0..front {
                    particles[i as usize] = particle(if permuted { front - 1 - i } else { i });
                }
                for i in 0..alpha {
                    particles[(capacity - 1 - i) as usize] = particle(front + if permuted { alpha - 1 - i } else { i });
                }
                pool.particles[1] = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&particles), usage: wgpu::BufferUsages::STORAGE });
                let mut sorter = Sorter::new(&device, &pool);
                let mut counts: GpuCounters = bytemuck::Zeroable::zeroed();
                counts.front[1] = front;
                counts.back[1] = alpha;
                counts.covered = u32::from(covered);
                queue.write_buffer(&pool.counters, 0, bytemuck::bytes_of(&counts));
                let mut indirect = vec![0u32; INDIRECT_SIZE as usize / 4];
                for step in 0..SORT_STEPS {
                    let sorted_front = if covered { front } else { 0 };
                    let groups = if step == 0 || sorted_front.max(alpha) > 32u32 << step { sorted_front.div_ceil(64) + alpha.div_ceil(64) } else { 0 };
                    let at = SORT_INDIRECT as usize / 4 + step * 4;
                    indirect[at..at + 4].copy_from_slice(&[groups, 1, 1, 0]);
                }
                queue.write_buffer(&pool.indirect, 0, bytemuck::cast_slice(&indirect));
                sorter.prepare(&queue, capacity, 0, [0.0, 0.0, 1.0, 0.0], Some([0.0, -3.0, 1.0, 0.0]));
                let read = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: capacity as u64 * 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("particle sort regression"),
                        timestamp_writes: query.as_ref().map(|query_set| wgpu::ComputePassTimestampWrites { query_set, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1) }),
                    });
                    sorter.encode(&mut pass, &pool, 0);
                }
                for view in 0..2 {
                    encoder.copy_buffer_to_buffer(&sorter.order[view], 0, &read, view as u64 * capacity as u64 * 4, capacity as u64 * 4);
                }
                if let Some(query) = &query {
                    encoder.resolve_query_set(query, 0..2, &times, 0);
                    encoder.copy_buffer_to_buffer(&times, 0, &timing_read, 0, 16);
                }
                queue.submit([encoder.finish()]);
                read.map_async(wgpu::MapMode::Read, .., |r| r.expect("particle sort readback"));
                if query.is_some() { timing_read.map_async(wgpu::MapMode::Read, .., |r| r.expect("particle timing readback")); }
                device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                let data = read.get_mapped_range(..).unwrap();
                let indices: &[u32] = bytemuck::cast_slice(&data);
                for view in 0..2 {
                    expected(&mut expected_front, view == 1);
                    expected(&mut expected_back, view == 1);
                    let offset = view * capacity as usize;
                    let actual_front: Vec<_> = (0..front).map(|i| particles[indices[offset + i as usize] as usize].data[0]).collect();
                    let actual_back: Vec<_> = (0..alpha).map(|i| particles[indices[offset + (capacity - 1 - i) as usize] as usize].data[0]).collect();
                    if covered { assert_eq!(actual_front, expected_front, "front: n={n}, permuted={permuted}, view={view}"); }
                    assert_eq!(actual_back, expected_back, "back: n={n}, permuted={permuted}, view={view}");
                }
                drop(data);
                read.unmap();
                if query.is_some() {
                    let data = timing_read.get_mapped_range(..).unwrap();
                    let t: &[u64] = bytemuck::cast_slice(&data);
                    let ms = (t[1] - t[0]) as f64 * queue.get_timestamp_period() as f64 / 1e6;
                    println!("{n} particles, both views, permuted={permuted}, covered={covered}: {ms:.3} ms GPU");
                    drop(data);
                    timing_read.unmap();
                }
            }
        }
    }
}
