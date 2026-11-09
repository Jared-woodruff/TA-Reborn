//! TA painted many of its hills and outcrops with hair-like strands: dark lines a pixel or two
//! wide that follow the height contours, packed tighter on steeper ground and fanning out 30 to
//! 100 pixels over the flat ground at the foot (mauve on Coast To Coast, black ink on Sherwood,
//! charcoal hatching on Lake Shore). From TA's distance they read as shading; drawn larger, as
//! the perspective camera draws them, they are a web of scribbles over every hill.
//!
//! [`clean_strands`] finds them in the map's tiles when its terrain is loaded (Reborn style, on
//! the worlds where they are hatching: `terrain_detail::world_strands`) and paints them over
//! with the ground beside them. A strand is a run of pixels darker than the pixels a couple to either
//! side of it, along some direction (an oriented line filter, the best of 16 directions), by
//! more than the map's own plain ground ever is (its dither sets the threshold), grown along
//! connected pixels a little less sure (hysteresis); only near sloping ground, where TA drew
//! them. Each strand pixel takes the colour of the least strand-like pixel a few away.

use rayon::prelude::*;

const TILE: usize = 32;
/// Pixels of context round a tile (from the tiles beside it on the map).
const BORDER: usize = 8;
const PATCH: usize = TILE + 2 * BORDER;
/// Directions tried, a strand's sampled length (pixels) and how far either side of it its
/// sides are sampled; how far in from a patch's edge pixels are measured (every sample inside).
const DIRECTIONS: usize = 16;
const LENGTH: i32 = 9;
const GAP: f32 = 2.0;
const MARGIN: usize = 7;
/// Where a strand is sure (times the plain ground's threshold), and where it may grow.
const SURE: f32 = 1.2;
const GROW: f32 = 0.75;
/// Passes over each tile: a second one takes the strands' faint remainder.
const PASSES: usize = 2;
/// The most places a tile cleaned shows in.
const MOST_USES: u32 = 4;

/// A tile's pixels (sRGB), row by row.
pub(crate) type TilePixels = [[u8; 3]; TILE * TILE];

/// The tiles (by index) whose strands were painted over, and their pixels after. `tiles`: every
/// tile's pixels; `shown`: the tile at each tile position (`tiles_x` × `tiles_y`); `heights`:
/// the heightmap (`width` × `height` cells, two a tile), to find the sloping ground.
pub(crate) fn clean_strands(tiles: &[TilePixels], shown: &[u16], tiles_x: usize, tiles_y: usize, heights: &[u8], width: usize, height: usize) -> Vec<(u32, Box<TilePixels>)> {
    if tiles.is_empty() || tiles_x == 0 || tiles_y == 0 || shown.len() < tiles_x * tiles_y || heights.len() < width * height {
        return Vec::new();
    }
    // Where each tile shows (its first place), how often, and how much sloping ground is near
    // any of its places.
    let mut first = vec![usize::MAX; tiles.len()];
    let mut uses = vec![0u32; tiles.len()];
    for (pos, &t) in shown.iter().enumerate().take(tiles_x * tiles_y) {
        if let Some(f) = first.get_mut(t as usize) {
            if *f == usize::MAX {
                *f = pos;
            }
            uses[t as usize] += 1;
        }
    }
    let gates: Vec<f32> = (0..tiles_x * tiles_y).into_par_iter().map(|pos| slope_gate(pos % tiles_x, pos / tiles_x, heights, width, height)).collect();
    let mut gate = vec![0.0f32; tiles.len()];
    for (pos, &t) in shown.iter().enumerate().take(tiles_x * tiles_y) {
        if let Some(g) = gate.get_mut(t as usize) {
            *g = g.max(gates[pos]);
        }
    }
    // The map's own dither sets the threshold: the strongest response over its plain ground
    // (the most used tiles), short of the odd speck.
    let mut plain: Vec<usize> = (0..tiles.len()).filter(|&t| uses[t] >= 3).collect();
    plain.sort_by(|&a, &b| uses[b].cmp(&uses[a]).then(a.cmp(&b)));
    plain.truncate(8);
    if plain.is_empty() {
        return Vec::new();
    }
    let mut responses: Vec<f32> = plain
        .par_iter()
        .flat_map_iter(|&t| {
            let patch = patch_at(first[t], tiles, shown, tiles_x, tiles_y);
            let (r, _) = response(&luminance(&patch));
            (BORDER..BORDER + TILE).flat_map(move |y| (BORDER..BORDER + TILE).map(move |x| y * PATCH + x)).map(move |i| r[i]).collect::<Vec<_>>()
        })
        .collect();
    responses.sort_by(f32::total_cmp);
    let threshold = responses[((responses.len() - 1) as f32 * 0.995) as usize].max(1.0);
    // (Not the map's plain ground itself, repeated all over it: a strand draws a tile of its
    // own, and a tile used often is ground painted to tile.)
    (0..tiles.len())
        .into_par_iter()
        .filter(|&t| gate[t] > 0.0 && first[t] != usize::MAX && uses[t] <= MOST_USES)
        .filter_map(|t| {
            let mut patch: Vec<[f32; 3]> = patch_at(first[t], tiles, shown, tiles_x, tiles_y).iter().map(|p| p.map(|v| v as f32)).collect();
            for _ in 0..PASSES {
                patch = dehair(&patch, threshold, gate[t]);
            }
            let mut out = Box::new([[0u8; 3]; TILE * TILE]);
            let mut changed = false;
            for y in 0..TILE {
                for x in 0..TILE {
                    let c = patch[(y + BORDER) * PATCH + x + BORDER].map(|v| v.round().clamp(0.0, 255.0) as u8);
                    changed |= c != tiles[t][y * TILE + x];
                    out[y * TILE + x] = c;
                }
            }
            changed.then_some((t as u32, out))
        })
        .collect()
}

/// How much sloping ground lies near the ground tile position (`tx`, `ty`) shows (0..1): the
/// steepest cell within three of it, and, as hills are painted north of where they stand (by
/// half their height), up to eight more to the south.
fn slope_gate(tx: usize, ty: usize, heights: &[u8], width: usize, height: usize) -> f32 {
    let h = |x: usize, z: usize| heights[z.min(height - 1) * width + x.min(width - 1)] as f32;
    let (x0, x1) = ((tx * 2).saturating_sub(3), (tx * 2 + 4).min(width - 1));
    let (z0, z1) = ((ty * 2).saturating_sub(3), (ty * 2 + 12).min(height - 1));
    let mut steepest = 0.0f32;
    for z in z0..z1 {
        for x in x0..x1 {
            let c = [h(x, z), h(x + 1, z), h(x, z + 1), h(x + 1, z + 1)];
            let range = c.iter().copied().fold(f32::MIN, f32::max) - c.iter().copied().fold(f32::MAX, f32::min);
            steepest = steepest.max(range / 16.0);
        }
    }
    smoothstep(0.03, 0.08, steepest)
}

/// The tile at tile position `pos` with `BORDER` pixels of the tiles round it (clamped at the
/// map's edges).
fn patch_at(pos: usize, tiles: &[TilePixels], shown: &[u16], tiles_x: usize, tiles_y: usize) -> Vec<[u8; 3]> {
    let (tx, ty) = ((pos % tiles_x) as i32, (pos / tiles_x) as i32);
    let mut out = vec![[0u8; 3]; PATCH * PATCH];
    for y in 0..PATCH as i32 {
        for x in 0..PATCH as i32 {
            // The map pixel, clamped to the map.
            let px = (tx * TILE as i32 + x - BORDER as i32).clamp(0, tiles_x as i32 * TILE as i32 - 1);
            let py = (ty * TILE as i32 + y - BORDER as i32).clamp(0, tiles_y as i32 * TILE as i32 - 1);
            let t = shown[(py as usize / TILE) * tiles_x + px as usize / TILE] as usize;
            out[y as usize * PATCH + x as usize] = tiles.get(t).map_or([0; 3], |tile| tile[(py as usize % TILE) * TILE + px as usize % TILE]);
        }
    }
    out
}

fn luminance<T: Copy + Into<f32>>(patch: &[[T; 3]]) -> Vec<f32> {
    patch.iter().map(|p| 0.2126 * p[0].into() + 0.7152 * p[1].into() + 0.0722 * p[2].into()).collect()
}

/// The oriented line filter over a patch's luminance `l`: per pixel, the best over the
/// directions of how much darker a short segment through it is than the segments a couple of
/// pixels to either side (the darker side counts); and how much darker the pixel itself is than
/// those sides.
fn response(l: &[f32]) -> (Vec<f32>, Vec<f32>) {
    // Each direction's sample offsets in the patch (flat): the segment through the pixel and
    // those to either side. (Only pixels far enough in from the patch's edge for every sample
    // to fall inside it are measured: the tile and a few pixels round it.)
    let offsets: Vec<[[isize; LENGTH as usize]; 3]> = (0..DIRECTIONS)
        .map(|k| {
            let th = std::f32::consts::PI * k as f32 / DIRECTIONS as f32;
            let (c, s) = (th.cos(), th.sin());
            let along = |ox: f32, oy: f32| -> [isize; LENGTH as usize] {
                std::array::from_fn(|i| {
                    let t = i as f32 - (LENGTH - 1) as f32 / 2.0;
                    (oy + t * s).round() as isize * PATCH as isize + (ox + t * c).round() as isize
                })
            };
            [along(0.0, 0.0), along(-s * GAP, c * GAP), along(s * GAP, -c * GAP)]
        })
        .collect();
    let mut best = vec![0.0f32; PATCH * PATCH];
    let mut dark = vec![0.0f32; PATCH * PATCH];
    for y in MARGIN..PATCH - MARGIN {
        for x in MARGIN..PATCH - MARGIN {
            let i = y * PATCH + x;
            let mean = |seg: &[isize; LENGTH as usize]| seg.iter().map(|&o| l[(i as isize + o) as usize]).sum::<f32>() / LENGTH as f32;
            for dir in &offsets {
                let (centre, a, b) = (mean(&dir[0]), mean(&dir[1]), mean(&dir[2]));
                let r = a.min(b) - centre;
                if r > best[i] {
                    best[i] = r;
                    dark[i] = 0.5 * (a + b) - l[i];
                }
            }
        }
    }
    (best, dark)
}

/// One pass over a patch: its strands (see [`response`]; `t` the plain ground's threshold,
/// `gate` how much sloping ground is near) painted over with the least strand-like pixel a
/// few away.
fn dehair(patch: &[[f32; 3]], t: f32, gate: f32) -> Vec<[f32; 3]> {
    let (r, dark) = response(&luminance(patch));
    let candidate: Vec<bool> = r.iter().zip(&dark).map(|(&r, &d)| r > GROW * t && d > 0.0).collect();
    let mut on: Vec<bool> = r.iter().zip(&dark).map(|(&r, &d)| r > SURE * t && d > 0.0).collect();
    // Grown along the less sure pixels it touches.
    for _ in 0..12 {
        let mut grown = on.clone();
        for y in 0..PATCH {
            for x in 0..PATCH {
                let i = y * PATCH + x;
                if on[i] || !candidate[i] {
                    continue;
                }
                let near = (y.saturating_sub(1)..=(y + 1).min(PATCH - 1)).any(|ny| (x.saturating_sub(1)..=(x + 1).min(PATCH - 1)).any(|nx| on[ny * PATCH + nx]));
                grown[i] = near;
            }
        }
        if grown == on {
            break;
        }
        on = grown;
    }
    let weight: Vec<f32> = (0..PATCH * PATCH).map(|i| if on[i] { smoothstep(GROW * t, 1.1 * t, r[i]) * gate } else { 0.0 }).collect();
    const AROUND: [(i32, i32); 12] = [(3, 0), (-3, 0), (0, 3), (0, -3), (2, 2), (-2, 2), (2, -2), (-2, -2), (4, 1), (-4, -1), (1, 4), (-1, -4)];
    let clamp = |v: i32| v.clamp(0, PATCH as i32 - 1) as usize;
    (0..PATCH * PATCH)
        .map(|i| {
            let w = weight[i];
            if w <= 0.0 {
                return patch[i];
            }
            let (x, y) = ((i % PATCH) as i32, (i / PATCH) as i32);
            let mut pick = i;
            let mut least = f32::MAX;
            for (dx, dy) in AROUND {
                let j = clamp(y + dy) * PATCH + clamp(x + dx);
                if weight[j] < least {
                    least = weight[j];
                    pick = j;
                }
            }
            let (a, b) = (patch[i], patch[pick]);
            [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w, a[2] + (b[2] - a[2]) * w]
        })
        .collect()
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map `n` × `n` tiles of dithered sand (two shades, every tile the same), heights from
    /// `height` per cell.
    fn sand_map(n: usize, height: impl Fn(usize, usize) -> u8) -> (Vec<TilePixels>, Vec<u16>, Vec<u8>) {
        let mut sand = [[200u8, 180, 140]; TILE * TILE];
        for (i, p) in sand.iter_mut().enumerate() {
            if (i * 7919) % 5 < 2 {
                *p = [190, 170, 132];
            }
        }
        let heights = (0..n * 2 * n * 2).map(|i| height(i % (n * 2), i / (n * 2))).collect();
        (vec![sand], vec![0u16; n * n], heights)
    }

    #[test]
    fn strands_on_a_slope_are_painted_over_and_plain_ground_is_left() {
        let n = 6;
        let (mut tiles, mut shown, heights) = sand_map(n, |x, _| (40 + x * 3) as u8);
        // A tile with a dark, wavy line across it, in the middle of the map.
        let mut hair = tiles[0];
        for x in 0..TILE {
            let y = 14 + ((x as f32 * 0.4).sin() * 3.0) as usize;
            hair[y * TILE + x] = [120, 70, 90];
        }
        tiles.push(hair);
        shown[2 * n + 3] = 1;
        let cleaned = clean_strands(&tiles, &shown, n, n, &heights, n * 2, n * 2);
        let (_, out) = cleaned.iter().find(|(t, _)| *t == 1).expect("the strand tile is cleaned");
        let dark = out.iter().filter(|p| p[0] < 150).count();
        assert!(dark < 4, "{dark} strand pixels left");
        assert!(cleaned.iter().all(|(t, _)| *t != 0), "the plain sand is left as it is");
    }

    #[test]
    fn nothing_is_touched_on_level_ground() {
        let n = 6;
        let (mut tiles, mut shown, heights) = sand_map(n, |_, _| 40);
        let mut hair = tiles[0];
        for x in 0..TILE {
            hair[16 * TILE + x] = [120, 70, 90];
        }
        tiles.push(hair);
        shown[2 * n + 3] = 1;
        assert!(clean_strands(&tiles, &shown, n, n, &heights, n * 2, n * 2).is_empty(), "a road or a marking on level ground stays");
    }
}
