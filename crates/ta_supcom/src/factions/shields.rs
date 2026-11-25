//! Forged Alliance's shields as TA Reborn's (docs/MODDING.md, "Shields"; docs/SUPCOM_UNITS.md,
//! "Gameplay"), read from the blueprints: a unit's `Defense.Shield` (made as its unit is finished,
//! where its `ShieldSize` is some and its `StartOn` not false), and a shield enhancement's own keys
//! (`Shield`, `ShieldHeavy`, `ShieldGeneratorField`):
//!
//! - **Its hit points, regaining and recharge**: `ShieldMaxHealth`, `ShieldRegenRate` (a second) from
//!   `ShieldRegenStartTime` seconds after a hit, `ShieldRechargeTime` (Forged Alliance's own
//!   defaults where one is missing: 250, 1, 5 and 10).
//! - **Personal or a bubble**: a personal shield where Forged Alliance makes one
//!   (`PersonalShield = true`, the units its abilities call a personal shield, and an enhancement
//!   naming no size: the commanders' personal shields), else a bubble. Forged Alliance's bubble is a
//!   ball `ShieldSize` across, its middle `ShieldVerticalOffset` over the unit's foot (under it, for
//!   all of the factions'); TA Reborn's is a dome round the unit's foot, as wide as that ball where
//!   it meets the ground and as high as it rises over it, ×32 both ([`super::UNIT_SCALE`]): the
//!   UEF's tech 2 generator 405 px across and 320 high.
//! - **Its upkeep**: the unit's `MaintenanceConsumptionPerSecondEnergy` (an enhancement's own),
//!   which pays for its shield alone, as `ShieldEnergyUse`, taken while the shield is switched on.
//!   A unit whose shield has a switch (`ToggleCaps`' `RULEUTC_ShieldToggle`: every one of the
//!   factions') is `OnOffable` and on once built: switched off, its shield is down and costs
//!   nothing.
//!
//! The Fatboy's is a bubble as a generator's is, round it as it goes.

use super::UNIT_SCALE;
use crate::lua::{Table, Value};
use crate::unit::UnitBlueprint;

/// A shield as Forged Alliance's blueprints give it (`Defense.Shield`, or an enhancement's keys).
#[derive(Clone, Debug, PartialEq)]
pub struct ShieldBlueprint {
    pub health: f64,
    /// Hit points a second it regains, from `regen_start` seconds after a hit.
    pub regen: f64,
    pub regen_start: f64,
    /// Seconds a collapse keeps it down.
    pub recharge: f64,
    /// A bubble's ball: across (`ShieldSize`, ogrids) and its middle over the unit's foot
    /// (`ShieldVerticalOffset`); none for a personal shield.
    pub ball: Option<(f64, f64)>,
}

impl ShieldBlueprint {
    /// The shield of a table of the shield keys: personal where `personal` says so, else a bubble of
    /// its `ShieldSize` (personal where it gives none).
    fn of(t: &Table, personal: bool) -> Option<ShieldBlueprint> {
        let num = |k: &str| t.get_ci(k).and_then(Value::num).filter(|v| v.is_finite());
        let health = num("ShieldMaxHealth")?;
        let size = num("ShieldSize").unwrap_or(0.0);
        let personal = personal || t.get_ci("PersonalShield").and_then(Value::bool).unwrap_or(false) || size <= 0.0;
        (health > 0.0).then(|| ShieldBlueprint {
            health,
            regen: num("ShieldRegenRate").unwrap_or(1.0).max(0.0),
            regen_start: num("ShieldRegenStartTime").unwrap_or(5.0).max(0.0),
            recharge: num("ShieldRechargeTime").unwrap_or(10.0).max(0.0),
            ball: (!personal).then(|| (size, num("ShieldVerticalOffset").unwrap_or(-1.0))),
        })
    }

    /// A bubble's dome in TA Reborn's pixels: across where Forged Alliance's ball meets the ground
    /// (all of it where its middle is over the ground), and how high it rises there; none for a
    /// personal shield.
    pub fn dome(&self) -> Option<(i64, i64)> {
        let (size, offset) = self.ball?;
        let r = size / 2.0;
        let below = (-offset).max(0.0);
        let across = (r * r - below * below).max(0.0).sqrt();
        let up = r - below;
        (across > 0.0 && up > 0.0).then(|| (((across * UNIT_SCALE).round() as i64).max(1), ((up * UNIT_SCALE).round() as i64).max(1)))
    }

    /// Its keys, as a unit file (or an enhancement's section) gives them, with its upkeep (energy a
    /// second; none, nothing).
    pub fn keys(&self, energy: f64) -> Vec<(&'static str, String)> {
        let mut out = vec![("ShieldHealth", number(self.health))];
        if self.regen > 0.0 {
            out.push(("ShieldRegen", number(self.regen)));
        }
        out.push(("ShieldRegenDelay", number(self.regen_start)));
        out.push(("ShieldRechargeTime", number(self.recharge)));
        if let Some((across, up)) = self.dome() {
            out.push(("ShieldRadius", across.to_string()));
            out.push(("ShieldHeight", up.to_string()));
        }
        if energy > 0.0 {
            out.push(("ShieldEnergyUse", number(energy)));
        }
        out
    }
}

/// A number as a TDF value: whole where it is, else to two places.
fn number(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{v:.2}") }
}

/// A unit's own shield, where Forged Alliance makes it one as it is finished (`Defense.Shield` with
/// a `ShieldSize`, its `StartOn` not false): personal where it says so (`PersonalShield`) or its
/// abilities call it one (the Harbinger's ball, as wide as itself, its overkill passed to it).
pub fn of_unit(u: &UnitBlueprint) -> Option<ShieldBlueprint> {
    let t = u.table.path(&["Defense", "Shield"])?;
    let size = t.get_ci("ShieldSize").and_then(Value::num).unwrap_or(0.0);
    if size <= 0.0 || t.get_ci("StartOn").and_then(Value::bool) == Some(false) {
        return None;
    }
    let called = u.table.path(&["Display", "Abilities"]).is_some_and(|a| a.array.iter().filter_map(Value::str).any(|s| s.to_ascii_lowercase().contains("ability_personalshield")));
    ShieldBlueprint::of(t, called)
}

/// The shield an enhancement's blueprint gives (its `ShieldMaxHealth` and the rest), if any.
pub fn of_enhancement(t: &Table) -> Option<ShieldBlueprint> {
    ShieldBlueprint::of(t, false)
}

/// Whether a unit's own shield has a switch (`General.ToggleCaps`' `RULEUTC_ShieldToggle`): it is
/// `OnOffable`, on once built.
pub fn switchable(u: &UnitBlueprint) -> bool {
    of_unit(u).is_some() && u.table.path(&["General", "ToggleCaps"]).and_then(|c| c.get_ci("RULEUTC_ShieldToggle")).and_then(Value::bool).unwrap_or(false)
}

/// A unit file's keys for its own shield: its numbers, at its upkeep (`MaintenanceConsumptionPerSecondEnergy`,
/// its shield's alone).
pub fn unit_keys(u: &UnitBlueprint) -> Vec<(&'static str, String)> {
    of_unit(u).map(|s| s.keys(u.upkeep_energy.max(0.0))).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit;

    fn blueprint(defense: &str, extra: &str) -> UnitBlueprint {
        unit::parse("TST4202", &format!("UnitBlueprint {{ Categories = {{ 'UEF', 'STRUCTURE' }}, Defense = {{ Health = 1000, MaxHealth = 1000, Shield = {{ {defense} }} }}, {extra} }}")).unwrap()
    }

    /// A generator's bubble at TA Reborn's scale, its switch and upkeep; a personal shield; one not
    /// made at birth; an enhancement's.
    #[test]
    fn shields_are_read_from_the_blueprints() {
        let mut generator = blueprint(
            "ShieldMaxHealth = 9000, ShieldRegenRate = 120, ShieldRegenStartTime = 1, ShieldRechargeTime = 15, ShieldSize = 26, ShieldVerticalOffset = -3",
            "General = { ToggleCaps = { RULEUTC_ShieldToggle = true } }",
        );
        generator.upkeep_energy = 200.0;
        let s = of_unit(&generator).unwrap();
        // (A ball 13 ogrids round, its middle 3 under the ground: 12.65 across there, 10 up.)
        assert_eq!(s.dome(), Some((405, 320)));
        assert!(switchable(&generator));
        let keys = unit_keys(&generator);
        for (k, v) in [("ShieldHealth", "9000"), ("ShieldRegen", "120"), ("ShieldRegenDelay", "1"), ("ShieldRechargeTime", "15"), ("ShieldRadius", "405"), ("ShieldHeight", "320"), ("ShieldEnergyUse", "200")] {
            assert!(keys.contains(&(k, v.to_string())), "{k}={v} in {keys:?}");
        }
        let personal = blueprint("PersonalShield = true, ShieldMaxHealth = 1750, ShieldRegenRate = 2, ShieldSize = 3, StartOn = true", "");
        let s = of_unit(&personal).unwrap();
        assert!(s.dome().is_none() && !unit_keys(&personal).iter().any(|(k, _)| *k == "ShieldRadius"), "personal: no dome");
        assert!(!switchable(&personal), "no switch without its toggle");
        let harbinger = blueprint("ShieldMaxHealth = 1300, ShieldSize = 2.5, ShieldVerticalOffset = 0", "Display = { Abilities = { '<LOC ability_personalshield>Personal Shield' } }");
        assert!(of_unit(&harbinger).unwrap().dome().is_none(), "called a personal shield");
        assert!(of_unit(&blueprint("ShieldMaxHealth = 500, ShieldSize = 10, StartOn = false", "")).is_none(), "not made at birth");
        assert!(of_unit(&blueprint("ShieldMaxHealth = 500", "")).is_none(), "no size, not made at birth");
        let table = |text: &str| crate::lua::parse(&format!("t = {{ {text} }}")).unwrap()[0].1.table().cloned().unwrap();
        let field = of_enhancement(&table("ShieldMaxHealth = 36000, ShieldRegenRate = 42, ShieldRechargeTime = 175, ShieldSize = 16, ShieldVerticalOffset = -3")).unwrap();
        assert_eq!(field.dome(), Some((237, 160)));
        let own = of_enhancement(&table("ShieldMaxHealth = 29000, ShieldRegenRate = 37, ShieldRechargeTime = 160")).unwrap();
        assert!(own.dome().is_none(), "an enhancement's without a size: personal");
        assert!(of_enhancement(&table("NewHealth = 3000")).is_none());
    }
}
