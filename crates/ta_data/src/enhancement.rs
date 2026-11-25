//! A unit's enhancements (`[ENHANCEMENTS]` within its `[UNITINFO]`): parts it installs on itself,
//! Supreme Commander's commander enhancements. Each takes a slot of the unit's (one enhancement a
//! slot), may need another installed first, costs metal, energy and build time as a unit does, and
//! changes the unit while installed: its numbers added to the unit's own, its weapon slots filled
//! or swapped, its menu grown. TA's own units have none.
//!
//! ```text
//! [UNITINFO]
//! {
//!     UnitName=UEL0001;
//!     ...
//!     [ENHANCEMENTS]
//!     {
//!         [AdvancedEngineering]
//!         {
//!         Name=Tech 2 Engineering Suite;
//!         Slot=LCH;
//!         BuildCostMetal=720;
//!         BuildCostEnergy=18000;
//!         BuildTime=9000;
//!         WorkerTime=200;
//!         MaxDamage=3000;
//!         HealRate=20;
//!         BuildOptions=UEB0201 UEB0202;
//!         }
//!         [T3Engineering]
//!         {
//!         Prerequisite=AdvancedEngineering;
//!         ...
//!         }
//!     }
//! }
//! ```

use ta_formats::tdf::TdfSection;

use crate::unit::MAX_WEAPONS;

/// Most enhancements a unit's file may give (the rest are left out, with a warning).
pub const MAX_ENHANCEMENTS: usize = 64;

/// One enhancement, as its section gives it. Its effects hold while it is installed, added to the
/// unit's own (a slot's weapon named here takes the slot's place).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnhancementDef {
    /// Its id: the section's name (`AdvancedEngineering`), what another's `Prerequisite` names.
    pub id: String,
    /// What players see (`Name`; its id where it gives none) and a line of what it does
    /// (`Description`).
    pub name: String,
    pub description: String,
    /// The slot it takes (`Slot`: `Back`, `LCH`, `RCH` on Supreme Commander's commanders); one
    /// enhancement a slot, one installed in it taking the place of the one there.
    pub slot: String,
    /// The enhancement it needs installed first (`Prerequisite`, an id of the unit's; empty: none).
    pub prerequisite: String,
    pub build_cost_metal: f64,
    pub build_cost_energy: f64,
    /// Build time in worker-time units, as a unit's: the unit installs it in `build_time /
    /// WorkerTime` seconds with its own build power.
    pub build_time: f64,
    /// Its picture, `unitpics/<picture>.pcx` (empty: none; the menus draw its name).
    pub picture: String,
    /// Hit points added to the unit's most (`MaxDamage`).
    pub max_damage: f64,
    /// Hit points a second it mends besides its own (`HealRate`; its own is 100 every `HealTime`).
    pub heal_rate: f64,
    /// Build power added (`WorkerTime`).
    pub worker_time: f64,
    /// Sight, radar and sonar added (`SightDistance`, `RadarDistance`, `SonarDistance`, px).
    pub sight_distance: f64,
    pub radar_distance: f64,
    pub sonar_distance: f64,
    /// Units added to its menu (`BuildOptions`, their names apart by spaces or commas).
    pub build_options: Vec<String>,
    /// The weapons its slots fire while it is installed (`Weapon1` to `Weapon8`): an empty slot
    /// filled (a missile launcher fitted), a slot's own weapon swapped for another (a longer gun).
    pub weapons: [Option<String>; MAX_WEAPONS],
    /// It cloaks the unit as TA's cloak does (`CloakCost` energy a second while cloaked,
    /// `CloakCostMoving` while moving, seen through within `MinCloakDistance`): the unit may cloak.
    pub cloak_cost: f64,
    pub cloak_cost_moving: f64,
    pub min_cloak_distance: f64,
    /// It hides the unit from radar (`Stealth`).
    pub stealth: bool,
    /// Jamming and stealth fields it gives the unit (`RadarDistanceJam`, `SonarDistanceJam`,
    /// `RadarStealthField`, `SonarStealthField`, px), as a unit's own keys do: the widest of each
    /// counts.
    pub radar_jam: f64,
    pub sonar_jam: f64,
    pub radar_stealth_field: f64,
    pub sonar_stealth_field: f64,
    /// Remote viewing it gives the unit (`RemoteViewRadius`, px; `RemoteViewEnergy`, energy a
    /// second while it views).
    pub remote_view_radius: f64,
    pub remote_view_energy: f64,
    /// A regeneration aura it gives the unit (`RegenAuraRadius`, px; `RegenAuraRate`, hit points a
    /// second; `RegenAuraShare`, a share of each one's most health a second).
    pub regen_aura_radius: f64,
    pub regen_aura_rate: f64,
    pub regen_aura_share: f64,
    /// What it makes a second while installed (`MetalMake`, `EnergyMake`: Supreme Commander's
    /// resource allocation), and the energy it takes a second (`EnergyUse`, paid while it is
    /// installed: short of it, the unit's radar, sonar, jamming, fields and remote viewing stop).
    pub metal_make: f64,
    pub energy_make: f64,
    pub energy_use: f64,
    /// A shield it gives the unit (the shield keys, as a unit's own: [`crate::ShieldDef`]), in place
    /// of the unit's own while it is installed (of several, the one of the most hit points).
    pub shield: Option<crate::ShieldDef>,
}

impl EnhancementDef {
    /// Reads one enhancement's section (its name is its id). None for one with no name.
    pub fn from_tdf(s: &TdfSection) -> Option<EnhancementDef> {
        let id = s.name.trim().to_string();
        if id.is_empty() {
            return None;
        }
        let f = |k: &str| s.get_f64_or(k, 0.0).max(0.0);
        let st = |k: &str| s.get(k).unwrap_or("").trim().to_string();
        let name = st("Name");
        let weapon = |k: &str| s.get(k).map(|w| w.trim().to_ascii_uppercase()).filter(|w| !w.is_empty());
        Some(EnhancementDef {
            name: if name.is_empty() { id.clone() } else { name },
            description: st("Description"),
            slot: st("Slot"),
            prerequisite: st("Prerequisite"),
            build_cost_metal: f("BuildCostMetal"),
            build_cost_energy: f("BuildCostEnergy"),
            build_time: s.get_f64_or("BuildTime", 1.0).max(1.0),
            picture: st("Picture"),
            max_damage: f("MaxDamage"),
            heal_rate: f("HealRate"),
            worker_time: f("WorkerTime"),
            sight_distance: f("SightDistance"),
            radar_distance: f("RadarDistance"),
            sonar_distance: f("SonarDistance"),
            build_options: st("BuildOptions").split([' ', ',', '\t']).map(|u| u.trim().to_ascii_uppercase()).filter(|u| !u.is_empty()).collect(),
            weapons: std::array::from_fn(|slot| weapon(&format!("Weapon{}", slot + 1))),
            cloak_cost: f("CloakCost"),
            cloak_cost_moving: f("CloakCostMoving"),
            min_cloak_distance: f("MinCloakDistance"),
            stealth: s.get_bool_or("Stealth", false),
            radar_jam: f("RadarDistanceJam"),
            sonar_jam: f("SonarDistanceJam"),
            radar_stealth_field: f("RadarStealthField"),
            sonar_stealth_field: f("SonarStealthField"),
            remote_view_radius: f("RemoteViewRadius"),
            remote_view_energy: f("RemoteViewEnergy"),
            regen_aura_radius: f("RegenAuraRadius"),
            regen_aura_rate: f("RegenAuraRate"),
            regen_aura_share: f("RegenAuraShare"),
            metal_make: f("MetalMake"),
            energy_make: f("EnergyMake"),
            energy_use: f("EnergyUse"),
            shield: crate::ShieldDef::of(s),
            id,
        })
    }

    /// Whether it gives the unit a regeneration aura (a reach, and something to mend with).
    pub fn mends_round(&self) -> bool {
        self.regen_aura_radius > 0.0 && (self.regen_aura_rate > 0.0 || self.regen_aura_share > 0.0)
    }

    /// The enhancements of a unit's `[UNITINFO]` section: its `[ENHANCEMENTS]` section's own, in
    /// the file's order (their index is the order players give), an id given twice taken once (the
    /// first).
    pub fn of_unit(info: &TdfSection) -> Vec<EnhancementDef> {
        let Some(list) = info.sections.iter().find(|s| s.name.eq_ignore_ascii_case("ENHANCEMENTS")) else { return Vec::new() };
        let mut out: Vec<EnhancementDef> = Vec::new();
        for s in &list.sections {
            let Some(e) = EnhancementDef::from_tdf(s) else { continue };
            if out.iter().any(|o| o.id.eq_ignore_ascii_case(&e.id)) {
                log::warn!("{}: enhancement {} given twice; the first is kept", info.get("UnitName").unwrap_or("?"), e.id);
                continue;
            }
            if out.len() >= MAX_ENHANCEMENTS {
                log::warn!("{}: more than {MAX_ENHANCEMENTS} enhancements; the rest are left out", info.get("UnitName").unwrap_or("?"));
                break;
            }
            out.push(e);
        }
        out
    }

    /// Whether it fills or swaps any weapon slot.
    pub fn arms(&self) -> bool {
        self.weapons.iter().any(Option::is_some)
    }

    /// Whether it lets the unit cloak.
    pub fn cloaks(&self) -> bool {
        self.cloak_cost > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::tdf;

    #[test]
    fn a_units_enhancements_are_read_in_their_order() {
        let doc = tdf::parse(
            "[UNITINFO]{UnitName=ACU;\n[ENHANCEMENTS]{\n[AdvancedEngineering]{Name=Tech 2 Engineering Suite;Slot=LCH;BuildCostMetal=720;BuildCostEnergy=18000;BuildTime=9000;\
             WorkerTime=200;MaxDamage=3000;HealRate=20;BuildOptions=ueb0201, UEB0202;}\n[T3Engineering]{Slot=LCH;Prerequisite=AdvancedEngineering;BuildTime=0;}\n\
             [Missile]{Name=Tactical Missile Launcher;Slot=Back;Weapon4=acu_missile;CloakCost=-5;Stealth=1;}\n[missile]{Slot=RCH;}\n}}",
        );
        let info = doc.section("UNITINFO").unwrap();
        let e = EnhancementDef::of_unit(info);
        assert_eq!(e.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["AdvancedEngineering", "T3Engineering", "Missile"], "in order, an id given twice once");
        assert_eq!((e[0].name.as_str(), e[0].slot.as_str(), e[0].build_cost_metal, e[0].build_cost_energy, e[0].build_time), ("Tech 2 Engineering Suite", "LCH", 720.0, 18000.0, 9000.0));
        assert_eq!((e[0].worker_time, e[0].max_damage, e[0].heal_rate), (200.0, 3000.0, 20.0));
        assert_eq!(e[0].build_options, ["UEB0201", "UEB0202"], "upper case, by spaces or commas");
        assert_eq!((e[1].name.as_str(), e[1].prerequisite.as_str(), e[1].build_time), ("T3Engineering", "AdvancedEngineering", 1.0), "named by its id; a tick of building at least");
        assert_eq!(e[2].weapons[3].as_deref(), Some("ACU_MISSILE"));
        assert!(e[2].arms() && !e[0].arms());
        assert!(e[2].stealth && !e[2].cloaks(), "no negative costs");
        // A unit without the section has none.
        assert!(EnhancementDef::of_unit(tdf::parse("[UNITINFO]{UnitName=ARMCOM;}").section("UNITINFO").unwrap()).is_empty());
    }

    /// What TA Reborn's abilities add: jamming and stealth fields, remote viewing, a regeneration
    /// aura, production and an upkeep.
    #[test]
    fn an_enhancements_abilities_are_read() {
        let doc = tdf::parse(
            "[UNITINFO]{UnitName=HELPER;\n[ENHANCEMENTS]{\n[Jammer]{Slot=Back;RadarDistanceJam=700;EnergyUse=60;}\n\
             [Allocator]{Slot=Back;MetalMake=12;EnergyMake=900;}\n[Mender]{Slot=Right;RegenAuraRadius=360;RegenAuraShare=0.004;RegenAuraRate=40;}\n\
             [Fields]{Slot=Left;SonarDistanceJam=300;RadarStealthField=500;SonarStealthField=600;RemoteViewRadius=800;RemoteViewEnergy=40;}\n}}",
        );
        let e = EnhancementDef::of_unit(doc.section("UNITINFO").unwrap());
        assert_eq!((e[0].radar_jam, e[0].energy_use), (700.0, 60.0));
        assert_eq!((e[1].metal_make, e[1].energy_make), (12.0, 900.0));
        assert!(e[2].mends_round() && !e[0].mends_round());
        assert_eq!((e[2].regen_aura_radius, e[2].regen_aura_rate, e[2].regen_aura_share), (360.0, 40.0, 0.004));
        assert_eq!((e[3].sonar_jam, e[3].radar_stealth_field, e[3].sonar_stealth_field), (300.0, 500.0, 600.0));
        assert_eq!((e[3].remote_view_radius, e[3].remote_view_energy), (800.0, 40.0));
    }
}
