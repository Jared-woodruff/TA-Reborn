//! Factories that move (a unit type's `MobileFactory`: Supreme Commander's Fatboy, carriers,
//! Atlantis, Tempest and CZAR). They fight with the army (`SkirmishAi::survey`), and every few
//! seconds each finished one of a computer player's with nothing in its queue is given the next
//! unit to make: of the combat and anti-aircraft units of its menu, the one worth the most for its
//! price that what is stored and a minute's income can pay for (`Command::Produce`). What it makes
//! comes off it and joins the army as any factory's units do. TA's own units have none: nothing
//! here happens in its games.

use ta_sim::{Command, Locomotion, Sim};

use super::{Role, SECOND, SkirmishAi, Survey, THINK_TICKS};

/// How often (ticks) the factories that move are looked at.
const EVERY: u64 = 3 * SECOND;

impl SkirmishAi {
    /// This think's production on the factories that move, where it is due.
    pub(crate) fn run_mobile_factories(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        if s.now % EVERY >= THINK_TICKS || s.peace {
            return;
        }
        // (What is stored and a minute's income: what a unit made may take.)
        let (metal, energy) = (s.metal + s.m_inc * 60, s.energy + s.e_inc * 60);
        for &u in s.own.iter().filter(|u| self.info[u.type_id].mobile_factory && u.is_complete() && !u.dead) {
            if !u.factory_queue.is_empty() || u.build.is_some() || u.carried_by.is_some() {
                continue;
            }
            let t = sim.unit_type(u);
            let best = t
                .ai_build_options
                .iter()
                .copied()
                .filter(|&o| sim.is_allowed(o) && sim.types.get(o).locomotion != Locomotion::Static)
                .filter(|&o| matches!(self.info[o].role, Role::Combat | Role::AntiAirUnit))
                .filter(|&o| {
                    let ot = sim.types.get(o);
                    ot.cost_metal <= metal && ot.cost_energy <= energy
                })
                .max_by_key(|&o| ((self.info[o].worth as i128 * 1000) / self.info[o].cost.max(1) as i128, std::cmp::Reverse(o)));
            if let Some(type_id) = best {
                out.push(Command::Produce { player: self.player, factory: u.id, type_id, count: 1 });
            }
        }
    }
}
