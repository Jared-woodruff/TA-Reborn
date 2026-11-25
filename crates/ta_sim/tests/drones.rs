//! Drones (`[DRONES]`: TA Reborn's own), played out with made-up units on a flat map (no TA install
//! needed): made with their unit and guarding it, made again at their price after a loss, held to
//! their leash, dying and changing sides with their unit, and given (and taken) by an enhancement.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::{Cheat, Command, Fx, FxVec3, Locomotion, Order, PlayerSetup, Sim, UnitId, UnitTypes};

fn data_of(files: &[(String, String)]) -> GameData {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta-sim-drones-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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

fn field() -> Sim {
    let data = Arc::new(data_of(&[
        // A station keeping two drones, each made again in three seconds for 30 metal and 300
        // energy, kept within 300 px of it.
        unit("STATION", "Builder=1; WorkerTime=100; BuildDistance=300; FootprintX=2; FootprintZ=2; UpgradesTo=STATION2; [DRONES] { [Pod] { Unit=DRONE; Count=2; RebuildTime=3; RebuildCostMetal=30; RebuildCostEnergy=300; Leash=300; } }"),
        // What it upgrades into: three drones.
        unit("STATION2", "Builder=1; WorkerTime=100; BuildDistance=300; FootprintX=2; FootprintZ=2; BuildTime=100; [DRONES] { Unit=DRONE; Count=3; Leash=300; }"),
        unit("DRONE", "CanFly=1; CanMove=1; MaxVelocity=5; Acceleration=1; BrakeRate=1; TurnRate=2048; CruiseAlt=60; Builder=1; WorkerTime=50; BuildDistance=100; FootprintX=1; FootprintZ=1;"),
        // A commander whose enhancement gives it a drone (made again in 20 seconds), and another
        // that gives it nothing of the kind.
        unit("ACU", "CanMove=1; BMcode=1; MaxVelocity=2; Acceleration=0.5; BrakeRate=1; TurnRate=2000; FootprintX=2; FootprintZ=2; Builder=1; WorkerTime=100; [ENHANCEMENTS] { [Pod] { Slot=Back; BuildTime=1; [DRONES] { Unit=DRONE; RebuildTime=20; } } [Armour] { Slot=Front; BuildTime=1; MaxDamage=2000; } }"),
        // A tank a builder of 100 build power would build in 30 s (and mend at that pace).
        unit("TANK", "CanMove=1; BMcode=1; MaxVelocity=2; Acceleration=0.5; BrakeRate=1; TurnRate=2000; FootprintX=2; FootprintZ=2; BuildTime=3000;"),
    ]));
    let types = Arc::new(UnitTypes::compile(&data, &|_| None));
    let cells = 128;
    let map = TntMap { width: cells as u32, height: cells as u32, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; cells * cells], features: vec![FEATURE_NONE; cells * cells], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    let setups: Vec<PlayerSetup> = (0..3).map(|i| PlayerSetup { side: "ARM".into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
    let mut sim = Sim::new(types, data, &map, &MapInfo::default(), 0, &setups, 7);
    for p in &mut sim.players {
        p.endless = true;
    }
    sim.cheats_allowed = true;
    sim
}

fn put(sim: &mut Sim, name: &str, owner: u8, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), owner, FxVec3::from_ints(x, 60, z), 0, true).unwrap()
}

fn run(sim: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        sim.step(&[]);
        sim.take_events();
    }
}

fn drones(sim: &Sim, parent: UnitId) -> Vec<UnitId> {
    let p = sim.unit(parent).unwrap();
    sim.drones_of(p).map(|d| d.id).collect()
}

#[test]
fn drones_are_made_with_their_unit_and_guard_it() {
    let mut sim = field();
    let station = put(&mut sim, "STATION", 0, 800, 800);
    let made = drones(&sim, station);
    assert_eq!(made.len(), 2, "two drones, made as it was finished");
    for &d in &made {
        let u = sim.unit(d).unwrap();
        assert_eq!((u.drone_of, u.owner, u.is_complete()), (Some(station), 0, true));
        assert_eq!(u.orders.front(), Some(&Order::Guard(station)), "guarding it");
        assert_eq!(sim.unit_type(u).locomotion, Locomotion::Air);
        assert!(!u.landed, "taken off");
    }
    run(&mut sim, 30 * 5);
    for &d in &made {
        assert!(sim.unit(d).unwrap().pos.within_xz(FxVec3::from_ints(800, 0, 800), Fx::from_int(200)), "by it");
    }
    // A frame is no unit of its own yet: none for it until it is finished.
    let frame = sim.spawn_unit(sim.types.id("STATION").unwrap(), 0, FxVec3::from_ints(300, 60, 300), 0, false).unwrap();
    assert!(drones(&sim, frame).is_empty());
}

#[test]
fn a_lost_drone_is_rebuilt_at_its_price() {
    let mut sim = field();
    let station = put(&mut sim, "STATION", 0, 800, 800);
    let lost = drones(&sim, station)[0];
    let (metal, energy) = (sim.players[0].economy.metal, sim.players[0].economy.energy);
    sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![lost]) }]);
    run(&mut sim, 2);
    assert!(sim.unit(lost).is_none_or(|u| u.dead), "lost");
    assert_eq!(drones(&sim, station).len(), 1);
    run(&mut sim, 30 * 2);
    assert_eq!(drones(&sim, station).len(), 1, "not yet: three seconds");
    assert!(sim.unit(station).unwrap().drone_work.is_some_and(|(_, w)| w > 0), "being made again");
    run(&mut sim, 30 + 5);
    let now = drones(&sim, station);
    assert_eq!(now.len(), 2, "made again");
    assert!(!now.contains(&lost));
    let (spent_metal, spent_energy) = (metal - sim.players[0].economy.metal, energy - sim.players[0].economy.energy);
    assert!((spent_metal - Fx::from_int(30)).abs() < Fx::from_int(2) && (spent_energy - Fx::from_int(300)).abs() < Fx::from_int(5), "its price: {spent_metal:?} metal, {spent_energy:?} energy");
}

#[test]
fn a_builder_drone_helps_its_unit_mend() {
    let mut sim = field();
    let station = put(&mut sim, "STATION", 0, 800, 800);
    let tank = put(&mut sim, "TANK", 0, 960, 800);
    let max = sim.unit_type(sim.unit(tank).unwrap()).max_health;
    sim.set_health(tank, max / 4);
    run(&mut sim, 30);
    sim.step(&[Command::Order { player: 0, units: vec![station], order: Order::Repair(tank), queue: false }]);
    let mut helped = false;
    for _ in 0..30 * 3 {
        sim.step(&[]);
        helped |= drones(&sim, station).iter().any(|&d| sim.unit(d).unwrap().orders.front() == Some(&Order::Repair(tank)));
    }
    assert!(helped, "its drones help mend what it mends");
    run(&mut sim, 30 * 20);
    assert_eq!(sim.unit(tank).unwrap().health, max, "mended");
    assert!(drones(&sim, station).iter().all(|&d| sim.unit(d).unwrap().orders.front() == Some(&Order::Guard(station))), "back to guarding it");
}

#[test]
fn a_drone_is_held_to_its_leash() {
    let mut sim = field();
    let station = put(&mut sim, "STATION", 0, 800, 800);
    let d = drones(&sim, station)[0];
    run(&mut sim, 30);
    sim.step(&[Command::Order { player: 0, units: vec![d], order: Order::Move(FxVec3::from_ints(1900, 0, 1900)), queue: false }]);
    let mut farthest = Fx::ZERO;
    for _ in 0..30 * 15 {
        sim.step(&[]);
        farthest = farthest.max(sim.unit(d).unwrap().pos.distance_xz(FxVec3::from_ints(800, 0, 800)));
    }
    assert!(farthest < Fx::from_int(420), "held within its leash (300 and the station's size): {farthest:?}");
    let u = sim.unit(d).unwrap();
    assert_eq!(u.orders.front(), Some(&Order::Guard(station)), "back to guarding it");
    assert!(u.pos.within_xz(FxVec3::from_ints(800, 0, 800), Fx::from_int(320)));
}

#[test]
fn drones_die_and_change_sides_with_their_unit() {
    let mut sim = field();
    sim.set_team(1, 0);
    let station = put(&mut sim, "STATION", 0, 800, 800);
    let made = drones(&sim, station);
    // Given to an ally: its drones go with it.
    sim.step(&[Command::GiveUnits { player: 0, units: vec![station], to: 1 }]);
    assert_eq!(sim.unit(station).unwrap().owner, 1);
    assert!(made.iter().all(|&d| sim.unit(d).is_some_and(|u| u.owner == 1 && u.drone_of == Some(station))), "its drones with it");
    // A drone given away on its own is no longer its drone (and is made again).
    sim.step(&[Command::GiveUnits { player: 1, units: vec![made[0]], to: 0 }]);
    assert_eq!(sim.unit(made[0]).unwrap().owner, 1, "a drone goes only with its unit");
    // Destroyed: its drones die with it.
    sim.step(&[Command::Cheat { player: 1, cheat: Cheat::Destroy(vec![station]) }]);
    run(&mut sim, 3);
    assert!(made.iter().all(|&d| sim.unit(d).is_none_or(|u| u.dead)), "they died with it");
}

#[test]
fn an_enhancement_gives_a_drone_and_takes_it_away() {
    let mut sim = field();
    let acu = put(&mut sim, "ACU", 0, 800, 800);
    assert!(drones(&sim, acu).is_empty());
    sim.step(&[Command::Order { player: 0, units: vec![acu], order: Order::Enhance { index: 0, remove: false }, queue: false }]);
    run(&mut sim, 30);
    let d = drones(&sim, acu);
    assert_eq!(d.len(), 1, "made as it is installed");
    sim.step(&[Command::Order { player: 0, units: vec![acu], order: Order::Enhance { index: 0, remove: true }, queue: false }]);
    run(&mut sim, 2);
    assert!(drones(&sim, acu).is_empty() && sim.unit(d[0]).is_none_or(|u| u.dead), "taken out, it goes");
}

/// A unit with drones given away counts with them against the receiver's unit limit, idle as it is:
/// with room for the unit alone, it is not given.
#[test]
fn a_unit_given_with_its_drones_counts_them_against_the_unit_limit() {
    let mut sim = field();
    sim.set_team(1, 0);
    let station = put(&mut sim, "STATION", 0, 800, 800);
    assert_eq!(drones(&sim, station).len(), 2);
    // (Room for one more unit, the station, at player 1.)
    let have = sim.units().filter(|u| !u.dead && u.owner == 1).count() as u32;
    sim.unit_limit = have + 1;
    sim.step(&[Command::GiveUnits { player: 0, units: vec![station], to: 1 }]);
    assert_eq!(sim.unit(station).unwrap().owner, 0, "the station and its two drones are three units: no room for them");
}

/// A drone lost stays lost when the unit's enhancements change otherwise: it is made again in its
/// time and at its price, its rebuilding so far kept, not for nothing at once.
#[test]
fn a_lost_drone_is_not_made_again_for_nothing_by_another_enhancement() {
    let mut sim = field();
    let acu = put(&mut sim, "ACU", 0, 800, 800);
    sim.step(&[Command::Order { player: 0, units: vec![acu], order: Order::Enhance { index: 0, remove: false }, queue: false }]);
    run(&mut sim, 30);
    let pod = drones(&sim, acu);
    assert_eq!(pod.len(), 1, "its pod");
    sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(pod.clone()) }]);
    run(&mut sim, 30);
    let work = sim.unit(acu).unwrap().drone_work;
    assert!(work.is_some_and(|(_, w)| w > 0), "being made again: {work:?}");
    // Another enhancement installed and taken out again.
    for remove in [false, true] {
        sim.step(&[Command::Order { player: 0, units: vec![acu], order: Order::Enhance { index: 1, remove }, queue: false }]);
        run(&mut sim, 30);
    }
    assert!(drones(&sim, acu).is_empty(), "not made for nothing");
    assert!(sim.unit(acu).unwrap().drone_work.is_some_and(|(_, w)| w > work.unwrap().1), "its rebuilding kept, and going on");
}

/// An upgraded unit's drones go to the unit it becomes, which makes the one more it keeps.
#[test]
fn an_upgraded_units_drones_go_to_what_it_becomes() {
    let mut sim = field();
    let station = put(&mut sim, "STATION", 0, 800, 800);
    let before = drones(&sim, station);
    sim.step(&[Command::Order { player: 0, units: vec![station], order: Order::Upgrade(sim.types.id("STATION2").unwrap()), queue: false }]);
    run(&mut sim, 60);
    let upgraded = sim.units().find(|u| !u.dead && sim.unit_type(u).name == "STATION2").expect("upgraded").id;
    let after = drones(&sim, upgraded);
    assert_eq!(after.len(), 3, "three kept");
    assert!(before.iter().all(|d| after.contains(d)), "its two among them: {before:?} {after:?}");
    assert!(after.iter().all(|&d| sim.unit(d).unwrap().drone_of == Some(upgraded)));
}

#[test]
fn drones_play_the_same_on_every_computer() {
    let play = || {
        let mut sim = field();
        let station = put(&mut sim, "STATION", 0, 800, 800);
        let tank = put(&mut sim, "TANK", 0, 900, 800);
        sim.set_health(tank, Fx::from_int(300));
        let d = drones(&sim, station)[1];
        sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![d]) }]);
        (0..30 * 8).map(|_| {
            sim.step(&[]);
            sim.checksum()
        }).collect::<Vec<u64>>()
    };
    assert_eq!(play(), play());
}
