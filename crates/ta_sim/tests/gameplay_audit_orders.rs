//! Player orders and transport lifecycle regressions, with original test data only.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ta_data::{GameData, MapInfo};
use ta_formats::cob::{CobFunction, CobScript, op};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::cob::CobProgram;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitId, UnitTypes};

fn fixture() -> Sim {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta-gameplay-audit-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(dir.join("units")).unwrap();
    for (name, keys) in [
        ("BUILDER", "Builder=1; CanCapture=1; CanReclaim=1; WorkerTime=300; BuildDistance=200; SightDistance=300;"),
        ("TRANSPORT", "TransportCapacity=1; TransportSize=2; BuildDistance=100;"),
        ("PASSENGER", ""),
        ("FACTORY", "Builder=1; WorkerTime=30; YardMap=y;"),
    ] {
        let speed = if name == "FACTORY" { 0 } else { 1 };
        let text = format!("[UNITINFO] {{ UnitName={name}; Objectname={name}; MaxDamage=1000; CanMove=1; MaxVelocity={speed}; TurnRate=2048; Acceleration=0.1; BrakeRate=0.1; FootprintX=1; FootprintZ=1; BuildCostMetal=10; BuildTime=100; {keys} }}");
        std::fs::write(dir.join(format!("units/{name}.fbi")), text).unwrap();
    }
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_mod_dir(&dir, "test").unwrap();
    let mut data = GameData::load(&vfs);
    let factory = data.unit_id("FACTORY").unwrap();
    data.units[factory].build_options = vec!["PASSENGER".into()];
    let data = Arc::new(data);
    let mut types = UnitTypes::compile(&data, &|_| None);
    let factory = types.id("FACTORY").unwrap();
    // Original minimal script: the factory is ready to work without an animation delay.
    types.types[factory].script = Some(Arc::new(CobProgram::new(CobScript {
        functions: vec![CobFunction { name: "Create".into(), offset: 0 }],
        code: vec![op::PUSH_CONSTANT, 5, op::PUSH_CONSTANT, 1, op::SET_VALUE, op::PUSH_CONSTANT, 0, op::RETURN],
        ..Default::default()
    }, &[])));
    let types = Arc::new(types);
    std::fs::remove_dir_all(&dir).unwrap();
    let map = TntMap { width: 64, height: 64, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; 64 * 64], features: vec![FEATURE_NONE; 64 * 64], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    let players: Vec<_> = (0..2).map(|i| PlayerSetup { side: "ARM".into(), color: i, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
    let mut sim = Sim::new(types, data, &map, &MapInfo::default(), 0, &players, 7);
    for p in &mut sim.players {
        p.endless = true;
    }
    sim
}

fn spawn(sim: &mut Sim, name: &str, player: u8, x: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), player, FxVec3::from_ints(x, 60, 300), 0, true).unwrap()
}

fn order(sim: &mut Sim, player: u8, unit: UnitId, order: Order) {
    sim.step(&[Command::Order { player, units: vec![unit], order, queue: false }]);
}

#[test]
fn capturing_a_loaded_transport_transfers_its_passenger_too() {
    let mut sim = fixture();
    let builder = spawn(&mut sim, "BUILDER", 0, 240);
    let carrier = spawn(&mut sim, "TRANSPORT", 1, 300);
    let passenger = spawn(&mut sim, "PASSENGER", 1, 330);
    order(&mut sim, 1, carrier, Order::Load(passenger));
    assert_eq!(sim.unit(passenger).unwrap().carried_by, Some(carrier));
    assert_eq!(sim.seen_by(sim.unit(carrier).unwrap(), 0), ta_sim::Seen::Visible);
    sim.unit_limit = 2; // The builder plus the transport fits; its passenger does not.
    order(&mut sim, 0, builder, Order::Capture(carrier));
    for _ in 0..60 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(carrier).unwrap().owner, 1, "the entire captured group must fit");
    assert_eq!(sim.unit(carrier).unwrap().capture_progress, Fx::ONE);
    sim.unit_limit = 3;
    sim.step(&[]);
    assert_eq!(sim.unit(carrier).unwrap().owner, 0, "the capture finishes");
    assert_eq!(sim.unit(passenger).unwrap().owner, 0, "cargo belongs to the captured transport's new owner");
    assert_eq!((sim.players[0].unit_count, sim.players[1].unit_count), (3, 0));
    order(&mut sim, 0, carrier, Order::Unload(FxVec3::from_ints(340, 60, 300)));
    assert_eq!(sim.unit(passenger).unwrap().carried_by, None);
    assert_eq!(sim.unit(passenger).unwrap().owner, 0);
}

#[test]
fn a_passenger_dying_aboard_frees_the_transport_for_another_load() {
    let mut sim = fixture();
    let carrier = spawn(&mut sim, "TRANSPORT", 0, 300);
    let passenger = spawn(&mut sim, "PASSENGER", 0, 330);
    // A countdown started before loading must still finish while aboard.
    sim.step(&[Command::SelfDestruct { player: 0, units: vec![passenger] }]);
    order(&mut sim, 0, carrier, Order::Load(passenger));
    assert_eq!(sim.unit(passenger).unwrap().carried_by, Some(carrier));
    for _ in 0..160 {
        sim.step(&[]);
    }
    assert!(sim.unit(passenger).is_none());
    assert!(sim.unit(carrier).unwrap().cargo.is_empty(), "a dead passenger no longer occupies the transport");
    let next = spawn(&mut sim, "PASSENGER", 0, 330);
    order(&mut sim, 0, carrier, Order::Load(next));
    assert_eq!(sim.unit(next).unwrap().carried_by, Some(carrier));
}

#[test]
fn a_factory_gift_needs_room_for_the_unit_on_its_build_pad() {
    let mut sim = fixture();
    sim.set_team(1, 0);
    let factory = spawn(&mut sim, "FACTORY", 0, 300);
    let product = sim.types.id("PASSENGER").unwrap();
    sim.step(&[Command::Produce { player: 0, factory, type_id: product, count: 1 }]);
    sim.step(&[]);
    let frame = sim.unit(factory).unwrap().build.expect("the factory starts production").target;
    assert!(!sim.unit(frame).unwrap().is_complete());
    sim.unit_limit = 1;
    sim.step(&[Command::GiveUnits { player: 0, units: vec![factory], to: 1 }]);
    assert_eq!(sim.unit(factory).unwrap().owner, 0, "the recipient cannot fit the factory and its product");
    assert_eq!(sim.unit(frame).unwrap().owner, 0);
    assert_eq!(sim.players[1].unit_count, 0);
    sim.unit_limit = 2;
    sim.step(&[Command::GiveUnits { player: 0, units: vec![factory], to: 1 }]);
    assert_eq!(sim.unit(factory).unwrap().owner, 1);
    assert_eq!(sim.unit(frame).unwrap().owner, 1);
    assert_eq!(sim.players[1].unit_count, 2);
}

#[test]
fn a_loaded_transport_gift_counts_and_transfers_nested_cargo() {
    let mut sim = fixture();
    sim.set_team(1, 0);
    let outer = spawn(&mut sim, "TRANSPORT", 0, 240);
    let inner = spawn(&mut sim, "TRANSPORT", 0, 300);
    let passenger = spawn(&mut sim, "PASSENGER", 0, 330);
    order(&mut sim, 0, inner, Order::Load(passenger));
    order(&mut sim, 0, outer, Order::Load(inner));
    assert_eq!(sim.unit(inner).unwrap().carried_by, Some(outer));
    sim.unit_limit = 2;
    sim.step(&[Command::GiveUnits { player: 0, units: vec![outer], to: 1 }]);
    assert_eq!(sim.players[1].unit_count, 0, "the entire carried group must fit");
    sim.unit_limit = 3;
    sim.step(&[Command::GiveUnits { player: 0, units: vec![outer], to: 1 }]);
    for id in [outer, inner, passenger] {
        assert_eq!(sim.unit(id).unwrap().owner, 1);
    }
    assert_eq!((sim.players[0].unit_count, sim.players[1].unit_count), (0, 3));
}

#[test]
fn a_mission_script_can_follow_an_enemy_without_allowing_player_guard_orders() {
    let mut sim = fixture();
    // Can Rush's jammer ultimately follows the player's Fusion Reactor with `g armfus`.
    // The same mission command by type on the synthetic map must survive execution.
    let mission = ta_data::mission::Mission::from_ota("Guard.ota", &ta_formats::tdf::parse(r#"
        [GlobalHeader] { DestroyAllUnits=1; [Schema 0] { Type=Easy; [units] {
            [unit0] { Unitname=BUILDER; Player=2; XPos=200; ZPos=300; Ident=FOLLOWER; InitialMission=g FACTORY; }
            [unit1] { Unitname=FACTORY; Player=1; XPos=700; ZPos=300; Ident=TARGET; }
        } } }
    "#));
    let setup = ta_sim::mission::setup_mission(&mut sim, &mission, &mission.schemas[0], None);
    let (follower, target) = (setup.idents["FOLLOWER"], setup.idents["TARGET"]);
    assert_eq!(sim.unit(follower).unwrap().orders.front(), Some(&Order::Guard(target)));
    let before = sim.unit(follower).unwrap().pos;
    sim.step(&[Command::Produce { player: 0, factory: target, type_id: sim.types.id("PASSENGER").unwrap(), count: 1 }]);
    for _ in 0..120 {
        sim.step(&[]);
        assert_eq!(sim.unit(follower).unwrap().orders.front(), Some(&Order::Guard(target)), "hostile following never assists the enemy factory");
    }
    assert_eq!(sim.unit(follower).unwrap().orders.front(), Some(&Order::Guard(target)), "the scripted hostile guard stays active");
    assert!(sim.unit(follower).unwrap().pos.distance_xz(sim.unit(target).unwrap().pos) < before.distance_xz(sim.unit(target).unwrap().pos));
    assert!(sim.runs_mission_script(follower));
    let scripted_orders = sim.unit(follower).unwrap().orders.clone();
    order(&mut sim, 1, follower, Order::Guard(target));
    assert_eq!(sim.unit(follower).unwrap().orders, scripted_orders, "an invalid player command does not replace the active script");
    assert!(sim.runs_mission_script(follower));
    order(&mut sim, 1, follower, Order::Stop);
    assert!(!sim.runs_mission_script(follower));
    order(&mut sim, 1, follower, Order::Guard(target));
    assert!(sim.unit(follower).unwrap().orders.is_empty(), "a player's hostile guard command remains invalid");
    assert!(sim.unit(follower).unwrap().path_goal.is_none());
}


/// Record actual visits, including unexpected waypoint order, rather than inspecting
/// the queue's implementation. These points are separated by at least 500 pixels.
fn note_visit(pos: FxVec3, points: &[FxVec3], visits: &mut Vec<usize>) {
    if let Some(index) = points.iter().position(|p| pos.within_xz(*p, Fx::from_int(64))) && visits.last() != Some(&index) {
        visits.push(index);
    }
}

#[test]
fn queued_patrol_visits_waypoints_in_click_order_and_can_be_replaced() {
    let mut sim = fixture();
    let points = [(200, 300), (800, 300), (800, 850), (200, 850)].map(|(x, z)| FxVec3::from_ints(x, 60, z));
    let unit = spawn(&mut sim, "PASSENGER", 0, 200);
    let command = |order, queue| Command::Order { player: 0, units: vec![unit], order, queue };
    let mut visits = vec![0];
    sim.step(&[command(Order::Patrol(points[1]), false)]);
    // Subsequent Shift clicks arrive while the unit is already walking its first leg.
    for point in &points[2..] {
        for _ in 0..15 { sim.step(&[]); }
        sim.step(&[command(Order::Patrol(*point), true)]);
    }
    for _ in 0..6000 {
        sim.step(&[]);
        note_visit(sim.unit(unit).unwrap().pos, &points, &mut visits);
        if visits.len() == 9 { break; }
    }
    assert_eq!(visits, [0, 1, 2, 3, 0, 1, 2, 3, 0], "each explicit point and the implicit starting point must be visited in sequence on successive laps");
    let replacement = FxVec3::from_ints(500, 60, 500);
    sim.step(&[command(Order::Move(replacement), false)]);
    for _ in 0..1200 { sim.step(&[]); }
    let u = sim.unit(unit).unwrap();
    assert!(u.orders.is_empty(), "an ordinary replacement command ends the patrol loop");
    assert!(u.pos.within_xz(replacement, Fx::from_int(32)));
}

#[test]
fn queued_move_then_patrol_closes_at_the_move_destination() {
    let mut sim = fixture();
    let points = [(200, 300), (800, 300), (800, 850), (200, 850)].map(|(x, z)| FxVec3::from_ints(x, 60, z));
    let unit = spawn(&mut sim, "PASSENGER", 0, 200);
    let command = |order, queue| Command::Order { player: 0, units: vec![unit], order, queue };
    sim.step(&[
        command(Order::Move(points[1]), false),
        command(Order::Patrol(points[2]), true),
        command(Order::Patrol(points[3]), true),
    ]);
    let mut visits = vec![0];
    for _ in 0..6000 {
        sim.step(&[]);
        note_visit(sim.unit(unit).unwrap().pos, &points, &mut visits);
        if visits.len() == 8 { break; }
    }
    assert_eq!(visits, [0, 1, 2, 3, 1, 2, 3, 1], "the initial move is completed once; the following patrol closes at that destination, without returning to the original position");
}

#[test]
fn factory_shift_patrol_routes_reach_every_waypoint_for_each_product() {
    let mut sim = fixture();
    let factory = spawn(&mut sim, "FACTORY", 0, 200);
    let product = sim.types.id("PASSENGER").unwrap();
    let points = [(800, 300), (800, 850), (200, 850)].map(|(x, z)| FxVec3::from_ints(x, 60, z));
    let command = |order, queue| Command::Order { player: 0, units: vec![factory], order, queue };
    sim.step(&[
        Command::Produce { player: 0, factory, type_id: product, count: 2 },
        command(Order::Move(points[0]), false),
        command(Order::Patrol(points[1]), true),
        command(Order::Patrol(points[2]), true),
    ]);
    let mut visits: std::collections::BTreeMap<UnitId, Vec<usize>> = Default::default();
    for _ in 0..6000 {
        sim.step(&[]);
        for unit in sim.units().filter(|u| u.type_id == product && u.is_complete()) {
            let route = visits.entry(unit.id).or_default();
            if route.len() < 7 { note_visit(unit.pos, &points, route); }
        }
        if visits.len() == 2 && visits.values().all(|v| v.len() == 7) { break; }
    }
    assert_eq!(visits.len(), 2, "patrolling products must release the factory for its next unit");
    for route in visits.values() {
        assert_eq!(route, &[0, 1, 2, 0, 1, 2, 0], "both products inherit the complete rally route and loop from its first waypoint");
    }
    assert!(sim.unit(factory).unwrap().factory_queue.is_empty());
}


#[test]
fn grouped_shift_patrols_preserve_every_leg_with_both_movement_rules() {
    for modern in [false, true] {
        let mut sim = fixture();
        sim.group_formations = modern;
        let ids = [spawn(&mut sim, "PASSENGER", 0, 200), spawn(&mut sim, "PASSENGER", 0, 240)];
        let points = [(220, 300), (780, 300), (780, 850), (220, 850)].map(|(x, z)| FxVec3::from_ints(x, 60, z));
        let commands: Vec<_> = points.iter().skip(1).enumerate().map(|(i, &p)| Command::Order {
            player: 0, units: ids.to_vec(), order: Order::Patrol(p), queue: i > 0,
        }).collect();
        sim.step(&commands);
        let mut visits = [vec![0], vec![0]];
        for _ in 0..6000 {
            sim.step(&[]);
            for (id, route) in ids.iter().zip(&mut visits) {
                if route.len() < 9 { note_visit(sim.unit(*id).unwrap().pos, &points, route); }
            }
            if visits.iter().all(|route| route.len() == 9) { break; }
        }
        for route in visits {
            assert_eq!(route, [0, 1, 2, 3, 0, 1, 2, 3, 0], "both selected units must traverse the complete loop (modern movement: {modern})");
        }
    }
}
