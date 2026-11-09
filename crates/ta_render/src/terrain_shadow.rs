//! The ground's heights and the shadows the relief casts, in one map every pass can read (the
//! frame bind group's `ground_map`): r = the ground height, g = how far above the ground the
//! light (the sun, or the moon at night) clears the hills between that point and the light,
//! b = how rich the metal deposit there is (see [`TerrainShadow::set_ore`]), a = how much of the
//! ground around lies under the sea (on lava worlds: how much melt glows nearby, see
//! [`lava_nearness`]).
//! A compute pass fills it when a map loads and, with the perspective camera, whenever the
//! light has moved; every lit surface compares its own height above the ground with the
//! clearance (`terrain_sunlight` in common.wgsl), so ridges shade the valleys and the units,
//! trees and water in them, and a low sun throws long shadows across the land.

use glam::{Vec2, Vec3};

use crate::util;

/// World units per texel of the map (half a heightmap cell).
const TEXEL: f32 = 8.0;
/// Most texels along a side (the largest maps get coarser texels).
const MAX_SIDE: u32 = 2048;
/// How far the light must turn (as the change of its unit direction, about half a degree)
/// before the map is filled again.
const REFRESH: f32 = 0.009;
/// Lowest rise of the light ray per unit it travels (the march stays bounded under a setting
/// sun; the shadows are endless by then anyway).
const MIN_TAN: f32 = 0.03;

/// Horizontal travel per unit of height, with the same low-sun bound as the march. Comparing
/// directions alone permits huge shadow jumps at dawn: near the horizon, a small elevation
/// change moves the end of a tall hill's shadow by hundreds of world units.
fn shadow_projection(light: Vec3) -> Vec2 {
    let flat = Vec2::new(light.x, light.z);
    let across = flat.length();
    if across <= 1e-3 { Vec2::ZERO } else { flat / light.y.max(across * MIN_TAN) }
}

fn light_needs_refresh(previous: Vec3, light: Vec3, height: f32, texel: f32) -> bool {
    previous.distance(light) >= REFRESH || shadow_projection(previous).distance(shadow_projection(light)) * height >= texel
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    size: [u32; 2],
    cells: [u32; 2],
    dir: [f32; 2],
    tan_elevation: f32,
    texel: f32,
    max_height: f32,
    _pad: f32,
    /// The first texel filled (the corner of the part of the map filled again).
    origin: [u32; 2],
}

pub struct TerrainShadow {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// r = ground height / 255, g = clearance / 255 (world units), b = ore; a flat, lit 1×1
    /// texel before a map is loaded.
    pub target: util::Target,
    bind_group: Option<wgpu::BindGroup>,
    /// The heightmap the map is made from, the ore per heightmap cell (0..1; 1×1 none) and how
    /// much sea lies around each cell (0..1; 1×1 none, see [`lava_nearness`]).
    heights: Option<wgpu::TextureView>,
    ore: util::Target,
    sea_near: util::Target,
    size: (u32, u32),
    texel: f32,
    cells: (u32, u32),
    max_height: f32,
    /// The light the map was last filled for (`None`: not filled yet).
    done_for: Option<Vec3>,
    /// Ground whose heights changed since (world x0, z0, x1, z1): that part of the map is filled
    /// again (see [`TerrainShadow::heights_changed_in`]).
    pending: Option<[f32; 4]>,
}

impl TerrainShadow {
    pub fn new(device: &wgpu::Device) -> Self {
        use wgpu::ShaderStages as S;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain shadow"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: S::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: S::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: S::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: S::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: S::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: S::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let shader = util::shader_standalone(device, "terrain shadow", include_str!("shaders/terrain_shadow.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("terrain shadow"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("terrain shadow"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain shadow params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terrain shadow heights"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        TerrainShadow {
            pipeline,
            layout,
            params,
            sampler,
            target: Self::texture(device, 1, 1),
            bind_group: None,
            heights: None,
            ore: Self::ore_texture(device, 1, 1),
            sea_near: Self::ore_texture(device, 1, 1),
            size: (1, 1),
            texel: TEXEL,
            cells: (1, 1),
            max_height: 0.0,
            done_for: None,
            pending: None,
        }
    }

    /// An R8 map per heightmap cell (ore, the sea around).
    fn ore_texture(device: &wgpu::Device, width: u32, height: u32) -> util::Target {
        util::Target::new(device, "ground cells", width, height, wgpu::TextureFormat::R8Unorm, 1, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST)
    }

    fn texture(device: &wgpu::Device, width: u32, height: u32) -> util::Target {
        // (New textures start zeroed: clear of every hill, so lit until filled.)
        util::Target::new(
            device,
            "terrain shadow",
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        )
    }

    /// Makes the map for a new terrain: its heightmap (r = height / 255 per vertex), vertices
    /// per axis and highest ground. The next [`TerrainShadow::update`] fills it.
    pub fn set_map(&mut self, device: &wgpu::Device, heights: &wgpu::TextureView, cells: (u32, u32), max_height: f32) {
        // (Nor any sea, until it is given.)
        self.sea_near = Self::ore_texture(device, 1, 1);
        let extent = (cells.0.max(1) as f32 * 16.0, cells.1.max(1) as f32 * 16.0);
        let texel = TEXEL.max(extent.0.max(extent.1) / MAX_SIDE as f32);
        let size = ((extent.0 / texel).ceil().max(1.0) as u32, (extent.1 / texel).ceil().max(1.0) as u32);
        self.target = Self::texture(device, size.0, size.1);
        self.size = size;
        self.texel = texel;
        self.cells = cells;
        self.max_height = max_height;
        self.heights = Some(heights.clone());
        // (A new map has no ore until it is given.)
        self.ore = Self::ore_texture(device, 1, 1);
        self.rebind(device);
    }

    /// The metal deposits of the map: how rich each heightmap cell is (0 = plain ground, 255 =
    /// the richest), `width` × `height` cells row by row. They glint in the rock (terrain.wgsl).
    pub fn set_ore(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32, ore: &[u8]) {
        if width == 0 || height == 0 || ore.len() < (width * height) as usize {
            return;
        }
        self.ore = Self::ore_texture(device, width, height);
        util::write_texture_level(queue, &self.ore.texture, 0, width, height, &ore[..(width * height) as usize], 1);
        self.rebind(device);
    }

    /// How much of the ground around each heightmap cell lies under the sea (see
    /// [`lava_nearness`]), `width` × `height` cells row by row.
    pub fn set_sea_near(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32, near: &[u8]) {
        if width == 0 || height == 0 || near.len() < (width * height) as usize {
            return;
        }
        self.sea_near = Self::ore_texture(device, width, height);
        util::write_texture_level(queue, &self.sea_near.texture, 0, width, height, &near[..(width * height) as usize], 1);
        self.rebind(device);
    }

    /// Whether the next [`TerrainShadow::update`] fills the map (the frame doing it is checked:
    /// see [`TerrainShadow::forget`]).
    pub(crate) fn will_fill(&self) -> bool {
        self.bind_group.is_some() && (self.done_for.is_none() || self.pending.is_some())
    }

    /// The frame that filled the map failed: the next fills it again.
    pub(crate) fn forget(&mut self) {
        self.done_for = None;
    }

    /// The ground's heights were changed in place (the map editor): the next
    /// [`TerrainShadow::update`] fills the map again. `top` is the highest drawn vertex after
    /// the edit: retaining the old maximum made the march stop below newly raised hills.
    pub fn heights_changed(&mut self, top: f32) {
        self.max_height = top;
        self.done_for = None;
    }

    /// The ground's heights changed within `area` (world x0, z0, x1, z1; a building's pad), the
    /// highest of them now `top`: the next [`TerrainShadow::update`] fills that part of the map
    /// again, and as far beyond it as a shadow of what is there may reach.
    pub fn heights_changed_in(&mut self, area: [f32; 4], top: f32) {
        self.max_height = self.max_height.max(top);
        if self.done_for.is_none() {
            return;
        }
        self.pending = Some(match self.pending {
            Some(a) => [a[0].min(area[0]), a[1].min(area[1]), a[2].max(area[2]), a[3].max(area[3])],
            None => area,
        });
    }

    fn rebind(&mut self, device: &wgpu::Device) {
        let Some(heights) = &self.heights else { return };
        self.done_for = None;
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain shadow"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(heights) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.target.view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&self.ore.view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&self.sea_near.view) },
            ],
        }));
    }

    /// Fills the map for a new terrain, and when `follow` is set again for the light
    /// direction `light` (towards it) once it has moved.
    pub fn update(&mut self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, light: Vec3, follow: bool) {
        let Some(group) = &self.bind_group else { return };
        let light = light.normalize_or_zero();
        let whole = self.done_for.is_none_or(|d| follow && light_needs_refresh(d, light, self.max_height, self.texel));
        // (A part of it is filled again for the light the rest was filled for.)
        let (light, part) = match (whole, self.done_for, self.pending) {
            (false, Some(done), Some(pending)) => (done, Some(pending)),
            (false, Some(_), None) => return,
            _ => (light, None),
        };
        self.pending = None;
        let flat = glam::Vec2::new(light.x, light.z);
        let across = flat.length();
        // (The light straight overhead casts no shadow worth drawing: the march is skipped.)
        let (dir, tan) = if across > 1e-3 { (flat / across, (light.y / across).max(MIN_TAN)) } else { (glam::Vec2::X, 1.0e4) };
        // The texels filled: the whole map, or the part whose heights changed and the ground
        // beyond it, away from the light, that it may shade: as far as the highest ground's
        // shadow reaches (a shadow lengthened by a pad on a hill ends where the hill's did, far
        // further off than the pad's own rise throws one).
        let (origin, extent) = match part {
            None => ([0, 0], [self.size.0, self.size.1]),
            Some(area) => {
                let reach = (self.max_height / tan).min(8192.0);
                let margin = self.texel * 2.0;
                let (x0, x1) = (area[0] - margin - (reach * dir.x).max(0.0), area[2] + margin + (-reach * dir.x).max(0.0));
                let (z0, z1) = (area[1] - margin - (reach * dir.y).max(0.0), area[3] + margin + (-reach * dir.y).max(0.0));
                let texel = |v: f32, size: u32| ((v / self.texel).floor().max(0.0) as u32).min(size);
                let (tx0, tz0) = (texel(x0, self.size.0), texel(z0, self.size.1));
                let (tx1, tz1) = (texel(x1, self.size.0) + 1, texel(z1, self.size.1) + 1);
                ([tx0, tz0], [tx1.min(self.size.0).saturating_sub(tx0), tz1.min(self.size.1).saturating_sub(tz0)])
            }
        };
        if extent[0] == 0 || extent[1] == 0 {
            return;
        }
        let params = Params {
            size: [self.size.0, self.size.1],
            cells: [self.cells.0, self.cells.1],
            dir: dir.to_array(),
            tan_elevation: tan,
            texel: self.texel,
            max_height: self.max_height,
            _pad: 0.0,
            origin,
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("terrain shadow"), timestamp_writes: None });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.dispatch_workgroups(extent[0].div_ceil(8), extent[1].div_ceil(8), 1);
        if part.is_none() {
            self.done_for = Some(light);
        }
    }

    /// For the frame uniform: x = `strength` (0 = off), yz = 1 / the map's world extent.
    pub fn uniform(&self, strength: f32) -> [f32; 4] {
        let extent = (self.size.0 as f32 * self.texel, self.size.1 as f32 * self.texel);
        [strength, 1.0 / extent.0.max(1.0), 1.0 / extent.1.max(1.0), 0.0]
    }
}

/// Cells the sea's nearness spreads over (16 world units each): the melt's glow lights what
/// stands within about this far of a lava sea's shore.
const SEA_REACH: i32 = 8;

/// How much of the ground round each of the `width` × `height` heightmap cells lies under the
/// sea `sea` (0..255): the cells under it, blurred over [`SEA_REACH`] cells either way (two box
/// blurs, about a Gaussian) and doubled, so the shore itself counts in full and the nearness
/// fades over the ground beyond. On lava worlds the melt glows on what stands near it by this
/// (`lava_light` in common.wgsl).
pub fn lava_nearness(width: u32, height: u32, heights: &[u8], sea: f32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || heights.len() < w * h {
        return Vec::new();
    }
    let mut v: Vec<f32> = heights[..w * h].iter().map(|&g| if (g as f32) < sea - 1.0 { 1.0 } else { 0.0 }).collect();
    // A box blur along each axis, twice.
    let blur = |v: &mut Vec<f32>, stride: usize, len: usize, lines: usize, step: usize| {
        let mut line = vec![0.0f32; len];
        for l in 0..lines {
            let at = |i: usize| l * step + i * stride;
            let mut sum = 0.0;
            // Clamped at the edges.
            let get = |v: &Vec<f32>, i: i32| v[at(i.clamp(0, len as i32 - 1) as usize)];
            for i in -SEA_REACH..=SEA_REACH {
                sum += get(v, i);
            }
            for (i, out) in line.iter_mut().enumerate() {
                *out = sum / (2 * SEA_REACH + 1) as f32;
                let i = i as i32;
                sum += get(v, i + SEA_REACH + 1) - get(v, i - SEA_REACH);
            }
            for (i, &x) in line.iter().enumerate() {
                v[at(i)] = x;
            }
        }
    };
    for _ in 0..2 {
        blur(&mut v, 1, w, h, w);
        blur(&mut v, w, h, w, 1);
    }
    v.iter().map(|&x| ((x * 2.0).min(1.0) * 255.0 + 0.5) as u8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_sun_refresh_bounds_the_motion_of_long_shadows() {
        let direction = |rise: f32| Vec3::new(1.0, rise, 0.0).normalize();
        let (before, after) = (direction(0.05), direction(0.055));
        assert!(before.distance(after) < REFRESH, "the old angular threshold misses this change");
        let shift = shadow_projection(before).distance(shadow_projection(after)) * 200.0;
        assert!(shift > 350.0, "a tall hill's shadow would jump {shift} world units");
        assert!(light_needs_refresh(before, after, 200.0, TEXEL));
        assert!(!light_needs_refresh(before, before, 200.0, TEXEL));
        assert!(!light_needs_refresh(direction(1.0), direction(1.001), 200.0, TEXEL), "subtexel motion can keep the cached map");
        assert_eq!(shadow_projection(Vec3::Y), Vec2::ZERO);
        assert!(shadow_projection(direction(-0.1)).is_finite(), "the march's minimum rise also bounds the refresh calculation");
    }

    /// Exercise the actual compute pass and invalidation, without loading any game assets.
    #[test]
    #[ignore = "requires a GPU; run with --ignored"]
    fn edited_hills_update_shadow_clearance_on_gpu() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or_else(crate::gpu::native_backends),
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let heights = util::Target::new(&device, "editable heights", 32, 8, wgpu::TextureFormat::Rgba8Unorm, 1, wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING);
        let upload = |raised: bool| {
            let pixels: Vec<u8> = (0..32 * 8).flat_map(|i| [if raised && (16..19).contains(&(i % 32)) { 140 } else { 20 }, 0, 0, 0]).collect();
            util::write_texture_level(&queue, &heights.texture, 0, 32, 8, &pixels, 4);
        };
        upload(false);
        let mut shadows = TerrainShadow::new(&device);
        shadows.set_map(&device, &heights.view, (32, 8), 20.0);
        // A readback-capable twin of the production target, with the same dimensions/format.
        shadows.target = util::Target::new(&device, "shadow regression", 64, 16, wgpu::TextureFormat::Rgba8Unorm, 1,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        shadows.rebind(&device);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("shadow readback"), size: 256 * 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let light = Vec3::new(1.0, 0.5, 0.0).normalize();
        let read = |shadows: &mut TerrainShadow| {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            shadows.update(&queue, &mut encoder, light, true);
            encoder.copy_texture_to_buffer(shadows.target.texture.as_image_copy(), wgpu::TexelCopyBufferInfo {
                buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: Some(16) },
            }, wgpu::Extent3d { width: 64, height: 16, depth_or_array_layers: 1 });
            queue.submit([encoder.finish()]);
            buffer.slice(..).map_async(wgpu::MapMode::Read, |result| result.expect("shadow readback"));
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let bytes = buffer.slice(..).get_mapped_range().unwrap();
            // Ground 36 world units before the ridge, on the side away from the sun.
            let pixel = bytes[4 * 256 + 27 * 4..4 * 256 + 27 * 4 + 4].to_vec();
            drop(bytes);
            buffer.unmap();
            pixel
        };
        assert_eq!(read(&mut shadows)[1], 0, "flat ground is lit");
        upload(true);
        shadows.heights_changed(140.0);
        assert!(read(&mut shadows)[1] > 80, "the raised ridge casts a shadow beyond the old maximum height");
        upload(false);
        shadows.heights_changed(20.0);
        assert_eq!(read(&mut shadows)[1], 0, "lowering the hill clears its old shadow");
    }

    /// The sea counts in full at its shore and under it, and fades over the ground beyond: none
    /// a dozen cells inland.
    #[test]
    fn the_sea_is_near_its_shore_and_not_far_inland() {
        // 40 x 8 cells: the west 10 columns under a sea at 30, land at 60 beyond.
        let (w, h) = (40u32, 8u32);
        let heights: Vec<u8> = (0..w * h).map(|i| if i % w < 10 { 10 } else { 60 }).collect();
        let near = lava_nearness(w, h, &heights, 30.0);
        let at = |x: u32| near[(4 * w + x) as usize];
        assert_eq!(at(2), 255, "under the sea");
        assert!(at(10) > 200, "at the shore: {}", at(10));
        assert!(at(16) < at(12) && at(16) > 0, "fading inland: {} then {}", at(12), at(16));
        assert_eq!(at(28), 0, "far inland");
        // No sea, no nearness.
        assert!(lava_nearness(w, h, &heights, 5.0).iter().all(|&v| v == 0));
    }
}
