//! Ambient occlusion for the perspective camera (the classic view never shows it).
//!
//! TA's map art has its lighting painted in, and the game is seen from high above, so instead
//! of screen-space occlusion (which needs a resolved depth buffer, halos around units against
//! the ground and shimmers as the camera moves) the renderer uses two cheap, stable terms:
//!
//! * **Terrain** ([`terrain_visibility`]): horizon-based occlusion of every heightmap vertex,
//!   baked once per map on the CPU into an R8 texture that the terrain shader samples
//!   bilinearly. Creases, valley floors and the foot of cliffs darken; ridges, slopes and flat
//!   ground stay as painted. The shader lets it act less on art that is already painted dark
//!   (see `terrain.wgsl`), so painted shadows are not darkened twice.
//! * **Contact** ([`OcclusionMap`]): every frame a soft footprint of each unit, building, wreck
//!   and tree is drawn top-down into a world-space map around the view (1024², multiplied
//!   together where they overlap, so gaps between buildings darken more than open ground).
//!   The ground, decals, and the lower parts of models and trees near the ground read it.
//!   A model's footprint is its lower half seen from above, rasterized once when the model is
//!   loaded into a 32² coverage mask (walls outline it and enclosed interiors are filled, so a
//!   cross-shaped lab darkens a cross, not its bounding box, and the open ground between a
//!   plant's ramps stays open), blurred by the falloff, and kept in an atlas; it turns with the
//!   unit and fades out with height above the ground, so aircraft in flight cast none. Trees
//!   and rocks get a round footprint.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};

use crate::util::Target;

// ---- Terrain: horizon-based occlusion baked from the heightmap ----------------------------

/// Directions searched for the horizon (heightmap cells per step).
const DIRECTIONS: [(i32, i32); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
/// Horizon samples along each direction, in steps.
const STEPS: [i32; 9] = [1, 2, 3, 4, 5, 7, 9, 12, 15];
/// Distance (heightmap cells) beyond which terrain no longer occludes; nearer terrain counts
/// more (a local crease matters, a mountain range on the horizon hardly).
const RADIUS: f32 = 16.0;

/// Sky visibility of every heightmap vertex of a `width` x `height` map (heights in world
/// units, cells 16 units apart), 255 = the whole sky, row-major.
///
/// Along eight directions, the highest horizon above the vertex's own tangent plane (so slopes
/// and flat ground are unoccluded, creases and the foot of cliffs are), weighted down with
/// distance; each direction hides the cosine-weighted share sin² of its horizon angle. The
/// result is blurred a little (1-2-1) to hide the eight-direction pattern.
pub fn terrain_visibility(width: u32, height: u32, heights: &[u8]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || heights.len() < w * h {
        return vec![255; w * h];
    }
    // Angle tables by height difference (-255..=255): per direction length (straight,
    // diagonal) and step for the horizon, and for the tangent (central difference).
    let lengths = [1.0f32, std::f32::consts::SQRT_2];
    let mut horizon = vec![0f32; 2 * STEPS.len() * 511];
    let mut weight = [[0f32; STEPS.len()]; 2];
    let mut tangent = vec![0f32; 2 * 511];
    for (diag, len) in lengths.iter().enumerate() {
        for (k, &s) in STEPS.iter().enumerate() {
            let r = s as f32 * len;
            weight[diag][k] = (1.0 - (r / RADIUS).powi(2)).max(0.0);
            for dh in -255i32..=255 {
                horizon[(diag * STEPS.len() + k) * 511 + (dh + 255) as usize] = (dh as f32 / (r * 16.0)).atan();
            }
        }
        for dh in -255i32..=255 {
            tangent[diag * 511 + (dh + 255) as usize] = (dh as f32 / (2.0 * len * 16.0)).atan();
        }
    }
    let at = |x: i32, z: i32| heights[z.clamp(0, h as i32 - 1) as usize * w + x.clamp(0, w as i32 - 1) as usize] as i32;
    let vertex = |x: i32, z: i32| -> f32 {
        let h0 = at(x, z);
        let mut hidden = 0.0;
        for &(dx, dz) in &DIRECTIONS {
            let diag = (dx != 0 && dz != 0) as usize;
            let t = tangent[diag * 511 + (at(x + dx, z + dz) - at(x - dx, z - dz) + 255) as usize];
            let mut elevation = 0f32;
            for (k, &s) in STEPS.iter().enumerate() {
                let wk = weight[diag][k];
                if wk <= 0.0 {
                    break;
                }
                let a = horizon[(diag * STEPS.len() + k) * 511 + (at(x + dx * s, z + dz * s) - h0 + 255) as usize];
                elevation = elevation.max((a - t) * wk);
            }
            hidden += elevation.sin().powi(2);
        }
        1.0 - hidden / DIRECTIONS.len() as f32
    };
    let mut raw = vec![0f32; w * h];
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 16);
    let rows = h.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (i, chunk) in raw.chunks_mut(rows * w).enumerate() {
            let vertex = &vertex;
            scope.spawn(move || {
                for (j, row) in chunk.chunks_mut(w).enumerate() {
                    let z = (i * rows + j) as i32;
                    for (x, v) in row.iter_mut().enumerate() {
                        *v = vertex(x as i32, z);
                    }
                }
            });
        }
    });
    // 1-2-1 blur, separable, clamped at the edges.
    let blur = |src: &[f32], step: (usize, usize)| -> Vec<f32> {
        let mut dst = vec![0f32; w * h];
        for z in 0..h {
            for x in 0..w {
                let (px, pz) = (x.saturating_sub(step.0), z.saturating_sub(step.1));
                let (nx, nz) = ((x + step.0).min(w - 1), (z + step.1).min(h - 1));
                dst[z * w + x] = (src[pz * w + px] + 2.0 * src[z * w + x] + src[nz * w + nx]) * 0.25;
            }
        }
        dst
    };
    let blurred = blur(&blur(&raw, (1, 0)), (0, 1));
    blurred.iter().map(|&v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()
}

// ---- Contact occlusion: soft footprints drawn into a map around the view ------------------

/// One soft footprint in the contact occlusion map (mirrors the vertex layout in
/// `occlusion.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub(crate) struct Occluder {
    /// World xz of the footprint's centre.
    pub center: [f32; 2],
    /// Unit direction (world xz) of the footprint's first axis.
    pub axis: [f32; 2],
    /// Half extents along the axis and across it (world units): of the footprint's rounded
    /// box, or with a mask, of the mask's square.
    pub half: [f32; 2],
    /// x = width of the soft falloff around a rounded box (world units), y = strength (0..1).
    pub shape: [f32; 2],
    /// The footprint's mask in the atlas (uv origin and size); zero size = a rounded box.
    pub mask: [f32; 4],
}

/// How far beyond the footprint an occluder reaches, in falloff widths (as in the shader).
const REACH: f32 = 1.6;

impl Occluder {
    /// Radius of the circle (world xz) outside which the occluder has no effect.
    pub fn reach(&self) -> f32 {
        Vec2::from(self.half).length() + self.shape[0] * REACH
    }
}

/// Texels per side of a model's footprint mask.
pub(crate) const MASK: usize = 32;

/// Where a model stands on the ground: the part of it that touches or overhangs the ground.
/// Computed once per model from its rest pose (see `ModelLibrary::add`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Footprint {
    /// Model-space xz bounds of the lower half of the model: min x, min z, max x, max z.
    pub bounds: [f32; 4],
    /// Height of the model's top above its origin.
    pub height: f32,
    /// Rest offset of the root piece (the model origin is the root piece's matrix moved back
    /// by this).
    pub root_offset: Vec3,
    /// Width of the soft falloff around the footprint (world units).
    pub falloff: f32,
    /// Model-space xz square the mask covers (the bounds plus the falloff): min x, min z,
    /// max x, max z.
    pub mask_rect: [f32; 4],
    /// `MASK` x `MASK` occlusion (255 = full) over `mask_rect`, rows along +z; empty when the
    /// model has no geometry.
    pub mask: Vec<u8>,
}

impl Footprint {
    /// Footprint of a model from its polygons in model space (rest pose, z already negated).
    pub fn of(polygons: &[Vec<Vec3>], root_offset: Vec3) -> Footprint {
        let points = || polygons.iter().flatten();
        let height = points().fold(0f32, |m, p| m.max(p.y));
        let low = points().fold(f32::MAX, |m, p| m.min(p.y));
        // The lower half (at least the bottom 4 units): hulls, tracks, legs and building
        // bases, but not barrels, wings of turrets or antennae sticking out higher up.
        let cut = (low + (height - low) * 0.5).max(low + 4.0);
        let xz_bounds = |points: &mut dyn Iterator<Item = &Vec3>| {
            points.fold(None, |b: Option<[f32; 4]>, p| {
                let b = b.unwrap_or([p.x, p.z, p.x, p.z]);
                Some([b[0].min(p.x), b[1].min(p.z), b[2].max(p.x), b[3].max(p.z)])
            })
        };
        let bounds = xz_bounds(&mut points().filter(|p| p.y <= cut)).or_else(|| xz_bounds(&mut points())).unwrap_or([0.0; 4]);
        let height = height.max(0.0);
        let falloff = (0.35 * height).clamp(3.0, 16.0);
        // The mask: a square around the bounds' centre with room for the falloff (twice the
        // width: the blur's reach).
        let centre = Vec2::new(bounds[0] + bounds[2], bounds[1] + bounds[3]) * 0.5;
        let half = ((bounds[2] - bounds[0]).max(bounds[3] - bounds[1]) * 0.5 + 2.0 * falloff).max(1.0);
        let mask_rect = [centre.x - half, centre.y - half, centre.x + half, centre.y + half];
        let mask = if polygons.is_empty() { Vec::new() } else { footprint_mask(polygons, cut, mask_rect, 2.0 * falloff) };
        Footprint { bounds, height, root_offset, falloff, mask_rect, mask }
    }

    /// The occluder of an instance whose root piece has world matrix `root`, standing over
    /// ground whose height `ground` gives (world xz), with `strength` (0..1), drawn with the
    /// mask at `mask_uv` in the atlas if it has one there (else as a rounded box). `None` for
    /// hidden or degenerate pieces, footprints too small to matter and models high above the
    /// ground.
    pub fn occluder(&self, root: &Mat4, ground: impl Fn(Vec2) -> f32, strength: f32, mask_uv: Option<[f32; 4]>) -> Option<Occluder> {
        let m = *root * Mat4::from_translation(-self.root_offset);
        let b = self.bounds;
        let ax = m.transform_vector3(Vec3::X);
        let az = m.transform_vector3(Vec3::Z);
        let (ax, az) = (Vec2::new(ax.x, ax.z), Vec2::new(az.x, az.z));
        let (lx, lz) = (ax.length(), az.length());
        let footprint = Vec2::new((b[2] - b[0]) * 0.5 * lx, (b[3] - b[1]) * 0.5 * lz);
        if lx < 1e-3 || lz < 1e-3 || !footprint.is_finite() || footprint.min_element() < 0.5 {
            return None;
        }
        // Fade with the model's height above the ground: landed aircraft keep their contact
        // shadow, flying ones (and ships over deep water) cast none.
        let base = m.transform_point3(Vec3::ZERO);
        let middle = m.transform_point3(Vec3::new((b[0] + b[2]) * 0.5, 0.0, (b[1] + b[3]) * 0.5));
        if !middle.is_finite() {
            return None;
        }
        let altitude = base.y - ground(Vec2::new(middle.x, middle.z));
        let lift = 1.0 - smoothstep(2.0, 6.0 + 0.8 * self.height, altitude);
        // Small things occlude less.
        let size = smoothstep(2.0, 10.0, footprint.min_element() * 2.0).max(0.25);
        let strength = (strength * lift * size).min(1.0);
        if strength <= 0.01 {
            return None;
        }
        let axis = (ax / lx).to_array();
        match mask_uv.filter(|_| !self.mask.is_empty()) {
            Some(uv) => {
                let r = self.mask_rect;
                let centre = m.transform_point3(Vec3::new((r[0] + r[2]) * 0.5, 0.0, (r[1] + r[3]) * 0.5));
                let half = [(r[2] - r[0]) * 0.5 * lx, (r[3] - r[1]) * 0.5 * lz];
                Some(Occluder { center: [centre.x, centre.z], axis, half, shape: [self.falloff, strength], mask: uv })
            }
            None => Some(Occluder { center: [middle.x, middle.z], axis, half: footprint.to_array(), shape: [self.falloff, strength], mask: [0.0; 4] }),
        }
    }
}

/// Whether `p` lies inside triangle `a b c` (either winding).
fn inside_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let d1 = (b - a).perp_dot(p - a);
    let d2 = (c - b).perp_dot(p - b);
    let d3 = (a - c).perp_dot(p - c);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// The occlusion mask of a model's footprint over `rect` (model-space xz square): its polygons
/// that reach below `cut`, seen from above (edges drawn conservatively so walls, which project
/// to lines, outline the footprint; interiors enclosed by them filled), blurred with a tent
/// `blur` world units wide. 255 = fully occluded.
fn footprint_mask(polygons: &[Vec<Vec3>], cut: f32, rect: [f32; 4], blur: f32) -> Vec<u8> {
    let n = MASK as i32;
    let origin = Vec2::new(rect[0], rect[1]);
    let texel = ((rect[2] - rect[0]) / MASK as f32).max(1e-3);
    let to_grid = |p: &Vec3| (Vec2::new(p.x, p.z) - origin) / texel;
    let mut solid = vec![false; MASK * MASK];
    let mark = |solid: &mut Vec<bool>, x: i32, z: i32| {
        if (0..n).contains(&x) && (0..n).contains(&z) {
            solid[(z * n + x) as usize] = true;
        }
    };
    for poly in polygons.iter().filter(|p| p.len() >= 3 && p.iter().any(|v| v.y <= cut)) {
        let g: Vec<Vec2> = poly.iter().map(to_grid).collect();
        for i in 0..g.len() {
            let (a, b) = (g[i], g[(i + 1) % g.len()]);
            let steps = ((b - a).length() * 4.0).ceil().clamp(1.0, 4096.0) as i32;
            for k in 0..=steps {
                let p = a.lerp(b, k as f32 / steps as f32);
                mark(&mut solid, p.x.floor() as i32, p.y.floor() as i32);
            }
        }
        for k in 1..g.len() - 1 {
            let (a, b, c) = (g[0], g[k], g[k + 1]);
            let lo = a.min(b).min(c).floor().max(Vec2::ZERO);
            let hi = a.max(b).max(c).ceil().min(Vec2::splat(MASK as f32));
            for z in lo.y as i32..hi.y as i32 {
                for x in lo.x as i32..hi.x as i32 {
                    if inside_triangle(Vec2::new(x as f32 + 0.5, z as f32 + 0.5), a, b, c) {
                        mark(&mut solid, x, z);
                    }
                }
            }
        }
    }
    // Everything the border cannot reach without crossing the footprint is inside it.
    let mut outside = vec![false; MASK * MASK];
    let mut stack: Vec<(i32, i32)> = Vec::new();
    for i in 0..n {
        stack.extend([(i, 0), (i, n - 1), (0, i), (n - 1, i)]);
    }
    while let Some((x, z)) = stack.pop() {
        if !(0..n).contains(&x) || !(0..n).contains(&z) {
            continue;
        }
        let i = (z * n + x) as usize;
        if solid[i] || outside[i] {
            continue;
        }
        outside[i] = true;
        stack.extend([(x + 1, z), (x - 1, z), (x, z + 1), (x, z - 1)]);
    }
    // Tent blur (two box passes per axis), zero outside the square.
    let mut v: Vec<f32> = outside.iter().map(|&o| if o { 0.0 } else { 1.0 }).collect();
    let r = ((blur / texel) * 0.5).round().clamp(0.0, MASK as f32 / 4.0) as i32;
    let box_blur = |src: &[f32], dx: i32, dz: i32| -> Vec<f32> {
        let mut dst = vec![0f32; MASK * MASK];
        for z in 0..n {
            for x in 0..n {
                let mut sum = 0.0;
                for k in -r..=r {
                    let (sx, sz) = (x + k * dx, z + k * dz);
                    if (0..n).contains(&sx) && (0..n).contains(&sz) {
                        sum += src[(sz * n + sx) as usize];
                    }
                }
                dst[(z * n + x) as usize] = sum / (2 * r + 1) as f32;
            }
        }
        dst
    };
    for (dx, dz) in [(1, 0), (1, 0), (0, 1), (0, 1)] {
        v = box_blur(&v, dx, dz);
    }
    // Full occlusion from halfway up the blurred edge inwards.
    v.iter().map(|&c| ((c * 2.0).min(1.0) * 255.0 + 0.5) as u8).collect()
}

/// The contact occluder of an upright sprite (a tree, a rock) of `width` world units anchored
/// on the ground at `pos`.
pub(crate) fn sprite_occluder(pos: [f32; 3], width: f32, strength: f32) -> Option<Occluder> {
    let r = (width * 0.22).max(2.0);
    let strength = strength * smoothstep(6.0, 24.0, width);
    (strength > 0.01 && pos.iter().all(|v| v.is_finite())).then(|| Occluder {
        center: [pos[0], pos[2]],
        axis: [1.0, 0.0],
        half: [r, r],
        shape: [(width * 0.15).clamp(2.0, 12.0), strength.min(1.0)],
        mask: [0.0; 4],
    })
}

/// The contact occluder of a 3D rock or vehicle meeting the ground `half` (along its length and
/// across, a rock's the same) from its foot at `pos`, turned by `yaw` (as the model is: its
/// length along (cos, -sin)): dark under it and at its edge, fading out over half as far again,
/// the shade where it meets the ground (round a rock's bulge, the ground it shelters).
pub(crate) fn contact_occluder(pos: [f32; 3], half: [f32; 2], yaw: f32, strength: f32) -> Option<Occluder> {
    let (s, c) = yaw.sin_cos();
    let least = half[0].min(half[1]);
    (strength > 0.01 && least > 1.0 && pos.iter().all(|v| v.is_finite())).then(|| Occluder {
        center: [pos[0], pos[2]],
        axis: [c, -s],
        half: half.map(|h| h * 1.05),
        shape: [(least * 0.5).clamp(2.0, 20.0), strength.min(1.0)],
        mask: [0.0; 4],
    })
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Texels per side of the contact occlusion map.
pub(crate) const MAP_TEXELS: u32 = 1024;
/// Smallest and largest world size of the map's square region.
const MIN_REGION: f32 = 512.0;
const MAX_REGION: f32 = 8192.0;

/// The square of ground (world xz) the contact occlusion map covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Region {
    pub origin: Vec2,
    pub size: f32,
}

impl Region {
    /// The region for the visible ground `bounds` (world xz) around `focus`: the bounds (at
    /// most `MAX_REGION / 2` from the focus) in a square whose size is `MIN_REGION` times a
    /// power of two, snapped to whole texels, so texels neither swim while the camera pans nor
    /// change size except in steps while it zooms.
    pub fn fit(focus: Vec2, bounds: (Vec2, Vec2)) -> Region {
        let lo = bounds.0.max(focus - MAX_REGION * 0.5);
        let hi = bounds.1.min(focus + MAX_REGION * 0.5).max(lo);
        let need = (hi - lo).max_element().max(MIN_REGION);
        let mut size = MIN_REGION;
        while size < need && size < MAX_REGION {
            size *= 2.0;
        }
        let texel = size / MAP_TEXELS as f32;
        let center = (lo + hi) * 0.5;
        let origin = ((center - size * 0.5) / texel).floor() * texel;
        Region { origin, size }
    }

    /// Whether a circle (world xz) overlaps the region.
    pub fn touches(&self, center: Vec2, radius: f32) -> bool {
        let lo = self.origin - radius;
        let hi = self.origin + self.size + radius;
        center.cmpge(lo).all() && center.cmple(hi).all()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OcclusionUniform {
    /// xy = world xz of the region's minimum corner, z = 1 / its size.
    region: [f32; 4],
}

/// The contact occlusion map and the pass that draws it.
/// Footprint masks per atlas row and rows in the atlas (capacity: models with a mask).
const ATLAS_COLUMNS: usize = 32;
const ATLAS_ROWS: usize = 64;

pub(crate) struct OcclusionMap {
    pub target: Target,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    count: u32,
    /// The region drawn this frame.
    pub region: Region,
    /// Footprint masks of the models, `MASK` texels square each.
    atlas: wgpu::Texture,
    /// Atlas uv rectangle of each model's mask by model id (`None`: no mask, or no room).
    masks: Vec<Option<[f32; 4]>>,
}

impl OcclusionMap {
    pub fn new(device: &wgpu::Device) -> Self {
        let target = Target::new(
            device,
            "contact occlusion",
            MAP_TEXELS,
            MAP_TEXELS,
            wgpu::TextureFormat::R8Unorm,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("contact occlusion"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("contact occlusion"),
            size: std::mem::size_of::<OcclusionUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("footprint masks"),
            size: wgpu::Extent3d { width: (ATLAS_COLUMNS * MASK) as u32, height: (ATLAS_ROWS * MASK) as u32, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("footprint masks"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("contact occlusion"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atlas.create_view(&Default::default())) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let shader = crate::util::shader_standalone(device, "contact occlusion", include_str!("shaders/occlusion.wgsl"));
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("contact occlusion"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // Footprints multiply: overlapping ones darken more.
        let multiply = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Dst, dst_factor: wgpu::BlendFactor::Zero, operation: wgpu::BlendOperation::Add };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("contact occlusion"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_occluder"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Occluder>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_occluder"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R8Unorm,
                    blend: Some(wgpu::BlendState { color: multiply, alpha: multiply }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let instances = Self::make_instances(device, 1024);
        OcclusionMap {
            target,
            pipeline,
            uniform,
            bind_group,
            instances,
            count: 0,
            region: Region { origin: Vec2::ZERO, size: MIN_REGION },
            atlas,
            masks: Vec::new(),
        }
    }

    /// Uploads the footprint masks of models added since the last call (`footprints` by model
    /// id, the library's whole list).
    pub fn upload_masks<'a>(&mut self, queue: &wgpu::Queue, footprints: impl ExactSizeIterator<Item = &'a Footprint>) {
        let total = footprints.len();
        for (id, fp) in footprints.enumerate().skip(self.masks.len()) {
            if fp.mask.len() != MASK * MASK || id >= ATLAS_COLUMNS * ATLAS_ROWS {
                self.masks.push(None);
                continue;
            }
            let (col, row) = (id % ATLAS_COLUMNS, id / ATLAS_COLUMNS);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: (col * MASK) as u32, y: (row * MASK) as u32, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &fp.mask,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(MASK as u32), rows_per_image: Some(MASK as u32) },
                wgpu::Extent3d { width: MASK as u32, height: MASK as u32, depth_or_array_layers: 1 },
            );
            let (w, h) = ((ATLAS_COLUMNS * MASK) as f32, (ATLAS_ROWS * MASK) as f32);
            self.masks.push(Some([(col * MASK) as f32 / w, (row * MASK) as f32 / h, MASK as f32 / w, MASK as f32 / h]));
        }
        debug_assert_eq!(self.masks.len(), total);
    }

    /// Atlas uv rectangle of model `id`'s footprint mask, once uploaded.
    pub fn mask_uv(&self, id: u32) -> Option<[f32; 4]> {
        self.masks.get(id as usize).copied().flatten()
    }

    fn make_instances(device: &wgpu::Device, n: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("occluders"),
            size: (n * std::mem::size_of::<Occluder>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Uploads this frame's region and the occluders touching it; returns how many there are.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, region: Region, occluders: &mut Vec<Occluder>) -> u32 {
        occluders.retain(|o| region.touches(Vec2::from(o.center), o.reach()));
        self.region = region;
        self.count = occluders.len() as u32;
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&OcclusionUniform { region: [region.origin.x, region.origin.y, 1.0 / region.size, 0.0] }));
        if occluders.is_empty() {
            return 0;
        }
        let bytes = std::mem::size_of_val(occluders.as_slice()) as u64;
        if bytes > self.instances.size() {
            self.instances = Self::make_instances(device, occluders.len().next_power_of_two());
        }
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(occluders));
        self.count
    }

    /// Clears the map to open ground and draws the prepared occluders.
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("contact occlusion"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.target.view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store },
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
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visibility(w: u32, h: u32, f: impl Fn(u32, u32) -> u8) -> (Vec<u8>, impl Fn(&[u8], u32, u32) -> u8) {
        let heights: Vec<u8> = (0..h).flat_map(|z| (0..w).map(move |x| (x, z))).map(|(x, z)| f(x, z)).collect();
        (terrain_visibility(w, h, &heights), move |v: &[u8], x: u32, z: u32| v[(z * w + x) as usize])
    }

    #[test]
    fn flat_ground_and_even_slopes_see_the_whole_sky() {
        let (v, _) = visibility(40, 30, |_, _| 90);
        assert!(v.iter().all(|&a| a == 255));
        // A steady slope: its own tangent plane hides nothing.
        let (v, at) = visibility(48, 48, |x, _| (x * 4) as u8);
        for z in 5..43 {
            for x in 5..43 {
                assert!(at(&v, x, z) >= 250, "({x}, {z}): {}", at(&v, x, z));
            }
        }
    }

    #[test]
    fn creases_and_the_foot_of_cliffs_are_occluded() {
        // A pit 80 units deep, 5 cells wide, in flat ground.
        let (v, at) = visibility(48, 48, |x, z| if (22..27).contains(&x) && (22..27).contains(&z) { 20 } else { 100 });
        assert!(at(&v, 24, 24) < 170, "pit floor {}", at(&v, 24, 24));
        assert!(at(&v, 2, 2) == 255, "open ground far away {}", at(&v, 2, 2));
        assert!(at(&v, 30, 24) >= 250, "the rim is not occluded: {}", at(&v, 30, 24));
        // A cliff: its foot is darker than its top, and both approach open sky away from it.
        let (v, at) = visibility(64, 16, |x, _| if x < 32 { 40 } else { 160 });
        let (foot, top) = (at(&v, 30, 8), at(&v, 34, 8));
        assert!(foot < 200 && top >= 250, "foot {foot}, top {top}");
        assert!(at(&v, 29, 8) > foot || at(&v, 29, 8) < 200, "falls off from the wall");
        assert!(at(&v, 5, 8) > 245, "far from the cliff: {}", at(&v, 5, 8));
        // Tiny or empty maps do not panic.
        assert_eq!(terrain_visibility(1, 1, &[7]), vec![255]);
        assert!(terrain_visibility(0, 5, &[]).is_empty());
    }

    /// The faces of an axis-aligned box (walls, and the roof unless `open`).
    fn cuboid(min: Vec3, max: Vec3, open: bool) -> Vec<Vec<Vec3>> {
        let (a, b) = (min, max);
        let mut faces = vec![
            vec![Vec3::new(a.x, a.y, a.z), Vec3::new(b.x, a.y, a.z), Vec3::new(b.x, b.y, a.z), Vec3::new(a.x, b.y, a.z)],
            vec![Vec3::new(a.x, a.y, b.z), Vec3::new(b.x, a.y, b.z), Vec3::new(b.x, b.y, b.z), Vec3::new(a.x, b.y, b.z)],
            vec![Vec3::new(a.x, a.y, a.z), Vec3::new(a.x, a.y, b.z), Vec3::new(a.x, b.y, b.z), Vec3::new(a.x, b.y, a.z)],
            vec![Vec3::new(b.x, a.y, a.z), Vec3::new(b.x, a.y, b.z), Vec3::new(b.x, b.y, b.z), Vec3::new(b.x, b.y, a.z)],
        ];
        if !open {
            faces.push(vec![Vec3::new(a.x, b.y, a.z), Vec3::new(b.x, b.y, a.z), Vec3::new(b.x, b.y, b.z), Vec3::new(a.x, b.y, b.z)]);
        }
        faces
    }

    fn footprint_box() -> Footprint {
        // A 20 x 12 box, 16 high, with a thin barrel sticking out 30 units at the top.
        let mut polygons = cuboid(Vec3::new(-10.0, 0.0, -6.0), Vec3::new(10.0, 16.0, 6.0), false);
        polygons.push(vec![Vec3::new(-0.5, 14.0, 0.0), Vec3::new(0.5, 14.0, 0.0), Vec3::new(0.0, 14.0, 30.0)]);
        Footprint::of(&polygons, Vec3::ZERO)
    }

    /// Mask occlusion (0..1) at model-space xz `p`.
    fn mask_at(fp: &Footprint, p: Vec2) -> f32 {
        let r = fp.mask_rect;
        let t = (p - Vec2::new(r[0], r[1])) / (r[2] - r[0]) * MASK as f32;
        let (x, z) = (t.x.floor().clamp(0.0, MASK as f32 - 1.0) as usize, t.y.floor().clamp(0.0, MASK as f32 - 1.0) as usize);
        fp.mask[z * MASK + x] as f32 / 255.0
    }

    #[test]
    fn footprints_ignore_parts_high_up_and_turn_with_the_unit() {
        let fp = footprint_box();
        assert_eq!(fp.bounds, [-10.0, -6.0, 10.0, 6.0], "the barrel is not part of the footprint");
        assert_eq!(fp.height, 16.0);
        // Standing at (100, 50, 200) turned 90°: the long axis now runs along z.
        let root = Mat4::from_translation(Vec3::new(100.0, 50.0, 200.0)) * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let o = fp.occluder(&root, |_| 50.0, 0.6, None).expect("occluder");
        assert!((Vec2::from(o.center) - Vec2::new(100.0, 200.0)).length() < 1e-3);
        assert!((Vec2::from(o.axis) - Vec2::new(0.0, -1.0)).length() < 1e-3, "{:?}", o.axis);
        assert!((Vec2::from(o.half) - Vec2::new(10.0, 6.0)).length() < 1e-3);
        assert!((o.shape[1] - 0.6).abs() < 1e-3 && o.shape[0] > 3.0 && o.mask == [0.0; 4]);
        assert!(o.reach() > 11.0);
        // With its mask: the mask's square, centred on the footprint.
        let o = fp.occluder(&root, |_| 50.0, 0.6, Some([0.25, 0.5, 0.03, 0.01])).expect("occluder");
        assert_eq!(o.mask, [0.25, 0.5, 0.03, 0.01]);
        assert!((Vec2::from(o.center) - Vec2::new(100.0, 200.0)).length() < 1e-3);
        assert!(o.half[0] > 10.0 + fp.falloff && (o.half[0] - o.half[1]).abs() < 1e-3);
        // The same unit flying 150 units up casts nothing; hidden pieces neither.
        assert!(fp.occluder(&root, |_| -100.0, 0.6, None).is_none());
        assert!(fp.occluder(&Mat4::ZERO, |_| 0.0, 0.6, None).is_none());
        // A root piece offset from the model origin is taken back out.
        let offset = Footprint { root_offset: Vec3::new(0.0, 5.0, 3.0), ..fp };
        let o = offset.occluder(&(Mat4::from_translation(Vec3::new(0.0, 5.0, 3.0))), |_| 0.0, 1.0, None).expect("occluder");
        assert!(Vec2::from(o.center).length() < 1e-3, "{:?}", o.center);
    }

    #[test]
    fn footprint_masks_follow_the_shape_seen_from_above() {
        // The box: full under it and at its edge, fading out beyond, nothing under the barrel.
        let fp = footprint_box();
        assert_eq!(fp.mask.len(), MASK * MASK);
        assert!(mask_at(&fp, Vec2::ZERO) > 0.99, "centre");
        assert!(mask_at(&fp, Vec2::new(9.5, 0.0)) > 0.85, "edge");
        assert!(mask_at(&fp, Vec2::new(10.0 + fp.falloff * 1.9, 0.0)) < 0.1, "beyond the falloff");
        assert!(mask_at(&fp, Vec2::new(0.0, 24.0)) < 0.05, "under the barrel");
        // A cross-shaped building (two slabs): its arms are occluded, the corners between them
        // much less than a bounding box would.
        let mut cross = cuboid(Vec3::new(-30.0, 0.0, -8.0), Vec3::new(30.0, 20.0, 8.0), false);
        cross.extend(cuboid(Vec3::new(-8.0, 0.0, -30.0), Vec3::new(8.0, 20.0, 30.0), false));
        let fp = Footprint::of(&cross, Vec3::ZERO);
        assert!(mask_at(&fp, Vec2::new(18.0, 0.0)) > 0.9 && mask_at(&fp, Vec2::new(0.0, -18.0)) > 0.9, "arms");
        assert!(mask_at(&fp, Vec2::new(24.0, 24.0)) < 0.3, "corner {}", mask_at(&fp, Vec2::new(24.0, 24.0)));
        // Walls without a roof or floor (they project to lines) still occlude what they enclose.
        let fp = Footprint::of(&cuboid(Vec3::new(-20.0, 0.0, -20.0), Vec3::new(20.0, 30.0, 20.0), true), Vec3::ZERO);
        assert!(mask_at(&fp, Vec2::ZERO) > 0.99, "enclosed interior {}", mask_at(&fp, Vec2::ZERO));
        // No geometry, no mask.
        assert!(Footprint::of(&[], Vec3::ZERO).mask.is_empty());
    }

    #[test]
    fn sprites_occlude_by_their_width() {
        let big = sprite_occluder([10.0, 5.0, 20.0], 60.0, 0.5).expect("tree");
        assert_eq!(big.center, [10.0, 20.0]);
        assert!(big.half[0] > 10.0 && (big.shape[1] - 0.5).abs() < 1e-6);
        let small = sprite_occluder([0.0; 3], 10.0, 0.5).expect("bush");
        assert!(small.shape[1] < big.shape[1]);
        assert!(sprite_occluder([0.0; 3], 3.0, 0.5).is_none());
    }

    #[test]
    fn regions_cover_the_view_in_whole_texels() {
        let focus = Vec2::new(1000.0, 800.0);
        let bounds = (Vec2::new(420.0, 400.0), Vec2::new(1590.0, 1200.0));
        let r = Region::fit(focus, bounds);
        let texel = r.size / MAP_TEXELS as f32;
        assert_eq!(r.size, 2048.0);
        assert!(r.origin.x <= bounds.0.x && r.origin.y <= bounds.0.y);
        assert!(r.origin.x + r.size >= bounds.1.x - texel && r.origin.y + r.size >= bounds.1.y - texel);
        assert_eq!((r.origin / texel).fract(), Vec2::ZERO);
        // Panning keeps the texel grid; small views get the smallest region; huge views are
        // capped around the focus.
        let moved = Region::fit(focus + 3.3, (bounds.0 + 3.3, bounds.1 + 3.3));
        assert_eq!(moved.size, r.size);
        assert_eq!(((moved.origin - r.origin) / texel).fract(), Vec2::ZERO);
        assert_eq!(Region::fit(focus, (focus - 10.0, focus + 10.0)).size, MIN_REGION);
        let huge = Region::fit(focus, (Vec2::splat(-1e6), Vec2::splat(1e6)));
        assert_eq!(huge.size, MAX_REGION);
        assert!(huge.touches(focus, 0.0) && !huge.touches(focus + 9000.0, 100.0));
    }
}
