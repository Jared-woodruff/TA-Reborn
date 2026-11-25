//! TA Reborn's abilities in the factions' files (docs/MODDING.md, "Abilities"; docs/SUPCOM_UNITS.md,
//! "Gameplay"), from what the blueprints and the scripts say ([`crate::abilities`]):
//!
//! - **Minimum ranges** (`minrange`): a weapon's `MinRadius`, ×32 (not a missile defence's: TA
//!   Reborn's interceptors take what comes into their coverage; nor an aircraft's, whose bombs and
//!   passes come as near as its runs take it).
//! - **Stuns** (`paralyzetime`, `paralyzeradius`, `paralyzeonfire`, `paralyzeexempt`): a weapon's
//!   stun buff (`Buffs`, `BuffType = 'STUN'`: its `Duration`, `Radius`, `OnFire`, and whom its
//!   `TargetAllow` and `TargetDisallow` leave out, as TA Reborn's categories name them: `STRUCTURE`
//!   for `MOBILE` alone, `LEVEL3` for `TECH3`, `EXPERIMENTAL`, `COMMANDER` for `COMMAND`), but where
//!   its script switches it off at birth; then the enhancement switching it back on brings a weapon
//!   of its own, the gun with its stun (the Cybran support commander's EMP Burst).
//! - **Jamming and stealth fields** (`RadarDistanceJam`, `RadarStealthField`, `SonarStealthField`):
//!   `JamRadius` (its table's `Max`), `RadarStealthFieldRadius`, `SonarStealthFieldRadius`, ×32,
//!   where its script does not switch them off at birth; their upkeep paid while on.
//! - **Remote viewing** (`RemoteViewRadius`, `RemoteViewEnergy`): `Intel.RemoteViewingRadius` ×32, at
//!   its upkeep (`MaintenanceConsumptionPerSecondEnergy`), where its script does not switch it off.
//! - **Weapon toggles** (`[TOGGLE]`): the weapons its script's weapon toggle switches on, in slots
//!   of their own after the unit's and its enhancements', those it switches off emptied.
//! - **What enhancements bring**: a jammer (`NewJammerRadius`, at the enhancement's upkeep), a
//!   resource allocation system (`ProductionPerSecondMass`, `ProductionPerSecondEnergy`), a
//!   regeneration aura (`Radius`, `RegenPerSecond` a share of each unit's most health a second,
//!   `RegenCeiling` the most a second) and a shield (`ShieldMaxHealth` and the rest, at the
//!   enhancement's upkeep: `factions::shields`).

use std::fmt::Write as _;

use super::UNIT_SCALE;
use super::units::{self, Kind, MAX_WEAPONS, Weapons};
use crate::lua::{Table, Value};
use crate::unit::{Motion, Tech, UnitBlueprint, WeaponBlueprint};

/// A weapon's stun as its blueprint gives it (its `Buffs`' `BuffType = 'STUN'`).
#[derive(Clone, Debug, PartialEq)]
pub struct StunBuff {
    pub seconds: f64,
    /// Ogrids round where it goes off (0: the unit it hits alone).
    pub radius: f64,
    /// It goes off round its unit as it fires (`Add = { OnFire = true }`).
    pub on_fire: bool,
    pub allow: String,
    pub disallow: String,
}

/// A weapon's stun buff, if it has one.
pub fn stun_buff(w: &WeaponBlueprint) -> Option<StunBuff> {
    let buffs = w.table.get_ci("Buffs").and_then(Value::table)?;
    let b = buffs.array.iter().filter_map(Value::table).find(|b| b.get_ci("BuffType").and_then(Value::str).is_some_and(|t| t.eq_ignore_ascii_case("STUN")))?;
    let num = |k: &str| b.get_ci(k).and_then(Value::num).unwrap_or(0.0);
    let text = |k: &str| b.get_ci(k).and_then(Value::str).unwrap_or("").to_string();
    let seconds = num("Duration");
    let on_fire = b.get_ci("Add").and_then(Value::table).and_then(|a| a.get_ci("OnFire")).and_then(Value::bool).unwrap_or(false);
    (seconds > 0.0).then(|| StunBuff { seconds, radius: num("Radius").max(0.0), on_fire, allow: text("TargetAllow"), disallow: text("TargetDisallow") })
}

/// Whether a weapon stuns (it takes a slot for its stun though it deals no damage: the Chrono
/// Dampener's point a shot).
pub fn stuns(w: &WeaponBlueprint) -> bool {
    stun_buff(w).is_some()
}

/// TA Reborn's categories of the units a stun spares (`paralyzeexempt`), from Forged Alliance's that
/// it allows and disallows.
fn exempt(b: &StunBuff) -> String {
    let words = |list: &str| list.split([',', ' ']).map(|w| w.trim().to_ascii_uppercase()).filter(|w| !w.is_empty()).collect::<Vec<String>>();
    let (allow, disallow) = (words(&b.allow), words(&b.disallow));
    let mut out: Vec<&str> = Vec::new();
    let mut add = |c: &'static str| {
        if !out.contains(&c) {
            out.push(c);
        }
    };
    if !allow.is_empty() && !allow.iter().any(|a| a == "ALLUNITS") {
        if allow.iter().all(|a| a == "MOBILE") {
            add("STRUCTURE");
        } else if allow.iter().all(|a| a == "TECH1" || a == "TECH2") {
            add("LEVEL3");
            add("COMMANDER");
        }
    }
    for d in &disallow {
        match d.as_str() {
            "TECH3" => add("LEVEL3"),
            "EXPERIMENTAL" => add("EXPERIMENTAL"),
            "COMMAND" => add("COMMANDER"),
            "STRUCTURE" => add("STRUCTURE"),
            _ => {}
        }
    }
    out.join(" ")
}

/// Whether weapon `w`'s stun is on in its section `name`: on at birth, or the section is the
/// variant of an enhancement that switches it on (`<weapon>_<ENHANCEMENT>`).
fn stun_on_in(u: &UnitBlueprint, w: &WeaponBlueprint, name: &str) -> bool {
    !u.abilities.stun_off(&w.label) || u.abilities.stun_on.iter().any(|(e, label)| label.eq_ignore_ascii_case(&w.label) && name.to_ascii_uppercase().ends_with(&format!("_{}", e.to_ascii_uppercase())))
}

/// A weapon section's lines (`\tkey=value;`) for TA Reborn's abilities: its minimum range, and its
/// stun where it is on in the section `name`.
pub fn weapon_keys(u: &UnitBlueprint, w: &WeaponBlueprint, name: &str) -> String {
    let mut s = String::new();
    let mut put = |k: &str, v: String| {
        let _ = writeln!(s, "\t{k}={v};");
    };
    let k = units::kind(w);
    let range = if k == Kind::Nuke { i64::MAX } else { (w.max_radius.max(1.0) * UNIT_SCALE).round() as i64 };
    let min = (w.min_radius * UNIT_SCALE).round() as i64;
    // (Not an aircraft's: TA Reborn's planes let their bombs go, and fire on their passes, as near
    // as their runs take them.)
    if min > 0 && min < range && k != Kind::AntiMissile && u.motion != Motion::Air {
        put("minrange", min.to_string());
    }
    if let Some(b) = stun_buff(w).filter(|_| stun_on_in(u, w, name)) {
        put("paralyzetime", number(b.seconds));
        // (The unit it hits alone, where it names no reach.)
        put("paralyzeradius", ((b.radius * UNIT_SCALE).round() as i64).max(1).to_string());
        if b.on_fire {
            put("paralyzeonfire", "1".into());
        }
        let spares = exempt(&b);
        if !spares.is_empty() {
            put("paralyzeexempt", spares);
        }
    }
    s
}

/// A number as a TDF value: whole where it is, else to two places.
fn number(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{v:.2}") }
}

/// Pixels of `ogrids`.
fn px(ogrids: f64) -> i64 {
    (ogrids * UNIT_SCALE).round() as i64
}

/// Whether its jamming is played (it has some, not switched off as it is made).
fn jams(u: &UnitBlueprint) -> bool {
    u.intel.jam > 0.0 && !u.birth.intel_off("Jammer")
}

/// Its stealth fields as played (radar, sonar: px, 0 none).
fn fields(u: &UnitBlueprint) -> (i64, i64) {
    let on = |kind: &str, r: f64| if r > 0.0 && !u.birth.intel_off(kind) { px(r) } else { 0 };
    (on("RadarStealthField", u.intel.radar_field), on("SonarStealthField", u.intel.sonar_field))
}

/// Whether it views from afar (its remote viewing not switched off by its script).
fn views(u: &UnitBlueprint) -> bool {
    u.intel.remote_view > 0.0 && !u.abilities.remote_view_off
}

/// Whether its upkeep (`MaintenanceConsumptionPerSecondEnergy`) pays for an ability TA Reborn plays:
/// its jamming or its stealth fields (a counter-intelligence unit's). Remote viewing's is its own
/// (`RemoteViewEnergy`).
pub fn upkeep_pays(u: &UnitBlueprint) -> bool {
    let (radar, sonar) = fields(u);
    !views(u) && (jams(u) || radar > 0 || sonar > 0)
}

/// A unit file's keys for TA Reborn's abilities: its jamming, stealth fields and remote viewing.
pub fn unit_keys(u: &UnitBlueprint) -> Vec<(&'static str, String)> {
    let mut s = Vec::new();
    let mut put = |k: &'static str, v: String| s.push((k, v));
    if jams(u) {
        put("RadarDistanceJam", px(u.intel.jam).to_string());
    }
    let (radar, sonar) = fields(u);
    if radar > 0 {
        put("RadarStealthField", radar.to_string());
    }
    if sonar > 0 {
        put("SonarStealthField", sonar.to_string());
    }
    if views(u) {
        put("RemoteViewRadius", px(u.intel.remote_view).to_string());
        put("RemoteViewEnergy", number(u.upkeep_energy.max(0.0)));
    }
    s
}

/// Categories its file carries for the abilities: what stuns spare (`STRUCTURE`, `EXPERIMENTAL`).
pub fn categories(u: &UnitBlueprint) -> Vec<&'static str> {
    let mut out = Vec::new();
    if u.motion == Motion::None {
        out.push("STRUCTURE");
    }
    if u.tech == Some(Tech::Experimental) {
        out.push("EXPERIMENTAL");
    }
    out
}

/// A unit's weapon toggle as its file gives it: its name, the slots it empties (its own weapons its
/// script switches off) and the weapons in slots of their own it fills.
#[derive(Clone, Debug)]
pub struct Toggle<'a> {
    pub name: String,
    pub off: Vec<usize>,
    pub on: Vec<(usize, &'a WeaponBlueprint)>,
}

/// A label's words: `SniperGun` "Sniper Gun", `AAGun` "AA Gun".
fn words_of(label: &str) -> String {
    let chars: Vec<char> = label.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        // (A capital after a small letter, or the last of a run of capitals before a small one.)
        let before = i.checked_sub(1).map(|j| chars[j]);
        let after = chars.get(i + 1).copied();
        let starts = c.is_ascii_uppercase() && before.is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b.is_ascii_uppercase() && after.is_some_and(|a| a.is_ascii_lowercase()));
        if starts {
            out.push(' ');
        }
        out.push(if c == '_' { ' ' } else { c });
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Its weapon toggle, where its script has one switching a weapon on: in the slots after its own
/// (`own`) and its enhancements' (`extra`), as many as there is room for.
pub fn toggle<'a>(u: &'a UnitBlueprint, own: &Weapons<'a>, extra: &[&'a WeaponBlueprint]) -> Option<Toggle<'a>> {
    let wt = u.abilities.toggle.as_ref()?;
    let taken: Vec<&WeaponBlueprint> = own.slots.iter().chain(extra).copied().collect();
    let off: Vec<usize> = taken.iter().enumerate().filter(|(_, w)| wt.off.iter().any(|l| l.eq_ignore_ascii_case(&w.label))).map(|(k, _)| k).collect();
    let mut on = Vec::new();
    for label in &wt.on {
        let slot = taken.len() + on.len();
        if slot >= MAX_WEAPONS {
            break;
        }
        if let Some(w) = u.weapons.iter().find(|w| w.label.eq_ignore_ascii_case(label) && !taken.iter().any(|t| std::ptr::eq(*t, *w)) && units::usable(u, w, true)) {
            on.push((slot, w));
        }
    }
    let first = on.first()?.1;
    Some(Toggle { name: words_of(&first.label), off, on })
}

/// The weapons a toggle fills its slots with, in order.
pub fn toggle_weapons<'a>(toggle: Option<&Toggle<'a>>) -> Vec<&'a WeaponBlueprint> {
    toggle.map(|t| t.on.iter().map(|(_, w)| *w).collect()).unwrap_or_default()
}

/// The `[TOGGLE]` section of a unit's file, and the keys its slots take beside the unit's own (a
/// weapon's bad targets and only targets, as `units::unit_file` gives its own slots theirs).
pub fn toggle_section(u: &UnitBlueprint, toggle: &Toggle) -> (String, Vec<(String, String)>) {
    let mut s = String::from("\t[TOGGLE]\n\t\t{\n");
    let _ = writeln!(s, "\t\tName={};", units::tdf_text(&toggle.name));
    let mut slots: Vec<(usize, String)> = toggle.off.iter().map(|&k| (k, String::new())).chain(toggle.on.iter().map(|&(k, w)| (k, units::weapon_name(u, w)))).collect();
    slots.sort();
    for (k, w) in slots {
        let _ = writeln!(s, "\t\tWeapon{}={w};", k + 1);
    }
    s.push_str("\t\t}\n");
    let keys = toggle.on.iter().flat_map(|&(slot, w)| slot_keys(slot, w)).collect();
    (s, keys)
}

/// What a slot of weapon `w` takes in its unit's file, as the unit's own slots take it: aircraft
/// last where Forged Alliance shoots none and TA Reborn's weapon could hit them; aircraft alone
/// for anti-air that takes on nothing else (`units::aircraft_alone`).
fn slot_keys(slot: usize, w: &WeaponBlueprint) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let k = units::kind(w);
    let hits_air_in_ta = matches!(k, Kind::Laser | Kind::Missile | Kind::Overcharge) || k == Kind::Shell && w.ballistic_arc.eq_ignore_ascii_case("RULEUBA_None");
    if !(units::shoots_air(w) || k == Kind::AntiAir) && hits_air_in_ta {
        let key = match slot {
            0 => "wpri_badTargetCategory".to_string(),
            1 => "wsec_badTargetCategory".to_string(),
            2 => "wspe_badTargetCategory".to_string(),
            n => format!("BadTargetCategory{}", n + 1),
        };
        out.push((key, "VTOL".to_string()));
    }
    if units::aircraft_alone(w) {
        out.push((format!("OnlyTargetCategory{}", slot + 1), "VTOL".to_string()));
    }
    out
}

/// What an enhancement brings of TA Reborn's abilities (beside what `factions::enhance` reads).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnhancementAbilities {
    /// A jammer's radius (px) and its upkeep (energy a second).
    pub radar_jam: i64,
    pub energy_use: f64,
    /// What it makes a second (resource allocation).
    pub metal_make: f64,
    pub energy_make: f64,
    /// A regeneration aura: its radius (px), the most it mends a second, its share of each unit's
    /// most health a second.
    pub aura: Option<(i64, f64, f64)>,
    /// A shield (`factions::shields`), and its upkeep (energy a second, the enhancement's
    /// `MaintenanceConsumptionPerSecondEnergy`), taken while it is switched on.
    pub shield: Option<super::shields::ShieldBlueprint>,
    pub shield_energy: f64,
}

impl EnhancementAbilities {
    pub fn is_empty(&self) -> bool {
        *self == EnhancementAbilities::default()
    }
}

/// What an enhancement's blueprint `t` brings of the abilities.
pub fn enhancement_abilities(t: &Table) -> EnhancementAbilities {
    let num = |k: &str| t.get_ci(k).and_then(Value::num).unwrap_or(0.0);
    let jam = num("NewJammerRadius");
    let aura = (t.get_ci("RegenPerSecond").is_some() && num("Radius") > 0.0).then(|| (px(num("Radius")), num("RegenCeiling").max(0.0), num("RegenPerSecond").max(0.0)));
    let shield = super::shields::of_enhancement(t);
    let upkeep = num("MaintenanceConsumptionPerSecondEnergy").max(0.0);
    EnhancementAbilities {
        radar_jam: if jam > 0.0 { px(jam) } else { 0 },
        // (Its upkeep, where it pays for the jammer TA Reborn plays; a shield's is the shield's.)
        energy_use: if jam > 0.0 && shield.is_none() { upkeep } else { 0.0 },
        metal_make: num("ProductionPerSecondMass").max(0.0),
        energy_make: num("ProductionPerSecondEnergy").max(0.0),
        aura: aura.filter(|&(_, rate, share)| rate > 0.0 || share > 0.0),
        shield_energy: if shield.is_some() { upkeep } else { 0.0 },
        shield,
    }
}

/// An enhancement's lines in its section of the unit file.
pub fn enhancement_keys(a: &EnhancementAbilities) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if a.radar_jam > 0 {
        out.push(("RadarDistanceJam", a.radar_jam.to_string()));
    }
    if a.energy_use > 0.0 {
        out.push(("EnergyUse", number(a.energy_use)));
    }
    if a.metal_make > 0.0 {
        out.push(("MetalMake", number(a.metal_make)));
    }
    if a.energy_make > 0.0 {
        out.push(("EnergyMake", number(a.energy_make)));
    }
    if let Some((radius, rate, share)) = a.aura {
        out.push(("RegenAuraRadius", radius.to_string()));
        if rate > 0.0 {
            out.push(("RegenAuraRate", number(rate)));
        }
        if share > 0.0 {
            out.push(("RegenAuraShare", format!("{share:.5}")));
        }
    }
    if let Some(shield) = &a.shield {
        out.extend(shield.keys(a.shield_energy));
    }
    out
}

/// The weapon of its own an enhancement `name` brings by switching the stun of one of the unit's
/// own weapons on (its slot, its section's name, the weapon), where its script does so.
pub fn stun_variant<'a>(u: &'a UnitBlueprint, own: &Weapons<'a>, name: &str) -> Option<(usize, String, WeaponBlueprint)> {
    let label = u.abilities.stun_on(name)?;
    let slot = own.slots.iter().position(|w| w.label.eq_ignore_ascii_case(label) && stuns(w))?;
    let section = format!("{}_{}", units::weapon_name(u, own.slots[slot]), name.to_ascii_uppercase());
    Some((slot, section, own.slots[slot].clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit;

    fn gun(extra: &str) -> WeaponBlueprint {
        let bp = unit::parse("TST0101", &format!("UnitBlueprint {{ Categories = {{ 'CYBRAN' }}, Weapon = {{ {{ Label = 'ArmGun', WeaponCategory = 'Direct Fire', Damage = 50, MaxRadius = 20, MinRadius = 2, RateOfFire = 1, {extra} }} }} }}")).unwrap();
        bp.weapons[0].clone()
    }

    /// A weapon's minimum range and its stun: its time, reach, whom it spares; the stun off in its
    /// section where its script switches it off at birth, on in an enhancement's variant.
    #[test]
    fn a_weapons_minimum_range_and_stun() {
        let w = gun("Buffs = { { Add = { OnImpact = true }, BuffType = 'STUN', Duration = 4, Radius = 2, TargetAllow = 'MOBILE', TargetDisallow = 'EXPERIMENTAL,COMMAND' } }");
        let mut u = UnitBlueprint { id: "TST0101".into(), weapons: vec![w.clone()], ..Default::default() };
        let keys = weapon_keys(&u, &w, "TST0101_ARMGUN");
        for line in ["minrange=64;", "paralyzetime=4;", "paralyzeradius=64;", "paralyzeexempt=STRUCTURE EXPERIMENTAL COMMANDER;"] {
            assert!(keys.contains(line), "{line} in {keys}");
        }
        assert!(!keys.contains("paralyzeonfire"));
        u.abilities.stun_off = vec!["ArmGun".into()];
        u.abilities.stun_on = vec![("Shock".into(), "ArmGun".into())];
        assert!(!weapon_keys(&u, &w, "TST0101_ARMGUN").contains("paralyzetime"), "off at birth");
        assert!(weapon_keys(&u, &w, "TST0101_ARMGUN_SHOCK").contains("paralyzetime=4;"), "on in the enhancement's own");
        // A pulse as it fires, sparing the higher tiers (of a unit whose script leaves it on); none at all.
        let pulse = gun("Buffs = { { Add = { OnFire = true }, BuffType = 'STUN', Duration = 2.5, Radius = 10, TargetAllow = 'TECH1,TECH2' } }");
        let u = UnitBlueprint { id: "TST0001".into(), weapons: vec![pulse.clone()], ..Default::default() };
        let keys = weapon_keys(&u, &pulse, "X");
        assert!(keys.contains("paralyzeonfire=1;") && keys.contains("paralyzetime=2.50;") && keys.contains("paralyzeradius=320;") && keys.contains("paralyzeexempt=LEVEL3 COMMANDER;"), "{keys}");
        assert!(stuns(&pulse) && !stuns(&gun("")));
        let mut past = gun("");
        past.min_radius = 30.0;
        assert!(!weapon_keys(&u, &past, "X").contains("minrange"), "a minimum range past its range is none");
    }

    #[test]
    fn labels_read_as_words() {
        assert_eq!(words_of("HeavyRifle"), "Heavy Rifle");
        assert_eq!(words_of("SurfaceCannon"), "Surface Cannon");
        assert_eq!(words_of("AA_Gun"), "AA Gun");
        assert_eq!(words_of("AAGun"), "AA Gun");
        assert_eq!(words_of("PulseEmitter"), "Pulse Emitter");
    }

    /// An enhancement's jammer at its upkeep, resource allocation and regeneration aura.
    #[test]
    fn what_enhancements_bring() {
        let table = |text: &str| crate::lua::parse(&format!("t = {{ {text} }}")).unwrap()[0].1.table().cloned().unwrap();
        let jam = enhancement_abilities(&table("NewJammerRadius = 20, MaintenanceConsumptionPerSecondEnergy = 60"));
        assert_eq!((jam.radar_jam, jam.energy_use), (640, 60.0));
        let ras = enhancement_abilities(&table("ProductionPerSecondEnergy = 900, ProductionPerSecondMass = 12"));
        assert_eq!((ras.metal_make, ras.energy_make, ras.energy_use), (12.0, 900.0, 0.0));
        let aura = enhancement_abilities(&table("Radius = 12, RegenCeiling = 40, RegenFloor = 0, RegenPerSecond = 0.0043333333"));
        assert_eq!(aura.aura, Some((384, 40.0, 0.0043333333)));
        let keys = enhancement_keys(&aura);
        assert_eq!(keys, [("RegenAuraRadius", "384".to_string()), ("RegenAuraRate", "40".to_string()), ("RegenAuraShare", "0.00433".to_string())]);
        // A shield, at its upkeep: a personal one where it names no size.
        let shield = enhancement_abilities(&table("ShieldMaxHealth = 2500, ShieldRegenRate = 20, ShieldRechargeTime = 150, MaintenanceConsumptionPerSecondEnergy = 300"));
        assert!(!shield.is_empty() && shield.energy_use == 0.0 && shield.shield_energy == 300.0);
        let keys = enhancement_keys(&shield);
        assert!(keys.contains(&("ShieldHealth", "2500".to_string())) && keys.contains(&("ShieldEnergyUse", "300".to_string())) && !keys.iter().any(|(k, _)| *k == "ShieldRadius"), "{keys:?}");
    }
}
