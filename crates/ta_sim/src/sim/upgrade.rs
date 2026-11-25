//! Upgrading in place (Supreme Commander's `UpgradesTo`; the unit file's `UpgradesTo`): a unit
//! given [`Order::Upgrade`] builds the type it upgrades into on itself, with its own build power
//! (`WorkerTime`) and that of the builders helping it (guarding it, or told to repair it), paying
//! that type's metal and energy over its build time as construction does, slowed by the economy as
//! construction is. Done, it is that type: a unit of it put where it stood, facing as it faced, its
//! owner's, finished, with the old one's share of its health, its orders after the upgrade, a
//! factory's queue, route, repeat and the unit on its pad; everything else a fresh unit of the type
//! has. The old one is taken off the map (no wreck, blast or loss), and [`SimEvent::UnitUpgraded`]
//! tells the interface, which carries its selection and control groups over.
//!
//! While it upgrades a factory makes nothing (the unit on its pad waits, and its queue), as
//! Forged Alliance's do; anything else goes on with its work: an extractor extracts, a generator
//! makes power, a radar watches. Called off (Stop, or any order not queued after it), the metal
//! paid for the upgrade comes back, as a frame given up gives its metal back, and its energy is
//! lost.

use ta_data::AdjacencyAffect;

use crate::events::SimEvent;
use crate::fixed::Fx;
use crate::unit::{BuildJob, Order, SelfJob, SelfWork, Unit, UnitId};
use crate::world::{Occupant, World};

use super::Sim;

impl Sim {
    /// Whether a unit may be ordered to upgrade into `to`: the next step of its type's upgrades,
    /// or one further along them (queued after the steps before it), and allowed in this battle.
    pub fn can_upgrade_into(&self, u: &Unit, to: usize) -> bool {
        self.is_allowed(to) && self.types.upgrade_chain(u.type_id).contains(&to)
    }

    /// The type a unit's next upgrade makes of it (its type's `UpgradesTo`), if it has one this
    /// battle allows.
    pub fn next_upgrade(&self, u: &Unit) -> Option<usize> {
        self.types.get(u.type_id).upgrades_to.filter(|&to| self.is_allowed(to))
    }

    /// What a unit builds on itself, and how far along (0 to 1).
    pub fn self_work_progress(&self, u: &Unit) -> Option<(SelfJob, Fx)> {
        let w = u.self_work?;
        let (_, _, total) = self.self_job_price(u.type_id, w.job);
        Some((w.job, Fx::fraction(w.work.min(total), total.max(1))))
    }

    /// What a job of a unit of type `of` costs (metal, energy) and the work it takes.
    pub fn self_job_price(&self, of: usize, job: SelfJob) -> (Fx, Fx, u64) {
        match job {
            SelfJob::Upgrade(to) => {
                let t = self.types.get(to);
                (t.cost_metal, t.cost_energy, t.build_work)
            }
            SelfJob::Enhance(i) => match self.types.get(of).enhancements.get(i as usize) {
                Some(e) => (e.cost_metal, e.cost_energy, e.build_work),
                None => (Fx::ZERO, Fx::ZERO, 1),
            },
        }
    }

    /// A unit's upgrade order at the front of its orders: started (it stops where it is), then
    /// built on a tick at a time with its own build power. One its type no longer upgrades into
    /// (a step queued past the next), or that the battle no longer allows, fails.
    pub(super) fn update_upgrade_order(&mut self, id: UnitId, t: &crate::defs::UnitType, to: usize) {
        let Some(u) = self.unit(id) else { return };
        if t.upgrades_to != Some(to) || !self.is_allowed(to) || t.work_rate == 0 {
            let reason = if t.work_rate == 0 { "it has no build power" } else { "cannot upgrade into that" };
            self.events.push(SimEvent::OrderFailed { unit: id, reason });
            self.cancel_self_work(id, true);
            if let Some(u) = self.unit_mut(id) {
                u.orders.pop_front();
            }
            return;
        }
        // (No room for what it becomes, where that takes more ground than it stands on: another
        // building is in the way, put up beside it.)
        if !self.room_to_become(id, to) {
            self.events.push(SimEvent::OrderFailed { unit: id, reason: "no room to upgrade into that" });
            self.cancel_self_work(id, true);
            if let Some(u) = self.unit_mut(id) {
                u.orders.pop_front();
            }
            return;
        }
        if u.self_work.is_none_or(|w| w.job != SelfJob::Upgrade(to)) {
            // (Another job's metal back first: none is under way but this order's.)
            self.cancel_self_work(id, true);
            self.stop(id);
            if let Some(u) = self.unit_mut(id) {
                u.self_work = Some(SelfWork { job: SelfJob::Upgrade(to), work: 0, metal_saved: Fx::ZERO });
            }
        }
        let rate = self.mutator_work_rate(self.perk_work_rate(id, t.work_rate));
        let owner = self.unit(id).map_or(0, |u| u.owner);
        self.work_on_self(id, rate, owner, id);
    }

    /// `rate` of work put into what a unit builds on itself (its own, or a helper's), paid for by
    /// `payer` (the one whose work it is) as construction is (a share of the price for the share
    /// of the work, slowed by its economy's stall; less for a worker `by` beside a building that
    /// cuts what building costs it: adjacency); done, it takes effect.
    pub(crate) fn work_on_self(&mut self, id: UnitId, rate: u64, payer: u8, by: UnitId) {
        let Some(u) = self.unit(id).filter(|u| !u.dead) else { return };
        let Some(w) = u.self_work else { return };
        let owner = payer as usize;
        if owner >= self.players.len() {
            return;
        }
        let (metal, energy, total) = self.self_job_price(u.type_id, w.job);
        let (metal_factor, energy_factor) = self.unit(by).map_or((None, None), |b| (b.adjacency_factor(AdjacencyAffect::BuildMetal), b.adjacency_factor(AdjacencyAffect::BuildEnergy)));
        let (metal_price, energy_price) = (super::adjacency::scaled(metal, metal_factor), super::adjacency::scaled(energy, energy_factor));
        let done = w.work.min(total);
        let work = if self.players.get(owner).is_some_and(|p| p.cheats.instant_build) {
            // (The instant-build cheat: done at once, for nothing.)
            total - done
        } else {
            let full = rate.min(total - done);
            if full == 0 && done < total {
                return;
            }
            let eco = &mut self.players[owner].economy;
            let factor = eco.build_factor;
            eco.spend_work(done, total, full, metal_price, energy_price, factor)
        };
        let worth = |price: Fx| price.share(done + work, total) - price.share(done, total);
        let saved = if metal_factor.is_some() { worth(metal) - worth(metal_price) } else { Fx::ZERO };
        if let Some(u) = self.unit_mut(id) {
            u.self_work = Some(SelfWork { work: done + work, metal_saved: w.metal_saved + saved, ..w });
        }
        if done + work >= total {
            match w.job {
                SelfJob::Upgrade(to) => self.finish_upgrade(id, to),
                SelfJob::Enhance(index) => self.finish_enhancement(id, index),
            }
        }
    }

    /// Whether a unit of type `to` would stand where `id` stands with no other building in its way
    /// (`World::building_at`): where its footprint is no larger, always; where it is (the Cybran
    /// tech 1 land factory's tech 2, a row longer), what it takes besides must be free of others.
    pub fn room_to_become(&self, id: UnitId, to: usize) -> bool {
        let Some(u) = self.unit(id) else { return false };
        let (from, into) = (self.types.get(u.type_id), self.types.get(to));
        if u.occupied_origin.is_none() || into.def.takes_no_ground || from.footprint == into.footprint && World::pos_to_anchor(u.pos, from.footprint) == World::pos_to_anchor(u.pos, into.footprint) {
            return true;
        }
        let (x0, z0) = World::footprint_origin(World::pos_to_anchor(u.pos, into.footprint), into.footprint);
        (z0..z0 + into.footprint.1 as i32).all(|z| (x0..x0 + into.footprint.0 as i32).all(|x| self.world.building_at(x, z).is_none_or(|s| s == id.slot)))
    }

    /// Calls off what a unit builds on itself: with `refund`, the metal paid for it so far comes
    /// back (its energy is spent), as a frame given up gives its metal back.
    pub(crate) fn cancel_self_work(&mut self, id: UnitId, refund: bool) {
        let Some((w, of)) = self.unit_mut(id).and_then(|u| u.self_work.take().map(|w| (w, u.type_id))) else { return };
        if !refund {
            return;
        }
        let (metal, _, total) = self.self_job_price(of, w.job);
        // (What was paid for it: its share of the price, less what adjacency spared those at it.)
        let back = (metal.share(w.work.min(total), total.max(1)) - w.metal_saved).max(Fx::ZERO);
        if let Some(owner) = self.unit(id).map(|u| u.owner as usize)
            && let Some(p) = self.players.get_mut(owner)
        {
            let e = &mut p.economy;
            e.metal = (e.metal + back).min(e.metal_capacity.max(e.metal));
            e.tick_metal_in += back;
        }
    }

    /// The upgrade done: a unit of type `to` in the old one's place, carrying over what it keeps.
    fn finish_upgrade(&mut self, id: UnitId, to: usize) {
        let types = self.types.clone();
        let Some(old) = self.unit_mut(id) else { return };
        let ot = types.get(old.type_id);
        let (owner, pos, heading) = (old.owner, old.pos, old.heading);
        let share = if ot.max_health > Fx::ZERO { (old.health / ot.max_health).clamp(Fx::ZERO, Fx::ONE) } else { Fx::ONE };
        // What carries over: its orders after the upgrade, a factory's work, its standing orders,
        // its experience. The old one gives them up.
        let mut orders = std::mem::take(&mut old.orders);
        if orders.front() == Some(&Order::Upgrade(to)) {
            orders.pop_front();
        }
        let queue = std::mem::take(&mut old.factory_queue);
        let rally = std::mem::take(&mut old.rally);
        let yard = std::mem::take(&mut old.yard);
        let job = old.build.take();
        let (repeat, fire, movement, activated, kills, xp, guided) = (old.factory_repeat, old.fire_state, old.move_state, old.activated, old.kills, old.xp, old.radar_guided);
        // (What adjacency spared those who worked on the upgrade: the new one, taken apart by its
        // side, gives back what was paid for it, not its price.)
        let saved = old.self_work.map_or(Fx::ZERO, |w| w.metal_saved);
        // Taken off the map as given up: no Killed script, wreck, blast or loss. Its ground is the
        // new one's from now (its slot goes at the end of the tick).
        old.self_work = None;
        old.dead = true;
        old.death_processed = true;
        old.removed = true;
        let origin = old.occupied_origin.take();
        if let Some(origin) = origin {
            self.world.release(Occupant::Unit(id.slot), origin, ot.footprint);
        }
        let Some(new) = self.spawn_unit(to, owner, pos, heading, true) else { return };
        // (Its drones go to the unit it becomes: `drones.rs`.)
        self.carry_drones_over(id, new);
        let nt = types.get(to);
        // A factory's queue: what the new one can build (all of it, upgraded in Forged Alliance).
        // (The unit on its pad, first in the queue, whatever it is: it is finished.)
        let queue: std::collections::VecDeque<usize> = queue.into_iter().enumerate().filter(|&(i, q)| i == 0 && job.is_some() || nt.ai_build_options.contains(&q)).map(|(_, q)| q).collect();
        let products: Vec<UnitId> = yard.iter().copied().chain(job.map(|j| j.target)).collect();
        if let Some(u) = self.unit_mut(new) {
            u.health = (nt.max_health * share).max(Fx::ONE).min(nt.max_health);
            u.metal_saved = saved;
            u.orders = orders;
            u.fire_state = fire;
            u.move_state = movement;
            u.kills = kills;
            u.xp = xp;
            u.radar_guided = guided;
            if nt.is_factory {
                u.factory_queue = queue;
                u.rally = rally;
                u.factory_repeat = repeat;
                u.yard = yard;
                // (The unit on the old one's pad, waiting through the upgrade: the new one's to
                // finish.)
                u.build = job.map(|j| BuildJob { target: j.target, started: true });
            }
        }
        for p in products {
            if let Some(product) = self.unit_mut(p)
                && product.factory == Some(id)
            {
                product.factory = Some(new);
            }
        }
        // A switch the player set (a radar turned off) stays as it was.
        if nt.def.on_offable && !activated {
            self.set_active(new, false);
        }
        // A factory with a unit on its pad opens up and builds it on.
        if let Some(j) = job.filter(|_| nt.is_factory) {
            self.set_active(new, true);
            self.start_script(new, "StartBuilding", &[]);
            self.events.push(SimEvent::NanoSpray { builder: new, target: j.target });
        }
        if let Some(p) = self.players.get_mut(owner as usize) {
            p.stats.built += 1;
        }
        // Who was guarding or mending the old one goes on with the new one (its side's).
        let team = self.players.get(owner as usize).map_or(owner, |p| p.team);
        for u in self.units.iter_mut().flatten() {
            if !self.players.get(u.owner as usize).is_some_and(|p| p.team == team) {
                continue;
            }
            for o in u.orders.iter_mut() {
                match o {
                    Order::Guard(g) | Order::Repair(g) if *g == id => *g = new,
                    _ => {}
                }
            }
        }
        self.events.push(SimEvent::UnitUpgraded { from: id, to: new, type_id: to });
    }
}
