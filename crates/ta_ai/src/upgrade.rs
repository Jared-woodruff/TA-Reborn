//! Upgrades in place (`Order::Upgrade`): the computer players climb the tiers by upgrading a
//! factory into the next (Supreme Commander's factories, which nothing builds outright), climb the
//! others to the top tier as the income grows and more when metal piles up, and upgrade their
//! extractors where it pays, one or a few at a time as what is left to pay on those under way
//! allows. TA's own units upgrade into nothing: nothing here happens in its games.

use std::cmp::Reverse;

use ta_sim::{Command, Fx, Locomotion, Order, Sim, Unit};

use super::info::Role;
use super::{SkirmishAi, Survey, dist, fx, ordered_units};

/// Whether a unit is upgrading (or told to).
fn upgrading(u: &Unit) -> bool {
    matches!(u.orders.front(), Some(Order::Upgrade(_)))
}

/// The metal still to pay on a unit's upgrade under way (all of it, told and not yet begun).
fn metal_left(sim: &Sim, u: &Unit) -> i64 {
    let Some(Order::Upgrade(to)) = u.orders.front() else { return 0 };
    let done = sim.self_work_progress(u).map_or(Fx::ZERO, |(_, p)| p);
    (sim.types.get(*to).cost_metal * (Fx::ONE - done.clamp(Fx::ZERO, Fx::ONE))).to_int()
}

/// Seconds of our metal income the factories' upgrades under way may have left to pay for another
/// to start beside them; the extractors', likewise. (A climb to the Supreme Commander factions'
/// third tier, or a tech 3 extractor of theirs, costs minutes of a middling income: three of those
/// at once starve everything else.)
const FACTORY_UPGRADES_LEFT: i64 = 30;
const EXTRACTOR_UPGRADES_LEFT: i64 = 45;

impl SkirmishAi {
    /// Whether one of our factories can be upgraded into a factory of a higher tier than any we
    /// have: the tiers are climbed so, not by building the next tier's factory (see
    /// `builder_task`).
    pub(crate) fn climbs_by_upgrading(&self, sim: &Sim, s: &Survey) -> bool {
        s.factories.iter().any(|f| sim.next_upgrade(f).is_some_and(|to| self.factory_tier(sim, to) > s.factory_tier))
    }

    /// This think's upgrades: a factory up a tier, more factories when rich, extractors where it
    /// pays. Each given to a unit not told anything else this think.
    pub(crate) fn run_upgrades_in_place(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        // (TA's own units upgrade into nothing: done at once, as before.)
        if !s.own.iter().any(|u| sim.types.get(u.type_id).upgrades_to.is_some()) {
            return;
        }
        let me = self.player;
        let ordered = ordered_units(out);
        let free = |u: &&&Unit| Self::can_command(sim, u) && !upgrading(u) && !ordered.contains(&u.id);
        // Factories: the tier above our highest first, alone, on the economy the first advanced
        // factory waits for; then more beside it while what the ones under way have left to pay is
        // within `FACTORY_UPGRADES_LEFT` of our income (three at most), and, metal piling up, our
        // lowest.
        let tech_economy = s.minutes >= self.p.tech_minute && (s.m_inc >= fx(self.p.tech_income) || s.metal >= fx(1200));
        let factories_upgrading = s.factories.iter().filter(|f| upgrading(f)).count();
        let factories_left: i64 = s.factories.iter().map(|f| metal_left(sim, f)).sum();
        let room = factories_upgrading == 0 || factories_upgrading < 3 && factories_left <= s.m_inc.to_int() * FACTORY_UPGRADES_LEFT;
        if tech_economy && room {
            let candidates: Vec<(&Unit, usize)> = s.factories.iter().filter(free).filter_map(|f| sim.next_upgrade(f).map(|to| (*f, to))).filter(|&(f, to)| self.factory_tier(sim, to) > self.factory_tier(sim, f.type_id)).collect();
            // (Past the first advanced factory, as a constructor of our top tier would have put up
            // the next tier's factory: one of those first.)
            let top_builder = s.factory_tier < 2 || s.builders.iter().any(|b| {
                let i = &self.info[b.type_id];
                !i.commander && (i.builds_mex || i.builds_energy) && i.tech >= s.factory_tier
            });
            // (A step to a tier none of ours has yet, alone: the second upgrade at once is another
            // factory's climb to where we are.)
            let alone = factories_upgrading == 0;
            let climbing: Vec<(&Unit, usize)> = candidates.iter().copied().filter(|&(_, to)| alone && top_builder && self.factory_tier(sim, to) > s.factory_tier).collect();
            let rich = self.metal_avg > Fx::from_ratio(6, 10) && s.m_inc >= fx(self.p.tech_income * 2) && s.minutes >= self.p.tech_minute + 4;
            // More advanced factories as the economy grows (`adv_factories_wanted`, as they are
            // built where nothing upgrades), and toward experimentals one that upgrades into a
            // factory leading there.
            let more = s.adv_factory_done && s.adv_factories < self.adv_factories_wanted(s);
            let leading: Vec<(&Unit, usize)> = if alone && self.toward_experimentals(sim, s) { candidates.iter().copied().filter(|&(_, to)| self.leads_to_experimentals(sim, to)).collect() } else { Vec::new() };
            // Once a ground factory of a tier above others of ours stands (the second, or the
            // third), those below climb to it as the economy grows (`top_factories_wanted` of them at
            // the top tier, those upgrading into it counted), the nearest the top first, while power
            // holds: the army leans to the top tier as its factories climb, and what they made of the
            // lower tiers, the top tier's make too (Forged Alliance's factories climb so, as its
            // players' do; TA's own upgrade into nothing, and are as they were).
            let ground = |f: &Unit| matches!(self.info[f.type_id].kind, Locomotion::Ground | Locomotion::Hover);
            let top_tier = self.top_ground_tier(sim, s);
            let at_top = s
                .factories
                .iter()
                .filter(|f| ground(f) && (self.factory_tier(sim, f.type_id) >= top_tier || matches!(f.orders.front(), Some(Order::Upgrade(to)) if self.factory_tier(sim, *to) >= top_tier)))
                .count();
            let power = self.energy_avg >= Fx::from_ratio(1, 10) || s.e_inc > s.e_exp;
            let toward_top: Vec<(&Unit, usize)> = if top_tier >= 2 && power && at_top < self.top_factories_wanted(s) {
                candidates.iter().copied().filter(|&(f, to)| ground(f) && self.factory_tier(sim, to) <= top_tier).collect()
            } else {
                Vec::new()
            };
            let (pool, highest_first) = if !climbing.is_empty() {
                (climbing, false)
            } else if !leading.is_empty() {
                (leading, false)
            } else if !toward_top.is_empty() {
                (toward_top, true)
            } else if alone && (rich || more) {
                (candidates, false)
            } else {
                (Vec::new(), false)
            };
            // Land first (it reaches the enemy), then the lowest tier (or, toward the top, the
            // highest), the nearest home.
            let pick = pool.into_iter().min_by_key(|&(f, to)| {
                let i = &self.info[f.type_id];
                let tier = self.factory_tier(sim, f.type_id) as i32;
                (i.kind == Locomotion::Air, i.kind == Locomotion::Naval, if highest_first { -tier } else { tier }, self.info[to].cost, dist(f.pos, self.base), f.id.slot)
            });
            if let Some((f, to)) = pick {
                log::debug!("{} min: P{me} upgrades its {} into {}", s.minutes, sim.unit_type(f).name, sim.types.get(to).name);
                out.push(Command::Order { player: me, units: vec![f.id], order: Order::Upgrade(to), queue: false });
            }
        }
        // Extractors: richer ones in place of ours, a few at a time as the income allows (and what
        // those under way have left to pay is within `EXTRACTOR_UPGRADES_LEFT` of it), the safe ones
        // nearest home first; not while power is short (an upgrade asks for a lot).
        let extracting = |t: usize| sim.types.get(t).extracts_metal;
        let mex: Vec<&&Unit> = s.own.iter().filter(|u| self.info[u.type_id].role == Role::Extractor && upgrading(u)).collect();
        let mex_upgrading = mex.len();
        let mex_left: i64 = mex.iter().map(|u| metal_left(sim, u)).sum();
        let room = (1 + s.m_inc.to_int() / 15).clamp(1, 4) as usize;
        let paid_for = mex_upgrading == 0 || mex_left <= s.m_inc.to_int() * EXTRACTOR_UPGRADES_LEFT;
        let power = self.energy_avg >= Fx::from_ratio(3, 10) && s.e_inc > s.e_exp;
        let metal = s.metal >= fx(300) || self.metal_avg >= Fx::from_ratio(3, 10);
        if s.minutes >= self.p.mex_upgrade_minute && mex_upgrading < room && paid_for && power && metal {
            let pick = s
                .own
                .iter()
                .filter(free)
                .filter(|u| self.info[u.type_id].role == Role::Extractor)
                .filter_map(|u| sim.next_upgrade(u).filter(|&to| extracting(to) > extracting(u.type_id)).map(|to| (*u, to)))
                .filter(|&(u, _)| self.threat.at(u.pos) == 0)
                .min_by_key(|&(u, to)| (dist(u.pos, self.base) / 400, self.info[to].cost, Reverse(u.id.slot)));
            if let Some((u, to)) = pick {
                log::debug!("{} min: P{me} upgrades an extractor into {}", s.minutes, sim.types.get(to).name);
                out.push(Command::Order { player: me, units: vec![u.id], order: Order::Upgrade(to), queue: false });
            }
        }
    }
}
