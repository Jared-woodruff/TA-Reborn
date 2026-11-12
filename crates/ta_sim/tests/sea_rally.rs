//! TA's aircraft sent to a place over the sea, where they cannot set down (skipped without a TA
//! install): fighters and bombers circle there, as TA's do, and so do the scouts, construction
//! aircraft and transports, each round its own circle so that they keep apart, rather than hang
//! there on top of each other; the gunships hover.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Fx, FxVec3, Order, PlayerSetup, Sim, UnitId, UnitTypes};

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
    let mut sim = Sim::new(types, data, &map, &info, 0, &players, 7);
    sim.los_enabled = false;
    Some(sim)
}

#[test]
fn aircraft_sent_over_the_sea_circle_there_apart() {
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    // The open sea: of the points on a coarse grid over the map, the one whose shallowest water
    // within 600 px (where a bomber's circle reaches) is deepest.
    let (w, h) = sim.world.size_px();
    let mut spot = FxVec3::ZERO;
    let mut deepest = Fx::ZERO;
    for z in (640..h.to_int() - 640).step_by(128) {
        for x in (640..w.to_int() - 640).step_by(128) {
            let mut shallowest = sim.world.depth_at(Fx::from_int(x), Fx::from_int(z));
            for r in [300i64, 600] {
                for k in 0..16 {
                    let a = k as f64 * std::f64::consts::TAU / 16.0;
                    let (px, pz) = (x + (a.cos() * r as f64) as i64, z + (a.sin() * r as f64) as i64);
                    shallowest = shallowest.min(sim.world.depth_at(Fx::from_int(px), Fx::from_int(pz)));
                }
            }
            if shallowest > deepest {
                (deepest, spot) = (shallowest, FxVec3::from_ints(x, 0, z));
            }
        }
    }
    assert!(deepest > Fx::ZERO, "Coast To Coast has open sea: {deepest:?}");
    let kinds = ["ARMCA", "ARMCA", "ARMATLAS", "ARMATLAS", "ARMPEEP", "ARMFIG", "ARMTHUND", "ARMBRAWL"];
    let start = spot + FxVec3::from_ints(-400, 0, 0);
    let fleet: Vec<UnitId> = kinds.iter().map(|name| sim.spawn_airborne(sim.types.id(name).unwrap(), 0, start, 0, None).unwrap()).collect();
    for &id in &fleet {
        sim.give_orders(id, [Order::Move(spot)]);
    }
    for _ in 0..30 * 40 { sim.step(&[]); }
    let mut moved = vec![Fx::ZERO; fleet.len()];
    let mut last: Vec<FxVec3> = fleet.iter().map(|&id| sim.unit(id).unwrap().pos).collect();
    // (Ticks each pair of the scouts, builders and transports spends within 24 px of each other:
    // passing, a moment; stacked, all.)
    let hovering = &fleet[..5];
    let mut together = vec![0u32; 10];
    for _ in 0..30 * 20 {
        sim.step(&[]);
        for (k, &id) in fleet.iter().enumerate() {
            let u = sim.unit(id).unwrap();
            moved[k] += (u.pos - last[k]).length_xz();
            last[k] = u.pos;
            assert!(!u.landed, "{} in the air over the sea", kinds[k]);
        }
        let mut pair = 0;
        for (a, &p) in hovering.iter().enumerate() {
            for &q in &hovering[a + 1..] {
                together[pair] += u32::from((sim.unit(p).unwrap().pos - sim.unit(q).unwrap().pos).length_xz() < Fx::from_int(24));
                pair += 1;
            }
        }
    }
    for (k, &id) in fleet.iter().enumerate() {
        let u = sim.unit(id).unwrap();
        println!("{}: moved {:.0} px in 20 s, {:.0} px from the spot, circling {}", kinds[k], moved[k].to_f32(), (u.pos - spot).length_xz().to_f32(), u.orbit.is_some());
    }
    for k in 0..7 {
        assert!(moved[k] > Fx::from_int(300) && sim.unit(fleet[k]).unwrap().orbit.is_some(), "{} circles: {:?}", kinds[k], moved[k]);
    }
    for k in 0..5 {
        assert!((sim.unit(fleet[k]).unwrap().pos - spot).length_xz() < Fx::from_int(500), "{} round the spot it was sent to", kinds[k]);
    }
    assert!(together.iter().all(|&t| t < 60), "the scouts, builders and transports keep apart: ticks together {together:?}");
    assert!(moved[7] < Fx::from_int(40) && sim.unit(fleet[7]).unwrap().orbit.is_none(), "the gunship hovers: {:?}", moved[7]);
}
