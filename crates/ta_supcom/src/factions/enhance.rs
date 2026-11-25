//! A commander's enhancements (Forged Alliance's `Enhancements`, the commanders' and the support
//! commanders') as TA Reborn plays them: the `[ENHANCEMENTS]` section of its unit file
//! (docs/MODDING.md), each its slot, prerequisite, price, build time and picture, and its effects
//! as the engine applies them, added to the unit's own. What each does is its blueprint's and its
//! unit script's (`CreateEnhancement`'s branch for it: `crate::behaviour::enhancements`), read as
//! data; nothing is run.
//!
//! - **Health and mending** (`NewHealth`, `NewRegenRate`): added. Along a slot's chain (an
//!   enhancement over the one it needs, as the Seraphim's Enhanced Nano-Repair System over the
//!   Nano-Repair System) they add up, as Forged Alliance's buffs stack; an engineering suite's
//!   replace the one before (its buff does).
//! - **Build power** (`NewBuildRate`, what the suites and the support commanders' fabricators
//!   make it): the difference from its own.
//! - **Build options** (`BuildableCategoryAdds`: the units of its faction with those categories,
//!   what Forged Alliance keeps from a commander until then): added, a suite past the first keeping
//!   the first's.
//! - **Weapons** switched on (a weapon's `EnabledByEnhancement`, or the script's
//!   `SetWeaponEnabledByLabel(.., true)`): the weapon in a slot of its own past the unit's, which a
//!   fresh unit leaves empty; switched off by a later one in the chain (the UEF's Billy takes the
//!   tactical launcher's place), not kept.
//! - **The main gun changed** (`NewMaxRadius`, `NewRateOfFire`, `ZephyrDamageMod`,
//!   `AdditionalDamage`, `NewDamageMod`, `NewDamageRadius`, `NewDamageRadiusMod`): a weapon of its
//!   own in the gun's slot, the gun's with those changes; a longer reach lengthens the Overcharge's
//!   too, as the commanders' scripts do.
//! - **Sight** (`NewVisionRadius`), and an omni sensor's radius (`NewOmniRadius`) as radar and
//!   sonar (TA Reborn has no omni); sonar the script switches on (the Cybran's torpedo tubes).
//! - **Stealth** the script switches on (`RadarStealth`, `SonarStealth`) and **cloak** (`Cloak`,
//!   at its upkeep while cloaked, TA's cloak).
//! - TA Reborn's abilities (`factions::abilities`): **a radar jammer** (`NewJammerRadius`, at its
//!   upkeep), **resource allocation** (`ProductionPerSecondMass`, `ProductionPerSecondEnergy`: what
//!   it makes, its own figures, not added along a chain: the advanced system's are the whole), **a
//!   regeneration aura** (`Radius`, `RegenPerSecond`, `RegenCeiling`), **a stun** the script
//!   switches on for a gun (`ReEnableBuff('STUN')`: the EMP Burst, a weapon of its own in the gun's
//!   slot), **a stunning weapon** it switches on (the Chrono Dampener's pulse), and **a shield**
//!   (`factions::shields`: the commanders' and support commanders' personal shields and the UEF's
//!   shield generator fields, at their upkeep).
//! - **The Personal Teleporter** (the script's `AddCommandCap('RULEUCC_Teleport')`): TA Reborn's
//!   teleport (`CanTeleport`), at Forged Alliance's charge for the unit (`lua/sim/Unit.lua`'s
//!   `InitiateTeleportThread`): its energy `BuildCostMass` times its `Economy.TeleportMassMod` and
//!   `BuildCostEnergy` times its `TeleportEnergyMod` (each 0.01 where none is given), over that
//!   times its `TeleportTimeMod` seconds, wherever it goes.
//!
//! - **Pods** (what the script makes, `CreateUnitHPR`: the UEF commander's two and its support
//!   commander's one): TA Reborn's drones (`[DRONES]`), each pod made at the bone its effects start
//!   at, made again after a loss in its build time at the unit's build rate, at its price (Forged
//!   Alliance takes the enhancement back instead), kept within how far it looks for work; a second
//!   pod over the first keeps both.
//!
//! Left out, with what TA Reborn does not play yet: the Sacrificial System: an enhancement none
//! of whose effects is played is not offered, nor one needing such another. The "Remove" ones are
//! TA Reborn's own order (a slot cleared at once).

use std::fmt::Write as _;

use super::units::{self, Kind, Weapons, WORK};
use super::UNIT_SCALE;
use crate::lua::{Table, Value};
use crate::unit::{UnitBlueprint, WeaponBlueprint, strip_loc};

/// One enhancement as the unit file gives it.
#[derive(Clone, Debug, Default)]
pub struct Enhancement {
    pub id: String,
    pub name: String,
    pub slot: String,
    pub prerequisite: Option<String>,
    pub cost_mass: f64,
    pub cost_energy: f64,
    /// TA's build time (Forged Alliance's times `WORK`).
    pub build_time: f64,
    /// The game's picture of its button (`textures/ui/common/game/<faction>-enhancements/<icon>_btn_up.dds`).
    pub icon: Option<String>,
    pub max_damage: f64,
    pub heal_rate: f64,
    pub worker_time: f64,
    pub sight: f64,
    pub radar: f64,
    pub sonar: f64,
    /// Units added to the menu (their ids).
    pub build_options: Vec<String>,
    /// Slots (from 0) and the weapon (its section's name) each fires while installed.
    pub weapons: Vec<(usize, String)>,
    pub cloak: Option<f64>,
    pub stealth: bool,
    /// What it brings of TA Reborn's abilities (a jammer, resource allocation, an aura).
    pub abilities: super::abilities::EnhancementAbilities,
    /// The teleport it gives (the Personal Teleporter): its charge's energy and seconds.
    pub teleport: Option<(f64, f64)>,
    /// The drones it gives (what its script makes, `CreateUnitHPR`: the UEF commanders' pods).
    pub drones: Vec<units::Drone>,
}

/// The weapon sections an enhancement brings that the unit's own weapons are not: its main gun's
/// and Overcharge's changed, with their names.
#[derive(Clone, Debug, Default)]
pub struct Variants<'a> {
    pub sections: Vec<(String, WeaponBlueprint, &'a UnitBlueprint)>,
}

/// The bones an enhancement shows (its `ShowBones`): hidden on a fresh unit, shown once it is
/// installed (the script's `SetEnhancement`).
pub fn shown_bones(u: &UnitBlueprint, e: &Enhancement) -> Vec<String> {
    let Some(t) = u.table.path(&["Enhancements"]).and_then(|l| l.get_ci(&e.id)).and_then(Value::table) else { return Vec::new() };
    t.get_ci("ShowBones").and_then(Value::table).map(|b| b.array.iter().filter_map(Value::str).map(str::to_string).collect()).unwrap_or_default()
}

/// The picture's name in `unitpics/` (lower case).
pub fn picture_name(u: &UnitBlueprint, e: &Enhancement) -> String {
    format!("{}_{}", u.id, e.id).to_ascii_lowercase()
}

/// The game's path of an enhancement's button picture.
fn icon_path(u: &UnitBlueprint, t: &Table) -> Option<String> {
    let icon = t.get_ci("Icon").and_then(Value::str)?.trim().to_ascii_lowercase();
    if icon.is_empty() || !icon.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let faction = u.faction.name().to_ascii_lowercase();
    Some(format!("textures/ui/common/game/{faction}-enhancements/{icon}_btn_up.dds"))
}

/// The weapons a unit's enhancements switch on, past its own slots, in the order of its
/// blueprint: each usable as its own are ([`units::usable`]) and not among them.
pub fn extra_weapons<'a>(u: &'a UnitBlueprint, own: &Weapons<'a>) -> Vec<&'a WeaponBlueprint> {
    let Some(list) = u.table.path(&["Enhancements"]) else { return Vec::new() };
    let switched_on = |w: &WeaponBlueprint| {
        let by_blueprint = w.table.get_ci("EnabledByEnhancement").and_then(Value::str).is_some_and(|e| list.get_ci(e).is_some());
        let by_script = u.enhancement_scripts.iter().any(|s| s.weapons_on.iter().any(|l| l.eq_ignore_ascii_case(&w.label)));
        by_blueprint || by_script
    };
    let mut out: Vec<&WeaponBlueprint> = Vec::new();
    for w in &u.weapons {
        if own.slots.iter().any(|o| std::ptr::eq(*o, w)) || !switched_on(w) || !units::usable(u, w, true) {
            continue;
        }
        if own.slots.len() + out.len() < units::MAX_WEAPONS {
            out.push(w);
        }
    }
    out
}

/// A unit's teleport charge as Forged Alliance works it out (`lua/sim/Unit.lua`'s
/// `InitiateTeleportThread`): its energy, its `BuildCostMass` times its `Economy.TeleportMassMod`
/// and its `BuildCostEnergy` times its `TeleportEnergyMod` (0.01 each where none is given), and its
/// seconds, that energy times its `TeleportTimeMod` (0.01 where none is given); wherever it goes.
pub fn teleport_charge(u: &UnitBlueprint) -> (f64, f64) {
    let economy = u.table.path(&["Economy"]);
    let factor = |k: &str| economy.and_then(|e| e.get_ci(k)).and_then(Value::num).unwrap_or(0.01).max(0.0);
    let energy = u.cost_mass * factor("TeleportMassMod") + u.cost_energy * factor("TeleportEnergyMod");
    (energy.round(), ((energy * factor("TeleportTimeMod") * 100.0).round() / 100.0).max(0.1))
}

/// A pod an enhancement of `u` makes (`pod`, its blueprint), as its `[DRONES]` gives it: made at
/// `bone`, made again in its build time at the unit's build rate and at its price (Forged Alliance
/// takes the enhancement back with its pod; TA Reborn makes the pod again), kept within how far it
/// looks for work (`AI.GuardScanRadius`).
fn pod_drone(u: &UnitBlueprint, pod: &UnitBlueprint, bone: &str) -> units::Drone {
    let reach = pod.table.path(&["AI"]).and_then(|a| a.get_ci("GuardScanRadius")).and_then(Value::num).unwrap_or(0.0);
    units::Drone {
        id: pod.id.to_ascii_uppercase(),
        count: 1,
        bone: bone.to_string(),
        rebuild: if u.build_rate > 0.0 { (pod.build_time / u.build_rate * 100.0).round() / 100.0 } else { 30.0 },
        mass: pod.cost_mass,
        energy: pod.cost_energy,
        leash: (reach * UNIT_SCALE).round(),
    }
}

/// A number as written in a unit file.
fn n(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{v:.2}") }
}

/// The unit's enhancements as TA Reborn plays them (those with an effect it plays, and whose
/// prerequisite it plays), in the order of their names, and the weapon sections they bring.
/// `faction`: the units of its faction made (build options among them); `own`: its weapons'
/// slots; `extra`: the weapons its enhancements switch on, in the slots after them.
pub fn enhancements<'a>(u: &'a UnitBlueprint, faction: &[&'a UnitBlueprint], own: &Weapons<'a>, extra: &[&'a WeaponBlueprint]) -> (Vec<Enhancement>, Variants<'a>) {
    let mut variants = Variants::default();
    let Some(list) = u.table.path(&["Enhancements"]) else { return (Vec::new(), variants) };
    let mut names: Vec<&str> = list.fields.iter().filter(|(k, v)| v.table().is_some() && !k.eq_ignore_ascii_case("Slots") && !k.to_ascii_lowercase().ends_with("remove")).map(|(k, _)| k.as_str()).collect();
    names.sort_unstable();
    let num = |t: &Table, k: &str| t.get_ci(k).and_then(Value::num);
    let own_sight = u.intel.vision.max(u.intel.water_vision) * UNIT_SCALE;
    let own_radar = if u.birth.intel_off("Radar") { 0.0 } else { u.intel.radar * UNIT_SCALE };
    let own_sonar = if u.birth.intel_off("Sonar") { 0.0 } else { u.intel.sonar * UNIT_SCALE };
    let slot_of = |label: &str| -> Option<usize> {
        own.slots.iter().position(|w| w.label.eq_ignore_ascii_case(label)).or_else(|| extra.iter().position(|w| w.label.eq_ignore_ascii_case(label)).map(|i| own.slots.len() + i))
    };
    let main = own.slots.iter().position(|w| units::kind(w) != Kind::Overcharge);
    // Each one's own effects first, then its chain's (its prerequisite's, in the same slot).
    let mut made: Vec<(Enhancement, bool)> = Vec::new();
    for &name in &names {
        let Some(t) = list.get_ci(name).and_then(Value::table) else { continue };
        let script = u.enhancement_scripts.iter().find(|s| s.name.eq_ignore_ascii_case(name));
        let mut e = Enhancement {
            id: name.to_string(),
            name: strip_loc(t.get_ci("Name").and_then(Value::str).unwrap_or(name)),
            slot: t.get_ci("Slot").and_then(Value::str).unwrap_or("").trim().to_string(),
            prerequisite: t.get_ci("Prerequisite").and_then(Value::str).map(|p| p.trim().to_string()).filter(|p| !p.is_empty()),
            cost_mass: num(t, "BuildCostMass").unwrap_or(0.0).max(0.0),
            cost_energy: num(t, "BuildCostEnergy").unwrap_or(0.0).max(0.0),
            build_time: (num(t, "BuildTime").unwrap_or(1.0) * WORK).round().max(1.0),
            icon: icon_path(u, t),
            ..Default::default()
        };
        let engineering = num(t, "NewBuildRate").is_some();
        e.abilities = super::abilities::enhancement_abilities(t);
        e.max_damage = num(t, "NewHealth").unwrap_or(0.0).max(0.0);
        e.heal_rate = num(t, "NewRegenRate").unwrap_or(0.0).max(0.0);
        if let Some(rate) = num(t, "NewBuildRate") {
            e.worker_time = ((rate - u.build_rate) * WORK).round().max(0.0);
        }
        if let Some(v) = num(t, "NewVisionRadius") {
            e.sight = (v * UNIT_SCALE - own_sight).round().max(0.0);
        }
        if let Some(omni) = num(t, "NewOmniRadius") {
            e.radar = (omni * UNIT_SCALE - own_radar).round().max(0.0);
            e.sonar = (omni * UNIT_SCALE - own_sonar).round().max(0.0);
        }
        if let Some(s) = script {
            let on = |kind: &str| s.intel_on.iter().any(|k| k.eq_ignore_ascii_case(kind));
            if on("Sonar") && own_sonar <= 0.0 {
                e.sonar = e.sonar.max((u.intel.sonar * UNIT_SCALE).round());
            }
            if on("RadarStealth") || on("SonarStealth") {
                e.stealth = true;
            }
            if on("Cloak") {
                e.cloak = Some(num(t, "MaintenanceConsumptionPerSecondEnergy").unwrap_or(0.0).max(1.0));
            }
            if s.command_caps.iter().any(|c| c.eq_ignore_ascii_case("RULEUCC_Teleport")) {
                e.teleport = Some(teleport_charge(u));
            }
            // (Its pods: what it makes of its faction's, made at the bone its effects start at.)
            let bone = t.get_ci("UpgradeEffectBones").and_then(Value::table).and_then(|b| b.array.iter().find_map(Value::str)).unwrap_or("").to_string();
            for made in &s.units_made {
                let Some(pod) = faction.iter().find(|p| p.id.eq_ignore_ascii_case(made)) else { continue };
                match e.drones.iter_mut().find(|d| d.id.eq_ignore_ascii_case(made)) {
                    Some(d) => d.count += 1,
                    None => e.drones.push(pod_drone(u, pod, &bone)),
                }
            }
        }
        if let Some(adds) = t.get_ci("BuildableCategoryAdds").and_then(Value::str) {
            for b in faction.iter().filter(|b| b.matches(adds) && b.id != u.id) {
                e.build_options.push(b.id.to_ascii_uppercase());
            }
        }
        // The weapons it switches on (its blueprint's or its script's), and off.
        for w in extra {
            let by_blueprint = w.table.get_ci("EnabledByEnhancement").and_then(Value::str).is_some_and(|x| x.eq_ignore_ascii_case(name));
            let by_script = script.is_some_and(|s| s.weapons_on.iter().any(|l| l.eq_ignore_ascii_case(&w.label)));
            if (by_blueprint || by_script)
                && let Some(slot) = slot_of(&w.label)
            {
                e.weapons.push((slot, units::weapon_name(u, w)));
            }
        }
        // Own slots its script switches on are its own already (the Overcharge, off until ordered).
        if let Some(s) = script {
            for label in &s.weapons_on {
                if let Some(slot) = own.slots.iter().position(|w| w.label.eq_ignore_ascii_case(label))
                    && u.birth.weapon_off(label)
                    && !e.weapons.iter().any(|(k, _)| *k == slot)
                {
                    e.weapons.push((slot, units::weapon_name(u, own.slots[slot])));
                }
            }
        }
        // The main gun changed: a weapon of its own in its slot (and the Overcharge reaching as far).
        let radius = num(t, "NewMaxRadius");
        let rate = num(t, "NewRateOfFire");
        let damage = ["ZephyrDamageMod", "AdditionalDamage", "NewDamageMod"].iter().find_map(|k| num(t, k));
        let blast = ["NewDamageRadius", "NewDamageRadiusMod"].iter().find_map(|k| num(t, k));
        if let Some(m) = main.filter(|_| radius.is_some() || rate.is_some() || damage.is_some() || blast.is_some()) {
            let mut w = own.slots[m].clone();
            if let Some(r) = radius {
                w.max_radius = r;
            }
            if let Some(r) = rate {
                w.rate_of_fire = r;
            }
            if let Some(d) = damage {
                w.damage = (w.damage + d).max(1.0);
            }
            if let Some(b) = blast {
                w.damage_radius = (w.damage_radius + b).max(0.0);
            }
            let section = format!("{}_{}", units::weapon_name(u, own.slots[m]), name.to_ascii_uppercase());
            e.weapons.retain(|(k, _)| *k != m);
            e.weapons.push((m, section.clone()));
            variants.sections.push((section, w, u));
            if let (Some(r), Some(d)) = (radius, own.dgun) {
                let mut oc = own.slots[d].clone();
                oc.max_radius = r;
                let section = format!("{}_{}", units::weapon_name(u, own.slots[d]), name.to_ascii_uppercase());
                e.weapons.retain(|(k, _)| *k != d);
                e.weapons.push((d, section.clone()));
                variants.sections.push((section, oc, u));
            }
        }
        // A gun's stun its script switches on (the EMP Burst): a weapon of its own in the gun's slot.
        if let Some((slot, section, w)) = super::abilities::stun_variant(u, own, name)
            && !e.weapons.iter().any(|(k, _)| *k == slot)
        {
            e.weapons.push((slot, section.clone()));
            variants.sections.push((section, w, u));
        }
        e.weapons.sort();
        made.push((e, engineering));
    }
    // A chain's: what the one it builds on gives, kept (build options, weapons it does not switch
    // off, stealth, sonar), and its health and mending added unless it is a suite.
    let snapshot: Vec<Enhancement> = made.iter().map(|(e, _)| e.clone()).collect();
    for (e, engineering) in &mut made {
        let mut seen = vec![e.id.clone()];
        let mut prerequisite = e.prerequisite.clone();
        while let Some(p) = prerequisite {
            if seen.iter().any(|s| s.eq_ignore_ascii_case(&p)) {
                break;
            }
            seen.push(p.clone());
            let Some(before) = snapshot.iter().find(|b| b.id.eq_ignore_ascii_case(&p) && b.slot.eq_ignore_ascii_case(&e.slot)) else { break };
            for o in &before.build_options {
                if !e.build_options.contains(o) {
                    e.build_options.push(o.clone());
                }
            }
            let off = u.enhancement_scripts.iter().find(|s| s.name.eq_ignore_ascii_case(&e.id)).map(|s| s.weapons_off.clone()).unwrap_or_default();
            for (slot, w) in &before.weapons {
                let label = own.slots.get(*slot).or_else(|| extra.get(slot - own.slots.len().min(*slot))).map(|w| w.label.clone()).unwrap_or_default();
                if !e.weapons.iter().any(|(k, _)| k == slot) && !off.iter().any(|l| l.eq_ignore_ascii_case(&label)) {
                    e.weapons.push((*slot, w.clone()));
                }
            }
            e.weapons.sort();
            e.stealth |= before.stealth;
            // (A second pod over the first: both.)
            for d in &before.drones {
                match e.drones.iter_mut().find(|o| o.id == d.id) {
                    Some(o) => o.count += d.count,
                    None => e.drones.push(d.clone()),
                }
            }
            e.sonar = e.sonar.max(before.sonar);
            e.radar = e.radar.max(before.radar);
            e.sight = e.sight.max(before.sight);
            if !*engineering {
                e.max_damage += before.max_damage;
                e.heal_rate += before.heal_rate;
            }
            prerequisite = before.prerequisite.clone();
        }
    }
    // Those with an effect played, whose prerequisite is played too.
    let plays = |e: &Enhancement| {
        e.max_damage > 0.0 || e.heal_rate > 0.0 || e.worker_time > 0.0 || e.sight > 0.0 || e.radar > 0.0 || e.sonar > 0.0 || !e.build_options.is_empty() || !e.weapons.is_empty() || e.cloak.is_some() || e.stealth || !e.abilities.is_empty()
            || e.teleport.is_some() || !e.drones.is_empty()
    };
    let mut kept: Vec<Enhancement> = made.into_iter().map(|(e, _)| e).filter(plays).collect();
    loop {
        let before = kept.len();
        let ids: Vec<String> = kept.iter().map(|e| e.id.to_ascii_lowercase()).collect();
        kept.retain(|e| e.prerequisite.as_ref().is_none_or(|p| ids.contains(&p.to_ascii_lowercase())));
        if kept.len() == before {
            break;
        }
    }
    let ids: Vec<String> = kept.iter().map(|e| e.id.to_ascii_uppercase()).collect();
    variants.sections.retain(|(section, ..)| ids.iter().any(|id| section.ends_with(&format!("_{id}"))));
    (kept, variants)
}

/// The `[ENHANCEMENTS]` section of a unit file (empty for none).
pub fn section(u: &UnitBlueprint, list: &[Enhancement]) -> String {
    if list.is_empty() {
        return String::new();
    }
    let mut s = String::from("\t[ENHANCEMENTS]\n\t\t{\n");
    for e in list {
        let _ = writeln!(s, "\t\t[{}]\n\t\t\t{{", e.id);
        let mut put = |k: &str, v: String| {
            let _ = writeln!(s, "\t\t\t{k}={v};");
        };
        put("Name", units::tdf_text(&e.name));
        put("Slot", units::tdf_text(&e.slot));
        if let Some(p) = &e.prerequisite {
            put("Prerequisite", units::tdf_text(p));
        }
        put("BuildCostMetal", n(e.cost_mass));
        put("BuildCostEnergy", n(e.cost_energy));
        put("BuildTime", n(e.build_time));
        if e.icon.is_some() {
            put("Picture", picture_name(u, e));
        }
        for (k, v) in [("MaxDamage", e.max_damage), ("HealRate", e.heal_rate), ("WorkerTime", e.worker_time), ("SightDistance", e.sight), ("RadarDistance", e.radar), ("SonarDistance", e.sonar)] {
            if v > 0.0 {
                put(k, n(v));
            }
        }
        if !e.build_options.is_empty() {
            put("BuildOptions", e.build_options.join(" "));
        }
        for (slot, w) in &e.weapons {
            put(&format!("Weapon{}", slot + 1), w.clone());
        }
        if let Some(cost) = e.cloak {
            put("CloakCost", n(cost));
            put("CloakCostMoving", n(cost));
            put("MinCloakDistance", "64".into());
        }
        if e.stealth {
            put("Stealth", "1".into());
        }
        for (k, v) in super::abilities::enhancement_keys(&e.abilities) {
            put(k, v);
        }
        if let Some((energy, seconds)) = e.teleport {
            put("CanTeleport", "1".into());
            put("TeleportEnergy", n(energy));
            put("TeleportTime", n(seconds));
        }
        s.push_str(&units::drones_section(&e.drones, 3));
        s.push_str("\t\t\t}\n");
    }
    s.push_str("\t\t}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A commander's teleport charge, as Forged Alliance works it out from its economy: the Aeon
    /// commander's 0.03 of its energy and none of its mass, a ten-thousandth of that in seconds;
    /// 0.01 each where its blueprint gives none.
    #[test]
    fn a_teleport_charge_is_worked_out_as_forged_alliance_does() {
        let table = Table { fields: crate::lua::parse("Economy = { TeleportEnergyMod = 0.03, TeleportMassMod = 0, TeleportTimeMod = 0.0001 }").unwrap(), array: Vec::new() };
        let acu = UnitBlueprint { cost_mass: 18000.0, cost_energy: 5_000_000.0, table, ..Default::default() };
        assert_eq!(teleport_charge(&acu), (150_000.0, 15.0));
        let plain = UnitBlueprint { cost_mass: 1000.0, cost_energy: 10_000.0, ..Default::default() };
        assert_eq!(teleport_charge(&plain), (110.0, 1.1));
    }
}
