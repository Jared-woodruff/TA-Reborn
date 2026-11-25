//! A unit's shield (TA Reborn's: docs/MODDING.md, "Shields"), as Supreme Commander's units carry
//! them: keys of its `[UNITINFO]`, or of an enhancement that gives it one. Two kinds:
//!
//! - **A personal shield** (`ShieldRadius` none): it takes the damage the unit takes before its
//!   health does.
//! - **A bubble** (`ShieldRadius` some): a dome round the unit, `ShieldRadius` across and
//!   `ShieldHeight` high, that stops its enemies' shots coming into it from outside; their blasts
//!   burst on it, sparing what is under it.
//!
//! Either has its own hit points (`ShieldHealth`), regains them (`ShieldRegen` a second, from
//! `ShieldRegenDelay` seconds after its last hit), and collapses when they run out, coming back
//! whole `ShieldRechargeTime` seconds later. It takes `ShieldEnergyUse` energy a second while it is
//! switched on, and is down while the unit is switched off or short of energy. TA's own units have
//! none.
//!
//! ```text
//! [UNITINFO]
//! {
//!     UnitName=MYSHIELD;
//!     OnOffable=1;
//!     ActivateWhenBuilt=1;
//!     ShieldHealth=9000;
//!     ShieldRegen=120;
//!     ShieldRechargeTime=15;
//!     ShieldRadius=400;
//!     ShieldHeight=80;
//!     ShieldEnergyUse=200;
//! }
//! ```

use ta_formats::tdf::TdfSection;

/// Seconds after its last hit before a shield regains health, where its file gives none.
pub const REGEN_DELAY: f64 = 1.0;
/// Seconds a collapsed shield stays down, where its file gives none.
pub const RECHARGE_TIME: f64 = 10.0;

/// The keys a shield is given by (for the mod check: one given without `ShieldHealth`).
pub const KEYS: [&str; 7] = ["ShieldHealth", "ShieldRegen", "ShieldRegenDelay", "ShieldRechargeTime", "ShieldRadius", "ShieldHeight", "ShieldEnergyUse"];

/// A shield, as a unit's (or an enhancement's) section gives it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShieldDef {
    /// Its hit points (`ShieldHealth`): none, no shield (the other keys alone do nothing).
    pub health: f64,
    /// Hit points a second it regains while it is up (`ShieldRegen`), from `regen_delay` seconds
    /// after its last hit (`ShieldRegenDelay`, [`REGEN_DELAY`] where none is given).
    pub regen: f64,
    pub regen_delay: f64,
    /// Seconds it stays down after collapsing, before it comes back whole (`ShieldRechargeTime`,
    /// [`RECHARGE_TIME`] where none is given).
    pub recharge_time: f64,
    /// A bubble's radius across (px, `ShieldRadius`); none, a personal shield.
    pub radius: f64,
    /// A bubble's height over the unit (px, `ShieldHeight`: as far down, under the ground); none, as
    /// high as it is wide (a hemisphere).
    pub height: f64,
    /// Energy a second it takes while it is switched on (`ShieldEnergyUse`), up or charging.
    pub energy_use: f64,
}

impl ShieldDef {
    /// The shield a section gives (a unit's `[UNITINFO]`, an enhancement's), if it names any of its
    /// [`KEYS`]: one without `ShieldHealth` has none to play ([`ShieldDef::works`]).
    pub fn of(s: &TdfSection) -> Option<ShieldDef> {
        if !KEYS.iter().any(|k| s.get(k).is_some()) {
            return None;
        }
        let f = |k: &str, d: f64| {
            let v = s.get_f64_or(k, d);
            if v.is_finite() { v.max(0.0) } else { d }
        };
        Some(ShieldDef {
            health: f("ShieldHealth", 0.0),
            regen: f("ShieldRegen", 0.0),
            regen_delay: f("ShieldRegenDelay", REGEN_DELAY),
            recharge_time: f("ShieldRechargeTime", RECHARGE_TIME),
            radius: f("ShieldRadius", 0.0),
            height: f("ShieldHeight", 0.0),
            energy_use: f("ShieldEnergyUse", 0.0),
        })
    }

    /// Whether it is a shield at all: it has hit points.
    pub fn works(&self) -> bool {
        self.health > 0.0
    }

    /// Whether it is a bubble (else a personal shield).
    pub fn is_bubble(&self) -> bool {
        self.radius > 0.0
    }

    /// A bubble's height: its own, else its radius.
    pub fn dome_height(&self) -> f64 {
        if self.height > 0.0 { self.height } else { self.radius }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::tdf;

    fn of(text: &str) -> Option<ShieldDef> {
        ShieldDef::of(tdf::parse(text).section("UNITINFO").unwrap())
    }

    /// A bubble's keys, a personal shield's defaults, keys without hit points, and none at all.
    #[test]
    fn a_shield_is_read_from_its_keys() {
        let bubble = of("[UNITINFO]{UnitName=GEN;ShieldHealth=9000;ShieldRegen=120;ShieldRegenDelay=2;ShieldRechargeTime=15;ShieldRadius=400;ShieldHeight=80;ShieldEnergyUse=200;}").unwrap();
        assert_eq!(bubble, ShieldDef { health: 9000.0, regen: 120.0, regen_delay: 2.0, recharge_time: 15.0, radius: 400.0, height: 80.0, energy_use: 200.0 });
        assert!(bubble.works() && bubble.is_bubble());
        assert_eq!(bubble.dome_height(), 80.0);
        let personal = of("[UNITINFO]{UnitName=BOT;ShieldHealth=1300;ShieldRegen=9;}").unwrap();
        assert!(personal.works() && !personal.is_bubble());
        assert_eq!((personal.regen_delay, personal.recharge_time), (REGEN_DELAY, RECHARGE_TIME));
        let dome = of("[UNITINFO]{UnitName=DOME;ShieldHealth=10;ShieldRadius=300;}").unwrap();
        assert_eq!(dome.dome_height(), 300.0, "as high as it is wide");
        let half = of("[UNITINFO]{UnitName=HALF;ShieldRadius=300;ShieldHealth=-5;}").unwrap();
        assert!(!half.works(), "no hit points, no shield");
        assert!(of("[UNITINFO]{UnitName=ARMPW;MaxDamage=300;}").is_none());
    }
}
