//! Factories that move (a unit type's `MobileFactory=1` with its `BuildPiece`: TA Reborn's own;
//! Supreme Commander's Fatboy, carriers, Atlantis, Tempest and CZAR). Such a unit is a factory
//! (`UnitType::is_factory`): its player queues the units of its menu (`Command::Produce`), and it
//! builds them one after another from its queue with its own build power, paid as construction is
//! paid, while it goes about its orders. Each is built on it, at its build piece (`Unit::carried_by`
//! it, `Unit::carry_piece` the piece, `Unit::factory` it: carried as a transport's passenger is, it
//! rides along, out of the way of collisions and targeting, and dies with it). Done, it is set down
//! beside the factory, behind it, on the nearest ground it can stand on (on the water for a ship),
//! or takes off from it and flies clear of it, behind it, if it flies; then it follows the factory's
//! rally orders (`Command::SetRally`), if any. Nothing is started while it upgrades or installs an
//! enhancement; one with nowhere to be set down waits on it. A factory that moves builds no
//! buildings from its queue (they are left out of it), and is given no order to build one of its
//! units in the field: they come from its queue.

use crate::defs::{Locomotion, UnitType};
use crate::events::SimEvent;
use crate::fixed::{Fx, FxVec3, rotate_y};
use crate::unit::{BuildJob, Order, UnitId, piece_world_position};

use super::{Sim, queue_order};

impl Sim {
    /// A factory that moves, its production for the tick (`update_factory` hands it over): the unit
    /// on it built on, or let go once done; else the next in its queue started on it.
    pub(super) fn update_mobile_factory(&mut self, id: UnitId, t: &UnitType) {
        let types = self.types.clone();
        let Some(f) = self.unit(id) else { return };
        // (Upgrading in place, or installing an enhancement, it makes nothing.)
        if matches!(f.orders.front(), Some(Order::Upgrade(_) | Order::Enhance { remove: false, .. })) {
            return;
        }
        if let Some(job) = f.build {
            let Some(target) = self.unit(job.target).filter(|p| !p.dead && p.factory == Some(id)) else {
                self.finish_build_job(id);
                return;
            };
            if target.is_complete() {
                self.release_product(id, t, job.target);
            } else {
                self.apply_build_power(id, t, job.target);
            }
            return;
        }
        // The next in its queue: a unit that moves (a building is left out of it).
        let Some(f) = self.unit(id) else { return };
        let Some(&type_id) = f.factory_queue.front() else { return };
        if types.get(type_id).locomotion == Locomotion::Static {
            if let Some(f) = self.unit_mut(id) {
                f.factory_queue.pop_front();
            }
            return;
        }
        let owner = f.owner;
        if self.at_unit_limit(owner) {
            return;
        }
        let at = self.build_piece_place(id, t);
        let heading = f.heading;
        self.start_script(id, "StartBuilding", &[]);
        let Some(new) = self.spawn_unit(type_id, owner, at, heading, false) else { return };
        if let Some(p) = self.unit_mut(new) {
            p.factory = Some(id);
            p.carried_by = Some(id);
            p.carry_piece = t.build_piece;
            p.pos = at;
        }
        if let Some(f) = self.unit_mut(id) {
            f.build = Some(BuildJob { target: new, started: true });
        }
        self.events.push(SimEvent::NanoSpray { builder: id, target: new });
    }

    /// Where a factory that moves builds (its build piece, else its middle), now.
    fn build_piece_place(&self, id: UnitId, t: &UnitType) -> FxVec3 {
        let Some(f) = self.unit(id) else { return FxVec3::ZERO };
        t.build_piece.map_or(f.pos, |piece| piece_world_position(f, t, f.cob.as_ref(), piece))
    }

    /// Lets the unit built on a factory that moves go: an aircraft taking off from it and flying
    /// clear of it, behind it; another set down behind it on the nearest ground (or water) it can
    /// stand on; then the factory's rally orders. One with nowhere to be set down waits on it.
    fn release_product(&mut self, id: UnitId, t: &UnitType, product: UnitId) {
        let types = self.types.clone();
        let (Some(f), Some(p)) = (self.unit(id), self.unit(product)) else { return };
        let pt = types.get(p.type_id);
        let (heading, rally) = (f.heading, f.rally.clone());
        let air = pt.locomotion == Locomotion::Air;
        // (Behind it, clear of it and all it reaches out with, on the map.)
        let room = t.radius.max(t.model.reach) + pt.radius.max(pt.model.reach) + Fx::from_int(16);
        let back = f.pos + rotate_y(FxVec3::new(Fx::ZERO, Fx::ZERO, -room), heading);
        let (w, h) = self.world.size_px();
        let clear = FxVec3::new(back.x.clamp(Fx::ZERO, w), Fx::ZERO, back.z.clamp(Fx::ZERO, h));
        let at = if air { Some(self.build_piece_place(id, t)) } else { self.drop_spot(p.type_id, clear).or_else(|| self.drop_spot(p.type_id, f.pos)) };
        let Some(at) = at else { return };
        let ground = self.ground_level(pt, at);
        if let Some(p) = self.unit_mut(product) {
            p.carried_by = None;
            p.carry_piece = None;
            p.factory = None;
            p.heading = heading;
            p.pos = if air { at } else { FxVec3::new(at.x, ground, at.z) };
            p.path_goal = None;
            p.move_request = None;
            // (An aircraft flies out of it, clear of it, first: made inside a carrier's hull.)
            if air && p.orders.is_empty() {
                p.orders.push_back(Order::Move(clear));
            }
            for o in &rally {
                let pos = p.pos;
                queue_order(&mut p.orders, o, true, pos);
            }
        }
        if air {
            self.put_in_flight(product, None);
        } else if let Some(p) = self.units.get_mut(product.slot as usize).and_then(|u| u.as_mut()) {
            Self::settle(p, pt, &self.world);
        }
        if let Some(f) = self.unit_mut(id) {
            f.build = None;
            let done = f.factory_queue.pop_front();
            if let (true, Some(t)) = (f.factory_repeat, done) {
                f.factory_queue.push_back(t);
            }
        }
        self.start_script(id, "StopBuilding", &[]);
    }
}
