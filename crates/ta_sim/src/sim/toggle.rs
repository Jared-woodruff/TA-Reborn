//! Weapon toggles (a unit type's `[TOGGLE]`: Supreme Commander's sniper bot's long gun, the Cybran
//! anti-air's and cruisers' ground guns). [`Command::SetToggle`] switches a unit's toggle on or off:
//! on, each slot the toggle names fires its weapon in place of the unit's own there (over what its
//! enhancements put there), or nothing; off, its own again. The unit's figures are made again as
//! an enhancement's are (`Sim::refit`): a slot whose weapon changed starts afresh, and a launcher
//! that stockpiles its missiles, swapped for another or taken out, loses those built and the one
//! under way. Its script is told (`SetToggle(on)`, where it has one), and
//! `SimEvent::UnitToggled` follows. [`Unit::toggled`] is in the checksum.
//!
//! [`Command::SetToggle`]: super::Command::SetToggle

use crate::events::SimEvent;
use crate::unit::{Unit, UnitId};

use super::Sim;

impl Sim {
    /// Whether `u` has a weapon toggle it may switch: finished, alive, of a type with one.
    pub fn can_toggle(&self, u: &Unit) -> bool {
        !u.dead && u.is_complete() && self.types.get(u.type_id).abilities.toggle.is_some()
    }

    /// `player`'s units given switched to their toggle's weapons (`on`) or back to their own:
    /// those of theirs that have a toggle and are not already so.
    pub(super) fn set_toggles(&mut self, player: u8, units: &[UnitId], on: bool) {
        for &id in units {
            if self.unit(id).is_some_and(|u| u.owner == player && u.toggled != on && self.can_toggle(u)) {
                self.set_toggle(id, on);
            }
        }
    }

    /// Switches a unit's toggle on or off, at once.
    pub(crate) fn set_toggle(&mut self, id: UnitId, on: bool) {
        let Some(u) = self.unit(id) else { return };
        let (installed, type_id) = (u.enhancements.clone(), u.type_id);
        self.refit(id, installed, on);
        if self.types.get(type_id).callins.set_toggle {
            self.call_script(id, "SetToggle", &[on as i32]);
        }
        self.events.push(SimEvent::UnitToggled { id, on });
    }
}
