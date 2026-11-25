//! Air transports with slots (`TransportSlots`, `TransportClass`: TA Reborn's own), played out with
//! made-up units on a flat map (no TA install needed): a transport loads three units one after
//! another, each into the next free slot, carries them hanging under their slots, and sets them
//! down at once, spread out; slots take passengers by their class, a larger one overlapping the
//! smaller nearest it; and a transport without slots carries one, whatever its file says, as TA's
//! Atlas does.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ta_data::{GameData, MapInfo};
use ta_formats::model3do::{Model3do, Piece};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, SimEvent, UnitId, UnitTypes, piece_world_position};

/// Game data of the files given (path, text), as a mod's would be read.
fn data_of(files: &[(String, String)]) -> GameData {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta-sim-slots-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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

/// A 3DO of a root and the pieces given under it (name, offset in px: x, y, z as the file has it).
fn model(pieces: &[(&str, [i32; 3])]) -> Vec<u8> {
    let piece = |name: &str, parent: Option<usize>, offset: [i32; 3], children: Vec<usize>| Piece {
        name: name.into(),
        parent,
        offset: offset.map(|v| v << 16),
        vertices: Vec::new(),
        primitives: Vec::new(),
        selection_primitive: None,
        children,
    };
    let mut m = Model3do::default();
    m.pieces.push(piece("base", None, [0; 3], (1..=pieces.len()).collect()));
    for &(name, offset) in pieces {
        m.pieces.push(piece(name, Some(0), offset, Vec::new()));
    }
    m.to_bytes()
}

/// A flat map of `cells` × `cells` 16-px cells, two players.
fn sim_of(data: GameData, models: HashMap<String, Vec<u8>>, cells: usize) -> Sim {
    let data = Arc::new(data);
    let types = Arc::new(UnitTypes::compile(&data, &|p| models.get(p).cloned()));
    let map = TntMap { width: cells as u32, height: cells as u32, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; cells * cells], features: vec![FEATURE_NONE; cells * cells], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    let setups: Vec<PlayerSetup> = (0..2).map(|i| PlayerSetup { side: "ARM".into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
    let mut sim = Sim::new(types, data, &map, &MapInfo::default(), 0, &setups, 7);
    for p in &mut sim.players {
        p.endless = true;
    }
    sim
}

const FLIER: &str = "CanFly=1; CanMove=1; MaxVelocity=4; Acceleration=1; BrakeRate=1; TurnRate=2048; CruiseAlt=80; TransportSize=3; FootprintX=2; FootprintZ=2;";
const TANK: &str = "CanMove=1; BMcode=1; MaxVelocity=2; Acceleration=0.5; BrakeRate=1; TurnRate=2000; FootprintX=2; FootprintZ=2;";

/// A transport with four small slots, two at its front and two at its back, and a medium one
/// between each pair; small and medium tanks; and an Atlas-like transport naming no slots.
fn lift_field() -> Sim {
    let data = data_of(&[
        unit("LIFTER", &format!("{FLIER} TransportSlots=FL FR BL BR MF:2 MB:2;")),
        unit("ATLAS", &format!("{FLIER} TransportCapacity=5;")),
        unit("TANK", TANK),
        unit("BIGTANK", &format!("{TANK} TransportClass=2;")),
    ]);
    let lifter = model(&[("FL", [-10, -6, 12]), ("FR", [10, -6, 12]), ("BL", [-10, -6, -12]), ("BR", [10, -6, -12]), ("MF", [0, -6, 12]), ("MB", [0, -6, -12])]);
    sim_of(data, HashMap::from([("objects3d/lifter.3do".to_string(), lifter)]), 128)
}

fn put(sim: &mut Sim, name: &str, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), 0, FxVec3::from_ints(x, 60, z), 0, true).unwrap()
}

fn load(sim: &mut Sim, transport: UnitId, units: &[UnitId]) {
    for (k, &u) in units.iter().enumerate() {
        sim.step(&[Command::Order { player: 0, units: vec![transport], order: Order::Load(u), queue: k > 0 }]);
    }
}

/// Steps until `done` (at most `ticks`), the events of the way.
fn until(sim: &mut Sim, ticks: u32, done: impl Fn(&Sim) -> bool) -> Vec<SimEvent> {
    let mut events = Vec::new();
    for _ in 0..ticks {
        if done(sim) {
            break;
        }
        sim.step(&[]);
        events.extend(sim.take_events());
    }
    events
}

#[test]
fn a_transport_with_slots_loads_three_carries_them_and_sets_them_down_spread_out() {
    let mut sim = lift_field();
    let lifter = put(&mut sim, "LIFTER", 300, 300);
    let tanks = [put(&mut sim, "TANK", 600, 300), put(&mut sim, "TANK", 640, 420), put(&mut sim, "TANK", 520, 520)];
    let t = sim.unit_type(sim.unit(lifter).unwrap()).clone();
    assert_eq!(t.transport_capacity, 4, "four small passengers at once");
    load(&mut sim, lifter, &tanks);
    until(&mut sim, 30 * 60, |sim| tanks.iter().all(|&k| sim.unit(k).unwrap().carried_by == Some(lifter)));
    assert!(tanks.iter().all(|&k| sim.unit(k).unwrap().carried_by == Some(lifter)), "all three aboard");
    let slots: Vec<u8> = tanks.iter().map(|&k| sim.unit(k).unwrap().carry_slot.unwrap()).collect();
    assert_eq!(slots, [0, 1, 2], "each in the next free slot, in the list's order");
    // Each hangs under its slot's piece (these have no height: at it), and is not stowed away
    // (with the fog of war off, the enemy sees it).
    sim.los_enabled = false;
    sim.step(&[]);
    let carrier = sim.unit(lifter).unwrap();
    for &k in &tanks {
        let u = sim.unit(k).unwrap();
        let piece = u.carry_piece.expect("hanging from a piece");
        assert_eq!(piece, t.transport_slots[u.carry_slot.unwrap() as usize].piece);
        assert_eq!(u.pos, piece_world_position(carrier, &t, carrier.cob.as_ref(), piece), "at its slot");
        assert_eq!(sim.seen_by(u, 1), ta_sim::Seen::Visible, "drawn, not stowed inside");
    }
    // Carried across the map, and set down there at once, spread out.
    let dest = FxVec3::from_ints(1500, 60, 1400);
    sim.step(&[Command::Order { player: 0, units: vec![lifter], order: Order::Unload(dest), queue: false }]);
    let events = until(&mut sim, 30 * 60, |sim| sim.unit(lifter).unwrap().cargo.is_empty());
    assert!(sim.unit(lifter).unwrap().cargo.is_empty(), "everyone set down");
    let ticks: Vec<_> = events.iter().filter(|e| matches!(e, SimEvent::UnitUnloaded { .. })).collect();
    assert_eq!(ticks.len(), 3);
    let places: Vec<FxVec3> = tanks.iter().map(|&k| sim.unit(k).unwrap().pos).collect();
    for (i, p) in places.iter().enumerate() {
        assert!(sim.unit(tanks[i]).unwrap().carried_by.is_none() && sim.unit(tanks[i]).unwrap().carry_slot.is_none());
        assert!(p.within_xz(dest, Fx::from_int(120)), "round the place it was sent: {p:?}");
        assert_eq!(p.y, Fx::from_int(60), "on the ground");
        for q in &places[i + 1..] {
            assert!(!p.within_xz(*q, Fx::from_int(30)), "spread out: {p:?} and {q:?}");
        }
    }
    assert!(sim.unit(lifter).unwrap().orders.is_empty(), "the order done");
}

#[test]
fn slots_take_passengers_by_their_class() {
    let mut sim = lift_field();
    let t = sim.types.by_name("LIFTER").unwrap().clone();
    assert!(t.fits(&[1, 1, 1, 1]) && !t.fits(&[1, 1, 1, 1, 1]), "four small");
    assert!(t.fits(&[2, 2]) && !t.fits(&[2, 2, 1]), "two medium, each over its pair");
    assert!(t.fits(&[2, 1, 1]) && !t.fits(&[2, 1, 1, 1]), "a medium and the other pair");
    assert!(!t.fits(&[3]), "no slot takes a large one");
    let lifter = put(&mut sim, "LIFTER", 300, 300);
    let big = put(&mut sim, "BIGTANK", 500, 300);
    let small: Vec<UnitId> = (0..3).map(|k| put(&mut sim, "TANK", 500 + 80 * k, 450)).collect();
    load(&mut sim, lifter, &[big, small[0], small[1], small[2]]);
    let events = until(&mut sim, 30 * 90, |sim| sim.unit(lifter).unwrap().orders.is_empty());
    let aboard: Vec<UnitId> = sim.unit(lifter).unwrap().cargo.clone();
    assert_eq!(aboard, [big, small[0], small[1]], "the medium one and two small; no room for the third");
    assert_eq!(sim.unit(big).unwrap().carry_slot, Some(4), "the medium one in the front medium slot");
    assert_eq!((sim.unit(small[0]).unwrap().carry_slot, sim.unit(small[1]).unwrap().carry_slot), (Some(2), Some(3)), "the small ones at the back");
    assert!(events.iter().any(|e| matches!(e, SimEvent::OrderFailed { unit, reason: "cannot carry that" } if *unit == lifter)), "the third refused");
    assert!(!sim.room_for(sim.unit(lifter).unwrap(), sim.types.by_name("TANK").unwrap()));
}

#[test]
fn a_transport_without_slots_carries_one_whatever_its_file_says() {
    let mut sim = lift_field();
    assert_eq!(sim.types.by_name("ATLAS").unwrap().transport_capacity, 1, "TA's Atlas says 5 and carries one");
    let atlas = put(&mut sim, "ATLAS", 300, 300);
    let tanks = [put(&mut sim, "TANK", 500, 300), put(&mut sim, "TANK", 500, 450)];
    load(&mut sim, atlas, &tanks);
    until(&mut sim, 30 * 60, |sim| sim.unit(atlas).unwrap().orders.is_empty());
    assert_eq!(sim.unit(atlas).unwrap().cargo, [tanks[0]], "one aboard");
    assert!(sim.unit(tanks[0]).unwrap().carry_slot.is_none(), "no slot of its own");
}

#[test]
fn transports_with_slots_play_the_same_on_every_computer() {
    let play = || {
        let mut sim = lift_field();
        let lifter = put(&mut sim, "LIFTER", 300, 300);
        let tanks = [put(&mut sim, "TANK", 600, 300), put(&mut sim, "BIGTANK", 640, 420), put(&mut sim, "TANK", 520, 520)];
        load(&mut sim, lifter, &tanks);
        sim.step(&[Command::Order { player: 0, units: vec![lifter], order: Order::Unload(FxVec3::from_ints(1200, 60, 1200)), queue: true }]);
        (0..30 * 50).map(|_| {
            sim.step(&[]);
            sim.checksum()
        }).collect::<Vec<u64>>()
    };
    assert_eq!(play(), play());
}
