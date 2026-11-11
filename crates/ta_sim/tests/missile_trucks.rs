//! Missile and rocket trucks launch and hit (against the real game data; skipped without a TA
//! install).

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, SimEvent, Target, UnitTypes};

fn sim_on(map_name: &str) -> Option<Sim> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map_name}.ota")).ok()?));
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup {
            side: if i == 0 { "ARM".into() } else { "CORE".into() },
            color: i as u8,
            start: FxVec3::ZERO,
            metal: Fx::from_int(1000),
            energy: Fx::from_int(1000),
            ai: false,
        })
        .collect();
    Some(Sim::new(types, data, &map, &info, 0, &players, 99))
}

/// Dry, nearly flat ground: an east-west strip varying by at most 3 px.
fn flat_spot(sim: &Sim, len: i64) -> Option<FxVec3> {
    let (w, h) = sim.world.size_px();
    let sea = sim.world.sea_level;
    for z in (64..h.to_int() - 64).step_by(32) {
        for x in (64..w.to_int() - len - 64).step_by(32) {
            let hs: Vec<Fx> = (0..=len).step_by(16).map(|dx| sim.world.height_at(Fx::from_int(x + dx), Fx::from_int(z))).collect();
            let (lo, hi) = (hs.iter().min()?, hs.iter().max()?);
            if *lo > sea && *hi - *lo <= Fx::from_int(3) {
                return Some(FxVec3::from_ints(x + 20, 0, z));
            }
        }
    }
    None
}

/// `launcher` (player 0's or 1's) and an enemy Raider or Stumpy `range` px east of it, on the
/// launcher's own (or told to attack it): the shots fired and the damage done in `seconds`.
fn duel(sim: &mut Sim, at: FxVec3, launcher: &str, range: i64, order: bool, seconds: u32) -> (u32, f32) {
    let side = if launcher.starts_with("ARM") { 0 } else { 1 };
    let enemy = if side == 0 { "CORRAID" } else { "ARMSTUMP" };
    let l = sim.spawn_unit(sim.types.id(launcher).unwrap(), side, at, -16384, true).unwrap();
    let e = sim.spawn_unit(sim.types.id(enemy).unwrap(), 1 - side, at + FxVec3::from_ints(range, 0, 0), 16384, true).unwrap();
    if order {
        sim.step(&[Command::Order { player: side, units: vec![l], order: Order::Attack(Target::Unit(e)), queue: false }]);
    }
    let before = sim.unit(e).unwrap().health.to_f32();
    let mut shots = 0;
    for _ in 0..30 * seconds {
        sim.step(&[]);
        shots += sim.take_events().iter().filter(|ev| matches!(ev, SimEvent::WeaponFired { unit, .. } if *unit == l)).count() as u32;
    }
    let damage = before - sim.unit(e).map_or(0.0, |u| u.health.to_f32());
    for id in [l, e] {
        sim.remove_unit(id);
    }
    sim.step(&[]);
    sim.take_events();
    (shots, damage)
}

/// The missile trucks (the Arm's Samson, the Core's Slasher) launch at enemies they see across
/// their range when told to. On their own they take a tank on too with no aircraft about: like
/// TA's other anti-air units they list NOTAIR among their bad targets (`BadTargetCategory`),
/// which they take on last. (Out of their own short sight they need a friend's, or radar, as in
/// TA: everything is seen here.)
#[test]
fn missile_trucks_launch_and_hit() {
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 420).expect("no flat land on the map");
    sim.los_enabled = false;
    for truck in ["ARMSAM", "CORMIST"] {
        let (shots, damage) = duel(&mut sim, a, truck, 300, false, 10);
        assert!(shots >= 3 && damage > 0.0, "{truck} on its own at a tank 300 px off, nothing better near: {shots} shots, {damage} damage");
        for range in [200, 400, 560] {
            let (shots, damage) = duel(&mut sim, a, truck, range, true, 10);
            assert!(shots >= 3 && damage > 0.0, "{truck} told to, at {range} px: {shots} shots, {damage} damage");
        }
    }
}

/// Rocket trucks' rockets (the Merl's and the Diplomat's, launched straight up) climb as they
/// gather speed and come down on their target. At full speed through the whole climb they rose
/// 2,000 px, out of sight, and their fuel ran out before they came down: no harm done.
#[test]
fn vertically_launched_rockets_come_down_on_their_target() {
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 420).expect("no flat land on the map");
    sim.los_enabled = false;
    for truck in ["ARMMERL", "CORVROC"] {
        if sim.types.id(truck).is_none() {
            eprintln!("{truck} not installed (The Core Contingency); skipping it");
            continue;
        }
        for (range, order) in [(400, false), (700, true)] {
            let (shots, damage) = duel(&mut sim, a, truck, range, order, 20);
            assert!(shots >= 1 && damage > 0.0, "{truck} at {range} px: {shots} rockets, {damage} damage");
        }
    }
}
