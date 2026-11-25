//! Weapon toggles (a unit type's `[TOGGLE]`, `Command::SetToggle`: Supreme Commander's sniper bot's
//! long gun, the Cybran anti-air's and cruisers' ground guns): every couple of seconds each of a
//! computer player's units that has one takes whichever of its two sets of weapons does more to the
//! enemies it sees within their reach (their damage a second, for each enemy a weapon can hit there:
//! in flight or not, under the water or not, past its minimum range), switching only for a fifth more
//! and keeping what it has with nothing about. TA's own units have none: nothing here happens in its
//! games.

use ta_sim::{Command, Fx, Locomotion, Sim, Unit};

use super::{SECOND, SkirmishAi, Survey, THINK_TICKS};

impl SkirmishAi {
    /// This think's toggles, where they are due (every two seconds).
    pub(crate) fn run_toggles(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        if s.now % (2 * SECOND) >= THINK_TICKS {
            return;
        }
        let (mut on, mut off) = (Vec::new(), Vec::new());
        for &u in s.own.iter().filter(|u| sim.can_toggle(u)) {
            let t = sim.unit_type(u);
            let Some(toggle) = t.abilities.toggle.as_ref() else { continue };
            let now = *u.weapon_slots(t);
            // The other set: the toggle's over what it has now, or back to the type's own where the
            // toggle changes a slot.
            let mut other = now;
            if u.toggled {
                for (slot, w) in toggle.weapons.iter().enumerate() {
                    if w.is_some() {
                        other[slot] = t.weapons[slot];
                    }
                }
            } else {
                toggle.apply(&mut other);
            }
            let (here, there) = (self.toggle_worth(sim, s, u, &now), self.toggle_worth(sim, s, u, &other));
            if there > here + here / 5 {
                if u.toggled { off.push(u.id) } else { on.push(u.id) }
            }
        }
        for (units, on) in [(on, true), (off, false)] {
            if !units.is_empty() {
                out.push(Command::SetToggle { player: self.player, units, on });
            }
        }
    }

    /// What weapons `slots` of `u` would do a second to the enemies it sees within their reach.
    fn toggle_worth(&self, sim: &Sim, s: &Survey, u: &Unit, slots: &[Option<usize>]) -> i64 {
        let mut worth = 0i64;
        for w in slots.iter().flatten().map(|&w| sim.weapon_types.get(w)) {
            if w.command_fire || w.def.stockpile || w.def.interceptor || w.default_damage <= Fx::ZERO {
                continue;
            }
            let per_second = w.default_damage.to_int().max(0) * w.burst.max(1) as i64 * 30 / w.reload_ticks.max(1) as i64;
            for e in &s.enemies {
                let et = sim.unit_type(e);
                let flying = et.locomotion == Locomotion::Air && !e.landed;
                let hits = if flying { w.hits_air } else { w.hits_ground } && (w.water || !sim.submerged(e));
                let d = u.pos.distance_xz(e.pos);
                if hits && d <= w.range + et.radius && d >= w.min_range {
                    worth = worth.saturating_add(per_second);
                }
            }
        }
        worth
    }
}
