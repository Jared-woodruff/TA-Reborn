//! Driving a unit: its owner takes it over from the first-person view (`Command::DirectControl`,
//! sent by `ta_game`'s perspective while the player has taken control) and steers, aims and fires
//! it by hand, its orders waiting until it is let go.

use super::*;
use crate::unit::{DIRECT_CONTROL_TICKS, DirectControl, DirectInput};

impl Sim {
    /// `player`'s input for their unit `id`, or (`None`) the unit let go. Taken over, a unit stops
    /// where it stands and its weapons let go of their targets; its orders wait (`order_unit` gives
    /// it no new ones, `update_orders` leaves them be). Let go, it stops again and takes up its
    /// orders where they were. Only the owner's own finished ground, hover or naval unit, standing
    /// free (not carried), is driven, and only with input within its bounds (a throttle and a turn
    /// of -1, 0 or 1, an aim's heading and pitch on the circle).
    pub(super) fn set_direct_control(&mut self, player: u8, id: UnitId, input: Option<DirectInput>) {
        let Some(unit) = self.unit(id) else { return };
        let ty = self.types.get(unit.type_id);
        if unit.owner != player || unit.dead || !unit.is_complete() || unit.carried_by.is_some()
            || !matches!(ty.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval) {
            return;
        }
        if input.is_some_and(|i| !(-1..=1).contains(&i.throttle) || !(-1..=1).contains(&i.turn) || !(-32768..=32767).contains(&i.heading) || !(-16384..=16384).contains(&i.pitch)) {
            return;
        }
        let entering = unit.direct_control.is_none();
        if entering && input.is_none() {
            return;
        }
        // (The piece its sight is drawn from, found on its first tick driven: kept.)
        let aim_piece = unit.direct_control.and_then(|control| control.aim_piece);
        if entering || input.is_none() {
            self.halt(id);
        }
        let expires = self.tick + DIRECT_CONTROL_TICKS;
        if let Some(unit) = self.unit_mut(id) {
            unit.direct_control = input.map(|input| DirectControl { player, input, expires, aim_piece });
            if entering || input.is_none() {
                for slot in 0..ta_data::unit::MAX_WEAPONS {
                    unit.retarget(slot, None);
                }
            }
        }
    }

    /// One tick of a driven unit's movement (`move_ground_units`, in place of `ground_step`): it
    /// turns at its turn rate, on the spot too, gathers speed towards its top speed (half of it in
    /// reverse) and brakes, slides along what it runs into, and settles on the ground or the
    /// water. Held still by its script (`IMMOBILE`: a weapon unpacked) it stays where it stands,
    /// told it is wanted on the move (`StartMoving`) so that its script packs up and lets it go,
    /// as `ground_step` tells one with somewhere to go.
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
        let acceleration = if unit.speed.signum() != target.signum() || target.abs() < unit.speed.abs() { ty.brake } else { ty.acceleration };
        unit.speed = unit.speed.approach(target, acceleration);
        // Straight on, else sliding along what it met (its x alone, its z alone).
        let step = heading_vector(unit.heading).scale(unit.speed);
        let candidates = [unit.pos + step, unit.pos + FxVec3::new(step.x, Fx::ZERO, Fx::ZERO), unit.pos + FxVec3::new(Fx::ZERO, Fx::ZERO, step.z)];
        let ignore = unit.factory.map(|factory| factory.slot);
        let current_ok = world.passable(&spec, World::pos_to_anchor(unit.pos, ty.footprint), ignore);
        if let Some(position) = candidates.into_iter().find(|&point| world.ground_segment_open(&spec, unit.pos, point, ignore, !current_ok)) {
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
}
