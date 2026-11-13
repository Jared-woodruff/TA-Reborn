//! What stands on the map: the deposits (each mass point a TA metal deposit), the vents (each
//! hydrocarbon point a geothermal vent, its plant's site kept clear), the civilians' walls
//! (dragon's teeth), the props (rocks, trees, bushes: the map's own features, [`super::kit`]),
//! and the starts kept clear. Then the ways: where the game's land units could drive from one
//! start to another, a TA tank can too ([`connect`]).

use std::collections::{BTreeMap, BinaryHeap};

use ta_formats::tnt::FEATURE_NONE;

use super::kit::{Class, Kit, Own, Templates, classify};
use super::land::{CLIFF, Land};
use crate::files::Files;
use crate::lua::{self, Value};
use crate::map::Map;
use crate::Result;

/// A tank's footprint (cells), the steepest slope it climbs and the deepest water it wades: TA's
/// `TANKSH3`, the class most of its tanks are.
const TANK: (i32, u8, i32) = (3, 15, 12);
/// How far from a start no rock or tree stands (cells).
const START_CLEAR: i32 = 6;

/// What stands on the map.
pub struct Placed {
    /// The feature slot at each cell (a feature's top-left one), `FEATURE_NONE` elsewhere.
    pub cells: Vec<u16>,
    pub names: Vec<String>,
    pub own: Own,
    /// Where the players start (TA pixels).
    pub starts: Vec<(f64, f64)>,
    /// What the map holds, for the report: deposits, vents, rocks, trees and bushes, walls.
    pub counts: Counts,
    /// Start pairs the game's land units could drive between that a tank still cannot.
    pub cut_off: Vec<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub deposits: usize,
    pub vents: usize,
    pub rocks: usize,
    pub trees: usize,
    pub walls: usize,
    /// Cells changed to give tanks a way (cliffs unmarked, ground laid, blocking features gone).
    pub opened: usize,
}

struct Grid<'a> {
    land: &'a Land,
    w: usize,
    h: usize,
    cells: Vec<u16>,
    /// Covered by some feature's footprint.
    taken: Vec<bool>,
    /// Covered by a feature units cannot pass.
    blocked: Vec<bool>,
    /// The top-left cell of the blocking feature covering each cell (`u32::MAX`: none), and each
    /// such feature's footprint.
    owner: Vec<u32>,
    footprints: BTreeMap<u32, i32>,
    /// Kept clear (starts, plants' sites).
    reserved: Vec<bool>,
    names: Vec<String>,
    slots: BTreeMap<String, u16>,
}

impl Grid<'_> {
    fn fits(&self, x: i32, z: i32, fp: i32, reserve_ok: bool) -> bool {
        if x < 0 || z < 0 || x + fp > self.w as i32 || z + fp > self.h as i32 {
            return false;
        }
        (z..z + fp).all(|cz| (x..x + fp).all(|cx| {
            let i = cz as usize * self.w + cx as usize;
            !self.taken[i] && (reserve_ok || !self.reserved[i])
        }))
    }

    fn put(&mut self, name: &str, x: i32, z: i32, fp: i32, blocking: bool) {
        let slot = match self.slots.get(name) {
            Some(&s) => s,
            None => {
                let s = self.names.len() as u16;
                self.names.push(name.to_string());
                self.slots.insert(name.to_string(), s);
                s
            }
        };
        let origin = z as usize * self.w + x as usize;
        self.cells[origin] = slot;
        if blocking {
            self.footprints.insert(origin as u32, fp);
        }
        for cz in z..z + fp {
            for cx in x..x + fp {
                let i = cz as usize * self.w + cx as usize;
                self.taken[i] = true;
                if blocking {
                    self.blocked[i] = true;
                    self.owner[i] = origin as u32;
                }
            }
        }
    }

    fn reserve(&mut self, cx: i32, cz: i32, r: i32) {
        for z in (cz - r).max(0)..(cz + r + 1).min(self.h as i32) {
            for x in (cx - r).max(0)..(cx + r + 1).min(self.w as i32) {
                if (x - cx) * (x - cx) + (z - cz) * (z - cz) <= r * r {
                    self.reserved[z as usize * self.w + x as usize] = true;
                }
            }
        }
    }

    fn wet(&self, x: i32, z: i32) -> bool {
        self.land.depth(x.clamp(0, self.w as i32 - 1) as usize, z.clamp(0, self.h as i32 - 1) as usize) > 0
    }
}

/// A place's variant among a kit's trees or rocks: the same on every computer, varied over the map.
fn variant(x: i32, z: i32, i: usize) -> usize {
    let mut v = (x as u32).wrapping_mul(0x9E37_79B1) ^ (z as u32).wrapping_mul(0x85EB_CA77) ^ (i as u32).wrapping_mul(0xC2B2_AE3D);
    v ^= v >> 15;
    v = v.wrapping_mul(0x2C1B_3C6D);
    v ^= v >> 12;
    v as usize
}

/// Whether a civilian unit of the game (its blueprint id) is a wall's section.
fn wall(unit: &str) -> bool {
    unit.ends_with("5101") || unit.starts_with("xec80")
}

/// Places everything on `land` (whose cliffs a way for tanks may open up).
pub fn place(map: &Map, land: &mut Land, kit: Kit, templates: &Templates, files: &Files) -> Result<Placed> {
    let frame = land.frame;
    let (w, h) = (frame.w, frame.h);
    let cell_of = |p: [f32; 2]| -> (i32, i32) {
        let (x, z) = frame.pixels(p[0] as f64, p[1] as f64);
        ((x / 16.0).floor() as i32, (z / 16.0).floor() as i32)
    };
    let starts: Vec<(f64, f64)> = map.starts.iter().map(|s| {
        let (x, z) = frame.pixels(s[0] as f64, s[1] as f64);
        (x.clamp(48.0, w as f64 * 16.0 - 48.0), z.clamp(48.0, h as f64 * 16.0 - 48.0))
    }).collect();
    let mut counts = Counts::default();
    let mut g = Grid { land, w, h, cells: vec![FEATURE_NONE; w * h], taken: vec![false; w * h], blocked: vec![false; w * h], owner: vec![u32::MAX; w * h], footprints: BTreeMap::new(), reserved: vec![false; w * h], names: Vec::new(), slots: BTreeMap::new() };
    for &(x, z) in &starts {
        g.reserve((x / 16.0) as i32, (z / 16.0) as i32, START_CLEAR);
    }
    // Deposits: a 3 x 3 one centred on each mass point, moved a little where two would overlap.
    let spiral: Vec<(i32, i32)> = {
        let mut s: Vec<(i32, i32)> = (-3..=3).flat_map(|z| (-3..=3).map(move |x| (x, z))).collect();
        s.sort_by_key(|&(x, z)| (x * x + z * z, z, x));
        s
    };
    for p in &map.mass {
        let (cx, cz) = cell_of(*p);
        let name = if g.wet(cx, cz) && templates.has(kit.water_metal) { kit.water_metal } else { kit.metal };
        if !templates.has(name) {
            continue;
        }
        let fp = templates.footprint(name) as i32;
        if let Some(&(dx, dz)) = spiral.iter().find(|&&(dx, dz)| g.fits(cx - fp / 2 + dx, cz - fp / 2 + dz, fp, true)) {
            g.put(name, cx - fp / 2 + dx, cz - fp / 2 + dz, fp, false);
            counts.deposits += 1;
        }
    }
    // Vents, their plants' sites kept clear.
    let vents: Vec<&str> = kit.vents.iter().copied().filter(|v| templates.has(v)).collect();
    for (i, p) in map.hydrocarbons.iter().enumerate() {
        let (cx, cz) = cell_of(*p);
        let Some(&name) = vents.get(i % vents.len().max(1)) else { break };
        let fp = templates.footprint(name) as i32;
        if let Some(&(dx, dz)) = spiral.iter().find(|&&(dx, dz)| g.fits(cx - fp / 2 + dx, cz - fp / 2 + dz, fp, true)) {
            g.put(name, cx - fp / 2 + dx, cz - fp / 2 + dz, fp, false);
            g.reserve(cx + dx, cz + dz, 3);
            counts.vents += 1;
        }
    }
    // The civilians' walls: dragon's teeth, every other cell (each is 2 x 2).
    if templates.has("DragonsTeeth") {
        for c in map.civilians.iter().filter(|c| !c.wreck && wall(&c.unit)) {
            let (cx, cz) = cell_of(c.position);
            let (x, z) = (cx & !1, cz & !1);
            if g.fits(x, z, 2, false) && !g.wet(x, z) {
                g.put("DragonsTeeth", x, z, 2, true);
                counts.walls += 1;
            }
        }
    }
    // The props, in the map's order.
    let mut own = Own::default();
    let mut blueprints: BTreeMap<String, Class> = BTreeMap::new();
    for (i, prop) in map.scmap.props.iter().enumerate() {
        let key = prop.blueprint.to_ascii_lowercase();
        let class = match blueprints.get(&key) {
            Some(c) => *c,
            None => {
                let bp = files.read(&prop.blueprint).ok().flatten().and_then(|b| lua::parse(&String::from_utf8_lossy(&b)).ok());
                let table = bp.as_ref().and_then(|s| lua::top(s, "PropBlueprint")).and_then(Value::table);
                let c = classify(&key, table);
                blueprints.insert(key, c);
                c
            }
        };
        let (cx, cz) = cell_of([prop.position[0], prop.position[2]]);
        match class {
            Class::Rock { size, blocks, .. } => {
                let fp_wanted = ((size as f64 * frame.s / 16.0).round() as u32).clamp(1, 4);
                let Some(name) = own.name(&kit, templates, class, variant(cx, cz, i), fp_wanted) else { continue };
                let fp = own_footprint(&own, &name);
                let (x, z) = (cx - fp / 2, cz - fp / 2);
                if g.fits(x, z, fp, false) && !g.wet(cx, cz) {
                    g.put(&name, x, z, fp, blocks);
                    counts.rocks += 1;
                }
            }
            Class::Tree | Class::Bush | Class::Trees(_) => {
                const AROUND: [(i32, i32); 6] = [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1), (1, 1)];
                let n = if let Class::Trees(n) = class { n as usize } else { 1 };
                for (k, &(dx, dz)) in AROUND.iter().take(n).enumerate() {
                    let (x, z) = (cx + dx, cz + dz);
                    let Some(name) = own.name(&kit, templates, class, variant(x, z, i + k), 1) else { break };
                    if g.fits(x, z, 1, false) && !g.wet(x, z) {
                        g.put(&name, x, z, 1, false);
                        counts.trees += 1;
                    }
                }
            }
            Class::None => {}
        }
    }
    let Grid { cells, blocked, owner, footprints, names, .. } = g;
    let joined = connect(map, land, Blocking { cells, blocked, owner, footprints }, names, &starts);
    counts.opened = joined.changed;
    Ok(Placed { cells: joined.cells, names: joined.names, own, starts, counts, cut_off: joined.cut_off })
}

/// What [`connect`] leaves: the features and their names (unused names dropped), the start pairs
/// still cut off, and how many cells were changed.
struct Joined {
    cells: Vec<u16>,
    names: Vec<String>,
    cut_off: Vec<(usize, usize)>,
    changed: usize,
}

/// The blocking features: the feature slots, the cells they block, which feature blocks each
/// (its top-left cell) and each one's footprint.
struct Blocking {
    cells: Vec<u16>,
    blocked: Vec<bool>,
    owner: Vec<u32>,
    footprints: BTreeMap<u32, i32>,
}

fn own_footprint(own: &Own, name: &str) -> i32 {
    own.document().section(name).map_or(1, |s| s.get_i64_or("footprintx", 1).max(s.get_i64_or("footprintz", 1)).clamp(1, 16) as i32)
}

/// Whether a cell lets a tank through ([`TANK`]).
fn tank_cell(land: &Land, blocked: &[bool], x: usize, z: usize) -> bool {
    let i = z * land.frame.w + x;
    !land.cliff[i] && !blocked[i] && land.depth(x, z) <= TANK.2 && land.slope(x, z) <= TANK.1
}

/// The regions a tank reaches from each anchor (its footprint's middle cell), as the game's
/// pathfinder joins them: 0 where its footprint does not fit.
fn tank_regions(land: &Land, blocked: &[bool]) -> Vec<u32> {
    let (w, h) = (land.frame.w, land.frame.h);
    let r = TANK.0 / 2;
    let cell_ok: Vec<bool> = (0..w * h).map(|i| tank_cell(land, blocked, i % w, i / w)).collect();
    // Anchors whose whole footprint is open.
    let mut open = vec![false; w * h];
    for z in r as usize..h.saturating_sub(r as usize) {
        for x in r as usize..w.saturating_sub(r as usize) {
            open[z * w + x] = (z - r as usize..=z + r as usize).all(|cz| (x - r as usize..=x + r as usize).all(|cx| cell_ok[cz * w + cx]));
        }
    }
    let mut region = vec![0u32; w * h];
    let mut next = 1;
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !open[start] || region[start] != 0 {
            continue;
        }
        region[start] = next;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, z) = ((i % w) as i64, (i / w) as i64);
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if nx < 0 || nz < 0 || nx >= w as i64 || nz >= h as i64 {
                    continue;
                }
                let n = nz as usize * w + nx as usize;
                if !open[n] || region[n] != 0 {
                    continue;
                }
                if dx != 0 && dz != 0 && (!open[z as usize * w + nx as usize] || !open[nz as usize * w + x as usize]) {
                    continue;
                }
                region[n] = next;
                stack.push(n);
            }
        }
        next += 1;
    }
    region
}

/// The region of the game's land units each start is in (the game's own cells: not too steep,
/// not under water).
fn game_regions(map: &Map) -> Vec<u32> {
    let m = &map.scmap;
    let (w, h) = (m.width as usize, m.height as usize);
    let surface = m.water.as_ref().map_or(f32::MIN, |w| w.surface);
    let ok: Vec<bool> = (0..w * h).map(|i| {
        let (x, z) = ((i % w) as i64, (i / w) as i64);
        let c = [m.raw(x, z), m.raw(x + 1, z), m.raw(x, z + 1), m.raw(x + 1, z + 1)];
        let range = (c.iter().max().unwrap() - c.iter().min().unwrap()) as f32 * m.height_scale;
        let low = *c.iter().min().unwrap() as f32 * m.height_scale;
        range <= CLIFF && low >= surface
    }).collect();
    let mut region = vec![0u32; w * h];
    let mut next = 1;
    let mut stack = Vec::new();
    let mut of_start = Vec::new();
    for s in &map.starts {
        let i = (s[1].clamp(0.0, h as f32 - 1.0) as usize) * w + s[0].clamp(0.0, w as f32 - 1.0) as usize;
        if ok[i] && region[i] == 0 {
            region[i] = next;
            stack.push(i);
            while let Some(c) = stack.pop() {
                let (x, z) = (c % w, c / w);
                for (nx, nz) in [(x.wrapping_sub(1), z), (x + 1, z), (x, z.wrapping_sub(1)), (x, z + 1)] {
                    if nx < w && nz < h {
                        let n = nz * w + nx;
                        if ok[n] && region[n] == 0 {
                            region[n] = next;
                            stack.push(n);
                        }
                    }
                }
            }
            next += 1;
        }
        of_start.push(region[i]);
    }
    of_start
}

/// Ways for tanks: wherever the game's land units could drive from one start to another and a
/// tank cannot, the cheapest way between them (cliffs and blocking features cost most) is opened
/// where a tank could not pass, three cells wide and a few cells either side to ramp in: its
/// cliffs unmarked, its ground laid no steeper than a tank climbs, the blocking features in it
/// taken away. One look at the whole map a round, until every such pair is joined (or a round
/// joins nothing more). Returns the features and their names (unused names dropped), the pairs
/// still cut off, and how many cells were changed.
fn connect(map: &Map, land: &mut Land, b: Blocking, names: Vec<String>, starts: &[(f64, f64)]) -> Joined {
    let Blocking { mut cells, mut blocked, mut owner, footprints } = b;
    let (w, h) = (land.frame.w, land.frame.h);
    let game = game_regions(map);
    let anchor = |s: (f64, f64)| -> usize { ((s.1 / 16.0) as usize).clamp(1, h - 2) * w + ((s.0 / 16.0) as usize).clamp(1, w - 2) };
    let mut changed = vec![false; w * h];
    let mut tried: Vec<(usize, usize)> = Vec::new();
    let pairs: Vec<(usize, usize)> = (0..starts.len()).flat_map(|a| (a + 1..starts.len()).map(move |b| (a, b))).filter(|&(a, b)| game[a] != 0 && game[a] == game[b]).collect();
    let cut = |regions: &[u32], tried: &[(usize, usize)]| -> Option<(usize, usize)> {
        pairs.iter().copied().find(|&(a, b)| {
            let (ra, rb) = (regions[anchor(starts[a])], regions[anchor(starts[b])]);
            (ra == 0 || ra != rb) && tried.iter().filter(|&&t| t == (a, b)).count() < 3
        })
    };
    for _round in 0..starts.len() * 3 {
        let regions = tank_regions(land, &blocked);
        let Some((a, b)) = cut(&regions, &tried) else { break };
        tried.push((a, b));
        let Some(path) = cheapest_way(land, &blocked, anchor(starts[a]), anchor(starts[b])) else { continue };
        // Where on the way a tank could not pass, and a few anchors either side to ramp in.
        let footprint_ok = |land: &Land, blocked: &[bool], i: usize| -> bool {
            let (x, z) = (i % w, i / w);
            (z.saturating_sub(1)..=(z + 1).min(h - 1)).all(|cz| (x.saturating_sub(1)..=(x + 1).min(w - 1)).all(|cx| tank_cell(land, blocked, cx, cz)))
        };
        let mut carve = vec![false; path.len()];
        for (k, &i) in path.iter().enumerate() {
            if !footprint_ok(land, &blocked, i) {
                for c in carve.iter_mut().take((k + 4).min(path.len())).skip(k.saturating_sub(3)) {
                    *c = true;
                }
            }
        }
        // The way's ground: its own heights, no steeper than 10 a cell along it either way.
        let mut target: Vec<f64> = path.iter().map(|&i| land.heights[i] as f64).collect();
        for k in 1..target.len() {
            target[k] = target[k].clamp(target[k - 1] - 10.0, target[k - 1] + 10.0);
        }
        for k in (0..target.len().saturating_sub(1)).rev() {
            target[k] = target[k].clamp(target[k + 1] - 10.0, target[k + 1] + 10.0);
        }
        for ((&i, &t), _) in path.iter().zip(&target).zip(&carve).filter(|(_, c)| **c) {
            let (x, z) = ((i % w) as i64, (i / w) as i64);
            for dz in -1..=2 {
                for dx in -1..=2 {
                    let (nx, nz) = (x + dx, z + dz);
                    if nx < 0 || nz < 0 || nx >= w as i64 || nz >= h as i64 {
                        continue;
                    }
                    let n = nz as usize * w + nx as usize;
                    let height = t.round().clamp(0.0, 255.0) as u8;
                    if land.heights[n] != height {
                        land.heights[n] = height;
                        changed[n] = true;
                    }
                    if dx <= 1 && dz <= 1 {
                        if land.cliff[n] {
                            land.cliff[n] = false;
                            changed[n] = true;
                        }
                        // A blocking feature in the way goes, all of it.
                        let o = owner[n];
                        if o != u32::MAX {
                            cells[o as usize] = FEATURE_NONE;
                            changed[n] = true;
                            let fp = footprints.get(&o).copied().unwrap_or(1) as usize;
                            let (ox, oz) = (o as usize % w, o as usize / w);
                            for fz in oz..(oz + fp).min(h) {
                                for fx in ox..(ox + fp).min(w) {
                                    blocked[fz * w + fx] = false;
                                    owner[fz * w + fx] = u32::MAX;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let regions = tank_regions(land, &blocked);
    let cut_off: Vec<(usize, usize)> = pairs.iter().copied().filter(|&(a, b)| {
        let (ra, rb) = (regions[anchor(starts[a])], regions[anchor(starts[b])]);
        ra == 0 || ra != rb
    }).collect();
    // Names no cell uses any more go, the others renumbered in their order.
    let mut used = vec![false; names.len()];
    for &c in &cells {
        if (c as usize) < names.len() {
            used[c as usize] = true;
        }
    }
    let mut new = vec![FEATURE_NONE; names.len()];
    let mut kept = Vec::new();
    for (i, name) in names.into_iter().enumerate() {
        if used[i] {
            new[i] = kept.len() as u16;
            kept.push(name);
        }
    }
    for c in cells.iter_mut() {
        if (*c as usize) < new.len() {
            *c = new[*c as usize];
        }
    }
    Joined { cells, names: kept, cut_off, changed: changed.iter().filter(|&&c| c).count() }
}

/// The cheapest way for a tank's middle from anchor `from` to `to`: each step costs a cell, and
/// more for the cells of its footprint a tank cannot pass (cliffs and blocking features 20, too
/// steep 20, too deep 400); `None` where none is found.
fn cheapest_way(land: &Land, blocked: &[bool], from: usize, to: usize) -> Option<Vec<usize>> {
    let (w, h) = (land.frame.w, land.frame.h);
    let cost_of = |i: usize| -> u64 {
        let (x, z) = (i % w, i / w);
        if x == 0 || z == 0 || x + 1 >= w || z + 1 >= h {
            return u64::MAX;
        }
        let mut c = 1u64;
        for cz in z - 1..=z + 1 {
            for cx in x - 1..=x + 1 {
                let n = cz * w + cx;
                if land.depth(cx, cz) > TANK.2 {
                    c += 400;
                } else if land.cliff[n] || blocked[n] || land.slope(cx, cz) > TANK.1 {
                    c += 20;
                }
            }
        }
        c
    };
    let mut dist = vec![u64::MAX; w * h];
    let mut prev = vec![usize::MAX; w * h];
    let mut heap = BinaryHeap::new();
    dist[from] = 0;
    heap.push(std::cmp::Reverse((0u64, from)));
    while let Some(std::cmp::Reverse((d, i))) = heap.pop() {
        if i == to {
            break;
        }
        if d > dist[i] {
            continue;
        }
        let (x, z) = ((i % w) as i64, (i / w) as i64);
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= w as i64 || nz >= h as i64 {
                continue;
            }
            let n = nz as usize * w + nx as usize;
            let c = cost_of(n);
            if c == u64::MAX {
                continue;
            }
            let nd = d + c;
            if nd < dist[n] {
                dist[n] = nd;
                prev[n] = i;
                heap.push(std::cmp::Reverse((nd, n)));
            }
        }
    }
    if dist[to] == u64::MAX {
        return None;
    }
    let mut path = vec![to];
    let mut i = to;
    while i != from {
        i = prev[i];
        path.push(i);
    }
    path.reverse();
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::kit::{GREEN, tests::templates};
    use crate::convert::land::{self, Frame};
    use crate::map::Civilian;
    use crate::scmap::{Prop, ScMap};

    /// A 64 x 64 map: flat, a wall of cliff across it (x 30-33) with a gap two units wide (too
    /// narrow for a TA tank), two starts either side, mass and a hydrocarbon by each, trees, a
    /// rock and a wall.
    fn map() -> Map {
        let (w, h) = (64u32, 64u32);
        let heights = (0..=h).flat_map(|z| (0..=w).map(move |x| {
            let ridge = (31..=33).contains(&x) && !(30..=31).contains(&z);
            if ridge { 30 * 128 } else { 10 * 128 }
        })).collect();
        let scmap = ScMap {
            width: w,
            height: h,
            height_scale: 1.0 / 128.0,
            heights,
            props: vec![
                Prop { blueprint: "/env/evergreen/props/trees/pine01_prop.bp".into(), position: [10.5, 10.0, 40.5], ..Default::default() },
                Prop { blueprint: "/env/evergreen/props/trees/pine01_prop.bp".into(), position: [11.5, 10.0, 40.5], ..Default::default() },
                Prop { blueprint: "/env/evergreen/props/rocks/rock01_prop.bp".into(), position: [20.5, 10.0, 50.5], ..Default::default() },
            ],
            ..Default::default()
        };
        Map {
            scmap,
            starts: vec![[10.0, 30.0], [54.0, 30.0]],
            mass: vec![[14.5, 30.5], [50.5, 30.5]],
            hydrocarbons: vec![[10.5, 20.5]],
            civilians: vec![Civilian { unit: "ueb5101".into(), position: [40.5, 50.5], wreck: false }],
            ..Default::default()
        }
    }

    #[test]
    fn deposits_vents_props_and_walls_stand_where_the_game_put_them_and_tanks_get_through() {
        let m = map();
        let files = Files::of(Vec::new(), Vec::new());
        let mut land = land::build(&m.scmap, Frame::of([0.0, 0.0, 64.0, 64.0]));
        assert!(!land.cliff[30 * 64 + 32] && land.cliff[40 * 64 + 30], "the gap is open, the ridge's side a cliff");
        let placed = place(&m, &mut land, GREEN, &templates(), &files).unwrap();
        let at = |x: usize, z: usize| placed.names.get(placed.cells[z * 64 + x] as usize).map(String::as_str);
        assert_eq!(at(13, 29), Some("RockMetal3"), "a deposit centred on the mass point");
        assert_eq!(at(10, 20), Some("GreenVent01"));
        assert!(at(10, 40).is_some_and(|n| n.starts_with("SC1_Green_T")) && at(11, 40).is_some_and(|n| n.starts_with("SC1_Green_T")), "trees");
        assert!(at(40, 50).is_some_and(|n| n == "DragonsTeeth"));
        assert_eq!(placed.counts.deposits, 2);
        assert_eq!(placed.counts.vents, 1);
        assert_eq!(placed.counts.trees, 2);
        // The gap was too narrow for a tank: it was opened, and the bases joined.
        assert!(placed.counts.opened > 0);
        assert!(placed.cut_off.is_empty());
        let regions = tank_regions(&land, &vec![false; 64 * 64]);
        assert_eq!(regions[30 * 64 + 10], regions[30 * 64 + 54]);
        // The starts in TA's pixels.
        assert_eq!(placed.starts, [(160.0, 480.0), (864.0, 480.0)]);
    }
}
