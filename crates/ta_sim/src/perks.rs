//! A Conquest Commander's perks: standing advantages for its player, set as the battle is made
//! ([`Sim::set_perks`]) and kept to its end. Each is applied where it changes the rules: build
//! power, damage dealt and taken, experience from kills, the D-gun's energy.

use crate::defs::WeaponType;
use crate::sim::Sim;
use crate::{Fx, UnitId};

/// What a player's Commander brings to a battle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Perks {
    /// The Commander builds and repairs half as fast again.
    pub engineer: bool,
    /// The Commander takes 30% less damage.
    pub armour: bool,
    /// The Commander's weapons hit a quarter harder.
    pub marksman: bool,
    /// The player's units gain experience from their kills half as fast again.
    pub instructor: bool,
    /// The Commander's D-gun costs half its energy.
    pub capacitors: bool,
}

impl Sim {
    /// Gives `player` its Commander's perks (as a battle is made).
    pub fn set_perks(&mut self, player: u8, perks: Perks) {
        if let Some(p) = self.players.get_mut(player as usize) {
            p.perks = perks;
        }
    }

    /// The perks of `unit`'s player, if `unit` is a Commander.
    fn commander_perks(&self, unit: UnitId) -> Option<Perks> {
        let u = self.unit(unit)?;
        self.types.get(u.type_id).def.is_commander.then(|| self.players.get(u.owner as usize).map(|p| p.perks)).flatten()
    }

    /// `damage` from `source` to `victim`, as their Commanders' perks change it.
    pub(crate) fn perk_damage(&self, damage: Fx, source: UnitId, victim: UnitId) -> Fx {
        let mut damage = damage;
        if source != victim && self.commander_perks(source).is_some_and(|p| p.marksman) {
            damage = damage * 5 / 4;
        }
        if self.commander_perks(victim).is_some_and(|p| p.armour) {
            damage = damage * 7 / 10;
        }
        damage
    }

    /// The work `builder` does a tick at `rate`: an engineer Commander's half as much again.
    pub(crate) fn perk_work_rate(&self, builder: UnitId, rate: u64) -> u64 {
        if self.commander_perks(builder).is_some_and(|p| p.engineer) { rate + rate / 2 } else { rate }
    }

    /// The experience `xp` a kill brings `killer`: half as much again with its player's
    /// instructor.
    pub(crate) fn perk_experience(&self, killer: UnitId, xp: Fx) -> Fx {
        let instructed = self.unit(killer).and_then(|k| self.players.get(k.owner as usize)).is_some_and(|p| p.perks.instructor);
        if instructed { xp * 3 / 2 } else { xp }
    }

    /// The energy a shot of `w` costs `shooter`: half a D-gun's with capacitors.
    pub fn shot_energy(&self, shooter: UnitId, w: &WeaponType) -> Fx {
        if w.command_fire && self.commander_perks(shooter).is_some_and(|p| p.capacitors) { w.energy_per_shot / 2 } else { w.energy_per_shot }
    }
}
