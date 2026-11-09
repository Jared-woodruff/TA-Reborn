//! Wakes behind ships and hovercraft (see `shaders/wake.wgsl`): the crest of foam a moving hull
//! pushes up at its bow, the two arms of the wake spreading behind it at the Kelvin angle as
//! waves that catch the light, and down its middle the churned water its screws and skirts
//! leave, white, then a pale band and a slick in which the wind's ripples are smoothed away,
//! fading over seconds; all scaled by how fast the hull went and how big it is.
//!
//! The game records the way each ship and hovercraft on the water came (its middle, heading,
//! speed and how far it had come, every few units, see [`WakePoint`]). Each frame a ribbon is
//! laid along that way from a little ahead of its bow, wide enough for its arms, and drawn into
//! the water's wake map ([`WaterScene`]'s, half the scene's size): every texel works out what the
//! wake is there from where on the ribbon it lies (how far across the way, how far behind the
//! bow, how long ago the bow passed, how fast it went then). The water reads the map
//! (water.wgsl): its foam, the slope of the hulls' waves and the slick. So the wakes cost a
//! ribbon of a hundred triangles or so a hull, and one look-up a water pixel, however many ships
//! there are.
//!
//! [`WaterScene`]: crate::water::WaterScene

use bytemuck::{Pod, Zeroable};

/// How long (seconds) a wake lasts: the game keeps a way's points this long (the shader fades
/// the wake out over its last third).
pub const WAKE_LIFE: f32 = 14.0;

/// tan(19.47°): a wake's arms open at the Kelvin angle (as in wake.wgsl).
const KELVIN: f32 = 0.354;

/// A point on the way a ship or hovercraft came: where its middle was (world xz), its heading
/// then (radians, as the units': forward is (sin, cos)), how long ago (seconds), how fast it went
/// (world units a second) and how far it had come along its way (world units: what lies on the
/// water there stays put as the hull goes on).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WakePoint {
    pub pos: [f32; 2],
    pub heading: f32,
    pub age: f32,
    pub speed: f32,
    pub travelled: f32,
}

/// A wake: `count` points from `first` of the list given with it, the newest (where the hull is
/// now) first, and the hull making it: how far its bow lies ahead of its middle and its stern
/// behind (world units), its half-width, and whether it rides a skirt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WakeTrail {
    pub first: u32,
    pub count: u32,
    pub bow: f32,
    pub stern: f32,
    pub beam: f32,
    pub hover: bool,
}

/// A corner of a wake's ribbon (as wake.wgsl reads it).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
struct WakeVertex {
    /// World xz, and the unit vector across the way (towards positive `shape.x`).
    pos_side: [f32; 4],
    /// x = across the way (world units, signed: the ribbon's edge here), y = how far behind the
    /// bow along the way (negative ahead of it), z = how long ago the bow passed (s), w = how
    /// fast it went then (world units a second).
    shape: [f32; 4],
    /// x = how far the hull had come along its way here (world units), y = its length, z = its
    /// half-width, w = 1 for a hovercraft.
    hull: [f32; 4],
}

/// The format of the water's wake map (rg the waves' slope, b the slick, a foam).
pub(crate) const WAKE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub(crate) struct WakeRenderer {
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    /// Scratch for this frame's ribbons.
    scratch_vertices: Vec<WakeVertex>,
    scratch_indices: Vec<u32>,
}

/// The wake shader: noise (the streaks in the churn) and the wake's entry points; `common.wgsl`
/// is prepended by [`crate::util::shader`].
pub(crate) fn shader_source() -> String {
    format!("{}\n{}", include_str!("shaders/noise.wgsl"), include_str!("shaders/wake.wgsl"))
}

impl WakeRenderer {
    pub fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout) -> Self {
        let shader = crate::util::shader(device, "wakes", &shader_source());
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("wakes"), bind_group_layouts: &[Some(frame_layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wakes"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_wake"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<WakeVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_wake"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: WAKE_FORMAT,
                    // Where wakes cross, their waves and slicks add up; the foam is the most of
                    // theirs.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Max },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        WakeRenderer {
            pipeline,
            vertices: Self::buffer(device, "wake vertices", 1024 * std::mem::size_of::<WakeVertex>(), wgpu::BufferUsages::VERTEX),
            indices: Self::buffer(device, "wake indices", 3072 * 4, wgpu::BufferUsages::INDEX),
            count: 0,
            scratch_vertices: Vec::new(),
            scratch_indices: Vec::new(),
        }
    }

    fn buffer(device: &wgpu::Device, label: &str, bytes: usize, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: bytes as u64, usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
    }

    /// Lays this frame's ribbons along `trails` (their points in `points`) and uploads them.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, points: &[WakePoint], trails: &[WakeTrail]) {
        let (mut vertices, mut indices) = (std::mem::take(&mut self.scratch_vertices), std::mem::take(&mut self.scratch_indices));
        vertices.clear();
        indices.clear();
        for t in trails {
            let Some(way) = points.get(t.first as usize..(t.first + t.count) as usize) else { continue };
            ribbon(t, way, &mut vertices, &mut indices);
        }
        self.count = indices.len() as u32;
        if !indices.is_empty() {
            let (vb, ib) = (std::mem::size_of_val(vertices.as_slice()), std::mem::size_of_val(indices.as_slice()));
            if vb as u64 > self.vertices.size() {
                self.vertices = Self::buffer(device, "wake vertices", vb.next_power_of_two(), wgpu::BufferUsages::VERTEX);
            }
            if ib as u64 > self.indices.size() {
                self.indices = Self::buffer(device, "wake indices", ib.next_power_of_two(), wgpu::BufferUsages::INDEX);
            }
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&vertices));
            queue.write_buffer(&self.indices, 0, bytemuck::cast_slice(&indices));
        }
        self.scratch_vertices = vertices;
        self.scratch_indices = indices;
    }

    /// Draws the wakes into the wake map `target` (cleared first, also when there are none).
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, frame: &wgpu::BindGroup) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("wakes"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, frame, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.count, 0, 0..1);
    }
}

/// How far out from the middle of its way the wake of hull `t` can reach at a point `age`
/// seconds old, made at `speed` (as wake.wgsl draws it: its arms open at the Kelvin angle,
/// broaden as they go and stagger in echelon; the churn down the middle spreads slowly).
fn reach(t: &WakeTrail, age: f32, speed: f32) -> f32 {
    let size = (t.beam / 10.0).clamp(0.6, 2.5);
    let spread = age * speed * KELVIN;
    let crest = (1.6 + 0.08 * spread) * size;
    let echelon = 0.3 * (0.35 * (t.bow + t.stern)).max(8.0);
    let arms = t.beam + spread + 3.0 * crest + echelon;
    let churn = t.beam * (1.1 + 0.07 * age).min(1.8) * 1.2;
    arms.max(churn) + 2.0
}

impl WakeTrail {
    /// Conservative half-width of the rendered ribbon, used by the client's visibility
    /// bounds as well. Mature fast wakes extend much farther than a fixed culling margin.
    pub fn reach(&self, age: f32, speed: f32) -> f32 {
        reach(self, age, speed)
    }
}

/// The ribbon along one wake's way: two corners at each point of the way its middle came, and
/// ahead of it along its heading at its bow and a little beyond, as wide as the wake reaches
/// there, but on the inside of a turn no wider than the turn is tight (the ribbon would fold over
/// itself). (The way of its middle, not its bow's: a hull turning swings its bow and stern out
/// either side of it.) Its corners say how far behind the bow they lie and how long ago the bow
/// passed there.
fn ribbon(t: &WakeTrail, way: &[WakePoint], vertices: &mut Vec<WakeVertex>, indices: &mut Vec<u32>) {
    if way.is_empty() {
        return;
    }
    let forward = |h: f32| glam::Vec2::new(h.sin(), h.cos());
    let points: Vec<glam::Vec2> = way.iter().map(|p| glam::Vec2::from(p.pos)).collect();
    let n = points.len();
    // The way's direction at each point (forwards), from its neighbours; the heading where it
    // stood still.
    let dirs: Vec<glam::Vec2> = (0..n)
        .map(|i| {
            let (a, b) = (points[i.saturating_sub(1)], points[(i + 1).min(n - 1)]);
            (a - b).try_normalize().unwrap_or_else(|| forward(way[i].heading))
        })
        .collect();
    let length = t.bow + t.stern;
    let hull = [0.0, length, t.beam, if t.hover { 1.0 } else { 0.0 }];
    let base = vertices.len() as u32;
    let mut corners = |at: glam::Vec2, dir: glam::Vec2, s: f32, age: f32, speed: f32, travelled: f32, left: f32, right: f32| {
        let side = glam::Vec2::new(dir.y, -dir.x);
        for (edge, w) in [(-1.0, left), (1.0, right)] {
            let c = at + side * (edge * w);
            vertices.push(WakeVertex { pos_side: [c.x, c.y, side.x, side.y], shape: [edge * w, s, age, speed], hull: [travelled, hull[1], hull[2], hull[3]] });
        }
    };
    // Ahead of its middle along its heading: its bow, and a little beyond (the water it pushes
    // up there).
    let (head, ahead) = (way[0], forward(way[0].heading));
    let lead = (t.beam * 0.8).max(4.0);
    let w = t.beam * 1.5 + 4.0;
    corners(points[0] + ahead * (t.bow + lead), ahead, -lead, 0.0, head.speed, head.travelled + t.bow + lead, w, w);
    corners(points[0] + ahead * t.bow, ahead, 0.0, 0.0, head.speed, head.travelled + t.bow, w, w);
    let mut s = t.bow;
    for i in 0..n {
        if i > 0 {
            s += points[i].distance(points[i - 1]);
        }
        // (The bow passed here before its middle did.)
        let age = way[i].age + t.bow / way[i].speed.max(10.0);
        let w = reach(t, age, way[i].speed);
        // How tight the way turns here: the radius of the circle through this point and its
        // neighbours, whose middle lies on the inside of the turn.
        let (mut left, mut right) = (w, w);
        if i > 0 && i + 1 < n {
            let (a, b, c) = (points[i - 1], points[i], points[i + 1]);
            let twice_area = (a - b).perp_dot(c - b).abs();
            if twice_area > 1e-4 {
                let radius = a.distance(b) * b.distance(c) * c.distance(a) / (2.0 * twice_area);
                let inner = (radius * 0.9).max(t.beam * 1.1).min(w);
                let side = glam::Vec2::new(dirs[i].y, -dirs[i].x);
                if ((a + c) * 0.5 - b).dot(side) > 0.0 {
                    right = inner;
                } else {
                    left = inner;
                }
            }
        }
        corners(points[i], dirs[i], s, age, way[i].speed, way[i].travelled, left, right);
    }
    for k in 0..=n as u32 {
        let v = base + 2 * k;
        indices.extend_from_slice(&[v, v + 1, v + 2, v + 1, v + 3, v + 2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ribbon_runs_along_the_way_behind_the_bow() {
        // A hull 40 long (its bow 24 ahead of its middle) going north (+z) at 60 units a second,
        // its points 10 units apart.
        let way: Vec<WakePoint> = (0..5).map(|i| WakePoint { pos: [100.0, 200.0 - 10.0 * i as f32], heading: 0.0, age: i as f32 / 6.0, speed: 60.0, travelled: 500.0 - 10.0 * i as f32 }).collect();
        let t = WakeTrail { first: 0, count: 5, bow: 24.0, stern: 16.0, beam: 8.0, hover: false };
        let (mut v, mut idx) = (Vec::new(), Vec::new());
        ribbon(&t, &way, &mut v, &mut idx);
        // Two corners a point, at the bow and a little ahead of it; two triangles between each
        // pair.
        assert_eq!(v.len(), 14);
        assert_eq!(idx.len(), 36);
        // Ahead of the bow (at 224), then the bow, then the hull's middle a bow's length back.
        assert!((v[0].pos_side[1] - (224.0 + 6.4)).abs() < 1e-3, "{:?}", v[0]);
        assert!((v[2].pos_side[1] - 224.0).abs() < 1e-3 && v[2].shape[1] == 0.0);
        assert!((v[4].pos_side[1] - 200.0).abs() < 1e-3 && (v[4].shape[1] - 24.0).abs() < 1e-3);
        // The bow passed the middle's place 24 units (0.4 s) before it did; where the wake lies
        // is fixed by how far the hull had come.
        assert!((v[4].shape[2] - 0.4).abs() < 1e-3);
        assert!((v[0].hull[0] - 530.4).abs() < 1e-3 && v[4].hull[0] == 500.0 && v[12].hull[0] == 460.0);
        // Behind it the points lie further back along the way, the ribbon wider as the wake
        // spreads, its corners either side of the way.
        assert!((v[12].shape[1] - 64.0).abs() < 1e-3);
        assert!(v[12].shape[0] < 0.0 && v[13].shape[0] > 0.0);
        assert!(v[13].shape[0] > v[5].shape[0]);
        assert!((v[12].pos_side[0] - 100.0 - v[12].shape[0] * v[12].pos_side[2]).abs() < 1e-3);
    }

    #[test]
    fn a_tight_turn_keeps_the_ribbon_from_folding_on_its_inside() {
        // A hull going round a circle of radius 30 at 100 units a second: its wake would reach
        // far wider than the turn.
        let way: Vec<WakePoint> = (0..20)
            .map(|i| {
                let a = -(i as f32) * 0.1;
                WakePoint { pos: [30.0 * a.cos(), 30.0 * a.sin()], heading: std::f32::consts::PI - a, age: i as f32 * 0.03, speed: 100.0, travelled: 0.0 }
            })
            .collect();
        let t = WakeTrail { first: 0, count: 20, bow: 2.0, stern: 10.0, beam: 5.0, hover: false };
        let (mut v, mut idx) = (Vec::new(), Vec::new());
        ribbon(&t, &way, &mut v, &mut idx);
        // (Its two ends have one neighbour each, and no measure of the turn.)
        for k in 1..19 {
            let (a, b) = (v[2 * k + 4].shape[0].abs(), v[2 * k + 5].shape[0].abs());
            assert!(a.min(b) <= 27.0 + 1e-3, "point {k}: {a} {b}");
        }
    }
}
