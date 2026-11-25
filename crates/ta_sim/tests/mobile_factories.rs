//! Factories that move (`MobileFactory`, `BuildPiece`: TA Reborn's own), played out with made-up
//! units on a flat map (no TA install needed): a unit built on the factory, at its build piece, while
//! it goes; set down behind it once done, or taking off from it if it flies; a building left out of
//! its queue, and no unit of its menu built in the field; and its unit dying with it.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ta_data::{GameData, MapInfo};
use ta_formats::model3do::{Model3do, Piece};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::{Cheat, Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitId, UnitTypes, piece_world_position};

fn data_of(files: &[(String, String)]) -> GameData {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta-sim-mobile-factories-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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

/// A model of a root and a pad 20 px up its back.
fn model() -> Vec<u8> {
    let piece = |name: &str, parent: Option<usize>, offset: [i32; 3], children: Vec<usize>| Piece { name: name.into(), parent, offset: offset.map(|v| v << 16), vertices: Vec::new(), primitives: Vec::new(), selection_primitive: None, children };
    Model3do { pieces: vec![piece("base", None, [0; 3], vec![1]), piece("pad", Some(0), [0, 20, -16], Vec::new())] }.to_bytes()
}

const MOVER: &str = "CanMove=1; BMcode=1; Acceleration=0.5; BrakeRate=1; TurnRate=2000; FootprintX=2; FootprintZ=2;";

fn field() -> Sim {
    let mut data = data_of(&[
        unit("FATBOY", &format!("{MOVER} MaxVelocity=1; FootprintX=4; FootprintZ=4; Builder=1; WorkerTime=300; MobileFactory=1; BuildPiece=pad;")),
        unit("TANK", &format!("{MOVER} MaxVelocity=2; BuildTime=600; BuildCostMetal=60; BuildCostEnergy=600;")),
        unit("PLANE", "CanFly=1; CanMove=1; MaxVelocity=5; Acceleration=1; BrakeRate=1; TurnRate=2048; CruiseAlt=80; FootprintX=1; FootprintZ=1; BuildTime=300;"),
        unit("TOWER", "FootprintX=2; FootprintZ=2; BuildTime=300;"),
    ]);
    let fatboy = data.unit_id("FATBOY").unwrap();
    data.units[fatboy].build_options = vec!["TANK".into(), "PLANE".into(), "TOWER".into()];
    let data = Arc::new(data);
    let models = HashMap::from([("objects3d/fatboy.3do".to_string(), model())]);
    let types = Arc::new(UnitTypes::compile(&data, &|p| models.get(p).cloned()));
    let cells = 128;
    let map = TntMap { width: cells as u32, height: cells as u32, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; cells * cells], features: vec![FEATURE_NONE; cells * cells], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    let setups: Vec<PlayerSetup> = (0..2).map(|i| PlayerSetup { side: "ARM".into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
    let mut sim = Sim::new(types, data, &map, &MapInfo::default(), 0, &setups, 7);
    for p in &mut sim.players {
        p.endless = true;
    }
    sim.cheats_allowed = true;
    sim
}

fn put(sim: &mut Sim, name: &str, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), 0, FxVec3::from_ints(x, 60, z), 0, true).unwrap()
}

fn produce(sim: &mut Sim, factory: UnitId, name: &str) {
    let type_id = sim.types.id(name).unwrap();
    sim.step(&[Command::Produce { player: 0, factory, type_id, count: 1 }]);
}

#[test]
fn a_factory_that_moves_builds_on_itself_as_it_goes_and_sets_its_unit_down_behind_it() {
    let mut sim = field();
    let fatboy = put(&mut sim, "FATBOY", 400, 600);
    let t = sim.unit_type(sim.unit(fatboy).unwrap()).clone();
    assert!(t.is_factory && t.mobile_factory, "a factory that moves");
    assert_eq!(t.build_piece, t.model.piece_index("pad"));
    sim.step(&[Command::Order { player: 0, units: vec![fatboy], order: Order::Move(FxVec3::from_ints(1600, 0, 600)), queue: false }]);
    produce(&mut sim, fatboy, "TANK");
    let start = sim.unit(fatboy).unwrap().pos;
    let tank = sim.unit(fatboy).unwrap().build.expect("building").target;
    for _ in 0..30 {
        sim.step(&[]);
        let (f, p) = (sim.unit(fatboy).unwrap(), sim.unit(tank).unwrap());
        assert_eq!((p.carried_by, p.factory), (Some(fatboy), Some(fatboy)), "built on it");
        assert_eq!(p.pos, piece_world_position(f, &t, f.cob.as_ref(), t.build_piece.unwrap()), "at its build piece");
    }
    assert!(sim.unit(tank).unwrap().build_progress > Fx::from_ratio(1, 3), "built on as it goes");
    assert!(!sim.unit(fatboy).unwrap().pos.within_xz(start, Fx::from_int(20)), "the factory on its way");
    for _ in 0..30 * 3 {
        if sim.unit(tank).unwrap().carried_by.is_none() {
            break;
        }
        sim.step(&[]);
    }
    let (f, p) = (sim.unit(fatboy).unwrap(), sim.unit(tank).unwrap());
    assert!(p.is_complete() && p.carried_by.is_none() && p.factory.is_none(), "done and let go");
    assert_eq!(p.pos.y, Fx::from_int(60), "on the ground");
    assert!(p.pos.x < f.pos.x - Fx::from_int(32), "set down behind it, as it goes east: {:?} {:?}", p.pos, f.pos);
    assert!(f.factory_queue.is_empty() && f.build.is_none());
}

#[test]
fn an_aircraft_takes_off_from_it() {
    let mut sim = field();
    let fatboy = put(&mut sim, "FATBOY", 600, 600);
    produce(&mut sim, fatboy, "PLANE");
    let plane = sim.unit(fatboy).unwrap().build.expect("building").target;
    for _ in 0..30 * 3 {
        if sim.unit(plane).unwrap().carried_by.is_none() {
            break;
        }
        sim.step(&[]);
    }
    let p = sim.unit(plane).unwrap();
    assert!(p.is_complete() && p.carried_by.is_none(), "let go");
    assert!(!p.landed && p.pos.y > Fx::from_int(100), "in flight: {:?}", p.pos);
    // It flies clear of the factory (made inside a carrier's hull, it does not stay there).
    for _ in 0..30 * 5 {
        sim.step(&[]);
    }
    let (f, p) = (sim.unit(fatboy).unwrap(), sim.unit(plane).unwrap());
    let reach = sim.unit_type(f).radius;
    assert!(!p.pos.within_xz(f.pos, reach), "clear of it: {:?} {:?}", p.pos, f.pos);
}

#[test]
fn it_makes_its_units_from_its_queue_and_no_building_from_it() {
    let mut sim = field();
    let fatboy = put(&mut sim, "FATBOY", 600, 600);
    let tank = sim.types.id("TANK").unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![fatboy], order: Order::Build { type_id: tank, pos: FxVec3::from_ints(700, 60, 700) }, queue: false }]);
    assert!(sim.unit(fatboy).unwrap().orders.is_empty(), "no unit of its menu built in the field");
    let units = sim.unit_count();
    produce(&mut sim, fatboy, "TOWER");
    sim.step(&[]);
    assert!(sim.unit(fatboy).unwrap().factory_queue.is_empty() && sim.unit_count() == units, "a building left out of its queue");
}

#[test]
fn its_unit_dies_with_it() {
    let mut sim = field();
    let fatboy = put(&mut sim, "FATBOY", 600, 600);
    produce(&mut sim, fatboy, "TANK");
    let tank = sim.unit(fatboy).unwrap().build.unwrap().target;
    sim.step(&[]);
    sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![fatboy]) }]);
    for _ in 0..3 {
        sim.step(&[]);
    }
    assert!(sim.unit(tank).is_none_or(|u| u.dead));
}

#[test]
fn factories_that_move_play_the_same_on_every_computer() {
    let play = || {
        let mut sim = field();
        let fatboy = put(&mut sim, "FATBOY", 400, 600);
        sim.step(&[Command::Order { player: 0, units: vec![fatboy], order: Order::Move(FxVec3::from_ints(1600, 0, 900)), queue: false }]);
        produce(&mut sim, fatboy, "TANK");
        produce(&mut sim, fatboy, "PLANE");
        (0..30 * 8).map(|_| {
            sim.step(&[]);
            sim.checksum()
        }).collect::<Vec<u64>>()
    };
    assert_eq!(play(), play());
}
