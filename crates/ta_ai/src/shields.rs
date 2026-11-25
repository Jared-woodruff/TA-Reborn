//! Shields (TA Reborn's: `ta_sim`'s `sim/shield.rs`; Supreme Commander's shield generators): the
//! computer players count an enemy's domes in what guards a place (an attack's target under one is
//! a harder one; their tactical missiles, which burst on a dome, look elsewhere, and their long-range
//! guns rather do), a personal shield in a unit's toughness (`info.rs`), and put shield generators
//! up by their factories and defences, as Forged Alliance's do, once their economy carries the
//! upkeep: one at a time, by the most valuable of them that no dome of theirs covers. TA's own units
//! have none: nothing here happens in its games.

use super::*;

/// From which minute a computer player puts up shield generators, the metal income (a second) it
/// wants for them, and the most of them for each so many minutes of the battle (and one).
const SHIELD_MINUTE: u64 = 8;
const SHIELD_INCOME: i64 = 10;
const SHIELD_MINUTES_EACH: u64 = 5;

impl SkirmishAi {
    /// What the enemy's domes we know of add to the guard of a place: each over it, its generator's
    /// cost twice (an army's shells and missiles burst on it, the generator under it).
    pub(crate) fn dome_guard(&self, pos: FxVec3) -> i64 {
        self.enemy_buildings.iter().filter(|(_, p, t)| self.info[*t].dome > 0 && p.within_xz(pos, fx(self.info[*t].dome))).map(|(_, _, t)| self.info[*t].cost * 2).sum()
    }

    /// Whether a place is under a dome of the enemy's we know of (finished: a frame has none up).
    pub(crate) fn under_enemy_dome(&self, pos: FxVec3) -> bool {
        self.enemy_buildings.iter().any(|(id, p, t)| self.info[*t].dome > 0 && !self.enemy_frames.contains(id) && p.within_xz(pos, fx(self.info[*t].dome * 9 / 10)))
    }

    /// A shield generator by what of ours most wants one, if one is due: from [`SHIELD_MINUTE`] on
    /// an income of [`SHIELD_INCOME`] metal a second and the power to keep it up (its upkeep a third
    /// of our energy income at most), one going up at a time and no more than one for every
    /// [`SHIELD_MINUTES_EACH`] minutes of the battle; the widest dome the builder can put up within
    /// two minutes of our metal income, by the most valuable of our factories and defences under no
    /// dome of ours (finished or going up), near enough that its dome takes it in (its middle well
    /// under the dome: the blasts outside spare it), or by the next of them where there is no room.
    pub(crate) fn shield_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        if s.minutes < SHIELD_MINUTE || s.m_inc < fx(SHIELD_INCOME) || plan.shields > 0 {
            return None;
        }
        let generators = |t: usize| self.info[t].dome > 0 && self.info[t].kind == Locomotion::Static && self.info[t].ground_dps == 0;
        // (Ours, finished or going up; one going up at a time.)
        let ours: Vec<(FxVec3, i64)> = s.own.iter().filter(|u| generators(u.type_id)).map(|u| (u.pos, self.info[u.type_id].dome)).collect();
        if s.nanoframes.iter().any(|n| generators(n.type_id)) || ours.len() > (s.minutes / SHIELD_MINUTES_EACH) as usize {
            return None;
        }
        let budget = s.m_inc.to_int() * 60 * 2;
        let power = s.e_inc.to_int();
        let pick = opts
            .iter()
            .copied()
            .filter(|&o| generators(o) && self.info[o].land && self.info[o].cost <= budget && self.info[o].shield_upkeep * 3 <= power)
            .max_by_key(|&o| (self.info[o].dome, self.info[o].shield, Reverse(o)))?;
        let dome = self.info[pick].dome;
        // (A dome of `r` takes in a building whose middle stands within seven eighths of it: what
        // its enemies aim at well under it, its blasts bursting on the dome.)
        let takes_in = |r: i64| r * 7 / 8;
        let covered = |u: &Unit| ours.iter().any(|&(c, r)| c.within_xz(u.pos, fx(takes_in(r))));
        // The most valuable of our factories and defences no dome of ours covers, nearest home
        // of equals; failing a site by it (a crowded base), the next (a few at most).
        let wants = |r: Role| matches!(r, Role::Factory | Role::Defense | Role::AntiAir | Role::Silo | Role::AntiNuke);
        let mut targets: Vec<&Unit> = s.own.iter().copied().filter(|u| u.is_complete() && wants(self.info[u.type_id].role) && !covered(u) && self.threat.at(u.pos) == 0).collect();
        if targets.is_empty() || self.place_failed.iter().any(|&(f, until)| f == pick && sim.tick < until) {
            return None;
        }
        targets.sort_by_key(|u| (Reverse(self.info[u.type_id].cost), dist(u.pos, self.base), u.id.slot));
        // (Beside it on the side of home if it can, near enough for its dome to take it in, and a
        // little nearer: the search reaches round to its far side.)
        let reach = takes_in(dome) - 16;
        let cells = (reach * 5 / 3 / 16 + 2).clamp(4, 24) as i32;
        let r = fx(sim.types.get(pick).radius.to_int() + 16);
        for target in targets.into_iter().take(3) {
            let back = (self.base - target.pos).xz();
            let near = if back.length_xz() > fx(1) { target.pos + back.normalize().scale(fx(reach * 2 / 3)) } else { target.pos };
            let ok = |p: FxVec3| p.within_xz(target.pos, fx(reach)) && plan.free(p, r) && self.threat.at(p) == 0 && self.builder_reaches(sim, b, p, pick);
            if let Some(site) = self.find_site(sim, pick, near, cells, &ok) {
                log::debug!("{} min: P{} puts up a shield generator, {}, by its {}", s.minutes, self.player, sim.types.get(pick).name, sim.types.get(target.type_id).name);
                return Some(Task::Build(pick, site));
            }
        }
        self.place_failed.push((pick, sim.tick + 30 * SECOND));
        None
    }
}
