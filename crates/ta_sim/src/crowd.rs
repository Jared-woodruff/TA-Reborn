//! Crowd separation: keeps ground units from overlapping.
//!
//! Every tick the positions of all ground units are snapshotted into a 64 px bucket grid
//! (counting sort, no per-tick allocation after warm-up). Each unit's push is computed from
//! the snapshot only, so the result does not depend on processing order (the pushes of all
//! those overlapping it averaged, so a crowd settles instead of going to and fro), then applied
//! where the terrain allows. Movers yield more than stationary units, so traffic flows around idle
//! units instead of through them. Two of a side that have held each other up head on (in a gap
//! one unit wide, say), or one of a side standing idle in the other's way there, slip past each
//! other: see `Sim::ground_step`. All maths is integer (fixed point in `i64`).

use crate::defs::Locomotion;
use crate::fixed::{Fx, FxVec3, heading_vector};
use crate::sim::Sim;
use crate::world::World;

const CELL_PX: i64 = 64;
const CELL_RAW: i64 = CELL_PX << 16;
/// Maximum correction per tick (2 px), keeping motion smooth.
const MAX_STEP: i64 = 2 << 16;

#[derive(Clone, Copy)]
struct Body {
    slot: u32,
    x: i64,
    z: i64,
    r: i64,
    moving: bool,
    /// Which way it faces (raw fixed point), whose it is, and whether it slips past its own
    /// side's units coming the other way or standing in its way (`Unit::squeeze_until`).
    hx: i64,
    hz: i64,
    owner: u8,
    squeeze: bool,
}

/// A push on a body: its slot, the push (raw fixed point) and whether one of its own side's
/// units coming the other way, or standing in its way, was among the pushers.
type Push = (u32, i64, i64, bool);

#[derive(Clone, Default)]
pub struct CrowdGrid {
    width: i64,
    height: i64,
    starts: Vec<u32>,
    bodies: Vec<Body>,
    sorted: Vec<Body>,
    pushes: Vec<Push>,
    max_radius: i64,
}

fn isqrt_u64(n: u64) -> u64 {
    // Float estimate then exact integer correction: deterministic on every platform.
    let mut x = (n as f64).sqrt() as u64;
    while x * x > n {
        x -= 1;
    }
    while (x + 1) * (x + 1) <= n {
        x += 1;
    }
    x
}

impl CrowdGrid {
    fn cell(&self, x: i64, z: i64) -> (i64, i64) {
        ((x / CELL_RAW).clamp(0, self.width - 1), (z / CELL_RAW).clamp(0, self.height - 1))
    }

    fn build(&mut self, world: &World) {
        self.max_radius = self.bodies.iter().map(|b| b.r).max().unwrap_or(0);
        self.width = (world.width as i64 * 16 / CELL_PX) + 1;
        self.height = (world.height as i64 * 16 / CELL_PX) + 1;
        let n = (self.width * self.height) as usize;
        self.starts.clear();
        self.starts.resize(n + 1, 0);
        for b in &self.bodies {
            let (cx, cz) = ((b.x / CELL_RAW).clamp(0, self.width - 1), (b.z / CELL_RAW).clamp(0, self.height - 1));
            self.starts[(cz * self.width + cx) as usize + 1] += 1;
        }
        for i in 1..=n {
            self.starts[i] += self.starts[i - 1];
        }
        self.sorted.clear();
        self.sorted.resize(self.bodies.len(), Body { slot: 0, x: 0, z: 0, r: 0, moving: false, hx: 0, hz: 0, owner: 0, squeeze: false });
        let mut fill = self.starts.clone();
        for b in &self.bodies {
            let (cx, cz) = ((b.x / CELL_RAW).clamp(0, self.width - 1), (b.z / CELL_RAW).clamp(0, self.height - 1));
            let c = (cz * self.width + cx) as usize;
            self.sorted[fill[c] as usize] = *b;
            fill[c] += 1;
        }
    }
}

impl Sim {
    pub(crate) fn resolve_collisions(&mut self) {
        use rayon::prelude::*;
        let types = self.types.clone();
        let mut grid = std::mem::take(&mut self.crowd);
        let tick = self.tick;
        // (Aircraft once they have landed: planes sent to one place used to set down all on the
        // same spot.)
        let body = |u: &Option<crate::unit::Unit>| -> Option<Body> {
            let u = u.as_ref()?;
            let t = types.get(u.type_id);
            let surface = matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval) || t.locomotion == Locomotion::Air && u.landed && u.air_pad.is_none();
            if u.dead || !u.is_complete() || u.carried_by.is_some() || !surface {
                return None;
            }
            let h = heading_vector(u.heading);
            Some(Body {
                slot: u.id.slot,
                x: u.pos.x.0,
                z: u.pos.z.0,
                r: (t.radius * 4 / 5).0,
                moving: u.path_goal.is_some(),
                hx: h.x.0,
                hz: h.z.0,
                owner: u.owner,
                squeeze: tick < u.squeeze_until,
            })
        };
        // (In slot order either way.)
        let big = self.parallel && self.units.len() >= PARALLEL_BODIES;
        if big {
            grid.bodies = self.units.par_iter().with_min_len(1024).filter_map(body).collect();
        } else {
            grid.bodies.clear();
            grid.bodies.extend(self.units.iter().filter_map(body));
        }
        grid.build(&self.world);
        // Every push comes from the snapshot alone, so bodies are independent: large crowds
        // are computed in parallel. The results keep the snapshot order, so the outcome is
        // identical to a sequential pass.
        let units = &self.units;
        let crowded = self.parallel && grid.sorted.len() >= PARALLEL_BODIES;
        let pushes: Vec<Push> = if crowded {
            (0..grid.sorted.len()).into_par_iter().with_min_len(256).filter_map(|i| body_push(&grid, units, i)).collect()
        } else {
            (0..grid.sorted.len()).filter_map(|i| body_push(&grid, units, i)).collect()
        };
        grid.pushes = pushes;
        // Where each pushed unit ends up: on the ground (or the water) there, or where it was if
        // it can't stand there. That is the world's to say, not the other units' (no push moves
        // another), so a big crowd's is worked out on all cores and then put in place.
        let (world, specs) = (&self.world, &self.move_specs);
        let sea = world.sea_level;
        let settle = |&(slot, px, pz, _): &Push| -> Option<(u32, FxVec3)> {
            let u = units[slot as usize].as_ref()?;
            let t = types.get(u.type_id);
            let target = u.pos + FxVec3::new(Fx(px), Fx::ZERO, Fx(pz));
            if t.locomotion == Locomotion::Air {
                // A landed aircraft: where it may set down (flat enough, off buildings and features).
                let y = world.air_landing_height(target, &t.landing())?;
                return Some((slot, FxVec3::new(target.x, y, target.z)));
            }
            if !world.ground_segment_open(&specs[t.id], u.pos, target, None, false) {
                return None;
            }
            let ground = world.height_at(target.x, target.z);
            let y = match t.locomotion {
                Locomotion::Hover | Locomotion::Naval => ground.max(sea),
                _ => ground,
            };
            Some((slot, FxVec3::new(target.x, y, target.z)))
        };
        let settled: Vec<(u32, FxVec3)> =
            if crowded { grid.pushes.par_iter().with_min_len(256).filter_map(settle).collect() } else { grid.pushes.iter().filter_map(settle).collect() };
        // (Who the crowd pushes, and who its own side's units coming the other way, or standing
        // in its way, push: a unit pressing into a crowd at its goal has arrived, one held up
        // head on slips past.)
        for &(slot, _, _, head_on) in &grid.pushes {
            if let Some(u) = self.units[slot as usize].as_mut() {
                u.jostled_tick = tick;
                if head_on {
                    u.head_on_tick = tick;
                }
            }
        }
        for (slot, pos) in settled {
            if let Some(u) = self.units[slot as usize].as_mut() {
                u.pos = pos;
            }
        }
        self.crowd = grid;
    }
}

/// Crowds at least this big separate on all cores.
const PARALLEL_BODIES: usize = 1_500;

/// The push on body `i` of the snapshot from every body overlapping it (quarters of the
/// overlap by who is moving), clamped to [`MAX_STEP`], and whether its own side's units coming
/// the other way, or standing idle in its way, pushed it; `None` when there is none. Two of a
/// side meeting so, one of them slipping past (`Body::squeeze`), go through each other.
fn body_push(grid: &CrowdGrid, units: &[Option<crate::unit::Unit>], i: usize) -> Option<Push> {
    let a = grid.sorted[i];
    // Units leaving a factory pass through the yard freely.
    if units[a.slot as usize].as_ref().is_some_and(|u| u.factory.is_some()) {
        return None;
    }
    let (cx, cz) = grid.cell(a.x, a.z);
    let (mut px, mut pz) = (0i64, 0i64);
    let mut pushers = 0i64;
    let mut head_on = false;
    let reach = (a.r + grid.max_radius + CELL_RAW - 1) / CELL_RAW;
    for nz in (cz - reach).max(0)..=(cz + reach).min(grid.height - 1) {
        for nx in (cx - reach).max(0)..=(cx + reach).min(grid.width - 1) {
            let c = (nz * grid.width + nx) as usize;
            for j in grid.starts[c] as usize..grid.starts[c + 1] as usize {
                let b = grid.sorted[j];
                if b.slot == a.slot {
                    continue;
                }
                let min = a.r + b.r;
                let (dx, dz) = (a.x - b.x, a.z - b.z);
                if dx.abs() >= min || dz.abs() >= min {
                    continue;
                }
                // 16.16 → 8.8 before squaring keeps the product well inside i64.
                let (sx, sz, sm) = (dx >> 8, dz >> 8, min >> 8);
                let d2 = (sx * sx + sz * sz) as u64;
                if d2 >= (sm * sm) as u64 {
                    continue;
                }
                // (Facing apart by more than a right angle; raw headings are at most 2^16.)
                let side = a.owner == b.owner;
                let squeezing = a.squeeze || b.squeeze;
                let facing_off = a.moving && b.moving && side && a.hx * b.hx + a.hz * b.hz < 0;
                if facing_off && squeezing {
                    continue;
                }
                head_on |= facing_off;
                // One of its side standing in its way, not fighting, holds it up as one coming
                // the other way does: in a pass one unit wide, an army gathering there (the
                // computer player's rally point at the top of Shore To Shore's pass) held up
                // everyone going through. (A line holding a fight is not slipped through.) Whether
                // the one standing fights is looked up only where it counts: one slipping past,
                // or one moving not yet known to be held up.
                if side && a.moving != b.moving && (squeezing || a.moving && !head_on) {
                    let slot = if a.moving { b.slot } else { a.slot };
                    let standing = !units[slot as usize].as_ref().is_some_and(|u| u.weapons.iter().any(|w| w.target.is_some()));
                    if standing && squeezing {
                        continue;
                    }
                    head_on |= standing && a.moving;
                }
                let dist = (isqrt_u64(d2) as i64) << 8;
                let overlap = min - dist;
                let share = match (a.moving, b.moving) {
                    (true, false) => 3,
                    (false, true) => 1,
                    _ => 2,
                }; // quarters of the overlap this unit takes
                let amount = overlap * share / 4;
                pushers += 1;
                if dist > (1 << 12) {
                    px += dx * amount / dist;
                    pz += dz * amount / dist;
                } else {
                    // Coincident: separate along a slot-derived direction.
                    let d = heading_vector((a.slot as i32).wrapping_mul(9973));
                    px += (d.x * Fx(amount)).0;
                    pz += (d.z * Fx(amount)).0;
                }
            }
        }
    }
    if px != 0 || pz != 0 {
        // The pushes averaged over its pushers: summed, a unit between two others took half of
        // each overlap from either side, all at once, so what was left of the overlaps next tick
        // was the mean of its neighbours' and a crowd went to and fro tick by tick for good (the
        // units shook). Averaged, one pushed by one still clears it in a tick.
        px /= pushers;
        pz /= pushers;
        // Clamp the step length (Chebyshev bound is enough for smoothing).
        let m = px.abs().max(pz.abs());
        if m > MAX_STEP {
            px = px * MAX_STEP / m;
            pz = pz * MAX_STEP / m;
        }
        return Some((a.slot, px, pz, head_on));
    }
    None
}
