//! Tracks: paths worn into the ground between the bases and the middle of the map, as convoys
//! would wear them. Each runs where it is easiest to go (cheapest by slope, water and forest,
//! with a little wander so it bends as paths do) on a grid of two cells, the later ones reusing
//! the first, then smoothed. Only painted (and kept clear of trees and rocks): the heights are
//! the land's. On a symmetric map the first half's are found and their twins turned round.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use super::field::Field;
use super::noise::{fbm, smoothstep};
use super::terrain::Land;

/// How far (cells) from a track its distance is kept; beyond, the field holds [`FAR`].
const REACH: f32 = 8.0;
pub const FAR: f32 = 1e6;

/// The tracks' distance field (cells to the nearest track's middle, [`FAR`] away from them),
/// their courses (cells; all the lines' points in one list, `joined[k]` whether point `k` runs
/// on to point `k + 1`), and for each cell near one the segment nearest it, so the painter can
/// measure the distance exactly ([`Tracks::exact`]).
pub struct Tracks {
    pub dist: Field,
    pub points: Vec<(f32, f32)>,
    pub joined: Vec<bool>,
    pub nearest: Vec<u32>,
    w: usize,
    h: usize,
}

impl Tracks {
    pub fn none(w: usize, h: usize) -> Tracks {
        Tracks { dist: Field::new(w, h, FAR), points: Vec::new(), joined: Vec::new(), nearest: vec![u32::MAX; w * h], w, h }
    }

    /// The distance (cells) from (x, z) (cells) to the nearest track, exactly, near one; beyond
    /// [`REACH`], [`FAR`].
    pub fn exact(&self, x: f32, z: f32) -> f32 {
        if self.points.is_empty() || x < 0.0 || z < 0.0 {
            return FAR;
        }
        let (cx, cz) = ((x as usize).min(self.w - 1), (z as usize).min(self.h - 1));
        let k = self.nearest[cz * self.w + cx];
        if k == u32::MAX {
            return FAR;
        }
        let k = k as usize;
        let mut best = FAR;
        for j in k.saturating_sub(3)..(k + 4).min(self.points.len()) {
            if !self.joined[j] {
                continue;
            }
            let ((ax, az), (bx, bz)) = (self.points[j], self.points[j + 1]);
            let (dx, dz) = (bx - ax, bz - az);
            let t = (((x - ax) * dx + (z - az) * dz) / (dx * dx + dz * dz).max(1e-6)).clamp(0.0, 1.0);
            best = best.min((x - ax - dx * t).hypot(z - az - dz * t));
        }
        best
    }
}

#[derive(Copy, Clone, PartialEq)]
struct Node {
    cost: f32,
    at: u32,
}

impl Eq for Node {}

impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.total_cmp(&self.cost).then(o.at.cmp(&self.at))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Finds the tracks: each start to the middle and to its nearest neighbour.
pub fn tracks(land: &Land, forest: &Field, seed: u32) -> Tracks {
    let (w, h) = (land.w, land.h);
    let (gw, gh) = (w / 2, h / 2);
    let slope = land.height.ta_slope();
    let sym = land.sym;
    // The cost of crossing each coarse cell.
    let mut cost: Vec<f32> = (0..gw * gh)
        .map(|g| {
            let (gx, gz) = (g % gw, g / gw);
            let (x, z) = (gx * 2, gz * 2);
            let mut s = 0.0f32;
            let mut wet = false;
            let mut f = 0.0f32;
            for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let i = (z + dz).min(h - 1) * w + (x + dx).min(w - 1);
                s = s.max(slope.data[i]);
                wet |= land.height.data[i] < land.sea;
                f = f.max(forest.data[i]);
            }
            if s > 13.0 {
                return f32::INFINITY;
            }
            // (In space, across the void only on a gantry.)
            if !land.void.is_empty() && [(0usize, 0usize), (1, 0), (0, 1), (1, 1)].iter().any(|&(dx, dz)| land.void[(z + dz).min(h - 1) * w + (x + dx).min(w - 1)]) {
                return f32::INFINITY;
            }
            // (Wading is cheap at a ford, dear anywhere else.)
            let ford = land.fords.iter().any(|f| (f.0 - x as f32).hypot(f.1 - z as f32) < 14.0);
            let wet_cost = if ford { 3.0 } else { 30.0 };
            let wander = sym.eval(x as f32, z as f32, |x, z| fbm(x / 70.0, z / 70.0, seed ^ 0x7A, 3) * 0.6 + fbm(x / 22.0, z / 22.0, seed ^ 0x7B, 2) * 0.4) * 0.5 + 0.5;
            1.0 + 2.0 * smoothstep(4.0, 12.0, s).powi(2) * 4.0 + if wet { wet_cost } else { 0.0 } + f * 1.5 + wander * 3.0
        })
        .collect();
    let to_grid = |p: (f32, f32)| ((p.0 as usize / 2).min(gw - 1), (p.1 as usize / 2).min(gh - 1));
    // The hub the bases' tracks meet at: the middle, when it is dry ground a track can cross (on
    // a symmetric map a track and its twin meet only there).
    let (mx, mz) = (gw / 2, gh / 2);
    let middle = mz * gw + mx;
    // (Not where the halves are two coasts apart: an island in the middle is no crossroads.)
    let hub = (!land.halves_apart && cost[middle] < 10.0 && land.height.data[(mz * 2) * w + mx * 2] > land.sea).then_some(middle);
    let mut lines: Vec<Vec<(f32, f32)>> = Vec::new();
    let starts: Vec<(f32, f32)> = land.starts.clone();
    let mut routes: Vec<((usize, usize), usize)> = Vec::new();
    let nearest = |s: (f32, f32), among: &[(f32, f32)]| among.iter().copied().filter(|p| (p.0 - s.0).hypot(p.1 - s.1) > 1.0).min_by(|a, b| (a.0 - s.0).hypot(a.1 - s.1).total_cmp(&(b.0 - s.0).hypot(b.1 - s.1)));
    for &s in &starts {
        if !sym.first(s.0, s.1) {
            continue;
        }
        let from = to_grid(s);
        if let Some(hub) = hub {
            routes.push((from, hub));
        }
        let mut to = |p: (f32, f32)| {
            let (tx, tz) = to_grid(p);
            routes.push((from, tz * gw + tx));
        };
        // On to the nearest other start: on a symmetric map one in this half (a track across to
        // the other half would come twice, itself and its twin, side by side through the open
        // middle, where the hub joins the halves), and with no hub the nearest across the line
        // too (it and its twin cross the water at fords of their own); and down to the nearest
        // ford.
        let mine: Vec<(f32, f32)> = starts.iter().copied().filter(|p| sym.first(p.0, p.1)).collect();
        let theirs: Vec<(f32, f32)> = starts.iter().copied().filter(|p| !sym.first(p.0, p.1)).collect();
        if let Some(p) = nearest(s, if sym.on { &mine } else { &starts }) {
            to(p);
        }
        if land.halves_apart {
            // (Two coasts: on along the coast to the next base but one too, so the coast's bases
            // are joined in a line; never across the ocean.)
            let mut by: Vec<(f32, f32)> = mine.iter().copied().filter(|p| (p.0 - s.0).hypot(p.1 - s.1) > 1.0).collect();
            by.sort_by(|a, b| (a.0 - s.0).hypot(a.1 - s.1).total_cmp(&(b.0 - s.0).hypot(b.1 - s.1)));
            if let Some(&p) = by.get(1) {
                to(p);
            }
        } else if sym.on && hub.is_none()
            && let Some(p) = nearest(s, &theirs)
        {
            to(p);
        }
        if let Some(p) = nearest(s, &land.fords) {
            to(p);
        }
    }
    for (from, to) in routes {
        let Some(path) = cheapest(&cost, gw, gh, from.1 * gw + from.0, to) else { continue };
        // Later tracks join the earlier ones where they can (on them, far cheaper than the
        // ground beside, rather than alongside).
        for &g in &path {
            cost[g] = (cost[g] * 0.2).max(0.3);
        }
        let pts: Vec<(f32, f32)> = path.iter().map(|&g| ((g % gw) as f32 * 2.0 + 1.0, (g / gw) as f32 * 2.0 + 1.0)).collect();
        let line = smooth(&pts);
        if sym.on {
            lines.push(line.iter().map(|&(x, z)| sym.twin(x, z)).collect());
        }
        lines.push(line);
    }
    let mut points = Vec::new();
    let mut joined = Vec::new();
    for line in &lines {
        for (k, &p) in line.iter().enumerate() {
            points.push(p);
            joined.push(k + 1 < line.len());
        }
    }
    let mut dist = Field::new(w, h, FAR);
    let mut nearest = vec![u32::MAX; w * h];
    for j in 0..points.len() {
        if !joined[j] {
            continue;
        }
        {
            let ((ax, az), (bx, bz)) = (points[j], points[j + 1]);
            let (x0, x1) = ((ax.min(bx) - REACH).floor().max(0.0) as usize, ((ax.max(bx) + REACH).ceil().max(0.0) as usize).min(w - 1));
            let (z0, z1) = ((az.min(bz) - REACH).floor().max(0.0) as usize, ((az.max(bz) + REACH).ceil().max(0.0) as usize).min(h - 1));
            let (dx, dz) = (bx - ax, bz - az);
            let len2 = (dx * dx + dz * dz).max(1e-6);
            for z in z0..=z1 {
                for x in x0..=x1 {
                    let (px, pz) = (x as f32 + 0.5, z as f32 + 0.5);
                    let t = (((px - ax) * dx + (pz - az) * dz) / len2).clamp(0.0, 1.0);
                    let d = (px - ax - dx * t).hypot(pz - az - dz * t);
                    if d < REACH && d < dist.data[z * w + x] {
                        dist.data[z * w + x] = d;
                        nearest[z * w + x] = j as u32;
                    }
                }
            }
        }
    }
    Tracks { dist, points, joined, nearest, w, h }
}

/// The cheapest way from coarse cell `from` to `to` (Dijkstra over 8 neighbours), as cells.
fn cheapest(cost: &[f32], gw: usize, gh: usize, from: usize, to: usize) -> Option<Vec<usize>> {
    let mut best = vec![f32::INFINITY; gw * gh];
    let mut prev = vec![u32::MAX; gw * gh];
    let mut heap = BinaryHeap::new();
    best[from] = 0.0;
    heap.push(Node { cost: 0.0, at: from as u32 });
    while let Some(Node { cost: c, at }) = heap.pop() {
        let at = at as usize;
        if at == to {
            break;
        }
        if c > best[at] {
            continue;
        }
        let (x, z) = ((at % gw) as i64, (at / gw) as i64);
        for (dx, dz) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= gw as i64 || nz >= gh as i64 {
                continue;
            }
            let n = nz as usize * gw + nx as usize;
            let step = if dx != 0 && dz != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
            let nc = c + step * (cost[at] + cost[n]) * 0.5;
            if nc < best[n] {
                best[n] = nc;
                prev[n] = at as u32;
                heap.push(Node { cost: nc, at: n as u32 });
            }
        }
    }
    if !best[to].is_finite() {
        return None;
    }
    let mut path = vec![to];
    let mut at = to;
    while at != from {
        at = prev[at] as usize;
        path.push(at);
    }
    path.reverse();
    Some(path)
}

/// A path through grid cells smoothed into a curve (Chaikin's corner cutting, after thinning
/// the cells to every third), points about half a cell apart.
fn smooth(pts: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut p: Vec<(f32, f32)> = pts.iter().copied().step_by(4).collect();
    if let Some(&last) = pts.last()
        && p.last() != Some(&last)
    {
        p.push(last);
    }
    for _ in 0..5 {
        if p.len() < 3 {
            break;
        }
        let mut q = vec![p[0]];
        for s in p.windows(2) {
            let (a, b) = (s[0], s[1]);
            q.push((a.0 * 0.75 + b.0 * 0.25, a.1 * 0.75 + b.1 * 0.25));
            q.push((a.0 * 0.25 + b.0 * 0.75, a.1 * 0.25 + b.1 * 0.75));
        }
        q.push(*p.last().unwrap());
        p = q;
    }
    // Resampled half a cell apart.
    let mut out = vec![p[0]];
    for s in p.windows(2) {
        let (a, b) = (s[0], s[1]);
        let l = (b.0 - a.0).hypot(b.1 - a.1);
        let n = (l / 0.5).ceil().max(1.0) as usize;
        for k in 1..=n {
            let t = k as f32 / n as f32;
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }
    out
}
