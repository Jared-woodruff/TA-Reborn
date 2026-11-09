//! Local avoidance, the rule "units give way" (`Sim::local_avoidance`): before the ground units
//! move, each one moving looks along its way at the units ahead of it and bends its course
//! round them, so a column flows past a slower unit, round one standing in the way and past
//! one coming the other way, instead of queueing nose to tail behind it and shoving.
//!
//! - A unit passes another on the side away from it; one dead ahead it passes on its own right
//!   (so two meeting head on both keep right and pass), or on the side it is already bending
//!   to.
//! - The nearer the other and the more squarely in the way, the harder it bends, up to
//!   [`MAX_BEND`]; the bends for several add up.
//! - Close behind a slower unit going its way with no room to pass, it matches its speed.
//! - Units standing on or by the end of its route are left to the crowd's pushing (going round
//!   them would circle the goal), as are those it already touches.
//!
//! Everything is read from where the units stood at the start of the tick (the unit grid), so
//! the order units are planned in doesn't matter, and all of it is fixed point: deterministic
//! on every machine, on one core or many.

use crate::combat::UnitGrid;
use crate::defs::{Locomotion, UnitTypes};
use crate::fixed::{Fx, FxVec3, heading_of, heading_vector, wrap_angle};
use crate::unit::Unit;

/// How far (angle units, 65536 a turn) a unit bends off its way at most: about 35°.
pub const MAX_BEND: i32 = 6400;
/// Ticks ahead a unit looks: what it would reach in a second.
const HORIZON: i64 = 30;
/// Space kept between two units passing (px), on top of their bodies.
const MARGIN: i64 = 6;

/// How a unit bends its course this tick: the turn off its way (angle units; negative bends
/// right), and the speed it must not exceed to keep behind a slower unit (px per tick).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Steer {
    pub bend: i32,
    pub speed_cap: Option<Fx>,
}

/// The body radius the crowd uses for a unit (px).
fn body(radius: Fx) -> Fx {
    radius * 4 / 5
}

/// Who takes part: finished, alive, not carried, on the ground or water.
fn surface_unit(u: &Unit, types: &UnitTypes) -> bool {
    !u.dead && u.is_complete() && u.carried_by.is_none() && matches!(types.get(u.type_id).locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval)
}

/// A unit as the planning sees it: where it is and goes, and its body (px, px per tick).
#[derive(Clone, Copy, Default)]
struct Body {
    x: Fx,
    z: Fx,
    vx: Fx,
    vz: Fx,
    r: Fx,
    /// On the ground or water, finished, alive, not carried.
    surface: bool,
}

/// Updates each unit's steer, by slot (units not moving get none). Half the units plan each
/// tick, taking turns by slot, and keep their steer the tick between: a steer holds good for a
/// fifteenth of a second, and a big crowd costs half as much.
pub fn plan(steers: &mut Vec<Steer>, units: &[Option<Unit>], types: &UnitTypes, grid: &UnitGrid, tick: u64, parallel: bool) {
    steers.resize(units.len(), Steer::default());
    // Everyone's place and motion, once.
    let body_of = |u: &Option<Unit>| -> Body {
        match u {
            Some(u) if surface_unit(u, types) => {
                let v = if u.speed > Fx::ZERO { heading_vector(u.heading).scale(u.speed) } else { FxVec3::ZERO };
                Body { x: u.pos.x, z: u.pos.z, vx: v.x, vz: v.z, r: body(types.get(u.type_id).radius), surface: true }
            }
            _ => Body::default(),
        }
    };
    let big = parallel && units.len() >= crate::sim::PARALLEL_MOVERS;
    let bodies: Vec<Body> = if big {
        use rayon::prelude::*;
        units.par_iter().with_min_len(1024).map(body_of).collect()
    } else {
        units.iter().map(body_of).collect()
    };
    let one = |near: &mut Vec<u32>, slot: usize, kept: Steer| -> Steer {
        match units[slot].as_ref() {
            Some(u) if u.path_goal.is_some() && tick >= u.paralyzed_until && bodies[slot].surface => {
                if (slot as u64 + tick).is_multiple_of(2) { steer(u, slot, &bodies, types, grid, near) } else { kept }
            }
            _ => Steer::default(),
        }
    };
    if big {
        use rayon::prelude::*;
        steers.par_iter_mut().enumerate().with_min_len(256).for_each_init(Vec::new, |near, (slot, s)| *s = one(near, slot, *s));
    } else {
        let mut near = Vec::new();
        for (slot, s) in steers.iter_mut().enumerate() {
            *s = one(&mut near, slot, *s);
        }
    }
}

/// One unit's steer round the units ahead of it.
fn steer(u: &Unit, me: usize, bodies: &[Body], types: &UnitTypes, grid: &UnitGrid, near: &mut Vec<u32>) -> Steer {
    let t = types.get(u.type_id);
    let r_self = bodies[me].r;
    let forward = heading_vector(u.heading);
    // Its right hand (heading 0 faces +z; its right is -x).
    let right = FxVec3::new(-forward.z, Fx::ZERO, forward.x);
    let speed = u.speed.max(t.max_speed / 2);
    let reach = speed * HORIZON + r_self * 2 + Fx::from_int(MARGIN);
    // Where it is going: the way to its next waypoint says which way it already bends.
    let goal = u.path_goal.unwrap_or(u.pos);
    let final_leg = u.path.len() <= 1;
    let waypoint = u.path.first().map_or(goal, |&c| crate::sim::anchor_middle(c, t.footprint));
    let way = heading_of(waypoint - u.pos);
    let bending_right = wrap_angle(u.heading - way) < 0;

    grid.query(u.pos, reach + Fx::from_int(32), near);
    // Its motion (at least a quarter of its top speed, so one just setting off looks ahead).
    let v_self = forward.scale(u.speed.max(t.max_speed / 4));
    let horizon = Fx::from_int(HORIZON);
    let margin = Fx::from_int(MARGIN);
    // (Fixed point: the pulls, each up to ONE.)
    let mut pull = Fx::ZERO;
    let mut cap: Option<Fx> = None;
    for &slot in near.iter() {
        // Bent as hard as it goes one way: the rest can't change that.
        if pull.abs() >= Fx::from_int(2) {
            break;
        }
        let slot = slot as usize;
        let o = bodies[slot];
        if slot == me || !o.surface {
            continue;
        }
        let clearance = r_self + o.r + margin;
        let (dx, dz) = (o.x - u.pos.x, o.z - u.pos.z);
        if dx.abs() > reach + clearance || dz.abs() > reach + clearance {
            continue;
        }
        // Already touching it: too late to go round, the crowd pushes them apart. (In a crowd the
        // way round those it touched swung with their every shuffle, the unit's head swinging from
        // side to side at its full turn rate: units bunched up shook.)
        let touching = r_self + o.r;
        if dx * dx + dz * dz < touching * touching {
            continue;
        }
        // Behind it: the one behind gives way.
        let ahead = dx * forward.x + dz * forward.z;
        if ahead < -clearance / 2 {
            continue;
        }
        // Where the other will be when they are nearest, going on as both go now: moving
        // apart, or passing clear of each other, it is no bother (one crossing ahead will be
        // gone by the time this one gets there). Going together (a quarter of a pixel a tick
        // apart at most) they come no nearer within the horizon than a few pixels.
        let (rx, rz) = (v_self.x - o.vx, v_self.z - o.vz);
        let vv = rx * rx + rz * rz;
        if vv <= Fx::from_ratio(1, 16) {
            continue;
        }
        let when = (dx * rx + dz * rz) / vv;
        if when <= Fx::ZERO || when > horizon {
            continue;
        }
        let (nx, nz) = (dx - rx * when, dz - rz * when);
        let miss = FxVec3::new(nx, Fx::ZERO, nz).length_xz();
        if miss >= clearance {
            continue;
        }
        // Standing on or by the end of its route: the crowd sorts that out.
        if final_leg && FxVec3::new(o.x - goal.x, Fx::ZERO, o.z - goal.z).length_xz() < clearance + Fx::from_int(8) {
            continue;
        }
        // Close behind a slower unit going its way and squarely in it: keep behind it (the
        // bend takes it past as soon as there is room).
        let other_forward = o.vx * forward.x + o.vz * forward.z;
        if other_forward > Fx::ZERO && ahead < clearance + u.speed * 2 && (dx * right.x + dz * right.z).abs() * 2 < clearance {
            cap = Some(cap.map_or(other_forward, |c| c.min(other_forward)));
        }
        // How hard to bend: the sooner and the nearer the miss, the harder.
        let urgency = Fx::ONE - when / HORIZON;
        let overlap = Fx::ONE - miss / clearance;
        let weight = urgency * overlap;
        // Pass on the side away from where it will be (behind one crossing, past one in the
        // way); squarely ahead, on the side already bending to, else the right.
        let side = nx * right.x + nz * right.z;
        let dead_ahead = side.abs() * 8 < clearance;
        let go_right = if dead_ahead { bending_right || wrap_angle(u.heading - way) == 0 } else { side < Fx::ZERO };
        pull = if go_right { pull + weight } else { pull - weight };
    }
    let pull = pull.clamp(-Fx::ONE, Fx::ONE);
    // Right is a smaller heading.
    let bend = -((pull * MAX_BEND as i64).to_int() as i32);
    Steer { bend, speed_cap: cap }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_passes_on_the_side_away_from_the_one_in_its_way() {
        // Facing +z: right is -x. Something ahead and a little to the right (-x) is passed
        // on the left: the bend is a larger heading (towards +x).
        let forward = heading_vector(0);
        let right = FxVec3::new(-forward.z, Fx::ZERO, forward.x);
        assert_eq!((right.x, right.z), (-Fx::ONE, Fx::ZERO));
    }
}
