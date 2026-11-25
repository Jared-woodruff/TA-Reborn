//! A unit's weapon toggle (`[TOGGLE]` within its `[UNITINFO]`): another set of weapons its player
//! switches it to and back, as Supreme Commander's units switch theirs (the sniper bot's long
//! gun, the Cybran anti-air's and cruisers' ground guns). Laid out as an enhancement is: its
//! name, and the weapon each slot fires while it is on in place of the unit's own there; a slot
//! named empty fires nothing while it is on. TA's own units have none.
//!
//! ```text
//! [UNITINFO]
//! {
//!     UnitName=XSL0305;
//!     Weapon1=XSL0305_MAINGUN;
//!     ...
//!     [TOGGLE]
//!     {
//!         Name=Sniper Gun;
//!         Weapon1=;
//!         Weapon2=XSL0305_SNIPERGUN;
//!     }
//! }
//! ```

use ta_formats::tdf::TdfSection;

use crate::unit::MAX_WEAPONS;

/// A unit's weapon toggle, as its section gives it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToggleDef {
    /// What players see on its button (`Name`; "Weapons" where it gives none) and a line of what
    /// it does (`Description`).
    pub name: String,
    pub description: String,
    /// Each slot while it is on (`Weapon1` to `Weapon8`): none, the unit's own; an empty name
    /// (`Weapon2=;`, or `NONE`), nothing; else the weapon of that name (upper case).
    pub weapons: [Option<String>; MAX_WEAPONS],
}

impl ToggleDef {
    /// The toggle of a unit's `[UNITINFO]` section, if it has one that changes a slot.
    pub fn of_unit(info: &TdfSection) -> Option<ToggleDef> {
        let s = info.sections.iter().find(|s| s.name.eq_ignore_ascii_case("TOGGLE"))?;
        let st = |k: &str| s.get(k).unwrap_or("").trim().to_string();
        let weapons: [Option<String>; MAX_WEAPONS] = std::array::from_fn(|slot| {
            s.get(&format!("Weapon{}", slot + 1)).map(|w| {
                let w = w.trim().to_ascii_uppercase();
                if w == "NONE" { String::new() } else { w }
            })
        });
        if weapons.iter().all(Option::is_none) {
            log::warn!("{}: its [TOGGLE] changes no weapon slot; left out", info.get("UnitName").unwrap_or("?"));
            return None;
        }
        let name = st("Name");
        Some(ToggleDef { name: if name.is_empty() { "Weapons".into() } else { name }, description: st("Description"), weapons })
    }

    /// The weapons it names (not the slots it empties).
    pub fn named(&self) -> impl Iterator<Item = (usize, &str)> {
        self.weapons.iter().enumerate().filter_map(|(slot, w)| w.as_deref().filter(|w| !w.is_empty()).map(|w| (slot, w)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::tdf;

    #[test]
    fn a_toggle_is_read_slot_by_slot() {
        let doc = tdf::parse("[UNITINFO]{UnitName=SNIPER; Weapon1=GUN; [TOGGLE]{ Name=Sniper Gun; Weapon1=; Weapon2= long_gun ; Weapon4=none; } }");
        let t = ToggleDef::of_unit(doc.section("UNITINFO").unwrap()).unwrap();
        assert_eq!(t.name, "Sniper Gun");
        assert_eq!(t.weapons[0].as_deref(), Some(""), "emptied while it is on");
        assert_eq!(t.weapons[1].as_deref(), Some("LONG_GUN"), "a weapon in capitals");
        assert_eq!(t.weapons[3].as_deref(), Some(""), "NONE empties it too");
        assert!(t.weapons[2].is_none() && t.weapons[4..].iter().all(Option::is_none), "the unit's own");
        assert_eq!(t.named().collect::<Vec<_>>(), [(1, "LONG_GUN")]);
        // None that changes a slot, or no section: no toggle.
        assert!(ToggleDef::of_unit(tdf::parse("[UNITINFO]{UnitName=X; [TOGGLE]{ Name=Nothing; } }").section("UNITINFO").unwrap()).is_none());
        assert!(ToggleDef::of_unit(tdf::parse("[UNITINFO]{UnitName=ARMPW;}").section("UNITINFO").unwrap()).is_none());
        let unnamed = ToggleDef::of_unit(tdf::parse("[UNITINFO]{UnitName=X; [TOGGLE]{ Weapon3=B; } }").section("UNITINFO").unwrap()).unwrap();
        assert_eq!(unnamed.name, "Weapons");
    }
}
