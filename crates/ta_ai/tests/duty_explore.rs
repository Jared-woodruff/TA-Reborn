//! Exploration of the duties on real game data, run by hand (every test here is ignored):
//! `cargo test --release -p ta_ai --test duty_explore <name> -- --ignored --nocapture`. They print what the
//! constructors on duty did (orders, stores, the audit's counts) for cases of the QA matrix, to find what the
//! tests of `duty_qa.rs` then pin down.
//!
//! `TA_QA_MAP`, `TA_QA_UNIT` (ARMCOM: the Commander), `TA_QA_DUTY` and `TA_QA_MINUTES` pick the case of `explore_one`.

mod common;

use common::*;
use ta_ai::Duty;
use ta_sim::{Command, Fx, FxVec3, Order, UnitId};

#[test]
#[ignore]
fn list_the_maps() {
    let Some(d) = data() else { return };
    let mut maps: Vec<String> = d.vfs.list("maps", Some("ota"), false).into_iter().filter_map(|p| p.strip_prefix("maps/").and_then(|f| f.strip_suffix(".ota")).map(str::to_string)).collect();
    maps.sort();
    maps.dedup();
    for name in maps {
        let Some(m) = map(&name) else { continue };
        let starts = m.info.schemas.first().map_or(0, |s| s.start_positions.len());
        if starts < 2 {
            continue;
        }
        let Some(scene) = Scene::on(&name, ["ARM", "CORE"], true) else { continue };
        let sea = scene.sim.world.sea_level;
        let (w, h) = scene.sim.world.size_px();
        let start_h = scene.sim.world.height_at(scene.start.x, scene.start.z);
        let foe_h = scene.sim.world.height_at(scene.foe.x, scene.foe.z);
        println!("{name:34} {:>5}x{:<5} starts {starts} sea {:>4} ours at {:>4},{:<4} h {:>4} foe {:>4},{:<4} h {:>4} dist {:>5} planet {} tide {}", w.to_int(), h.to_int(), sea.to_int(), scene.start.x.to_int(), scene.start.z.to_int(), start_h.to_int(), scene.foe.x.to_int(), scene.foe.z.to_int(), foe_h.to_int(), px(scene.start, scene.foe), m.info.planet, m.info.tidal_strength);
    }
}

fn explore(map_name: &str, sides: [&str; 2], units: &[(&str, i64, i64, Duty)], minutes: u64, list: bool) {
    let Some(mut scene) = Scene::on(map_name, sides, true) else { return };
    let ids: Vec<_> = units
        .iter()
        .map(|&(name, dx, dz, duty)| {
            let id = scene.spawn(name, dx, dz);
            scene.officer.set(id, Some(duty));
            id
        })
        .collect();
    println!("{map_name} {sides:?}: {:?} on duty for {minutes} min", units.iter().map(|u| (u.0, u.3.name())).collect::<Vec<_>>());
    let mut log = Vec::new();
    for minute in 0..minutes {
        let ordered = scene.play(60);
        let builds = ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Build { .. }, .. })).count();
        let moves = ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Move(_), .. })).count();
        println!(
            "  min {:>2}: {:>3} orders ({builds} build, {moves} move), metal {:>5} energy {:>6}, income {:.0}/{:.0}, {} extractors {} generators",
            minute + 1,
            ordered.len(),
            scene.metal(),
            scene.energy(),
            scene.sim.players[0].economy.metal_income.to_f32(),
            scene.sim.players[0].economy.energy_income.to_f32(),
            scene.extractors(),
            scene.generators()
        );
        if list {
            for (t, c) in &ordered {
                log.push(format!("    t={:>6} {}", t, words(&scene.sim, c)));
            }
        }
    }
    println!("  census: {:?}", scene.census());
    for id in ids {
        if let Some(u) = scene.sim.unit(id) {
            println!("  unit {} at ({}, {}) orders {:?} duty {:?}", id.slot, u.pos.x.to_int(), u.pos.z.to_int(), u.orders.len(), scene.officer.duty(id));
        }
    }
    if list {
        println!("{}", log.join("\n"));
    }
}

#[test]
#[ignore]
fn explore_economy_sherwood() {
    explore("sherwood", ["ARM", "CORE"], &[("ARMCK", 60, 60, Duty::Economy), ("ARMCOM", 0, 0, Duty::Economy)], 8, false);
}

/// A player with a lab turning out tanks as fast as it can, on small stores, and constructors on a duty
/// (none: under command): how many tanks in `minutes`, and the stores each minute.
fn loaded(map_name: &str, builders: &[(&str, Option<Duty>)], minutes: u64, metal: i64, energy: i64) -> usize {
    let Some(mut scene) = Scene::on(map_name, ["ARM", "CORE"], true) else { return 0 };
    scene.sim.players[0].economy.metal = ta_sim::Fx::from_int(metal);
    scene.sim.players[0].economy.energy = ta_sim::Fx::from_int(energy);
    let lab = scene.spawn("ARMLAB", 0, 140);
    let tank = scene.type_id("ARMPW");
    scene.sim.step(&[Command::Produce { player: 0, factory: lab, type_id: tank, count: 200 }]);
    for (k, &(name, duty)) in builders.iter().enumerate() {
        let id = scene.spawn(name, -80 - 40 * k as i64, 80);
        scene.officer.set(id, duty);
    }
    let mut line = String::new();
    for minute in 0..minutes {
        scene.play(60);
        line += &format!(" [{} m/{} e, {} ex]", scene.metal(), scene.energy(), scene.extractors());
        let eco = &scene.sim.players[0].economy;
        println!("    min {}: income {:.0}m {:.0}e, expense {:.0}m {:.0}e; {:?}", minute + 1, eco.metal_income.to_f32(), eco.energy_income.to_f32(), eco.metal_expense.to_f32(), eco.energy_expense.to_f32(), scene.census());
    }
    let tanks = scene.count("ARMPW");
    println!("  {} builders {:?}: {tanks} tanks;{line}", builders.len(), builders.iter().map(|b| b.1.map(|d| d.name())).collect::<Vec<_>>());
    tanks
}

#[test]
#[ignore]
fn explore_many_builders() {
    for n in [0usize, 2, 4, 8] {
        let builders: Vec<(&str, Option<Duty>)> = (0..n).map(|_| ("ARMCK", Some(Duty::Economy))).collect();
        println!("{n} builders on the economy duty, stores 1000/1000");
        loaded("sherwood", &builders, 8, 1000, 1000);
    }
}

#[test]
#[ignore]
fn explore_loaded_economy() {
    for (metal, energy) in [(300, 300), (100, 100)] {
        println!("stores {metal}/{energy}");
        loaded("sherwood", &[], 6, metal, energy);
        loaded("sherwood", &[("ARMCK", None)], 6, metal, energy);
        loaded("sherwood", &[("ARMCK", Some(Duty::Economy))], 6, metal, energy);
        loaded("sherwood", &[("ARMCK", Some(Duty::Economy)), ("ARMCK", Some(Duty::Economy)), ("ARMCK", Some(Duty::Economy))], 6, metal, energy);
        loaded("sherwood", &[("ARMCK", Some(Duty::Defence)), ("ARMCK", Some(Duty::Defence))], 6, metal, energy);
    }
}

#[test]
#[ignore]
fn explore_commander_economy() {
    // (The scene's own Commander is ours at the start: put it on duty.)
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let com = scene.sim.units().find(|u| u.owner == 0).unwrap().id;
    scene.officer.set(com, Some(Duty::Economy));
    let ordered = scene.play(5 * 60);
    for (t, c) in &ordered {
        println!("  t={:>6} {}", t, words(&scene.sim, c));
    }
    println!("census {:?}", scene.census());
}


/// One case with its orders listed, picked by `TA_QA_MAP`, `TA_QA_UNIT` (ARMCOM: the Commander), `TA_QA_DUTY`, `TA_QA_MINUTES`.
#[test]
#[ignore]
fn explore_one() {
    let map_name = std::env::var("TA_QA_MAP").unwrap_or_else(|_| "sherwood".into());
    let kind = std::env::var("TA_QA_UNIT").unwrap_or_else(|_| "ARMCOM".into());
    let duty = duty_named(&std::env::var("TA_QA_DUTY").unwrap_or_else(|_| "economy".into()));
    let minutes: u64 = std::env::var("TA_QA_MINUTES").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
    let side = if kind.starts_with("COR") { ["CORE", "ARM"] } else { ["ARM", "CORE"] };
    let Some(mut scene) = Scene::on(&map_name, side, true) else { return };
    let id = if kind.ends_with("COM") {
        let com = scene.sim.units().find(|u| u.owner == 0).unwrap().id;
        scene.officer.set(com, Some(duty));
        com
    } else {
        serve(&mut scene, &kind, duty, 0)
    };
    let mut line = Vec::new();
    for minute in 0..minutes {
        let a = audit(&mut scene, 60, &[]);
        for (t, c) in &a.ordered {
            line.push(format!("  t={:>6} {}", t, words(&scene.sim, c)));
        }
        a.report(&format!("minute {}", minute + 1));
        let u = scene.sim.unit(id).unwrap();
        println!("    stores {}m {}e; unit at ({}, {}), {} orders", scene.metal(), scene.energy(), u.pos.x.to_int(), u.pos.z.to_int(), u.orders.len());
    }
    println!("{}", line.join("\n"));
    println!("census {:?}", scene.census());
}


#[test]
#[ignore]
fn explore_a_tower_over_a_deposit() {
    // An enemy laser tower stands over the third nearest deposit, in sight at first and then not.
    for sight in [false] {
        let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
        let deposits = deposits_near(&scene, 1500);
        println!("deposits near the start: {:?}", deposits.iter().map(|p| (p.x.to_int(), p.z.to_int(), px(*p, scene.start))).collect::<Vec<_>>());
        let target = deposits[std::env::var("TA_QA_DEPOSIT").ok().and_then(|v| v.parse().ok()).unwrap_or(2)];
        let tower = scene.spawn_at("CORLLT", 1, target + FxVec3::from_ints(40, 0, 40));
        scene.sim.set_health(tower, scene.sim.unit_type(scene.sim.unit(tower).unwrap()).max_health);
        let b = serve(&mut scene, "ARMCK", Duty::Economy, 0);
        if sight {
            scene.sim.los_enabled = false;
        }
        let a = audit(&mut scene, 8 * 60, &[]);
        a.report(&format!("tower over {:?}, whole map in sight {sight}", (target.x.to_int(), target.z.to_int())));
        println!("    ordered: {:?}", a.ordered.iter().map(|(t, c)| format!("{t}: {}", words(&scene.sim, c))).collect::<Vec<_>>());
        let _ = b;
    }
}

#[test]
#[ignore]
fn explore_a_squad_over_a_builder() {
    // A builder is out at a far deposit (800 px away) when six enemy Peewees come at it.
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let builder = serve(&mut scene, "ARMCK", Duty::Economy, 0);
    // Let it set off for the third deposit (nearest first): a minute of play.
    let a = audit(&mut scene, 40, &[]);
    a.report("40 s of peace");
    let at = scene.sim.unit(builder).unwrap().pos;
    println!("builder at ({}, {}), orders {:?}", at.x.to_int(), at.z.to_int(), scene.sim.unit(builder).unwrap().orders);
    // The raiders appear 300 px from it and attack it.
    let squad: Vec<UnitId> = (0..6).map(|k| scene.spawn_at("CORAK", 1, at + FxVec3::from_ints(300 + 20 * k, 0, 0))).collect();
    let attack = Command::Order { player: 1, units: squad.clone(), order: Order::Attack(ta_sim::Target::Unit(builder)), queue: false };
    let a = audit(&mut scene, 90, &[attack]);
    a.report("a squad over the builder");
    let u = scene.sim.unit(builder);
    println!("builder now {:?}; ordered {:?}", u.map(|u| (u.pos.x.to_int(), u.pos.z.to_int(), u.health.to_int())), a.ordered.iter().map(|(t, c)| format!("{t}: {}", words(&scene.sim, c))).collect::<Vec<_>>());
}

#[test]
#[ignore]
fn explore_makers_and_a_hungry_lab() {
    for builders in [0usize, 1, 2] {
        let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
        for (x, z) in [(200, 1576), (248, 1784), (584, 1336), (88, 2248)] {
            scene.spawn_at("ARMMEX", 0, FxVec3::from_ints(x, 0, z));
        }
        for k in 0..8 {
            scene.spawn("ARMSOLAR", -140 - 70 * (k % 4), 250 + 90 * (k / 4));
        }
        let lab = scene.spawn("ARMLAB", 0, 140);
        let lab2 = scene.spawn("ARMLAB", 220, 160);
        let kbot = scene.type_id("ARMPW");
        scene.sim.step(&[Command::Produce { player: 0, factory: lab, type_id: kbot, count: 300 }, Command::Produce { player: 0, factory: lab2, type_id: kbot, count: 300 }]);
        for k in 0..builders {
            serve(&mut scene, "ARMCK", Duty::Economy, k as i64);
        }
        let mut dry_e = 0u64;
        let mut dry_m = 0u64;
        for minute in 0..12 {
            let a = audit(&mut scene, 60, &[]);
            dry_e += a.energy_dry;
            dry_m += a.metal_dry;
            let eco = &scene.sim.players[0].economy;
            println!("  {builders} builders, min {:>2}: {}m {}e (cap {:.0}/{:.0}), income {:.0}m {:.0}e, expense {:.0}m {:.0}e; makers {} (on {}); {} kbots", minute + 1, scene.metal(), scene.energy(), eco.metal_capacity.to_f32(), eco.energy_capacity.to_f32(), eco.metal_income.to_f32(), eco.energy_income.to_f32(), eco.metal_expense.to_f32(), eco.energy_expense.to_f32(), scene.count("ARMMAKR"), scene.sim.units().filter(|u| u.owner == 0 && !u.dead && scene.sim.unit_type(u).name == "ARMMAKR" && u.activated).count(), scene.count("ARMPW"));
        }
        println!("{builders} builders: energy dry {dry_e} s, metal dry {dry_m} s, {} kbots; {:?}", scene.count("ARMPW"), scene.census());
    }
}

#[test]
#[ignore]
fn explore_late_game() {
    for kind in ["ARMACK", "ARMACV", "ARMACA", "ARMCK"] {
        let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
        scene.sim.players[0].economy.metal = Fx::from_int(2000);
        scene.sim.players[0].economy.energy = Fx::from_int(8000);
        let deposits = deposits_near(&scene, 1500);
        for d in deposits.iter().take(12) {
            scene.spawn_at("ARMMEX", 0, *d);
        }
        for k in 0..8 {
            scene.spawn("ARMSOLAR", -140 - 70 * (k % 4), 250 + 90 * (k / 4));
        }
        scene.spawn("ARMALAB", 150, 200);
        scene.spawn("ARMLAB", 0, 140);
        scene.sim.step(&[]);
        serve(&mut scene, kind, Duty::Economy, 0);
        let a = audit(&mut scene, 8 * 60, &[]);
        a.report(&format!("{kind} on economy, late game"));
        println!("    ordered {:?}", a.ordered.iter().map(|(t, c)| format!("{t}: {}", words(&scene.sim, c))).collect::<Vec<_>>());
        let eco = &scene.sim.players[0].economy;
        println!("    income {:.0}m {:.0}e; {:?}", eco.metal_income.to_f32(), eco.energy_income.to_f32(), scene.census());
    }
}

#[test]
#[ignore]
fn explore_what_an_officer_costs() {
    for map_name in ["the pass", "sherwood", "painted desert", "red river", "seven islands", "acid pools"] {
        let Some(mut scene) = Scene::on(map_name, ["ARM", "CORE"], true) else { continue };
        let started = std::time::Instant::now();
        let officer = ta_ai::DutyOfficer::new(0, &scene.sim);
        let made = started.elapsed();
        drop(officer);
        // Ten constructors idle on a duty: what a think costs.
        let ids: Vec<_> = (0..10).map(|k| serve(&mut scene, "ARMCK", Duty::Economy, k)).collect();
        let _ = ids;
        scene.play(120);
        let started = std::time::Instant::now();
        let n = 30 * 30;
        for _ in 0..n {
            let c = scene.officer.think(&scene.sim);
            scene.sim.step(&c);
        }
        let per = started.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("{map_name:16} officer made in {:>6.1} ms; ten constructors: {per:.3} ms a tick (sim included)", made.as_secs_f64() * 1000.0);
    }
}

#[test]
#[ignore]
fn explore_costs() {
    let Some(scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    for name in ["ARMMEX", "ARMMOHO", "ARMSOLAR", "ARMWIN", "ARMADVSOL", "ARMFUS", "ARMGEO", "ARMMAKR", "ARMMMKR", "ARMESTOR", "ARMMSTOR", "ARMLLT", "ARMHLT", "ARMRL", "ARMFRT", "ARMAMB", "ARMLAB", "ARMVP", "ARMALAB", "ARMCK", "ARMACK", "ARMUWMEX", "ARMTIDE", "CORMEX", "CORMOHO", "CORSOLAR", "CORFUS", "CORMAKR"] {
        let Some(id) = scene.sim.types.id(name) else { continue };
        let t = scene.sim.types.get(id);
        println!("{name:10} metal {:>6} energy {:>7} build time {:>8} worker {:>4} make {:.1}/{:.1} use {:.1}", t.cost_metal.to_int(), t.cost_energy.to_int(), t.build_time.to_int(), t.worker_rate.to_int(), t.energy_make.to_f32() * 30.0, t.metal_make.to_f32() * 30.0, t.energy_use.to_f32() * 30.0);
    }
}

#[test]
#[ignore]
fn explore_a_crowd_of_constructors() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    scene.sim.players[0].economy.metal = Fx::from_int(200);
    // A lab with work, hurt buildings, wrecks.
    let lab = scene.spawn("ARMLAB", 0, 140);
    let kbot = scene.type_id("ARMPW");
    scene.sim.step(&[Command::Produce { player: 0, factory: lab, type_id: kbot, count: 100 }]);
    let mut hurt = Vec::new();
    for k in 0..4 {
        let s = scene.spawn("ARMSOLAR", -140 - 70 * k, 150);
        let max = scene.sim.unit_type(scene.sim.unit(s).unwrap()).max_health;
        scene.sim.set_health(s, max / 3);
        hurt.push(s);
    }
    for k in 0..6 {
        let at = scene.start + FxVec3::from_ints(150 + 60 * k, 0, -120);
        scene.sim.place_feature(if k % 2 == 0 { "ARMFLASH_DEAD" } else { "CORAK_DEAD" }, at, 0);
    }
    scene.spawn_frame("ARMSOLAR", scene.start + FxVec3::from_ints(-400, 0, 300));
    scene.spawn_frame("ARMMEX", scene.start + FxVec3::from_ints(-300, 0, 400));
    for (k, duty) in [Duty::Economy, Duty::Economy, Duty::Economy, Duty::Economy, Duty::Repair, Duty::Repair, Duty::Reclaim, Duty::Reclaim, Duty::Reclaim, Duty::Defence].into_iter().enumerate() {
        serve(&mut scene, "ARMCK", duty, k as i64);
    }
    let a = audit(&mut scene, 6 * 60, &[]);
    a.report("ten constructors on four duties");
    for c in a.conflicts.iter().take(10) {
        println!("    conflict: {c}");
    }
    println!("{:?}", scene.census());
}

#[test]
#[ignore]
fn explore_defence_and_planes() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    for (x, z) in [(200, 1576), (248, 1784), (584, 1336), (88, 2248)] {
        scene.spawn_at("ARMMEX", 0, FxVec3::from_ints(x, 0, z));
    }
    for k in 0..4 {
        scene.spawn("ARMSOLAR", -140 - 70 * k, 150);
    }
    let _b = serve(&mut scene, "ARMCK", Duty::Defence, 0);
    let a = audit(&mut scene, 60, &[]);
    a.report("a minute of peace");
    // Three bombers over the base for a few seconds.
    let planes: Vec<UnitId> = (0..3).map(|k| scene.spawn_at("CORSHAD", 1, scene.start + FxVec3::from_ints(100 + 40 * k, 0, 0))).collect();
    scene.play(3);
    for p in planes {
        scene.sim.remove_unit(p);
    }
    let a = audit(&mut scene, 5 * 60, &[]);
    a.report("after the planes");
    println!("ordered {:?}", a.ordered.iter().map(|(t, c)| format!("{t}: {}", words(&scene.sim, c))).collect::<Vec<_>>());
    println!("{:?}", scene.census());
}

#[test]
#[ignore]
fn explore_a_plain_reclaim() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    scene.sim.players[0].economy.metal = Fx::from_int(50);
    scene.sim.clear_features_where(|_, _| true);
    let at = scene.start + FxVec3::from_ints(100, 0, 40);
    let f = scene.sim.place_feature("ARMFLASH_DEAD", at, 0).expect("the wreck");
    let com = scene.sim.units().find(|u| u.owner == 0).unwrap().id;
    let d = &scene.sim.data.features[scene.sim.features[f].def];
    println!("wreck {f}: metal {} energy {} reclaimable {} blocking {}", d.metal, d.energy, d.reclaimable, d.blocking);
    scene.sim.step(&[Command::Order { player: 0, units: vec![com], order: Order::Reclaim(f), queue: false }]);
    for sec in 0..30 {
        for _ in 0..30 {
            scene.sim.step(&[]);
        }
        let u = scene.sim.unit(com).unwrap();
        println!("  {sec}s: reclaimed {:.2} alive {} orders {:?} metal {}", scene.sim.features[f].reclaimed.to_f32(), scene.sim.features[f].alive, u.orders.iter().collect::<Vec<_>>(), scene.metal());
    }
}

#[test]
#[ignore]
fn explore_reclaim_far_from_home() {
    // Wrecks lie at 500, 900, 1300 and 1500 px from the base towards the enemy; stores nearly empty.
    for kind in ["ARMCOM", "ARMCK"] {
        let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
        scene.sim.players[0].economy.metal = Fx::from_int(50);
        scene.sim.players[0].economy.energy = Fx::from_int(50);
        let toward = (scene.foe - scene.start).xz().normalize();
        scene.sim.clear_features_where(|_, _| true);
        for d in [500, 900, 1300, 1500] {
            let at = scene.start + toward.scale(Fx::from_int(d));
            scene.sim.place_feature("ARMFLASH_DEAD", at, 0).expect("the wreck");
        }
        let id = if kind == "ARMCOM" {
            let com = scene.sim.units().find(|u| u.owner == 0).unwrap().id;
            scene.officer.set(com, Some(Duty::Reclaim));
            com
        } else {
            serve(&mut scene, "ARMCK", Duty::Reclaim, 0)
        };
        if std::env::var("TA_QA_TRACE").is_ok() {
            scene.play_watched(40, &[], |sim, commands| {
                if sim.tick % 45 == 0 || !commands.is_empty() {
                    let u = sim.unit(id).unwrap();
                    println!("  t={} at ({}, {}) orders {:?} build {} moving {} reclaimed {:.2}/{:.2}/{:.2}/{:.2}{}", sim.tick, u.pos.x.to_int(), u.pos.z.to_int(), u.orders, u.build.is_some(), u.moving, sim.features[609].reclaimed.to_f32(), sim.features[610].reclaimed.to_f32(), sim.features[611].reclaimed.to_f32(), sim.features[612].reclaimed.to_f32(), if commands.is_empty() { String::new() } else { format!("  <- {:?}", commands) });
                }
            });
        }
        let a = audit(&mut scene, 8 * 60, &[]);
        a.report(&format!("{kind} on reclaim, wrecks at 500/900/1300/1500"));
        for f in 609..613 {
            let ft = &scene.sim.features[f];
            println!("    feature {f}: alive {} reclaimed {:.2}", ft.alive, ft.reclaimed.to_f32());
        }
        println!("    features now: {}", scene.sim.features.len());
        let far = scene.sim.unit(id).map(|u| px(u.pos, scene.start));
        println!("    ended {far:?} px from start; ordered: {:?}", a.ordered.iter().map(|(t, c)| format!("{t}: {}", words(&scene.sim, c))).collect::<Vec<_>>());
    }
}

#[test]
#[ignore]
fn explore_stranded_constructors() {
    for (map_name, kind) in [("hundred isles", "ARMCK"), ("hundred isles", "ARMCS"), ("coast to coast", "ARMCV"), ("sherwood", "ARMCK")] {
        for duty in Duty::ALL {
            let Some(mut scene) = Scene::on(map_name, ["ARM", "CORE"], true) else { continue };
            let Some(at) = cut_off_spot(&scene, kind) else {
                println!("{map_name} {kind}: no cut-off spot");
                break;
            };
            let id = scene.spawn_at(kind, 0, at);
            scene.officer.set(id, Some(duty));
            let a = audit(&mut scene, 3 * 60, &[]);
            let u = scene.sim.unit(id).unwrap();
            a.report(&format!("{map_name:14} {kind} stranded at ({}, {}) {:8} now ({}, {})", at.x.to_int(), at.z.to_int(), duty.name(), u.pos.x.to_int(), u.pos.z.to_int()));
        }
    }
}

#[test]
#[ignore]
fn sweep_builders_at_sea() {
    let maps = ["coast to coast", "hundred isles", "ring atoll", "brain coral", "sail away", "canal crossing"];
    for map_name in maps {
        for (sides, kinds) in [(["ARM", "CORE"], ["ARMCOM", "ARMCK", "ARMCV", "ARMCA", "ARMCH", "ARMCS"]), (["CORE", "ARM"], ["CORCOM", "CORCK", "CORCV", "CORCA", "CORCH", "CORCS"])] {
            for kind in kinds {
                for duty in Duty::ALL {
                    let Some(mut scene) = Scene::on(map_name, sides, true) else { continue };
                    if scene.sim.types.id(kind).is_none() {
                        println!("{map_name:14} no unit {kind}");
                        continue;
                    }
                    if kind.ends_with("COM") {
                        let com = scene.sim.units().find(|u| u.owner == 0).unwrap().id;
                        scene.officer.set(com, Some(duty));
                    } else {
                        serve(&mut scene, kind, duty, 0);
                    }
                    let a = audit(&mut scene, 4 * 60, &[]);
                    a.report(&format!("{map_name:14} {} {kind:7} {:8}", sides[0], duty.name()));
                }
            }
        }
    }
}

#[test]
#[ignore]
fn sweep_builders_on_land() {
    let maps = ["sherwood", "metal heck", "the pass", "painted desert"];
    for map_name in maps {
        for (sides, kinds) in [(["ARM", "CORE"], ["ARMCK", "ARMCV", "ARMACK", "ARMCA"]), (["CORE", "ARM"], ["CORCK", "CORCV", "CORACK", "CORCA"])] {
            for kind in kinds {
                for duty in Duty::ALL {
                    let Some(mut scene) = Scene::on(map_name, sides, true) else { continue };
                    if scene.sim.types.id(kind).is_none() {
                        continue;
                    }
                    serve(&mut scene, kind, duty, 0);
                    let a = audit(&mut scene, 4 * 60, &[]);
                    a.report(&format!("{map_name:14} {} {kind:7} {:8}", sides[0], duty.name()));
                }
            }
        }
    }
}

/// A real fight: a computer player (Normal, or `TA_QA_LEVEL`) attacks while six constructors of ours, on four duties,
/// work with a lab turning out kbots (left at the rally point). What each minute looked like, and the audit's counts for the lot.
#[test]
#[ignore]
fn explore_a_real_fight() {
    use ta_ai::{Difficulty, SkirmishAi};
    let map_name = std::env::var("TA_QA_MAP").unwrap_or_else(|_| "sherwood".into());
    let minutes: u64 = std::env::var("TA_QA_MINUTES").ok().and_then(|v| v.parse().ok()).unwrap_or(15);
    let level = match std::env::var("TA_QA_LEVEL").as_deref() {
        Ok("easy") => Difficulty::Easy,
        Ok("hard") => Difficulty::Hard,
        _ => Difficulty::Normal,
    };
    let Some(mut scene) = Scene::on(&map_name, ["ARM", "CORE"], true) else { return };
    let mut enemy = SkirmishAi::new(1, &scene.sim, level);
    let lab = scene.spawn("ARMLAB", 0, 140);
    let kbot = scene.type_id("ARMPW");
    scene.sim.step(&[Command::Produce { player: 0, factory: lab, type_id: kbot, count: 100 }]);
    let duties = [Duty::Economy, Duty::Economy, Duty::Repair, Duty::Reclaim, Duty::Defence, Duty::Defence];
    let ids: Vec<UnitId> = duties.iter().enumerate().map(|(k, &d)| serve(&mut scene, "ARMCK", d, k as i64)).collect();
    let com = scene.commander();
    scene.officer.set(com, Some(Duty::Economy));
    let mut total = 0usize;
    let mut last: std::collections::BTreeMap<UnitId, (u64, Order)> = Default::default();
    let mut spam = 0usize;
    for minute in 0..minutes {
        let mut orders = 0;
        for _ in 0..30 * 60 {
            let mut commands = scene.officer.think(&scene.sim);
            for c in &commands {
                if let Command::Order { units, order, queue: false, .. } = c {
                    for &u in units {
                        if let Some((t, o)) = last.get(&u)
                            && o == order
                            && scene.sim.tick - t < 150
                            && !matches!(order, Order::Stop)
                        {
                            spam += 1;
                        }
                        last.insert(u, (scene.sim.tick, order.clone()));
                    }
                }
            }
            orders += commands.len();
            commands.extend(enemy.think(&scene.sim));
            scene.sim.step(&commands);
            scene.sim.take_events();
        }
        total += orders;
        let alive = ids.iter().filter(|&&i| scene.sim.unit(i).is_some_and(|u| !u.dead)).count();
        let events = scene.officer.take_events();
        println!(
            "  min {:>2}: {orders:>3} orders, {alive}/6 constructors alive, Commander {}, {} on duty, metal {} energy {}, {} units of ours, {} of theirs; events {events:?}",
            minute + 1,
            scene.sim.unit(com).map_or("dead".into(), |u| format!("{}%", (u.health * 100 / scene.sim.unit_type(u).max_health).to_int())),
            scene.officer.len(),
            scene.metal(),
            scene.energy(),
            scene.sim.players[0].unit_count,
            scene.sim.players[1].unit_count
        );
    }
    println!("{total} orders in {minutes} minutes, {spam} repeated within five seconds; census {:?}", scene.census());
}
