//! Teleporting (a unit type's `CanTeleport`, `TeleportEnergy` and `TeleportTime`, or an
//! enhancement's: `ta_data::TeleportDef`; Supreme Commander's commanders' Personal Teleporter).
//! Given [`Order::Teleport`], a unit that can teleport stands where it is and pays its charge (the
//! energy for the way it goes, over its time), as construction pays (`Economy::spend_work`: slowed
//! by the economy as a build is, nothing paid while there is no energy), its [`Unit::teleport`]
//! counting the work; paid, it lands on the nearest spot to the place it was sent that it can stand
//! on (`Sim::drop_spot`; an aircraft at the place itself, as high over the ground as it was), and
//! [`SimEvent::Teleported`] tells the interface, which shows it go and come at both ends. The order
//! taken off its orders (Stop, another order not queued after it) calls the charge off: what was
//! paid is spent. A place with nowhere to stand within a few cells fails the order, before the
//! charge and as it lands. Anywhere on the map: it has no range. A building does not teleport.
//!
//! [`Order::Teleport`]: crate::unit::Order::Teleport
//! [`Unit::teleport`]: crate::unit::Unit::teleport
//! [`SimEvent::Teleported`]: crate::events::SimEvent::Teleported

use crate::cob::TICKS_PER_SECOND;
use crate::defs::{Locomotion, UnitType};
use crate::events::SimEvent;
use crate::fixed::{FRAC_BITS, Fx, FxVec3};
use crate::unit::{Order, Unit, UnitId};

use super::Sim;

/// The work a tick of a teleport's charge is (16.16, as construction's work counts).
const TICK_WORK: u64 = 1 << FRAC_BITS;

/// A teleport (`ta_data::TeleportDef`), compiled: the energy it takes, a base and so much a pixel
/// of the way, and the ticks it takes, likewise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Teleport {
    pub energy: Fx,
    pub energy_per_px: Fx,
    pub ticks: Fx,
    pub ticks_per_px: Fx,
}

impl Teleport {
    pub(crate) fn of(def: &ta_data::TeleportDef) -> Teleport {
        let fx = |v: f64| Fx::from_f64(v.clamp(0.0, 1.0e12));
        Teleport { energy: fx(def.energy), energy_per_px: fx(def.energy_per_px), ticks: fx(def.time * TICKS_PER_SECOND as f64), ticks_per_px: fx(def.time_per_px * TICKS_PER_SECOND as f64) }
    }

    /// Its charge for a way of `distance` px: the energy, and the ticks it takes (one at least).
    pub fn charge(&self, distance: Fx) -> (Fx, u64) {
        let d = distance.max(Fx::ZERO);
        let ticks = (self.ticks + self.ticks_per_px * d).to_int().clamp(1, i64::MAX >> FRAC_BITS) as u64;
        (self.energy + self.energy_per_px * d, ticks)
    }
}

/// A teleport's charge under way: where to, the work paid for so far of its whole (a tick's work
/// each tick at full pace), and the energy the whole costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Charge {
    pub to: FxVec3,
    pub work: u64,
    pub total: u64,
    pub energy: Fx,
}

impl Charge {
    /// How far along it is, 0 to 1.
    pub fn progress(&self) -> Fx {
        Fx::ONE.share(self.work.min(self.total), self.total.max(1))
    }
}

impl Sim {
    /// The teleport `u` has: its type's, else one of its enhancements' (the last installed's).
    pub fn teleport_of(&self, u: &Unit) -> Option<Teleport> {
        let t = self.types.get(u.type_id);
        if t.locomotion == Locomotion::Static {
            return None;
        }
        u.enhancements.iter().rev().find_map(|&i| t.enhancements.get(i as usize).and_then(|e| e.teleport)).or(t.teleport)
    }

    /// Whether `u` can teleport (its type, or an enhancement it has installed).
    pub fn can_teleport(&self, u: &Unit) -> bool {
        self.teleport_of(u).is_some()
    }

    /// Where a unit of `type_id` sent to `to` lands: the nearest spot to it that it can stand on,
    /// within a few cells (`drop_spot`); an aircraft the place itself. None: nowhere.
    pub fn teleport_landing(&self, type_id: usize, to: FxVec3) -> Option<FxVec3> {
        let (w, h) = self.world.size_px();
        let to = FxVec3::new(to.x.clamp(Fx::ZERO, w), Fx::ZERO, to.z.clamp(Fx::ZERO, h));
        if self.types.get(type_id).locomotion == Locomotion::Air {
            return Some(to);
        }
        self.drop_spot(type_id, to)
    }

    /// `Order::Teleport` at the front of a unit's orders: the charge started (it stands), a tick of
    /// it paid as construction pays, and once paid, the unit lands.
    pub(super) fn update_teleport_order(&mut self, id: UnitId, t: &UnitType, to: FxVec3) {
        let fail = |sim: &mut Sim, reason: &'static str| {
            sim.events.push(SimEvent::OrderFailed { unit: id, reason });
            if let Some(u) = sim.unit_mut(id) {
                u.orders.pop_front();
                u.teleport = None;
            }
        };
        let Some(u) = self.unit(id) else { return };
        let Some(teleport) = self.teleport_of(u) else {
            fail(self, "it cannot teleport");
            return;
        };
        let (owner, from, type_id) = (u.owner, u.pos, u.type_id);
        let charge = match u.teleport.filter(|c| c.to == to) {
            Some(c) => c,
            None => {
                // (Somewhere to land, or it is not started.)
                if self.teleport_landing(type_id, to).is_none() {
                    fail(self, "nowhere to land there");
                    return;
                }
                let (energy, ticks) = teleport.charge(from.distance_xz(to));
                Charge { to, work: 0, total: ticks.saturating_mul(TICK_WORK), energy }
            }
        };
        // (It stands while it charges.)
        self.stop(id);
        let e = &mut self.players[owner as usize].economy;
        let full = TICK_WORK.min(charge.total.saturating_sub(charge.work));
        let factor = e.build_factor;
        let paid = e.spend_work(charge.work, charge.total, full, Fx::ZERO, charge.energy, factor);
        let charge = Charge { work: charge.work + paid, ..charge };
        if charge.work < charge.total {
            if let Some(u) = self.unit_mut(id) {
                u.teleport = Some(charge);
                u.speed = Fx::ZERO;
            }
            return;
        }
        let Some(spot) = self.teleport_landing(type_id, to) else {
            fail(self, "nowhere to land there");
            return;
        };
        self.land_teleport(id, t, spot);
    }

    /// Puts a unit whose charge is paid down at `spot`: on the ground (or the water) there, an
    /// aircraft as high over it as it flew; its order done.
    fn land_teleport(&mut self, id: UnitId, t: &UnitType, spot: FxVec3) {
        self.finish_build_job(id);
        let Some(u) = self.unit(id) else { return };
        let from = u.pos;
        let height = if t.locomotion == Locomotion::Air { (from.y - self.world.height_at(from.x, from.z).max(self.world.sea_level)).max(Fx::ZERO) } else { Fx::ZERO };
        let ground = self.ground_level(t, spot);
        if let Some(u) = self.unit_mut(id) {
            u.pos = FxVec3::new(spot.x, ground + height, spot.z);
            u.orders.pop_front();
            u.teleport = None;
            u.speed = Fx::ZERO;
            u.path.clear();
            u.path_goal = None;
            u.move_request = None;
            u.idle_anchor = None;
            u.formations.clear();
            u.drift = None;
            u.orbit = None;
            u.new_route();
        }
        if matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval)
            && let Some(u) = self.units.get_mut(id.slot as usize).and_then(|u| u.as_mut())
        {
            Self::settle(u, t, &self.world);
        }
        let to = self.unit(id).map_or(spot, |u| u.pos);
        self.events.push(SimEvent::Teleported { id, from, to });
    }

    /// A charge whose order is gone (Stop, another order) is called off: what was paid is spent.
    pub(super) fn end_teleport(&mut self, id: UnitId) {
        if let Some(u) = self.unit_mut(id)
            && let Some(c) = u.teleport
            && !matches!(u.orders.front(), Some(Order::Teleport(p)) if *p == c.to)
        {
            u.teleport = None;
        }
    }
}
