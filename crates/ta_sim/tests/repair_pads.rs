//! TA's aircraft and its repair pads (skipped without a TA install): every aircraft worn below
//! two-thirds of its health on patrol breaks off for a free Air Repair Pad, lands on it, is
//! mended there and takes up its patrol again; and the bombers that brake slowest, the Thunder
//! and the Hurricane, sent onto a pad (a right-click on it: a move to its middle), stop well
//! beyond it but come back to land on it and are mended.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::fixed::heading_of;
use ta_sim::{Command, Fx, FxVec3, Locomotion, Order, PlayerSetup, Sim, UnitId, UnitTypes};

fn installed() -> Option<(Arc<GameData>, Arc<UnitTypes>)> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    Some((data, types))
}

/// A dry, level field of 128 × 128 cells (2048 px square), an Air Repair Pad finished in its
/// middle.
fn field(data: &Arc<GameData>, types: &Arc<UnitTypes>) -> (Sim, UnitId) {
    let cells = 128 * 128;
    let map = TntMap { width: 128, height: 128, sea_level: 0, heights: vec![60; cells], features: vec![FEATURE_NONE; cells], tiles: Vec::new(), tile_map: Vec::new(), feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    let players = [PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::ZERO, energy: Fx::ZERO, ai: false }];
    let mut sim = Sim::new(types.clone(), data.clone(), &map, &MapInfo::default(), 0, &players, 73);
    sim.players[0].endless = true;
    sim.los_enabled = false;
    let pad = sim.spawn_unit(types.id("ARMASP").unwrap(), 0, FxVec3::from_ints(1024, 60, 1024), 0, true).unwrap();
    (sim, pad)
}

/// Whether `id` has landed on `pad`'s deck.
fn on_deck(sim: &Sim, id: UnitId, pad: UnitId) -> bool {
    sim.unit(id).is_some_and(|u| u.landed && u.air_pad.is_some_and(|(p, _)| p == pad))
}

fn seconds(tick: Option<u32>) -> String {
    tick.map_or("never".into(), |t| format!("{:.1} s", t as f32 / 30.0))
}

#[test]
fn every_aircraft_worn_on_patrol_is_mended_on_a_repair_pad_and_patrols_on() {
    let Some((data, types)) = installed() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let (mut checked, mut failed) = (0, Vec::new());
    for t in types.types.iter().filter(|t| t.locomotion == Locomotion::Air) {
        let (mut sim, pad) = field(&data, &types);
        let at = sim.unit(pad).unwrap().pos;
        // 600 px east of the pad, on patrol 500 px south and back, at half its health.
        let start = FxVec3::from_ints(1624, 0, 1024);
        let aircraft = sim.spawn_airborne(t.id, 0, start, 0, None).unwrap();
        sim.set_health(aircraft, t.max_health / 2);
        sim.step(&[Command::Order { player: 0, units: vec![aircraft], order: Order::Patrol(start + FxVec3::from_ints(0, 0, 500)), queue: false }]);
        let (mut trip, mut decked, mut mended) = (None, None, None);
        for tick in 0..30 * 120 {
            sim.step(&[]);
            let u = sim.unit(aircraft).unwrap();
            if trip.is_none() && u.repair_trip {
                trip = Some(tick);
            }
            if decked.is_none() && on_deck(&sim, aircraft, pad) {
                decked = Some(tick);
            }
            if u.health >= t.max_health {
                mended = Some(tick);
                break;
            }
        }
        // (Within ten seconds even the slowest to get going, the Valkyrie, is back on its route,
        // 600 px from the pad at its nearest.)
        let mut away = Fx::ZERO;
        for _ in 0..30 * 10 {
            sim.step(&[]);
            away = away.max((sim.unit(aircraft).unwrap().pos - at).length_xz());
        }
        let u = sim.unit(aircraft).unwrap();
        let patrols = !u.landed && !u.repair_trip && u.air_pad.is_none() && matches!(u.orders.front(), Some(Order::Patrol(_))) && away > Fx::from_int(500);
        println!(
            "{:9} off for the pad after {:>7}, on its deck after {:>7}, mended after {:>7}; patrolling again {patrols}, as far as {:>5.0} px from the pad",
            t.name,
            seconds(trip),
            seconds(decked),
            seconds(mended),
            away.to_f32()
        );
        if decked.is_none() || mended.is_none() || !patrols {
            failed.push(t.name.clone());
        }
        checked += 1;
    }
    println!("{checked} aircraft checked");
    assert!(checked >= 20, "checked {checked} original aircraft");
    assert!(failed.is_empty(), "not mended on the pad, or not back on patrol: {failed:?}");
}

#[test]
fn slow_braking_bombers_sent_onto_a_repair_pad_land_on_it_and_are_mended() {
    let Some((data, types)) = installed() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    for name in ["ARMTHUND", "CORHURC"] {
        let (mut sim, pad) = field(&data, &types);
        let at = sim.unit(pad).unwrap().pos;
        let kind = types.id(name).unwrap();
        let max = types.get(kind).max_health;
        // 700 px off, flying straight at it.
        let start = FxVec3::from_ints(1724, 0, 1024);
        let bomber = sim.spawn_airborne(kind, 0, start, heading_of(at - start), None).unwrap();
        sim.set_health(bomber, max / 2);
        sim.step(&[Command::Order { player: 0, units: vec![bomber], order: Order::Move(at), queue: false }]);
        let (mut beyond, mut decked, mut mended) = (Fx::ZERO, None, None);
        for tick in 0..30 * 90 {
            sim.step(&[]);
            let u = sim.unit(bomber).unwrap();
            // (Where it came to a stop with nothing more to do, before it took the pad.)
            if u.orders.is_empty() && u.air_pad.is_none() {
                beyond = beyond.max((u.pos - at).length_xz());
            }
            if decked.is_none() && on_deck(&sim, bomber, pad) {
                decked = Some(tick);
            }
            if u.health >= max {
                mended = Some(tick);
                break;
            }
        }
        println!("{name}: came to a stop {:.0} px past the pad's middle, on its deck after {}, mended after {}", beyond.to_f32(), seconds(decked), seconds(mended));
        assert!(beyond > Fx::from_int(56), "{name} stops beyond the reach an idle aircraft looks for a pad in: {beyond:?}");
        assert!(decked.is_some() && mended.is_some(), "{name} lands on the pad and is mended: {:?}", sim.unit(bomber).unwrap().pos);
    }
}
