//! Teleporting (`CanTeleport`, `TeleportEnergy`, `TeleportTime`, `Order::Teleport`: TA Reborn's own),
//! played out with made-up units (no TA install needed): a unit charges, standing and paying its
//! energy as construction pays, then lands on the nearest spot it can stand on; Stop calls the
//! charge off; short of energy it waits; nowhere to land fails the order; an enhancement may give
//! the teleport.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, SimEvent, UnitId, UnitTypes};

fn data_of(files: &[(String, String)]) -> GameData {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta-sim-teleport-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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

/// A map of 128 × 128 cells, its western half under the sea.
fn sim_of(data: GameData) -> Sim {
    let data = Arc::new(data);
    let types = Arc::new(UnitTypes::compile(&data, &|_| None));
    let cells = 128;
    let heights = (0..cells * cells).map(|i| if i % cells < cells / 2 { 10 } else { 60 }).collect();
    let map = TntMap { width: cells as u32, height: cells as u32, sea_level: 40, tiles: Vec::new(), tile_map: Vec::new(), heights, features: vec![FEATURE_NONE; cells * cells], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    let setups: Vec<PlayerSetup> = (0..2).map(|i| PlayerSetup { side: "ARM".into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
    let mut sim = Sim::new(types, data, &map, &MapInfo::default(), 0, &setups, 7);
    for p in &mut sim.players {
        p.endless = true;
    }
    sim
}

const WALKER: &str = "CanMove=1; BMcode=1; MaxVelocity=2; Acceleration=0.5; BrakeRate=1; TurnRate=2000; FootprintX=2; FootprintZ=2; MaxWaterDepth=0;";

fn field() -> Sim {
    sim_of(data_of(&[
        // 300 energy and 0.1 more a pixel, over two seconds.
        unit("PORTER", &format!("{WALKER} CanTeleport=1; TeleportEnergy=300 0.1; TeleportTime=2;")),
        unit("PLAIN", WALKER),
        unit("ACU", &format!("{WALKER} Builder=1; WorkerTime=100; [ENHANCEMENTS] {{ [Teleporter] {{ Slot=Back; BuildTime=1; CanTeleport=1; TeleportEnergy=50; TeleportTime=1; }} }}")),
    ]))
}

fn put(sim: &mut Sim, name: &str, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), 0, FxVec3::from_ints(x, 60, z), 0, true).unwrap()
}

fn run(sim: &mut Sim, ticks: u32) -> Vec<SimEvent> {
    let mut events = Vec::new();
    for _ in 0..ticks {
        sim.step(&[]);
        events.extend(sim.take_events());
    }
    events
}

fn order(sim: &mut Sim, id: UnitId, order: Order) {
    sim.step(&[Command::Order { player: 0, units: vec![id], order, queue: false }]);
}

#[test]
fn a_teleport_charges_then_lands_where_it_can_stand() {
    let mut sim = field();
    let porter = put(&mut sim, "PORTER", 1200, 400);
    let plain = put(&mut sim, "PLAIN", 1250, 400);
    let to = FxVec3::from_ints(1800, 0, 1700);
    let before = sim.players[0].economy.energy;
    sim.step(&[Command::Order { player: 0, units: vec![porter, plain], order: Order::Teleport(to), queue: false }]);
    assert!(sim.unit(plain).unwrap().orders.is_empty(), "a unit that cannot teleport is not given the order");
    let start = sim.unit(porter).unwrap().pos;
    let events = run(&mut sim, 40);
    let charge = sim.unit(porter).unwrap().teleport.expect("charging");
    assert_eq!(sim.unit(porter).unwrap().pos, start, "it stands while it charges");
    assert!(charge.progress() > Fx::from_ratio(1, 2) && charge.progress() < Fx::ONE, "two seconds' charge, two thirds along: {:?}", charge.progress());
    assert!(!events.iter().any(|e| matches!(e, SimEvent::Teleported { .. })));
    let events = run(&mut sim, 30);
    let landed = events.iter().find_map(|e| match e {
        SimEvent::Teleported { id, from, to } if *id == porter => Some((*from, *to)),
        _ => None,
    });
    let (from, at) = landed.expect("it teleported");
    assert_eq!(from, start);
    let u = sim.unit(porter).unwrap();
    assert!(u.pos.within_xz(to, Fx::from_int(24)) && u.pos == at, "landed where it was sent: {:?}", u.pos);
    assert_eq!(u.pos.y, Fx::from_int(60), "on the ground");
    assert!(u.teleport.is_none() && u.orders.is_empty(), "its order done");
    // Its charge: 300 and 0.1 a pixel of the way (some 1,400 px), paid as construction pays.
    let spent = before - sim.players[0].economy.energy;
    let distance = start.distance_xz(to).to_f64();
    let expected = 300.0 + 0.1 * distance;
    assert!((spent.to_f64() - expected).abs() < 2.0, "paid {spent:?} for {expected}");
}

#[test]
fn stop_calls_a_charge_off() {
    let mut sim = field();
    let porter = put(&mut sim, "PORTER", 1200, 400);
    let start = sim.unit(porter).unwrap().pos;
    order(&mut sim, porter, Order::Teleport(FxVec3::from_ints(1800, 0, 1700)));
    run(&mut sim, 20);
    assert!(sim.unit(porter).unwrap().teleport.is_some());
    let energy = sim.players[0].economy.energy;
    order(&mut sim, porter, Order::Stop);
    let events = run(&mut sim, 90);
    assert!(sim.unit(porter).unwrap().teleport.is_none(), "the charge called off");
    assert!(!events.iter().any(|e| matches!(e, SimEvent::Teleported { .. })), "it never went");
    assert_eq!(sim.unit(porter).unwrap().pos, start);
    assert!(sim.players[0].economy.energy >= energy, "nothing more paid");
}

#[test]
fn short_of_energy_a_charge_waits() {
    let mut sim = field();
    let porter = put(&mut sim, "PORTER", 1200, 400);
    sim.players[0].economy.energy = Fx::ZERO;
    order(&mut sim, porter, Order::Teleport(FxVec3::from_ints(1800, 0, 1700)));
    run(&mut sim, 90);
    assert_eq!(sim.unit(porter).unwrap().teleport.map(|c| c.work), Some(0), "nothing paid, nothing charged");
    sim.players[0].economy.energy = Fx::from_int(1000);
    let events = run(&mut sim, 90);
    assert!(events.iter().any(|e| matches!(e, SimEvent::Teleported { id, .. } if *id == porter)), "with its energy, it goes");
}

#[test]
fn nowhere_to_land_fails_the_order() {
    let mut sim = field();
    let porter = put(&mut sim, "PORTER", 1200, 400);
    // (Out at sea: a walker that goes no deeper than the shore stands nowhere near it.)
    order(&mut sim, porter, Order::Teleport(FxVec3::from_ints(300, 0, 1000)));
    let events = run(&mut sim, 5);
    assert!(events.iter().any(|e| matches!(e, SimEvent::OrderFailed { unit, reason: "nowhere to land there" } if *unit == porter)));
    assert!(sim.unit(porter).unwrap().orders.is_empty() && sim.unit(porter).unwrap().teleport.is_none());
    // Near the shore it lands on the nearest dry ground.
    order(&mut sim, porter, Order::Teleport(FxVec3::from_ints(1000, 0, 1000)));
    run(&mut sim, 90);
    let u = sim.unit(porter).unwrap();
    assert!(u.pos.x >= Fx::from_int(1024) && u.pos.within_xz(FxVec3::from_ints(1000, 0, 1000), Fx::from_int(96)), "on the shore: {:?}", u.pos);
}

#[test]
fn an_enhancement_gives_the_teleport() {
    let mut sim = field();
    let acu = put(&mut sim, "ACU", 1200, 400);
    assert!(!sim.can_teleport(sim.unit(acu).unwrap()));
    order(&mut sim, acu, Order::Teleport(FxVec3::from_ints(1500, 0, 900)));
    assert!(sim.unit(acu).unwrap().orders.is_empty(), "not yet");
    order(&mut sim, acu, Order::Enhance { index: 0, remove: false });
    run(&mut sim, 30);
    assert!(sim.can_teleport(sim.unit(acu).unwrap()), "installed, it can");
    order(&mut sim, acu, Order::Teleport(FxVec3::from_ints(1500, 0, 900)));
    let events = run(&mut sim, 45);
    assert!(events.iter().any(|e| matches!(e, SimEvent::Teleported { id, .. } if *id == acu)));
}

#[test]
fn teleports_play_the_same_on_every_computer() {
    let play = || {
        let mut sim = field();
        let porter = put(&mut sim, "PORTER", 1200, 400);
        let acu = put(&mut sim, "ACU", 1300, 400);
        sim.step(&[
            Command::Order { player: 0, units: vec![porter], order: Order::Teleport(FxVec3::from_ints(1800, 0, 1700)), queue: false },
            Command::Order { player: 0, units: vec![acu], order: Order::Enhance { index: 0, remove: false }, queue: false },
        ]);
        sim.step(&[Command::Order { player: 0, units: vec![acu], order: Order::Teleport(FxVec3::from_ints(1500, 0, 1500)), queue: true }]);
        (0..30 * 6).map(|_| {
            sim.step(&[]);
            sim.checksum()
        }).collect::<Vec<u64>>()
    };
    assert_eq!(play(), play());
}
