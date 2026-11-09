//! Terrain resources: heightmap, tile map, tile atlas with per-tile mips, and chunked drawing.

use bytemuck::{Pod, Zeroable};
use glam::{UVec2, Vec2};
use ta_formats::image::Palette;
use ta_formats::tnt::{TILE_SIZE, TntMap};
use wgpu::util::DeviceExt;
use crate::gpu::CheckedBufferInit;

use crate::terrain_pads::GroundPad;
use crate::util::{linear_to_srgb, srgb_to_linear_lut};

/// Cells per chunk edge; each chunk is one instance of a grid mesh drawn from the heightmap.
pub const CHUNK_CELLS: u32 = 32;
/// Vertices a chunk is drawn with: six a cell (each cell's split is its own, see
/// [`split_flipped`]).
const CHUNK_VERTICES: u32 = CHUNK_CELLS * CHUNK_CELLS * 6;
/// Atlas mip levels stored (32 px down to 2 px per tile); level 5 lives in `tile_avg`.
const ATLAS_LEVELS: u32 = 5;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct TerrainParams {
    cells: [u32; 2],
    tiles: [u32; 2],
    atlas_cols: u32,
    chunk_cells: u32,
    detail: f32,
    _pad: u32,
}

/// The most used of a map's plain ground tiles (mirrors `PlainTiles` in terrain_common.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct PlainTiles {
    /// xyz = square root of the tile's mean colour (linear; the shader compares colours in that
    /// roughly perceptual space), w unused.
    tiles: [[f32; 4]; 16],
    /// x = number of tiles in use.
    count: [u32; 4],
}

impl PlainTiles {
    /// The first 16 of `means` (linear colours, most used first).
    fn new(means: impl IntoIterator<Item = [f32; 3]>) -> Self {
        let mut out = PlainTiles { tiles: [[0.0; 4]; 16], count: [0; 4] };
        for (slot, c) in out.tiles.iter_mut().zip(means) {
            *slot = [c[0].max(0.0).sqrt(), c[1].max(0.0).sqrt(), c[2].max(0.0).sqrt(), 0.0];
            out.count[0] += 1;
        }
        out
    }
}

/// CPU copy of the heightmap for picking and placement.
#[derive(Clone)]
pub struct HeightField {
    pub width: u32,
    pub height: u32,
    /// TA's heights (whole units), as the simulation has them.
    pub heights: Vec<u8>,
    pub sea_level: f32,
    /// The heights the ground is drawn at (see [`crate::terrain_surface`]); empty: TA's.
    pub display: Vec<f32>,
}

impl HeightField {
    /// Height at a vertex of the cell grid as it is drawn (clamped at the edges).
    #[inline]
    pub fn vertex(&self, x: i32, z: i32) -> f32 {
        let x = x.clamp(0, self.width as i32 - 1) as u32;
        let z = z.clamp(0, self.height as i32 - 1) as u32;
        let i = (z * self.width + x) as usize;
        self.display.get(i).copied().unwrap_or_else(|| self.heights[i] as f32)
    }

    /// The highest ground as drawn.
    pub fn highest(&self) -> f32 {
        if self.display.is_empty() {
            return self.heights.iter().copied().max().unwrap_or(0) as f32;
        }
        self.display.iter().copied().fold(0.0, f32::max)
    }

    /// Ground height at a world position, exactly as the mesh is drawn (see [`cell_height`]).
    pub fn sample(&self, x: f32, z: f32) -> f32 {
        let (fx, fz) = (x / 16.0, z / 16.0);
        let (x0, z0) = (fx.floor(), fz.floor());
        let (cx, cz) = (x0 as i32, z0 as i32);
        let h = [self.vertex(cx, cz), self.vertex(cx + 1, cz), self.vertex(cx, cz + 1), self.vertex(cx + 1, cz + 1)];
        cell_height(h, fx - x0, fz - z0)
    }

    pub fn size_px(&self) -> Vec2 {
        Vec2::new(self.width as f32 * 16.0, self.height as f32 * 16.0)
    }

    /// Intersects a ray with the terrain by marching and refining. Returns the hit point.
    pub fn raycast(&self, origin: glam::Vec3, dir: glam::Vec3) -> Option<glam::Vec3> {
        let size = self.size_px();
        // Start where the ray enters the height band [0, 300].
        let mut t = if dir.y < -1e-5 { ((origin.y - 300.0) / -dir.y).max(0.0) } else { 0.0 };
        let step = 4.0;
        let mut prev_t = t;
        for _ in 0..20000 {
            let p = origin + dir * t;
            if p.y < -50.0 {
                break;
            }
            let inside = p.x >= -64.0 && p.z >= -64.0 && p.x <= size.x + 64.0 && p.z <= size.y + 64.0;
            if inside && p.y <= self.sample(p.x, p.z) {
                // Binary refinement between prev_t and t.
                let (mut lo, mut hi) = (prev_t, t);
                for _ in 0..16 {
                    let mid = 0.5 * (lo + hi);
                    let q = origin + dir * mid;
                    if q.y <= self.sample(q.x, q.z) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                return Some(origin + dir * hi);
            }
            prev_t = t;
            t += step;
        }
        None
    }
}

/// The current map's ground as it is drawn ([`HeightField::display`]), for what stands on it to
/// be drawn there (`ta_game::scene::drawn_ground`, which has only the simulation's world to go
/// on): set when a map's terrain is loaded and when it is edited.
static DRAWN: std::sync::RwLock<Option<std::sync::Arc<HeightField>>> = std::sync::RwLock::new(None);

fn set_drawn(field: &HeightField) {
    if let Ok(mut drawn) = DRAWN.write() {
        *drawn = Some(std::sync::Arc::new(field.clone()));
    }
}

/// [`set_drawn`] after the drawn heights of the vertices of `rect` alone changed (a building's
/// pad): those copied in place (a whole map's copy for each building would be a megabyte or so).
fn update_drawn(field: &HeightField, [x0, z0, x1, z1]: [u32; 4]) {
    let Ok(mut drawn) = DRAWN.write() else { return };
    let same = |d: &HeightField| d.width == field.width && d.height == field.height && d.display.len() == field.display.len() && d.heights == field.heights;
    if let Some(d) = drawn.as_mut().and_then(std::sync::Arc::get_mut).filter(|d| same(d)) {
        for z in z0..z1 {
            let row = (z * field.width) as usize;
            d.display[row + x0 as usize..row + x1 as usize].copy_from_slice(&field.display[row + x0 as usize..row + x1 as usize]);
        }
        return;
    }
    *drawn = Some(std::sync::Arc::new(field.clone()));
}

/// The ground's height at world (`x`, `z`) as it is drawn, on the map of `width` × `height`
/// cells whose terrain was loaded last; none when no such map's terrain is.
pub fn drawn_height(width: u32, height: u32, x: f32, z: f32) -> Option<f32> {
    let drawn = DRAWN.read().ok()?;
    drawn.as_ref().filter(|f| f.width == width && f.height == height && !f.display.is_empty()).map(|f| f.sample(x, z))
}

/// The ground as it is drawn ([`drawn_height`]'s), to sample many times over without asking
/// each time: what poses every unit of a battle at once takes it once. (Held only while it is
/// used: a building's pad changes it in place when nothing else holds it.)
pub fn drawn_field(width: u32, height: u32) -> Option<std::sync::Arc<HeightField>> {
    let drawn = DRAWN.read().ok()?;
    drawn.as_ref().filter(|f| f.width == width && f.height == height && !f.display.is_empty()).cloned()
}

/// Whether a cell with corner heights `[h00, h10, h01, h11]` is drawn split along its
/// (1,0)-(0,1) diagonal rather than (0,0)-(1,1): a steep cell (corners 16 units or more apart)
/// is split along the diagonal whose ends differ less, so a slope running across the other
/// diagonal is drawn as the straight slope it is, not as a step a cell (a cliff's lip running
/// from south-west to north-east was a staircase); gentle ground keeps the one split. The
/// terrain shaders (`split_flipped` in terrain_common.wgsl) and TA Reborn's map generator
/// (`Field::sample_tri`) use the same rule. (The simulation keeps the one split: things
/// standing on a flipped cell are drawn at the drawn ground's height.)
pub fn split_flipped([h00, h10, h01, h11]: [f32; 4]) -> bool {
    let range = h00.max(h10).max(h01).max(h11) - h00.min(h10).min(h01).min(h11);
    range >= 16.0 && (h10 - h01).abs() < (h00 - h11).abs()
}

/// Height at fraction (`tx`, `tz`) of a cell with corner heights `[h00, h10, h01, h11]`, on the
/// two triangles it is drawn as (split as [`split_flipped`] says).
pub fn cell_height(h: [f32; 4], tx: f32, tz: f32) -> f32 {
    let [h00, h10, h01, h11] = h;
    if split_flipped(h) {
        if tx + tz <= 1.0 {
            h00 + (h10 - h00) * tx + (h01 - h00) * tz
        } else {
            h11 + (h01 - h11) * (1.0 - tx) + (h10 - h11) * (1.0 - tz)
        }
    } else if tx >= tz {
        h00 + (h10 - h00) * tx + (h11 - h10) * tz
    } else {
        h00 + (h11 - h01) * tx + (h01 - h00) * tz
    }
}

/// The key of `map`'s land (linear luminance): the log-average of its tiles' mean painted
/// luminance (`means`, by the tiles `shown` at each tile position) over those standing above the
/// sea, or over all of them on a map under water; and its colour balance, the mean of those
/// tiles' colours brought to luminance 1. What a world's colour grade draws towards its own key and
/// balance (`terrain_detail::WorldGrade`): the sea, painted dark, would make a coast look dimmer
/// than its land is.
fn land_key(map: &TntMap, shown: &[u16], means: &[[f32; 3]]) -> (f32, [f32; 3]) {
    let (tiles_x, tiles_y) = ((map.width / 2) as usize, (map.height / 2) as usize);
    // (Sums of ln luminance, of the colours, and counts: [under the sea, above it].)
    let mut sums = [(0.0f64, [0.0f64; 3], 0u32); 2];
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let c = shown.get(ty * tiles_x + tx).and_then(|&t| means.get(t as usize)).copied().unwrap_or([0.0; 3]);
            let l = (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).max(1e-4);
            // (A tile is two cells a side; land where they stand above the sea on average.)
            let h: i32 = (0..4).map(|k| map.heights.get((ty * 2 + k / 2) * map.width as usize + tx * 2 + k % 2).copied().unwrap_or(0) as i32).sum();
            let s = &mut sums[usize::from(h >= map.sea_level * 4)];
            s.0 += (l as f64).ln();
            for (b, v) in s.1.iter_mut().zip(c) {
                *b += v as f64;
            }
            s.2 += 1;
        }
    }
    let (sum, balance, n) = if sums[1].2 > 0 {
        sums[1]
    } else {
        (sums[0].0 + sums[1].0, [0, 1, 2].map(|i| sums[0].1[i] + sums[1].1[i]), sums[0].2 + sums[1].2)
    };
    let l = 0.2126 * balance[0] + 0.7152 * balance[1] + 0.0722 * balance[2];
    if n == 0 || l <= 0.0 {
        return (0.05, [1.0; 3]);
    }
    ((sum / n as f64).exp() as f32, balance.map(|b| (b / l) as f32))
}

pub struct TerrainGpu {
    pub field: HeightField,
    /// Mean painted colour of the whole map (linear RGB).
    pub(crate) average_color: [f32; 3],
    /// The key of the map's land: the log-average luminance of its painted ground above the sea,
    /// and its colour balance (rgb at luminance 1; see [`land_key`]).
    pub(crate) land_key: f32,
    pub(crate) land_balance: [f32; 3],
    /// The heightmap (r = height / 255 per cell, g = under sea level), for the frame bind
    /// group's ground heights.
    pub(crate) height_view: wgpu::TextureView,
    /// Mean painted colour of each 32 px tile, indexed by map tile position (an sRGB texture),
    /// for the particles that take the ground's colour.
    pub(crate) tile_colors: wgpu::TextureView,
    /// Whether the map has ground under sea level, i.e. whether a sea may continue beyond it.
    pub(crate) sea_beyond: bool,
    /// What lies round the map, for the ground beyond it (see surroundings.rs).
    around: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Chunk instances: this frame's visible chunks, then (from `chunks.x * chunks.y`) those
    /// the reflection pass draws.
    instance_buffer: wgpu::Buffer,
    visible_chunks: u32,
    reflected_chunks: u32,
    chunks: UVec2,
    /// Highest vertex of each chunk (row-major): chunks entirely under the sea mirror nothing.
    chunk_tops: Vec<u8>,
    /// Map overlay colours, one RGBA texel per heightmap cell (see `set_overlay`).
    overlay: wgpu::Texture,
    cells: (u32, u32),
    /// What the map editor's in-place updates need (see [`TerrainGpu::update_heights`]).
    edit: TerrainEdit,
    /// The ground levelled under buildings (see [`TerrainGpu::add_pads`]).
    pads: crate::terrain_pads::Pads,
}

/// What laying or taking up pads changed (see [`TerrainGpu::add_pads`]): the ground's area
/// (world units: x0, z0, x1, z1) and the highest pad laid.
#[derive(Clone, Copy, Debug)]
pub struct PadChange {
    pub area: [f32; 4],
    pub top: f32,
}

/// The textures an edited map changes, and the CPU copies their mips are made from.
struct TerrainEdit {
    heightmap: wgpu::Texture,
    /// The heightmap's levels (drawn height / 255, under-sea mask) as uploaded.
    height_mips: Vec<(u32, u32, Vec<f32>)>,
    /// Whether the drawn heights are a metal or urban world's (see `set_built`).
    built: bool,
    /// The map's void cells (TA's impassable cells, `FEATURE_VOID`), and whether they are drawn
    /// as nothing (a world in space, see `set_space`).
    void: Vec<bool>,
    space: bool,
    /// The tile atlas, its tiles a row, and what painting over TA's hair-like strands needs
    /// (see `set_strands`): the tiles (palette indices) and the palette, the tiles as shown,
    /// the tiles cleaned (once asked for) and whether they are shown cleaned.
    atlas: wgpu::Texture,
    atlas_cols: u32,
    tiles: Vec<[u8; TILE_SIZE * TILE_SIZE]>,
    palette: Palette,
    shown: Vec<u16>,
    strands: Option<Vec<(u32, Box<crate::terrain_strands::TilePixels>)>>,
    strands_cleaned: bool,
    occlusion: wgpu::Texture,
    /// The relief's ambient occlusion as baked (each vertex's sky visibility), which shares
    /// `occlusion` with the waterline's distances.
    visibility: Vec<u8>,
    tile_map: wgpu::Texture,
    tile_avg: wgpu::Texture,
    /// Mean colour of every tile (linear RGB).
    tile_means: Vec<[f32; 3]>,
    /// The tile-average image's levels (linear RGB) as uploaded.
    avg_levels: Vec<(u32, u32, Vec<[f32; 3]>)>,
}

pub struct TerrainPipeline {
    pub layout: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
    /// The extended ground beyond the map (sky.rs): same fragment shader, skirt mesh vertices.
    pub(crate) skirt: wgpu::RenderPipeline,
    /// Both of the above mirrored in the water (reflection.rs): only the ground above it.
    pub(crate) reflection: wgpu::RenderPipeline,
    pub(crate) skirt_reflection: wgpu::RenderPipeline,
    /// Ray tracing's depth prepass (raytrace.rs): the map's ground, depth only, single-sampled.
    pub(crate) prepass: Option<wgpu::RenderPipeline>,
    /// The ground's detail materials (group 2 of the terrain's pipelines).
    pub detail: wgpu::BindGroup,
}

/// Terrain shader: shared terrain bindings and sampling, noise, procedural water (the sea
/// bed's caustics, shoreline foam), then the terrain and skirt entry
/// points. `common.wgsl` is prepended by [`crate::util::shader`]. The ground beyond the map's
/// edges (the skirt's pipelines) shades as [`shader_source_for`] with `far_land` has it.
#[cfg(test)]
pub(crate) fn shader_source() -> String {
    shader_source_for(true)
}

/// The terrain shader with the ground beyond the map's edges shaded (`far_land`: the skirt's
/// pipelines), or without (the map's own: no point of it lies beyond the map). Only the skirt's
/// carry that code: a fragment shader runs as slowly as its longest path makes it, however rarely
/// taken, and the map's ground drew some tenth slower for the far land's.
pub(crate) fn shader_source_for(far_land: bool) -> String {
    format!(
        "const FAR_LAND: bool = {far_land};\n{}\n{}\n{}\n{}",
        include_str!("shaders/terrain_common.wgsl"),
        include_str!("shaders/noise.wgsl"),
        include_str!("shaders/water_common.wgsl"),
        include_str!("shaders/terrain.wgsl")
    )
}

/// Box-filtered mip chain of a `width` x `height` image of `channels` interleaved bytes per
/// texel, level 0 first (odd edges reuse their last texel).
pub(crate) fn byte_mips(width: u32, height: u32, channels: usize, level0: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(width, height, level0.to_vec())];
    while let Some(&(w, h, ref src)) = levels.last().filter(|(w, h, _)| *w > 1 || *h > 1) {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut dst = vec![0u8; (nw * nh) as usize * channels];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..channels {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
                        sum += src[(sy * w + sx) as usize * channels + c] as u32;
                    }
                    dst[(y * nw + x) as usize * channels + c] = ((sum + 2) / 4) as u8;
                }
            }
        }
        levels.push((nw, nh, dst));
    }
    levels
}

/// Uploads tile `t`'s pixels (sRGB) into the atlas (`cols` tiles a row), all its levels.
fn upload_tile(queue: &wgpu::Queue, atlas: &wgpu::Texture, cols: u32, t: u32, pixels: &crate::terrain_strands::TilePixels) {
    let lut = srgb_to_linear_lut();
    let mut level: Vec<[f32; 3]> = pixels.iter().map(|p| [lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize]]).collect();
    for l in 0..ATLAS_LEVELS {
        let ts = (TILE_SIZE as u32) >> l;
        if l > 0 {
            let src = ts * 2;
            level = (0..ts * ts)
                .map(|i| {
                    let (x, y) = (i % ts, i / ts);
                    let at = |dx: u32, dy: u32| level[((y * 2 + dy) * src + x * 2 + dx) as usize];
                    let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
                    std::array::from_fn(|k| (a[k] + b[k] + c[k] + d[k]) * 0.25)
                })
                .collect();
        }
        let bytes: Vec<u8> = level.iter().flat_map(|c| [linear_to_srgb(c[0]), linear_to_srgb(c[1]), linear_to_srgb(c[2]), 255]).collect();
        write_packed(queue, atlas, l, [(t % cols) * ts, (t / cols) * ts, (t % cols + 1) * ts, (t / cols + 1) * ts], &bytes, 4);
    }
}

/// How far (cells) a change to TA's heights changes the drawn ones (the smoothing's passes and
/// the spline's reach, see terrain_surface.rs).
const DRAWN_REACH: u32 = 27;

/// The heightmap texture's texels: each vertex's drawn height / 255 and whether it lies under
/// the sea (1).
fn height_texels(display: &[f32], sea: f32) -> Vec<f32> {
    display.iter().flat_map(|&h| [h / 255.0, if h < sea { 1.0 } else { 0.0 }]).collect()
}

/// On a world in space: how far below the rock beside it the edge of a rock is drawn (its slab's
/// thickness), and the mark (in the heightmap's second channel, the under-sea mask, which a
/// world without a sea has no other use for) of a cell drawn as nothing: the terrain's vertex
/// shader (`vs_main`) leaves the cells whose four corners are all void out.
pub const VOID_SLAB: f32 = 30.0;
pub const VOID_MARK: f32 = 2.0;

/// A world in space's ground (`drawn`, the heights as drawn, `w` x `h` vertices; `void`, the void
/// cells): each void vertex beside the rock drawn a slab's thickness below the rock round it (the
/// rock's edge a level slab, not dropping to the void's own height). Returns which cells have all
/// four corners void, to be left out.
fn space_ground(drawn: &mut [f32], void: &[bool], w: usize, h: usize) -> Vec<bool> {
    let (wi, hi) = (w as i64, h as i64);
    let at = |x: i64, z: i64| (z.clamp(0, hi - 1) * wi + x.clamp(0, wi - 1)) as usize;
    let rock = drawn.to_vec();
    for z in 0..hi {
        for x in 0..wi {
            let i = at(x, z);
            if !void[i] {
                continue;
            }
            let (mut sum, mut n) = (0.0f32, 0.0f32);
            for (dx, dz) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                if !void[at(x + dx, z + dz)] {
                    sum += rock[at(x + dx, z + dz)];
                    n += 1.0;
                }
            }
            if n > 0.0 {
                drawn[i] = sum / n - VOID_SLAB;
            }
        }
    }
    (0..hi).flat_map(|z| (0..wi).map(move |x| (x, z))).map(|(x, z)| void[at(x, z)] && void[at(x + 1, z)] && void[at(x, z + 1)] && void[at(x + 1, z + 1)]).collect()
}

/// Box-filtered mip chain of a `width` x `height` image of `channels` interleaved floats per
/// texel, level 0 first (as [`byte_mips`]).
fn float_mips(width: u32, height: u32, channels: usize, level0: &[f32]) -> Vec<(u32, u32, Vec<f32>)> {
    let mut levels = vec![(width, height, level0.to_vec())];
    while let Some(&(w, h, _)) = levels.last().filter(|(w, h, _)| *w > 1 || *h > 1) {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        levels.push((nw, nh, vec![0.0; (nw * nh) as usize * channels]));
    }
    update_float_mips(&mut levels, channels, [0, 0, width, height]);
    levels
}

/// Refilters the coarser levels of `levels` (as [`float_mips`] made them) under `rect` of level
/// 0; returns the rectangle changed at each level.
fn update_float_mips(levels: &mut [(u32, u32, Vec<f32>)], channels: usize, rect: [u32; 4]) -> Vec<[u32; 4]> {
    let mut rects = vec![rect];
    for level in 1..levels.len() {
        let (pw, ph) = (levels[level - 1].0, levels[level - 1].1);
        let (lw, lh) = (levels[level].0, levels[level].1);
        let r = parent_rect(rects[level - 1], lw, lh);
        let (upper, lower) = levels.split_at_mut(level);
        let (src, dst) = (&upper[level - 1].2, &mut lower[0].2);
        for y in r[1]..r[3] {
            for x in r[0]..r[2] {
                for c in 0..channels {
                    let mut sum = 0.0;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = ((x * 2 + dx).min(pw - 1), (y * 2 + dy).min(ph - 1));
                        sum += src[(sy * pw + sx) as usize * channels + c];
                    }
                    dst[(y * lw + x) as usize * channels + c] = sum * 0.25;
                }
            }
        }
        rects.push(r);
    }
    rects
}

/// `values` as half floats (little-endian), rounded to the nearest.
fn halves(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|&v| half_bits(v).to_le_bytes()).collect()
}

/// `x` (0 or a normal number below 65504) as IEEE half-precision bits, rounded to the nearest
/// (a carry out of the mantissa raises the exponent, as it should).
fn half_bits(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32 - 127 + 15;
    if exp <= 0 {
        return sign;
    }
    if exp >= 31 {
        return sign | 0x7BFF;
    }
    let mantissa = (bits & 0x7F_FFFF) + 0x1000;
    sign | (((exp as u32) << 10) + (mantissa >> 13)).min(0x7BFF) as u16
}

/// Uploads the texels of `rect` of a level `width` wide holding `channels` floats a texel, as
/// half floats.
fn write_float_region(queue: &wgpu::Queue, texture: &wgpu::Texture, level: u32, width: u32, data: &[f32], channels: usize, rect: [u32; 4]) {
    let [x0, y0, x1, y1] = rect;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let mut packed = Vec::with_capacity((x1 - x0) as usize * (y1 - y0) as usize * channels * 2);
    for y in y0..y1 {
        let start = (y * width + x0) as usize * channels;
        packed.extend(halves(&data[start..start + (x1 - x0) as usize * channels]));
    }
    write_packed(queue, texture, level, rect, &packed, channels as u32 * 2);
}

impl TerrainPipeline {
    pub fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout, detail: &crate::terrain_detail::TerrainDetail, hdr_format: wgpu::TextureFormat, samples: u32) -> Self {
        use wgpu::{BindingType as B, ShaderStages as S, TextureSampleType as T};
        let tex = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: S::VERTEX | S::FRAGMENT,
            ty: B::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: S::VERTEX | S::FRAGMENT,
                    ty: B::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex(1, T::Float { filterable: true }),
                tex(2, T::Uint),
                tex(3, T::Float { filterable: true }),
                tex(4, T::Float { filterable: true }),
                // Vertex stage too: the extended ground samples the heightmap's mips.
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: S::VERTEX | S::FRAGMENT,
                    ty: B::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // What lies round the map (`Surroundings`): the far land's heights, coasts and
                // colours.
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: S::VERTEX | S::FRAGMENT,
                    ty: B::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // The map's plain ground tiles (`PlainTiles`).
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: S::FRAGMENT,
                    ty: B::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // The terrain's ambient occlusion (sky visibility per heightmap vertex).
                tex(11, T::Float { filterable: true }),
                // Map overlays (metal, movement, sight): a colour per heightmap cell.
                tex(12, T::Float { filterable: true }),
            ],
        });
        let shader = crate::util::shader(device, "terrain", &shader_source_for(false));
        let skirt_shader = crate::util::shader(device, "terrain skirt", &shader_source_for(true));
        // (The detail materials in a group of their own: the water's pipeline shares the
        // terrain's group, and already binds all the textures a stage may have.)
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(frame_layout), Some(&layout), Some(&detail.layout)],
            immediate_size: 0,
        });
        let chunk_instances = wgpu::VertexBufferLayout {
            array_stride: 8,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Uint32x2],
        };
        // The scene pass's pipelines, or with `mirrored` those of the reflection pass (`shader`:
        // the map's or the skirt's).
        let make = |label: &str, shader: &wgpu::ShaderModule, vs: &str, buffers: &[Option<wgpu::VertexBufferLayout<'_>>], mirrored: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some(vs),
                    buffers,
                    compilation_options: Default::default(),
                },
                primitive: if mirrored { crate::reflection::primitive() } else { wgpu::PrimitiveState { cull_mode: None, ..Default::default() } },
                depth_stencil: Some(crate::util::depth_state(true)),
                multisample: wgpu::MultisampleState { count: if mirrored { 1 } else { samples }, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(if mirrored { "fs_reflection" } else { "fs_main" }),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: hdr_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let chunks = [Some(chunk_instances)];
        let skirt_vertices = [Some(crate::sky::SKIRT_VERTEX_LAYOUT)];
        let prepass = crate::util::ray_traced().then(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("terrain depth prepass"),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &chunks, compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(crate::util::depth_state(true)),
                multisample: wgpu::MultisampleState::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            })
        });
        TerrainPipeline {
            layout,
            pipeline: make("terrain", &shader, "vs_main", &chunks, false),
            skirt: make("terrain skirt", &skirt_shader, "vs_skirt", &skirt_vertices, false),
            reflection: make("terrain reflection", &shader, "vs_main", &chunks, true),
            skirt_reflection: make("terrain skirt reflection", &skirt_shader, "vs_skirt", &skirt_vertices, true),
            prepass,
            detail: detail.bind_group.clone(),
        }
    }
}

impl TerrainGpu {
    /// Why `map`'s ground can't be drawn with textures of at most `max` texels a side, if so:
    /// its heightmap (and the maps beside it) take a texel a cell, its tile atlas 128 tiles of
    /// 32 px across. (Maps go to 16384 cells a side; some cards take 8192.)
    pub fn fits(map: &TntMap, max: u32) -> Result<(), String> {
        let tiles = map.tiles.len().max(1) as u32;
        let atlas_height = tiles.div_ceil(tiles.min(128)) * TILE_SIZE as u32;
        if map.width.max(map.height) > max {
            return Err(format!("the map is {} x {} cells, and this graphics card draws ground of at most {max} cells a side", map.width, map.height));
        }
        if atlas_height > max {
            return Err(format!("the map has {tiles} tiles, and this graphics card holds at most {}", max / TILE_SIZE as u32 * 128));
        }
        Ok(())
    }

    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, pipeline: &TerrainPipeline, map: &TntMap, palette: &Palette) -> Self {
        let lut = srgb_to_linear_lut();
        let tiles_x = map.width / 2;
        let tiles_y = map.height / 2;
        let tile_count = map.tiles.len().max(1) as u32;
        let atlas_cols = tile_count.min(128);
        let atlas_rows = tile_count.div_ceil(atlas_cols);
        let (aw, ah) = (atlas_cols * TILE_SIZE as u32, atlas_rows * TILE_SIZE as u32);

        // Per-tile colour pyramid in linear space: levels 0..=5 (32 px .. 1 px).
        let mut levels: Vec<Vec<[f32; 3]>> = Vec::with_capacity(6);
        let base: Vec<[f32; 3]> = map
            .tiles
            .iter()
            .flat_map(|t| {
                t.iter().map(|&i| {
                    let [r, g, b] = palette.rgb(i);
                    [lut[r as usize], lut[g as usize], lut[b as usize]]
                })
            })
            .collect();
        levels.push(base);
        for l in 1..=5u32 {
            let src_size = (TILE_SIZE as u32) >> (l - 1);
            let dst_size = src_size / 2;
            let src = &levels[l as usize - 1];
            let mut dst = vec![[0f32; 3]; map.tiles.len() * (dst_size * dst_size) as usize];
            for t in 0..map.tiles.len() {
                let so = t * (src_size * src_size) as usize;
                let d_o = t * (dst_size * dst_size) as usize;
                for y in 0..dst_size as usize {
                    for x in 0..dst_size as usize {
                        let s = |dx: usize, dy: usize| src[so + (y * 2 + dy) * src_size as usize + x * 2 + dx];
                        let (a, b, c, d) = (s(0, 0), s(1, 0), s(0, 1), s(1, 1));
                        dst[d_o + y * dst_size as usize + x] =
                            std::array::from_fn(|k| (a[k] + b[k] + c[k] + d[k]) * 0.25);
                    }
                }
            }
            levels.push(dst);
        }

        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tile atlas"),
            size: wgpu::Extent3d { width: aw, height: ah, depth_or_array_layers: 1 },
            mip_level_count: ATLAS_LEVELS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for l in 0..ATLAS_LEVELS {
            let ts = (TILE_SIZE as u32) >> l;
            let (lw, lh) = (aw >> l, ah >> l);
            let mut rgba = vec![0u8; (lw * lh * 4) as usize];
            let src = &levels[l as usize];
            for t in 0..map.tiles.len() as u32 {
                let (sx, sy) = ((t % atlas_cols) * ts, (t / atlas_cols) * ts);
                for y in 0..ts {
                    for x in 0..ts {
                        let c = src[(t * ts * ts + y * ts + x) as usize];
                        let o = (((sy + y) * lw + sx + x) * 4) as usize;
                        rgba[o..o + 4].copy_from_slice(&[linear_to_srgb(c[0]), linear_to_srgb(c[1]), linear_to_srgb(c[2]), 255]);
                    }
                }
            }
            crate::util::write_texture_level(queue, &atlas, l, lw, lh, &rgba, 4);
        }

        // The tiles as shown: TA's unseen strip cleared of junk (see `shown_tiles`).
        let shown = shown_tiles(&map.tile_map, tiles_x as usize, tiles_y as usize, &levels[5]);

        // Average-colour image indexed by map tile position (virtual mip level 5), with mips.
        let avg_levels = 32 - tiles_x.max(tiles_y).max(1).leading_zeros();
        let tile_avg = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tile averages"),
            size: wgpu::Extent3d { width: tiles_x, height: tiles_y, depth_or_array_layers: 1 },
            mip_level_count: avg_levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut cur: Vec<[f32; 3]> =
            shown.iter().map(|&t| levels[5].get(t as usize).copied().unwrap_or([0.0; 3])).collect();
        let (mut cw, mut ch) = (tiles_x, tiles_y);
        let mut avg_store = Vec::with_capacity(avg_levels as usize);
        for l in 0..avg_levels {
            avg_store.push((cw, ch, cur.clone()));
            let rgba: Vec<u8> = cur
                .iter()
                .flat_map(|c| [linear_to_srgb(c[0]), linear_to_srgb(c[1]), linear_to_srgb(c[2]), 255])
                .collect();
            crate::util::write_texture_level(queue, &tile_avg, l, cw, ch, &rgba, 4);
            let (nw, nh) = ((cw / 2).max(1), (ch / 2).max(1));
            let mut next = vec![[0f32; 3]; (nw * nh) as usize];
            for y in 0..nh {
                for x in 0..nw {
                    let mut acc = [0f32; 3];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(cw - 1);
                        let sy = (y * 2 + dy).min(ch - 1);
                        let c = cur[(sy * cw + sx) as usize];
                        for k in 0..3 {
                            acc[k] += c[k] * 0.25;
                        }
                    }
                    next[(y * nw + x) as usize] = acc;
                }
            }
            cur = next;
            (cw, ch) = (nw, nh);
        }
        // The last level is the mean colour of the whole map.
        let average_color = cur.first().copied().unwrap_or([0.0; 3]);
        let (land_key, land_balance) = land_key(map, &shown, &levels[5]);

        // The heights the ground is drawn at (see terrain_surface.rs; a metal or urban world's
        // squarer ones are set by `set_built`).
        let sea = map.sea_level as f32;
        let started = std::time::Instant::now();
        let display = crate::terrain_surface::display_heights(map.width, map.height, &map.heights, sea, false);
        log::info!("terrain surface smoothed in {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
        // Heightmap (r = the drawn height / 255, in half floats: drawn heights are not whole
        // units) and "under sea level" mask (g) with box-filtered mips: the terrain reads the
        // heights of level 0 exactly; the extended ground beyond the map samples coarser levels
        // further out, where g is the fraction of the filtered area under water.
        let height_mips = float_mips(map.width, map.height, 2, &height_texels(&display, sea));
        let heightmap = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("heightmap"),
            size: wgpu::Extent3d { width: map.width, height: map.height, depth_or_array_layers: 1 },
            mip_level_count: height_mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (w, h, data)) in height_mips.iter().enumerate() {
            crate::util::write_texture_level(queue, &heightmap, level as u32, *w, *h, &halves(data), 4);
        }
        // Ambient occlusion of the relief, baked once (see occlusion.rs), and next to it each
        // vertex's distance from the waterline (`water::shore_distances`; a texture of its own
        // would be one more than the water's shaders may bind).
        let started = std::time::Instant::now();
        let visibility = crate::occlusion::terrain_visibility(map.width, map.height, &map.heights);
        let shore = crate::water::shore_distances(map.width, map.height, &display, sea, [0, 0, map.width, map.height]);
        log::info!("terrain occlusion and shore distances baked in {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
        let occlusion_texels: Vec<u8> = visibility.iter().zip(&shore).flat_map(|(&v, &d)| [v, d]).collect();
        let occlusion = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("terrain occlusion"),
                size: wgpu::Extent3d { width: map.width, height: map.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rg8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &occlusion_texels,
        );

        let field = HeightField { width: map.width, height: map.height, heights: map.heights.clone(), sea_level: map.sea_level as f32, display };
        set_drawn(&field);
        // The ground beyond the map mirrors the map's next to it, seas and all; further out its
        // seas are those round the map's edges.
        let sea_beyond = map.heights.iter().any(|&h| (h as f32) < sea);
        let around = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor {
            label: Some("surroundings"),
            contents: bytemuck::bytes_of(&crate::surroundings::Surroundings::measure(map.width, map.height, &field.display, sea, &avg_store[0].2)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // The map's plain ground tiles: the most used tiles that are flat at a 4x4 texel scale,
        // i.e. carry texture but no feature (the broad look recolours the ground showing them).
        let mut uses = vec![0u32; map.tiles.len()];
        for &t in &shown {
            if let Some(n) = uses.get_mut(t as usize) {
                *n += 1;
            }
        }
        let luminance = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let plain = |t: usize| {
            let mean = luminance(levels[5][t]).max(0.02);
            levels[3][t * 16..t * 16 + 16].iter().all(|&c| (luminance(c) - mean).abs() < 0.2 * mean)
        };
        let mut order: Vec<usize> = (0..map.tiles.len()).filter(|&t| uses[t] >= 3 && plain(t)).collect();
        order.sort_by(|&a, &b| uses[b].cmp(&uses[a]).then(a.cmp(&b)));
        let plain_tiles = PlainTiles::new(order.iter().map(|&t| levels[5][t]));
        let plain_buf = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor {
            label: Some("plain tiles"),
            contents: bytemuck::bytes_of(&plain_tiles),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let tile_map = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("tile map"),
                size: wgpu::Extent3d { width: tiles_x, height: tiles_y, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R16Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytemuck::cast_slice(&shown),
        );

        let params = TerrainParams {
            cells: [map.width, map.height],
            tiles: [tiles_x, tiles_y],
            atlas_cols,
            chunk_cells: CHUNK_CELLS,
            detail: 0.6,
            _pad: 0,
        };
        let params_buf = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("terrain linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let overlay = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("map overlay"),
            size: wgpu::Extent3d { width: map.width, height: map.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let height_view = view(&heightmap);
        let tile_colors = view(&tile_avg);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view(&heightmap)) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view(&tile_map)) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&view(&atlas)) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&view(&tile_avg)) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 6, resource: around.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: plain_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&view(&occlusion)) },
                wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(&view(&overlay)) },
            ],
        });

        let chunks = UVec2::new(map.width.div_ceil(CHUNK_CELLS), map.height.div_ceil(CHUNK_CELLS));
        // Room for two lists: the scene's chunks and the reflection's.
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain chunks"),
            size: (chunks.x * chunks.y * 8 * 2) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let chunk_tops = chunk_tops(&field, chunks);
        let edit = TerrainEdit {
            heightmap,
            height_mips,
            built: false,
            void: map.features.iter().map(|&f| f == ta_formats::tnt::FEATURE_VOID).collect(),
            space: false,
            atlas,
            atlas_cols,
            tiles: map.tiles.iter().map(|t| **t).collect(),
            palette: palette.clone(),
            shown: shown.clone(),
            strands: None,
            strands_cleaned: false,
            occlusion,
            visibility,
            tile_map,
            tile_avg,
            tile_means: levels.swap_remove(5),
            avg_levels: avg_store,
        };

        log::info!(
            "terrain: {}x{} cells, {} tiles (atlas {aw}x{ah}), {} chunks",
            map.width,
            map.height,
            map.tiles.len(),
            chunks.x * chunks.y
        );
        TerrainGpu {
            field,
            average_color,
            land_key,
            land_balance,
            height_view,
            tile_colors,
            sea_beyond,
            around,
            bind_group,
            instance_buffer,
            visible_chunks: 0,
            reflected_chunks: 0,
            chunks,
            chunk_tops,
            overlay,
            cells: (map.width, map.height),
            edit,
            pads: crate::terrain_pads::Pads::new(map.width, map.height, sea),
        }
    }

    /// Uploads a map overlay: one RGBA colour (straight alpha) per heightmap cell, row by row
    /// (`width × height` cells of the map). Shown while [`crate::Renderer::set_overlay`] says so.
    pub fn set_overlay(&self, queue: &wgpu::Queue, rgba: &[u8]) {
        let (w, h) = self.cells;
        if rgba.len() >= (w * h * 4) as usize {
            crate::util::write_texture_level(queue, &self.overlay, 0, w, h, &rgba[..(w * h * 4) as usize], 4);
        }
    }

    /// Heightmap cells per axis.
    pub fn cells(&self) -> (u32, u32) {
        self.cells
    }

    /// Chunks overlapping the ground rectangle `min`..`max` that `keep` (row-major chunk
    /// index) accepts.
    fn chunks_in(&self, min: Vec2, max: Vec2, keep: impl Fn(usize) -> bool) -> Vec<[u32; 2]> {
        let size = CHUNK_CELLS as f32 * 16.0;
        let x0 = ((min.x / size).floor() as i32).clamp(0, self.chunks.x as i32);
        let x1 = ((max.x / size).ceil() as i32).clamp(0, self.chunks.x as i32);
        // Tall terrain near the far edge can poke into view: widen north/south by one chunk.
        let z0 = ((min.y / size).floor() as i32 - 1).clamp(0, self.chunks.y as i32);
        let z1 = ((max.y / size).ceil() as i32 + 1).clamp(0, self.chunks.y as i32);
        let mut list: Vec<[u32; 2]> = Vec::new();
        for z in z0..z1 {
            for x in x0..x1 {
                if keep((z as u32 * self.chunks.x + x as u32) as usize) {
                    list.push([x as u32, z as u32]);
                }
            }
        }
        list
    }

    /// Selects chunks overlapping the visible ground rectangle.
    pub fn cull(&mut self, queue: &wgpu::Queue, min: Vec2, max: Vec2) {
        let list = self.chunks_in(min, max, |_| true);
        if !list.is_empty() {
            queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&list));
        }
        self.visible_chunks = list.len() as u32;
    }

    /// Selects the chunks the reflection pass mirrors: those overlapping the ground rectangle
    /// the mirrored view can see (`min`..`max`) with ground above the sea.
    pub(crate) fn cull_reflection(&mut self, queue: &wgpu::Queue, min: Vec2, max: Vec2) {
        let sea = self.field.sea_level;
        let list = self.chunks_in(min, max, |i| self.chunk_tops[i] as f32 > sea);
        if !list.is_empty() {
            queue.write_buffer(&self.instance_buffer, self.reflection_offset(), bytemuck::cast_slice(&list));
        }
        self.reflected_chunks = list.len() as u32;
    }

    /// Where the reflection's chunk list starts in the instance buffer (bytes).
    fn reflection_offset(&self) -> u64 {
        (self.chunks.x * self.chunks.y * 8) as u64
    }

    /// Heightmap, tile map and atlas bindings (group 1), shared with the water pass.
    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, pipeline: &'a TerrainPipeline) {
        if self.visible_chunks == 0 {
            return;
        }
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_bind_group(2, &pipeline.detail, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..CHUNK_VERTICES, 0..self.visible_chunks);
    }

    /// Draws the visible chunks' depth for ray tracing (raytrace.rs).
    pub(crate) fn draw_prepass<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, pipeline: &'a TerrainPipeline) {
        let Some(prepass) = &pipeline.prepass else { return };
        if self.visible_chunks == 0 {
            return;
        }
        pass.set_pipeline(prepass);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_bind_group(2, &pipeline.detail, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..CHUNK_VERTICES, 0..self.visible_chunks);
    }

    /// Draws the chunks chosen by [`TerrainGpu::cull_reflection`] into the reflection.
    pub(crate) fn draw_reflection<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, pipeline: &'a TerrainPipeline) {
        if self.reflected_chunks == 0 {
            return;
        }
        pass.set_pipeline(&pipeline.reflection);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_bind_group(2, &pipeline.detail, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(self.reflection_offset()..));
        pass.draw(0..CHUNK_VERTICES, 0..self.reflected_chunks);
    }
}

/// Map editing: the heightmap and the tile map change in place, a region at a time, without
/// rebuilding the terrain. What is derived from the whole map when it loads (the ground beyond
/// the edges, the painted sea's colours, the water surface's chunks, the map's mean colour)
/// keeps its load-time state; [`crate::Renderer::load_map`] refreshes everything.
impl TerrainGpu {
    /// Number of tiles in the atlas: [`TerrainGpu::update_tiles`] can show these only.
    pub fn tile_count(&self) -> usize {
        self.edit.tile_means.len()
    }

    /// Copies the heights of cells `x0..x1` × `z0..z1` from `heights` (the whole map's,
    /// row-major) into the height field, the heightmap texture and its mips, the terrain's
    /// ambient occlusion, the distances from the waterline and the chunk bounds.
    pub fn update_heights(&mut self, queue: &wgpu::Queue, heights: &[u8], x0: u32, z0: u32, x1: u32, z1: u32) {
        let (w, h) = (self.field.width, self.field.height);
        let (x0, z0, x1, z1) = (x0.min(w), z0.min(h), x1.min(w), z1.min(h));
        if x0 >= x1 || z0 >= z1 || heights.len() < (w * h) as usize {
            return;
        }
        let sea = self.field.sea_level;
        for z in z0..z1 {
            for x in x0..x1 {
                let i = (z * w + x) as usize;
                self.field.heights[i] = heights[i];
            }
        }
        // The drawn heights change as far as the smoothing reaches (see terrain_surface.rs):
        // `DRAWN_REACH` cells round the change, computed over twice that.
        let grow = |n: u32| [x0.saturating_sub(n), z0.saturating_sub(n), (x1 + n).min(w), (z1 + n).min(h)];
        let [ix0, iz0, ix1, iz1] = grow(DRAWN_REACH);
        let outer = grow(2 * DRAWN_REACH);
        let ow = outer[2] - outer[0];
        let window: Vec<u8> = (outer[1]..outer[3]).flat_map(|z| self.field.heights[(z * w + outer[0]) as usize..(z * w + outer[2]) as usize].iter().copied()).collect();
        let drawn = crate::terrain_surface::display_heights(ow, outer[3] - outer[1], &window, sea, self.edit.built);
        // (Under the pads, if any, the ground without them: they are laid on it again.)
        let natural = match self.pads.natural_mut() {
            Some(natural) => natural,
            None => &mut self.field.display,
        };
        for z in iz0..iz1 {
            for x in ix0..ix1 {
                let (i, j) = ((z * w + x) as usize, ((z - outer[1]) * ow + x - outer[0]) as usize);
                natural[i] = drawn[j];
            }
        }
        let changed = [ix0, iz0, ix1, iz1];
        self.pads.apply(changed.map(|v| v as i32), &mut self.field.display);
        let e = &mut self.edit;
        for z in iz0..iz1 {
            for x in ix0..ix1 {
                let i = (z * w + x) as usize;
                let d = self.field.display[i];
                e.height_mips[0].2[i * 2] = d / 255.0;
                e.height_mips[0].2[i * 2 + 1] = if d < sea { 1.0 } else { 0.0 };
            }
        }
        for (level, rect) in update_float_mips(&mut e.height_mips, 2, changed).into_iter().enumerate() {
            write_float_region(queue, &e.heightmap, level as u32, e.height_mips[level].0, &e.height_mips[level].2, 2, rect);
        }
        // (The occlusion of the relief, from TA's heights, reaches further than the change; the
        // waterline's distances, from the drawn ones, 3 cells further than they change.)
        let (rect, occlusion) = occlusion_around(w, h, &self.field.heights, changed);
        for z in rect[1]..rect[3] {
            let (row, at) = (((z - rect[1]) * (rect[2] - rect[0])) as usize, (z * w + rect[0]) as usize);
            let n = (rect[2] - rect[0]) as usize;
            e.visibility[at..at + n].copy_from_slice(&occlusion[row..row + n]);
        }
        let shore = crate::water::shore_distances(w, h, &self.field.display, sea, rect);
        let texels: Vec<u8> = occlusion.iter().zip(&shore).flat_map(|(&v, &d)| [v, d]).collect();
        write_packed(queue, &e.occlusion, 0, rect, &texels, 2);
        set_drawn(&self.field);
        self.measure_surroundings(queue);
        // Chunk bounds (a chunk's vertices include its far edges).
        let (cx0, cz0) = (ix0.saturating_sub(1) / CHUNK_CELLS, iz0.saturating_sub(1) / CHUNK_CELLS);
        let (cx1, cz1) = ((ix1 / CHUNK_CELLS + 1).min(self.chunks.x), (iz1 / CHUNK_CELLS + 1).min(self.chunks.y));
        for cz in cz0..cz1 {
            for cx in cx0..cx1 {
                let mut top = 0.0f32;
                for z in cz * CHUNK_CELLS..=(cz + 1) * CHUNK_CELLS {
                    for x in cx * CHUNK_CELLS..=(cx + 1) * CHUNK_CELLS {
                        top = top.max(self.field.vertex(x as i32, z as i32));
                    }
                }
                self.chunk_tops[(cz * self.chunks.x + cx) as usize] = top as u8;
            }
        }
    }

    /// Draws the ground as a metal or urban world's (`built`: walls and terraces built square,
    /// smoothed less, see terrain_surface.rs) or as any other world's.
    pub fn set_built(&mut self, queue: &wgpu::Queue, built: bool) {
        if self.edit.built == built {
            return;
        }
        self.edit.built = built;
        self.redraw(queue);
    }

    /// Whether the map is a world in space: its void cells drawn as nothing (the sky showing
    /// through), the rocks' edges as a slab [`VOID_SLAB`] thick, rather than as their painting
    /// (the classic view still draws that).
    pub fn set_space(&mut self, queue: &wgpu::Queue, space: bool) {
        let space = space && self.edit.void.iter().any(|&v| v);
        if self.edit.space == space {
            return;
        }
        self.edit.space = space;
        self.redraw(queue);
    }

    /// The drawn heights made again from the map's (as built and in space or not), and
    /// everything that follows them uploaded.
    fn redraw(&mut self, queue: &wgpu::Queue) {
        let built = self.edit.built;
        let (w, h) = (self.field.width, self.field.height);
        let sea = self.field.sea_level;
        let mut drawn = crate::terrain_surface::display_heights(w, h, &self.field.heights, sea, built);
        let marks = if self.edit.space { space_ground(&mut drawn, &self.edit.void, w as usize, h as usize) } else { Vec::new() };
        // (The pads, if any, laid on it again.)
        match self.pads.natural_mut() {
            Some(natural) => {
                *natural = drawn;
                self.pads.apply([0, 0, w as i32, h as i32], &mut self.field.display);
            }
            None => self.field.display = drawn,
        }
        let e = &mut self.edit;
        let mut texels = height_texels(&self.field.display, sea);
        for (i, &m) in marks.iter().enumerate() {
            if m {
                texels[i * 2 + 1] = VOID_MARK;
            }
        }
        e.height_mips = float_mips(w, h, 2, &texels);
        for (level, (lw, lh, data)) in e.height_mips.iter().enumerate() {
            crate::util::write_texture_level(queue, &e.heightmap, level as u32, *lw, *lh, &halves(data), 4);
        }
        let shore = crate::water::shore_distances(w, h, &self.field.display, sea, [0, 0, w, h]);
        e.visibility = crate::occlusion::terrain_visibility(w, h, &self.field.heights);
        let texels: Vec<u8> = e.visibility.iter().zip(&shore).flat_map(|(&v, &d)| [v, d]).collect();
        write_packed(queue, &e.occlusion, 0, [0, 0, w, h], &texels, 2);
        self.chunk_tops = chunk_tops(&self.field, self.chunks);
        set_drawn(&self.field);
        self.measure_surroundings(queue);
    }

    /// Measures what lies round the map again (see surroundings.rs), after its heights or its
    /// tiles changed.
    fn measure_surroundings(&self, queue: &wgpu::Queue) {
        let around = crate::surroundings::Surroundings::measure(self.field.width, self.field.height, &self.field.display, self.field.sea_level, &self.edit.avg_levels[0].2);
        queue.write_buffer(&self.around, 0, bytemuck::bytes_of(&around));
    }

    /// Levels the drawn ground under buildings (see terrain_pads.rs): each pad's footprint drawn
    /// at its base, the ground round it banked away to its own. Only the ground the new pads
    /// reach changes: in the height field, the heightmap and its levels, the waterline's
    /// distances, the chunk bounds and what stands on the ground ([`drawn_height`]). What
    /// changed, if anything did (the ground map every pass reads is to be filled again there).
    pub fn add_pads(&mut self, queue: &wgpu::Queue, pads: &[GroundPad]) -> Option<PadChange> {
        let mut rects = Vec::new();
        for &pad in pads {
            if let Some(rect) = self.pads.add(&self.field.display, pad) {
                rects.push(rect);
            }
        }
        let top = pads.iter().map(|p| p.height).fold(f32::MIN, f32::max);
        self.relevel(queue, rects, top)
    }

    /// Takes up the pads laid after `tick` (the game went back to it, see [`Self::add_pads`]).
    pub fn forget_pads_after(&mut self, queue: &wgpu::Queue, tick: u64) -> Option<PadChange> {
        let rects = self.pads.retain_until(tick);
        self.relevel(queue, rects, f32::MIN)
    }

    /// Draws the ground with the pads again over `rects` (vertices, `[x0, z0, x1, z1)`), and
    /// pushes what changed everywhere it is drawn.
    fn relevel(&mut self, queue: &wgpu::Queue, rects: Vec<[i32; 4]>, top: f32) -> Option<PadChange> {
        if rects.is_empty() {
            return None;
        }
        let rects = merged_rects(rects);
        let started = std::time::Instant::now();
        let (mut rise, mut area) = (0.0f32, [f32::MAX, f32::MAX, f32::MIN, f32::MIN]);
        for rect in &rects {
            rise = rise.max(self.pads.apply(*rect, &mut self.field.display));
            let r = rect.map(|v| v.max(0) as u32);
            self.push_drawn(queue, [r[0], r[1], r[2].min(self.field.width), r[3].min(self.field.height)]);
            area = [area[0].min(r[0] as f32 * 16.0), area[1].min(r[1] as f32 * 16.0), area[2].max(r[2] as f32 * 16.0), area[3].max(r[3] as f32 * 16.0)];
        }
        log::debug!("{} pads: {} areas levelled (by up to {rise:.1}) in {:.2} ms", self.pads.len(), rects.len(), started.elapsed().as_secs_f64() * 1000.0);
        Some(PadChange { area, top })
    }

    /// Pushes the drawn heights of the vertices of `rect` (`[x0, z0, x1, z1)`, changed in the
    /// height field) to the heightmap and its levels, the waterline's distances (2 cells round
    /// a vertex reach it), the chunk bounds and what stands on the ground.
    fn push_drawn(&mut self, queue: &wgpu::Queue, rect: [u32; 4]) {
        let (w, h) = (self.field.width, self.field.height);
        let [x0, z0, x1, z1] = rect;
        if x0 >= x1 || z0 >= z1 {
            return;
        }
        let sea = self.field.sea_level;
        let e = &mut self.edit;
        for z in z0..z1 {
            for x in x0..x1 {
                let i = (z * w + x) as usize;
                let d = self.field.display[i];
                e.height_mips[0].2[i * 2] = d / 255.0;
                e.height_mips[0].2[i * 2 + 1] = if d < sea { 1.0 } else { 0.0 };
            }
        }
        for (level, r) in update_float_mips(&mut e.height_mips, 2, rect).into_iter().enumerate() {
            write_float_region(queue, &e.heightmap, level as u32, e.height_mips[level].0, &e.height_mips[level].2, 2, r);
        }
        let shore_rect = [x0.saturating_sub(3), z0.saturating_sub(3), (x1 + 3).min(w), (z1 + 3).min(h)];
        let shore = crate::water::shore_distances(w, h, &self.field.display, sea, shore_rect);
        let sw = (shore_rect[2] - shore_rect[0]) as usize;
        let mut texels = Vec::with_capacity(shore.len() * 2);
        for (k, &d) in shore.iter().enumerate() {
            let (x, z) = (shore_rect[0] as usize + k % sw, shore_rect[1] as usize + k / sw);
            texels.extend_from_slice(&[e.visibility[z * w as usize + x], d]);
        }
        write_packed(queue, &e.occlusion, 0, shore_rect, &texels, 2);
        // Chunk bounds (a chunk's vertices include its far edges).
        let (cx0, cz0) = (x0.saturating_sub(1) / CHUNK_CELLS, z0.saturating_sub(1) / CHUNK_CELLS);
        let (cx1, cz1) = ((x1 / CHUNK_CELLS + 1).min(self.chunks.x), (z1 / CHUNK_CELLS + 1).min(self.chunks.y));
        for cz in cz0..cz1 {
            for cx in cx0..cx1 {
                let mut top = 0.0f32;
                for z in cz * CHUNK_CELLS..=(cz + 1) * CHUNK_CELLS {
                    for x in cx * CHUNK_CELLS..=(cx + 1) * CHUNK_CELLS {
                        top = top.max(self.field.vertex(x as i32, z as i32));
                    }
                }
                self.chunk_tops[(cz * self.chunks.x + cx) as usize] = top as u8;
            }
        }
        update_drawn(&self.field, rect);
    }

    /// Shows the map's tiles with TA's hair-like strands painted over (see terrain_strands.rs),
    /// or as painted. Found the first time they are asked for.
    pub fn set_strands(&mut self, queue: &wgpu::Queue, cleaned: bool) {
        let e = &mut self.edit;
        if e.strands_cleaned == cleaned || (!cleaned && e.strands.is_none()) {
            return;
        }
        let pixels = |tile: &[u8; TILE_SIZE * TILE_SIZE]| -> crate::terrain_strands::TilePixels { std::array::from_fn(|i| e.palette.rgb(tile[i])) };
        if e.strands.is_none() {
            let started = std::time::Instant::now();
            let tiles: Vec<crate::terrain_strands::TilePixels> = e.tiles.iter().map(&pixels).collect();
            let (w, h) = (self.field.width as usize, self.field.height as usize);
            let found = crate::terrain_strands::clean_strands(&tiles, &e.shown, w / 2, h / 2, &self.field.heights, w, h);
            log::info!("strands painted over in {} of {} tiles in {:.1} ms", found.len(), tiles.len(), started.elapsed().as_secs_f64() * 1000.0);
            e.strands = Some(found);
        }
        for (t, clean) in e.strands.as_deref().unwrap_or_default() {
            let shown = if cleaned { **clean } else { pixels(&e.tiles[*t as usize]) };
            upload_tile(queue, &e.atlas, e.atlas_cols, *t, &shown);
        }
        e.strands_cleaned = cleaned;
    }

    /// Copies the tile indices of 32 px tiles `x0..x1` × `z0..z1` from `tile_map` (the whole
    /// map's, row-major) into the tile map texture and the tile-average image. Tiles past
    /// [`TerrainGpu::tile_count`] show as tile 0 until the map is loaded again. TA's unseen
    /// strip shows as loading the map shows it (see `shown_tiles`): a change at its edge, or in
    /// it, shows the whole strip again.
    pub fn update_tiles(&mut self, queue: &wgpu::Queue, tile_map: &[u16], x0: u32, z0: u32, x1: u32, z1: u32) {
        let (tw, th) = (self.edit.avg_levels[0].0, self.edit.avg_levels[0].1);
        if tile_map.len() < (tw * th) as usize {
            return;
        }
        if x1 + 1 >= tw || z1 + 5 >= th {
            let shown = shown_tiles(tile_map, tw as usize, th as usize, &self.edit.tile_means);
            self.upload_tiles(queue, &shown, x0, z0, x1, z1);
            self.upload_tiles(queue, &shown, tw.saturating_sub(1), 0, tw, th);
            self.upload_tiles(queue, &shown, 0, th.saturating_sub(4), tw, th);
        } else {
            self.upload_tiles(queue, tile_map, x0, z0, x1, z1);
        }
    }

    fn upload_tiles(&mut self, queue: &wgpu::Queue, tile_map: &[u16], x0: u32, z0: u32, x1: u32, z1: u32) {
        let e = &mut self.edit;
        let (tw, th) = (e.avg_levels[0].0, e.avg_levels[0].1);
        let (x0, z0, x1, z1) = (x0.min(tw), z0.min(th), x1.min(tw), z1.min(th));
        if x0 >= x1 || z0 >= z1 {
            return;
        }
        let count = e.tile_means.len();
        let mut indices = Vec::with_capacity(((x1 - x0) * (z1 - z0)) as usize * 2);
        for z in z0..z1 {
            for x in x0..x1 {
                let i = (z * tw + x) as usize;
                let t = if (tile_map[i] as usize) < count { tile_map[i] } else { 0 };
                indices.extend_from_slice(&t.to_le_bytes());
                e.avg_levels[0].2[i] = e.tile_means[t as usize];
            }
        }
        write_packed(queue, &e.tile_map, 0, [x0, z0, x1, z1], &indices, 2);
        // The changed texels of a level as sRGB, row by row.
        let pack = |level: &[[f32; 3]], width: u32, [x0, y0, x1, y1]: [u32; 4]| -> Vec<u8> {
            let mut out = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
            for y in y0..y1 {
                for c in &level[(y * width + x0) as usize..(y * width + x1) as usize] {
                    out.extend_from_slice(&[linear_to_srgb(c[0]), linear_to_srgb(c[1]), linear_to_srgb(c[2]), 255]);
                }
            }
            out
        };
        for (level, rect) in update_color_mips(&mut e.avg_levels, [x0, z0, x1, z1]).into_iter().enumerate() {
            let (width, texels) = (e.avg_levels[level].0, &e.avg_levels[level].2);
            write_packed(queue, &e.tile_avg, level as u32, rect, &pack(texels, width, rect), 4);
        }
        self.measure_surroundings(queue);
    }
}

/// Rectangles to draw the ground again over (`[x0, z0, x1, z1)`), those that overlap or touch
/// made one (their bounding rectangle) until none do: each is drawn and pushed to the GPU on
/// its own, a pad or a few at a time. Many at once (a game loaded, caught up, gone back in) are
/// the one rectangle round them all.
fn merged_rects(mut rects: Vec<[i32; 4]>) -> Vec<[i32; 4]> {
    const MANY: usize = 32;
    let union = |a: [i32; 4], b: [i32; 4]| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])];
    if rects.len() <= MANY {
        let mut merging = true;
        while merging {
            merging = false;
            let mut i = 0;
            while i < rects.len() {
                let mut j = i + 1;
                while j < rects.len() {
                    let (a, b) = (rects[i], rects[j]);
                    if a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3] {
                        rects[i] = union(a, b);
                        rects.swap_remove(j);
                        merging = true;
                    } else {
                        j += 1;
                    }
                }
                i += 1;
            }
        }
    }
    if rects.len() > MANY / 4 * 3 {
        rects = vec![rects.iter().copied().reduce(union).unwrap_or_default()];
    }
    rects
}

/// The texels of the next level down over the changed texels `rect` of a `w` × `h` level
/// (each texel averages a 2 × 2 block), clipped to the next level's `lw` × `lh`.
fn parent_rect([x0, y0, x1, y1]: [u32; 4], lw: u32, lh: u32) -> [u32; 4] {
    [x0 / 2, y0 / 2, x1.div_ceil(2).min(lw), y1.div_ceil(2).min(lh)]
}


/// The same for the tile-average image's levels (linear colours, as `TerrainGpu::new` makes
/// them).
fn update_color_mips(levels: &mut [(u32, u32, Vec<[f32; 3]>)], rect: [u32; 4]) -> Vec<[u32; 4]> {
    let mut rects = vec![rect];
    for level in 1..levels.len() {
        let (pw, ph) = (levels[level - 1].0, levels[level - 1].1);
        let (lw, lh) = (levels[level].0, levels[level].1);
        let r = parent_rect(rects[level - 1], lw, lh);
        let (upper, lower) = levels.split_at_mut(level);
        let (src, dst) = (&upper[level - 1].2, &mut lower[0].2);
        for y in r[1]..r[3] {
            for x in r[0]..r[2] {
                let mut acc = [0f32; 3];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let c = src[((y * 2 + dy).min(ph - 1) * pw + (x * 2 + dx).min(pw - 1)) as usize];
                    for k in 0..3 {
                        acc[k] += c[k] * 0.25;
                    }
                }
                dst[(y * lw + x) as usize] = acc;
            }
        }
        rects.push(r);
    }
    rects
}

/// Ambient occlusion around a change to cells `rect` of a `width` × `height` heightmap: the
/// vertices whose occlusion can change, and their values row by row, equal to a bake of the
/// whole map. A height reaches 16 cells (15 horizon steps and the tangent, then the blur), so
/// the vertices 17 cells around the change are baked again, from a window 17 cells wider still
/// (at the map's edges the window stops there, clamping as the whole map's bake does).
fn occlusion_around(width: u32, height: u32, heights: &[u8], rect: [u32; 4]) -> ([u32; 4], Vec<u8>) {
    const REACH: u32 = 17;
    let grow = |[x0, z0, x1, z1]: [u32; 4]| [x0.saturating_sub(REACH), z0.saturating_sub(REACH), (x1 + REACH).min(width), (z1 + REACH).min(height)];
    let out = grow(rect);
    let win = grow(out);
    let ww = win[2] - win[0];
    let mut window = Vec::with_capacity((ww * (win[3] - win[1])) as usize);
    for z in win[1]..win[3] {
        window.extend_from_slice(&heights[(z * width + win[0]) as usize..(z * width + win[2]) as usize]);
    }
    let visibility = crate::occlusion::terrain_visibility(ww, win[3] - win[1], &window);
    let ow = (out[2] - out[0]) as usize;
    let mut values = Vec::with_capacity(ow * (out[3] - out[1]) as usize);
    for z in out[1]..out[3] {
        let row = ((z - win[1]) * ww + out[0] - win[0]) as usize;
        values.extend_from_slice(&visibility[row..row + ow]);
    }
    (out, values)
}


/// Uploads `packed`, the texels of `rect` (x0, y0, x1, y1) row by row, `bytes` per texel.
fn write_packed(queue: &wgpu::Queue, texture: &wgpu::Texture, level: u32, rect: [u32; 4], packed: &[u8], bytes: u32) {
    let [x0, y0, x1, y1] = rect;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture, mip_level: level, origin: wgpu::Origin3d { x: x0, y: y0, z: 0 }, aspect: wgpu::TextureAspect::All },
        packed,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some((x1 - x0) * bytes), rows_per_image: Some(y1 - y0) },
        wgpu::Extent3d { width: x1 - x0, height: y1 - y0, depth_or_array_layers: 1 },
    );
}

/// Tiles' mean colours, and how far apart tiles look by them: in linear light, as the strip's
/// junk has been judged from the first, and as the screen shows them (sRGB-encoded, worked out
/// when first wanted). In linear light every dark colour lies close to every other: Shore To
/// Shore's grey junk lay 0.180 from its deep sea there, the distance of two sands a shade apart
/// and a hair's breadth either side of [`JUNK_LINEAR`], so most of it was cleared and a tile in
/// eight left (grey rectangles in the water along the map's foot).
struct TileColours<'a> {
    /// Each tile's mean colour (linear), and as the screen shows it once asked for.
    means: &'a [[f32; 3]],
    shown: Vec<Option<[f32; 3]>>,
}

impl<'a> TileColours<'a> {
    fn new(means: &'a [[f32; 3]]) -> TileColours<'a> {
        TileColours { means, shown: vec![None; means.len()] }
    }

    /// Tile `t`'s colour as the screen shows it (black for a tile `means` has none of).
    fn shown(&mut self, t: u16) -> [f32; 3] {
        let Some(slot) = self.shown.get_mut(t as usize) else { return [0.0; 3] };
        *slot.get_or_insert_with(|| self.means[t as usize].map(srgb_encode))
    }

    /// The distance between tiles `a` and `b`'s mean colours in linear light.
    fn linear_distance(&self, a: u16, b: u16) -> f32 {
        let mean = |t: u16| self.means.get(t as usize).copied().unwrap_or([0.0; 3]);
        let (a, b) = (mean(a), mean(b));
        (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt()
    }

    /// How far apart tiles `a` and `b` look: 0 alike, about 1.7 black against white.
    fn distance(&mut self, a: u16, b: u16) -> f32 {
        let (a, b) = (self.shown(a), self.shown(b));
        (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt()
    }
}

/// A linear colour channel as the screen shows it (0..1).
fn srgb_encode(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Linear-RGB distance past which a tile in TA's unseen strip is junk next to the shown tile
/// beside it (neighbouring painted ground differs by a tenth or so).
const JUNK_LINEAR: f32 = 0.18;

/// [`TileColours::distance`] past which a tile in the strip is junk too if it looks like none of
/// the shown ground round it: the dark ground a linear distance cannot tell apart (Shore To
/// Shore's flat grey lies 0.35 from its sea's blues, two shades of one sea 0.01). High, so that
/// only what is plainly foreign goes: across the install's 275 maps, 0.18 would clear 629 more
/// tiles in 69 maps, 0.26 324 in 21 (a coast crossing the strip, painted as it runs, among
/// them), 0.30 262 in 10 (247 of them Shore To Shore's).
const JUNK_DISTANCE: f32 = 0.30;

/// How many tiles either side of a tile of the unseen strip are looked at for the shown ground
/// like it, in each of the last [`JUNK_ROWS`] shown rows (or columns).
const JUNK_REACH: usize = 2;
const JUNK_ROWS: usize = 2;

/// The tile map as the terrain shows it (`tw` × `th` tiles; `means`: each tile's linear
/// colour). TA never showed the last 2 columns and the last 8 rows of cells (one tile
/// column, four tile rows): its scrolling and its minimaps stop short of them, and many maps
/// leave junk there (Painted Desert's last column is tile 0, flat blue, from its 32nd row
/// down). A tile there is junk when its colour is far from the shown tile beside it (in the last
/// column, the one to its left; in the last rows, the last shown one above: [`JUNK_LINEAR`]), or
/// when it looks like none of the shown ground near it (within [`JUNK_REACH`] tiles either way
/// in the last [`JUNK_ROWS`] shown rows or columns: [`TileColours::distance`] past
/// [`JUNK_DISTANCE`] from every one). Junk is replaced by the shown ground mirrored at the edge
/// (the tile to its left; in the last rows, the tile as far above the edge as it lies below),
/// tile by tile; the rest, painted like the ground beside them, stay.
fn shown_tiles(tile_map: &[u16], tw: usize, th: usize, means: &[[f32; 3]]) -> Vec<u16> {
    shown_tiles_with(tile_map, tw, th, means, JUNK_DISTANCE)
}

/// [`shown_tiles`] with its own [`JUNK_DISTANCE`] (the survey in the tests tries several).
fn shown_tiles_with(tile_map: &[u16], tw: usize, th: usize, means: &[[f32; 3]], junk_distance: f32) -> Vec<u16> {
    let mut out = tile_map.to_vec();
    if tw < 2 || th < 5 || out.len() < tw * th {
        return out;
    }
    let mut colours = TileColours::new(means);
    // (The shown ground is never changed here, so it is always read from the map as it came.)
    let mut near = Vec::with_capacity(JUNK_ROWS * (2 * JUNK_REACH + 1));
    let (shown_rows, last) = (th - 4, tw - 1);
    for y in 0..shown_rows {
        near.clear();
        for x in last.saturating_sub(JUNK_ROWS)..last {
            for yy in y.saturating_sub(JUNK_REACH)..=(y + JUNK_REACH).min(shown_rows - 1) {
                near.push(tile_map[yy * tw + x]);
            }
        }
        let (strip, beside) = (tile_map[y * tw + last], tile_map[y * tw + last - 1]);
        if colours.linear_distance(strip, beside) > JUNK_LINEAR || near.iter().all(|&n| colours.distance(strip, n) > junk_distance) {
            out[y * tw + last] = out[y * tw + last - 1];
        }
    }
    for (k, y) in (th - 4..th).enumerate() {
        for x in 0..tw {
            let x_shown = x.min(tw - 2);
            near.clear();
            for yy in shown_rows.saturating_sub(JUNK_ROWS)..shown_rows {
                for xx in x_shown.saturating_sub(JUNK_REACH)..=(x_shown + JUNK_REACH).min(tw - 2) {
                    near.push(tile_map[yy * tw + xx]);
                }
            }
            let (strip, above) = (tile_map[y * tw + x], tile_map[(shown_rows - 1) * tw + x_shown]);
            if colours.linear_distance(strip, above) > JUNK_LINEAR || near.iter().all(|&n| colours.distance(strip, n) > junk_distance) {
                out[y * tw + x] = out[(shown_rows - 1).saturating_sub(k) * tw + x_shown];
            }
        }
    }
    out
}

/// Highest vertex of every chunk, row-major (a chunk's vertices include its far edges, which
/// it shares with its neighbours).
fn chunk_tops(field: &HeightField, chunks: UVec2) -> Vec<u8> {
    let mut tops = Vec::with_capacity((chunks.x * chunks.y) as usize);
    for cz in 0..chunks.y {
        for cx in 0..chunks.x {
            let mut top = 0.0f32;
            for z in cz * CHUNK_CELLS..=(cz + 1) * CHUNK_CELLS {
                for x in cx * CHUNK_CELLS..=(cx + 1) * CHUNK_CELLS {
                    top = top.max(field.vertex(x as i32, z as i32));
                }
            }
            tops.push(top as u8);
        }
    }
    tops
}

#[cfg(test)]
mod space_tests {
    use super::*;

    #[test]
    fn a_rock_in_space_ends_in_a_slab() {
        // A 6 x 6 map, a rock of 2 x 2 at 100 in the middle, the rest void.
        let (w, h) = (6usize, 6usize);
        let void: Vec<bool> = (0..w * h).map(|i| !((2..4).contains(&(i % w)) && (2..4).contains(&(i / w)))).collect();
        let mut drawn: Vec<f32> = void.iter().map(|&v| if v { 0.0 } else { 100.0 }).collect();
        let marks = space_ground(&mut drawn, &void, w, h);
        // The rock as it was, the void round it a slab below it, the void beyond as it was.
        assert_eq!(drawn[2 * w + 2], 100.0);
        assert_eq!(drawn[w + 1], 100.0 - VOID_SLAB);
        assert_eq!(drawn[0], 0.0);
        // Left out: the cells whose corners are all void (none touching the rock).
        assert!(marks[0] && !marks[w + 1] && !marks[2 * w + 2] && !marks[3 * w + 3]);
        assert!(marks[5 * w + 5] && marks[4 * w]);
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    #[test]
    fn steep_cells_split_along_their_level_diagonal() {
        // A lip rising from the north-west corner to the south-east one (running south-west to
        // north-east): split along the other diagonal, the slope is a plane through the cell.
        let lip = [0.0, 20.0, 20.0, 40.0];
        assert!(split_flipped(lip));
        for (tx, tz) in [(0.5f32, 0.5f32), (0.25, 0.75), (0.9, 0.3), (0.1, 0.2)] {
            assert!((cell_height(lip, tx, tz) - 40.0 * (tx + tz) / 2.0).abs() < 1e-4, "{tx}, {tz}");
        }
        // Gentle ground, and a slope the first split already draws, keep it.
        assert!(!split_flipped([0.0, 4.0, 4.0, 14.0]));
        assert!(!split_flipped([20.0, 0.0, 40.0, 20.0]));
        // Both splits meet every corner, and agree along the cell's sides (no cracks).
        for h in [lip, [5.0, 60.0, 9.0, 31.0], [80.0, 10.0, 12.0, 79.0]] {
            for (tx, tz, want) in [(0.0, 0.0, h[0]), (1.0, 0.0, h[1]), (0.0, 1.0, h[2]), (1.0, 1.0, h[3])] {
                assert!((cell_height(h, tx, tz) - want).abs() < 1e-4);
            }
            for t in [0.2f32, 0.5, 0.8] {
                assert!((cell_height(h, t, 0.0) - (h[0] + (h[1] - h[0]) * t)).abs() < 1e-4);
                assert!((cell_height(h, 1.0, t) - (h[1] + (h[3] - h[1]) * t)).abs() < 1e-4);
            }
        }
        // The height field draws the same.
        let field = HeightField { width: 2, height: 2, heights: vec![0, 20, 20, 40], sea_level: 0.0, display: Vec::new() };
        assert!((field.sample(8.0, 8.0) - 20.0).abs() < 1e-4);
    }

    #[test]
    fn junk_in_the_strip_ta_never_showed_becomes_the_ground_beside_it() {
        // 4 x 7 tiles: 0 sand, 1 darker sand, 2 blue junk. The last column and the last
        // four rows are the unseen strip.
        let means = [[0.35, 0.22, 0.12], [0.30, 0.20, 0.11], [0.09, 0.14, 0.47]];
        #[rustfmt::skip]
        let map = [
            1, 0, 1, 2,
            0, 1, 0, 2,
            1, 0, 1, 1,
            0, 0, 1, 2,
            0, 2, 0, 0,
            2, 2, 1, 2,
            0, 0, 0, 0,
        ];
        let shown = shown_tiles(&map, 4, 7, &means);
        assert_eq!(&shown[..8], &[1, 0, 1, 1, 0, 1, 0, 0], "junk in the last column: the tile to its left");
        assert_eq!(&shown[8..12], &[1, 0, 1, 1], "ground like its neighbour stays");
        // The last four rows, junk judged against the last shown row (1, 0, 1, 1) and
        // replaced by the rows above mirrored at the edge: the first by that row, the second
        // by the one above it, and so on.
        #[rustfmt::skip]
        assert_eq!(&shown[12..], &[
            0, 0, 1, 1,
            0, 1, 0, 0,
            1, 0, 1, 1,
            0, 0, 0, 0,
        ]);
    }

    /// Shore To Shore's foot, 12 x 9 tiles: five rows of deep sea (tiles 0 and 1, two shades of
    /// it) over four rows of grey junk (tiles 2 and 3, a shade apart), the means as measured
    /// there: the grey lies 0.1800 to 0.1810 from the sea in linear light.
    fn shore_to_shore() -> (Vec<u16>, [[f32; 3]; 4]) {
        let means = [[0.015, 0.085, 0.207], [0.014, 0.083, 0.204], [0.164, 0.168, 0.150], [0.163, 0.167, 0.149]];
        let mut map = Vec::new();
        for y in 0..9 {
            for x in 0..12 {
                map.push(if y < 5 { (x % 2) as u16 } else { 2 + (x % 8 == 0) as u16 });
            }
        }
        (map, means)
    }

    #[test]
    fn grey_junk_by_a_dark_sea_is_cleared_whatever_its_hair_of_distance() {
        let (map, means) = shore_to_shore();
        let linear = |a: u16, b: u16| (0..3).map(|k| (means[a as usize][k] - means[b as usize][k]).powi(2)).sum::<f32>().sqrt();
        assert!((0.175..0.185).contains(&linear(0, 2)) && (0.175..0.185).contains(&linear(1, 3)), "the knife edge the old threshold sat on");
        let shown = shown_tiles(&map, 12, 9, &means);
        assert!(shown[..5 * 12] == map[..5 * 12], "the shown ground is not touched");
        for (i, &t) in shown.iter().enumerate().skip(5 * 12) {
            assert!(t < 2, "tile {} of the strip (row {}) is still junk: {t}", i % 12, i / 12);
        }
        // Mirrored at the edge: the first strip row is the last shown one, the second the one
        // above it (and the last column's tile is its neighbour's).
        let mirrored = |row: usize| (0..12).map(|x| map[row * 12 + x.min(10)]).collect::<Vec<_>>();
        assert_eq!(&shown[5 * 12..6 * 12], &mirrored(4)[..]);
        assert_eq!(&shown[6 * 12..7 * 12], &mirrored(3)[..]);
    }

    #[test]
    fn a_strip_painted_like_the_ground_round_it_stays() {
        // 0 and 1 dark sea, 2 grey rock, 3 grey junk: the rock and the sea are 0.17 apart in
        // linear light (under the old threshold) and 0.33 as the screen shows them. A coast
        // reaching the foot, rock on the left of the shown rows and sea on the right; in the strip
        // the coast runs on two tiles further east, so rock lies under sea (unlike the tile above
        // it, like the rock a tile or two off). And one grey tile among the sea.
        let means = [[0.015, 0.085, 0.207], [0.014, 0.083, 0.204], [0.15, 0.15, 0.13], [0.164, 0.168, 0.150]];
        #[rustfmt::skip]
        let map = [
            2, 2, 2, 2, 2, 2, 0, 1, 0, 1, 0, 1,
            2, 2, 2, 2, 2, 2, 1, 0, 1, 0, 1, 0,
            2, 2, 2, 2, 2, 2, 0, 1, 0, 1, 0, 1,
            2, 2, 2, 2, 2, 2, 1, 0, 1, 0, 1, 0,
            2, 2, 2, 2, 2, 2, 0, 1, 0, 1, 0, 1,
            2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 1, 0,
            2, 2, 2, 2, 2, 2, 2, 2, 0, 1, 0, 1,
            2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 3, 0,
            2, 2, 2, 2, 2, 2, 2, 2, 0, 1, 0, 1,
        ];
        let mut colours = TileColours::new(&means);
        assert!(colours.linear_distance(0, 2) < JUNK_LINEAR, "the old rule leaves the rock under the sea");
        assert!(colours.distance(0, 2) > JUNK_DISTANCE, "the rock looks unlike the sea");
        let shown = shown_tiles(&map, 12, 9, &means);
        let mut want = map;
        // (The junk: the tile three rows into the strip, so the shown row three above the edge
        // mirrored: the sea tile at (4 - 2, 10).)
        want[7 * 12 + 10] = map[2 * 12 + 10];
        assert_eq!(shown, want, "the strip as painted stays, bar the junk");
        // The same rock under a sea with no rock near it is junk.
        let mut far = map;
        for row in far.chunks_mut(12).take(5) {
            row.fill(0);
        }
        let shown = shown_tiles(&far, 12, 9, &means);
        assert!(shown[5 * 12..].iter().all(|&t| t < 2), "a rock far from any is junk in a sea");
    }

    /// Every installed map's unseen strip, judged as the terrain does and as it was (linear
    /// light, the tile beside it alone): how many tiles each clears, and how many more the
    /// terrain clears at other `JUNK_DISTANCE`s (`cargo test --release -p ta_render --lib
    /// the_unseen_strips -- --ignored --nocapture`).
    #[test]
    #[ignore = "surveys every map of the install"]
    fn the_unseen_strips_of_every_installed_map() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "original").unwrap();
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let lut = srgb_to_linear_lut();
        let old = |tile_map: &[u16], tw: usize, th: usize, means: &[[f32; 3]]| {
            let colours = TileColours::new(means);
            let mut out = tile_map.to_vec();
            for y in 0..th - 4 {
                let (strip, beside) = (y * tw + tw - 1, y * tw + tw - 2);
                if colours.linear_distance(out[strip], out[beside]) > JUNK_LINEAR {
                    out[strip] = out[beside];
                }
            }
            for (k, y) in (th - 4..th).enumerate() {
                for x in 0..tw {
                    let x_shown = x.min(tw - 2);
                    let (strip, above, mirrored) = (y * tw + x, (th - 5) * tw + x_shown, (th - 5).saturating_sub(k) * tw + x_shown);
                    if colours.linear_distance(out[strip], out[above]) > JUNK_LINEAR {
                        out[strip] = out[mirrored];
                    }
                }
            }
            out
        };
        let mut maps = vfs.list("maps", Some("tnt"), true);
        maps.sort();
        let tries = [0.18f32, 0.22, 0.26, 0.30, 0.36, 0.45];
        let mut added = [0usize; 6];
        let mut changed = [0usize; 6];
        let mut lost = 0usize;
        for path in &maps {
            let Ok(map) = ta_formats::tnt::TntMap::parse(&vfs.read(path).unwrap()) else { continue };
            let (tw, th) = ((map.width / 2) as usize, (map.height / 2) as usize);
            if tw < 4 || th < 6 || map.tile_map.len() < tw * th {
                continue;
            }
            let means: Vec<[f32; 3]> = map
                .tiles
                .iter()
                .map(|t| {
                    let mut sum = [0f32; 3];
                    for &i in t.iter() {
                        let [r, g, b] = palette.rgb(i);
                        sum = [sum[0] + lut[r as usize], sum[1] + lut[g as usize], sum[2] + lut[b as usize]];
                    }
                    sum.map(|s| s / (t.len() as f32))
                })
                .collect();
            let was = old(&map.tile_map, tw, th, &means);
            let mut line = format!("{path}: {tw}x{th} tiles, cleared {} before;", (0..tw * th).filter(|&i| was[i] != map.tile_map[i]).count());
            let mut any = false;
            for (n, &t) in tries.iter().enumerate() {
                let now = shown_tiles_with(&map.tile_map, tw, th, &means, t);
                // (Tiles the old rule cleared and this one leaves: none, it only adds.)
                lost += (0..tw * th).filter(|&i| was[i] != map.tile_map[i] && now[i] == map.tile_map[i]).count();
                let more = (0..tw * th).filter(|&i| now[i] != map.tile_map[i] && was[i] == map.tile_map[i]).count();
                added[n] += more;
                changed[n] += (more > 0) as usize;
                any |= more > 0;
                line += &format!(" +{more} at {t}");
            }
            if any {
                eprintln!("{line}");
            }
        }
        eprintln!("{} maps; tiles the old rule cleared and a new one left: {lost}", maps.len());
        for (n, &t) in tries.iter().enumerate() {
            eprintln!("JUNK_DISTANCE {t}: {} more tiles cleared, in {} maps", added[n], changed[n]);
        }
    }

    /// The foot of the maps in `TA_STRIP_MAPS` (names as the install lists them, lower case,
    /// comma apart), as stored, as the strip was shown and as it is, each a picture in
    /// `TA_STRIP_OUT` (the last eight rows of tiles, eight pixels a tile).
    #[test]
    #[ignore = "writes pictures of the maps' unseen strips"]
    fn the_unseen_strips_as_pictures() {
        let (Ok(maps), Some(out)) = (std::env::var("TA_STRIP_MAPS"), std::env::var_os("TA_STRIP_OUT")) else { return };
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "original").unwrap();
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let lut = srgb_to_linear_lut();
        for name in maps.split(',') {
            let Some(map) = vfs.read_opt(&format!("maps/{}.tnt", name.trim())).and_then(|b| ta_formats::tnt::TntMap::parse(&b).ok()) else {
                eprintln!("{name}: not found");
                continue;
            };
            let (tw, th) = ((map.width / 2) as usize, (map.height / 2) as usize);
            let means: Vec<[f32; 3]> = map
                .tiles
                .iter()
                .map(|t| {
                    let mut sum = [0f32; 3];
                    for &i in t.iter() {
                        let [r, g, b] = palette.rgb(i);
                        sum = [sum[0] + lut[r as usize], sum[1] + lut[g as usize], sum[2] + lut[b as usize]];
                    }
                    sum.map(|s| s / (t.len() as f32))
                })
                .collect();
            // The strip as it was shown: the tile beside it alone, in linear light.
            let colours = TileColours::new(&means);
            let mut was = map.tile_map.clone();
            for y in 0..th - 4 {
                let (strip, beside) = (y * tw + tw - 1, y * tw + tw - 2);
                if colours.linear_distance(was[strip], was[beside]) > JUNK_LINEAR {
                    was[strip] = was[beside];
                }
            }
            for (k, y) in (th - 4..th).enumerate() {
                for x in 0..tw {
                    let x_shown = x.min(tw - 2);
                    let (strip, above, mirrored) = (y * tw + x, (th - 5) * tw + x_shown, (th - 5).saturating_sub(k) * tw + x_shown);
                    if colours.linear_distance(was[strip], was[above]) > JUNK_LINEAR {
                        was[strip] = was[mirrored];
                    }
                }
            }
            let now = shown_tiles(&map.tile_map, tw, th, &means);
            for (label, tiles) in [("stored", &map.tile_map), ("before", &was), ("after", &now)] {
                let marked: Vec<bool> = (0..tw * th).map(|i| label == "after" && now[i] != map.tile_map[i] && was[i] == map.tile_map[i]).collect();
                let rows = 8.min(th);
                let (w, h) = (tw * 8, rows * 8);
                let mut rgb = vec![0u8; w * h * 3];
                for ty in 0..rows {
                    for tx in 0..tw {
                        let tile = &map.tiles[tiles[(th - rows + ty) * tw + tx] as usize];
                        for y in 0..8 {
                            for x in 0..8 {
                                let mut c = palette.rgb(tile[(y * 4 + 2) * 32 + x * 4 + 2]);
                                if marked[(th - rows + ty) * tw + tx] && (x == 0 || y == 0 || x == 7 || y == 7) {
                                    c = [255, 0, 0];
                                }
                                let o = ((ty * 8 + y) * w + tx * 8 + x) * 3;
                                rgb[o..o + 3].copy_from_slice(&c);
                            }
                        }
                    }
                }
                let file = std::path::Path::new(&out).join(format!("{}_{label}.png", name.trim().replace(' ', "_")));
                image::save_buffer(&file, &rgb, w as u32, h as u32, image::ColorType::Rgb8).unwrap();
            }
            eprintln!("{name}: {tw}x{th} tiles, pictures written");
        }
    }

    /// Deterministic noise bytes.
    fn noise(n: usize, seed: u64) -> Vec<u8> {
        let mut x = seed | 1;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    }

    #[test]
    fn edited_mips_equal_a_rebuild() {
        // Odd sizes: the mip chain drops a row or column on the way down.
        let (w, h) = (37u32, 29u32);
        let mut level0: Vec<f32> = noise((w * h * 2) as usize, 3).iter().map(|&b| b as f32 / 255.0).collect();
        let mut levels = float_mips(w, h, 2, &level0);
        let rect = [5, 7, 16, 11];
        for (y, x) in (rect[1]..rect[3]).flat_map(|y| (rect[0]..rect[2]).map(move |x| (y, x))) {
            for c in 0..2 {
                level0[((y * w + x) * 2 + c) as usize] = 1.0 - level0[((y * w + x) * 2 + c) as usize];
            }
        }
        levels[0].2.clone_from(&level0);
        let rects = update_float_mips(&mut levels, 2, rect);
        assert_eq!(levels, float_mips(w, h, 2, &level0), "every level as a rebuild makes it");
        assert_eq!(rects[1], [2, 3, 8, 6]);
        assert_eq!(rects.len(), levels.len());
    }

    #[test]
    fn pads_far_apart_are_levelled_apart_and_many_at_once_together() {
        // Two that overlap and one that touches them become one; one far off stays its own.
        let rects = merged_rects(vec![[0, 0, 10, 10], [8, 8, 20, 20], [20, 0, 30, 8], [100, 100, 110, 110]]);
        assert_eq!(rects.len(), 2);
        assert!(rects.contains(&[0, 0, 30, 20]) && rects.contains(&[100, 100, 110, 110]));
        // Many: the one rectangle round them all.
        let many: Vec<[i32; 4]> = (0..40).map(|i| [i * 50, 0, i * 50 + 10, 10]).collect();
        assert_eq!(merged_rects(many), vec![[0, 0, 39 * 50 + 10, 10]]);
    }

    #[test]
    fn half_floats_keep_the_drawn_heights() {
        for h in [0.0f32, 1.0, 37.25, 85.0, 128.6, 200.0, 254.9, 255.0] {
            let bits = half_bits(h / 255.0);
            // Back from the bits: sign 0, exponent, mantissa.
            let (e, m) = ((bits >> 10) as i32, (bits & 0x3FF) as f32);
            let back = if e == 0 { 0.0 } else { (1.0 + m / 1024.0) * 2f32.powi(e - 15) * 255.0 };
            assert!((back - h).abs() <= 0.07, "{h}: {back}");
        }
    }

    #[test]
    fn edited_tile_averages_equal_a_rebuild() {
        let (w, h) = (13u32, 10u32);
        let colour = |b: &[u8]| [b[0] as f32 / 255.0, b[1] as f32 / 255.0, b[2] as f32 / 255.0];
        let rebuild = |level0: &[[f32; 3]]| {
            // As `TerrainGpu::new` builds the tile-average levels.
            let mut out = Vec::new();
            let (mut cur, mut cw, mut ch) = (level0.to_vec(), w, h);
            let count = 32 - w.max(h).leading_zeros();
            for _ in 0..count {
                out.push((cw, ch, cur.clone()));
                let (nw, nh) = ((cw / 2).max(1), (ch / 2).max(1));
                let mut next = vec![[0f32; 3]; (nw * nh) as usize];
                for y in 0..nh {
                    for x in 0..nw {
                        let mut acc = [0f32; 3];
                        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            let c = cur[((y * 2 + dy).min(ch - 1) * cw + (x * 2 + dx).min(cw - 1)) as usize];
                            for k in 0..3 {
                                acc[k] += c[k] * 0.25;
                            }
                        }
                        next[(y * nw + x) as usize] = acc;
                    }
                }
                (cur, cw, ch) = (next, nw, nh);
            }
            out
        };
        let mut level0: Vec<[f32; 3]> = noise((w * h * 3) as usize, 9).chunks(3).map(colour).collect();
        let mut levels = rebuild(&level0);
        for i in [3usize, 4, 16, 17, 120] {
            level0[i] = [0.9, 0.1, 0.4];
        }
        levels[0].2.clone_from(&level0);
        update_color_mips(&mut levels, [3, 0, 5, 10]);
        assert_eq!(levels, rebuild(&level0));
    }

    #[test]
    fn occlusion_around_a_change_equals_a_bake_of_the_whole_map() {
        let (w, h) = (96u32, 80u32);
        // Rolling hills, so the occlusion is far from uniform.
        let mut heights: Vec<u8> = (0..w * h).map(|i| ((((i % w) as f32 * 0.21).sin() + ((i / w) as f32 * 0.17).cos()) * 50.0 + 110.0) as u8).collect();
        for rect in [[40u32, 30, 47, 38], [0, 0, 6, 5], [90, 70, 96, 80]] {
            let before = crate::occlusion::terrain_visibility(w, h, &heights);
            let mut edited = heights.clone();
            for z in rect[1]..rect[3] {
                for x in rect[0]..rect[2] {
                    edited[(z * w + x) as usize] = 250;
                }
            }
            let after = crate::occlusion::terrain_visibility(w, h, &edited);
            let (out, values) = occlusion_around(w, h, &edited, rect);
            let inside = |x: u32, z: u32| x >= out[0] && x < out[2] && z >= out[1] && z < out[3];
            for z in 0..h {
                for x in 0..w {
                    let i = (z * w + x) as usize;
                    if inside(x, z) {
                        assert_eq!(values[((z - out[1]) * (out[2] - out[0]) + x - out[0]) as usize], after[i], "baked again at ({x}, {z}) for {rect:?}");
                    } else {
                        assert_eq!(before[i], after[i], "({x}, {z}) is out of reach of {rect:?}");
                    }
                }
            }
            heights = edited;
        }
    }
}
