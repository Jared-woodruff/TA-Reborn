//! Drones (a unit type's `[DRONES]`, or an enhancement's: `ta_data::DroneDef`; Supreme Commander's
//! Kennel's engineering drone and its UEF commanders' pods). A unit keeps the drones its type and its
//! installed enhancements give it ([`Sim::wanted_drones`]: summed by the drone's type): made at once
//! as it is finished or the enhancement is installed (`Sim::make_drones`, for nothing), at the piece
//! its section names (an aircraft taking off from there), each linked to it ([`Unit::drone_of`],
//! [`Unit::drones`]). A drone's standing order is to guard its unit (`Order::Guard`: a builder helps
//! what it builds and mends it), given whenever it is idle; a builder drone also helps mend what its
//! unit mends within its leash (`Order::Repair`); sent farther from it than its leash, it comes back
//! to it. One lost is made again (`Sim::update_drones`, a kind at a time, the first kind
//! short first): its unit pays its rebuild price over its rebuild time as construction pays
//! (`Economy::spend_work`, slowed by the economy as a build is; [`Unit::drone_work`]), finished,
//! alive, not paralysed and not carried. Drones die with their unit (taken off the map with it,
//! where it was given up or upgraded: an upgraded unit's drones go to the unit it becomes), change
//! sides with it (`Sim::change_owner`), and one that changes sides on its own (captured) is no
//! longer its drone. An enhancement taken out takes its drones with it. A drone makes no drones of
//! its own. Nothing of it runs in a game whose units have no drones (`UnitTypes::drones`: TA's).
//!
//! [`Unit::drone_of`]: crate::unit::Unit::drone_of
//! [`Unit::drones`]: crate::unit::Unit::drones
//! [`Unit::drone_work`]: crate::unit::Unit::drone_work

use crate::defs::{Locomotion, ModelDef};
use crate::fixed::{FRAC_BITS, Fx, FxVec3};
use crate::unit::{Order, Unit, UnitId, piece_world_position};

use super::Sim;

/// The work a tick of a drone's rebuilding is (16.16, as construction's work counts).
const TICK_WORK: u64 = 1 << FRAC_BITS;

/// How often (ticks) a drone's leash and idleness are looked at.
const LOOK_EVERY: u64 = 8;

/// One kind of drone a unit keeps (`ta_data::DroneDef`), compiled: the drone's type, how many,
/// what one lost takes to make again (its work, a tick's [`TICK_WORK`] a tick at full pace, and its
/// price), where it is made (a piece of the unit's model), and how far from the unit it may go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroneSpec {
    pub unit_type: usize,
    pub count: u32,
    pub rebuild_work: u64,
    pub cost_metal: Fx,
    pub cost_energy: Fx,
    pub piece: Option<usize>,
    pub leash: Fx,
}

/// The drones a section gives (`defs`) a unit of `parent`'s file and model, compiled: those whose
/// unit the data has (and that is not the unit itself). `worker_time`: the build power its
/// rebuilding goes at where its section gives no time.
pub(crate) fn compile_drones(defs: &[ta_data::DroneDef], parent: &ta_data::UnitDef, worker_time: f64, model: &ModelDef, data: &ta_data::GameData) -> Vec<DroneSpec> {
    defs.iter()
        .filter_map(|d| {
            let unit_type = data.unit_id(&d.unit).filter(|&k| !data.units[k].name.eq_ignore_ascii_case(&parent.name))?;
            let drone = &data.units[unit_type];
            let seconds = d.rebuild_time.unwrap_or(if worker_time > 0.0 { drone.build_time / worker_time } else { 30.0 });
            let ticks = (seconds.clamp(0.0, 1.0e7) * 30.0).round().max(1.0) as u64;
            Some(DroneSpec {
                unit_type,
                count: d.count.min(ta_data::drones::MAX_DRONES),
                rebuild_work: ticks.saturating_mul(TICK_WORK),
                cost_metal: Fx::from_f64(d.rebuild_metal.unwrap_or(drone.build_cost_metal).clamp(0.0, 1.0e12)),
                cost_energy: Fx::from_f64(d.rebuild_energy.unwrap_or(drone.build_cost_energy).clamp(0.0, 1.0e12)),
                piece: (!d.piece.is_empty()).then(|| model.piece_index(&d.piece)).flatten(),
                leash: Fx::from_f64(d.leash.unwrap_or(ta_data::drones::DEFAULT_LEASH).clamp(0.0, 1.0e6)),
            })
        })
        .collect()
}

impl Sim {
    /// The drones `u` keeps: its type's, and those of the enhancements it has installed, summed by
    /// the drone's type (the first section's rebuild, piece and leash for a type). None for a drone.
    pub fn wanted_drones(&self, u: &Unit) -> Vec<DroneSpec> {
        if u.drone_of.is_some() {
            return Vec::new();
        }
        let t = self.types.get(u.type_id);
        let from_enhancements = u.enhancements.iter().filter_map(|&i| t.enhancements.get(i as usize)).flat_map(|e| e.drones.iter());
        let mut out: Vec<DroneSpec> = Vec::new();
        for spec in t.drones.iter().chain(from_enhancements) {
            match out.iter_mut().find(|o| o.unit_type == spec.unit_type) {
                Some(o) => o.count = (o.count + spec.count).min(ta_data::drones::MAX_DRONES),
                None => out.push(spec.clone()),
            }
        }
        out
    }

    /// `u`'s drones alive, linked to it, in the order they were made.
    pub fn drones_of<'a>(&'a self, u: &'a Unit) -> impl Iterator<Item = &'a Unit> + 'a {
        u.drones.iter().filter_map(|&d| self.unit(d)).filter(move |d| !d.dead && d.drone_of == Some(u.id))
    }

    /// What `u` keeps of each kind of drone, its type's and its enhancements' (a kind, how many):
    /// [`Sim::make_drones`]'s `before`.
    pub(super) fn drone_entitlement(&self, u: &Unit) -> Vec<(usize, u32)> {
        self.wanted_drones(u).iter().map(|w| (w.unit_type, w.count)).collect()
    }

    /// The unit's drones as it is finished, its enhancements change or it upgrades: those past what
    /// it keeps taken off the map (the newest first), and of each kind those it now keeps more of
    /// than `before` (what it kept until now: nothing for a unit just finished) made at once, for
    /// nothing. One lost before stays lost, to be made again at its price (`update_drones`), its
    /// rebuilding so far kept.
    pub(super) fn make_drones(&mut self, parent: UnitId, before: &[(usize, u32)]) {
        if !self.types.drones {
            return;
        }
        let Some(u) = self.unit(parent).filter(|u| !u.dead && u.is_complete() && u.drone_of.is_none()) else { return };
        let wanted = self.wanted_drones(u);
        if wanted.is_empty() && u.drones.is_empty() {
            return;
        }
        let living: Vec<(UnitId, usize)> = self.drones_of(u).map(|d| (d.id, d.type_id)).collect();
        // Past what it keeps (an enhancement taken out), the newest first.
        let mut kept: Vec<UnitId> = Vec::new();
        for &(d, kind) in &living {
            let keeps = wanted.iter().find(|w| w.unit_type == kind).map_or(0, |w| w.count) as usize;
            if kept.iter().filter(|&&k| self.unit(k).is_some_and(|k| k.type_id == kind)).count() < keeps {
                kept.push(d);
            } else {
                self.remove_unit(d);
            }
        }
        if let Some(u) = self.unit_mut(parent) {
            u.drones = kept.clone();
            u.drone_work = u.drone_work.filter(|&(kind, _)| wanted.iter().any(|w| w.unit_type == kind));
        }
        for spec in &wanted {
            let have = kept.iter().filter(|&&k| self.unit(k).is_some_and(|k| k.type_id == spec.unit_type)).count() as u32;
            let had = before.iter().find(|&&(kind, _)| kind == spec.unit_type).map_or(0, |&(_, n)| n);
            let granted = spec.count.saturating_sub(had);
            for _ in 0..granted.min(spec.count.saturating_sub(have)) {
                if self.spawn_drone(parent, spec).is_none() {
                    break;
                }
            }
        }
    }

    /// A drone of `spec` made for `parent`, finished, at its piece (an aircraft in flight there; a
    /// unit on the ground on the nearest spot it can stand on), guarding it. None at the unit limit.
    fn spawn_drone(&mut self, parent: UnitId, spec: &DroneSpec) -> Option<UnitId> {
        let p = self.unit(parent)?;
        let (owner, heading) = (p.owner, p.heading);
        if self.at_unit_limit(owner) {
            return None;
        }
        let pt = self.types.get(p.type_id);
        let at = spec.piece.map_or(p.pos, |piece| piece_world_position(p, pt, p.cob.as_ref(), piece));
        let air = self.types.get(spec.unit_type).locomotion == Locomotion::Air;
        let at = if air { at } else { self.drop_spot(spec.unit_type, at).unwrap_or(at) };
        // (Made unfinished, linked, then finished: a drone makes no drones of its own.)
        let id = self.spawn_unit(spec.unit_type, owner, at, heading, false)?;
        if let Some(d) = self.unit_mut(id) {
            d.drone_of = Some(parent);
            d.orders.push_back(Order::Guard(parent));
        }
        self.finish_frame(id);
        if air {
            self.put_in_flight(id, None);
        }
        if let Some(p) = self.unit_mut(parent) {
            p.drones.push(id);
        }
        Some(id)
    }

    /// The drones of `id` that change sides with it (those of `from`, its side before).
    pub(super) fn drones_following(&self, id: UnitId, from: u8) -> Vec<UnitId> {
        let Some(u) = self.unit(id) else { return Vec::new() };
        self.drones_of(u).filter(|d| d.owner == from).map(|d| d.id).collect()
    }

    /// An upgraded unit's drones go to the unit it became (`new`), which keeps as many as it should.
    pub(super) fn carry_drones_over(&mut self, old: UnitId, new: UnitId) {
        if !self.types.drones {
            return;
        }
        let Some(u) = self.unit(old) else { return };
        let drones: Vec<UnitId> = self.drones_of(u).map(|d| d.id).collect();
        if drones.is_empty() {
            return;
        }
        // (What the old unit kept: the new one makes at once only the more it keeps.)
        let before = self.drone_entitlement(u);
        for &d in &drones {
            if let Some(d) = self.unit_mut(d) {
                d.drone_of = Some(new);
                d.orders.clear();
                d.orders.push_back(Order::Guard(new));
            }
        }
        if let Some(u) = self.unit_mut(old) {
            u.drones.clear();
        }
        // (Those it carried over first: those made just now for it go, where it has too many.)
        if let Some(n) = self.unit_mut(new) {
            let made = std::mem::take(&mut n.drones);
            n.drones = drones.into_iter().chain(made).collect();
        }
        self.make_drones(new, &before);
    }

    /// The drones' tick: each one's link (its unit gone, dead or given up; a drone captured on its
    /// own), leash and standing order; each unit's rebuilding of those it lacks.
    pub(super) fn update_drones(&mut self) {
        if !self.types.drones {
            return;
        }
        let look = self.tick.is_multiple_of(LOOK_EVERY);
        let types = self.types.clone();
        // The drones and their units, in slot order.
        let drones: Vec<(UnitId, UnitId)> = self.units.iter().flatten().filter(|u| !u.dead).filter_map(|u| u.drone_of.map(|p| (u.id, p))).collect();
        for (id, parent) in drones {
            let Some(p) = self.unit(parent) else {
                self.kill_drone(id);
                continue;
            };
            match (p.dead, p.removed) {
                // (Given up or upgraded: taken off the map with it.)
                (true, true) => {
                    self.remove_unit(id);
                    continue;
                }
                (true, false) => {
                    self.kill_drone(id);
                    continue;
                }
                _ => {}
            }
            let (owner, pos, reach) = (p.owner, p.pos, types.get(p.type_id).radius);
            let kind = self.unit(id).map_or(usize::MAX, |d| d.type_id);
            let leash = self.wanted_drones(p).iter().find(|w| w.unit_type == kind).map_or(Fx::from_f64(ta_data::drones::DEFAULT_LEASH), |w| w.leash);
            // (What its unit mends, a finished unit of its side or an ally's, within the leash: a
            // builder drone helps with it, as guarding it helps with what it builds.)
            let mending = (look && types.get(kind).is_builder)
                .then_some(p.build)
                .flatten()
                .map(|j| j.target)
                .filter(|&w| w != id && self.unit(w).is_some_and(|w| !w.dead && w.is_complete() && w.carried_by.is_none() && w.pos.within_xz(pos, leash + reach) && self.allied(owner, w.owner) && self.needs_work(w, types.get(w.type_id))));
            let Some(d) = self.unit_mut(id) else { continue };
            if d.owner != owner {
                // (Captured, or given away, on its own: a unit of its own now.)
                d.drone_of = None;
                continue;
            }
            if !look || d.carried_by.is_some() || d.direct_control.is_some() {
                continue;
            }
            let guarding = matches!(d.orders.front(), Some(Order::Guard(g)) if *g == parent);
            let far = !d.pos.within_xz(pos, leash + reach);
            if d.orders.is_empty() || far && !guarding {
                // (Back to its unit: what it was told to do farther off given up.)
                if far {
                    d.orders.clear();
                    d.path.clear();
                    d.path_goal = None;
                    d.move_request = None;
                }
                d.orders.push_back(Order::Guard(parent));
                if far {
                    self.finish_build_job(id);
                }
            } else if guarding && !far && let Some(w) = mending {
                d.orders.push_front(Order::Repair(w));
            }
        }
        // The units rebuilding those they lack, in slot order.
        let parents: Vec<UnitId> = self.units.iter().flatten().filter(|u| !u.dead && u.drone_of.is_none() && (!u.drones.is_empty() || u.drone_work.is_some() || !types.get(u.type_id).drones.is_empty() || !u.enhancements.is_empty())).map(|u| u.id).collect();
        for parent in parents {
            self.rebuild_drones(parent);
        }
    }

    /// A unit's rebuilding: the first kind it is short of, a tick of its work paid; done, the drone
    /// made. Not while it is unfinished, paralysed or carried, nor at the unit limit.
    fn rebuild_drones(&mut self, parent: UnitId) {
        let Some(u) = self.unit(parent) else { return };
        let wanted = self.wanted_drones(u);
        if wanted.is_empty() {
            if u.drone_work.is_some() || !u.drones.is_empty() {
                // (Its drones' enhancement taken out meanwhile: `make_drones` took them.)
                if let Some(u) = self.unit_mut(parent) {
                    u.drone_work = None;
                }
            }
            return;
        }
        // (Forget the drones lost.)
        let living: Vec<UnitId> = self.drones_of(u).map(|d| d.id).collect();
        let short = wanted.iter().find(|w| (living.iter().filter(|&&d| self.unit(d).is_some_and(|d| d.type_id == w.unit_type)).count() as u32) < w.count).cloned();
        let able = u.is_complete() && u.carried_by.is_none() && self.tick >= u.paralyzed_until && !self.at_unit_limit(u.owner);
        let owner = u.owner;
        let work = u.drone_work;
        if let Some(u) = self.unit_mut(parent) {
            u.drones = living;
        }
        let Some(spec) = short else {
            if let Some(u) = self.unit_mut(parent) {
                u.drone_work = None;
            }
            return;
        };
        if !able {
            return;
        }
        let done = work.filter(|&(kind, _)| kind == spec.unit_type).map_or(0, |(_, w)| w);
        let e = &mut self.players[owner as usize].economy;
        let factor = e.build_factor;
        let full = TICK_WORK.min(spec.rebuild_work.saturating_sub(done));
        let paid = e.spend_work(done, spec.rebuild_work, full, spec.cost_metal, spec.cost_energy, factor);
        let done = done + paid;
        if done >= spec.rebuild_work {
            if let Some(u) = self.unit_mut(parent) {
                u.drone_work = None;
            }
            self.spawn_drone(parent, &spec);
        } else if let Some(u) = self.unit_mut(parent) {
            u.drone_work = Some((spec.unit_type, done));
        }
    }

    /// A drone dies with its unit, as a passenger dies with its transport.
    fn kill_drone(&mut self, id: UnitId) {
        let max = self.unit(id).map(|u| self.types.get(u.type_id).max_health).unwrap_or(Fx::ONE);
        self.damage_unit(id, max * 10, id, FxVec3::ZERO);
        if let Some(u) = self.unit_mut(id)
            && !u.dead
        {
            u.health = Fx::ZERO;
            u.dead = true;
            u.killer = Some(id);
        }
    }
}
