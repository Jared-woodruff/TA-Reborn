//! Units don't waste shots on targets the ground hides: automatic targeting passes over an
//! enemy behind a hill and takes one in the open (skipped without a TA install).

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, SimEvent, UnitTypes};

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
    Some(Sim::new(types, data, &map, &info, 0, &players, 99))
}

/// Two points `len` px apart east-west on low, level ground with a hill between them that
/// rises `rise` px above both.
fn across_a_hill(sim: &Sim, len: i64, rise: i64) -> Option<(FxVec3, FxVec3)> {
    let (w, h) = sim.world.size_px();
    let sea = sim.world.sea_level;
    let ht = |x: i64, z: i64| sim.world.height_at(Fx::from_int(x), Fx::from_int(z));
    let level = |x: i64, z: i64| (-24..=24).step_by(8).all(|d| (ht(x + d, z) - ht(x, z)).abs() <= Fx::from_int(3) && (ht(x, z + d) - ht(x, z)).abs() <= Fx::from_int(3));
    for z in (96..h.to_int() - 96).step_by(32) {
        for x in (96..w.to_int() - len - 96).step_by(16) {
            let (a, b) = (ht(x, z), ht(x + len, z));
            if a <= sea || b <= sea || (a - b).abs() > Fx::from_int(8) {
                continue;
            }
            let top = (x + 48..x + len - 48).step_by(8).map(|xx| ht(xx, z)).max()?;
            if top > a.max(b) + Fx::from_int(rise) && level(x, z) && level(x + len, z) {
                return Some((FxVec3::from_ints(x, 0, z), FxVec3::from_ints(x + len, 0, z)));
            }
        }
    }
    None
}

#[test]
fn shots_the_ground_would_stop_are_held_back() {
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let (a, b) = across_a_hill(&sim, 220, 40).expect("a hill with low ground both sides");
    let (tank_t, foe_t) = (sim.types.id("ARMSTUMP").unwrap(), sim.types.id("CORAK").unwrap());
    let tank = sim.spawn_unit(tank_t, 0, a, 16384, true).unwrap();
    let hidden = sim.spawn_unit(foe_t, 1, b, -16384, true).unwrap();
    // Both hold their ground (neither goes round the hill).
    sim.step(&[
        Command::Order { player: 0, units: vec![tank], order: Order::Stop, queue: false },
        Command::Order { player: 1, units: vec![hidden], order: Order::Stop, queue: false },
        Command::SetStates { player: 0, units: vec![tank], fire: None, movement: Some(0) },
        Command::SetStates { player: 1, units: vec![hidden], fire: Some(0), movement: Some(0) },
    ]);
    let shots = |sim: &mut Sim, ticks: u32| {
        let mut n = 0;
        for _ in 0..ticks {
            sim.step(&[]);
            n += sim.take_events().iter().filter(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == tank)).count();
        }
        n
    };
    let blind = shots(&mut sim, 30 * 8);
    println!("tank at ({}, {}), enemy behind the hill at ({}, {}): {blind} shots", a.x.to_int(), a.z.to_int(), b.x.to_int(), b.z.to_int());
    assert_eq!(blind, 0, "no shots into the hill");
    // An enemy in the open on the tank's side: that one is taken on.
    let open = sim.spawn_unit(foe_t, 1, a + FxVec3::from_ints(0, 0, 150), 0, true).unwrap();
    sim.step(&[Command::SetStates { player: 1, units: vec![open], fire: Some(0), movement: Some(0) }]);
    let seen = shots(&mut sim, 30 * 8);
    println!("with an enemy in the open: {seen} shots");
    assert!(seen > 0, "the enemy in the open should be fired at");
}
