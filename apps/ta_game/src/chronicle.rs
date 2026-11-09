//! The battle's chronicle, for its result: which unit destroyed which enemy and when, the first
//! blood, the Commanders that fell and the nuclear missiles launched, noted as the battle is
//! played (or caught up, or watched) beside its highlights' tally. It is kept beside the battle,
//! never part of it: nothing in the simulation reads it.

use glam::Vec3;
use ta_sim::{Sim, SimEvent, UnitId};

/// An enemy unit destroyed: when, and by which unit (its owner and type then).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Kill {
    tick: u64,
    by: UnitId,
    owner: u8,
    type_id: usize,
}

/// A Commander destroyed, or a nuclear missile launched: when, whose, and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub tick: u64,
    pub player: u8,
    pub pos: Vec3,
}

/// The unit of a player's that destroyed the most enemy units: its type, how many, and whether
/// it is still standing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Best {
    pub unit: UnitId,
    pub type_id: usize,
    pub kills: u32,
    pub alive: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Chronicle {
    kills: Vec<Kill>,
    /// When the first unit fell to an enemy's.
    pub first_blood: Option<u64>,
    pub commanders: Vec<Mark>,
    pub launches: Vec<Mark>,
}

impl Chronicle {
    /// Notes a simulation event (as it happens: the units it names are still in `sim`).
    pub fn note(&mut self, e: &SimEvent, sim: &Sim) {
        match e {
            SimEvent::UnitDestroyed { type_id, owner, pos, killer, .. } => {
                let t = sim.types.get(*type_id);
                if t.def.is_feature {
                    return;
                }
                if t.def.is_commander {
                    self.commanders.push(Mark { tick: sim.tick, player: *owner, pos: Vec3::from_array(pos.to_f32()) });
                }
                // (A killer destroyed in the same blow is not known by then.)
                if let Some(k) = killer.and_then(|k| sim.unit(k))
                    && sim.is_enemy(k.owner, *owner)
                {
                    self.first_blood.get_or_insert(sim.tick);
                    self.kills.push(Kill { tick: sim.tick, by: k.id, owner: k.owner, type_id: k.type_id });
                }
            }
            SimEvent::WeaponFired { unit, weapon, .. } => {
                let w = sim.weapon_types.get(*weapon);
                if w.def.stockpile
                    && !w.def.interceptor
                    && let Some(u) = sim.unit(*unit)
                {
                    self.launches.push(Mark { tick: sim.tick, player: u.owner, pos: Vec3::from_array(u.pos.to_f32()) });
                }
            }
            _ => {}
        }
    }

    /// `player`'s unit that destroyed the most enemy units (of two as good, the one that scored
    /// first); none when none of theirs destroyed any.
    pub fn best(&self, player: u8, sim: &Sim) -> Option<Best> {
        let (unit, type_id, kills) = self.leader(player)?;
        Some(Best { unit, type_id, kills, alive: sim.unit(unit).is_some_and(|u| !u.dead) })
    }

    /// [`Chronicle::best`]'s unit, its type and its kills.
    fn leader(&self, player: u8) -> Option<(UnitId, usize, u32)> {
        // (Each unit's type, kills and first kill.)
        let mut by_unit: std::collections::BTreeMap<UnitId, (usize, u32, u64)> = Default::default();
        for k in self.kills.iter().filter(|k| k.owner == player) {
            by_unit.entry(k.by).or_insert((k.type_id, 0, k.tick)).1 += 1;
        }
        let (unit, (type_id, kills, _)) = by_unit.into_iter().max_by(|a, b| a.1.1.cmp(&b.1.1).then(b.1.2.cmp(&a.1.2)))?;
        Some((unit, type_id, kills))
    }

    /// The battle taken back to `tick` (a replay rewound): what came after it is forgotten.
    pub fn forget_after(&mut self, tick: u64) {
        self.kills.retain(|k| k.tick <= tick);
        self.commanders.retain(|m| m.tick <= tick);
        self.launches.retain(|m| m.tick <= tick);
        self.first_blood = self.first_blood.filter(|&t| t <= tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kill(tick: u64, slot: u32, owner: u8) -> Kill {
        Kill { tick, by: UnitId { slot, generation: 0 }, owner, type_id: slot as usize }
    }

    #[test]
    fn the_best_unit_destroyed_the_most_and_first() {
        let c = Chronicle { kills: vec![kill(5, 2, 0), kill(10, 1, 0), kill(12, 3, 1), kill(13, 3, 1), kill(14, 3, 1), kill(20, 2, 0), kill(30, 1, 0)], ..Default::default() };
        // Two each: the one that scored first.
        assert_eq!(c.leader(0), Some((UnitId { slot: 2, generation: 0 }, 2, 2)));
        assert_eq!(c.leader(1).map(|l| l.2), Some(3));
        assert_eq!(c.leader(2), None);
    }

    #[test]
    fn forgetting_takes_back_what_came_after() {
        let mut c = Chronicle { kills: vec![kill(10, 1, 0), kill(50, 2, 0)], first_blood: Some(10), ..Default::default() };
        c.commanders.push(Mark { tick: 40, player: 1, pos: Vec3::ZERO });
        c.launches.push(Mark { tick: 5, player: 0, pos: Vec3::ZERO });
        c.forget_after(30);
        assert_eq!((c.kills.len(), c.commanders.len(), c.launches.len(), c.first_blood), (1, 0, 1, Some(10)));
        c.forget_after(5);
        assert_eq!((c.kills.len(), c.first_blood), (0, None));
    }
}
