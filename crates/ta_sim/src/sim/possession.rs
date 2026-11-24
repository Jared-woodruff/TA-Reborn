//! Driving a unit: its owner takes it over from the first-person view (`Command::DirectControl`,
//! sent by `ta_game`'s perspective while the player has taken control) and steers, aims and fires
//! it by hand, its orders waiting until it is let go.

use super::*;
use crate::unit::{DIRECT_CONTROL_TICKS, DirectControl, DirectInput};
use ta_data::unit::MAX_WEAPONS;

impl Sim {
    /// `player`'s input for their unit `id`, or (`None`) the unit let go. Taken over, a unit stops
    /// where it stands ([`Sim::take_over`]); its orders wait (`order_unit` gives it no new ones,
    /// `update_orders` leaves them be). Let go, it stops again and takes up its orders where they
    /// were ([`Sim::let_go`]). Only the owner's own finished ground, hover or naval unit, standing
    /// free (not carried), is driven, and only with input within its bounds (a throttle and a turn
    /// of -1, 0 or 1, an aim's heading and pitch on the circle); its owner lets it go in whatever
    /// state it is (one being taken apart too: built up again before its input lapsed, it would
    /// carry on with that input).
    pub(super) fn set_direct_control(&mut self, player: u8, id: UnitId, input: Option<DirectInput>) {
        let Some(unit) = self.unit(id).filter(|unit| unit.owner == player) else { return };
        let Some(input) = input else {
            if unit.direct_control.is_some() {
                self.let_go(id);
            }
            return;
        };
        let ty = self.types.get(unit.type_id);
        if unit.dead || !unit.is_complete() || unit.carried_by.is_some()
            || !matches!(ty.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval) {
            return;
        }
        if !(-1..=1).contains(&input.throttle) || !(-1..=1).contains(&input.turn) || !(-32768..=32767).contains(&input.heading) || !(-16384..=16384).contains(&input.pitch) {
            return;
        }
        let entering = unit.direct_control.is_none();
        // (The piece its sight is drawn from, found on its first tick driven: kept.)
        let aim_piece = unit.direct_control.and_then(|control| control.aim_piece);
        if entering {
            self.take_over(id);
        }
        let expires = self.tick + DIRECT_CONTROL_TICKS;
        if let Some(unit) = self.unit_mut(id) {
            unit.direct_control = Some(DirectControl { player, input, expires, aim_piece });
        }
    }

    /// A unit taken over: it stops where it stands (its orders wait, their way forgotten), leaves
    /// the formations it marched in (they do not wait for it), its build job ends (its nano stream
    /// and its build stance with it; not a factory's product on its pad, which goes on being made)
    /// and its weapons let go of their targets.
    fn take_over(&mut self, id: UnitId) {
        self.halt(id);
        let yard = self.unit(id).and_then(|unit| unit.build).is_some_and(|job| self.unit(job.target).is_some_and(|product| product.factory == Some(id)));
        if !yard {
            self.finish_build_job(id);
        }
        self.leave_formations(id);
        if let Some(unit) = self.unit_mut(id) {
            for slot in 0..MAX_WEAPONS {
                unit.retarget(slot, None);
            }
        }
    }

    /// A driven unit let go (by its player, its input lapsed, or no longer theirs to drive): it
    /// stops and takes up its orders where they were. Going astern it comes to a stop (its orders
    /// only move it ahead: from astern it gathered way at its acceleration, counted itself stuck
    /// meanwhile and planned its way again). Its weapons let go of what the sight was on, turrets
    /// and arms going back to rest as when a target is lost (`TargetCleared`).
    pub(super) fn let_go(&mut self, id: UnitId) {
        let rest = self.unit(id).is_some_and(|unit| !unit.dead && self.types.get(unit.type_id).callins.target_cleared);
        let mut held = [false; MAX_WEAPONS];
        if let Some(unit) = self.unit_mut(id) {
            unit.direct_control = None;
            unit.speed = unit.speed.max(Fx::ZERO);
            for (slot, held) in held.iter_mut().enumerate() {
                *held = unit.weapons[slot].target.is_some();
                unit.retarget(slot, None);
            }
        }
        self.halt(id);
        if rest {
            for slot in (0..MAX_WEAPONS).filter(|&slot| held[slot]) {
                self.start_script(id, "TargetCleared", &[slot as i32 + 1]);
            }
        }
    }

    /// One tick of a driven unit's movement (`move_ground_units`, in place of `ground_step`): it
    /// turns at its turn rate, on the spot too, gathers speed towards its top speed (half of it in
    /// reverse) at its acceleration and sheds it at its brake rate, slides along what it runs into,
    /// and settles on the ground or the water. Held still by its script (`IMMOBILE`: a weapon
    /// unpacked) it stays where it stands, told it is wanted on the move (`StartMoving`) so that
    /// its script packs up and lets it go, as `ground_step` tells one with somewhere to go.
    pub(super) fn direct_step(unit: &mut Unit, ty: &UnitType, spec: MoveSpec, world: &World, input: DirectInput) {
        if unit.immobile {
            unit.speed = Fx::ZERO;
            let wants = input.throttle != 0 || input.turn != 0;
            if wants != unit.moving {
                unit.moving = wants;
                if wants {
                    // (Packing up to go: it aims anew once let go, as `ground_step` has it.)
                    unit.forget_aims();
                }
                if let Some(script) = &mut unit.cob {
                    script.start(if wants { "StartMoving" } else { "StopMoving" }, &[], ThreadTag::None);
                }
            }
            return;
        }
        unit.heading = wrap_angle(unit.heading - input.turn as i32 * ty.turn_rate);
        let top = if input.throttle < 0 { ty.max_speed / 2 } else { ty.max_speed };
        let target = top * input.throttle as i64;
        unit.speed = Self::driven_speed(unit.speed, target, ty);
        // Straight on, else sliding along what it met (its x alone, its z alone). A slide that goes
        // nowhere (driven square into a wall, the step all along one axis) is as blocked as the
        // rest: taken, the unit ran on the spot at full speed.
        let step = heading_vector(unit.heading).scale(unit.speed);
        let candidates = [unit.pos + step, unit.pos + FxVec3::new(step.x, Fx::ZERO, Fx::ZERO), unit.pos + FxVec3::new(Fx::ZERO, Fx::ZERO, step.z)];
        let ignore = unit.factory.map(|factory| factory.slot);
        let current_ok = world.passable(&spec, World::pos_to_anchor(unit.pos, ty.footprint), ignore);
        let here = unit.pos;
        if let Some(position) = candidates
            .into_iter()
            .find(|&point| (point.x != here.x || point.z != here.z) && world.ground_segment_open(&spec, here, point, ignore, !current_ok))
        {
            unit.pos.x = position.x;
            unit.pos.z = position.z;
        } else {
            unit.speed = Fx::ZERO;
        }
        let moving = !unit.speed.is_zero();
        if moving != unit.moving {
            unit.moving = moving;
            if let Some(script) = &mut unit.cob {
                script.start(if moving { "StartMoving" } else { "StopMoving" }, &[], ThreadTag::None);
            }
        }
        Self::settle(unit, ty, world);
    }

    /// A driven unit's next speed, from `speed` towards `target` (ahead positive, astern negative):
    /// way it is to shed (going the other way, or faster than it is to go) it sheds at its brake
    /// rate, down to a stop at most; way it gathers (from a stop, or past it the other way) at its
    /// acceleration. (Braking from a stop, it set off at its brake rate, with a lurch.)
    fn driven_speed(speed: Fx, target: Fx, ty: &UnitType) -> Fx {
        let sheds = !speed.is_zero() && (speed.signum() != target.signum() || target.abs() < speed.abs());
        if !sheds {
            return speed.approach(target, ty.acceleration);
        }
        let braked = speed.approach(if speed.signum() == target.signum() { target } else { Fx::ZERO }, ty.brake);
        if braked.is_zero() { braked.approach(target, ty.acceleration) } else { braked }
    }
}
