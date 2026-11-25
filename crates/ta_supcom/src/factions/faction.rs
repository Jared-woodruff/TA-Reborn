//! A faction's files beside its units': its side (`gamedata/sidedata_supcom.tdf`, an add-on to the
//! game's sides, with what its builders build for the computer players), its build menus
//! (`download/supcom_<faction>.tdf`), its wrecks (`features/corpses/supcom_<faction>.tdf`) and its
//! weapons (`weapons/supcom_<faction>.tdf`).

use std::fmt::Write as _;

use super::units::{self, Builds};
use crate::unit::{Faction, Motion, Tech, UnitBlueprint};

/// Build pictures a menu page: TA's, two across and three down.
const PAGE: usize = 6;

/// A faction's commander (the unit whose blueprint says COMMAND), if it has one.
pub fn commander<'a>(units: &[&'a UnitBlueprint]) -> Option<&'a UnitBlueprint> {
    units.iter().copied().find(|u| u.is("COMMAND"))
}

/// A unit's tier, 1 to 3, an experimental 4 and a commander 5.
fn tier(u: &UnitBlueprint) -> u8 {
    match u.tech {
        Some(Tech::T1) | None => 1,
        Some(Tech::T2) => 2,
        Some(Tech::T3) => 3,
        Some(Tech::Experimental) => 4,
        Some(Tech::Commander) => 5,
    }
}

/// What `builder` builds by its lists' categories: its own units, those of every one of its
/// `BuildableCategory` entries; a factory's units alone (the buildings its lists name are its own
/// upgrades, in place: [`upgrade_of`]); a commander's not those Forged Alliance keeps from it
/// until its engineering enhancements (`BUILTBYTIER2COMMANDER`, `BUILTBYTIER3COMMANDER`: the
/// Cybran commander's Engineering Station).
fn listed<'a>(builder: &UnitBlueprint, units: &[&'a UnitBlueprint]) -> Vec<&'a UnitBlueprint> {
    if units::builds(builder) == Builds::No {
        return Vec::new();
    }
    let lists = units::build_lists(builder);
    let factory = units::builds(builder) == Builds::Factory;
    let commander = builder.is("COMMAND");
    units
        .iter()
        .copied()
        .filter(|u| u.id != builder.id && lists.iter().any(|e| u.matches(e)) && !(factory && u.motion == Motion::None))
        .filter(|u| !(commander && (u.is("BUILTBYTIER2COMMANDER") || u.is("BUILTBYTIER3COMMANDER"))))
        .collect()
}

/// The unit Forged Alliance upgrades `u` into in place (its `General.UpgradesTo`), where that is
/// one of `units` (of its faction, made) and `u` has build power to upgrade with (`BuildRate`: a
/// fabricator naming an upgrade has none, and never upgrades in Forged Alliance either). The tech
/// 2 and 3 factories are made so, the one below upgraded: nothing builds them outright.
pub fn upgrade_of<'a>(u: &UnitBlueprint, units: &[&'a UnitBlueprint]) -> Option<&'a UnitBlueprint> {
    let to = u.upgrades_to.as_deref()?;
    if u.build_rate <= 0.0 {
        return None;
    }
    units.iter().copied().find(|t| t.id.eq_ignore_ascii_case(to) && t.id != u.id && t.faction == u.faction)
}

/// What each builder of `units` builds, in the order its menu shows them (see [`builds`]).
pub fn all_builds<'a>(units: &[&'a UnitBlueprint]) -> Vec<(&'a UnitBlueprint, Vec<&'a UnitBlueprint>)> {
    let mut lists: Vec<(&UnitBlueprint, Vec<&UnitBlueprint>)> = units.iter().copied().map(|b| (b, listed(b, units))).collect();
    for (_, built) in &mut lists {
        sort(built);
    }
    lists.retain(|(_, built)| !built.is_empty());
    lists
}

/// What `builder` builds of `units`, in the order its menu shows them: buildings before units,
/// the lower tech first; among buildings economy, factories, defences, then the rest: its lists'
/// units ([`listed`]). (What Forged Alliance makes only by upgrading, its tech 2 and 3 factories,
/// no builder builds: [`upgrade_of`].)
pub fn builds<'a>(builder: &UnitBlueprint, units: &[&'a UnitBlueprint]) -> Vec<&'a UnitBlueprint> {
    all_builds(units).into_iter().find(|(b, _)| b.id == builder.id).map(|(_, built)| built).unwrap_or_default()
}

/// The menu's order: buildings before units, the lower tech first; among buildings economy,
/// factories, defences, then the rest.
fn sort(out: &mut [&UnitBlueprint]) {
    let tech = tier;
    let kind = |u: &UnitBlueprint| -> u8 {
        if u.motion != Motion::None {
            return 10;
        }
        if u.is("MASSEXTRACTION") || u.is("MASSFABRICATION") || u.is("ENERGYPRODUCTION") || u.is("MASSSTORAGE") || u.is("ENERGYSTORAGE") {
            0
        } else if u.is("FACTORY") {
            1
        } else if u.is("DEFENSE") || u.is("DIRECTFIRE") || u.is("ANTIAIR") || u.is("ARTILLERY") || u.is("ANTINAVY") {
            2
        } else {
            3
        }
    };
    out.sort_by(|a, b| (a.motion != Motion::None, tech(a), kind(a), &a.id).cmp(&(b.motion != Motion::None, tech(b), kind(b), &b.id)));
}

/// The sides the side add-on adds: those of the factions that have a commander.
pub fn sides<'a>(factions: &'a [(Faction, Vec<&UnitBlueprint>)]) -> impl Iterator<Item = (&'a Faction, &'a UnitBlueprint)> + 'a {
    factions.iter().filter_map(|(faction, units)| Some((faction, commander(units)?)))
}

/// The side add-on: a side for each faction that has a commander ([`sides`]), with its emblem in
/// the classic interface (`logogaf` and `logo`: [`super::emblem`]'s, its interface keeping TA's
/// plain square where the mod has none), and what each builder builds for the computer players
/// (`[CANBUILD]`).
pub fn side_add_on(factions: &[(Faction, Vec<&UnitBlueprint>)]) -> String {
    let mut s = String::new();
    for (n, (faction, commander)) in sides(factions).enumerate() {
        let core = matches!(faction, Faction::Cybran | Faction::Seraphim);
        let side = units::side(faction);
        let _ = writeln!(
            s,
            "[SIDE{n}]\n\t{{\n\tname={side};\n\tnameprefix={};\n\tcommander={};\n\tintgaf={};\n\tlogogaf={};\n\tlogo={};\n\tenergycolor=208;\n\tmetalcolor=224;\n\t}}",
            units::tdf_text(&commander.id.chars().take(2).collect::<String>()),
            commander.id.to_ascii_uppercase(),
            if core { "CORINT" } else { "ARMINT" },
            super::emblem::GAF,
            super::emblem::entry(side)
        );
    }
    s.push_str("[CANBUILD]\n\t{\n");
    for (_, units) in factions {
        for (builder, built) in all_builds(units) {
            let _ = writeln!(s, "\t[{}]\n\t\t{{", builder.id.to_ascii_uppercase());
            for (k, u) in built.iter().enumerate() {
                let _ = writeln!(s, "\t\tcanbuild{}={};", k + 1, u.id.to_ascii_uppercase());
            }
            s.push_str("\t\t}\n");
        }
    }
    s.push_str("\t}\n");
    s
}

/// The faction's build menus: each builder's entries, six a page from TA's first build page.
pub fn menus(units: &[&UnitBlueprint]) -> String {
    let mut s = String::new();
    let mut n = 0;
    for (builder, built) in all_builds(units) {
        for (k, u) in built.iter().enumerate() {
            let _ = writeln!(
                s,
                "[MENUENTRY{n}]\n\t{{\n\tUNITMENU={};\n\tMENU={};\n\tBUTTON={};\n\tUNITNAME={};\n\t}}",
                builder.id.to_ascii_uppercase(),
                k / PAGE + 2,
                k % PAGE,
                u.id.to_ascii_uppercase()
            );
            n += 1;
        }
    }
    s
}

/// Whether a unit leaves a wreck: all but aircraft whose blueprint gives none.
pub fn leaves_wreck(u: &UnitBlueprint) -> bool {
    u.motion != Motion::Air || u.wreckage.is_some()
}

/// The faction's wrecks: each unit's own model where it fell, worth its wreckage's share of its
/// mass, as hard to clear as its wreckage's share of its health. Metal alone, as TA's wrecks give
/// (TA Reborn pays a wreck's metal alone whatever its file says, `Sim::reclaim_value`): no energy,
/// whatever its `EnergyMult` (no Forged Alliance unit's wreckage gives any). A ship's or a
/// submarine's lies on the sea bed and blocks nothing, as TA's do.
pub fn wrecks(units: &[&UnitBlueprint]) -> String {
    let mut s = String::new();
    for u in units.iter().filter(|u| leaves_wreck(u)) {
        let (fx, fz) = units::footprint(u);
        let w = u.wreckage.unwrap_or(crate::unit::Wreckage { mass: 0.9, energy: 0.0, health: 0.9 });
        let id = u.id.to_ascii_lowercase();
        let height = ((u.size[1].max(0.2)) * super::UNIT_SCALE).round().clamp(4.0, 200.0) as i64;
        let blocking = if matches!(u.motion, Motion::Water | Motion::Sub) { 0 } else { 1 };
        let _ = writeln!(
            s,
            "[{id}_dead]\n\t{{\n\tworld=All Worlds;\n\tdescription={} Wreckage;\n\tcategory=supcom_corpses;\n\tobject={id};\n\tfootprintx={fx};\n\tfootprintz={fz};\n\theight={height};\n\tblocking={blocking};\n\thitdensity=100;\n\tmetal={};\n\tenergy=0;\n\tdamage={};\n\treclaimable=1;\n\tfeaturereclamate=smudge01;\n\tseqnamereclamate=tree1reclamate;\n\t}}",
            // (By the name its unit is shown by: never its id.)
            units::tdf_text(&units::shown_name(u, units)),
            (u.cost_mass * w.mass).round() as i64,
            (u.health * w.health).round().max(1.0) as i64
        );
    }
    s
}

/// The faction's weapons: each unit's in its slots, those its enhancements switch on and change,
/// and its death's.
pub fn weapons(units: &[&UnitBlueprint]) -> String {
    let mut s = String::new();
    for u in units {
        let w = units::weapons(u);
        for weapon in &w.slots {
            s.push_str(&units::weapon_section(u, weapon));
        }
        let extra = super::enhance::extra_weapons(u, &w);
        for weapon in &extra {
            s.push_str(&units::weapon_section(u, weapon));
        }
        // (Its weapon toggle's: `abilities::toggle`.)
        for weapon in super::abilities::toggle_weapons(super::abilities::toggle(u, &w, &extra).as_ref()) {
            s.push_str(&units::weapon_section(u, weapon));
        }
        let (_, variants) = super::enhance::enhancements(u, units, &w, &extra);
        for (name, weapon, owner) in &variants.sections {
            s.push_str(&units::weapon_section_named(owner, weapon, name));
        }
        if let Some(death) = w.death.and_then(|d| units::death_section(u, d)) {
            s.push_str(&death);
        }
        if let Some(blast) = w.kamikaze.and_then(|k| units::kamikaze_section(u, k)) {
            s.push_str(&blast);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit;

    fn unit(id: &str, cats: &str, buildable: &str, motion: &str) -> UnitBlueprint {
        unit::parse(
            id,
            &format!(
                "UnitBlueprint {{ Categories = {{ {cats} }}, General = {{ FactionName = 'UEF' }}, Economy = {{ BuildRate = {}, BuildableCategory = {{ {buildable} }}, BuildCostMass = 10 }}, Defense = {{ Health = 100 }}, Physics = {{ MotionType = '{motion}' }} }}",
                if buildable.is_empty() { 0 } else { 10 }
            ),
        )
        .unwrap()
    }

    /// A commander building a factory and an extractor, the factory a tank: the side added with
    /// its commander, the lists for the computer players, the menus (buildings, economy first).
    #[test]
    fn a_faction_has_a_side_menus_and_wrecks() {
        let acu = unit("UEL0001", "'UEF', 'COMMAND', 'MOBILE'", "'BUILTBYCOMMANDER UEF'", "RULEUMT_Amphibious");
        let factory = unit("UEB0101", "'UEF', 'STRUCTURE', 'FACTORY', 'TECH1', 'BUILTBYCOMMANDER'", "'BUILTBYTIER1FACTORY UEF'", "RULEUMT_None");
        let mex = unit("UEB1103", "'UEF', 'STRUCTURE', 'MASSEXTRACTION', 'TECH1', 'BUILTBYCOMMANDER'", "", "RULEUMT_None");
        let tank = unit("UEL0201", "'UEF', 'MOBILE', 'LAND', 'TECH1', 'BUILTBYTIER1FACTORY'", "", "RULEUMT_Land");
        let all = vec![&acu, &factory, &mex, &tank];
        assert_eq!(builds(&acu, &all).iter().map(|u| u.id.as_str()).collect::<Vec<_>>(), ["UEB1103", "UEB0101"]);
        assert_eq!(builds(&factory, &all).iter().map(|u| u.id.as_str()).collect::<Vec<_>>(), ["UEL0201"]);
        let side = side_add_on(&[(Faction::Uef, all.clone())]);
        let doc = ta_formats::tdf::parse(&side);
        let uef = doc.section("SIDE0").unwrap();
        assert_eq!(uef.get("commander"), Some("UEL0001"));
        assert_eq!((uef.get("logogaf"), uef.get("logo")), (Some("supcomlogos"), Some("UEF32Lt")), "its emblem, in the mod's GAF");
        assert_eq!(doc.section("CANBUILD").and_then(|c| c.sections.iter().find(|b| b.name == "UEB0101")).and_then(|b| b.get("canbuild1")), Some("UEL0201"));
        let menus = ta_formats::tdf::parse(&menus(&all));
        let first = &menus.sections[0];
        assert_eq!((first.get("UNITMENU"), first.get("MENU"), first.get("BUTTON"), first.get("UNITNAME")), (Some("UEL0001"), Some("2"), Some("0"), Some("UEB1103")));
        let wrecks = ta_formats::tdf::parse(&wrecks(&all));
        let striker = wrecks.section("uel0201_dead").unwrap();
        assert_eq!((striker.get("object"), striker.get("metal"), striker.get("energy")), (Some("uel0201"), Some("9"), Some("0")));
    }

    /// Forged Alliance makes its tech 2 and 3 factories only by upgrading the one below: no builder
    /// builds them (not on the menus, nor on the computer players' lists), each the upgrade of the
    /// one below ([`upgrade_of`]; a fabricator naming an upgrade, with no build power, never makes
    /// it). The commander builds none of what Forged Alliance keeps from it until its engineering
    /// enhancements (the Cybran's Engineering Station). Every unit is reached from the commander,
    /// by builders building builders and buildings upgrading.
    #[test]
    fn upgraded_factories_are_made_by_upgrading_alone() {
        let factory = |id: &str, tier: &str, built_by: &str, buildable: &str, to: &str| {
            let mut u = unit(id, &format!("'UEF', 'STRUCTURE', 'FACTORY', 'LAND', '{tier}', '{built_by}'"), buildable, "RULEUMT_None");
            u.upgrades_to = (!to.is_empty()).then(|| to.to_string());
            u
        };
        let acu = unit("UEL0001", "'UEF', 'COMMAND', 'MOBILE'", "'BUILTBYCOMMANDER UEF', 'BUILTBYTIER2COMMANDER UEF'", "RULEUMT_Amphibious");
        let t1 = factory("UEB0101", "TECH1", "BUILTBYTIER1ENGINEER', 'BUILTBYCOMMANDER", "'BUILTBYTIER1FACTORY UEF MOBILE', 'BUILTBYTIER1FACTORY UEF STRUCTURE LAND'", "ueb0201");
        let t2 = factory("UEB0201", "TECH2", "BUILTBYTIER1FACTORY", "'BUILTBYTIER2FACTORY UEF MOBILE', 'BUILTBYTIER2FACTORY UEF STRUCTURE LAND'", "ueb0301");
        let t3 = factory("UEB0301", "TECH3", "BUILTBYTIER2FACTORY", "'BUILTBYTIER3FACTORY UEF MOBILE'", "");
        let e1 = unit("UEL0105", "'UEF', 'MOBILE', 'LAND', 'TECH1', 'ENGINEER', 'BUILTBYTIER1FACTORY', 'BUILTBYTIER2FACTORY', 'BUILTBYTIER3FACTORY'", "'BUILTBYTIER1ENGINEER UEF'", "RULEUMT_Land");
        let e2 = unit("UEL0208", "'UEF', 'MOBILE', 'LAND', 'TECH2', 'ENGINEER', 'BUILTBYTIER2FACTORY', 'BUILTBYTIER3FACTORY'", "'BUILTBYTIER2ENGINEER UEF'", "RULEUMT_Land");
        let station = unit("UEB0104", "'UEF', 'STRUCTURE', 'TECH2', 'BUILTBYCOMMANDER', 'BUILTBYTIER2COMMANDER', 'BUILTBYTIER2ENGINEER'", "", "RULEUMT_None");
        let tank = unit("UEL0303", "'UEF', 'MOBILE', 'LAND', 'TECH3', 'BUILTBYTIER3FACTORY'", "", "RULEUMT_Land");
        let mut fab = unit("UEB1104", "'UEF', 'STRUCTURE', 'TECH2', 'BUILTBYTIER2ENGINEER'", "", "RULEUMT_None");
        fab.upgrades_to = Some("ueb0301".into());
        // The T1 engineer builds the T1 factory, the T2 one both and the station.
        let mut e2 = e2;
        e2.buildable.push("BUILTBYTIER1ENGINEER UEF".into());
        let mut t1 = t1;
        t1.categories.push("BUILTBYTIER1ENGINEER".into());
        let all = vec![&acu, &t1, &t2, &t3, &e1, &e2, &station, &tank, &fab];
        let ids = |b: &UnitBlueprint| builds(b, &all).iter().map(|u| u.id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids(&acu), ["UEB0101"], "a commander's own tier, not the station its enhancements bring");
        assert_eq!(ids(&e1), ["UEB0101"], "a tech 1 engineer: no tech 2 factory");
        assert_eq!(ids(&e2), ["UEB0101", "UEB0104", "UEB1104"], "a tech 2 engineer: no tech 2 or 3 factory");
        assert_eq!(ids(&t2), ["UEL0105", "UEL0208"], "a factory builds units, its upgrade in place");
        assert_eq!(upgrade_of(&t1, &all).map(|u| u.id.as_str()), Some("UEB0201"));
        assert_eq!(upgrade_of(&t2, &all).map(|u| u.id.as_str()), Some("UEB0301"));
        assert!(upgrade_of(&t3, &all).is_none() && upgrade_of(&fab, &all).is_none(), "nothing to upgrade into, or no build power");
        // Every one reached from the commander, by builds and upgrades.
        let mut reached = vec!["UEL0001".to_string()];
        let mut k = 0;
        while k < reached.len() {
            let b = all.iter().find(|u| u.id == reached[k]).unwrap();
            for u in builds(b, &all).into_iter().chain(upgrade_of(b, &all)) {
                if !reached.contains(&u.id) {
                    reached.push(u.id.clone());
                }
            }
            k += 1;
        }
        reached.sort();
        let mut every: Vec<String> = all.iter().map(|u| u.id.clone()).collect();
        every.sort();
        assert_eq!(reached, every);
        // The menus and the computer players' lists say the same.
        let side = side_add_on(&[(Faction::Uef, all.clone())]);
        let doc = ta_formats::tdf::parse(&side);
        let lists = doc.section("CANBUILD").and_then(|c| c.sections.iter().find(|b| b.name == "UEL0105")).unwrap();
        assert_eq!((lists.get("canbuild1"), lists.get("canbuild2")), (Some("UEB0101"), None));
        assert!(!menus(&all).contains("UNITNAME=UEB0201;") && !menus(&all).contains("UNITNAME=UEB0301;"), "on no menu");
    }

    /// A ship's wreck lies on the sea bed and blocks nothing, as TA's do; an aircraft with no
    /// wreckage leaves none (and its file names none).
    #[test]
    fn ships_wrecks_block_nothing() {
        let ship = unit("UES0103", "'UEF', 'MOBILE', 'NAVAL', 'TECH1'", "", "RULEUMT_Water");
        let tank = unit("UEL0201", "'UEF', 'MOBILE', 'LAND', 'TECH1'", "", "RULEUMT_Land");
        let plane = unit("UEA0102", "'UEF', 'MOBILE', 'AIR', 'TECH1'", "", "RULEUMT_Air");
        let doc = ta_formats::tdf::parse(&wrecks(&[&ship, &tank, &plane]));
        assert_eq!(doc.section("ues0103_dead").and_then(|s| s.get("blocking")), Some("0"));
        assert_eq!(doc.section("uel0201_dead").and_then(|s| s.get("blocking")), Some("1"));
        assert!(doc.section("uea0102_dead").is_none() && !leaves_wreck(&plane));
        let none = units::Weapons { slots: Vec::new(), death: None, dgun: None, kamikaze: None };
        assert!(!units::unit_file(&plane, &none, None, &units::Extra::default()).contains("Corpse"));
    }

    /// A side's prefix is its commander's first two letters, whatever letters they are.
    #[test]
    fn a_sides_prefix_takes_two_letters() {
        let acu = unit("ÜEL0001", "'UEF', 'COMMAND', 'MOBILE'", "", "RULEUMT_Amphibious");
        let side = side_add_on(&[(Faction::Uef, vec![&acu])]);
        assert!(side.contains("nameprefix=ÜE;"), "{side}");
    }

    /// A wreck gives metal alone, as TA's do, whatever energy its blueprint's wreckage would give.
    #[test]
    fn a_wreck_gives_no_energy() {
        let mut tank = unit("UEL0201", "'UEF', 'MOBILE', 'LAND', 'TECH1'", "", "RULEUMT_Land");
        tank.cost_energy = 500.0;
        tank.wreckage = Some(crate::unit::Wreckage { mass: 0.9, energy: 0.5, health: 0.9 });
        let wrecks = ta_formats::tdf::parse(&wrecks(&[&tank]));
        let wreck = wrecks.section("uel0201_dead").unwrap();
        assert_eq!((wreck.get("metal"), wreck.get("energy")), (Some("9"), Some("0")));
    }
}
