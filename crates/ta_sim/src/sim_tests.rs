//! Limits past TA's own numbers: huge build times, more category tokens than 64, more
//! players than 16 or 32. Made-up game data (a mod's folder of TDF files) on a flat map, so no
//! install is needed.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::{FEATURE_NONE, TntMap};

use super::*;
use crate::unit::{PlayerSetup, Target};
use crate::vision::Seen;

#[path = "construction_tests.rs"]
mod construction;

#[path = "air_tests.rs"]
mod air;

#[path = "air_original_tests.rs"]
mod air_original;

#[path = "land_tests.rs"]
mod land;

#[path = "land_original_tests.rs"]
mod land_original;

#[path = "naval_tests.rs"]
mod naval;

#[path = "naval_original_tests.rs"]
mod naval_original;

#[path = "building_tests.rs"]
mod buildings;

#[path = "building_original_tests.rs"]
mod buildings_original;

#[path = "activation_original_tests.rs"]
mod activation_original;

#[path = "scenario_tests.rs"]
mod scenarios;

/// Game data from TDF files (path, text), loaded as a mod's folder is.
fn data_of(files: &[(String, String)]) -> GameData {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta-sim-limits-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_mod_dir(&dir, "test").unwrap();
    let data = GameData::load(&vfs);
    std::fs::remove_dir_all(&dir).ok();
    data
}

fn unit(name: &str, keys: &str) -> (String, String) {
    (format!("units/{}.fbi", name.to_ascii_lowercase()), format!("[UNITINFO]\n{{\nUnitName={name};\nObjectname={name};\nMaxDamage=1000;\nSightDistance=600;\n{keys}\n}}\n"))
}

/// A flat 64 × 64-cell map.
fn flat_map() -> TntMap {
    let cells = 64 * 64;
    TntMap { width: 64, height: 64, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; cells], features: vec![FEATURE_NONE; cells], feature_names: Vec::new(), minimap: Default::default() }
}

fn sim_of(data: GameData, players: usize) -> Sim {
    sim_on_map(data, players, &flat_map())
}

#[test]
fn checksums_detect_missile_inventory_production_reload_and_flight_divergence() {
    let data = data_of(&[unit("SILO", "Weapon1=NUKE;"), weapon_file("NUKE", "stockpile=1; commandfire=1; targetable=1;")]);
    let mut sim = sim_of(data, 2);
    let silo = sim.spawn_unit(0, 0, FxVec3::from_ints(100, 60, 100), 0, true).unwrap();
    let at = FxVec3::from_ints(300, 500, 300);
    sim.projectiles.push(crate::Projectile { id: 100, weapon: 0, owner: silo, owner_player: 0, pos: at, prev_pos: at, vel: FxVec3::from_ints(1, 2, 3), target: Some(Target::Ground(at)), target_pos: at, age: 5, ttl: 1000, hits: Vec::new(), dead: false });
    let expected = sim.checksum();
    assert_eq!(sim.clone().checksum(), expected, "snapshot/replay clones preserve the checksum");
    for field in 0..12 {
        let mut changed = sim.clone();
        match field {
            0 => changed.unit_mut(silo).unwrap().stockpile = 1,
            1 => changed.unit_mut(silo).unwrap().stockpile_work = 1,
            2 => changed.unit_mut(silo).unwrap().weapons[0].reload = 1,
            3 => changed.next_projectile_id += 1,
            4 => changed.projectiles[0].owner_player = 1,
            5 => changed.projectiles[0].pos.y += Fx::ONE,
            6 => changed.projectiles[0].vel.x += Fx::ONE,
            7 => changed.projectiles[0].target = Some(Target::Projectile(90)),
            8 => changed.projectiles[0].target_pos.x += Fx::ONE,
            9 => changed.projectiles[0].age += 1,
            10 => changed.projectiles[0].ttl += 1,
            11 => changed.projectiles[0].id += 1,
            _ => unreachable!(),
        }
        assert_ne!(changed.checksum(), expected, "nuclear state divergence must not pass a lockstep check (field {field})");
    }
}

#[test]
fn queued_orders_are_bounded_and_stop_can_clear_a_full_queue() {
    let data = data_of(&[unit("TANK", "CanMove=1; MaxVelocity=1;")]);
    let mut sim = sim_of(data, 1);
    let id = sim.spawn_unit(0, 0, FxVec3::from_ints(100, 60, 100), 0, true).unwrap();
    for _ in 0..ORDER_QUEUE_MAX * 2 {
        sim.apply(&Command::Order { player: 0, units: vec![id], order: Order::Patrol(FxVec3::from_ints(200, 60, 200)), queue: true });
    }
    assert_eq!(sim.unit(id).unwrap().orders.len(), ORDER_QUEUE_MAX);
    sim.apply(&Command::Order { player: 0, units: vec![id], order: Order::Stop, queue: false });
    assert_eq!(sim.unit(id).unwrap().orders.len(), 1);
    assert_eq!(sim.unit(id).unwrap().orders.front(), Some(&Order::Stop));
}

#[test]
fn nobody_owns_units_who_is_not_in_the_battle() {
    let data = data_of(&[unit("TANK", "CanMove=1; MaxVelocity=1;")]);
    let mut sim = sim_of(data, 2);
    assert!(sim.spawn_unit(0, 1, FxVec3::from_ints(100, 60, 100), 0, true).is_some());
    assert!(sim.spawn_unit(0, 2, FxVec3::from_ints(100, 60, 100), 0, true).is_none(), "a third player who is not there");
    sim.step(&[]);
}

#[test]
fn commands_with_extreme_coordinates_never_enter_unit_state() {
    let data = data_of(&[unit("TANK", "CanMove=1; MaxVelocity=1;")]);
    let mut sim = sim_of(data, 1);
    let id = sim.spawn_unit(0, 0, FxVec3::from_ints(100, 60, 100), 0, true).unwrap();
    let far = FxVec3::new(Fx(i64::MIN), Fx(i64::MAX), Fx(i64::MIN));
    for order in [Order::Move(far), Order::Patrol(far), Order::Attack(Target::Ground(far)), Order::Unload(far)] {
        sim.apply(&Command::Order { player: 0, units: vec![id], order, queue: false });
        assert!(sim.unit(id).unwrap().orders.is_empty());
    }
    sim.apply(&Command::SetRally { player: 0, factory: id, pos: far });
    assert!(sim.unit(id).unwrap().rally.is_empty());
    sim.apply(&Command::Formation { player: 0, units: vec![id], order: Order::Move(FxVec3::ZERO), queue: false, shape: crate::formation::Shape::Spots(vec![far]) });
    assert!(sim.unit(id).unwrap().orders.is_empty());
}

fn sim_on_map(data: GameData, players: usize, map: &TntMap) -> Sim {
    let data = Arc::new(data);
    let types = Arc::new(UnitTypes::compile(&data, &|_| None));
    let setups: Vec<PlayerSetup> =
        (0..players).map(|i| PlayerSetup { side: "ARM".into(), color: (i % 10) as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
    Sim::new(types, data, map, &MapInfo::default(), 0, &setups, 7)
}

/// Work orders must walk into their circular reach, including when approaching a
/// blocking rock diagonally. Reclaiming releases its cells and pays its full value.
#[test]
fn reclaim_reaches_and_removes_rocks_from_every_direction() {
    let data = data_of(&[
        unit("COMMANDER", "Builder=1; CanMove=1; MaxVelocity=2; Acceleration=1; BrakeRate=1; TurnRate=32768; WorkerTime=300; BuildDistance=100; CanReclamate=1; FootprintX=2; FootprintZ=2;"),
        ("features/all/rock.tdf".into(), "[ROCK] { footprintx=2; footprintz=2; blocking=1; reclaimable=1; metal=100; }".into()),
    ]);
    let mut map = flat_map();
    map.feature_names.push("ROCK".into());
    map.features[32 * 64 + 32] = 0;
    let original = sim_on_map(data, 1, &map);
    let rock = original.features[0].pos;
    for (dx, dz) in [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)] {
        let mut sim = original.clone();
        let start = rock + FxVec3::from_ints(dx * 320, 0, dz * 320);
        let builder = sim.spawn_unit(sim.types.id("COMMANDER").unwrap(), 0, start, 0, true).unwrap();
        sim.players[0].economy.metal = Fx::ZERO;
        sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Reclaim(0), queue: false }]);
        for _ in 0..900 {
            sim.step(&[]);
            if !sim.features[0].alive { break; }
        }
        let u = sim.unit(builder).unwrap();
        assert!(!sim.features[0].alive, "approach ({dx}, {dz}): position {:?}, orders {:?}", u.pos, u.orders);
        assert_eq!(sim.players[0].economy.metal, Fx::from_int(100));
        assert_eq!(sim.world.occupant(32, 32), crate::world::Occupant::Free);
        assert!(sim.take_events().iter().any(|e| matches!(e, SimEvent::FeatureDestroyed { index: 0, .. })));
    }
}

/// A building put over what lies loose on the ground (TA lets it be: none of it blocks) clears
/// its footprint of the plants, stones and scars there, whose 3D stand-ins would stand through
/// it. Wreckage keeps its metal under it, an indestructible anemone stays, a vent stays for its
/// plant, and what lies beside it stays. A building placed whole (a mission's) clears its ground
/// too.
#[test]
fn a_building_clears_the_loose_features_under_it() {
    let data = || data_of(&[
        unit("SOLAR", "BuildTime=600; FootprintX=4; FootprintZ=4;"),
        ("features/all/loose.tdf".into(), [
            "[FERN] { footprintx=1; footprintz=1; blocking=0; reclaimable=1; flamable=1; energy=75; }",
            "[PEBBLE] { footprintx=1; footprintz=1; blocking=0; }",
            "[HEAP] { footprintx=2; footprintz=2; blocking=0; reclaimable=1; metal=40; }",
            "[ANEMONE] { footprintx=2; footprintz=2; blocking=0; indestructible=1; }",
            "[VENT] { footprintx=1; footprintz=1; geothermal=1; }",
        ].join("\n")),
    ]);
    let mut map = flat_map();
    map.feature_names.extend(["FERN", "PEBBLE", "HEAP", "ANEMONE", "VENT"].map(String::from));
    // (Cells: the building below takes 31..35 each way.)
    for (x, z, f) in [(32, 32, 0u16), (34, 33, 1), (31, 33, 2), (33, 31, 3), (32, 34, 4), (36, 32, 0), (40, 40, 1)] {
        map.features[z * 64 + x] = f;
    }
    let mut sim = sim_on_map(data(), 1, &map);
    let standing = |sim: &Sim| -> Vec<(String, (i32, i32))> {
        sim.features.iter().filter(|f| f.alive).map(|f| (sim.data.features[f.def].name.to_ascii_uppercase(), f.origin)).collect()
    };
    assert_eq!(standing(&sim).len(), 7);
    sim.take_events();
    let solar = sim.types.id("SOLAR").unwrap();
    let site = sim.spawn_unit(solar, 0, FxVec3::from_ints(528, 0, 528), 0, false).unwrap();
    assert_eq!(sim.unit(site).unwrap().occupied_origin, Some((31, 31)));
    let mut left = standing(&sim);
    left.sort();
    assert_eq!(left, [("ANEMONE".to_string(), (33, 31)), ("FERN".into(), (36, 32)), ("HEAP".into(), (31, 33)), ("PEBBLE".into(), (40, 40)), ("VENT".into(), (32, 34))]);
    let gone = sim.take_events().iter().filter(|e| matches!(e, SimEvent::FeatureDestroyed { .. })).count();
    assert_eq!(gone, 2, "the fern and the pebble under it");
    // Placed whole, as a mission places its buildings: the same.
    let mut whole = sim_on_map(data(), 1, &map);
    whole.spawn_unit(solar, 0, FxVec3::from_ints(528, 0, 528), 0, true).unwrap();
    assert_eq!(standing(&whole).len(), 5);
}

#[test]
fn construction_submarine_approaches_before_assisting_and_stops_its_previous_beam() {
    let data = data_of(&[
        unit("SUB", "Builder=1; CanMove=1; MaxVelocity=2; Acceleration=1; BrakeRate=1; TurnRate=32768; WorkerTime=300; BuildDistance=64; MinWaterDepth=20; WaterLine=30; Floater=1;"),
        unit("BUILDING", "BuildTime=600; FootprintX=4; FootprintZ=4; MinWaterDepth=20; Floater=1;"),
    ]);
    let mut map = flat_map();
    map.sea_level = 100;
    map.heights.fill(0);
    let mut sim = sim_on_map(data, 1, &map);
    let sub_type = sim.types.id("SUB").unwrap();
    assert_eq!(sim.types.get(sub_type).locomotion, Locomotion::Naval);
    let builder = sim.spawn_unit(sub_type, 0, FxVec3::from_ints(100, 0, 100), 0, true).unwrap();
    let building = sim.types.id("BUILDING").unwrap();
    let near = sim.spawn_unit(building, 0, FxVec3::from_ints(180, 0, 100), 0, false).unwrap();
    let far = sim.spawn_unit(building, 0, FxVec3::from_ints(750, 0, 750), 0, false).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Repair(near), queue: false }]);
    sim.step(&[]);
    assert!(sim.unit(builder).unwrap().in_build_stance);
    sim.take_events();
    sim.apply(&Command::Order { player: 0, units: vec![builder], order: Order::Repair(far), queue: false });
    assert!(!sim.unit(builder).unwrap().in_build_stance, "cancel the previous job's stance immediately");
    let mut worked = false;
    for _ in 0..900 {
        let target = sim.unit(far).unwrap();
        let in_range = sim.in_build_range(sim.unit(builder).unwrap(), target.pos, sim.unit_type(target).footprint);
        let before = target.build_work;
        sim.step(&[]);
        let sprayed = sim.take_events().iter().any(|e| matches!(e, SimEvent::NanoSpray { builder: b, target: t } if *b == builder && *t == far));
        if !in_range {
            assert!(!sprayed, "no nanolathe while crossing the map");
            assert_eq!(sim.unit(far).unwrap().build_work, before, "no build power from outside reach");
        }
        worked |= sim.unit(far).unwrap().build_work > before;
        if sim.unit(far).unwrap().is_complete() { break; }
    }
    assert!(worked && sim.unit(far).unwrap().is_complete(), "the submarine reaches and finishes the building");
}

/// Synthetic folding wings and callback counters, without original scripts or models.
fn folding_aircraft_sim(water: bool, activate_when_built: bool) -> Sim {
    use crate::cob::CobProgram;
    use ta_formats::cob::{CobFunction, CobScript, op};
    let data = data_of(&[unit("PLANE", &format!("CanFly=1; CanMove=1; MaxVelocity=4; CruiseAlt=90; ActivateWhenBuilt={};", activate_when_built as u8))]);
    let mut map = flat_map();
    if water { map.sea_level = 100; }
    let mut sim = sim_on_map(data, 1, &map);
    sim.players[0].endless = true;
    let mut script = CobScript { pieces: vec!["leftwing".into(), "rightwing".into()], static_var_count: 2, ..Default::default() };
    for (name, counter, angle) in [("Activate", 0, 16384i32), ("Deactivate", 1, 0)] {
        script.functions.push(CobFunction { name: name.into(), offset: script.code.len() as u32 });
        script.code.extend([
            op::PUSH_STATIC, counter, op::PUSH_CONSTANT, 1, op::ADD, op::POP_STATIC, counter,
            op::PUSH_CONSTANT, (-angle) as u32, op::TURN_NOW, 0, 2,
            op::PUSH_CONSTANT, angle as u32, op::TURN_NOW, 1, 2,
            op::PUSH_CONSTANT, 0, op::RETURN,
        ]);
    }
    Arc::get_mut(&mut sim.types).unwrap().types[0].script = Some(Arc::new(CobProgram::new(script, &[])));
    sim
}

#[test]
fn aircraft_receive_first_flight_and_later_takeoff_callbacks_once() {
    for complete in [false, true] {
        for activate_when_built in [false, true] {
            let mut sim = folding_aircraft_sim(false, activate_when_built);
            let id = sim.spawn_unit(0, 0, FxVec3::from_ints(100, 0, 100), 16384, complete).unwrap();
            if !complete {
                sim.step(&[]);
                assert_eq!(sim.unit(id).unwrap().cob.as_ref().unwrap().statics, [0, 0], "unfinished aircraft must not take off");
                sim.finish_frame(id);
            }
            let fly = Command::Order { player: 0, units: vec![id], order: Order::Move(FxVec3::from_ints(900, 0, 100)), queue: false };
            sim.step(std::slice::from_ref(&fly));
            for _ in 0..45 { sim.step(&[]); }
            let u = sim.unit(id).unwrap();
            assert!(!u.landed && u.speed > Fx::ZERO);
            let script = u.cob.as_ref().unwrap();
            assert_eq!(script.statics, [1, 0], "the first flight needs exactly one Activate (complete={complete}, activate_when_built={activate_when_built})");
            assert_eq!(script.pieces.iter().map(|p| p.rotation[2]).collect::<Vec<_>>(), [-16384, 16384]);
            sim.step(&[Command::Order { player: 0, units: vec![id], order: Order::Stop, queue: false }]);
            for _ in 0..180 { sim.step(&[]); }
            assert!(sim.unit(id).unwrap().landed);
            assert_eq!(sim.unit(id).unwrap().cob.as_ref().unwrap().statics, [1, 1]);
            sim.step(&[fly]);
            assert!(!sim.unit(id).unwrap().landed);
            assert_eq!(sim.unit(id).unwrap().cob.as_ref().unwrap().statics, [2, 1], "taking off again reopens the wings");
        }
    }
}

#[test]
fn initial_flight_callbacks_cover_water_and_prepositioned_aircraft_without_repeating() {
    for prepositioned in [false, true] {
        let mut sim = folding_aircraft_sim(true, false);
        let pos = FxVec3::from_ints(300, 0, 300);
        let id = if prepositioned { sim.spawn_airborne(0, 0, pos, 16384, None) } else { sim.spawn_unit(0, 0, pos, 16384, true) }.unwrap();
        for _ in 0..180 { sim.step(&[]); }
        let u = sim.unit(id).unwrap();
        assert!(!u.landed, "an idle aircraft over water cannot land");
        assert_eq!(u.cob.as_ref().unwrap().statics, [1, 0], "initial activation happens once, even as movement rates change");
        assert_eq!(u.cob.as_ref().unwrap().pieces[0].rotation[2], -16384);
    }
}

fn with_menu(mut data: GameData, builder: &str, products: &[&str]) -> GameData {
    let id = data.unit_id(builder).unwrap();
    data.units[id].build_options = products.iter().map(|s| (*s).into()).collect();
    data
}

fn construction_data() -> GameData {
    let data = data_of(&[
        unit("BUILDER", "Builder=1; WorkerTime=30; CanMove=1; MaxVelocity=1; BuildDistance=200;"),
        unit("FACTORY", "Builder=1; WorkerTime=30; BuildDistance=200; BuildTime=1000; BuildCostMetal=100;"),
        unit("TANK", "CanMove=1; MaxVelocity=1; BuildTime=1000; BuildCostMetal=100;"),
        unit("OTHER", "CanMove=1; MaxVelocity=1; BuildTime=1000; BuildCostMetal=100;"),
        unit("BUILDING", "FootprintX=2; FootprintZ=2; BuildTime=1000; BuildCostMetal=100;"),
    ]);
    with_menu(with_menu(data, "BUILDER", &["BUILDING", "FACTORY"]), "FACTORY", &["TANK", "OTHER"])
}

#[test]
fn production_extreme_counts_are_bounded_and_cancel_from_the_back() {
    let mut sim = sim_of(construction_data(), 1);
    let (factory, tank, other) = (sim.types.id("FACTORY").unwrap(), sim.types.id("TANK").unwrap(), sim.types.id("OTHER").unwrap());
    // Queuing while the factory is under construction is still supported.
    let f = sim.spawn_unit(factory, 0, FxVec3::from_ints(300, 0, 300), 0, false).unwrap();
    let produce = |sim: &mut Sim, type_id, count| sim.apply(&Command::Produce { player: 0, factory: f, type_id, count });
    produce(&mut sim, tank, i32::MAX);
    assert_eq!(sim.unit(f).unwrap().factory_queue.len(), FACTORY_QUEUE_MAX);
    produce(&mut sim, other, i32::MAX);
    assert_eq!(sim.unit(f).unwrap().factory_queue.len(), FACTORY_QUEUE_MAX);
    produce(&mut sim, tank, i32::MIN);
    assert!(sim.unit(f).unwrap().factory_queue.is_empty());
    produce(&mut sim, tank, i32::MIN); // Empty cancellation also has bounded work.
    for (t, count) in [(tank, 2), (other, 1), (tank, 2), (other, 1)] {
        produce(&mut sim, t, count);
    }
    produce(&mut sim, tank, -3);
    assert_eq!(sim.unit(f).unwrap().factory_queue.iter().copied().collect::<Vec<_>>(), [tank, other, other]);
    produce(&mut sim, other, i32::MIN);
    assert_eq!(sim.unit(f).unwrap().factory_queue.iter().copied().collect::<Vec<_>>(), [tank]);
}

#[test]
fn a_factory_resumes_production_after_losing_its_product() {
    use crate::cob::CobProgram;
    use ta_formats::cob::{CobFunction, CobScript, op};
    for destroyed in [false, true] {
        let mut data = construction_data();
        let tank = data.unit_id("TANK").unwrap();
        data.units[tank].build_time = 60.0;
        let mut sim = sim_of(data, 1);
        let factory_type = sim.types.id("FACTORY").unwrap();
        let mut script = CobScript { static_var_count: 2, ..Default::default() };
        for (name, counter) in [("Activate", 0), ("StopBuilding", 1)] {
            script.functions.push(CobFunction { name: name.into(), offset: script.code.len() as u32 });
            script.code.extend([op::PUSH_STATIC, counter, op::PUSH_CONSTANT, 1, op::ADD, op::POP_STATIC, counter]);
            if name == "Activate" {
                // Only activation makes this factory ready; StopBuilding does not
                // reopen its yard or reset its readiness between products.
                script.code.extend([op::PUSH_CONSTANT, crate::host::value::IN_BUILD_STANCE as u32, op::PUSH_CONSTANT, 1, op::SET_VALUE]);
            }
            script.code.extend([op::PUSH_CONSTANT, 0, op::RETURN]);
        }
        Arc::get_mut(&mut sim.types).unwrap().types[factory_type].script = Some(Arc::new(CobProgram::new(script, &[])));
        let factory = sim.spawn_unit(factory_type, 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
        sim.step(&[Command::Produce { player: 0, factory, type_id: tank, count: 2 }]);
        for _ in 0..10 { sim.step(&[]); }
        let lost = sim.unit(factory).unwrap().build.unwrap().target;
        assert!(sim.unit(lost).unwrap().build_work > 0);
        assert!(!sim.unit(lost).unwrap().is_complete());
        if destroyed {
            sim.damage_unit(lost, Fx::from_int(2000), factory, FxVec3::ZERO);
        } else {
            sim.remove_unit(lost);
        }
        let mut completed = false;
        for _ in 0..180 {
            sim.step(&[]);
            completed = sim.units().any(|u| u.id != lost && u.type_id == tank && u.is_complete());
            if completed { break; }
        }
        let f = sim.unit(factory).unwrap();
        assert!(completed, "destroyed={destroyed}: factory must replace its lost product, queue {:?}, ready {}", f.factory_queue, f.in_build_stance);
        assert_eq!(f.cob.as_ref().unwrap().statics[0], 1, "the existing activation remains ready");
        assert!(f.cob.as_ref().unwrap().statics[1] >= 1, "the lost build still receives StopBuilding");
    }
}

#[test]
fn production_cancels_and_refunds_active_frame_only_after_later_matches() {
    let mut sim = sim_of(construction_data(), 1);
    let (factory, tank) = (sim.types.id("FACTORY").unwrap(), sim.types.id("TANK").unwrap());
    let f = sim.spawn_unit(factory, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    // No COB in this fixture: put the factory in the stance normally set by its script.
    sim.unit_mut(f).unwrap().in_build_stance = true;
    sim.step(&[Command::Produce { player: 0, factory: f, type_id: tank, count: 2 }]);
    for _ in 0..10 {
        sim.step(&[]);
    }
    let frame = sim.unit(f).unwrap().build.unwrap().target;
    assert!(sim.unit(frame).unwrap().build_work > 0);
    let metal = sim.players[0].economy.metal;
    sim.apply(&Command::Produce { player: 0, factory: f, type_id: tank, count: -1 });
    assert_eq!(sim.unit(f).unwrap().build.unwrap().target, frame);
    assert_eq!(sim.players[0].economy.metal, metal);
    sim.apply(&Command::Produce { player: 0, factory: f, type_id: tank, count: i32::MIN });
    assert!(sim.unit(f).unwrap().build.is_none());
    assert!(sim.unit(frame).is_none_or(|u| u.dead));
    assert!(sim.players[0].economy.metal > metal, "the unfinished frame is refunded");
}

#[test]
fn stop_empties_a_factory_queue_and_refunds_the_unit_in_production() {
    let mut sim = sim_of(construction_data(), 1);
    let (factory, tank, other) = (sim.types.id("FACTORY").unwrap(), sim.types.id("TANK").unwrap(), sim.types.id("OTHER").unwrap());
    let f = sim.spawn_unit(factory, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    sim.unit_mut(f).unwrap().in_build_stance = true;
    sim.step(&[
        Command::Produce { player: 0, factory: f, type_id: tank, count: 3 },
        Command::Produce { player: 0, factory: f, type_id: other, count: 2 },
        Command::Order { player: 0, units: vec![f], order: Order::Move(FxVec3::from_ints(500, 0, 500)), queue: false },
    ]);
    for _ in 0..10 {
        sim.step(&[]);
    }
    let frame = sim.unit(f).unwrap().build.unwrap().target;
    let metal = sim.players[0].economy.metal;
    sim.step(&[Command::Order { player: 0, units: vec![f], order: Order::Stop, queue: false }]);
    let fu = sim.unit(f).unwrap();
    assert!(fu.factory_queue.is_empty() && fu.build.is_none() && fu.rally.is_empty());
    assert!(sim.unit(frame).is_none_or(|u| u.dead), "the unit in production is given up");
    assert!(sim.players[0].economy.metal > metal, "and refunded");
    // Nothing more is built afterwards.
    for _ in 0..60 {
        sim.step(&[]);
    }
    assert!(sim.unit(f).unwrap().build.is_none());
}

#[test]
fn build_commands_require_valid_types_builders_and_their_own_menu() {
    let mut sim = sim_of(construction_data(), 2);
    let (builder, tank, building) = (sim.types.id("BUILDER").unwrap(), sim.types.id("TANK").unwrap(), sim.types.id("BUILDING").unwrap());
    let b = sim.spawn_unit(builder, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let ordinary = sim.spawn_unit(tank, 0, FxVec3::from_ints(250, 0, 300), 0, true).unwrap();
    let enemy = sim.spawn_unit(builder, 1, FxVec3::from_ints(400, 0, 300), 0, true).unwrap();
    let site = FxVec3::from_ints(350, 0, 300);
    for invalid in [tank, usize::MAX, sim.types.types.len()] {
        sim.step(&[Command::Order { player: 0, units: vec![b, ordinary, enemy], order: Order::Build { type_id: invalid, pos: site }, queue: false }]);
        assert!(sim.unit(b).unwrap().orders.is_empty());
        assert_eq!(sim.units().count(), 3);
    }
    sim.step(&[Command::Order { player: 0, units: vec![ordinary, enemy], order: Order::Build { type_id: building, pos: site }, queue: false }]);
    assert_eq!(sim.units().count(), 3, "neither a non-builder nor another player's builder may construct");
    sim.step(&[Command::Order { player: 0, units: vec![b], order: Order::Build { type_id: building, pos: site }, queue: false }]);
    assert_eq!(sim.unit(sim.unit(b).unwrap().build.unwrap().target).unwrap().type_id, building);
    // Internal/scripted orders also cannot reach indexed access with an invalid type.
    sim.step(&[Command::Order { player: 0, units: vec![b], order: Order::Stop, queue: false }]);
    sim.give_orders(b, [Order::Build { type_id: usize::MAX, pos: site }]);
    sim.step(&[]);
    assert!(sim.unit(b).unwrap().orders.is_empty());
    assert!(!sim.is_allowed(usize::MAX));
}

#[test]
fn opposing_builders_do_not_share_nanoframes_and_assistance_rechecks_ownership() {
    let mut sim = sim_of(construction_data(), 2);
    let (builder, building) = (sim.types.id("BUILDER").unwrap(), sim.types.id("BUILDING").unwrap());
    let a = sim.spawn_unit(builder, 0, FxVec3::from_ints(280, 0, 300), 0, true).unwrap();
    let b = sim.spawn_unit(builder, 1, FxVec3::from_ints(400, 0, 300), 0, true).unwrap();
    let site = FxVec3::from_ints(336, 0, 304);
    let build = Order::Build { type_id: building, pos: site };
    sim.step(&[
        Command::Order { player: 0, units: vec![a], order: build.clone(), queue: false },
        Command::Order { player: 1, units: vec![b], order: build, queue: false },
    ]);
    let frame = sim.unit(a).unwrap().build.unwrap().target;
    assert_eq!(sim.unit(frame).unwrap().owner, 0);
    assert!(sim.unit(b).unwrap().build.is_none());
    let enemy_metal = sim.players[1].economy.metal;
    for _ in 0..10 {
        sim.step(&[]);
    }
    assert!(sim.unit(frame).unwrap().build_work > 0);
    assert_eq!(sim.players[1].economy.metal, enemy_metal);
    sim.transfer_unit(frame, 1);
    let work = sim.unit(frame).unwrap().build_work;
    let metal = sim.players[0].economy.metal;
    sim.step(&[]);
    assert!(sim.unit(a).unwrap().build.is_none());
    assert_eq!(sim.unit(frame).unwrap().build_work, work);
    assert_eq!(sim.players[0].economy.metal, metal);

    // Repair assistance is legal, but stops when its target changes sides.
    sim.transfer_unit(frame, 0);
    sim.step(&[Command::Order { player: 0, units: vec![a], order: Order::Repair(frame), queue: false }]);
    for _ in 0..3 { sim.step(&[]); }
    assert!(sim.unit(a).unwrap().build.is_some());
    sim.transfer_unit(frame, 1);
    let work = sim.unit(frame).unwrap().build_work;
    let metal = sim.players[0].economy.metal;
    sim.step(&[]);
    assert!(sim.unit(a).unwrap().build.is_none());
    assert_eq!(sim.unit(frame).unwrap().build_work, work);
    assert_eq!(sim.players[0].economy.metal, metal);
    // A guard must also stop following a captured ally.
    sim.transfer_unit(b, 0);
    sim.step(&[Command::Order { player: 0, units: vec![a], order: Order::Guard(b), queue: false }]);
    assert_eq!(sim.unit(a).unwrap().orders.front(), Some(&Order::Guard(b)));
    sim.transfer_unit(b, 1);
    sim.step(&[]);
    assert!(sim.unit(a).unwrap().orders.is_empty());
}

#[test]
fn sonar_detects_submerged_ground_units_and_respects_jamming() {
    let data = data_of(&[
        unit("COMMANDER", "Commander=1; CanMove=1; MaxVelocity=1; MaxWaterDepth=500;"),
        unit("AMPHIBIAN", "CanMove=1; MaxVelocity=1; MaxWaterDepth=500;"),
        unit("SUB", "CanMove=1; MaxVelocity=1; Floater=1; WaterLine=30;"),
    ]);
    let mut map = flat_map();
    map.sea_level = 150;
    let mut sim = sim_on_map(data, 2, &map);
    let targets: Vec<_> = ["COMMANDER", "AMPHIBIAN", "SUB"].iter().enumerate().map(|(i, name)| {
        sim.spawn_unit(sim.types.id(name).unwrap(), 1, FxVec3::from_ints(300 + i as i64 * 50, 0, 300), 0, true).unwrap()
    }).collect();
    sim.vision.set_all(0, vision::SONAR);
    for &id in &targets {
        assert!(sim.submerged(sim.unit(id).unwrap()));
        assert_eq!(sim.seen_by(sim.unit(id).unwrap(), 0), Seen::Radar);
    }
    sim.vision.set_all(1, vision::SONAR_JAM);
    for &id in &targets {
        assert_eq!(sim.seen_by(sim.unit(id).unwrap(), 0), Seen::Hidden);
    }
    sim.vision.rebuild(&[]);
    sim.vision.set_all(0, vision::RADAR);
    for &id in &targets {
        assert_eq!(sim.seen_by(sim.unit(id).unwrap(), 0), Seen::Hidden, "radar cannot see underwater");
    }
}

fn weapon_file(name: &str, keys: &str) -> (String, String) {
    (format!("weapons/{name}.tdf"), format!("[{name}] {{ range=400; reloadtime=0.1; weaponvelocity=400; {keys} [DAMAGE] {{ default=1; }} }}"))
}

#[test]
fn ceasefire_protects_against_projectiles_after_the_attacker_is_removed() {
    for protected in [false, true] {
        for (through, paralyzer) in [(false, false), (true, false), (false, true)] {
            let mut data = data_of(&[
                unit("SHOOTER", "Weapon1=SHOT;"), unit("TARGET", "FootprintX=2; FootprintZ=2;"),
                weapon_file("SHOT", &format!("areaofeffect=64; noexplode={}; paralyzer={};", through as u8, paralyzer as u8)),
            ]);
            let weapon = data.weapon_id("SHOT").unwrap();
            data.weapons[weapon].default_damage = 60.0;
            let mut sim = sim_of(data, 2);
            if protected { sim.set_mutators(crate::Mutators { no_rush_minutes: 1, ..Default::default() }); }
            let shooter = sim.spawn_unit(sim.types.id("SHOOTER").unwrap(), 0, FxVec3::from_ints(200, 60, 300), 0, true).unwrap();
            let target = sim.spawn_unit(sim.types.id("TARGET").unwrap(), 1, FxVec3::from_ints(400, 60, 300), 0, true).unwrap();
            let full = sim.unit(target).unwrap().health;
            let pos = sim.unit(target).unwrap().pos + FxVec3::from_ints(-200, 8, 0);
            sim.projectiles.push(crate::combat::Projectile {
                id: 1, weapon, owner: shooter, owner_player: 0, pos, prev_pos: pos, vel: FxVec3::from_ints(4, 0, 0),
                target: Some(Target::Unit(target)), target_pos: sim.unit(target).unwrap().pos,
                age: 0, ttl: 180, hits: Vec::new(), dead: false,
            });
            sim.remove_unit(shooter);
            let mut stunned = false;
            for _ in 0..120 {
                sim.step(&[]);
                stunned |= sim.unit(target).unwrap().paralyzed_until > sim.tick;
            }
            assert!(sim.unit(shooter).is_none());
            let damaged = sim.unit(target).unwrap().health < full;
            assert_eq!((damaged, stunned), (!protected && !paralyzer, !protected && paralyzer), "ceasefire={protected}, through={through}, EMP={paralyzer}");
        }
    }
}

fn amphibious_combat_data(weapon_keys: &str) -> GameData {
    data_of(&[
        unit("TANK", "CanMove=1; MaxVelocity=4; Acceleration=1; BrakeRate=1; TurnRate=4096; MaxWaterDepth=500; Weapon1=GUN;"),
        unit("TARGET", "CanMove=1; CanHover=1; MaxVelocity=1;"),
        weapon_file("GUN", weapon_keys),
    ])
}

#[test]
fn submerged_tanks_cannot_acquire_or_fire_surface_weapons() {
    // Automatic fire, explicit unit/ground attacks and manual fire all obey the same rule.
    for mode in 0..4 {
        let mut map = flat_map();
        map.sea_level = 100;
        let mut sim = sim_on_map(amphibious_combat_data(if mode == 3 { "commandfire=1;" } else { "" }), 2, &map);
        let tank = sim.spawn_unit(sim.types.id("TANK").unwrap(), 0, FxVec3::from_ints(200, 0, 300), 16384, true).unwrap();
        let target = sim.spawn_unit(sim.types.id("TARGET").unwrap(), 1, FxVec3::from_ints(400, 0, 350), 0, true).unwrap();
        let ground = Target::Ground(sim.unit(target).unwrap().pos);
        let order = match mode {
            0 => None,
            1 => Some(Order::Attack(Target::Unit(target))),
            2 => Some(Order::Attack(ground)),
            _ => Some(Order::ManualFire(ground)),
        };
        if let Some(order) = order { sim.give_orders(tank, [order]); }
        for _ in 0..48 {
            sim.step(&[]);
            assert!(sim.submerged(sim.unit(tank).unwrap()));
            assert!(sim.unit(tank).unwrap().weapons[0].target.is_none(), "a submerged surface gun must not hold a target (mode={mode})");
            assert!(!sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == tank)), "surface gun fired underwater (mode={mode})");
        }
    }
}

#[test]
fn submerged_fight_and_patrol_orders_ignore_stale_surface_weapon_targets() {
    for patrol in [false, true] {
        let mut map = flat_map();
        map.sea_level = 100;
        let mut sim = sim_on_map(amphibious_combat_data(""), 2, &map);
        let tank = sim.spawn_unit(sim.types.id("TANK").unwrap(), 0, FxVec3::from_ints(200, 0, 300), 16384, true).unwrap();
        let target = sim.spawn_unit(sim.types.id("TARGET").unwrap(), 1, FxVec3::from_ints(400, 0, 350), 0, true).unwrap();
        let dest = FxVec3::from_ints(900, 0, 300);
        sim.give_orders(tank, [if patrol { Order::Patrol(dest) } else { Order::Fight(dest) }]);
        // A target acquired before entering water is still present in the order phase.
        sim.unit_mut(tank).unwrap().retarget(0, Some(Target::Unit(target)));
        sim.step(&[]);
        assert!(sim.unit(tank).unwrap().path_goal.is_some(), "an unusable gun cannot interrupt the route");
        for _ in 0..120 { sim.step(&[]); }
        assert!(sim.unit(tank).unwrap().pos.x > Fx::from_int(600), "the tank must keep crossing the water (patrol={patrol})");
        assert!(!sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == tank)));
    }
}

#[test]
fn amphibious_tanks_resume_surface_combat_after_reaching_the_shore() {
    for attack in [false, true] {
        let mut map = flat_map();
        map.sea_level = 100;
        for z in 0..64 { for x in 32..64 { map.heights[z * 64 + x] = 120; } }
        let mut sim = sim_on_map(amphibious_combat_data(""), 2, &map);
        let tank = sim.spawn_unit(sim.types.id("TANK").unwrap(), 0, FxVec3::from_ints(200, 0, 300), 16384, true).unwrap();
        let target = sim.spawn_unit(sim.types.id("TARGET").unwrap(), 1, FxVec3::from_ints(700, 0, 350), 0, true).unwrap();
        sim.give_orders(tank, [if attack { Order::Attack(Target::Unit(target)) } else { Order::Fight(FxVec3::from_ints(900, 0, 300)) }]);
        let mut fired = false;
        for _ in 0..240 {
            sim.step(&[]);
            let underwater = sim.submerged(sim.unit(tank).unwrap());
            for event in sim.take_events() {
                if matches!(event, SimEvent::WeaponFired { unit, .. } if unit == tank) {
                    assert!(!underwater, "surface fire must wait until the tank emerges");
                    fired = true;
                }
            }
            if fired { break; }
        }
        assert!(fired, "the tank must cross the water and resume firing (attack={attack})");
    }
}

#[test]
fn underwater_capable_weapons_and_shallow_water_surface_guns_remain_usable() {
    for (sea, water_weapon) in [(100, true), (64, false)] {
        let mut map = flat_map();
        map.sea_level = sea;
        let mut sim = sim_on_map(amphibious_combat_data(if water_weapon { "waterweapon=1;" } else { "" }), 2, &map);
        let tank = sim.spawn_unit(sim.types.id("TANK").unwrap(), 0, FxVec3::from_ints(200, 0, 300), 16384, true).unwrap();
        let target = sim.spawn_unit(sim.types.id("TARGET").unwrap(), 1, FxVec3::from_ints(400, 0, 350), 0, true).unwrap();
        assert_eq!(sim.submerged(sim.unit(tank).unwrap()), water_weapon);
        for _ in 0..24 { sim.step(&[]); }
        assert_eq!(sim.unit(tank).unwrap().weapons[0].target, Some(Target::Unit(target)));
        assert!(sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == tank)), "a usable weapon must still fire (sea={sea})");
    }
}

#[test]
fn mixed_loadouts_return_fire_and_close_to_the_underwater_weapons_range() {
    for command_fire in [false, true] {
        let data = data_of(&[
            unit("TANK", "CanMove=1; MaxVelocity=4; Acceleration=1; BrakeRate=1; TurnRate=4096; MaxWaterDepth=500; Weapon1=GUN; Weapon2=TORPEDO;"),
            unit("TARGET", "CanMove=1; CanHover=1; MaxVelocity=1;"),
            weapon_file("GUN", ""),
            weapon_file("TORPEDO", &format!("waterweapon=1; range=160; commandfire={};", command_fire as u8)),
        ]);
        let mut map = flat_map();
        map.sea_level = 100;
        let mut sim = sim_on_map(data, 2, &map);
        let tank = sim.spawn_unit(sim.types.id("TANK").unwrap(), 0, FxVec3::from_ints(200, 0, 300), 16384, true).unwrap();
        let target = sim.spawn_unit(sim.types.id("TARGET").unwrap(), 1, FxVec3::from_ints(500, 0, 300), 0, true).unwrap();
        sim.damage_unit(tank, Fx::ONE, target, FxVec3::ZERO);
        assert!(sim.unit(tank).unwrap().weapons[0].target.is_none(), "return fire cannot assign a submerged surface gun");
        assert_eq!(sim.unit(tank).unwrap().weapons[1].target, (!command_fire).then_some(Target::Unit(target)));
        sim.give_orders(tank, [Order::Attack(Target::Unit(target))]);
        let torpedo = sim.types.get(sim.unit(tank).unwrap().type_id).weapons[1].unwrap();
        let mut fired = false;
        for _ in 0..120 {
            sim.step(&[]);
            assert!(sim.unit(tank).unwrap().weapons[0].target.is_none());
            for event in sim.take_events() {
                if let SimEvent::WeaponFired { unit, weapon, .. } = event && unit == tank {
                    assert_eq!(weapon, torpedo, "only the underwater weapon may fire");
                    fired = true;
                }
            }
        }
        assert!(fired, "the secondary underwater weapon must remain usable (command_fire={command_fire})");
        assert!(sim.unit(tank).unwrap().pos.x > Fx::from_int(300), "the longer surface gun cannot determine the stopping distance");
    }
}

#[test]
fn entering_deep_water_clears_surface_targets_and_suspends_bursts() {
    let mut map = flat_map();
    map.sea_level = 100;
    for z in 0..64 { for x in 0..16 { map.heights[z * 64 + x] = 120; } }
    let mut sim = sim_on_map(amphibious_combat_data("burst=100; burstrate=0.1;"), 2, &map);
    let tank = sim.spawn_unit(sim.types.id("TANK").unwrap(), 0, FxVec3::from_ints(200, 0, 300), 16384, true).unwrap();
    let target = sim.spawn_unit(sim.types.id("TARGET").unwrap(), 1, FxVec3::from_ints(200, 0, 500), 0, true).unwrap();
    for _ in 0..24 { sim.step(&[]); }
    assert_eq!(sim.unit(tank).unwrap().weapons[0].target, Some(Target::Unit(target)));
    assert!(sim.unit(tank).unwrap().weapons[0].burst_left > 0);
    assert!(sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == tank)));
    sim.give_orders(tank, [Order::Move(FxVec3::from_ints(800, 0, 300))]);
    let mut underwater_ticks = 0;
    for _ in 0..120 {
        sim.step(&[]);
        let events = sim.take_events();
        if sim.submerged(sim.unit(tank).unwrap()) {
            underwater_ticks += 1;
            assert!(sim.unit(tank).unwrap().weapons[0].target.is_none());
            assert!(!events.iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == tank)), "a pending burst cannot bypass the water restriction");
        }
    }
    assert!(underwater_ticks > 30, "the route must actually enter deep water");
}

#[test]
fn loaded_air_transports_hover_after_moving_or_stopping_and_land_after_unloading() {
    use crate::cob::CobProgram;
    use crate::defs::{ModelDef, PieceDef};
    use ta_formats::cob::{CobFunction, CobScript, op};

    for stop in [false, true] {
        let data = data_of(&[
            unit("TRANSPORT", "CanFly=1; CanMove=1; MaxVelocity=4; Acceleration=1; BrakeRate=1; CruiseAlt=90; TransportCapacity=1; TransportSize=2;"),
            unit("PASSENGER", "CanMove=1; MaxVelocity=1;"),
        ]);
        let mut sim = sim_of(data, 1);
        let transport = sim.types.id("TRANSPORT").unwrap();
        let passenger = sim.types.id("PASSENGER").unwrap();
        let types = Arc::get_mut(&mut sim.types).unwrap();
        // A visible passenger hangs below the aircraft, as it does on real air transports.
        types.types[transport].model = Arc::new(ModelDef {
            pieces: vec![PieceDef { name: "hook".into(), parent: None, offset: FxVec3::from_ints(0, -16, 0), top: Fx::ZERO }],
            ..Default::default()
        });
        types.types[transport].script = Some(Arc::new(CobProgram::new(CobScript {
            pieces: vec!["hook".into()],
            functions: vec![CobFunction { name: "QueryTransport".into(), offset: 0 }],
            code: vec![op::PUSH_CONSTANT, 0, op::RETURN],
            ..Default::default()
        }, &["hook".into()])));
        types.types[passenger].model = Arc::new(ModelDef { height: Fx::from_int(16), ..Default::default() });
        let carrier = sim.spawn_unit(transport, 0, FxVec3::from_ints(100, 0, 300), 0, true).unwrap();
        let cargo = sim.spawn_unit(passenger, 0, FxVec3::from_ints(160, 0, 300), 0, true).unwrap();
        sim.step(&[Command::Order { player: 0, units: vec![carrier], order: Order::Load(cargo), queue: false }]);
        for _ in 0..180 {
            sim.step(&[]);
            if sim.unit(cargo).unwrap().carried_by.is_some() { break; }
        }
        assert_eq!(sim.unit(cargo).unwrap().carried_by, Some(carrier), "the load command must pick up the passenger");
        let dest = FxVec3::from_ints(700, 0, 300);
        sim.step(&[Command::Order { player: 0, units: vec![carrier], order: Order::Move(dest), queue: false }]);
        for _ in 0..240 { sim.step(&[]); }
        assert!(sim.unit(carrier).unwrap().pos.within_xz(dest, Fx::from_int(12)));
        assert!(sim.unit(carrier).unwrap().orders.is_empty());
        if stop { sim.step(&[Command::Order { player: 0, units: vec![carrier], order: Order::Stop, queue: false }]); }
        for _ in 0..180 {
            sim.step(&[]);
            let carrier_unit = sim.unit(carrier).unwrap();
            let cargo_unit = sim.unit(cargo).unwrap();
            assert!(!carrier_unit.landed, "loaded aircraft cannot land (stop={stop})");
            assert_eq!(cargo_unit.carried_by, Some(carrier));
            assert!(cargo_unit.pos.y >= sim.world.height_at(cargo_unit.pos.x, cargo_unit.pos.z), "attached cargo cannot sink through the terrain");
        }
        sim.step(&[Command::Order { player: 0, units: vec![carrier], order: Order::Unload(dest), queue: false }]);
        for _ in 0..240 {
            sim.step(&[]);
            if sim.unit(cargo).unwrap().carried_by.is_none() { break; }
        }
        assert!(sim.unit(cargo).unwrap().carried_by.is_none(), "hovering must still allow deliberate unloading");
        assert_eq!(sim.unit(cargo).unwrap().pos.y, Fx::from_int(60));
        for _ in 0..180 { sim.step(&[]); }
        assert!(sim.unit(carrier).unwrap().cargo.is_empty());
        assert!(sim.unit(carrier).unwrap().landed, "an empty transport may land normally");
    }
}

#[test]
fn cloaking_drops_an_explicit_attack_and_its_pursuit_but_ground_fire_remains() {
    let data = data_of(&[
        unit("SHOOTER", "CanMove=1; MaxVelocity=1; Weapon1=GUN;"),
        unit("CLOAKER", "CloakCost=1; MinCloakDistance=8;"),
        weapon_file("GUN", ""),
    ]);
    let mut sim = sim_of(data, 2);
    // The cloaker acts first, so its cloak can change between order and weapon updates.
    let c = sim.spawn_unit(sim.types.id("CLOAKER").unwrap(), 1, FxVec3::from_ints(450, 0, 300), 0, true).unwrap();
    let s = sim.spawn_unit(sim.types.id("SHOOTER").unwrap(), 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![s], order: Order::Attack(Target::Unit(c)), queue: false }]);
    for _ in 0..8 { sim.step(&[]); }
    assert!(sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == s)));
    sim.step(&[Command::SetCloak { player: 1, units: vec![c], on: true }]);
    assert_eq!(sim.seen_by(sim.unit(c).unwrap(), 0), Seen::Hidden);
    for _ in 0..12 { sim.step(&[]); }
    assert!(!sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == s)));
    assert!(sim.unit(s).unwrap().orders.is_empty());

    // A pursuit already in progress must also lose its live destination.
    sim.step(&[Command::SetCloak { player: 1, units: vec![c], on: false }]);
    sim.unit_mut(c).unwrap().pos.x = Fx::from_int(820);
    sim.vision.set_all(0, vision::LOS);
    sim.step(&[Command::Order { player: 0, units: vec![s], order: Order::Attack(Target::Unit(c)), queue: false }]);
    assert!(sim.unit(s).unwrap().path_goal.is_some());
    sim.step(&[Command::SetCloak { player: 1, units: vec![c], on: true }]);
    sim.step(&[]);
    assert!(sim.unit(s).unwrap().path_goal.is_none());
    assert!(sim.unit(s).unwrap().move_request.is_none());
    assert!(sim.unit(s).unwrap().orders.is_empty());
    let ground = Target::Ground(FxVec3::from_ints(450, 60, 300));
    sim.step(&[Command::Order { player: 0, units: vec![s], order: Order::Attack(ground), queue: false }]);
    for _ in 0..8 { sim.step(&[]); }
    assert!(sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == s)));
}

#[test]
fn ground_weapon_acquires_landed_aircraft_and_releases_on_takeoff() {
    let data = data_of(&[
        unit("SHOOTER", "Weapon1=GUN;"),
        unit("FLYER", "CanFly=1; CanMove=1; MaxVelocity=2; CruiseAlt=150;"),
        weapon_file("GUN", "ballistic=1;"),
    ]);
    let mut sim = sim_of(data, 2);
    let s = sim.spawn_unit(sim.types.id("SHOOTER").unwrap(), 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let f = sim.spawn_unit(sim.types.id("FLYER").unwrap(), 1, FxVec3::from_ints(420, 0, 300), 0, true).unwrap();
    for _ in 0..150 { sim.step(&[]); }
    assert!(sim.unit(f).unwrap().landed);
    sim.take_events();
    for _ in 0..16 { sim.step(&[]); }
    assert_eq!(sim.unit(s).unwrap().weapons[0].target, Some(Target::Unit(f)));
    assert!(sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == s)));
    sim.step(&[Command::Order { player: 1, units: vec![f], order: Order::Move(FxVec3::from_ints(700, 0, 300)), queue: false }]);
    assert!(!sim.unit(f).unwrap().landed);
    sim.take_events();
    for _ in 0..16 { sim.step(&[]); }
    assert!(sim.unit(s).unwrap().weapons[0].target.is_none());
    assert!(!sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == s)));
}

#[test]
fn radar_contacts_accept_attacks_and_losing_contact_preserves_queued_orders() {
    let data = data_of(&[
        unit("SHOOTER", "CanMove=1; MaxVelocity=1; Weapon1=GUN; SightDistance=32; RadarDistance=500;"),
        unit("CLOAKER", "CloakCost=1; MinCloakDistance=8;"),
        weapon_file("GUN", ""),
    ]);
    let mut sim = sim_of(data, 2);
    let c = sim.spawn_unit(sim.types.id("CLOAKER").unwrap(), 1, FxVec3::from_ints(450, 0, 300), 0, true).unwrap();
    let s = sim.spawn_unit(sim.types.id("SHOOTER").unwrap(), 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    sim.step(&[]);
    assert_eq!(sim.seen_by(sim.unit(c).unwrap(), 0), Seen::Radar);
    let retreat = FxVec3::from_ints(100, 0, 300);
    sim.step(&[
        Command::Order { player: 0, units: vec![s], order: Order::ManualFire(Target::Unit(c)), queue: false },
        Command::Order { player: 0, units: vec![s], order: Order::Move(retreat), queue: true },
    ]);
    assert_eq!(sim.unit(s).unwrap().orders.front(), Some(&Order::ManualFire(Target::Unit(c))));
    sim.step(&[Command::SetCloak { player: 1, units: vec![c], on: true }]);
    sim.step(&[]);
    assert_eq!(sim.unit(s).unwrap().orders.front(), Some(&Order::Move(retreat)));
    sim.step(&[]);
    assert_eq!(sim.unit(s).unwrap().move_request, Some(retreat));
}

/// A factory (a builder that does not move) takes the orders its weapons fire on, as TA's do, what
/// it is making going on: manual fire with a weapon for it (Total Mayhem's and Escalation's
/// factories turn their buildpad so). Stockpiled missiles launch on Attack as well;
/// an order waiting for a missile is taken back by Stop.
#[test]
fn an_armed_factory_fires_when_told_and_goes_on_making() {
    let data = data_of(&[
        unit("SILO", "Builder=1; WorkerTime=30; BuildDistance=200; Weapon1=NUKE;"),
        unit("MISSILE", "BuildTime=100000; BuildCostMetal=100;"),
        // (A missile a thousand seconds in the making: none is ready unless granted.)
        weapon_file("NUKE", "stockpile=1; commandfire=1; targetable=1; reloadtime=1000;"),
    ]);
    let mut sim = sim_of(with_menu(data, "SILO", &["MISSILE"]), 2);
    let (silo_type, missile) = (sim.types.id("SILO").unwrap(), sim.types.id("MISSILE").unwrap());
    assert!(sim.types.get(silo_type).is_factory);
    let silo = sim.spawn_unit(silo_type, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    // No COB in this fixture: put the factory in the stance normally set by its script.
    sim.unit_mut(silo).unwrap().in_build_stance = true;
    sim.step(&[Command::Produce { player: 0, factory: silo, type_id: missile, count: 1 }]);
    for _ in 0..5 { sim.step(&[]); }
    let making = sim.unit(silo).unwrap().build.map(|b| b.target);
    assert!(making.is_some(), "it is making its missile");
    let launch = |at| Command::Order { player: 0, units: vec![silo], order: Order::Attack(Target::Ground(at)), queue: false };
    let target = FxVec3::from_ints(600, 0, 300);
    // No missile ready: the order waits, and Stop takes it back.
    sim.apply(&launch(target));
    assert_eq!(sim.unit(silo).unwrap().orders.front(), Some(&Order::Attack(Target::Ground(target))), "taken");
    sim.step(&[]);
    assert_eq!(sim.unit(silo).unwrap().orders.front(), Some(&Order::Attack(Target::Ground(target))), "kept");
    sim.step(&[Command::Order { player: 0, units: vec![silo], order: Order::Stop, queue: false }]);
    assert!(sim.unit(silo).unwrap().orders.is_empty());
    // A missile ready: it goes.
    sim.step(&[Command::Produce { player: 0, factory: silo, type_id: missile, count: 1 }]);
    for _ in 0..5 { sim.step(&[]); }
    let making = sim.unit(silo).unwrap().build.map(|b| b.target);
    sim.grant_stockpile(silo, 2);
    sim.take_events();
    sim.step(&[launch(target)]);
    let mut fired = false;
    for _ in 0..30 {
        sim.step(&[]);
        if sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == silo)) {
            fired = true;
            break;
        }
    }
    assert!(fired, "the silo launched");
    let u = sim.unit(silo).unwrap();
    assert_eq!(u.stockpile, 1);
    assert!(u.orders.is_empty(), "one launch an order");
    assert!(making.is_some() && u.build.map(|b| b.target) == making, "what it was making goes on");
    for _ in 0..120 { sim.step(&[]); }
    assert_eq!(sim.unit(silo).unwrap().stockpile, 1, "one Attack never spends the next missile");
}

/// Escalation's Big Bertha, Guardian and Sentinel are builders that do not move, factories to
/// the game: told to attack, they fire, as a tower does.
#[test]
fn a_factory_with_a_gun_takes_attack_orders() {
    let data = data_of(&[
        unit("TOWER", "Builder=1; WorkerTime=30; BuildDistance=200; Weapon1=GUN;"),
        unit("TARGET", "CanMove=1; MaxVelocity=1;"),
        weapon_file("GUN", ""),
    ]);
    let mut sim = sim_of(with_menu(data, "TOWER", &["TARGET"]), 2);
    let tower_type = sim.types.id("TOWER").unwrap();
    assert!(sim.types.get(tower_type).is_factory);
    let tower = sim.spawn_unit(tower_type, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let target = sim.spawn_unit(sim.types.id("TARGET").unwrap(), 0, FxVec3::from_ints(450, 0, 300), 0, true).unwrap();
    sim.unit_mut(tower).unwrap().fire_state = 0;
    sim.take_events();
    sim.step(&[Command::Order { player: 0, units: vec![tower], order: Order::Attack(Target::Unit(target)), queue: false }]);
    assert_eq!(sim.unit(tower).unwrap().orders.front(), Some(&Order::Attack(Target::Unit(target))));
    let mut fired = false;
    for _ in 0..30 {
        sim.step(&[]);
        fired |= sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == tower));
    }
    assert!(fired, "holding its fire, it fires when told");
}

/// Total Mayhem's vertical launchers (its missile trucks, ships and hovercraft, its EMP and
/// tactical missiles, both nukes) declare no `guidance`: their missiles still turn to the target
/// once they have climbed. A nuke leaving its tube at rest, gathering speed only when guided,
/// used to hang over its silo.
#[test]
fn a_vertically_launched_missile_comes_down_on_its_target_without_guidance() {
    let data = data_of(&[
        unit("SILO", "Weapon1=NUKE;"),
        weapon_file("NUKE", "vlaunch=1; turnrate=32768; weaponacceleration=50; weapontimer=1; flighttime=60; stockpile=1; commandfire=1; areaofeffect=100;"),
    ]);
    let mut sim = sim_of(data, 2);
    let silo = sim.spawn_unit(sim.types.id("SILO").unwrap(), 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    sim.grant_stockpile(silo, 1);
    let target = FxVec3::from_ints(600, 60, 300);
    sim.take_events();
    sim.step(&[Command::Order { player: 0, units: vec![silo], order: Order::ManualFire(Target::Ground(target)), queue: false }]);
    let mut landed = None;
    for _ in 0..900 {
        sim.step(&[]);
        if let Some(p) = sim.take_events().iter().find_map(|e| match e { SimEvent::Explosion { pos, .. } => Some(*pos), _ => None }) {
            landed = Some(p);
            break;
        }
    }
    let landed = landed.expect("the missile came down");
    assert!(landed.distance_xz(target) < Fx::from_int(48), "on its target, not at {landed:?}");
}

/// A nuke like TA's sent at a truck 2,000 px off, the truck driving away as it is launched: the
/// missile goes to where the truck stood at its launch and comes down there, as TA's did,
/// leaving the truck, 700 px on by then, unharmed. (It followed the truck wherever it went and
/// came down on it.) The same missile saying `tracks` (a mod's) follows its target down.
#[test]
fn a_nuke_sent_at_a_unit_comes_down_where_the_unit_stood_at_its_launch() {
    for tracks in [false, true] {
        let nuke = format!(
            "vlaunch=1; cruise=1; guidance=1; stockpile=1; targetable=1; commandfire=1; range=32000; weapontimer=5; flighttime=400; \
             weaponvelocity=350; weaponacceleration=50; turnrate=32768; areaofeffect=512; edgeeffectiveness=0.25; tracks={};",
            tracks as u8
        );
        let mut data = data_of(&[
            unit("SILO", "Weapon1=NUKE; FootprintX=4; FootprintZ=4;"),
            unit("TRUCK", "CanMove=1; MaxVelocity=2; Acceleration=0.5; BrakeRate=0.5; TurnRate=1000; FootprintX=2; FootprintZ=2;"),
            weapon_file("NUKE", &nuke),
        ]);
        let weapon = data.weapon_id("NUKE").unwrap();
        data.weapons[weapon].default_damage = 5500.0;
        // A flat field 4,096 px square.
        let side = 256;
        let cells = (side * side) as usize;
        let map = TntMap { width: side, height: side, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; cells], features: vec![FEATURE_NONE; cells], feature_names: Vec::new(), minimap: Default::default() };
        let mut sim = sim_on_map(data, 2, &map);
        // (The silo's side sees what it aims at: an attack order needs its target in sight.)
        sim.los_enabled = false;
        let silo = sim.spawn_unit(sim.types.id("SILO").unwrap(), 0, FxVec3::from_ints(400, 60, 2048), 0, true).unwrap();
        let truck = sim.spawn_unit(sim.types.id("TRUCK").unwrap(), 1, FxVec3::from_ints(2400, 60, 2048), 0, true).unwrap();
        sim.grant_stockpile(silo, 1);
        let full = sim.unit(truck).unwrap().health;
        sim.take_events();
        sim.step(&[
            Command::Order { player: 0, units: vec![silo], order: Order::Attack(Target::Unit(truck)), queue: false },
            // Off north, across the missile's way in.
            Command::Order { player: 1, units: vec![truck], order: Order::Move(FxVec3::from_ints(2400, 60, 600)), queue: false },
        ]);
        let (mut aimed, mut stood, mut blast) = (None, None, None);
        for _ in 0..30 * 60 {
            let events = sim.take_events();
            if let Some(&SimEvent::WeaponFired { target, .. }) = events.iter().find(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == silo)) {
                aimed = Some(target);
                stood = sim.unit(truck).map(|u| u.pos);
            }
            if let Some(&SimEvent::Explosion { pos, .. }) = events.iter().find(|e| matches!(e, SimEvent::Explosion { weapon: w, .. } if *w == weapon)) {
                blast = Some(pos);
                break;
            }
            if let (Some(aimed), Some(p)) = (aimed, sim.projectiles.iter().find(|p| p.owner == silo)) && !tracks {
                assert_eq!(p.target_pos, aimed, "the missile makes for where it was sent, never after the truck");
            }
            sim.step(&[]);
        }
        let (aimed, stood) = (aimed.expect("the silo launches"), stood.expect("the truck is there at the launch"));
        let blast = blast.expect("the missile comes down");
        let after = sim.unit(truck).filter(|u| !u.dead);
        let moved = after.map(|u| u.pos.distance_xz(stood).to_f32());
        if tracks {
            assert!(after.is_none(), "tracks: the missile follows the truck down and destroys it ({moved:?} px on)");
        } else {
            assert!(aimed.distance_xz(stood) <= Fx::from_int(2) && aimed.y == Fx::from_int(60), "sent at the ground under the truck at the launch: {:?} for {:?}", aimed.to_f32(), stood.to_f32());
            assert!(blast.distance_xz(stood) <= Fx::from_int(16), "comes down where the truck stood: {:?}, not {:?}", blast.to_f32(), stood.to_f32());
            let after = after.expect("the truck, gone from there, survives");
            assert!(moved.is_some_and(|m| m > 600.0), "the truck drove on: {moved:?} px");
            assert_eq!(after.health, full, "and is unharmed");
        }
    }
}

#[test]
fn interceptor_waits_for_emp_to_expire() {
    let data = data_of(&[
        unit("DEFENDER", "Weapon1=ANTI;"), unit("SILO", "Weapon1=NUKE;"),
        weapon_file("ANTI", "interceptor=1; stockpile=1; coverage=1000; tracks=1; turnrate=1000;"),
        weapon_file("NUKE", "targetable=1; stockpile=1; commandfire=1;"),
    ]);
    let mut sim = sim_of(data, 2);
    let defender = sim.spawn_unit(sim.types.id("DEFENDER").unwrap(), 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let silo_type = sim.types.id("SILO").unwrap();
    let silo = sim.spawn_unit(silo_type, 1, FxVec3::from_ints(900, 0, 900), 0, true).unwrap();
    sim.grant_stockpile(defender, 1);
    let pos = FxVec3::from_ints(600, 500, 300);
    sim.projectiles.push(crate::combat::Projectile {
        id: 100, weapon: sim.types.get(silo_type).weapons[0].unwrap(), owner: silo, owner_player: 1,
        pos, prev_pos: pos, vel: FxVec3::ZERO, target: None, target_pos: FxVec3::from_ints(300, 60, 300),
        age: 0, ttl: 1000, hits: Vec::new(), dead: false,
    });
    sim.paralyze_unit(defender, Fx::from_int(60), silo); // 30 ticks of EMP.
    for _ in 0..29 { sim.step(&[]); }
    assert_eq!(sim.unit(defender).unwrap().stockpile, 1);
    assert!(!sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == defender)));
    sim.step(&[]);
    assert!(sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == defender)), "interception resumes at the paralysis deadline");
}

#[test]
fn interceptors_wait_for_their_scripts_launch_readiness_and_respect_a_refusal() {
    use crate::cob::CobProgram;
    use ta_formats::cob::{CobFunction, CobScript, op};
    for answer in [0, 1] {
        let data = data_of(&[unit("DEFENDER", "Weapon1=ANTI;"), unit("SILO", "Weapon1=NUKE;"),
            weapon_file("ANTI", "interceptor=1; stockpile=1; coverage=1000; reloadtime=1000;"),
            weapon_file("NUKE", "stockpile=1; commandfire=1; targetable=1; reloadtime=1000;")]);
        let mut sim = sim_of(data, 2);
        let defender_type = sim.types.id("DEFENDER").unwrap();
        let mut script = CobScript::default();
        script.functions.push(CobFunction { name: "AimPrimary".into(), offset: 0 });
        script.code = vec![op::CREATE_LOCAL, op::CREATE_LOCAL, op::PUSH_CONSTANT, 1000, op::SLEEP, op::PUSH_CONSTANT, answer, op::RETURN];
        Arc::get_mut(&mut sim.types).unwrap().types[defender_type].script = Some(Arc::new(CobProgram::new(script, &[])));
        let defender = sim.spawn_unit(defender_type, 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
        let silo_type = sim.types.id("SILO").unwrap();
        let silo = sim.spawn_unit(silo_type, 1, FxVec3::from_ints(900, 60, 900), 0, true).unwrap();
        sim.grant_stockpile(defender, 1);
        let pos = FxVec3::from_ints(600, 500, 300);
        sim.projectiles.push(crate::Projectile { id: 100, weapon: sim.types.get(silo_type).weapons[0].unwrap(), owner: silo, owner_player: 1, pos, prev_pos: pos, vel: FxVec3::ZERO, target: None, target_pos: FxVec3::from_ints(300, 60, 300), age: 0, ttl: 1000, hits: Vec::new(), dead: false });
        for _ in 0..30 { sim.step(&[]); }
        assert_eq!(sim.unit(defender).unwrap().stockpile, 1, "the script's preparation delay must complete before launch");
        for _ in 0..15 { sim.step(&[]); }
        assert_eq!(sim.unit(defender).unwrap().stockpile, 1 - answer, "only an affirmative aim result authorizes launch");
    }
}

/// A build time 2^40 times a second (36,000 years of a Commander's work), against a builder
/// 2^28: each tick does a 122,880th of it. With progress a 16.16 fraction a tick's share was
/// zero and the build never moved; work counted whole moves exactly, and finishes on the tick.
#[test]
fn huge_build_times_progress_exactly() {
    let data = data_of(&[
        unit("BUILDER", "Builder=1;\nWorkerTime=268435456;\nMaxVelocity=1;\nCanMove=1;\nBuildDistance=200;\nBuildTime=10;"),
        unit("BIG", "BuildTime=1099511627776;\nFootprintX=2;\nFootprintZ=2;"),
    ]);
    let mut sim = sim_of(data, 1);
    let (builder, big) = (sim.types.id("BUILDER").unwrap(), sim.types.id("BIG").unwrap());
    let (rate, total) = (sim.types.get(builder).work_rate, sim.types.get(big).build_work);
    assert_eq!(rate, 1 << 44, "WorkerTime in 16.16");
    assert_eq!(total, (1u64 << 56) * 30, "BuildTime in 16.16 worker-time ticks");
    let b = sim.spawn_unit(builder, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let target = sim.spawn_unit(big, 0, FxVec3::from_ints(340, 0, 300), 0, false).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![b], order: Order::Repair(target), queue: false }]);
    let work = |sim: &Sim| sim.unit(target).unwrap().build_work;
    let mut guard = 0;
    while work(&sim) == 0 {
        sim.step(&[]);
        guard += 1;
        assert!(guard < 30, "the builder never started");
    }
    // Every tick adds exactly the builder's work, and the progress shown follows it.
    let start = work(&sim);
    for k in 1..=100u64 {
        sim.step(&[]);
        assert_eq!(work(&sim), start + k * rate);
    }
    assert_eq!(sim.unit(target).unwrap().build_progress, Fx::fraction(start + 100 * rate, total));
    assert!(sim.unit(target).unwrap().build_progress > Fx::ZERO);
    // Near the end: three ticks short of it and a little more, it finishes on the fourth.
    sim.unit_mut(target).unwrap().set_build_work(total - 3 * rate - 5, total);
    for _ in 0..3 {
        sim.step(&[]);
        assert!(!sim.unit(target).unwrap().is_complete());
    }
    assert_eq!(work(&sim), total - 5);
    sim.step(&[]);
    let u = sim.unit(target).unwrap();
    assert!(u.is_complete() && u.build_progress == Fx::ONE && u.build_work == total);
    assert_eq!(sim.players[0].stats.built, 1);
}

/// At full pace a build takes exactly its build time (here 1000 / 30 seconds: 1000 ticks) and
/// costs exactly its price, every tick paying what its share of the work is worth.
#[test]
fn builds_take_their_time_and_cost_their_price() {
    let data = data_of(&[
        unit("BUILDER", "Builder=1;\nWorkerTime=30;\nMaxVelocity=1;\nCanMove=1;\nBuildDistance=200;\nBuildTime=10;"),
        unit("MID", "BuildTime=1000;\nBuildCostMetal=777;\nBuildCostEnergy=3001;\nFootprintX=2;\nFootprintZ=2;"),
    ]);
    let mut sim = sim_of(data, 1);
    sim.set_starting_resources(0, Fx::from_int(100_000), Fx::from_int(100_000));
    let (builder, mid) = (sim.types.id("BUILDER").unwrap(), sim.types.id("MID").unwrap());
    let b = sim.spawn_unit(builder, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let target = sim.spawn_unit(mid, 0, FxVec3::from_ints(340, 0, 300), 0, false).unwrap();
    let before = (sim.players[0].economy.metal, sim.players[0].economy.energy);
    sim.step(&[Command::Order { player: 0, units: vec![b], order: Order::Repair(target), queue: false }]);
    let mut ticks = 0;
    for _ in 0..1100 {
        let was = sim.unit(target).unwrap().build_work;
        sim.step(&[]);
        if sim.unit(target).unwrap().build_work > was {
            ticks += 1;
        }
        if sim.unit(target).unwrap().is_complete() {
            break;
        }
    }
    assert!(sim.unit(target).unwrap().is_complete());
    assert_eq!(ticks, 1000);
    let e = &sim.players[0].economy;
    assert_eq!(before.0 - e.metal, Fx::from_int(777));
    assert_eq!(before.1 - e.energy, Fx::from_int(3001));
    assert_eq!(sim.unit(target).unwrap().health, sim.types.get(mid).max_health);
}

/// Category tokens past the 64th (stock TA uses 54; mods add more) still count: a gun told
/// to leave the 71st for last does.
#[test]
fn bad_targets_past_sixty_four_tokens_come_last() {
    let mut files: Vec<(String, String)> = (0..80).map(|i| unit(&format!("U{i}"), &format!("Category=COMMON TOKEN{i};"))).collect();
    files.push(unit("SHOOTER", "Weapon1=GUN;\nBadTargetCategory=TOKEN70;\nCategory=COMMON;"));
    files.push(("weapons/gun.tdf".into(), "[GUN]\n{\nrange=400;\nreloadtime=3;\nweaponvelocity=400;\n[DAMAGE]\n{\ndefault=1;\n}\n}\n".into()));
    let mut sim = sim_of(data_of(&files), 2);
    let types = sim.types.clone();
    let (shooter, bad, good) = (types.id("SHOOTER").unwrap(), types.id("U70").unwrap(), types.id("U5").unwrap());
    assert!(types.get(bad).category_mask.intersects(&types.get(shooter).bad_target_mask));
    assert!(!types.get(good).category_mask.intersects(&types.get(shooter).bad_target_mask));
    assert_eq!(types.categories("token70"), types.get(shooter).bad_target_mask);
    let s = sim.spawn_unit(shooter, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    sim.spawn_unit(bad, 1, FxVec3::from_ints(360, 0, 300), 0, true).unwrap();
    let far = sim.spawn_unit(good, 1, FxVec3::from_ints(420, 0, 300), 0, true).unwrap();
    for _ in 0..10 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(s).unwrap().weapons[0].target, Some(Target::Unit(far)), "the nearer one is a bad target");
}

#[test]
fn category_sets_hold_their_capacity() {
    let mut s = crate::defs::CategorySet::EMPTY;
    assert!(s.insert(0) && s.insert(64) && s.insert(255));
    assert!(!s.insert(256), "past the capacity");
    assert!(s.contains(64) && s.contains(255) && !s.contains(63) && !s.contains(256));
    let (mut t, mut u) = (crate::defs::CategorySet::EMPTY, crate::defs::CategorySet::EMPTY);
    t.insert(255);
    u.insert(254);
    assert!(s.intersects(&t) && !s.intersects(&u));
    assert!(crate::defs::CategorySet::EMPTY.is_empty() && !s.is_empty());
}

/// Forty players: masks of players past the 16th (cloak reveals) and the 32nd (the unit grid,
/// enemies) are their own bits, not another player's.
#[test]
fn players_past_sixteen_and_thirty_two_have_bits_of_their_own() {
    let files = vec![
        unit("SHOOTER", "Weapon1=GUN;"),
        unit("TANK", "Category=TANK;"),
        unit("CLOAKER", "CloakCost=1;\nInitCloaked=1;"),
        ("weapons/gun.tdf".into(), "[GUN]\n{\nrange=400;\nreloadtime=3;\nweaponvelocity=400;\n[DAMAGE]\n{\ndefault=1;\n}\n}\n".into()),
    ];
    let mut sim = sim_of(data_of(&files), 40);
    // Everyone for themselves, but players 6 and 7 fight beside 39 (where 38 and 39 would land
    // in 32-bit masks).
    for p in 0..40u8 {
        sim.set_team(p, p);
    }
    for p in [6, 7, 39] {
        sim.set_team(p, 100);
    }
    assert_eq!(sim.enemy_mask(39) & (1 << 38 | 1 << 6 | 1 << 7 | 1 << 39), 1 << 38);
    assert_eq!(sim.team_mask(39), 1 << 6 | 1 << 7 | 1 << 39);
    let types = sim.types.clone();
    let (shooter, tank, cloaker) = (types.id("SHOOTER").unwrap(), types.id("TANK").unwrap(), types.id("CLOAKER").unwrap());
    // Player 39 finds player 38's tank in the unit grid.
    let s = sim.spawn_unit(shooter, 39, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let t = sim.spawn_unit(tank, 38, FxVec3::from_ints(400, 0, 300), 0, true).unwrap();
    // Player 38's cloaked unit, with player 20's beside it (and 21's far off).
    let c = sim.spawn_unit(cloaker, 38, FxVec3::from_ints(700, 0, 700), 0, true).unwrap();
    sim.spawn_unit(tank, 20, FxVec3::from_ints(712, 0, 700), 0, true).unwrap();
    sim.spawn_unit(tank, 21, FxVec3::from_ints(100, 0, 900), 0, true).unwrap();
    for _ in 0..14 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(s).unwrap().weapons[0].target, Some(Target::Unit(t)));
    let cu = sim.unit(c).unwrap();
    assert!(cu.cloaked);
    assert_eq!(cu.revealed_to, 1 << 20, "player 20's unit is close enough to see through it");
    assert_eq!(sim.seen_by(cu, 20), Seen::Visible);
    assert_eq!(sim.seen_by(cu, 21), Seen::Hidden);
}

/// An aircraft put in the air (a staged scene's): at its cruising height, at speed and heading
/// as it was put, flying on as told; at a height of its own if given one; with nothing to do it
/// comes down as any does; and the same game twice is the same game.
#[test]
fn aircraft_can_be_put_in_the_air() {
    let flyer = || unit("FLYER", "CanFly=1;\nCanMove=1;\nMaxVelocity=6;\nAcceleration=0.2;\nBrakeRate=0.4;\nTurnRate=600;\nCruiseAlt=150;\nFootprintX=2;\nFootprintZ=2;");
    let fly = |height: Option<Fx>, ordered: bool, ticks: u32| {
        let mut sim = sim_of(data_of(&[flyer()]), 1);
        let t = sim.types.id("FLYER").unwrap();
        let at = FxVec3::from_ints(200, 0, 500);
        let id = sim.spawn_airborne(t, 0, at, 16384, height).unwrap();
        let ground = sim.world.height_at(at.x, at.z);
        let u = sim.unit(id).unwrap();
        assert!(!u.landed && u.activated && u.moving);
        assert_eq!((u.speed, u.heading, u.pos.y), (sim.types.get(t).max_speed, 16384, ground + height.unwrap_or(Fx::from_int(150))));
        let order = ordered.then(|| Command::Order { player: 0, units: vec![id], order: Order::Move(FxVec3::from_ints(900, 0, 500)), queue: false });
        sim.step(&order.into_iter().collect::<Vec<_>>());
        for _ in 1..ticks {
            sim.step(&[]);
        }
        let u = sim.unit(id).unwrap();
        (u.pos, u.speed, u.landed, sim.checksum(), ground)
    };
    // Flying on east at full speed, at its height (a second: 30 ticks of 6 px).
    let (pos, speed, landed, check, ground) = fly(None, true, 30);
    assert!(!landed && speed == Fx::from_int(6) && pos.y == ground + Fx::from_int(150));
    assert_eq!(pos.x, Fx::from_int(200 + 30 * 6));
    // Low, at a height of its own.
    let (low, ..) = fly(Some(Fx::from_int(40)), true, 30);
    assert_eq!(low.y, ground + Fx::from_int(40));
    // Nothing to do: it slows and lands.
    let (_, speed, landed, ..) = fly(None, false, 300);
    assert!(landed && speed.is_zero());
    // Deterministic.
    assert_eq!(fly(None, true, 30).3, check);
}


#[test]
fn a_unit_changing_sides_mid_tick_is_its_new_owners_to_the_target_searches() {
    let data = data_of(&[unit("A", ""), unit("B", "")]);
    let mut sim = sim_of(data, 2);
    let a = sim.spawn_unit(sim.types.id("A").unwrap(), 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let b = sim.spawn_unit(sim.types.id("B").unwrap(), 0, FxVec3::from_ints(360, 0, 300), 0, true).unwrap();
    sim.step(&[]);
    let mut near = Vec::new();
    sim.grid.query_owned(FxVec3::from_ints(300, 0, 300), Fx::from_int(200), crate::unit::player_bit(1), &mut near);
    assert!(near.is_empty(), "no cell holds one of player 1's");
    // Taken over after the grid was made, as a capture is in the tick.
    sim.transfer_unit(b, 1);
    sim.grid.query_owned(FxVec3::from_ints(300, 0, 300), Fx::from_int(200), crate::unit::player_bit(1), &mut near);
    assert!(near.contains(&b.slot) && near.contains(&a.slot), "its cell is searched for player 1's units at once: {near:?}");
}

#[test]
fn a_transferred_factory_keeps_its_active_frame_ahead_of_new_orders() {
    for captured in [false, true] {
        let mut sim = sim_of(construction_data(), 2);
        sim.players.iter_mut().for_each(|p| p.endless = true);
        let (factory, tank, other) = (sim.types.id("FACTORY").unwrap(), sim.types.id("TANK").unwrap(), sim.types.id("OTHER").unwrap());
        let f = sim.spawn_unit(factory, 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
        sim.unit_mut(f).unwrap().in_build_stance = true;
        sim.step(&[Command::Produce { player: 0, factory: f, type_id: tank, count: 3 }]);
        sim.step(&[]);
        let frame = sim.unit(f).unwrap().build.unwrap().target;
        if captured {
            sim.transfer_unit(f, 1);
        } else {
            sim.set_team(1, 0);
            sim.apply(&Command::GiveUnits { player: 0, units: vec![f], to: 1 });
            assert!(sim.events.iter().any(|event| matches!(event, SimEvent::UnitsGiven { count: 2, .. })), "the gift includes the product on its pad");
        }
        assert_eq!(sim.unit(frame).unwrap().owner, 1);
        assert_eq!(sim.unit(f).unwrap().factory_queue.iter().copied().collect::<Vec<_>>(), [tank], "only the frame in production transfers with the factory");
        sim.apply(&Command::Produce { player: 1, factory: f, type_id: other, count: 2 });
        sim.finish_frame(frame);
        sim.step(&[]);
        assert_eq!(sim.unit(f).unwrap().factory_queue.iter().copied().collect::<Vec<_>>(), [other, other], "finishing the inherited frame must not remove a new order");
    }
}

#[test]
fn a_nested_transport_gift_reports_every_transferred_unit() {
    let data = data_of(&[
        unit("TRANSPORT", "CanMove=1; MaxVelocity=1; TransportCapacity=2; TransportSize=2;"),
        unit("PASSENGER", "CanMove=1; MaxVelocity=1;"),
    ]);
    let mut sim = sim_of(data, 2);
    sim.set_team(1, 0);
    let transport = sim.types.id("TRANSPORT").unwrap();
    let passenger = sim.types.id("PASSENGER").unwrap();
    let outer = sim.spawn_unit(transport, 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
    let inner = sim.spawn_unit(transport, 0, FxVec3::from_ints(340, 60, 300), 0, true).unwrap();
    let cargo = sim.spawn_unit(passenger, 0, FxVec3::from_ints(380, 60, 300), 0, true).unwrap();
    sim.attach(inner, cargo, None);
    sim.attach(outer, inner, None);
    sim.apply(&Command::GiveUnits { player: 0, units: vec![outer], to: 1 });
    assert!(sim.events.iter().any(|event| matches!(event, SimEvent::UnitsGiven { count: 3, .. })));
    assert_eq!((sim.players[0].unit_count, sim.players[1].unit_count), (0, 3));
}

#[test]
fn a_commander_never_changes_hands_inside_a_transport() {
    let data = data_of(&[
        unit("TRANSPORT", "CanMove=1; MaxVelocity=1; TransportCapacity=2; TransportSize=2;"),
        unit("BOSS", "CanMove=1; MaxVelocity=1; Commander=1;"),
    ]);
    let mut sim = sim_of(data, 2);
    sim.set_team(1, 0);
    let (transport, boss) = (sim.types.id("TRANSPORT").unwrap(), sim.types.id("BOSS").unwrap());
    let carrier = sim.spawn_unit(transport, 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
    let commander = sim.spawn_unit(boss, 0, FxVec3::from_ints(340, 60, 300), 0, true).unwrap();
    sim.attach(carrier, commander, None);
    // Given to an ally: kept back (its Commander with it), as a Commander alone is.
    sim.apply(&Command::GiveUnits { player: 0, units: vec![carrier], to: 1 });
    assert_eq!(sim.unit(carrier).unwrap().owner, 0);
    assert_eq!(sim.unit(commander).unwrap().owner, 0);
}

#[test]
fn an_oversized_transport_gift_does_not_block_smaller_units_that_fit() {
    for action in 0..3 {
        let data = data_of(&[
            unit("TRANSPORT", "CanMove=1; MaxVelocity=1; TransportCapacity=2; TransportSize=2;"),
            unit("PASSENGER", "CanMove=1; MaxVelocity=1;"),
        ]);
        let mut sim = sim_of(data, 2);
        sim.set_team(1, 0);
        let (transport, passenger) = (sim.types.id("TRANSPORT").unwrap(), sim.types.id("PASSENGER").unwrap());
        let carrier = sim.spawn_unit(transport, 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
        let cargo = sim.spawn_unit(passenger, 0, FxVec3::from_ints(340, 60, 300), 0, true).unwrap();
        let loose = sim.spawn_unit(passenger, 0, FxVec3::from_ints(380, 60, 300), 0, true).unwrap();
        sim.attach(carrier, cargo, None);
        assert_eq!(sim.unit(cargo).unwrap().carried_by, Some(carrier));
        // The carrier and its cargo cannot fit, but the unladen unit can. This also
        // matters when a departing player's army is inherited automatically.
        sim.unit_limit = 1;
        let command = match action {
            0 => Command::GiveUnits { player: 0, units: vec![carrier, cargo, loose, loose], to: 1 },
            1 => Command::Abandon { player: 0 },
            _ => Command::Resign { player: 0 },
        };
        sim.apply(&command);
        assert_eq!(sim.unit(loose).unwrap().owner, 1, "eligible later units must still transfer (action {action})");
        assert_eq!(sim.unit(carrier).unwrap().owner, 0);
        assert_eq!(sim.unit(cargo).unwrap().owner, 0);
        assert_eq!(sim.unit(cargo).unwrap().carried_by, Some(carrier));
        assert_eq!((sim.players[0].unit_count, sim.players[1].unit_count), (2, 1));
        assert!(sim.events.iter().any(|event| matches!(event, SimEvent::UnitsGiven { from: 0, to: 1, count: 1 })));
    }
}

#[test]
fn units_taken_apart_by_their_own_side_are_no_battle_loss() {
    let data = data_of(&[
        unit("BUILDER", "Builder=1; WorkerTime=1000; MaxVelocity=1; CanMove=1; BuildDistance=200; CanReclamate=1;"),
        unit("WALL", "BuildTime=10; BuildCostMetal=10; FootprintX=1; FootprintZ=1;"),
    ]);
    let mut sim = sim_of(data, 1);
    let (builder, wall) = (sim.types.id("BUILDER").unwrap(), sim.types.id("WALL").unwrap());
    let b = sim.spawn_unit(builder, 0, FxVec3::from_ints(150, 60, 150), 0, true).unwrap();
    let w = sim.spawn_unit(wall, 0, FxVec3::from_ints(200, 60, 150), 0, true).unwrap();
    sim.apply(&Command::Order { player: 0, units: vec![b], order: Order::ReclaimUnit(w), queue: false });
    for _ in 0..600 {
        sim.step(&[]);
        if sim.unit(w).is_none() {
            break;
        }
    }
    assert!(sim.unit(w).is_none(), "taken apart");
    assert_eq!(sim.players[0].stats.lost, 0, "no battle loss");
}

#[test]
fn a_death_names_its_wreck_and_a_unit_taken_apart_is_no_death() {
    let data = data_of(&[
        unit("TOWER", "BuildTime=10; BuildCostMetal=10; FootprintX=2; FootprintZ=2; Corpse=TOWER_DEAD;"),
        unit("BUILDER", "Builder=1; WorkerTime=1000; MaxVelocity=1; CanMove=1; BuildDistance=200; CanReclamate=1;"),
        ("features/all/tower_dead.tdf".to_string(), "[TOWER_DEAD]\n{\nworld=all;\nfootprintx=2;\nfootprintz=2;\nblocking=1;\nreclaimable=1;\nmetal=5;\n}\n".to_string()),
    ]);
    let mut sim = sim_of(data, 1);
    let (tower, builder) = (sim.types.id("TOWER").unwrap(), sim.types.id("BUILDER").unwrap());
    let b = sim.spawn_unit(builder, 0, FxVec3::from_ints(150, 60, 450), 0, true).unwrap();
    // Shot dead (a light last blow: its wreck stays), and a frame shot before it was finished.
    let shot = sim.spawn_unit(tower, 0, FxVec3::from_ints(200, 60, 200), 0, true).unwrap();
    let frame = sim.spawn_unit(tower, 0, FxVec3::from_ints(500, 60, 200), 0, false).unwrap();
    for id in [shot, frame] {
        let u = sim.unit_mut(id).unwrap();
        (u.health, u.last_damage, u.dead) = (Fx::ZERO, Fx::ONE, true);
    }
    sim.step(&[]);
    let events = sim.take_events();
    // The wreck names the unit it is of, and comes before the unit is gone (the client shows
    // what is left of a unit once, its wreck or its own model).
    let wrecks: Vec<(usize, usize, UnitId)> = events
        .iter()
        .enumerate()
        .filter_map(|(at, e)| match e {
            SimEvent::FeatureCreated { index, wreck_of: Some(of) } => Some((at, *index, *of)),
            _ => None,
        })
        .collect();
    assert_eq!(wrecks.len(), 1, "a frame leaves no wreck: {events:?}");
    let (created_at, index, of) = wrecks[0];
    assert_eq!(of, shot);
    assert_eq!(sim.data.features[sim.features[index].def].name.to_ascii_uppercase(), "TOWER_DEAD");
    let gone = |id: UnitId| {
        events.iter().enumerate().find_map(|(at, e)| match e {
            SimEvent::UnitDestroyed { id: dead, removed, complete, .. } if *dead == id => Some((at, *removed, *complete)),
            _ => None,
        })
    };
    let (gone_at, removed, complete) = gone(shot).expect("the tower is gone");
    assert!(created_at < gone_at && !removed && complete);
    assert_eq!(gone(frame).map(|g| (g.1, g.2)), Some((false, false)), "killed, unfinished");
    // Taken apart by its own side: gone, and no death.
    let w = sim.spawn_unit(tower, 0, FxVec3::from_ints(250, 60, 450), 0, true).unwrap();
    sim.apply(&Command::Order { player: 0, units: vec![b], order: Order::ReclaimUnit(w), queue: false });
    let mut taken = None;
    for _ in 0..600 {
        sim.step(&[]);
        for e in sim.take_events() {
            match e {
                SimEvent::UnitDestroyed { id, removed, .. } if id == w => taken = Some(removed),
                SimEvent::FeatureCreated { wreck_of: Some(of), .. } => panic!("{of:?} left a wreck"),
                _ => {}
            }
        }
        if taken.is_some() {
            break;
        }
    }
    assert_eq!(taken, Some(true), "taken apart, not killed");
}

#[test]
fn blocked_surface_unloading_reports_failure_and_continues_the_queue() {
    use ta_formats::cob::{CobFunction, CobScript, op};
    use crate::cob::CobProgram;
    for scripted in [false, true] {
        let data = data_of(&[
            unit("TRANSPORT", "CanMove=1; CanHover=1; MaxVelocity=1; TransportCapacity=2; TransportSize=2; BuildDistance=200; FootprintX=1; FootprintZ=1;"),
            unit("PASSENGER", "CanMove=1; MaxVelocity=1; MaxWaterDepth=0; FootprintX=1; FootprintZ=1;"),
        ]);
        let mut map = flat_map();
        map.sea_level = 50;
        for z in 0..64 { for x in 32..64 { map.heights[z * 64 + x] = 0; } }
        let mut sim = sim_on_map(data, 1, &map);
        sim.players[0].endless = true;
        let transport = sim.types.id("TRANSPORT").unwrap();
        if scripted {
            // The boom cannot drop on water and returns without releasing its passenger.
            Arc::get_mut(&mut sim.types).unwrap().types[transport].script = Some(Arc::new(CobProgram::new(CobScript {
                functions: vec![CobFunction { name: "TransportDrop".into(), offset: 0 }],
                code: vec![op::PUSH_CONSTANT, 0, op::RETURN],
                ..Default::default()
            }, &[])));
        }
        let passenger = sim.types.id("PASSENGER").unwrap();
        let carrier = sim.spawn_unit(transport, 0, FxVec3::from_ints(500, 60, 300), 0, true).unwrap();
        let cargo = sim.spawn_unit(passenger, 0, FxVec3::from_ints(460, 60, 300), 0, true).unwrap();
        sim.attach(carrier, cargo, None);
        let sea = FxVec3::from_ints(700, 50, 300);
        sim.give_orders(carrier, [Order::Unload(sea), Order::Wait(300)]);
        let mut failures = 0;
        for _ in 0..12 {
            sim.step(&[]);
            failures += sim.take_events().iter().filter(|event| matches!(event, SimEvent::OrderFailed { unit, reason: "no room to unload" } if *unit == carrier)).count();
        }
        assert_eq!(failures, 1, "a failed unload must be reported once (scripted={scripted})");
        assert!(matches!(sim.unit(carrier).unwrap().orders.front(), Some(Order::Wait(_))), "an impossible unload cannot stall queued orders");
        assert_eq!(sim.unit(cargo).unwrap().carried_by, Some(carrier));
        let shore = FxVec3::from_ints(440, 60, 300);
        sim.apply(&Command::Order { player: 0, units: vec![carrier], order: Order::Unload(shore), queue: false });
        for _ in 0..12 { sim.step(&[]); }
        assert_eq!(sim.unit(cargo).unwrap().carried_by, None, "the retained cargo can still be unloaded on valid ground");
    }
}


#[test]
fn reclaiming_a_factory_releases_its_unfinished_product_for_repair() {
    let mut data = construction_data();
    let builder = data.unit_id("BUILDER").unwrap();
    data.units[builder].can_reclaim = true;
    let mut sim = sim_of(data, 1);
    sim.players[0].endless = true;
    let (factory, tank) = (sim.types.id("FACTORY").unwrap(), sim.types.id("TANK").unwrap());
    let f = sim.spawn_unit(factory, 0, FxVec3::from_ints(320, 60, 320), 0, true).unwrap();
    let b = sim.spawn_unit(builder, 0, FxVec3::from_ints(240, 60, 320), 0, true).unwrap();
    sim.unit_mut(f).unwrap().in_build_stance = true;
    sim.step(&[Command::Produce { player: 0, factory: f, type_id: tank, count: 1 }]);
    sim.step(&[]);
    let frame = sim.unit(f).unwrap().build.unwrap().target;
    sim.step(&[Command::Order { player: 0, units: vec![b], order: Order::ReclaimUnit(f), queue: false }]);
    // (By its metal: 100, twenty times that in worker time at 30 a second.)
    for _ in 0..2200 {
        if sim.unit(f).is_none() { break; }
        sim.step(&[]);
    }
    assert!(sim.unit(f).is_none(), "the factory was fully reclaimed");
    assert_eq!(sim.unit(frame).unwrap().factory, None, "a frame cannot remain tied to a removed factory");
    sim.step(&[Command::Order { player: 0, units: vec![b], order: Order::Repair(frame), queue: false }]);
    for _ in 0..1100 {
        if sim.unit(frame).unwrap().is_complete() { break; }
        sim.step(&[]);
    }
    assert!(sim.unit(frame).unwrap().is_complete(), "a constructor can finish the abandoned frame");
}


#[test]
fn replacing_transport_orders_clears_the_old_job_and_ignores_late_script_requests() {
    use crate::host::HostRequest;
    let data = data_of(&[
        unit("TRANSPORT", "CanMove=1; MaxVelocity=1; TransportCapacity=2; TransportSize=2;"),
        unit("PASSENGER", "CanMove=1; MaxVelocity=1;"),
    ]);
    let mut sim = sim_of(data, 1);
    let transport = sim.types.id("TRANSPORT").unwrap();
    let passenger = sim.types.id("PASSENGER").unwrap();
    let carrier = sim.spawn_unit(transport, 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
    let first = sim.spawn_unit(passenger, 0, FxVec3::from_ints(320, 60, 300), 0, true).unwrap();
    let second = sim.spawn_unit(passenger, 0, FxVec3::from_ints(340, 60, 300), 0, true).unwrap();
    sim.give_orders(carrier, [Order::Load(first)]);
    // The previous pickup's script is still animating when the player changes its target.
    sim.unit_mut(carrier).unwrap().transport_job = Some((first, sim.tick));
    sim.unit_mut(carrier).unwrap().hover_alt = Some(Fx::from_int(80));
    sim.apply(&Command::Order { player: 0, units: vec![carrier], order: Order::Load(second), queue: false });
    assert_eq!(sim.unit(carrier).unwrap().transport_job, None);
    assert_eq!(sim.unit(carrier).unwrap().hover_alt, None);
    sim.host_requests.push(HostRequest::Attach { transport: carrier, unit: first.script_id(), piece: -1 });
    sim.apply_host_requests();
    assert_eq!(sim.unit(first).unwrap().carried_by, None, "a superseded script cannot pick up its old target");
    sim.host_requests.push(HostRequest::Attach { transport: carrier, unit: second.script_id(), piece: -1 });
    sim.apply_host_requests();
    assert_eq!(sim.unit(second).unwrap().carried_by, Some(carrier), "the current pickup remains valid");
    sim.apply(&Command::Order { player: 0, units: vec![carrier], order: Order::Stop, queue: false });
    sim.host_requests.push(HostRequest::Drop { transport: carrier, unit: second.script_id() });
    sim.apply_host_requests();
    assert_eq!(sim.unit(second).unwrap().carried_by, Some(carrier), "a cancelled unload keeps its passenger aboard");
    sim.apply(&Command::Order { player: 0, units: vec![carrier], order: Order::Unload(FxVec3::from_ints(340, 60, 300)), queue: false });
    sim.host_requests.push(HostRequest::Drop { transport: carrier, unit: second.script_id() });
    sim.apply_host_requests();
    assert_eq!(sim.unit(second).unwrap().carried_by, None, "the current unload remains valid");
    assert!(sim.unit_by_script(i32::MIN).is_none());
}

/// A skirmish's mutators: income in quarters of TA's, build power so many times TA's, damage
/// divided by how tough units are (in quarters).
#[test]
fn mutators_scale_income_build_power_and_damage() {
    let data = || {
        data_of(&[
            unit("MAKER", "EnergyMake=30; MetalMake=3;"),
            unit("BUILDER", "Builder=1; WorkerTime=30; MaxVelocity=1; CanMove=1; BuildDistance=200; BuildTime=10;"),
            unit("MID", "BuildTime=1000; BuildCostMetal=10; FootprintX=2; FootprintZ=2;"),
        ])
    };
    let income = |mutators: crate::Mutators| {
        let mut sim = sim_of(data(), 1);
        sim.set_mutators(mutators);
        sim.spawn_unit(sim.types.id("MAKER").unwrap(), 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
        sim.step(&[]);
        let e = &sim.players[0].economy;
        (e.tick_energy_in, e.tick_metal_in)
    };
    let ta = income(crate::Mutators::default());
    assert!(ta.0 > Fx::ZERO && ta.1 > Fx::ZERO);
    assert_eq!(income(crate::Mutators { resources: 8, ..Default::default() }), (ta.0 * 2, ta.1 * 2));
    assert_eq!(income(crate::Mutators { resources: 2, ..Default::default() }), (ta.0 / 2, ta.1 / 2));

    // (Ticks the frame grows in: a thousand as TA has it, a third of them three times as fast.)
    let build = |mutators: crate::Mutators| {
        let mut sim = sim_of(data(), 1);
        sim.set_mutators(mutators);
        sim.set_starting_resources(0, Fx::from_int(100_000), Fx::from_int(100_000));
        let b = sim.spawn_unit(sim.types.id("BUILDER").unwrap(), 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
        let target = sim.spawn_unit(sim.types.id("MID").unwrap(), 0, FxVec3::from_ints(340, 0, 300), 0, false).unwrap();
        sim.step(&[Command::Order { player: 0, units: vec![b], order: Order::Repair(target), queue: false }]);
        let mut ticks = 0;
        for _ in 0..1100 {
            let was = sim.unit(target).unwrap().build_work;
            sim.step(&[]);
            if sim.unit(target).unwrap().build_work > was {
                ticks += 1;
            }
            if sim.unit(target).unwrap().is_complete() {
                break;
            }
        }
        assert!(sim.unit(target).unwrap().is_complete());
        ticks
    };
    assert_eq!(build(crate::Mutators::default()), 1000);
    assert!((333..=334).contains(&build(crate::Mutators { build_speed: 3, ..Default::default() })));

    let hit = |mutators: crate::Mutators| {
        let mut sim = sim_of(data(), 2);
        sim.set_mutators(mutators);
        let maker = sim.types.id("MAKER").unwrap();
        let a = sim.spawn_unit(maker, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
        let b = sim.spawn_unit(maker, 1, FxVec3::from_ints(500, 0, 300), 0, true).unwrap();
        sim.damage_unit(b, Fx::from_int(100), a, FxVec3::ZERO);
        sim.types.get(maker).max_health - sim.unit(b).unwrap().health
    };
    assert_eq!(hit(crate::Mutators::default()), Fx::from_int(100));
    assert_eq!(hit(crate::Mutators { toughness: 8, ..Default::default() }), Fx::from_int(50));
    assert_eq!(hit(crate::Mutators { toughness: 2, ..Default::default() }), Fx::from_int(200));
}

/// No rush: while the ceasefire lasts no weapon harms an enemy (an ally's or one's own shots
/// still hurt) and nobody builds near an enemy's start; then the war is on.
#[test]
fn a_ceasefire_spares_enemies_until_it_ends() {
    let data = data_of(&[
        unit("BUILDER", "Builder=1; WorkerTime=30; MaxVelocity=1; CanMove=1; BuildDistance=200;"),
        unit("TOWER", "BuildTime=10; BuildCostMetal=10; FootprintX=2; FootprintZ=2;"),
    ]);
    let mut sim = sim_of(with_menu(data, "BUILDER", &["TOWER"]), 3);
    sim.set_mutators(crate::Mutators { no_rush_minutes: 1, ..Default::default() });
    // Player 2 is player 0's ally; player 1 their enemy, across the map.
    sim.players[2].team = 0;
    for (p, (x, z)) in [(100, 100), (900, 900), (100, 900)].into_iter().enumerate() {
        sim.players[p].start = FxVec3::from_ints(x, 60, z);
    }
    let (builder, tower) = (sim.types.id("BUILDER").unwrap(), sim.types.id("TOWER").unwrap());
    let ours = sim.spawn_unit(builder, 0, FxVec3::from_ints(150, 60, 150), 0, true).unwrap();
    let theirs = sim.spawn_unit(builder, 1, FxVec3::from_ints(850, 60, 850), 0, true).unwrap();
    let ally = sim.spawn_unit(builder, 2, FxVec3::from_ints(150, 60, 850), 0, true).unwrap();
    let health = |sim: &Sim, id| sim.unit(id).unwrap().health;
    assert_eq!(sim.ceasefire_left(), Some(1800));

    let full = health(&sim, theirs);
    sim.damage_unit(theirs, Fx::from_int(100), ours, FxVec3::ZERO);
    sim.paralyze_unit(theirs, Fx::from_int(600), ours);
    assert_eq!(health(&sim, theirs), full);
    assert!(sim.unit(theirs).unwrap().paralyzed_until <= sim.tick);
    sim.damage_unit(ally, Fx::from_int(100), ours, FxVec3::ZERO);
    assert!(health(&sim, ally) < full);

    // (Two fifths of the way to the enemy's start is its home ground: about 450 round it.)
    let build = |sim: &mut Sim, x: i64, z: i64| {
        let order = Order::Build { type_id: tower, pos: FxVec3::from_ints(x, 60, z) };
        sim.apply(&Command::Order { player: 0, units: vec![ours], order, queue: false });
        let given = !sim.unit(ours).unwrap().orders.is_empty();
        sim.apply(&Command::Order { player: 0, units: vec![ours], order: Order::Stop, queue: false });
        given
    };
    assert!(!build(&mut sim, 800, 800));
    assert!(build(&mut sim, 500, 500));

    sim.tick = 1800;
    assert_eq!(sim.ceasefire_left(), None);
    sim.damage_unit(theirs, Fx::from_int(100), ours, FxVec3::ZERO);
    assert_eq!(health(&sim, theirs), full - Fx::from_int(100));
    assert!(build(&mut sim, 800, 800));
}

/// Kinds of unit left out go from what can be built, with the factories that would make
/// only them; the rest stays.
#[test]
fn mutators_leave_out_kinds_of_unit_and_the_factories_for_them() {
    const ALL: [&str; 11] = ["BUILDER", "AIRPLANT", "SHIPYARD", "LAB", "PLANE", "SHIP", "KBOT", "GIANT", "SILO", "CANNON", "TOWER"];
    let only = |mutators: crate::Mutators| {
        let data = data_of(&[
            unit("BUILDER", "Builder=1; WorkerTime=30; CanMove=1; MaxVelocity=1; BuildDistance=200;"),
            unit("AIRPLANT", "Builder=1; WorkerTime=30; FootprintX=4; FootprintZ=4;"),
            unit("SHIPYARD", "Builder=1; WorkerTime=30; FootprintX=4; FootprintZ=4;"),
            unit("LAB", "Builder=1; WorkerTime=30; FootprintX=4; FootprintZ=4;"),
            unit("PLANE", "CanFly=1; CanMove=1; MaxVelocity=2;"),
            unit("SHIP", "CanMove=1; MaxVelocity=1; MinWaterDepth=10;"),
            unit("KBOT", "CanMove=1; MaxVelocity=1;"),
            unit("GIANT", "CanMove=1; MaxVelocity=1; BuildCostMetal=29000;"),
            unit("SILO", "Weapon1=NUKE;"),
            unit("CANNON", "Weapon1=BIGGUN;"),
            unit("TOWER", "Weapon1=GUN;"),
            weapon_file("NUKE", "targetable=1; stockpile=1; commandfire=1;"),
            ("weapons/BIGGUN.tdf".into(), "[BIGGUN] { range=5000; reloadtime=1; weaponvelocity=900; [DAMAGE] { default=1; } }".into()),
            weapon_file("GUN", ""),
        ]);
        let data = with_menu(data, "BUILDER", &["AIRPLANT", "SHIPYARD", "LAB", "SILO", "CANNON", "TOWER"]);
        let data = with_menu(with_menu(with_menu(data, "AIRPLANT", &["PLANE"]), "SHIPYARD", &["SHIP"]), "LAB", &["KBOT", "GIANT"]);
        let mut sim = sim_of(data, 1);
        sim.set_mutators(mutators);
        ALL.into_iter().filter(|n| sim.is_allowed(sim.types.id(n).unwrap())).collect::<Vec<_>>()
    };
    let without = |out: &[&str]| ALL.into_iter().filter(|n| !out.contains(n)).collect::<Vec<_>>();
    assert_eq!(only(Default::default()), ALL.to_vec());
    assert_eq!(only(crate::Mutators { no_air: true, ..Default::default() }), without(&["AIRPLANT", "PLANE"]));
    assert_eq!(only(crate::Mutators { no_naval: true, ..Default::default() }), without(&["SHIPYARD", "SHIP"]));
    assert_eq!(only(crate::Mutators { no_nukes: true, ..Default::default() }), without(&["SILO"]));
    assert_eq!(only(crate::Mutators { no_artillery: true, ..Default::default() }), without(&["CANNON"]));
    assert_eq!(only(crate::Mutators { no_experimentals: true, ..Default::default() }), without(&["GIANT"]));
    let ground_war = crate::Mutators { no_air: true, no_naval: true, no_nukes: true, no_artillery: true, no_experimentals: true, ..Default::default() };
    assert_eq!(only(ground_war), ["BUILDER", "LAB", "KBOT", "TOWER"].to_vec());
}

/// A scenario's reinforcements are the human's: they wait where they come in, hunt nobody and
/// are none of the enemy's waves. A goal to destroy every unit of a type waits for the waves
/// still bringing one.
#[test]
fn reinforcements_and_the_waves_still_to_come() {
    use crate::mission::{MissionRules, Objective, Wave};
    let data = data_of(&[unit("TANK", "CanMove=1; MaxVelocity=1;"), unit("GIANT", "CanMove=1; MaxVelocity=1; MaxDamage=500;")]);
    let mut sim = sim_of(data, 2);
    let (tank, giant) = (sim.types.id("TANK").unwrap(), sim.types.id("GIANT").unwrap());
    let at = |x, z| FxVec3::from_ints(x, 60, z);
    sim.spawn_unit(tank, 0, at(100, 100), 0, true).unwrap();
    let wave = |tick: u64, owner: u8, units: Vec<(usize, u16)>, from: FxVec3, friendly: bool| Wave { tick, owner, units, from, target: None, hunt: !friendly, friendly };
    sim.mission = Some(MissionRules {
        human: 0,
        victory: vec![Objective::KillAllOfTypeToCome(giant)],
        waves: vec![wave(2, 0, vec![(tank, 2)], at(300, 300), true), wave(3, 1, vec![(giant, 1)], at(800, 800), false), wave(400, 1, vec![(giant, 1)], at(800, 800), false)],
        ..Default::default()
    });
    for _ in 0..5 {
        sim.step(&[]);
    }
    let m = sim.mission.as_ref().unwrap();
    assert_eq!(m.waves_sent, 1, "reinforcements are none of the enemy's waves");
    assert_eq!(m.hunters.len(), 1, "and hunt nobody");
    assert_eq!(m.next_wave().map(|w| w.tick), Some(400));
    let come: Vec<FxVec3> = sim.units().filter(|u| u.owner == 0 && u.type_id == tank && u.pos.within_xz(at(300, 300), Fx::from_int(120))).map(|u| u.pos).collect();
    assert_eq!(come.len(), 2, "ours, where they came in");
    // The first giant down: the one still to come keeps the goal open.
    let kill = |sim: &mut Sim| {
        let id = sim.units().find(|u| u.type_id == giant && !u.dead).unwrap().id;
        sim.damage_unit(id, Fx::from_int(100_000), id, FxVec3::ZERO);
        for _ in 0..60 {
            sim.step(&[]);
        }
    };
    kill(&mut sim);
    assert_eq!(sim.mission.as_ref().unwrap().outcome, None);
    while sim.tick < 410 {
        sim.step(&[]);
    }
    kill(&mut sim);
    assert_eq!(sim.mission.as_ref().unwrap().outcome, Some(true));
}

/// A battle of units whose every script function runs `program` (see
/// `scripts_of_random_code_never_stop_the_battle`): its final checksum.
fn battle_of_scripts(program: &Arc<crate::cob::CobProgram>) -> u64 {
    let data = data_of(&[
        unit("FUZZ", "Weapon1=GUN; CanMove=1; MaxVelocity=2; Builder=1; WorkerTime=100; BuildDistance=120; CanReclamate=1; TransportCapacity=2; TransportSize=2; FootprintX=2; FootprintZ=2;"),
        unit("PLANE", "Weapon1=GUN; CanFly=1; CanMove=1; MaxVelocity=4; CruiseAlt=120;"),
        weapon_file("GUN", "areaofeffect=48; turret=1;"),
    ]);
    let mut sim = sim_of(data, 2);
    sim.players.iter_mut().for_each(|p| p.endless = true);
    let (fuzz, plane) = (sim.types.id("FUZZ").unwrap(), sim.types.id("PLANE").unwrap());
    for t in [fuzz, plane] {
        Arc::get_mut(&mut sim.types).unwrap().types[t].script = Some(program.clone());
    }
    let mut ids = Vec::new();
    for (i, (t, owner)) in [(fuzz, 0u8), (fuzz, 0), (plane, 0), (fuzz, 1), (fuzz, 1), (plane, 1)].into_iter().enumerate() {
        let x = if owner == 0 { 250 } else { 650 };
        ids.push(sim.spawn_unit(t, owner, FxVec3::from_ints(x, 60, 200 + i as i64 * 90), 0, true).unwrap());
    }
    sim.apply(&Command::Order { player: 0, units: ids[..3].to_vec(), order: Order::Fight(FxVec3::from_ints(700, 60, 400)), queue: false });
    sim.apply(&Command::Order { player: 1, units: ids[3..].to_vec(), order: Order::Fight(FxVec3::from_ints(200, 60, 400)), queue: false });
    sim.apply(&Command::Order { player: 0, units: vec![ids[1]], order: Order::Load(ids[0]), queue: true });
    // (The scripts do run: each unit has its own, and what they did shows on their pieces and
    // in their variables.)
    sim.step(&[]);
    assert!(ids.iter().all(|&id| sim.unit(id).is_some_and(|u| u.cob.is_some())), "every unit runs the script");
    let mut traces = 0;
    for _ in 0..360 {
        sim.step(&[]);
        sim.take_events();

        traces += sim.units().filter_map(|u| u.cob.as_ref()).filter(|c| c.statics.first() == Some(&7) || c.pieces.iter().any(|p| p.offset != [0; 3] || p.rotation != [0; 3] || !p.visible)).count();
    }
    assert!(traces > 0, "the scripts ran");
    sim.checksum()
}

#[test]
fn scripts_of_random_code_never_stop_the_battle() {
    // Every function a unit's script answers, its code random words and random instructions
    // with random operands (piece numbers in and out of range, jumps anywhere, unit values
    // known and unknown, huge numbers): the battle plays on, the same every time, whatever a
    // mod's damaged or malicious script does.
    use ta_formats::cob::{CobFunction, CobScript, op, operand_count};
    use crate::cob::CobProgram;
    const OPS: [u32; 56] = [
        op::MOVE, op::TURN, op::SPIN, op::STOP_SPIN, op::SHOW, op::HIDE, op::CACHE, op::DONT_CACHE, op::MOVE_NOW, op::TURN_NOW, op::SHADE,
        op::DONT_SHADE, op::EMIT_SFX, op::WAIT_TURN, op::WAIT_MOVE, op::SLEEP, op::PUSH_CONSTANT, op::PUSH_LOCAL, op::PUSH_STATIC,
        op::CREATE_LOCAL, op::POP_LOCAL, op::POP_STATIC, op::POP_STACK, op::ADD, op::SUB, op::MUL, op::DIV, op::BITWISE_AND, op::BITWISE_OR,
        op::BITWISE_XOR, op::BITWISE_NOT, op::RAND, op::GET_UNIT_VALUE, op::GET, op::SET_LESS, op::SET_LESS_OR_EQUAL, op::SET_GREATER,
        op::SET_GREATER_OR_EQUAL, op::SET_EQUAL, op::SET_NOT_EQUAL, op::LOGICAL_AND, op::LOGICAL_OR, op::LOGICAL_XOR, op::LOGICAL_NOT,
        op::START_SCRIPT, op::CALL_SCRIPT, op::JUMP, op::RETURN, op::JUMP_IF_FALSE, op::SIGNAL, op::SET_SIGNAL_MASK, op::EXPLODE,
        op::PLAY_SOUND, op::SET_VALUE, op::ATTACH_UNIT, op::DROP_UNIT,
    ];
    const CALLINS: [&str; 26] = [
        "Create", "StartMoving", "StopMoving", "Activate", "Deactivate", "Killed", "QueryPrimary", "AimPrimary", "AimFromPrimary",
        "FirePrimary", "Shot1", "EndBurst1", "TargetCleared", "QueryNanoPiece", "StartBuilding", "StopBuilding", "SweetSpot",
        "HitByWeapon", "SetSpeed", "SetDirection", "RockUnit", "MoveRate1", "QueryTransport", "TransportPickup", "TransportDrop", "Go",
    ];
    for seed in 1..=6u64 {
        let mut rng = crate::rng::Rng::new(seed);
        let len = 700usize;
        let mut code = Vec::with_capacity(len + 4);
        // Half small numbers (pieces, axes, statics: some past the end), then places in the
        // code and past it, distances and angles, unit values known and unknown, the extremes,
        // anything.
        let value = |rng: &mut crate::rng::Rng| match rng.range_i32(0, 9) {
            0..=4 => rng.range_i32(0, 5) as u32,
            5 => rng.range_i32(0, len as i32 + 40) as u32,
            6 => rng.range_i32(-70_000, 70_000) as u32,
            7 => rng.range_i32(0, 140) as u32,
            8 => u32::MAX - rng.range_i32(0, 3) as u32,
            _ => rng.next_u32(),
        };
        while code.len() < len {
            // (A word in twenty anything at all: a thread ends at the first it meets.)
            if rng.range_i32(0, 19) > 0 {
                // (Values on the stack first, mostly: an instruction short of them ends its
                // thread before it does anything.)
                for _ in 0..rng.range_i32(0, 3) {
                    code.push(op::PUSH_CONSTANT);
                    code.push(value(&mut rng));
                }
                let opcode = OPS[rng.range_i32(0, OPS.len() as i32 - 1) as usize];
                code.push(opcode);
                for _ in 0..operand_count(opcode).unwrap_or(0) {
                    code.push(value(&mut rng));
                }
            } else {
                code.push(rng.next_u32());
            }
        }
        code.truncate(len);
        // Each function marks that it ran (7 in the first static), then goes anywhere in the
        // code, its end itself included.
        let mut functions = Vec::new();
        for name in CALLINS {
            functions.push(CobFunction { name: name.into(), offset: code.len() as u32 });
            code.extend([op::PUSH_CONSTANT, 7, op::POP_STATIC, 0, op::JUMP, rng.range_i32(0, len as i32) as u32]);
        }
        let script = CobScript { functions, code, pieces: vec!["base".into(), "turret".into(), "barrel".into(), "flare".into()], static_var_count: 3 };
        let program = Arc::new(CobProgram::new(script, &[]));
        let first = battle_of_scripts(&program);
        if seed == 1 {
            assert_eq!(battle_of_scripts(&program), first, "a script's damage plays the same every time");
        }
    }
}

#[test]
fn queue_edits_find_their_entry_where_the_queue_has_moved_it() {
    let mut sim = sim_of(construction_data(), 1);
    let (factory, tank, other, builder, building) = (sim.types.id("FACTORY").unwrap(), sim.types.id("TANK").unwrap(), sim.types.id("OTHER").unwrap(), sim.types.id("BUILDER").unwrap(), sim.types.id("BUILDING").unwrap());
    // A factory's queue that moved on by one (the first unit finished) since the click.
    let f = sim.spawn_unit(factory, 0, FxVec3::from_ints(300, 0, 300), 0, false).unwrap();
    sim.unit_mut(f).unwrap().factory_queue = [tank, other, tank, tank].into_iter().collect();
    sim.unit_mut(f).unwrap().factory_queue.pop_front();
    sim.apply(&Command::EditQueue { player: 0, unit: f, index: 1, type_id: other, pos: None, to_front: true });
    assert_eq!(sim.unit(f).unwrap().factory_queue.iter().copied().collect::<Vec<_>>(), [other, tank, tank], "OTHER, not what came to its index");
    sim.apply(&Command::EditQueue { player: 0, unit: f, index: 2, type_id: other, pos: None, to_front: false });
    assert_eq!(sim.unit(f).unwrap().factory_queue.iter().copied().collect::<Vec<_>>(), [tank, tank], "dropped where it had gone");
    sim.apply(&Command::EditQueue { player: 0, unit: f, index: 0, type_id: other, pos: None, to_front: true });
    assert_eq!(sim.unit(f).unwrap().factory_queue.len(), 2, "none left: nothing");
    sim.apply(&Command::EditQueue { player: 1, unit: f, index: 0, type_id: tank, pos: None, to_front: false });
    assert_eq!(sim.unit(f).unwrap().factory_queue.len(), 2, "not the player's factory");
    // A constructor whose first building finished between the click and the command: the
    // finished one does not come back.
    let b = sim.spawn_unit(builder, 0, FxVec3::from_ints(600, 0, 600), 0, true).unwrap();
    let at = |x: i64| FxVec3::from_ints(x, 0, 700);
    for (i, x) in [500, 600, 700].into_iter().enumerate() {
        sim.apply(&Command::Order { player: 0, units: vec![b], order: Order::Build { type_id: building, pos: at(x) }, queue: i > 0 });
    }
    sim.unit_mut(b).unwrap().orders.pop_front();
    sim.apply(&Command::EditQueue { player: 0, unit: b, index: 2, type_id: building, pos: Some(at(700)), to_front: true });
    let orders: Vec<FxVec3> = sim.unit(b).unwrap().orders.iter().filter_map(|o| if let Order::Build { pos, .. } = o { Some(*pos) } else { None }).collect();
    assert_eq!(orders, [at(700), at(600)]);
    sim.apply(&Command::EditQueue { player: 0, unit: b, index: 0, type_id: building, pos: Some(at(600)), to_front: false });
    let orders: Vec<FxVec3> = sim.unit(b).unwrap().orders.iter().filter_map(|o| if let Order::Build { pos, .. } = o { Some(*pos) } else { None }).collect();
    assert_eq!(orders, [at(700)]);
}

#[test]
fn nearest_index_looks_both_ways_from_where_it_was() {
    let even = |j: usize| j % 2 == 0;
    assert_eq!(super::nearest_index(6, 3, even), Some(2), "the lower of two as near");
    assert_eq!(super::nearest_index(6, 4, even), Some(4));
    assert_eq!(super::nearest_index(3, 9, |j| j == 0), Some(0), "an index past the end");
    assert_eq!(super::nearest_index(0, 0, |_| true), None);
    assert_eq!(super::nearest_index(5, 1, |j| j == 4), Some(4));
}

// ---------------------------------------------------------------------------------------
// Big walkers stand level.

/// A big walker is one that walks (a kbot or Commander by its `TEDClass`, a kbot by its
/// `Category`, a spider by a movement class that climbs any slope) and stands 60 px tall or more:
/// TA's walkers that the ground tilts are 32 px at most, the mods' giants 63 to 136.
#[test]
fn big_walkers_walk_and_stand_tall() {
    use crate::defs::{BIG_WALKER_HEIGHT, ModelDef, big_walker};
    let def = |keys: &str| ta_data::UnitDef::from_tdf(&ta_formats::tdf::parse(&format!("[UNITINFO]{{UnitName=X;{keys}}}")).sections[0]).unwrap();
    let tall = |h: i64| ModelDef { height: Fx::from_int(h), ..Default::default() };
    let class = |slope: f64| ta_data::MoveClass { name: "C".into(), max_slope: slope, ..Default::default() };
    let (spider, tank) = (class(255.0), class(15.0));
    let ground = Locomotion::Ground;
    assert!(big_walker(&def("TEDClass=KBOT;"), &tall(136), None, ground), "the SpiderBot");
    assert!(big_walker(&def("TEDClass=COMMANDER;"), &tall(BIG_WALKER_HEIGHT), None, ground));
    assert!(big_walker(&def("TEDClass=CNSTR; Category=CORE KBOT LEVEL2;"), &tall(70), None, ground), "a kbot builder");
    assert!(big_walker(&def("TEDClass=TANK;"), &tall(92), Some(&spider), ground), "a spider: TA's spiders are tanks on SPID3");
    assert!(!big_walker(&def("TEDClass=KBOT;"), &tall(32), None, ground), "TA's Eraser keeps tilting");
    assert!(!big_walker(&def("TEDClass=TANK;"), &tall(92), Some(&tank), ground), "a tall tank keeps tilting");
    assert!(!big_walker(&def("TEDClass=KBOT;"), &tall(136), None, Locomotion::Hover));
}

/// On rough ground a big walker stands level, as an upright unit does (so its pieces, its guns'
/// muzzles among them, are where a level unit has them); a small kbot and a tank tilt with the
/// slope under their footprints, as in TA.
#[test]
fn big_walkers_stand_level_on_rough_ground() {
    let data = data_of(&[unit("GIANT", "CanMove=1; MaxVelocity=1; Acceleration=0.5; TurnRate=3000; TEDClass=KBOT;"), unit("KBOT", "CanMove=1; MaxVelocity=1; Acceleration=0.5; TurnRate=3000; TEDClass=KBOT;"), unit("TANK", "CanMove=1; MaxVelocity=1; Acceleration=0.5; TurnRate=3000; TEDClass=TANK;")]);
    let data = Arc::new(data);
    let mut types = UnitTypes::compile(&data, &|_| None);
    // (No models here: the giant made one, as a model 60 px tall or more makes it.)
    let giant = types.id("GIANT").unwrap();
    types.types[giant].big_walker = true;
    assert!(types.types.iter().all(|t| t.big_walker == (t.id == giant)));
    // Ground rising 2 px a cell eastwards and bumps of 6 px every other cell southwards.
    let cells = 64 * 64;
    let heights = (0..cells).map(|i| (60 + 2 * (i % 64) + 6 * ((i / 64) % 2)) as u8).collect();
    let map = TntMap { width: 64, height: 64, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights, features: vec![FEATURE_NONE; cells], feature_names: Vec::new(), minimap: Default::default() };
    let setups = [PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }];
    let mut sim = Sim::new(Arc::new(types), data, &map, &MapInfo::default(), 0, &setups, 7);
    let id = |sim: &Sim, name: &str| sim.types.id(name).unwrap();
    let ids: Vec<UnitId> = ["GIANT", "KBOT", "TANK"].iter().enumerate().map(|(k, n)| sim.spawn_unit(id(&sim, n), 0, FxVec3::from_ints(300 + 200 * k as i64, 0, 308), 0, true).unwrap()).collect();
    // (They settle on the ground as they move: walking south-east over the bumps.)
    let orders: Vec<Command> = ids.iter().map(|&u| Command::Order { player: 0, units: vec![u], order: Order::Move(sim.unit(u).unwrap().pos + FxVec3::from_ints(150, 0, 300)), queue: false }).collect();
    sim.step(&orders);
    let tilt = |sim: &Sim, u: UnitId| sim.unit(u).map(|u| (u.pitch, u.roll)).unwrap();
    let mut tilted = [false; 3];
    for _ in 0..90 {
        sim.step(&[]);
        assert_eq!(tilt(&sim, ids[0]), (0, 0), "the giant stands level");
        for (k, &u) in ids.iter().enumerate() {
            let (pitch, roll) = tilt(&sim, u);
            tilted[k] |= pitch != 0 && roll != 0;
        }
    }
    assert!(sim.unit(ids[0]).unwrap().pos.z > Fx::from_int(330), "the giant walked");
    assert_eq!(tilted, [false, true, true], "the others tilt with the ground");
}

// ---------------------------------------------------------------------------------------
// Angles handed to scripts, as TA hands them: 0..65535.

/// A body with one foot (30 to its side, 20 ahead), its script a walker's search for the spot
/// its foot was set on, as mods' walkers keep their feet on the ground (TA: Enhanced's Cavor,
/// TA: Escalation's walking cruisers, Total Mayhem's Amphibious Construction Sub): `Save`
/// remembers where the foot is, `Keep` moves it back there, halving its steps along x, z and
/// y by what `get XZ_ATAN` and `get ATAN` answer about the spot from where it is. `Angles`
/// keeps what they answer about two points and two slopes.
fn walker() -> (UnitType, World) {
    use crate::cob::CobProgram;
    use crate::defs::{ModelDef, PieceDef};
    use ta_formats::cob::{CobFunction, CobScript, op};
    let data = data_of(&[unit("WALKER", "CanMove=1; MaxVelocity=1; Upright=1;")]);
    let mut types = UnitTypes::compile(&data, &|_| None);
    let mut t = types.types.swap_remove(0);
    let piece = |name: &str, parent, offset| PieceDef { name: name.into(), parent, offset, top: Fx::ZERO };
    t.model = Arc::new(ModelDef { pieces: vec![piece("base", None, FxVec3::ZERO), piece("foot", Some(0), FxVec3::from_ints(30, 0, 20))], ..Default::default() });
    t.model_to_cob = vec![Some(0), Some(1)];
    const FOOT: u32 = 1;
    let c = |v: i32| [op::PUSH_CONSTANT, v as u32];
    // get PIECE_XZ(foot), get PIECE_Y(foot).
    let foot_xz = [op::PUSH_CONSTANT, 7, op::PUSH_CONSTANT, FOOT, op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 0, op::GET];
    let foot_y = [op::PUSH_CONSTANT, 8, op::PUSH_CONSTANT, FOOT, op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 0, op::GET];
    let none3 = [op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 0];
    let mut script = CobScript { pieces: vec!["base".into(), "foot".into()], static_var_count: 6, ..Default::default() };
    // Save() { static0 = get PIECE_XZ(foot); static1 = get PIECE_Y(foot); }
    script.functions.push(CobFunction { name: "Save".into(), offset: script.code.len() as u32 });
    script.code.extend(foot_xz);
    script.code.extend([op::POP_STATIC, 0]);
    script.code.extend(foot_y);
    script.code.extend([op::POP_STATIC, 1]);
    script.code.extend(c(0));
    script.code.push(op::RETURN);
    // Keep(): locals 0..2 the furthest move along x, z, y still short of the spot, 3 the
    // step, 4..6 the moves tried, 7 the heading of the spot.
    let start = script.code.len();
    script.functions.push(CobFunction { name: "Keep".into(), offset: start as u32 });
    let mut k: Vec<u32> = vec![op::CREATE_LOCAL; 8];
    for (local, v) in [(0, -80 << 16), (1, -80 << 16), (2, -80 << 16), (3, 80 << 16)] {
        k.extend(c(v));
        k.extend([op::POP_LOCAL, local]);
    }
    let top = start + k.len();
    // while (step != 0)
    k.extend([op::PUSH_LOCAL, 3]);
    k.extend(c(0));
    k.extend([op::SET_NOT_EQUAL, op::JUMP_IF_FALSE, u32::MAX]);
    let to_end = k.len() - 1;
    // tried = furthest + step; move foot to x-axis, z-axis, y-axis tried now
    for (furthest, tried, axis) in [(0, 4, 0), (1, 5, 2), (2, 6, 1)] {
        k.extend([op::PUSH_LOCAL, furthest, op::PUSH_LOCAL, 3, op::ADD, op::POP_LOCAL, tried]);
        k.extend([op::PUSH_LOCAL, tried, op::MOVE_NOW, FOOT, axis]);
    }
    // heading = get XZ_ATAN(static0 - get PIECE_XZ(foot))
    k.extend(c(12));
    k.extend([op::PUSH_STATIC, 0]);
    k.extend(foot_xz);
    k.push(op::SUB);
    k.extend(none3);
    k.extend([op::GET, op::POP_LOCAL, 7]);
    // if (heading > lo AND heading < hi) furthest = tried
    for (lo, hi, furthest, tried) in [(0, 32760, 0, 4), (16380, 49140, 1, 5)] {
        k.extend([op::PUSH_LOCAL, 7]);
        k.extend(c(lo));
        k.extend([op::SET_GREATER, op::PUSH_LOCAL, 7]);
        k.extend(c(hi));
        k.extend([op::SET_LESS, op::LOGICAL_AND, op::JUMP_IF_FALSE]);
        let skip = start + k.len() + 1 + 4;
        k.extend([skip as u32, op::PUSH_LOCAL, tried, op::POP_LOCAL, furthest]);
    }
    // if (get ATAN(static1 - get PIECE_Y(foot), get XZ_HYPOT(static0 - get PIECE_XZ(foot))) < 16380)
    //     furthest along y = tried
    k.extend(c(14));
    k.extend([op::PUSH_STATIC, 1]);
    k.extend(foot_y);
    k.push(op::SUB);
    k.extend(c(13));
    k.extend([op::PUSH_STATIC, 0]);
    k.extend(foot_xz);
    k.push(op::SUB);
    k.extend(none3);
    k.push(op::GET);
    k.extend(c(0));
    k.extend(c(0));
    k.push(op::GET);
    k.extend(c(16380));
    k.extend([op::SET_LESS, op::JUMP_IF_FALSE]);
    let skip = start + k.len() + 1 + 4;
    k.extend([skip as u32, op::PUSH_LOCAL, 6, op::POP_LOCAL, 2]);
    // step = step / 2
    k.extend([op::PUSH_LOCAL, 3]);
    k.extend(c(2));
    k.extend([op::DIV, op::POP_LOCAL, 3, op::JUMP, top as u32]);
    k[to_end] = (start + k.len()) as u32;
    k.extend(c(0));
    k.push(op::RETURN);
    script.code.extend(k);
    // Angles() { static2 = get XZ_ATAN(PACKXZ(10, 10)); static3 = get XZ_ATAN(PACKXZ(-10, 10));
    //   static4 = get ATAN([5], [10]); static5 = get ATAN(0 - [5], [10]); }
    script.functions.push(CobFunction { name: "Angles".into(), offset: script.code.len() as u32 });
    for (id, a, b, out) in [(12, (10 << 16) | 10, 0, 2), (12, ((-10i32) << 16) | 10, 0, 3), (14, 5 << 16, 10 << 16, 4), (14, -5 << 16, 10 << 16, 5)] {
        script.code.extend(c(id));
        script.code.extend(c(a));
        script.code.extend(c(b));
        script.code.extend(c(0));
        script.code.extend(c(0));
        script.code.extend([op::GET, op::POP_STATIC, out]);
    }
    script.code.extend(c(0));
    script.code.push(op::RETURN);
    t.script = Some(Arc::new(CobProgram::new(script, &["base".into(), "foot".into()])));
    (t, World::new(&flat_map()))
}

/// Runs `name` in the walker's script, as the unit stands.
fn call(u: &mut Unit, t: &UnitType, world: &World, name: &str) {
    let mut cob = u.cob.take().expect("a script");
    let (mut requests, mut events) = (Vec::new(), Vec::new());
    let mut host = UnitHost { unit: &mut *u, requests: &mut requests, unit_type: t, world, events: &mut events, now_ms: 0, lookup: &|_| None, max_id: 1, allies: 1 };
    cob.call(&mut host, name, &[]).expect("the function runs");
    u.cob = Some(cob);
}

/// TA gives scripts its angles as 16-bit numbers, 0..65535: a heading or a slope just short of
/// straight ahead is just short of a whole turn, not negative (scripts compare them with
/// 16384, 32768 and 49152 for their quarters).
#[test]
fn scripts_get_headings_and_slopes_from_0_to_65535() {
    let (t, world) = walker();
    let mut u = Unit::new(UnitId { slot: 0, generation: 0 }, 0, 0, FxVec3::from_ints(400, 60, 400));
    u.cob = Some(CobInstance::new(t.script.clone().unwrap()));
    // XZ_ATAN(dx, dz) is the heading (from the unit's) along -(dx, dz): -(10, 10) is behind a
    // unit facing +z and to its right (16384 faces +x, its left), -(-10, 10) behind it to its
    // left; the same turned with units facing +x and -x.
    for (heading, back_right, back_left) in [(0, 40960, 24576), (16384, 24576, 8192), (-16384, 57344, 40960)] {
        u.heading = heading;
        call(&mut u, &t, &world, "Angles");
        let s = &u.cob.as_ref().unwrap().statics;
        assert_eq!([s[2], s[3]], [back_right, back_left], "XZ_ATAN facing {heading}");
        // ATAN(a, b): the slope of (b, a), 26.6 degrees up or down.
        assert!((4830..=4842).contains(&s[4]) && (65536 - 4842..=65536 - 4830).contains(&s[5]), "ATAN: {} {}", s[4], s[5]);
    }
}

/// Mods' walkers find the ground their feet were set on by asking which way the spot is from
/// the foot (`XZ_ATAN(spot - foot) > 16380 AND < 49140`: ahead) and whether it is above it
/// (`ATAN(spot_y - foot_y, distance) < 16380`). With angles from -32768 the answers were wrong
/// for every spot below the foot and every one ahead of it on one side: the feet climbed into
/// the air and the walkers stood on their backs, legs up (TA: Enhanced's Cavor).
#[test]
fn walkers_find_the_spot_their_foot_was_set_on() {
    let (t, world) = walker();
    for (dx, dy, dz, heading) in [
        (25, 0, 0, 0),
        (-25, 0, 0, 0),
        (0, 0, 25, 0),
        (0, 0, -25, 0),
        (20, 0, 20, 0),
        (-20, 0, -20, 0),
        (20, 0, -20, 0),
        (-20, 0, 20, 0),
        // (Up or down as it walks on: straight up or down, the search's first try finds the
        // spot right above or below and takes it for neither, as in TA.)
        (10, 30, -15, 0),
        (-12, -30, 8, 0),
        (15, 20, -20, 9000),
        (-18, -25, 12, -20000),
        (-10, 10, 15, 30000),
    ] {
        let mut u = Unit::new(UnitId { slot: 0, generation: 0 }, 0, 0, FxVec3::from_ints(400, 60, 400));
        u.cob = Some(CobInstance::new(t.script.clone().unwrap()));
        call(&mut u, &t, &world, "Save");
        let spot = piece_world_position(&u, &t, u.cob.as_ref(), 1);
        // The body moves on (and up or down, and round); the foot finds its spot again.
        u.pos += FxVec3::from_ints(dx, dy, dz);
        u.heading = heading;
        call(&mut u, &t, &world, "Keep");
        let foot = piece_world_position(&u, &t, u.cob.as_ref(), 1);
        let off = [foot.x - spot.x, foot.y - spot.y, foot.z - spot.z].map(|d| d.to_f64().abs());
        assert!(off.iter().all(|&d| d <= 2.0), "the body moved ({dx}, {dy}, {dz}), facing {heading}: the foot is {off:?} off its spot");
    }
}

/// The heading and pitch an aim script is called with are TA's 16-bit angles too: a target
/// behind a gun to its right is at 40960 (not -24576), one below it just short of 65536. Mods
/// pick a gun by the heading's quarter (Total Mayhem's shipyards: `heading > 24575 AND <= 40959`
/// behind), keep a turret to its arc (TA: Escalation's `heading > 16384 AND < 49152`), or stop a
/// barrel 50 degrees down (TA: Enhanced's SpiderBot: `pitch > 65536 - <50>`).
#[test]
fn aims_are_called_with_headings_and_pitches_from_0_to_65535() {
    use crate::cob::CobProgram;
    use ta_formats::cob::{CobFunction, CobScript, op};
    let data = data_of(&[unit("SHOOTER", "Weapon1=GUN;"), unit("TARGET", ""), weapon_file("GUN", "")]);
    // The gun on a rise 80 above the ground round it.
    let mut map = flat_map();
    for z in 20..30 {
        for x in 20..30 {
            map.heights[z * 64 + x] = 140;
        }
    }
    let mut sim = sim_on_map(data, 2, &map);
    // AimPrimary(heading, pitch) { static0 = heading; static1 = pitch; return 1; }
    let mut script = CobScript { static_var_count: 2, ..Default::default() };
    script.functions.push(CobFunction { name: "AimPrimary".into(), offset: 0 });
    script.code.extend([op::CREATE_LOCAL, op::CREATE_LOCAL, op::PUSH_LOCAL, 0, op::POP_STATIC, 0, op::PUSH_LOCAL, 1, op::POP_STATIC, 1, op::PUSH_CONSTANT, 1, op::RETURN]);
    let shooter = sim.types.id("SHOOTER").unwrap();
    Arc::get_mut(&mut sim.types).unwrap().types[shooter].script = Some(Arc::new(CobProgram::new(script, &[])));
    // Facing +z; the target behind it to its right (-x, -z), on the low ground.
    let s = sim.spawn_unit(shooter, 0, FxVec3::from_ints(400, 0, 400), 0, true).unwrap();
    let t = sim.spawn_unit(sim.types.id("TARGET").unwrap(), 1, FxVec3::from_ints(250, 0, 250), 0, true).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![s], order: Order::Attack(Target::Unit(t)), queue: false }]);
    for _ in 0..10 {
        sim.step(&[]);
    }
    let statics = &sim.unit(s).unwrap().cob.as_ref().unwrap().statics;
    let (heading, pitch) = (statics[0], statics[1]);
    assert!((40960 - 64..=40960 + 64).contains(&heading), "behind to the right: {heading}");
    assert!((65536 - 16384..65536).contains(&pitch), "below: {pitch}");
}

/// TA set its `NoAutoFire=1` units (the Big Bertha, the Vulcan, the Buzzsaw, the Doomsday
/// Machine) to hold fire on construction, whatever their `StandingFireOrder` (fire at will in the
/// Doomsday Machine's file) and whatever they were set to while they were built; other units
/// start as `StandingFireOrder` says, and so do the anti-nukes, `NoAutoFire=1` as they are: they
/// fire at nukes by default.
#[test]
fn no_auto_fire_units_are_set_to_hold_fire_when_finished() {
    let data = data_of(&[
        unit("DOOM", "Weapon1=SHELL; NoAutoFire=1; StandingFireOrder=2;"),
        unit("TOWER", "Weapon1=SHELL; StandingFireOrder=2;"),
        unit("AMD", "Weapon1=ABM; NoAutoFire=1; StandingFireOrder=2;"),
        weapon_file("SHELL", "range=900;"),
        weapon_file("ABM", "interceptor=1; coverage=1000; stockpile=1;"),
    ]);
    let mut sim = sim_of(data, 2);
    let (doom, tower) = (sim.types.id("DOOM").unwrap(), sim.types.id("TOWER").unwrap());
    let built = sim.spawn_unit(doom, 0, FxVec3::from_ints(100, 60, 100), 0, true).unwrap();
    assert_eq!(sim.unit(built).unwrap().fire_state, 0, "a finished one holds its fire");
    let other = sim.spawn_unit(tower, 0, FxVec3::from_ints(300, 60, 100), 0, true).unwrap();
    assert_eq!(sim.unit(other).unwrap().fire_state, 2, "without the tag: as StandingFireOrder says");
    let building = sim.spawn_unit(doom, 0, FxVec3::from_ints(500, 60, 100), 0, false).unwrap();
    sim.unit_mut(building).unwrap().fire_state = 2;
    sim.on_completed(building);
    assert_eq!(sim.unit(building).unwrap().fire_state, 0, "set to hold fire as it is finished");
    let amd = sim.types.id("AMD").unwrap();
    let anti_nuke = sim.spawn_unit(amd, 0, FxVec3::from_ints(700, 60, 100), 0, false).unwrap();
    assert_eq!(sim.unit(anti_nuke).unwrap().fire_state, 2, "an anti-nuke the exception");
    sim.on_completed(anti_nuke);
    assert_eq!(sim.unit(anti_nuke).unwrap().fire_state, 2, "finished too");
}

/// TA 3.1's questions about units by id, which the mods made for the patched engines ask (TA:
/// Escalation in most of its scripts; TA's own never): the range of ids, the unit's own, and
/// another unit's player, how much of it is left to build, whether it is an ally and whether
/// this computer has it, answered from the battle (every unit is this computer's: each plays the
/// whole battle). A unit asked about that is not there is 0 to every question.
#[test]
fn scripts_ask_about_units_by_id_as_ta_31_answered() {
    use crate::cob::CobProgram;
    use crate::host::value::*;
    use ta_formats::cob::{CobFunction, CobScript, op};
    let data = data_of(&[unit("PROBE", "")]);
    let mut sim = sim_of(data, 3);
    // Players 0 and 1 on one side, 2 their enemy.
    sim.players[1].team = sim.players[0].team;
    let questions = [
        (MY_ID, 0), (MIN_ID, 0), (MAX_ID, 0),
        (UNIT_TEAM, 2), (UNIT_TEAM, 3), (UNIT_ALLIED, 2), (UNIT_ALLIED, 3), (UNIT_ALLIED, 1),
        (UNIT_BUILD_PERCENT_LEFT, 3), (UNIT_BUILD_PERCENT_LEFT, 1), (UNIT_IS_ON_THIS_COMP, 3), (UNIT_IS_ON_THIS_COMP, 99),
        (UNIT_TEAM, 99),
    ];
    let mut script = CobScript { static_var_count: questions.len() as u32, ..Default::default() };
    script.functions.push(CobFunction { name: "Ask".into(), offset: 0 });
    for (i, (value, unit)) in questions.iter().enumerate() {
        // static i = get value(unit, 0, 0, 0)
        script.code.extend([op::PUSH_CONSTANT, *value as u32, op::PUSH_CONSTANT, *unit as u32]);
        script.code.extend([op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 0, op::GET, op::POP_STATIC, i as u32]);
    }
    script.code.extend([op::PUSH_CONSTANT, 0, op::RETURN]);
    Arc::get_mut(&mut sim.types).unwrap().types[0].script = Some(Arc::new(CobProgram::new(script, &[])));
    let ours = sim.spawn_unit(0, 0, FxVec3::from_ints(100, 60, 100), 0, true).unwrap();
    let ally = sim.spawn_unit(0, 1, FxVec3::from_ints(300, 60, 100), 0, true).unwrap();
    let enemy = sim.spawn_unit(0, 2, FxVec3::from_ints(500, 60, 500), 0, false).unwrap();
    assert_eq!([ours.script_id(), ally.script_id(), enemy.script_id()], [1, 2, 3], "ids are places from 1");
    sim.with_script(ours, |cob, host| {
        cob.call(host, "Ask", &[]).expect("the function runs");
    });
    let s = &sim.unit(ours).unwrap().cob.as_ref().unwrap().statics;
    assert_eq!(&s[..2], [1, 1], "its own id; the lowest an id may be");
    assert!(s[2] >= 3, "the highest an id may be covers every unit: {}", s[2]);
    assert_eq!(&s[3..5], [1, 2], "the ally's player, the enemy's");
    assert_eq!(&s[5..8], [1, 0, 1], "the ally is, the enemy is not, the unit is its own");
    assert_eq!(&s[8..10], [100, 0], "the enemy's frame is all to build, the unit is finished");
    assert_eq!(&s[10..12], [1, 0], "a unit there is this computer's; none at all is not");
    assert_eq!(s[12], 0);
}

/// A script as a mod's upgradeable building has it. Readied to build: Total Mayhem's geothermal
/// plant readies itself when switched on, and switches itself off a moment later (`Activate`);
/// TA: Escalation's Sentinel, which has no Activate, readies itself as it is made (`Create`). It is
/// not ready while it builds (`StartBuilding`), to build with no nano spray; when done it takes on
/// the unit in the next place, the module it made, and armours itself (`StopBuilding`).
fn upgradeable_building_script(readies_when_made: bool) -> crate::cob::CobProgram {
    use crate::host::value::*;
    use ta_formats::cob::{CobFunction, CobScript, op};
    let mut script = CobScript { pieces: vec!["pad".into()], ..Default::default() };
    let set = |code: &mut Vec<u32>, value: i32, to: u32| code.extend([op::PUSH_CONSTANT, value as u32, op::PUSH_CONSTANT, to, op::SET_VALUE]);
    if readies_when_made {
        script.functions.push(CobFunction { name: "Create".into(), offset: script.code.len() as u32 });
        set(&mut script.code, IN_BUILD_STANCE, 1);
        script.code.extend([op::PUSH_CONSTANT, 0, op::RETURN]);
    } else {
        script.functions.push(CobFunction { name: "Activate".into(), offset: script.code.len() as u32 });
        set(&mut script.code, IN_BUILD_STANCE, 1);
        script.code.extend([op::PUSH_CONSTANT, 70, op::SLEEP]);
        set(&mut script.code, ACTIVATION, 0);
        script.code.extend([op::PUSH_CONSTANT, 0, op::RETURN]);
    }
    script.functions.push(CobFunction { name: "StartBuilding".into(), offset: script.code.len() as u32 });
    set(&mut script.code, IN_BUILD_STANCE, 0);
    script.code.extend([op::PUSH_CONSTANT, 0, op::RETURN]);
    // attach-unit (get MY_ID + 1) to pad; set ARMORED to 1
    script.functions.push(CobFunction { name: "StopBuilding".into(), offset: script.code.len() as u32 });
    script.code.extend([op::PUSH_CONSTANT, MY_ID as u32, op::GET_UNIT_VALUE, op::PUSH_CONSTANT, 1, op::ADD, op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 0, op::ATTACH_UNIT]);
    set(&mut script.code, ARMORED, 1);
    script.code.extend([op::PUSH_CONSTANT, 0, op::RETURN]);
    crate::cob::CobProgram::new(script, &["pad".into()])
}

/// The mods' upgradeable buildings (Total Mayhem's geothermal plants and moho mines, TA:
/// Escalation's Aegis) have build power but no Builder flag, and a menu page offering their
/// upgrade module (no footprint, energy of its own): they make it, at their full rate though their
/// scripts switch them off and leave the build stance as they build, then take it on. The module
/// takes no ground, and on its building it makes its energy still (TA: Escalation's fusion
/// reactor upgrade: +250).
#[test]
fn a_building_makes_its_own_upgrade_module_and_takes_it_on() {
    let gui = concat!(
        "[GADGET0]{[COMMON]{id=0;name=HEADER;xpos=0;ypos=128;width=128;height=640;}}\n",
        "[GADGET1]{[COMMON]{id=1;name=MODULE;xpos=0;ypos=0;width=64;height=64;attribs=32;commonattribs=4;}}",
    );
    let data = data_of(&[
        unit("PLANT", "Builder=0; WorkerTime=300; FootprintX=4; FootprintZ=4; BuildTime=1000; BuildCostMetal=100;"),
        unit("MODULE", "FootprintX=0; FootprintZ=0; BuildTime=300; BuildCostMetal=10; BuildCostEnergy=10; EnergyMake=50;"),
        ("guis/plant1.gui".to_string(), gui.to_string()),
    ]);
    let mut sim = sim_of(data, 1);
    let (plant_type, module_type) = (sim.types.id("PLANT").unwrap(), sim.types.id("MODULE").unwrap());
    let t = sim.types.get(plant_type);
    assert!(t.is_builder && t.is_factory && t.build_options == [module_type], "it builds what its page offers");
    assert!(!sim.types.makes_units(t), "no factory to the player: it makes no unit that moves");
    Arc::get_mut(&mut sim.types).unwrap().types[plant_type].script = Some(Arc::new(upgradeable_building_script(false)));
    let plant = sim.spawn_unit(plant_type, 0, FxVec3::from_ints(400, 60, 400), 0, true).unwrap();
    let occupant = |sim: &Sim| sim.world.occupant(400 / 16, 400 / 16);
    let before = occupant(&sim);
    sim.step(&[Command::Produce { player: 0, factory: plant, type_id: module_type, count: 1 }]);
    let (mut module, mut ticks) = (None, 0);
    for tick in 0..600 {
        sim.step(&[]);
        module = sim.units().find(|u| u.type_id == module_type).map(|u| u.id);
        if module.is_some_and(|m| sim.unit(m).unwrap().carried_by.is_some()) {
            ticks = tick;
            break;
        }
    }
    let module = module.expect("the module is made");
    let m = sim.unit(module).unwrap();
    assert!(m.is_complete(), "finished, though the plant switched itself off and left its stance");
    // (A second of work at its rate, `BuildTime` 300 by `WorkerTime` 300, and a few ticks to start.)
    assert!(ticks <= 40, "built at its full rate: {ticks} ticks");
    assert!(!sim.unit(plant).unwrap().activated, "left switched off, as its script switched it");
    assert_eq!(m.carried_by, Some(plant), "the plant took it on");
    assert_eq!(occupant(&sim), before, "it takes no ground");
    for _ in 0..5 {
        sim.step(&[]);
    }
    assert!(sim.players[0].economy.energy_income >= Fx::from_int(49), "its energy counts on its building: {:?}", sim.players[0].economy.energy_income);
}

/// A factory makes a unit that cannot move where it makes its units (TA: Escalation's labs, their
/// Level 2 upgrade): it stays there, and the next unit is made and goes out.
#[test]
fn a_factory_makes_an_upgrade_that_cannot_move_and_goes_on() {
    use crate::host::value::*;
    use ta_formats::cob::{CobFunction, CobScript, op};
    let data = data_of(&[
        unit("FACTORY", "Builder=1; WorkerTime=300; FootprintX=4; FootprintZ=4; BuildTime=1000; BuildCostMetal=100;"),
        unit("UPGRADE", "FootprintX=0; FootprintZ=0; BuildTime=300; BuildCostMetal=10;"),
        unit("TANK", "CanMove=1; MaxVelocity=1; BuildTime=300; BuildCostMetal=10;"),
    ]);
    let mut sim = sim_of(with_menu(data, "FACTORY", &["UPGRADE", "TANK"]), 1);
    let (factory_type, upgrade_type, tank_type) = (sim.types.id("FACTORY").unwrap(), sim.types.id("UPGRADE").unwrap(), sim.types.id("TANK").unwrap());
    assert!(sim.types.makes_units(sim.types.get(factory_type)), "a factory, its upgrade beside its tanks");
    let mut script = CobScript::default();
    script.functions.push(CobFunction { name: "Activate".into(), offset: 0 });
    script.code.extend([op::PUSH_CONSTANT, IN_BUILD_STANCE as u32, op::PUSH_CONSTANT, 1, op::SET_VALUE, op::PUSH_CONSTANT, 0, op::RETURN]);
    Arc::get_mut(&mut sim.types).unwrap().types[factory_type].script = Some(Arc::new(crate::cob::CobProgram::new(script, &[])));
    let factory = sim.spawn_unit(factory_type, 0, FxVec3::from_ints(400, 60, 400), 0, true).unwrap();
    sim.step(&[
        Command::Produce { player: 0, factory, type_id: upgrade_type, count: 1 },
        Command::Produce { player: 0, factory, type_id: tank_type, count: 1 },
    ]);
    let mut tank_done = false;
    for _ in 0..900 {
        sim.step(&[]);
        tank_done = sim.units().any(|u| u.type_id == tank_type && u.is_complete());
        if tank_done {
            break;
        }
    }
    let upgrade = sim.units().find(|u| u.type_id == upgrade_type).expect("the upgrade is made");
    assert!(upgrade.is_complete() && upgrade.orders.is_empty(), "made, and sent nowhere");
    assert!(upgrade.pos.within_xz(FxVec3::from_ints(400, 60, 400), Fx::from_int(40)), "where it was made");
    assert!(tank_done, "the next unit is made, the upgrade holding nothing up");
    for _ in 0..3 {
        sim.step(&[]);
    }
    assert!(sim.unit(factory).unwrap().factory_queue.is_empty(), "and the factory is done");
}

/// A building whose script readies it as it is made, and leaves the stance as it starts (TA:
/// Escalation's Sentinel, no Activate), makes the module its menu offers: one that cannot get
/// going (a speed, an acceleration of none: to TA a mobile unit), with a cell of footprint
/// (Total Mayhem's underwater moho mine's). It is made just on the pad, takes no ground, stays
/// there for the building's script to take on, and goes with the building, though it is armoured
/// untouchable (TA: Escalation's heavy fusion reactor's: `DamageModifier=0`).
#[test]
fn a_tower_makes_a_module_that_cannot_get_going_and_it_goes_with_the_tower() {
    let data = data_of(&[
        unit("TOWER", "Builder=1; WorkerTime=300; FootprintX=4; FootprintZ=4; BuildTime=1000; BuildCostMetal=100;"),
        unit("MODULE", "BMcode=1; MaxVelocity=1; Acceleration=0; FootprintX=1; FootprintZ=1; BuildTime=300; BuildCostMetal=10; DamageModifier=0;"),
    ]);
    let mut sim = sim_of(with_menu(data, "TOWER", &["MODULE"]), 1);
    let (tower_type, module_type) = (sim.types.id("TOWER").unwrap(), sim.types.id("MODULE").unwrap());
    let mt = sim.types.get(module_type);
    assert!(mt.locomotion == Locomotion::Static && mt.def.takes_no_ground, "it stands, taking no ground");
    Arc::get_mut(&mut sim.types).unwrap().types[tower_type].script = Some(Arc::new(upgradeable_building_script(true)));
    let tower = sim.spawn_unit(tower_type, 0, FxVec3::from_ints(400, 60, 400), 0, true).unwrap();
    let pad = sim.unit(tower).unwrap().pos;
    let ground = |sim: &Sim| (20..30).flat_map(|z| (20..30).map(move |x| (x, z))).map(|(x, z)| sim.world.occupant(x, z)).collect::<Vec<_>>();
    let cells = ground(&sim);
    sim.step(&[Command::Produce { player: 0, factory: tower, type_id: module_type, count: 1 }]);
    let mut module = None;
    for _ in 0..600 {
        sim.step(&[]);
        if module.is_none() && let Some(m) = sim.units().find(|u| u.type_id == module_type) {
            assert!(m.pos.within_xz(pad, Fx::ONE), "made just on the pad, not on a cell of its own: {:?} by {pad:?}", m.pos);
            module = Some(m.id);
        }
        if module.is_some_and(|m| sim.unit(m).unwrap().carried_by.is_some()) {
            break;
        }
    }
    let module = module.expect("the module is made");
    let m = sim.unit(module).unwrap();
    assert!(m.is_complete() && m.carried_by == Some(tower), "made out of the tower's stance, and taken on");
    assert!(sim.unit(tower).unwrap().armored, "the tower upgraded");
    assert_eq!(ground(&sim), cells, "it takes no ground");
    sim.unit_mut(module).unwrap().armored = true;
    let health = sim.types.get(tower_type).max_health;
    sim.damage_unit(tower, health * 2, tower, FxVec3::ZERO);
    for _ in 0..5 {
        sim.step(&[]);
    }
    assert!(sim.unit(module).is_none_or(|m| m.dead), "it goes with the tower, untouchable as it is");
}

/// A mod's building that makes its own upgrade, switched on when built (`ActivateWhenBuilt=1`), is
/// switched on as it is finished, and stays on with nothing to make: on is its work (Total
/// Mayhem's moho mines extract, TA: Escalation's repair pads repair). A factory as TA's are
/// (`TEDClass=PLANT`) opens only for work.
#[test]
fn a_building_that_builds_is_switched_on_as_built_and_a_plant_opens_for_work() {
    let data = data_of(&[
        unit("MINE", "Builder=0; WorkerTime=200; FootprintX=2; FootprintZ=2; ActivateWhenBuilt=1; OnOffable=1; TEDClass=METAL;"),
        unit("PLANT", "Builder=1; WorkerTime=200; FootprintX=4; FootprintZ=4; ActivateWhenBuilt=1; TEDClass=PLANT;"),
        unit("MODULE", "FootprintX=0; FootprintZ=0; BuildTime=300;"),
    ]);
    let data = with_menu(with_menu(data, "MINE", &["MODULE"]), "PLANT", &["MODULE"]);
    let mut sim = sim_of(data, 1);
    let (mine, plant) = (sim.types.id("MINE").unwrap(), sim.types.id("PLANT").unwrap());
    assert!(sim.types.get(mine).is_factory && !sim.types.get(mine).is_plant() && sim.types.get(plant).is_plant());
    let mine = sim.spawn_unit(mine, 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
    let plant = sim.spawn_unit(plant, 0, FxVec3::from_ints(600, 60, 600), 0, true).unwrap();
    for _ in 0..30 {
        sim.step(&[]);
    }
    assert!(sim.unit(mine).unwrap().activated, "the mine is on, as its file says, with nothing to make");
    assert!(!sim.unit(plant).unwrap().activated, "the factory waits for work");
}

/// A unit without build power offers nothing to build: Total Mayhem lists a missile pseudo-unit
/// for its silos (`MAKENUKEARM`, for its computer players); TA's silos make their missiles on
/// their own.
#[test]
fn a_unit_without_build_power_offers_nothing_to_build() {
    let data = with_menu(data_of(&[unit("SILO", "Builder=1;"), unit("MISSILE", "")]), "SILO", &["MISSILE"]);
    let sim = sim_of(data, 1);
    let silo = sim.types.get(sim.types.id("SILO").unwrap());
    assert!(!silo.is_builder && silo.build_options.is_empty());
}

/// The mod check tells a mod's scripts which unit values Reborn answers (`script.value`): the
/// host's, every one.
#[test]
fn the_mod_check_knows_the_values_the_engine_answers() {
    assert_eq!(crate::host::value::ANSWERED, ta_data::validation::ANSWERED_VALUES);
}

/// A script cloaks its own unit (`UNIT_CLOAKED`, a later patch's: Total Mayhem's mines while they
/// wait), at no cost when its file gives none, and uncloaks it.
#[test]
fn a_script_cloaks_its_own_unit() {
    use crate::host::value::*;
    use ta_formats::cob::{CobFunction, CobScript, op};
    let data = data_of(&[unit("MINE", "")]);
    let mut sim = sim_of(data, 1);
    let mut script = CobScript { static_var_count: 1, ..Default::default() };
    for (name, on) in [("Hide", 1), ("Show", 0)] {
        script.functions.push(CobFunction { name: name.into(), offset: script.code.len() as u32 });
        script.code.extend([op::PUSH_CONSTANT, UNIT_CLOAKED as u32, op::PUSH_CONSTANT, on, op::SET_VALUE, op::PUSH_CONSTANT, 0, op::RETURN]);
    }
    script.functions.push(CobFunction { name: "Ask".into(), offset: script.code.len() as u32 });
    script.code.extend([op::PUSH_CONSTANT, UNIT_CLOAKED as u32, op::GET_UNIT_VALUE, op::POP_STATIC, 0, op::PUSH_CONSTANT, 0, op::RETURN]);
    Arc::get_mut(&mut sim.types).unwrap().types[0].script = Some(Arc::new(crate::cob::CobProgram::new(script, &[])));
    let mine = sim.spawn_unit(0, 0, FxVec3::from_ints(400, 60, 400), 0, true).unwrap();
    let call = |sim: &mut Sim, name: &str| {
        sim.with_script(mine, |cob, host| {
            cob.call(host, name, &[]).expect("runs");
        })
    };
    call(&mut sim, "Hide");
    for _ in 0..3 {
        sim.step(&[]);
    }
    assert!(sim.unit(mine).unwrap().cloaked, "cloaked by its script");
    call(&mut sim, "Ask");
    assert_eq!(sim.unit(mine).unwrap().cob.as_ref().unwrap().statics[0], 1);
    call(&mut sim, "Show");
    for _ in 0..3 {
        sim.step(&[]);
    }
    assert!(!sim.unit(mine).unwrap().cloaked, "and uncloaked");
}

/// A factory's menu is what its player is offered; the computer players' list (the 3.1 patch's
/// `[CANBUILD]`) adds units of the mod's AI, which an order may still name, as the computer players'
/// do: TA: The Final Frontier's command cruiser builds its AI's `AAILAB` for them, never on its menu.
#[test]
fn units_of_the_computer_players_lists_are_built_but_not_offered() {
    let side = "[SIDE0]{name=ARM;commander=CRUISER;} [CANBUILD]{[YARD]{canbuild1=SHIP;canbuild2=AISHIP;}}";
    let data = data_of(&[
        unit("YARD", "Builder=1; WorkerTime=300; BuildDistance=200;"),
        unit("SHIP", "CanMove=1; MaxVelocity=1; BuildTime=30; BuildCostMetal=10; FootprintX=1; FootprintZ=1;"),
        unit("AISHIP", "CanMove=1; MaxVelocity=1; BuildTime=30; BuildCostMetal=10; FootprintX=1; FootprintZ=1;"),
        ("guis/yard1.gui".to_string(), "[A]{[COMMON]{id=1;name=SHIP;attribs=32;commonattribs=4;}}".to_string()),
        ("gamedata/sidedata.tdf".to_string(), side.to_string()),
    ]);
    let mut sim = sim_of(data, 1);
    sim.set_starting_resources(0, Fx::from_int(10000), Fx::from_int(100000));
    let (yard, ship, ai_ship) = (sim.types.id("YARD").unwrap(), sim.types.id("SHIP").unwrap(), sim.types.id("AISHIP").unwrap());
    let t = sim.types.get(yard);
    assert_eq!((t.build_options.clone(), t.ai_build_options.clone()), (vec![ship], vec![ship, ai_ship]), "its menu, and all it builds");
    assert!(t.is_factory, "a factory by all it builds");
    // (Ready to build as it opens up, as TA's factories' scripts make it.)
    use crate::cob::CobProgram;
    use ta_formats::cob::{CobFunction, CobScript, op};
    let mut script = CobScript::default();
    script.functions.push(CobFunction { name: "Activate".into(), offset: 0 });
    script.code.extend([op::PUSH_CONSTANT, crate::host::value::IN_BUILD_STANCE as u32, op::PUSH_CONSTANT, 1, op::SET_VALUE, op::PUSH_CONSTANT, 0, op::RETURN]);
    Arc::get_mut(&mut sim.types).unwrap().types[yard].script = Some(Arc::new(CobProgram::new(script, &[])));
    let factory = sim.spawn_unit(yard, 0, FxVec3::from_ints(400, 60, 400), 0, true).unwrap();
    sim.step(&[Command::Produce { player: 0, factory, type_id: ai_ship, count: 1 }]);
    assert!((0..300).any(|_| {
        sim.step(&[]);
        sim.units().any(|u| u.type_id == ai_ship && u.is_complete())
    }), "made when an order names it, as a computer player's");
}
