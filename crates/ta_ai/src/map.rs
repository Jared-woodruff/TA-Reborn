//! Map knowledge: metal deposits, geothermal vents, coarse strength grids and terrain
//! reachability. Everything is integer or fixed point and iterated in a fixed order.

use ta_sim::world::{MoveSpec, Region, World};
use ta_sim::{Fx, FxVec3, Sim, UnitId};

/// A metal deposit (map feature) and what we know about it.
#[derive(Clone, Debug)]
pub(crate) struct Deposit {
    pub pos: FxVec3,
    /// Sum of the deposit's metal values (extraction scales with it).
    pub metal: i64,
    /// Player whose extractor stands on it.
    pub owner: Option<u8>,
    /// Our extractor on it and its type.
    pub ours: Option<(UnitId, usize)>,
    /// A blocking, reclaimable feature (usually a wreck) sitting on it.
    pub wreck: Option<usize>,
    /// Don't send builders before this tick (lost there recently, or a job failed).
    pub avoid_until: u64,
    /// How often we lost a building here.
    pub losses: u32,
    /// Terrain regions (per movement spec and working distance) from which a builder can
    /// work on the site.
    regions: Vec<(MoveSpec, i32, Vec<Region>)>,
    /// A deposit feature marks it (other buildings keep off); spots found in the metal
    /// layer alone are not reserved (on a map of metal ground they are everywhere).
    pub marked: bool,
    /// On dry land none of our ground constructors can get to (another island): work for
    /// constructors that fly or hover.
    pub far: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Vent {
    pub pos: FxVec3,
    pub avoid_until: u64,
    regions: Vec<(MoveSpec, i32, Vec<Region>)>,
}

pub(crate) fn find_deposits(sim: &Sim) -> Vec<Deposit> {
    let mut out: Vec<Deposit> = Vec::new();
    for f in &sim.features {
        let def = &sim.data.features[f.def];
        if !f.alive || !def.is_metal_deposit() {
            continue;
        }
        let fp = (def.footprint_x.max(1), def.footprint_z.max(1));
        let metal = sim.world.metal_under(f.origin, fp);
        if metal <= 0 || out.iter().any(|d| d.pos.within_xz(f.pos, Fx::from_int(24))) {
            continue;
        }
        out.push(Deposit { pos: f.pos, metal, owner: None, ours: None, wreck: None, avoid_until: 0, losses: 0, regions: Vec::new(), marked: true, far: false });
    }
    out
}

/// Metal an extractor spot must hold on average per cell to be worth one (TA's plain
/// ground holds a few; painted patches and metal worlds up to 255).
const SPOT_METAL: i64 = 40;
/// Metal-layer spots kept at most (the richest, then the nearest home).
const SPOTS_MAX: usize = 400;

/// Rich metal the deposit features don't mark (maps of metal ground, painted patches): the
/// best extractor-sized spots on a lattice an extractor apart, away from marked deposits.
pub(crate) fn find_metal_spots(sim: &Sim, marked: &[Deposit], fp: (u32, u32), home: FxVec3) -> Vec<Deposit> {
    let step = fp.0.max(fp.1) as i32 + 2;
    let (w, h) = (sim.world.width as i32, sim.world.height as i32);
    let mut spots: Vec<(i64, i64, FxVec3)> = Vec::new();
    for z in (0..h - fp.1 as i32).step_by(step as usize) {
        for x in (0..w - fp.0 as i32).step_by(step as usize) {
            let metal = sim.world.metal_under((x, z), fp);
            // Only where extractors may stand: over a deposit, or anywhere on a metal world.
            if metal < SPOT_METAL * (fp.0 * fp.1) as i64 || !sim.world.metal_world() && !sim.world.ore_under((x, z), fp) {
                continue;
            }
            let pos = sim.world.anchor_to_pos((x + fp.0 as i32 / 2, z + fp.1 as i32 / 2), fp);
            if marked.iter().any(|d| d.pos.within_xz(pos, Fx::from_int(64))) {
                continue;
            }
            spots.push((metal, (pos - home).xz().length_sq_raw() as i64, pos));
        }
    }
    spots.sort_unstable_by_key(|&(metal, d, p)| (std::cmp::Reverse(metal), d, p.x.0, p.z.0));
    spots.truncate(SPOTS_MAX);
    spots.into_iter().map(|(metal, _, pos)| Deposit { pos, metal, owner: None, ours: None, wreck: None, avoid_until: 0, losses: 0, regions: Vec::new(), marked: false, far: false }).collect()
}

pub(crate) fn find_vents(sim: &Sim) -> Vec<Vent> {
    let mut out: Vec<Vent> = Vec::new();
    for f in &sim.features {
        if f.alive && sim.data.features[f.def].geothermal && !out.iter().any(|v| v.pos.within_xz(f.pos, Fx::from_int(48))) {
            out.push(Vent { pos: f.pos, avoid_until: 0, regions: Vec::new() });
        }
    }
    out
}

/// Terrain region of a unit standing at `pos` (looking around a little when it stands on
/// an impassable cell, e.g. inside a factory yard). 0 = none found.
pub(crate) fn region_at(world: &World, spec: &MoveSpec, pos: FxVec3) -> Region {
    let a = World::pos_to_anchor(pos, spec.footprint);
    for r in 0..=3i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let c = world.component(spec, (a.0 + dx, a.1 + dz));
                if c != 0 {
                    return c;
                }
            }
        }
    }
    0
}

/// Regions touching the cells within `slack` cells of `pos`.
pub(crate) fn regions_near(world: &World, spec: &MoveSpec, pos: FxVec3, slack: i32) -> Vec<Region> {
    let a = World::pos_to_anchor(pos, spec.footprint);
    let mut out: Vec<Region> = Vec::new();
    for dz in -slack..=slack {
        for dx in -slack..=slack {
            let c = world.component(spec, (a.0 + dx, a.1 + dz));
            if c != 0 && !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out.sort_unstable();
    out
}

/// Whether a unit with movement `spec` at `from` can get within `slack` cells of `to`.
pub(crate) fn can_reach(world: &World, spec: &MoveSpec, from: FxVec3, to: FxVec3, slack: i32) -> bool {
    let r = region_at(world, spec, from);
    r != 0 && world.region_within(spec, r, World::pos_to_anchor(to, spec.footprint), slack)
}

/// How far every cell lies from a movement region (cells, Chebyshev, capped at 254): what
/// ships of a class in that sea can get within gun range of, and where a hover or naval
/// transport can take units on or set them down. One sweep per class and sea answers every
/// such question after it.
#[derive(Clone, Debug)]
pub(crate) struct Reach {
    pub spec: MoveSpec,
    pub region: Region,
    w: i32,
    h: i32,
    dist: Vec<u8>,
    /// Per enemy start: the nearest point of the region (where a fleet looks for the
    /// enemy when it knows of nothing to hit).
    pub coast: Vec<(FxVec3, Option<FxVec3>)>,
}

impl Reach {
    pub fn new(world: &World, spec: &MoveSpec, region: Region) -> Reach {
        let (w, h) = (world.width as i32, world.height as i32);
        let mut dist = vec![u8::MAX; (w * h).max(0) as usize];
        let mut queue = std::collections::VecDeque::new();
        for z in 0..h {
            for x in 0..w {
                if world.component(spec, (x, z)) == region {
                    dist[(z * w + x) as usize] = 0;
                    queue.push_back((x, z));
                }
            }
        }
        while let Some((x, z)) = queue.pop_front() {
            let d = dist[(z * w + x) as usize];
            if d >= u8::MAX - 1 {
                continue;
            }
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let (nx, nz) = (x + dx, z + dz);
                    if nx < 0 || nz < 0 || nx >= w || nz >= h {
                        continue;
                    }
                    let i = (nz * w + nx) as usize;
                    if dist[i] == u8::MAX {
                        dist[i] = d + 1;
                        queue.push_back((nx, nz));
                    }
                }
            }
        }
        Reach { spec: *spec, region, w, h, dist, coast: Vec::new() }
    }

    /// Cells between `pos` and the region (0 inside it).
    pub fn cells_to(&self, pos: FxVec3) -> i32 {
        let (x, z) = World::pos_to_anchor(pos, self.spec.footprint);
        let (x, z) = (x.clamp(0, self.w - 1), z.clamp(0, self.h - 1));
        self.dist.get((z * self.w + x) as usize).map_or(i32::MAX, |&d| d as i32)
    }
}

impl Deposit {
    /// Whether a builder with movement `spec` standing in `region` can work on this deposit.
    pub fn reachable(&mut self, world: &World, spec: &MoveSpec, region: Region, slack: i32) -> bool {
        reachable_cached(&mut self.regions, world, spec, self.pos, region, slack)
    }
}

impl Vent {
    pub fn reachable(&mut self, world: &World, spec: &MoveSpec, region: Region, slack: i32) -> bool {
        reachable_cached(&mut self.regions, world, spec, self.pos, region, slack)
    }
}

fn reachable_cached(cache: &mut Vec<(MoveSpec, i32, Vec<Region>)>, world: &World, spec: &MoveSpec, pos: FxVec3, region: Region, slack: i32) -> bool {
    if region == 0 {
        return false;
    }
    if let Some((_, _, rs)) = cache.iter().find(|(s, sl, _)| s == spec && *sl == slack) {
        return rs.binary_search(&region).is_ok();
    }
    let rs = regions_near(world, spec, pos, slack);
    let ok = rs.binary_search(&region).is_ok();
    cache.push((*spec, slack, rs));
    ok
}

/// A coarse grid of summed strengths (enemy threat, anti-air cover...).
#[derive(Clone, Debug)]
pub(crate) struct Grid {
    w: i64,
    h: i64,
    v: Vec<i64>,
}

/// Grid cell size in pixels.
pub(crate) const GRID_CELL: i64 = 256;

impl Grid {
    pub fn new(sim: &Sim) -> Grid {
        let (w, h) = sim.world.size_px();
        let (w, h) = (w.to_int() / GRID_CELL + 1, h.to_int() / GRID_CELL + 1);
        Grid { w, h, v: vec![0; (w * h) as usize] }
    }

    pub fn clear(&mut self) {
        self.v.iter_mut().for_each(|x| *x = 0);
    }

    fn cell(&self, pos: FxVec3) -> (i64, i64) {
        ((pos.x.to_int() / GRID_CELL).clamp(0, self.w - 1), (pos.z.to_int() / GRID_CELL).clamp(0, self.h - 1))
    }

    pub fn add(&mut self, pos: FxVec3, value: i64) {
        let (x, z) = self.cell(pos);
        self.v[(z * self.w + x) as usize] += value;
    }

    /// Adds `value` to every cell within `radius` pixels of `pos`.
    pub fn add_disc(&mut self, pos: FxVec3, radius: i64, value: i64) {
        let (cx, cz) = self.cell(pos);
        let r = radius / GRID_CELL + 1;
        for z in (cz - r).max(0)..=(cz + r).min(self.h - 1) {
            for x in (cx - r).max(0)..=(cx + r).min(self.w - 1) {
                let (px, pz) = (x * GRID_CELL + GRID_CELL / 2, z * GRID_CELL + GRID_CELL / 2);
                let (dx, dz) = (px - pos.x.to_int(), pz - pos.z.to_int());
                if dx * dx + dz * dz <= (radius + GRID_CELL / 2) * (radius + GRID_CELL / 2) {
                    self.v[(z * self.w + x) as usize] += value;
                }
            }
        }
    }

    /// Sum over the cells within `radius` pixels of `pos` (whole cells).
    pub fn around(&self, pos: FxVec3, radius: i64) -> i64 {
        let (cx, cz) = self.cell(pos);
        let r = (radius + GRID_CELL / 2) / GRID_CELL;
        let mut sum = 0;
        for z in (cz - r).max(0)..=(cz + r).min(self.h - 1) {
            for x in (cx - r).max(0)..=(cx + r).min(self.w - 1) {
                sum += self.v[(z * self.w + x) as usize];
            }
        }
        sum
    }

    pub fn at(&self, pos: FxVec3) -> i64 {
        let (x, z) = self.cell(pos);
        self.v[(z * self.w + x) as usize]
    }
}
