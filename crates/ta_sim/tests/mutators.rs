//! A skirmish's mutators against the real game data (skipped without a TA install): each kind
//! of unit left out takes what it says from the build menus, with the factories for it, and
//! nothing else (`--nocapture` lists what goes).

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Fx, FxVec3, Mutators, PlayerSetup, Sim, UnitTypes};

fn sim() -> Option<Sim> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").ok()?));
    let players: Vec<PlayerSetup> = ["ARM", "CORE"]
        .iter()
        .enumerate()
        .map(|(i, side)| PlayerSetup { side: (*side).into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    Some(Sim::new(types, data, &map, &info, 0, &players, 7))
}

/// The units `mutators` leave out, by name.
fn left_out(mutators: Mutators) -> Option<Vec<String>> {
    let mut sim = sim()?;
    sim.set_mutators(mutators);
    let mut out: Vec<String> = sim.types.types.iter().enumerate().filter(|&(i, _)| !sim.is_allowed(i)).map(|(_, t)| t.name.to_ascii_uppercase()).collect();
    out.sort();
    Some(out)
}

#[test]
fn each_mutator_leaves_out_its_kind_of_unit() {
    let Some(none) = left_out(Mutators::default()) else {
        eprintln!("no TA install: skipped");
        return;
    };
    assert!(none.is_empty(), "{none:?}");
    let check = |what: &str, mutators: Mutators, gone: &[&str], kept: &[&str]| {
        let out = left_out(mutators).unwrap();
        println!("{what}: {}", out.join(" "));
        for g in gone {
            assert!(out.iter().any(|o| o == g), "{what} should leave out {g}: {out:?}");
        }
        for k in kept {
            assert!(!out.iter().any(|o| o == k), "{what} should keep {k}");
        }
    };
    let only = |f: fn(&mut Mutators)| {
        let mut m = Mutators::default();
        f(&mut m);
        m
    };
    check("no aircraft", only(|m| m.no_air = true), &["ARMAP", "CORAP", "ARMAAP", "CORAAP", "ARMCA", "CORCA", "ARMFIG", "CORVAMP", "ARMBRAWL"], &["ARMLAB", "ARMVP", "ARMCOM", "ARMSH", "ARMSY"]);
    check("no ships", only(|m| m.no_naval = true), &["ARMSY", "CORSY", "ARMASY", "CORASY", "ARMCS", "CORCS", "ARMBATS", "CORSUB"], &["ARMLAB", "ARMAP", "ARMSH", "CORSH", "ARMCOM"]);
    check("no nukes", only(|m| m.no_nukes = true), &["ARMSILO", "CORSILO", "ARMEMP", "CORTRON", "ARMAMD", "CORFMD", "ARMSCAB", "CORMABM"], &["ARMGUARD", "CORPUN", "ARMBRTHA", "ARMCOM"]);
    check("no artillery", only(|m| m.no_artillery = true), &["ARMBRTHA", "CORINT", "ARMVULC", "CORBUZZ"], &["ARMGUARD", "CORPUN", "ARMSILO", "ARMAMD", "CORFMD", "ARMMART", "CORMART"]);
    check("no experimentals", only(|m| m.no_experimentals = true), &["CORKROG", "CORGANT"], &["ARMCOM", "CORCOM", "ARMLAB", "ARMALAB", "CORALAB", "CORSUMO", "ARMFARK"]);
}
