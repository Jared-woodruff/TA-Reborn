//! The interface's side of TA Reborn's abilities (docs/MODDING.md, "Abilities"): the weapon toggle
//! the selection switches (`Command::SetToggle`: a button in both interfaces, its key), remote
//! viewing's order (`CommandMode::RemoteView`, `Order::RemoteView`: a button, its key and its
//! cursor) and teleporting's (`CommandMode::Teleport`, `Order::Teleport`: the same; its charge and
//! its landing shown by `effects_teleport.rs`). Stuns show as TA's paralysis does (the crackle, the
//! bar, "Paralyzed"), and the fields, auras and minimum ranges on the information page.

use ta_sim::{Command, UnitId};
use ta_ui::i18n::{t, tf};

use super::Game;

/// The selection's weapon toggle, for its button: its name (the first unit's toggle's), whether
/// every unit that has one is switched on, and whether some are and some are not.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponToggle {
    pub name: String,
    pub on: bool,
    pub mixed: bool,
}

impl Game {
    /// The selected units of ours that have a weapon toggle.
    fn toggles(&self) -> Vec<UnitId> {
        self.selection.iter().copied().filter(|&id| self.sim.unit(id).is_some_and(|u| u.owner == self.local_player && self.sim.can_toggle(u))).collect()
    }

    /// The selection's weapon toggle, if any of it has one.
    pub fn weapon_toggle(&self) -> Option<WeaponToggle> {
        let units = self.toggles();
        let first = self.sim.unit(*units.first()?)?;
        let name = self.sim.unit_type(first).def.toggle.as_ref().map_or_else(String::new, |t| t.name.clone());
        let on = units.iter().filter(|&&id| self.sim.unit(id).is_some_and(|u| u.toggled)).count();
        Some(WeaponToggle { name, on: on == units.len(), mixed: on > 0 && on < units.len() })
    }

    /// Switches the selection's weapon toggles: on, or off where every one is on (some on and some
    /// off: all on).
    pub fn toggle_selection_weapons(&mut self) {
        let units = self.toggles();
        let Some(state) = self.weapon_toggle() else { return };
        self.ui_sounds.push("IMMEDIATEORDERS");
        self.issue(Command::SetToggle { player: self.local_player, units, on: !state.on });
    }

    /// The weapon toggle's words for its button's tooltip: what it switches to, and its key.
    pub fn weapon_toggle_tip(&self) -> String {
        let Some(state) = self.weapon_toggle() else { return String::new() };
        let key = self.keys.text("weapon_toggle").to_string();
        if state.on {
            tf("{name} ({key}): its other weapons in use. Click for its own again", &[("name", &state.name), ("key", &key)])
        } else {
            tf("{name} ({key}): click to switch its weapons over", &[("name", &state.name), ("key", &key)])
        }
    }

    /// The label of the toggle's button: its name, in capitals, as the interface's buttons are.
    pub fn weapon_toggle_label(&self) -> String {
        self.weapon_toggle().map_or_else(|| t("WEAPONS").to_string(), |s| ta_ui::i18n::upper(&s.name))
    }

    /// Whether any of the selection can view a place from afar.
    pub fn selection_views_afar(&self) -> bool {
        self.selection.iter().any(|&id| self.sim.unit(id).is_some_and(|u| self.sim.can_view_afar(u)))
    }

    /// Whether any of the selection can teleport (its type, or an enhancement it has installed).
    pub fn selection_teleports(&self) -> bool {
        self.selection.iter().any(|&id| self.sim.unit(id).is_some_and(|u| u.owner == self.local_player && self.sim.can_teleport(u)))
    }
}
