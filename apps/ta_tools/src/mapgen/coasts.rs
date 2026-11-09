//! Two coasts facing each other across an ocean (Shore to Shore Reborn's land), laid out rather
//! than grown. Each coast is a line of headlands and bays, coves, river mouths and a lagoon behind
//! its spit of sand; inland the ground rises over a coastal plain to uplands and a ridge of
//! highlands, the rivers in valleys cut down through them. Under the sea a shelf runs off each
//! coast (wide in the bays, narrow off the headlands) and falls down its slope to the abyss;
//! islands stand in chains with reef flats of their own; seamounts rise towards the surface; the
//! coves (each base's harbour) open into canyons across the shelf, and from every harbour a way
//! every ship sails runs out to the deep; and a bank crosses the ocean, shallow enough that
//! battleships and submarines cross it only by the channels cut through it, deep enough that no
//! tank or kbot wades it (light ships and hovercraft go where they please).
//!
//! Point-symmetric, as the generator's duels and team maps are: the coast is laid out in the west
//! and read in the east at each cell's twin (the cell turned half round the middle), and what
//! lies about the middle (the islands, the bank, the seamounts) is taken with its twin.

use rayon::prelude::*;

use super::field::{Field, distance};
use super::noise::{fbm, lerp, perlin, ridged, smoothstep, unit};
use super::terrain::{Land, Shape, Sym, catmull_rom, connect, harbours, label_regions, level, meander};

/// What the west coast is made of, where along it (`z`, a fraction of the map's height) and how
/// big (cells). The east coast is its twin.
#[derive(Clone)]
pub enum Coastal {
    /// A headland: a ridge of the uplands reaching `reach` cells out to sea, `width` across at its
    /// root, its tip turned `bend` cells south (north if negative); cliffs round its end.
    Headland { z: f32, reach: f32, width: f32, bend: f32 },
    /// A bay: the coast drawn back `depth` cells over `width`, its shore sand.
    Bay { z: f32, depth: f32, width: f32 },
    /// A cove: a basin of deep water `radius` cells across just inside the coast, its mouth `mouth`
    /// cells wide (a harbour).
    Cove { z: f32, radius: f32, mouth: f32 },
    /// A river from its source (fractions of the map) down to the sea at `z`: wadeable, in a valley
    /// cut down through the tiers, its mouth widening to `mouth` cells, sandy.
    River { z: f32, source: (f32, f32), mouth: f32 },
    /// A spit of sand across a bay, `length` cells long, its ends `back` cells in from the coast's
    /// line and its middle bowed `width / 2` out to sea, an inlet through it; the water behind it
    /// a lagoon.
    Lagoon { z: f32, length: f32, width: f32, back: f32 },
}

/// An island (in the west half, or about the middle line; its twin is made): where (fractions of
/// the map), how big (cells), its hills' height (units over the sea), a peak in its middle (0
/// none), stretched along `angle` (radians from the east) `stretch` times its width.
#[derive(Clone)]
pub struct Isle {
    pub x: f32,
    pub z: f32,
    pub r: f32,
    pub height: f32,
    pub peak: f32,
    pub stretch: f32,
    pub angle: f32,
}

/// A bank across the ocean: its course from the west coast to the middle (fractions; on through
/// its twin), half its width (cells), how deep the water over it is, and channels cut across it
/// (x as a fraction, their half width in cells; their twins too) deep enough for every ship.
#[derive(Clone)]
pub struct Bank {
    pub course: Vec<(f32, f32)>,
    pub half: f32,
    pub depth: f32,
    pub channels: Vec<(f32, f32)>,
}

/// Two coasts across an ocean.
#[derive(Clone)]
pub struct Coasts {
    /// Where the west coast's line runs (cells in from the map's edge), and how far it wanders.
    pub line: f32,
    pub wander: f32,
    pub features: Vec<Coastal>,
    pub isles: Vec<Isle>,
    pub bank: Option<Bank>,
    /// Seamounts: where (fractions of the map), how wide (cells), how deep the water over the top.
    pub seamounts: Vec<(f32, f32, f32, f32)>,
    /// The land's tiers (units over the sea): the coastal plain, the uplands, the highlands; and
    /// how far in from the coast's line (cells) the steps up to the uplands and the highlands lie.
    pub plain: f32,
    pub upland: f32,
    pub highland: f32,
    pub scarps: (f32, f32),
    /// The sea: how deep at the shelf's edge, how wide the shelf (cells), how deep the abyss.
    pub shelf: f32,
    pub shelf_width: f32,
    pub abyss: f32,
    /// Where the west coast's bases stand, in the order of their pairs (StartPos1, 3, 5...): along
    /// the coast (a fraction of the height) and in from its line (cells).
    pub bases: Vec<(f32, f32)>,
}

/// The distance from (x, z) to the segment a-b, and how far along it the nearest point lies.
fn seg(x: f32, z: f32, a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    let (dx, dz) = (b.0 - a.0, b.1 - a.1);
    let t = (((x - a.0) * dx + (z - a.1) * dz) / (dx * dx + dz * dz).max(1e-6)).clamp(0.0, 1.0);
    ((x - a.0 - dx * t).hypot(z - a.1 - dz * t), t)
}

/// How far each cell is from a course (points a cell apart; `f32::MAX` beyond `reach`) and how
/// far along it (0 at its first point, 1 at its last) the nearest point lies.
fn near_course(pts: &[(f32, f32)], w: usize, h: usize, reach: f32, dist: &mut [f32], along: &mut [f32]) {
    let last = (pts.len().max(2) - 1) as f32;
    for (j, &(px, pz)) in pts.iter().enumerate() {
        let (x0, x1) = ((px - reach).floor().max(0.0) as usize, ((px + reach).ceil().max(0.0) as usize).min(w - 1));
        let (z0, z1) = ((pz - reach).floor().max(0.0) as usize, ((pz + reach).ceil().max(0.0) as usize).min(h - 1));
        for z in z0..=z1 {
            for x in x0..=x1 {
                let d = (x as f32 - px).hypot(z as f32 - pz);
                let i = z * w + x;
                if d < dist[i] {
                    dist[i] = d;
                    along[i] = j as f32 / last;
                }
            }
        }
    }
}

/// What the west coast is at a cell (read in the west).
#[derive(Clone, Copy)]
struct Here {
    /// In from the shore (cells, positive on land, near enough): its line, headlands and bays,
    /// the coves, the rivers and the lagoon cut into it.
    land: f32,
    /// How much of a headland the cell is (0..1), how far out along it (0 its root, 1 its tip),
    /// and how near one (1 in it, 0 seventy cells off).
    head: f32,
    along: f32,
    head_near: f32,
    /// How much of a bay (0..1, rising over forty cells to its edge), and how near a cove's mouth
    /// (1 within thirty cells, 0 ninety off).
    bay: f32,
    cove_near: f32,
    /// How much of a canyon from a cove's mouth across the shelf (0..1, 1 on its line).
    canyon: f32,
    /// What made it water: 0 the sea, 1 a cove, 2 a river, 3 the lagoon.
    kind: u8,
    /// The river: how far along it (0 its source, 1 its mouth), and how far beyond its channel's
    /// edge (cells; negative in it).
    river_t: f32,
    river_e: f32,
}

/// How deep the way out from a harbour to the deep is (every ship's: a submarine's draft is 25,
/// a battleship's way 30), how far either side of its line it is that deep and over how many
/// cells more its sides ease out, and how deep the water it leads to.
const WAY_DEPTH: f32 = 32.0;
const WAY_HALF: f32 = 5.0;
const WAY_SIDE: f32 = 5.0;
const WAY_TO: f32 = 44.0;

/// How deep the sea is at `(x, z)` over a seamount about `middle`, `r` cells across or so (its
/// broad foot), rising from the `abyss` to `top` deep: a volcano's cone under the sea, long
/// along an axis of its own and lobed along three rift zones, a cone or two on its flanks, and
/// gullies down them; its outline wandering (the first were round, discs of the deep's blue
/// on the abyss's navy, read as drawn with compasses). `key` makes each one its own.
fn seamount(x: f32, z: f32, middle: (f32, f32), r: f32, top: f32, abyss: f32, key: u32) -> f32 {
    use std::f32::consts::TAU;
    let pick = |n: i32| unit(n, 0, key);
    // (The point read through a warp: no outline comes out as drawn.)
    let wx = x + 0.32 * r * fbm(x / (0.9 * r), z / (0.9 * r), key ^ 1, 3);
    let wz = z + 0.32 * r * fbm(x / (0.9 * r) + 5.2, z / (0.9 * r) - 3.1, key ^ 2, 3);
    let (dx, dz) = (wx - middle.0, wz - middle.1);
    let (sa, ca) = (pick(1) * TAU).sin_cos();
    let (along, across) = (dx * ca + dz * sa, dz * ca - dx * sa);
    // Its rift zones: three arms reaching out, the cone between them drawn in.
    let arms = 1.0 + 0.24 * (3.0 * across.atan2(along) + pick(2) * TAU).cos();
    let e = (along / 1.3).hypot(across / 0.8) / (r * arms);
    let mut rise = (-e * e).exp();
    // Its flank cones, lower than its summit.
    for n in 0..1 + (pick(3) * 2.0) as i32 {
        let (s, c) = (pick(10 + n) * TAU).sin_cos();
        let off = r * (0.75 + 0.45 * pick(20 + n));
        let cone = r * (0.32 + 0.18 * pick(30 + n));
        let e = (wx - middle.0 - off * c).hypot(wz - middle.1 - off * s) / cone;
        rise = rise.max((0.55 + 0.2 * pick(40 + n)) * (-e * e).exp());
    }
    // Gullies down its flanks (none at the summit or on the abyss's floor).
    let flank = 4.0 * rise * (1.0 - rise);
    let gullies = 4.0 * flank * (ridged(x / 16.0, z / 16.0, key ^ 3, 2) - 0.35);
    lerp(abyss, top, rise) + gullies
}

/// Joins every base's harbour to the deep by water every ship sails: from the deepest water within
/// 70 cells of the base (its berth), the cheapest way over the water (the shallower the dearer) to
/// the abyss, deepened to [`WAY_DEPTH`] where it is shallower, its sides easing out; and its twin.
/// (The shelf's sills, a few units under a submarine's draft, are where the coves' own canyons do
/// not reach.) Returns how many ways were laid.
fn ways_out(height: &mut Field, sea: f32, starts: &[(f32, f32)], sym: &Sym) -> usize {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let (w, h) = (height.w, height.h);
    let n = w * h;
    let depth: Vec<f32> = height.data.iter().map(|&v| sea - v).collect();
    let mut target = vec![0.0f32; n];
    let mut laid = 0;
    for &(sx, sz) in starts.iter().filter(|&&(x, z)| !sym.on || sym.first(x, z)) {
        let mut from: Option<(f32, usize)> = None;
        for z in (sz as i64 - 70).max(0)..(sz as i64 + 70).min(h as i64) {
            for x in (sx as i64 - 70).max(0)..(sx as i64 + 70).min(w as i64) {
                let i = z as usize * w + x as usize;
                if ((x as f32 - sx).hypot(z as f32 - sz)) <= 70.0 && depth[i] > 0.0 && from.is_none_or(|f| depth[i] > f.0) {
                    from = Some((depth[i], i));
                }
            }
        }
        let Some((_, from)) = from else { continue };
        // (Costs in tenths of a cell.)
        let mut best = vec![u32::MAX; n];
        let mut prev = vec![u32::MAX; n];
        let mut heap = BinaryHeap::new();
        best[from] = 0;
        heap.push(Reverse((0u32, from as u32)));
        let mut goal = None;
        while let Some(Reverse((c, at))) = heap.pop() {
            let at = at as usize;
            if c > best[at] {
                continue;
            }
            if depth[at] >= WAY_TO {
                goal = Some(at);
                break;
            }
            let (x, z) = ((at % w) as i64, (at / w) as i64);
            for (dx, dz) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= w as i64 || nz >= h as i64 {
                    continue;
                }
                let j = nz as usize * w + nx as usize;
                if depth[j] <= 0.0 {
                    continue;
                }
                let step = if dx != 0 && dz != 0 { 14 } else { 10 };
                let nc = c + step * (1 + ((WAY_DEPTH - depth[j]).max(0.0) * 0.4) as u32);
                if nc < best[j] {
                    best[j] = nc;
                    prev[j] = at as u32;
                    heap.push(Reverse((nc, j as u32)));
                }
            }
        }
        let Some(goal) = goal else { continue };
        let reach = (WAY_HALF + WAY_SIDE).ceil() as i64;
        let mut at = goal;
        loop {
            let (px, pz) = ((at % w) as i64, (at / w) as i64);
            for z in (pz - reach).max(0)..=(pz + reach).min(h as i64 - 1) {
                for x in (px - reach).max(0)..=(px + reach).min(w as i64 - 1) {
                    let i = z as usize * w + x as usize;
                    let d = ((x - px) as f32).hypot((z - pz) as f32);
                    target[i] = target[i].max(WAY_DEPTH * smoothstep(WAY_HALF + WAY_SIDE, WAY_HALF, d));
                }
            }
            if at == from {
                break;
            }
            at = prev[at] as usize;
        }
        laid += 1 + sym.on as usize;
    }
    for i in 0..n {
        let t = if sym.on { target[i].max(target[sym.twin_index(i)]) } else { target[i] };
        // (Only the water: never a shore cut back.)
        if t > 0.0 && depth[i] > 0.0 {
            height.data[i] = height.data[i].min(sea - t);
        }
    }
    laid
}

/// The land of two coasts across an ocean (see the module).
pub fn build(s: &Shape, c: &Coasts, sym: &Sym) -> Land {
    let (w, h) = (s.w, s.h);
    let n = w * h;
    let (wf, hf) = ((w - 1) as f32, (h - 1) as f32);
    let seed = s.seed;
    let sea = s.sea;
    // The west coast's line: its x at row z.
    let line = |z: f32| c.line + c.wander * fbm(z / 170.0, 0.37, seed ^ 0xC01, 3);
    // Each cell read in the west: itself there, its twin in the east.
    let west = |x: f32, z: f32| if sym.first(x, z) { (x, z) } else { sym.twin(x, z) };

    // The rivers, from source to mouth, a cell apart; how far each cell is from the nearest (in the
    // west) and how far along it.
    let mut river_d = vec![f32::MAX; n];
    let mut river_t = vec![0.0f32; n];
    let mut river_m = vec![0.0f32; n];
    let mut fords: Vec<(f32, f32)> = Vec::new();
    for (k, f) in c.features.iter().enumerate() {
        if let Coastal::River { z, source, mouth } = *f {
            let zm = z * hf;
            // (On out to sea past the coast, wherever its wandering puts it.)
            let end = (line(zm) + 40.0, zm);
            let src = (source.0 * wf, source.1 * hf);
            let steps = 6;
            let mut ctrl: Vec<(f32, f32)> = (0..=steps)
                .map(|i| {
                    let t = i as f32 / steps as f32;
                    let bow = perlin(t * 2.7 + k as f32 * 1.7, 0.5, seed ^ 0xC07) * 44.0 * t * (1.0 - t) * 4.0;
                    (lerp(src.0, end.0, t), lerp(src.1, end.1, t) + bow)
                })
                .collect();
            // (Steered in from beyond the source and out to sea beyond the mouth.)
            ctrl.insert(0, (src.0 - 30.0, src.1));
            ctrl.push((end.0 + 30.0, end.1));
            let course = meander(&catmull_rom(&ctrl), seed ^ (k as u32).wrapping_mul(131), 13.0, 24.0, true);
            let mut d = vec![f32::MAX; n];
            let mut t = vec![0.0f32; n];
            near_course(&course, w, h, 64.0, &mut d, &mut t);
            for i in 0..n {
                if d[i] < river_d[i] {
                    river_d[i] = d[i];
                    river_t[i] = t[i];
                    river_m[i] = mouth;
                }
            }
            // A ford where the coast's road crosses it, on the plain a little in from the mouth.
            if let Some(&p) = course.iter().rev().find(|p| line(p.1) - p.0 > 70.0) {
                fords.push(p);
                fords.push(sym.twin(p.0, p.1));
            }
        }
    }

    // The bank's course, through the middle and on by its twin, and how far each cell is from it.
    let mut bank_d = Field::new(w, h, 1e6);
    if let Some(b) = &c.bank {
        let mut pts: Vec<(f32, f32)> = b.course.iter().map(|&(x, z)| (x * wf, z * hf)).collect();
        let twins: Vec<(f32, f32)> = pts.iter().rev().map(|&(x, z)| sym.twin(x, z)).collect();
        pts.push((wf * 0.5, hf * 0.5));
        pts.extend(twins);
        let (a, b2) = (pts[0], pts[1]);
        let steer = (a.0 * 2.0 - b2.0, a.1 * 2.0 - b2.1);
        pts.insert(0, steer);
        pts.push(sym.twin(steer.0, steer.1));
        let dense = catmull_rom(&pts);
        let mut along = vec![0.0f32; n];
        near_course(&dense, w, h, b.half * 1.8 + 60.0, &mut bank_d.data, &mut along);
    }
    // (Every shape at sea and on the coast read through a warp, so none comes out as drawn with
    // compasses: two noises, broad and fine, each way.)
    let warp = |x: f32, z: f32| -> (f32, f32) {
        (
            x + 18.0 * fbm(x / 120.0, z / 120.0, seed ^ 0xC21, 3) + 6.0 * fbm(x / 36.0, z / 36.0, seed ^ 0xC22, 2),
            z + 18.0 * fbm(x / 120.0 + 7.3, z / 120.0 - 3.1, seed ^ 0xC23, 3) + 6.0 * fbm(x / 36.0 + 2.2, z / 36.0 + 5.5, seed ^ 0xC24, 2),
        )
    };

    // The coast at a cell (read in the west, `qi` its index).
    let here = |qx: f32, qz: f32, qi: usize| -> Here {
        let (wx, wz) = warp(qx, qz);
        let mut coast = line(wz) - wx;
        let (mut head, mut along, mut head_near, mut bay, mut cove_near, mut canyon) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for (k, f) in c.features.iter().enumerate() {
            if let Coastal::Headland { z, reach, width, bend } = *f {
                let zc = z * hf;
                let lc = line(zc);
                let (d, t) = seg(wx, wz, (lc - 50.0, zc), (lc + reach, zc + bend));
                // (Broad at its root, lobed along its length, a rounded end.)
                let lobes = 0.16 * width * fbm(t * 3.0 + k as f32 * 3.7, d / 30.0, seed ^ 0xC25, 2);
                let cap = lerp(width * 0.5, width * 0.24, t.powf(0.8)) + lobes - d;
                coast = coast.max(cap);
                let kk = smoothstep(-10.0, 8.0, cap);
                if kk > head {
                    head = kk;
                    along = t;
                }
                head_near = head_near.max(smoothstep(-70.0, 0.0, cap));
            }
        }
        for f in &c.features {
            if let Coastal::Bay { z, depth, width } = *f {
                let zc = z * hf;
                let (cx, ax, az) = (line(zc) + depth * 0.3, depth * 1.3, width * 0.5);
                let sdf = (1.0 - ((wx - cx) / ax).hypot((wz - zc) / az)) * ax.min(az);
                coast = coast.min(-sdf);
                bay = bay.max(smoothstep(-40.0, 0.0, sdf));
            }
        }
        // (The shore wandering on its own.)
        coast += 8.0 * fbm(qx / 30.0, qz / 30.0, seed ^ 0xC02, 3) + 2.8 * fbm(qx / 9.0, qz / 9.0, seed ^ 0xC03, 2);
        let mut land = coast;
        let (mut kind, mut best) = (0u8, 0.0f32);
        for f in &c.features {
            match *f {
                Coastal::Cove { z, radius, mouth } => {
                    let zc = z * hf;
                    let lc = line(zc);
                    let (bx, bz) = (lc - radius * 0.85, zc);
                    let basin = radius * (1.0 + 0.1 * perlin(wx / 12.0, wz / 12.0, seed ^ 0xC04)) - (wx - bx).hypot(wz - bz);
                    let (d, _) = seg(wx, wz, (bx, bz), (lc + 34.0, zc));
                    let cut = basin.max(mouth * 0.5 - d);
                    // (The cove what it cuts from the land; out at sea, its canyon.)
                    if cut > best && land > 0.0 {
                        best = cut;
                        kind = 1;
                    }
                    land = land.min(-cut);
                    // (The shelf narrow off a cove's mouth: deep water at its door.)
                    cove_near = cove_near.max(smoothstep(90.0, 30.0, (wx - lc - 34.0).hypot(wz - zc)));
                    // On from its mouth across the shelf, a canyon down to the deep: every ship's
                    // way out, the submarines' (25 deep) too.
                    let (dc, t) = seg(wx, wz, (bx, bz), (lc + 100.0, zc));
                    canyon = canyon.max(smoothstep(mouth * 0.5 + 8.0, mouth * 0.5 - 1.0, dc) * smoothstep(1.0, 0.8, t));
                }
                Coastal::Lagoon { z, length, width, back } => {
                    // A spit of sand across the bay, bowed out to sea, its ends curving in to the
                    // shore; an inlet through it a little north of its middle; the water behind it
                    // a lagoon.
                    let zc = z * hf;
                    let along = (wz - zc) / (length * 0.5);
                    if along.abs() < 1.15 {
                        let sx = line(zc) - back + width * (1.0 - along * along) * 0.5;
                        let spit = 2.4 + 0.9 * perlin(wz / 9.0, 0.5, seed ^ 0xC05) - (wx - sx).abs();
                        let inlet = (-0.32..-0.18).contains(&along);
                        if !inlet {
                            // (Thinning out at its ends: only what is land of it; scaled whole, the
                            // sea far off either side came up to the surface along those rows.)
                            land = land.max(if spit > 0.0 { spit * smoothstep(1.15, 0.95, along.abs()) } else { spit });
                        }
                        if along.abs() < 1.0 && wx < sx - 1.5 && land <= 0.0 && kind == 0 {
                            kind = 3;
                        }
                    }
                }
                _ => {}
            }
        }
        // The rivers (not warped: they wander on their own), a river only where they cut the land.
        let (mut river_e, mut rt) = (f32::MAX, 0.0f32);
        if river_d[qi] < f32::MAX {
            rt = river_t[qi];
            let half = lerp(3.2, 5.0, rt) + (river_m[qi] * 0.5 - 5.0).max(0.0) * smoothstep(0.78, 1.0, rt);
            river_e = river_d[qi] - half;
            if -river_e > best && land > 0.0 {
                kind = 2;
            }
            land = land.min(river_e);
        }
        Here { land, head, along, head_near, bay, cove_near, canyon, kind, river_t: rt, river_e }
    };

    // The islands: how far in from the shore of the nearest (cells, positive on it) and how high its
    // ground rises there, the cell and its twin taken together.
    let isle_one = |k: usize, x: f32, z: f32| -> f32 {
        let i = &c.isles[k];
        let s = (i.r * 0.8).max(8.0);
        let key = seed ^ 0xC10 ^ (k as u32).wrapping_mul(977);
        // (Each its own shape: warped, lobed.)
        let (x2, z2) = (x + 0.5 * i.r * fbm(x / s, z / s, key ^ 1, 2), z + 0.5 * i.r * fbm(x / s + 4.1, z / s - 2.7, key ^ 2, 2));
        let (dx, dz) = (x2 - i.x * wf, z2 - i.z * hf);
        let (ca, sa) = (i.angle.cos(), i.angle.sin());
        let (u, v) = (dx * ca + dz * sa, -dx * sa + dz * ca);
        i.r * (1.0 + 0.32 * fbm(x / s, z / s, key, 3) + 0.12 * fbm(x / (s * 0.3), z / (s * 0.3), key ^ 3, 2)) - (u / i.stretch).hypot(v)
    };
    let isles_at = |x: f32, z: f32| -> (f32, f32) {
        let t = sym.twin(x, z);
        let mut out = (-1e6f32, 0.0f32);
        for (k, i) in c.isles.iter().enumerate() {
            for p in [(x, z), t] {
                let g = isle_one(k, p.0, p.1);
                out.0 = out.0.max(g);
                // (The peak in ridges running down from its top.)
                let ridges = 0.55 + 0.6 * ridged(p.0 / 22.0, p.1 / 22.0, seed ^ 0xC1C ^ k as u32, 4);
                out.1 = out.1.max(i.height * smoothstep(0.0, i.r * 0.7, g) + i.peak * smoothstep(i.r * 0.25, i.r * 0.9, g).powf(1.4) * ridges);
            }
        }
        out
    };

    let cells: Vec<(Here, f32, f32)> = (0..n)
        .into_par_iter()
        .map(|i| {
            let (x, z) = ((i % w) as f32, (i / w) as f32);
            let (qx, qz) = west(x, z);
            let qi = qz as usize * w + qx as usize;
            let (isle, core) = isles_at(x, z);
            (here(qx, qz, qi), isle, core)
        })
        .collect();

    // Land and water, without specks: land of under 16 cells sinks, water shut in by the land and
    // under 300 cells fills.
    let mut dry: Vec<bool> = cells.iter().map(|(hr, isle, _)| hr.land.max(*isle) > 0.0).collect();
    let (label, sizes) = label_regions(w, h, &dry);
    for i in 0..n {
        if dry[i] && sizes[label[i] as usize] < 16 {
            dry[i] = false;
        }
    }
    let wet: Vec<bool> = dry.iter().map(|&d| !d).collect();
    let (label, sizes) = label_regions(w, h, &wet);
    for i in 0..n {
        if wet[i] && sizes[label[i] as usize] < 300 {
            dry[i] = true;
        }
    }
    // The mainland (the coasts) and the islands.
    let main: Vec<bool> = (0..n).map(|i| dry[i] && cells[i].0.land >= cells[i].1).collect();
    let isle_land: Vec<bool> = (0..n).map(|i| dry[i] && !main[i]).collect();
    let water: Vec<bool> = dry.iter().map(|&d| !d).collect();
    let water_dist = distance(w, h, &water);
    let water_in = distance(w, h, &dry);
    let d_cont = distance(w, h, &main);
    let d_isle = if isle_land.iter().any(|&v| v) { distance(w, h, &isle_land) } else { Field::new(w, h, 1e6) };

    // Each cell's place across the shore (cells, positive on land): near it the continuous field
    // (the waterline between the cells, where the shore runs), beyond it the distances.
    let shore = |i: usize| -> f32 {
        let lf = cells[i].0.land.max(cells[i].1);
        let lf = if dry[i] { lf.max(0.05) } else { lf.min(-0.05) };
        let far = if dry[i] { water_dist.data[i] - 0.5 } else { 0.5 - water_in.data[i] };
        lerp(lf, far, smoothstep(1.5, 3.0, lf.abs()))
    };
    // How far above or below the sea the ground lies `a` cells from the shore, running into a beach
    // or bed rising `k` a cell (as the grown maps' shores).
    let profile = |a: f32, k: f32| lerp(2.0 * a, 2.5 + k * (a + 0.5), smoothstep(0.0, 1.5, a));

    let heights: Vec<f32> = (0..n)
        .into_par_iter()
        .map(|i| {
            let (x, z) = ((i % w) as f32, (i / w) as f32);
            let (qx, qz) = west(x, z);
            let (hr, _, core) = cells[i];
            let a = shore(i);
            if main[i] {
                // The coast's tiers: the plain, a step up to the uplands and another to the
                // highlands, each a cliff or (where the ramp field says) a long slope; the headlands
                // ridges of the uplands reaching out to sea.
                let t = line(qz) - qx;
                // (Cliffs or long ramps, as the grown maps' steps are.)
                let ramp = smoothstep(0.15, 0.55, fbm(qx / 90.0, qz / 90.0, seed ^ 0x44, 3) + s.ramps);
                // (Wandering far either way, so the steps face every way, south too, where TA's
                // view sees them: plateaus reaching out, the plain reaching in.)
                let mut u = t + 75.0 * fbm(qx / 190.0, qz / 190.0, seed ^ 0x11, 3) + 26.0 * fbm(qx / 64.0, qz / 64.0, seed ^ 0x12, 2);
                u = lerp(u, u.max(c.scarps.0 + 30.0 - 22.0 * hr.along), hr.head);
                let half = lerp(1.5, 12.0, ramp);
                let s0 = smoothstep(-half, half, u - c.scarps.0);
                let s1 = smoothstep(-half, half, u - c.scarps.1);
                let mut v = sea + c.plain + (c.upland - c.plain) * s0 + (c.highland - c.upland) * s1 + 0.035 * u.max(0.0);
                v += (fbm(qx / s.hill_scale, qz / s.hill_scale, seed ^ 0x55, 4) * 0.5 + 0.5) * s.hills * (0.45 + 0.35 * s0 + 0.5 * s1);
                v += fbm(qx / 9.0, qz / 9.0, seed ^ 0x66, 2) * 1.4;
                // Knolls over the plain and the uplands (not the highlands: their peaks).
                v += smoothstep(0.56, 0.86, fbm(qx / 45.0, qz / 45.0, seed ^ 0x13, 3) * 0.5 + 0.5) * 14.0 * (1.0 - s1);
                v += ridged(qx / 46.0, qz / 46.0, seed ^ 0x77, 5).powf(1.6) * s.peaks * smoothstep(c.scarps.1 + 15.0, c.scarps.1 + 70.0, u);
                // The rivers' valleys: a floor widening downstream, walls up to the land.
                if hr.river_e < 400.0 {
                    let floor = lerp(4.0, 14.0, hr.river_t);
                    let wall = lerp(7.0, 2.6, ramp);
                    v = v.min(sea + 1.2 + 0.5 * hr.river_e.max(0.0) + (hr.river_e - floor).max(0.0) * wall);
                }
                // The shore: sand in the bays, cliffs where the uplands meet the sea.
                let cliffy = smoothstep(sea + 20.0, sea + 38.0, v);
                let k = lerp(lerp(1.2, 0.75, hr.bay), 9.5, cliffy);
                v.min(sea + profile(a, k))
            } else if dry[i] {
                // An island: its hills (a peak in the middle of the great ones), beaches, and
                // cliffs on some of its shores.
                let v = sea + 2.5 + core + (fbm(qx / 34.0, qz / 34.0, seed ^ 0x56, 3) * 0.5 + 0.5) * 6.0;
                let cliffy = smoothstep(0.56, 0.7, fbm(qx / 50.0, qz / 50.0, seed ^ 0xC11, 3) * 0.5 + 0.5) * smoothstep(sea + 14.0, sea + 28.0, v);
                v.min(sea + profile(a, lerp(1.0, 7.0, cliffy)))
            } else {
                let depth = match hr.kind {
                    // A cove: deep, steep sided.
                    1 => profile(-a, 5.0).min(34.0),
                    // A river: wadeable, its mouth a little deeper, sandy.
                    2 => profile(-a, 1.8).min(lerp(6.5, 9.5, smoothstep(0.8, 1.0, hr.river_t))),
                    // The lagoon: shallows.
                    3 => profile(-a, 0.7).min(4.5),
                    _ => {
                        let n01 = |sx: f32, key: u32| fbm(qx / sx, qz / sx, seed ^ key, 3) * 0.5 + 0.5;
                        // The shelf off the coast (wider in the bays, narrow off the headlands and at
                        // the coves' doors), sandbanks and gullies over it, its slope down to the
                        // abyss.
                        let dc = d_cont.data[i];
                        let abyss = c.abyss + 3.5 * fbm(qx / 70.0, qz / 70.0, seed ^ 0xC13, 3);
                        let sw = c.shelf_width * (0.55 + 0.9 * n01(130.0, 0xC12)) * lerp(1.0, 1.25, hr.bay) * lerp(1.0, 0.45, hr.head_near) * lerp(1.0, 0.3, hr.cove_near);
                        let mottle = 3.0 * fbm(qx / 40.0, qz / 40.0, seed ^ 0xC17, 3) + 1.2 * fbm(qx / 13.0, qz / 13.0, seed ^ 0xC18, 2);
                        let mut d = if dc <= sw { (lerp(2.5, c.shelf, (dc / sw).powf(0.85)) + mottle * smoothstep(2.0, 10.0, dc)).max(1.5) } else { lerp(c.shelf, abyss, smoothstep(sw, sw + 46.0, dc)) };
                        // Round the islands a reef flat (wider on some shores), then down to the
                        // abyss.
                        let di = d_isle.data[i];
                        let flat = 9.0 * (0.6 + 1.2 * n01(50.0, 0xC19));
                        d = d.min(if di <= flat { lerp(2.0, 9.0, di / flat) } else { lerp(9.0, abyss, smoothstep(flat, flat + 27.0 * (0.5 + 1.4 * n01(70.0, 0xC1D)), di)) });
                        // The bank: its edges lobed, wide and narrow by turns; never shallow enough
                        // for a tank or a kbot (12) to wade it, never deep enough for a submarine
                        // (20) or a battleship to sail it; the channels across it deep enough for
                        // every ship.
                        if let Some(b) = &c.bank {
                            let (bx, bz) = warp(x, z);
                            let (tx, tz) = sym.twin(x, z);
                            let (tbx, tbz) = warp(tx, tz);
                            let near = bank_d.sample(bx, bz).min(bank_d.sample(tbx, tbz));
                            let edge = b.half * (0.7 + 0.6 * n01(80.0, 0xC14));
                            let top = (b.depth + 2.0 * fbm(qx / 28.0, qz / 28.0, seed ^ 0xC15, 2)).clamp(13.5, 18.5);
                            let mut bank = lerp(abyss, top, smoothstep(edge + 26.0, edge, near));
                            for &(cx, half) in &b.channels {
                                let xc = cx * wf + 14.0 * perlin(qz / 45.0, 0.3, seed ^ 0xC16) + 5.0 * perlin(qz / 14.0, 1.7, seed ^ 0xC1A);
                                let half = half * (0.8 + 0.4 * (perlin(qz / 30.0, 2.9, seed ^ 0xC1B) * 0.5 + 0.5));
                                bank = lerp(bank, bank.max(40.0), smoothstep(half + 10.0, half, (qx - xc).abs()));
                            }
                            d = d.min(bank);
                        }
                        // Seamounts.
                        for (k, &(sx, sz, r, top)) in c.seamounts.iter().enumerate() {
                            for (px, pz) in [(x, z), sym.twin(x, z)] {
                                d = d.min(seamount(px, pz, (sx * wf, sz * hf), r, top, abyss, seed ^ (0x5EA0 + k as u32 * 0x9E37)));
                            }
                        }
                        // (Through sea level at the shore, whatever the bed beyond; a cove's canyon
                        // steeply.)
                        d.min(profile(-a, 1.5)).max((34.0 * hr.canyon).min(profile(-a, 5.0)))
                    }
                };
                // (At least a unit under the sea as the map stores it: a little less rounds up to
                // the sea's own height, which the game takes for dry ground.)
                sea - depth.max(0.75)
            }
        })
        .collect();
    let mut height = Field { w, h, data: heights };

    // The bases: on the coastal plain, the given way along the coast and in from its line, on the
    // most level ground near there with room round it (the ground within 20 cells gentle and dry).
    let slope = height.ta_slope();
    let mut firsts: Vec<(f32, f32)> = Vec::new();
    for &(zf, inland) in &c.bases {
        let z0 = zf * hf;
        let x0 = line(z0) - inland;
        let mut best = (f32::MAX, x0, z0);
        for dz in (-40i64..=40).step_by(2) {
            for dx in (-40i64..=40).step_by(2) {
                let (x, z) = (x0 as i64 + dx, z0 as i64 + dz);
                if x < 28 || z < 28 || x >= w as i64 - 28 || z >= h as i64 - 28 || !sym.first(x as f32, z as f32) {
                    continue;
                }
                let i = z as usize * w + x as usize;
                if !main[i] || water_dist.data[i] < 18.0 || height.data[i] > sea + c.plain + 18.0 {
                    continue;
                }
                let (mut rough, mut bad) = (0.0f32, 0usize);
                for oz in (-20i64..=20).step_by(4) {
                    for ox in (-20i64..=20).step_by(4) {
                        if ox * ox + oz * oz > 400 {
                            continue;
                        }
                        let j = (z + oz) as usize * w + (x + ox) as usize;
                        let sl = slope.data[j];
                        if !dry[j] || sl > 6.0 || (height.data[j] - height.data[i]).abs() > 12.0 {
                            bad += 1;
                        }
                        if ox.abs() <= 8 && oz.abs() <= 8 {
                            rough += sl;
                        }
                    }
                }
                let score = rough + bad as f32 * 4.0 + 0.05 * ((dx * dx + dz * dz) as f32).sqrt();
                if score < best.0 {
                    best = (score, x as f32, z as f32);
                }
            }
        }
        firsts.push((best.1, best.2));
    }
    let mut starts: Vec<(f32, f32)> = Vec::new();
    for &(x, z) in &firsts {
        starts.push((x, z));
        starts.push(sym.twin(x, z));
    }
    starts.truncate(s.players);
    for &(sx, sz) in &starts {
        level(&mut height, sx, sz, 16.0, 30.0);
    }
    // Every base on a coast a tank can drive to from the others there (across the ocean no way:
    // `Shape::connect`, every start from the first, cannot hold here).
    let mine: Vec<(f32, f32)> = starts.iter().copied().filter(|&(x, z)| sym.first(x, z)).collect();
    if mine.len() > 1 {
        let cut = connect(&mut height, sea, &mine, sym, None, s.whole);
        if cut > 0 {
            eprintln!("  cut {cut} ramp(s) so every base on a coast can be driven to");
        }
    }
    if s.harbours > 0.0 {
        let dredged = harbours(&mut height, sea, &starts, sym, s.harbours);
        eprintln!("  dredged {dredged} harbour(s)");
        let ways = ways_out(&mut height, sea, &starts, sym);
        eprintln!("  {ways} way(s) out to the deep for every ship");
    }
    sym.settle(&mut height);
    let wet: Vec<bool> = height.data.iter().map(|&v| v < sea).collect();
    let water_dist = distance(w, h, &wet);
    let moisture = Field::from_fn(w, h, |x, z| {
        let near = smoothstep(90.0, 4.0, water_dist.at(x, z));
        let (qx, qz) = west(x as f32, z as f32);
        let m = fbm(qx / 160.0, qz / 160.0, seed ^ 0xBB, 4) * 0.5 + 0.5;
        (m * 0.75 + near * 0.35).clamp(0.0, 1.0)
    });
    Land {
        w,
        h,
        sea,
        height,
        water_dist,
        moisture,
        starts,
        bed: Field::new(w, h, 1e6),
        dune_mask: Field::new(w, h, 0.0),
        fords,
        built: None,
        void: Vec::new(),
        gantry: Vec::new(),
        road: Vec::new(),
        ice: Vec::new(),
        geo: None,
        sym: *sym,
        halves_apart: true,
    }
}
