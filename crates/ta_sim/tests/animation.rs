//! The engine's calls into unit scripts that drive their animation, and where extractors
//! may stand (skipped without a TA install): extractors spin with the metal they draw and
//! go on deposits except on metal worlds, wind generators spin with the wind and face it,
//! amphibians change to swim in deep water, aircraft tell their speed, tanks rock when they
//! fire, factories spray from both nozzles, turrets learn their reload time, radar dishes
//! turn.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::world::PlacementError;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, Target, UnitId, UnitTypes};

fn sim_on(map_name: &str) -> Option<Sim> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map_name}.ota")).ok()?));
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: if i == 0 { "ARM".into() } else { "CORE".into() }, color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(5000), energy: Fx::from_int(5000), ai: false })
        .collect();
    Some(Sim::new(types, data, &map, &info, 0, &players, 99))
}

fn cell_pos(x: i32, z: i32) -> FxVec3 {
    FxVec3::from_ints(x as i64 * 16 + 8, 0, z as i64 * 16 + 8)
}

/// A piece's rotation about an axis, per tick, over the next tick.
fn spin_per_tick(sim: &mut Sim, id: UnitId, piece: usize, axis: usize) -> i32 {
    let before = sim.unit(id).unwrap().cob.as_ref().unwrap().pieces[piece].rotation[axis];
    sim.step(&[]);
    let after = sim.unit(id).unwrap().cob.as_ref().unwrap().pieces[piece].rotation[axis];
    ta_sim::fixed::wrap_angle(after.wrapping_sub(before))
}

fn statics(sim: &Sim, id: UnitId) -> Vec<i32> {
    sim.unit(id).unwrap().cob.as_ref().unwrap().statics.clone()
}

#[test]
fn extractors_need_ore_except_on_metal_worlds() {
    let Some(sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mex = sim.types.id("ARMMEX").unwrap();
    let w = sim.world.width as i32;
    let ore = (0..sim.world.metal.len() as i32).find(|&i| sim.world.metal[i as usize] > sim.world.surface_metal).expect("a deposit");
    let site = sim.best_extraction_site(mex, cell_pos(ore % w, ore / w), 2);
    assert_eq!(sim.placement_error(mex, site), None, "on a deposit");
    // Plain ground somewhere: refused for the want of ore (and for nothing else).
    let plain = (0..sim.world.height as i32)
        .step_by(7)
        .flat_map(|z| (0..w).step_by(7).map(move |x| (x, z)))
        .find(|&(x, z)| sim.placement_error(mex, cell_pos(x, z)) == Some(PlacementError::NeedsMetal));
    assert!(plain.is_some(), "plain ground refuses extractors");

    let Some(metal_world) = sim_on("Metal Heck") else { return };
    assert!(metal_world.world.metal_world());
    let mex = metal_world.types.id("ARMMEX").unwrap();
    let w = metal_world.world.width as i32;
    let sites: Vec<_> = (0..metal_world.world.height as i32).step_by(5).flat_map(|z| (0..w).step_by(5).map(move |x| (x, z))).collect();
    assert!(sites.iter().all(|&(x, z)| metal_world.placement_error(mex, cell_pos(x, z)) != Some(PlacementError::NeedsMetal)));
    assert!(sites.iter().any(|&(x, z)| metal_world.placement_error(mex, cell_pos(x, z)).is_none()), "somewhere flat takes one");
}

#[test]
fn an_extractor_spins_with_the_metal_it_draws() {
    let Some(mut sim) = sim_on("Painted Desert") else { return };
    let mex = sim.types.id("ARMMEX").unwrap();
    let w = sim.world.width as i32;
    let ore = (0..sim.world.metal.len() as i32).find(|&i| sim.world.metal[i as usize] > sim.world.surface_metal).unwrap();
    let site = sim.best_extraction_site(mex, cell_pos(ore % w, ore / w), 2);
    let id = sim.spawn_unit(mex, 0, site, 0, true).unwrap();
    let rate = sim.extraction_at(mex, site);
    // Spun up within seconds (the arm accelerates by the frame), then steady.
    for _ in 0..30 * 30 {
        sim.step(&[]);
    }
    let per_tick = spin_per_tick(&mut sim, id, 1, 1).abs();
    let expected = (rate * 500).to_int() as i32 * 45 / 30;
    assert!(expected > 100, "a deposit extractor spins visibly ({expected} per tick)");
    assert!((per_tick - expected).abs() <= expected / 20 + 2, "arm turns {per_tick} per tick, expected {expected} from {rate:?} metal/s");
}

#[test]
fn wind_generators_are_told_the_wind() {
    let Some(mut sim) = sim_on("Painted Desert") else { return };
    let win = sim.types.id("ARMWIN").unwrap();
    let id = sim.spawn_unit(win, 0, sim.find_build_site(win, FxVec3::from_ints(2000, 0, 2000), 40).unwrap(), 0, true).unwrap();
    // SetSpeed and SetDirection store the blade speed (static 1) and heading (static 2).
    let s = statics(&sim, id);
    assert_eq!(s[1] as i64, (sim.wind * 3000).to_int(), "blade speed from the wind");
    assert_eq!(s[2], sim.wind_heading, "facing the wind");
    for _ in 0..10 {
        sim.step(&[]);
    }
    if sim.wind > Fx::ZERO {
        assert_ne!(spin_per_tick(&mut sim, id, 1, 2), 0, "the blades turn");
    }
}

#[test]
fn amphibians_change_to_swim_in_deep_water() {
    let Some(mut sim) = sim_on("Coast To Coast") else { return };
    let pelican = sim.types.id("ARMAMPH").unwrap();
    let (w, h) = (sim.world.width as i32, sim.world.height as i32);
    let deep = (0..h).flat_map(|z| (0..w).map(move |x| (x, z))).find(|&(x, z)| {
        let p = cell_pos(x, z);
        (sim.world.sea_level - sim.world.height_at(p.x, p.z)) > Fx::from_int(40)
    });
    let (x, z) = deep.expect("deep water");
    let id = sim.spawn_unit(pelican, 0, cell_pos(x, z), 0, true).unwrap();
    // (Its change of form takes a few seconds.)
    for _ in 0..30 * 10 {
        sim.step(&[]);
    }
    let s = statics(&sim, id);
    assert_eq!(s[6], 2, "told it is in deep water");
    assert_eq!(s[3], 1, "changed to swim");
}

#[test]
fn aircraft_tell_their_speed() {
    let Some(mut sim) = sim_on("Painted Desert") else { return };
    let fighter = sim.types.id("ARMFIG").unwrap();
    let id = sim.spawn_unit(fighter, 0, FxVec3::from_ints(1000, 0, 1000), 0, true).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![id], order: Order::Move(FxVec3::from_ints(5000, 0, 3000)), queue: false }]);
    for _ in 0..150 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(id).unwrap().move_rate, 2, "a plane at speed is at MoveRate2");
}

#[test]
fn core_bombers_deploy_their_wings_on_the_first_flight_and_after_landing() {
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    for player in &mut sim.players { player.endless = true; }
    let bomber = sim.types.id("CORSHAD").unwrap();
    let wings = ["winga", "wingb"].map(|name| sim.types.get(bomber).script.as_ref().unwrap().script.pieces.iter()
        .position(|piece| piece.eq_ignore_ascii_case(name)).expect("Shadow wing"));
    let fresh = sim.spawn_unit(bomber, 0, FxVec3::from_ints(1000, 0, 1000), 16384, true).unwrap();
    let cycled = sim.spawn_unit(bomber, 0, FxVec3::from_ints(1000, 0, 1300), 16384, true).unwrap();
    let destination = FxVec3::from_ints(6500, 0, 1000);
    sim.step(&[Command::Order { player: 0, units: vec![fresh], order: Order::Move(destination), queue: false }]);
    for _ in 0..150 { sim.step(&[]); }
    assert!(sim.unit(cycled).unwrap().landed, "the comparison aircraft has completed a landing");
    sim.step(&[Command::Order { player: 0, units: vec![cycled], order: Order::Move(destination), queue: false }]);
    for _ in 0..90 { sim.step(&[]); }
    for (label, id) in [("first flight", fresh), ("after landing", cycled)] {
        let unit = sim.unit(id).unwrap();
        assert!(!unit.landed && unit.speed > Fx::ZERO, "{label}: the Shadow is flying");
        let pose = wings.map(|piece| unit.cob.as_ref().unwrap().pieces[piece].rotation[2]);
        assert_eq!(pose, [-16384, 16384], "{label}: both wings must be fully deployed");
    }
}

#[test]
fn a_tank_rocks_when_it_fires() {
    let Some(mut sim) = sim_on("Painted Desert") else { return };
    let (tank, target) = (sim.types.id("ARMSTUMP").unwrap(), sim.types.id("CORSOLAR").unwrap());
    let a = sim.find_build_site(target, FxVec3::from_ints(2200, 0, 2000), 40).unwrap();
    let foe = sim.spawn_unit(target, 1, a, 0, true).unwrap();
    let id = sim.spawn_unit(tank, 0, a - FxVec3::from_ints(0, 0, 200), 0, true).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![id], order: Order::Attack(Target::Unit(foe)), queue: false }]);
    let mut rocked = 0;
    for _ in 0..30 * 10 {
        sim.step(&[]);
        let base = sim.unit(id).map(|u| u.cob.as_ref().unwrap().pieces[0].rotation).unwrap_or_default();
        rocked = rocked.max(base[0].abs() + base[2].abs());
    }
    assert!(rocked > 0, "the hull tilts with the recoil");
}

/// A hit jolts the hull a few degrees, as a shot's recoil does: `HitByWeapon(x, z)` is told
/// 500 × the direction of the hit's push, as `RockUnit` is. (Read as angles of up to 45°,
/// Raiders' shells tipped a radar vehicle half over and into the ground.)
#[test]
fn a_hit_jolts_the_hull_a_little() {
    let Some(mut sim) = sim_on("Painted Desert") else { return };
    let (seer, raider) = (sim.types.id("ARMSEER").unwrap(), sim.types.id("CORRAID").unwrap());
    let at = sim.find_build_site(seer, FxVec3::from_ints(2000, 0, 2000), 40).unwrap();
    let id = sim.spawn_unit(seer, 0, at, 0, true).unwrap();
    for k in 0..3 {
        sim.spawn_unit(raider, 1, at + FxVec3::from_ints(200 + k * 30, 0, (k - 1) * 60), -16384, true);
    }
    sim.los_enabled = false;
    let (mut hits, mut tilt) = (0, 0);
    let mut health = sim.unit(id).unwrap().health;
    for _ in 0..30 * 8 {
        sim.step(&[]);
        let Some(u) = sim.unit(id) else { break };
        if u.health < health {
            hits += 1;
            health = u.health;
        }
        // The hull is the script's piece 0 (`base`).
        let base = u.cob.as_ref().unwrap().pieces[0].rotation;
        tilt = tilt.max(base[0].abs()).max(base[2].abs());
    }
    assert!(hits > 3, "the Raiders hit it ({hits} hits)");
    assert!(tilt > 0, "the hits jolt it");
    assert!(tilt <= 500, "by about 3° at most, not {:.0}°", tilt as f64 * 360.0 / 65536.0);
}

#[test]
fn a_factory_sprays_from_both_nozzles() {
    let Some(mut sim) = sim_on("Painted Desert") else { return };
    let (plant, product) = (sim.types.id("ARMVP").unwrap(), sim.types.id("ARMFLASH").unwrap());
    let id = sim.spawn_unit(plant, 0, sim.find_build_site(plant, FxVec3::from_ints(2000, 0, 2000), 40).unwrap(), 0, true).unwrap();
    sim.step(&[Command::Produce { player: 0, factory: id, type_id: product, count: 1 }]);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..30 * 6 {
        sim.step(&[]);
        seen.extend(sim.unit(id).unwrap().nano_piece);
    }
    assert!(seen.len() >= 2, "both nozzles spray (pieces {seen:?})");
}

#[test]
fn turrets_learn_their_reload_time() {
    let Some(mut sim) = sim_on("Painted Desert") else { return };
    let skimmer = sim.types.id("ARMSH").unwrap();
    let id = sim.spawn_unit(skimmer, 0, FxVec3::from_ints(2000, 0, 2000), 0, true).unwrap();
    sim.step(&[]);
    let t = sim.types.get(skimmer);
    let reload = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w).reload_ticks as i32).max().unwrap();
    // SetMaxReloadTime(ms): the Skimmer's turret rests after twice that (static 1).
    assert_eq!(statics(&sim, id)[1], reload * 1000 / 30 * 2);
}


#[test]
fn radar_dishes_turn() {
    let Some(mut sim) = sim_on("Painted Desert") else { return };
    let radar = sim.types.id("ARMRAD").unwrap();
    let id = sim.spawn_unit(radar, 0, sim.find_build_site(radar, FxVec3::from_ints(2000, 0, 2000), 40).unwrap(), 0, true).unwrap();
    for _ in 0..30 {
        sim.step(&[]);
    }
    // The dish (piece 1) spins about y at 60° a second, from the start (no acceleration).
    assert_eq!(spin_per_tick(&mut sim, id, 1, 1).abs(), 10922 / 30);
}
