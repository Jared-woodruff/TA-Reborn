//! The upgrades of the mods made for the patched engines, played (a report; ignored: it needs a
//! mod): `TA_TEST_MOD=<absolute mod folder> cargo test --release -p ta_sim --test mod_upgrades --
//! --ignored --nocapture`.
//!
//! Every building and factory of the mod whose menu offers a unit that takes no ground (an upgrade
//! module, as Total Mayhem's geothermal plants and moho mines and TA: Escalation's fusion
//! reactors and towers make; an upgrade, as TA: Escalation's labs make) makes one on a flat map,
//! with resources to spare and building sped up. The report says whether it was made, whether its
//! building took it on (`attach-unit`), and how the building stands after: switched on, armored,
//! its script's statics before and after. `TA_UPGRADES=ARMGEO,ARMALAB` plays those builders alone;
//! `TA_UPGRADE_WATCH=ARMMANNI` then has a unit of that kind made by the builder when it can make one,
//! else born on the upgrade, and another born far off, and reports both one's statics (a unit whose script looks for its side's upgrade as it is
//! born: TA: Escalation's Penetrator, level 2 when born by an `ALL_L2`). `TA_UPGRADE_TRACE=1`
//! follows the builder second by second until the upgrade is made.
//!
//! `mod_shield_report` (TA: Escalation's Aegis, `ARMSHGEN`: its units' scripts look for one of
//! their side's nearby and armour themselves while it stands): `TA_SHIELD=ARMSHGEN` the shield,
//! `TA_SHIELD_UNITS=ARMANNI,ARMLLT` the units put beside it and far off, each one's armour and
//! statics reported after a while, with the shield and once it is gone.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::{Command, Fx, FxVec3, Locomotion, Mutators, PlayerSetup, Sim, UnitTypes};

fn flat_map() -> TntMap {
    let cells = 96 * 96;
    TntMap { width: 96, height: 96, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; cells], features: vec![FEATURE_NONE; cells], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() }
}

/// The game and the mod `TA_TEST_MOD` names, mounted as the game mounts them.
fn game_and_mod() -> Option<ta_vfs::Vfs> {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return None;
    };
    let Ok(dir) = std::env::var("TA_TEST_MOD") else {
        eprintln!("TA_TEST_MOD not set; skipping");
        return None;
    };
    let module = ta_vfs::mods::read_mod(std::path::Path::new(&dir)).expect("TA_TEST_MOD: a mod's folder").expect("TA_TEST_MOD: a mod's folder");
    println!("mod: {} {}", module.manifest.name, module.manifest.version);
    let mounted = ta_vfs::mods::mount(Some(&install.root), None, &[&module], &|m| m.manifest.id.clone());
    mounted.install.unwrap();
    mounted.mods.expect("the mod mounts");
    Some(mounted.vfs)
}

fn sim_for(data: &Arc<GameData>, types: &Arc<UnitTypes>) -> Sim {
    let setup = PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::from_ints(200, 60, 200), metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false };
    // (Metal in the ground everywhere, for the extractors: 20 a cell, as on lava worlds.)
    let info = MapInfo { schemas: vec![ta_data::map::MapSchema { surface_metal: 20.0, ..Default::default() }], ..MapInfo::default() };
    let mut sim = Sim::new(types.clone(), data.clone(), &flat_map(), &info, 0, &[setup], 7);
    sim.set_mutators(Mutators { build_speed: 20, ..Mutators::default() });
    sim
}

/// Resources to spare.
fn fill(sim: &mut Sim) {
    let p = &mut sim.players[0];
    p.base_storage = (Fx::from_int(10_000_000), Fx::from_int(10_000_000));
    p.economy.metal = Fx::from_int(5_000_000);
    p.economy.energy = Fx::from_int(5_000_000);
}

#[test]
#[ignore]
fn mod_upgrades_report() {
    let Some(vfs) = game_and_mod() else { return };
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let only: Vec<String> = std::env::var("TA_UPGRADES").map(|s| s.split(',').map(|n| n.trim().to_ascii_uppercase()).collect()).unwrap_or_default();
    let watch = std::env::var("TA_UPGRADE_WATCH").ok().and_then(|n| types.id(&n.trim().to_ascii_uppercase()));
    let mut builders: Vec<(usize, usize)> = Vec::new();
    for t in &types.types {
        if !t.is_builder || t.locomotion != Locomotion::Static || (!only.is_empty() && !only.contains(&t.name)) {
            continue;
        }
        for &o in &t.build_options {
            if data.units[o].takes_no_ground {
                builders.push((t.id, o));
            }
        }
    }
    println!("{} builders make a unit that takes no ground", builders.len());
    for (builder, product) in builders {
        let (bt, pt) = (types.get(builder), types.get(product));
        let mut sim = sim_for(&data, &types);
        fill(&mut sim);
        let at = FxVec3::from_ints(768, 60, 768);
        let b = sim.spawn_unit(builder, 0, at, 0, true).unwrap();
        for _ in 0..60 {
            fill(&mut sim);
            sim.step(&[]);
        }
        let statics = |sim: &Sim, id| sim.unit(id).and_then(|u| u.cob.as_ref()).map(|c| c.statics.clone()).unwrap_or_default();
        let before = statics(&sim, b);
        let (on_when_built, income_before) = (sim.unit(b).unwrap().activated, (sim.players[0].economy.metal_income.to_f64(), sim.players[0].economy.energy_income.to_f64()));
        sim.step(&[Command::Produce { player: 0, factory: b, type_id: product, count: 1 }]);
        let trace = std::env::var_os("TA_UPGRADE_TRACE").is_some();
        let mut made = None;
        for tick in 0..12_000 {
            fill(&mut sim);
            sim.step(&[]);
            if made.is_none() {
                made = sim.units().find(|u| u.type_id == product && u.is_complete()).map(|u| (u.id, tick));
            }
            if trace && made.is_none() && tick % 30 == 0 {
                let u = sim.unit(b).unwrap();
                let frame = sim.units().find(|m| m.type_id == product).map(|m| (m.build_progress.to_f64(), m.pos.to_f32()));
                println!(
                    "  {:>3} s: on {}, stance {}, job {:?}, queue {:?}, frame {:?}",
                    tick / 30, u.activated, u.in_build_stance, u.build.map(|j| j.target), u.factory_queue, frame
                );
            }
            // (Some seconds more for the scripts to take it on and react.)
            if made.is_some_and(|(_, t)| tick > t + 300) {
                break;
            }
        }
        let u = sim.unit(b).unwrap();
        let product_line = match made {
            Some((m, tick)) => {
                let mu = sim.unit(m).unwrap();
                format!("made at {:.1} s, {}", tick as f64 / 30.0, if mu.carried_by == Some(b) { "taken on" } else if mu.carried_by.is_some() { "carried by another" } else { "standing" })
            }
            None => "not made".to_string(),
        };
        println!(
            "{} ({}) -> {} ({}): {}; builder on {} (when built {}), armored {}, income metal {:.1} energy {:.0}/s (before {:.1}, {:.0}); statics {:?} -> {:?}",
            bt.name, bt.def.display_name, pt.name, pt.def.display_name, product_line, u.activated, on_when_built, u.armored,
            sim.players[0].economy.metal_income.to_f64(), sim.players[0].economy.energy_income.to_f64(), income_before.0, income_before.1, before, statics(&sim, b)
        );
        // A unit of the watched kind made by the builder after its upgrade (else born on the
        // upgrade), and one born far off.
        if let Some(w) = watch {
            let near = if bt.build_options.contains(&w) {
                sim.step(&[Command::Produce { player: 0, factory: b, type_id: w, count: 1 }]);
                let mut next = None;
                for _ in 0..6000 {
                    fill(&mut sim);
                    sim.step(&[]);
                    next = sim.units().find(|u| u.type_id == w && u.is_complete()).map(|u| u.id);
                    if next.is_some() {
                        break;
                    }
                }
                next
            } else {
                let on = made.and_then(|(m, _)| sim.unit(m)).map_or(at, |m| m.pos);
                sim.spawn_unit(w, 0, on, 0, true)
            };
            let far = sim.spawn_unit(w, 0, FxVec3::from_ints(1400, 60, 1400), 0, true);
            for _ in 0..30 {
                fill(&mut sim);
                sim.step(&[]);
            }
            let s = |id: Option<ta_sim::UnitId>| id.map(|id| statics(&sim, id)).unwrap_or_default();
            let how = if bt.build_options.contains(&w) { "made by it" } else { "born on it" };
            println!("    {} {how} {:?}, far off {:?}", types.get(w).name, s(near), s(far));
        }
    }
}

#[test]
#[ignore]
fn mod_shield_report() {
    let Some(vfs) = game_and_mod() else { return };
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let shield = std::env::var("TA_SHIELD").unwrap_or_else(|_| "ARMSHGEN".into()).to_ascii_uppercase();
    let Some(shield) = types.id(&shield) else {
        eprintln!("no {shield} in this mod; skipping");
        return;
    };
    let names = std::env::var("TA_SHIELD_UNITS").unwrap_or_else(|_| "ARMANNI,ARMLLT,ARMHLT,ARMZEUS".into());
    let mut sim = sim_for(&data, &types);
    fill(&mut sim);
    let at = FxVec3::from_ints(768, 60, 768);
    let s = sim.spawn_unit(shield, 0, at, 0, true).unwrap();
    let mut pairs = Vec::new();
    for (i, name) in names.split(',').map(|n| n.trim().to_ascii_uppercase()).enumerate() {
        let Some(t) = types.id(&name) else {
            println!("no {name}");
            continue;
        };
        let offset = 120 + 70 * i as i64;
        let near = sim.spawn_unit(t, 0, FxVec3::from_ints(768 + offset, 60, 768), 0, true).unwrap();
        let far = sim.spawn_unit(t, 0, FxVec3::from_ints(400 + 90 * i as i64, 60, 1450), 0, true).unwrap();
        pairs.push((name, near, far));
    }
    let report = |sim: &Sim, when: &str| {
        println!("{when}:");
        for (name, near, far) in &pairs {
            let state = |id| sim.unit(id).map(|u| (u.armored, u.cob.as_ref().map(|c| c.statics.clone()).unwrap_or_default()));
            println!("  {name}: beside it {:?}; far off {:?}", state(*near), state(*far));
        }
    };
    for _ in 0..30 * 15 {
        fill(&mut sim);
        sim.step(&[]);
    }
    report(&sim, &format!("{} standing", types.get(shield).name));
    // (Taken off the map, not blown up: its blast would take the units beside it.)
    sim.remove_unit(s);
    for _ in 0..30 * 15 {
        fill(&mut sim);
        sim.step(&[]);
    }
    println!("(shield {})", if sim.unit(s).is_none_or(|u| u.dead) { "gone" } else { "still standing" });
    report(&sim, "after");
}
