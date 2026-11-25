//! Regeneration auras (a unit type's `RegenAuraRadius` with its `RegenAuraRate` or
//! `RegenAuraShare`, or an enhancement's: Supreme Commander's Seraphim commander's Restoration
//! Field). Once a second ([`AURA_TICKS`]) each unit with an aura that is at work (`Sim::working`:
//! finished, switched on or not switchable, its upkeep paid, not paralysed) mends its side's other
//! finished units round it, those whose middle is within its radius and their own: each by the
//! aura's rate, or by its share of the unit's most health (up to the rate, where both are given), to
//! its most at the most. A unit within several auras takes the strongest alone (Forged Alliance's
//! aura buffs replace each other). At no cost; nothing of it runs in a game whose units have no
//! aura (`UnitTypes::auras`: TA's own).

use crate::fixed::Fx;
use crate::unit::UnitId;

use super::Sim;

/// How often the auras mend (ticks): once a second.
pub const AURA_TICKS: u64 = 30;

impl Sim {
    /// The auras' mending for the second, where it is due.
    pub(super) fn mend_round(&mut self) {
        if !self.types.auras || !self.tick.is_multiple_of(AURA_TICKS) {
            return;
        }
        let types = self.types.clone();
        let givers: Vec<_> = self
            .units
            .iter()
            .flatten()
            .filter(|u| u.carried_by.is_none() && self.working(u))
            .filter_map(|u| u.abilities(types.get(u.type_id)).aura.map(|a| (u.id, u.owner, u.pos, a)))
            .collect();
        if givers.is_empty() {
            return;
        }
        // Each patient's strongest mending, by slot.
        let mut best: Vec<Option<(UnitId, Fx)>> = vec![None; self.units.len()];
        let mut near = std::mem::take(&mut self.scratch);
        for (giver, owner, pos, aura) in givers {
            self.grid.query(pos, aura.radius + types.target_reach, &mut near);
            for &slot in &near {
                let Some(o) = self.units.get(slot as usize).and_then(|o| o.as_ref()) else { continue };
                if o.id == giver || o.dead || !o.is_complete() || o.carried_by.is_some() || !self.allied(owner, o.owner) {
                    continue;
                }
                let ot = types.get(o.type_id);
                let most = o.max_health(ot);
                if o.health <= Fx::ZERO || o.health >= most || !o.pos.within_xz(pos, aura.radius + ot.radius) {
                    continue;
                }
                let heal = aura.mends(most);
                if let Some(b) = best.get_mut(slot as usize)
                    && b.is_none_or(|(_, h)| heal > h)
                {
                    *b = Some((o.id, heal));
                }
            }
        }
        self.scratch = near;
        for (id, heal) in best.into_iter().flatten() {
            let Some(u) = self.unit_mut(id) else { continue };
            let most = u.max_health(types.get(u.type_id));
            u.health = (u.health + heal).min(most);
        }
    }
}
