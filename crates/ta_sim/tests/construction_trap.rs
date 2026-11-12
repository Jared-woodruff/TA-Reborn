//! Constructors are never walled in by what they build (skipped without a TA install). A block of
//! buildings that touch (the game's Alt-drag rectangle of solar collectors) left the Commander
//! inside it: laying a site it stood on, it stepped aside onto the site it built next, a walk
//! another building cut short left it a few pixels inside a frame, and from inside a building no
//! route leads anywhere, so it waited on its next site for ever. Units now step off a site clear
//! of the builder's next ones, and a unit standing in a building is put on the nearest free cell.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitId, UnitTypes};

fn sim_on(map_name: &str) -> Option<Sim> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map_name}.ota")).ok()?));
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: if i == 0 { "ARM".into() } else { "CORE".into() }, color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let mut sim = Sim::new(types, data, &map, &info, 0, &players, 99);
    sim.los_enabled = false;
    Some(sim)
}

/// Whether the unit, sent 400 px away from the block, gets there in 40 seconds.
fn walks_out(sim: &mut Sim, id: UnitId) -> bool {
    let out = FxVec3::from_ints(1300, 0, 1300);
    sim.step(&[Command::Order { player: 0, units: vec![id], order: Order::Move(out), queue: false }]);
    for _ in 0..30 * 40 {
        sim.step(&[]);
        sim.take_events();
    }
    sim.unit(id).is_some_and(|u| (u.pos - out).xz().length() <= Fx::from_int(64))
}

/// A Commander at `start` builds a 3 by 3 block of solar collectors (footprints touching) in
/// `order`: how many it built, and whether it can walk away afterwards.
fn block(start: (i64, i64), far_first: bool) -> (usize, bool) {
    let mut sim = sim_on("Painted Desert").expect("the install");
    let (com, solar) = (sim.types.id("ARMCOM").unwrap(), sim.types.id("ARMSOLAR").unwrap());
    let c = sim.spawn_unit(com, 0, FxVec3::from_ints(start.0, 0, start.1), 0, true).unwrap();
    // (Row after row from one corner, as the game laid an Alt-drag rectangle out.)
    let mut sites: Vec<FxVec3> = (0..9).map(|k| sim.snap_to_grid(solar, FxVec3::from_ints(1600 + 80 * (k % 3), 0, 1500 + 80 * (k / 3)))).collect();
    if far_first {
        let p = sim.unit(c).unwrap().pos;
        sites.sort_by_key(|s| std::cmp::Reverse((*s - p).xz().length_sq_raw()));
    }
    let orders: Vec<Command> = sites.iter().enumerate().map(|(i, &s)| Command::Order { player: 0, units: vec![c], order: Order::Build { type_id: solar, pos: s }, queue: i > 0 }).collect();
    sim.step(&orders);
    for _ in 0..30 * 300 {
        // (Stores kept full: the building, not the economy, is under test.)
        let e = &mut sim.players[0].economy;
        (e.metal, e.energy) = (e.metal_capacity, e.energy_capacity);
        sim.step(&[]);
        sim.take_events();
        if sim.unit(c).unwrap().orders.is_empty() {
            break;
        }
    }
    let built = sim.units().filter(|u| u.type_id == solar && u.is_complete()).count();
    (built, walks_out(&mut sim, c))
}

#[test]
fn a_constructor_is_never_walled_in_by_a_block_it_builds() {
    if sim_on("Painted Desert").is_none() {
        eprintln!("TA install not found; skipping");
        return;
    }
    for start in [(1500, 1580), (1680, 1420), (1880, 1580), (1680, 1760), (1520, 1420), (1850, 1750)] {
        for far_first in [false, true] {
            let (built, free) = block(start, far_first);
            println!("from {start:?}, {}: {built} of 9 built, {}", if far_first { "far side first" } else { "corner first" }, if free { "walked away" } else { "WALLED IN" });
            assert!(free, "the Commander from {start:?} ({}) was walled in", if far_first { "far side first" } else { "corner first" });
            assert_eq!(built, 9, "every site built from {start:?}");
        }
    }
}

/// A unit standing in a building, however it got there, is put on the nearest free cell and
/// can go on again.
#[test]
fn a_unit_standing_in_a_building_steps_out_of_it() {
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let (solar, tank) = (sim.types.id("ARMSOLAR").unwrap(), sim.types.id("ARMSTUMP").unwrap());
    let site = sim.snap_to_grid(solar, FxVec3::from_ints(1680, 0, 1580));
    sim.spawn_unit(solar, 0, site, 0, true).unwrap();
    let t = sim.spawn_unit(tank, 0, site, 0, true).unwrap();
    for _ in 0..30 {
        sim.step(&[]);
    }
    let p = sim.unit(t).unwrap().pos;
    let half = Fx::from_int(40);
    assert!((p.x - site.x).abs() >= half || (p.z - site.z).abs() >= half, "out of the collector's footprint: ({:.0}, {:.0})", p.x.to_f32(), p.z.to_f32());
    assert!(walks_out(&mut sim, t), "and it goes where it is sent");
}
