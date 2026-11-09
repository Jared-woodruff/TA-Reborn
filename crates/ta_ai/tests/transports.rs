//! Transports on maps where the ground army can't get at the enemy (skipped without a TA
//! install; run with `cargo test --release -p ta_ai --test transports -- --nocapture`):
//! air transports carry units across an archipelago of cliff-ringed islands, and with air
//! transports forbidden, hovercraft or ships do it across the sea. `TA_TEST_LOG=1` prints
//! the AI's debug log.

use std::sync::Arc;

use ta_ai::{Difficulty, SkirmishAi};
use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Fx, FxVec3, Locomotion, PlayerSetup, Sim, SimEvent, UnitTypes};

struct PrintLog;

impl log::Log for PrintLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target().starts_with("ta_ai")
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            println!("[{}] {}", r.level(), r.args());
        }
    }
    fn flush(&self) {}
}

/// Units set down by transports of each kind (air, surface) per player, over `minutes`
/// of two Normal AIs on `map` from random `seed`; `forbid` lists unit types not allowed.
fn lifts(map_name: &str, minutes: u64, seed: u64, forbid: &(dyn Fn(&ta_sim::UnitType) -> bool + Sync)) -> Option<[[u32; 2]; 2]> {
    if std::env::var("TA_TEST_LOG").is_ok() && log::set_logger(&PrintLog).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read_opt(&format!("maps/{map_name}.tnt"))?).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read_opt(&format!("maps/{map_name}.ota"))?));
    let starts = info.schemas[0].start_positions.clone();
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::new(Fx::from_f32(starts[i].1), Fx::ZERO, Fx::from_f32(starts[i].2)), metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: true })
        .collect();
    let mut sim = Sim::new(types.clone(), data, &map, &info, 0, &players, seed);
    sim.allowed = Some(types.types.iter().map(|t| !forbid(t)).collect());
    for (i, p) in players.iter().enumerate() {
        sim.spawn_unit(types.id(["ARMCOM", "CORCOM"][i]).unwrap(), i as u8, p.start, 0, true);
    }
    let mut ais: Vec<SkirmishAi> = (0..2).map(|i| SkirmishAi::new(i as u8, &sim, Difficulty::Normal)).collect();
    let mut set_down = [[0u32; 2]; 2];
    for _ in 0..30 * 60 * minutes {
        let mut commands = Vec::new();
        for ai in &mut ais {
            commands.extend(ai.think(&sim));
        }
        sim.step(&commands);
        for e in sim.take_events() {
            if let SimEvent::UnitUnloaded { transport, .. } = e && let Some(t) = sim.unit(transport) {
                let air = sim.unit_type(t).locomotion == Locomotion::Air;
                set_down[t.owner as usize][(!air) as usize] += 1;
            }
        }
    }
    println!("{map_name} (seed {seed}): units set down (air, surface) per player: {set_down:?}");
    Some(set_down)
}

/// `lifts` over a few seeds at once, summed (one game can go either way).
fn lifts_over_seeds(map_name: &str, minutes: u64, seeds: &[u64], forbid: &(dyn Fn(&ta_sim::UnitType) -> bool + Sync)) -> Option<[[u32; 2]; 2]> {
    let games: Vec<Option<[[u32; 2]; 2]>> = std::thread::scope(|scope| {
        let handles: Vec<_> = seeds.iter().map(|&seed| scope.spawn(move || lifts(map_name, minutes, seed, forbid))).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut sum = [[0u32; 2]; 2];
    for g in games {
        let g = g?;
        for p in 0..2 {
            for k in 0..2 {
                sum[p][k] += g[p][k];
            }
        }
    }
    Some(sum)
}

#[test]
fn air_transports_carry_units_between_islands() {
    // Biggie Biggs: islands ringed by cliffs; nothing but aircraft gets from one to another.
    let Some(set_down) = lifts("Biggie Biggs", 30, 7, &|_| false) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let air: u32 = set_down.iter().map(|p| p[0]).sum();
    assert!(air >= 4, "air transports should set units down across the water ({set_down:?})");
}

#[test]
fn surface_transports_when_air_transports_are_forbidden() {
    // Pillopeens: islands in a sea that hovercraft and ships cross. (Six games: in some, a
    // lift sets out late or is shot down; a small change anywhere in the computer players
    // decides which, and over three the count swung from 6 to 2 while twelve games landed
    // more than before. `TA_TEST_SEEDS=1,2,3,4`: other games.)
    let seeds: Vec<u64> = std::env::var("TA_TEST_SEEDS").ok().map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect()).unwrap_or_else(|| (1..=6).collect());
    let Some(set_down) = lifts_over_seeds("Pillopeens", 25, &seeds, &|t| t.transport_capacity > 0 && t.locomotion == Locomotion::Air) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let surface: u32 = set_down.iter().map(|p| p[1]).sum();
    assert!(set_down.iter().all(|p| p[0] == 0), "no air transports allowed");
    assert!(surface >= 8, "hovercraft or ships should carry units across ({set_down:?})");
}
