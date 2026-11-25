//! TA Reborn's stun (a weapon's `paralyzetime`: Supreme Commander's EMP shots and Chrono Dampener):
//! the enemies its shot hits are left as TA's paralyzers leave theirs, unable to move, fire, build or
//! run their scripts (`Unit::paralyzed_until`), for its time, beside the damage it deals. It reaches
//! the unit the shot hits and those round where it goes off, as far as its `paralyzeradius` (else as
//! far as its blast), or round the unit firing it as it fires (`paralyzeonfire`), never its own side
//! or its allies. Aircraft, `ImmuneToParalyzer` units and those of a category its `paralyzeexempt`
//! names shrug it off; a stun already longer is not cut short, and none goes on past a minute.

use super::*;
use crate::defs::Stun;

/// The longest a stun lasts (ticks): a minute, as TA's paralyzers.
const LONGEST: u64 = 60 * TICKS_PER_SECOND as u64;

impl Sim {
    /// The stun of weapon `w` going off at `pos` (its shot's blast, or round its unit as it fires):
    /// `source`'s, of `player`, `direct` the unit the shot hit.
    pub(crate) fn stun_round(&mut self, w: &WeaponType, pos: FxVec3, source: UnitId, player: Option<u8>, direct: Option<UnitId>) {
        let (Some(stun), Some(player)) = (w.stun, player) else { return };
        let r = stun.radius.unwrap_or(w.aoe_radius);
        let mut near = std::mem::take(&mut self.scratch);
        self.grid.query(pos, r + self.types.collision_reach, &mut near);
        let mut hit: Vec<UnitId> = Vec::new();
        for &slot in &near {
            let Some(u) = self.units.get(slot as usize).and_then(|u| u.as_ref()) else { continue };
            if u.dead || u.carried_by.is_some() || !self.is_enemy(player, u.owner) || !w.water && self.submerged(u) {
                continue;
            }
            if Some(u.id) != direct {
                // (As a blast reaches: to the edge of what it meets.)
                let (c, ur) = hit_sphere(u, self);
                if (c.distance(pos) - ur).max(Fx::ZERO) >= r {
                    continue;
                }
            }
            hit.push(u.id);
        }
        self.scratch = near;
        for id in hit {
            self.stun_unit(id, stun, source, player);
        }
    }

    /// A unit stunned for `stun`'s time (from `source`, `player`'s), unless it shrugs it off.
    fn stun_unit(&mut self, id: UnitId, stun: Stun, source: UnitId, player: u8) {
        let tick = self.tick;
        let Some(u) = self.unit(id) else { return };
        let t = self.types.get(u.type_id);
        if u.dead || t.def.immune_to_paralyzer || t.locomotion == Locomotion::Air || t.category_mask.intersects(&stun.exempt) || self.ceasefire_spares_side(player, u.owner) {
            return;
        }
        let by_a_unit = self.unit(source).is_some();
        if let Some(u) = self.unit_mut(id) {
            u.paralyzed_until = u.paralyzed_until.max((tick + stun.ticks).min(tick + LONGEST));
            if by_a_unit {
                u.last_attacker = Some(source);
                u.last_hit_tick = tick;
            }
        }
    }
}
