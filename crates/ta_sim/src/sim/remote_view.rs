//! Remote viewing (a unit type's `RemoteViewRadius` and `RemoteViewEnergy`, or an enhancement's:
//! the Aeon's Eye of Rhianne). Given [`Order::RemoteView`], a unit that has it views the place, as
//! long as the order stands (another order not queued, or Stop, ends it): every tick it is at work
//! (`Sim::working`: finished, switched on or not switchable, its upkeep paid, not paralysed) it pays
//! its `RemoteViewEnergy` for the tick, and paid, the place is its side's to see
//! ([`Unit::remote_view`]): the vision grid sees round it as far as its `RemoteViewRadius`, from an
//! eye [`REMOTE_EYE`] over the ground there (`Sim::rebuild_vision`). Short of energy, or not at
//! work, it sees nothing there that tick. Anywhere on the map: it has no range.
//!
//! [`Order::RemoteView`]: crate::unit::Order::RemoteView
//! [`Unit::remote_view`]: crate::unit::Unit::remote_view

use crate::defs::UnitType;
use crate::events::SimEvent;
use crate::fixed::{Fx, FxVec3};
use crate::unit::{Order, Unit, UnitId};

use super::Sim;

/// How high over the ground a remote view's eye is (px): it sees what hills would hide from a
/// unit there, with TA's "true" line of sight.
pub const REMOTE_EYE: i64 = 400;

impl Sim {
    /// Whether `u` can view a place from afar (its type's remote viewing, or an enhancement's).
    pub fn can_view_afar(&self, u: &Unit) -> bool {
        u.abilities(self.types.get(u.type_id)).remote_view.is_some()
    }

    /// The place `u` views from afar now, and how far round it: none while it views nothing (no
    /// order to, short of energy, not at work).
    pub fn viewing(&self, u: &Unit) -> Option<(FxVec3, Fx)> {
        let view = u.abilities(self.types.get(u.type_id)).remote_view?;
        let at = u.remote_view?;
        (matches!(u.orders.front(), Some(Order::RemoteView(p)) if self.on_map(*p) == at) && self.working(u)).then_some((at, view.radius))
    }

    /// `p` held within the map.
    fn on_map(&self, p: FxVec3) -> FxVec3 {
        let (w, h) = self.world.size_px();
        FxVec3::new(p.x.clamp(Fx::ZERO, w), Fx::ZERO, p.z.clamp(Fx::ZERO, h))
    }

    /// `Order::RemoteView` at the front of a unit's orders: this tick's energy paid while it is at
    /// work, and the place viewed if it was. One that cannot view from afar (an enhancement that
    /// gave it taken out) fails.
    pub(super) fn update_remote_view_order(&mut self, id: UnitId, t: &UnitType, at: FxVec3) {
        let Some(u) = self.unit(id) else { return };
        let Some(view) = u.abilities(t).remote_view else {
            self.events.push(SimEvent::OrderFailed { unit: id, reason: "it cannot see from afar" });
            if let Some(u) = self.unit_mut(id) {
                u.orders.pop_front();
                u.remote_view = None;
            }
            return;
        };
        let (owner, working) = (u.owner as usize, self.working(u));
        let at = self.on_map(at);
        let paid = working && {
            let e = &mut self.players[owner].economy;
            e.tick_energy_wanted += view.energy;
            let paid = e.energy >= view.energy;
            if paid {
                e.energy -= view.energy;
                e.tick_energy_out += view.energy;
            }
            paid
        };
        if let Some(u) = self.unit_mut(id) {
            u.remote_view = paid.then_some(at);
        }
    }

    /// A unit's view from afar given up with its order (Stop, another order).
    pub(super) fn end_remote_view(&mut self, id: UnitId) {
        if let Some(u) = self.unit_mut(id)
            && u.remote_view.is_some()
            && !matches!(u.orders.front(), Some(Order::RemoteView(_)))
        {
            u.remote_view = None;
        }
    }
}
