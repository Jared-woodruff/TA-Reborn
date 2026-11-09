//! Every skirmish map with two AIs for a minute (skipped without a TA install; run with
//! `cargo test --release -p ta_ai --test every_map -- --ignored --nocapture`). Catches maps
//! that fail to load or make the simulation or the AI panic, and reports what each AI got
//! going in that minute. `TA_TEST_SECONDS` sets the length.

use std::sync::Arc;
use std::time::Instant;

use ta_ai::{Difficulty, SkirmishAi};
use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Fx, FxVec3, PlayerSetup, Sim, UnitTypes};

#[test]
#[ignore]
fn every_map_plays_a_minute() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    // (TA Reborn's own maps too.)
    vfs.mount_builtin();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let seconds: u64 = std::env::var("TA_TEST_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    let mut maps: Vec<String> = vfs.list("maps", Some("ota"), false).into_iter().filter_map(|p| p.strip_prefix("maps/").and_then(|f| f.strip_suffix(".ota")).map(str::to_string)).collect();
    maps.sort();
    maps.dedup();
    let (mut played, mut skipped, mut failed) = (0, Vec::new(), Vec::new());
    let started = Instant::now();
    for name in &maps {
        let (Some(tnt), Some(ota)) = (vfs.read_opt(&format!("maps/{name}.tnt")), vfs.read_opt(&format!("maps/{name}.ota"))) else {
            skipped.push(format!("{name} (no .tnt)"));
            continue;
        };
        let info = MapInfo::from_ota(&tdf::parse_bytes(&ota));
        let Some(starts) = info.schemas.first().map(|s| s.start_positions.clone()).filter(|s| s.len() >= 2) else {
            skipped.push(format!("{name} (fewer than 2 starts)"));
            continue;
        };
        let t0 = Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let map = TntMap::parse(&tnt).map_err(|e| format!("tnt: {e}"))?;
            let players: Vec<PlayerSetup> = (0..2)
                .map(|i| PlayerSetup {
                    side: ["ARM", "CORE"][i].into(),
                    color: i as u8,
                    start: FxVec3::new(Fx::from_f32(starts[i].1), Fx::ZERO, Fx::from_f32(starts[i].2)),
                    metal: Fx::from_int(1000),
                    energy: Fx::from_int(1000),
                    ai: true,
                })
                .collect();
            let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 11);
            for (i, p) in players.iter().enumerate() {
                sim.spawn_unit(types.id(["ARMCOM", "CORCOM"][i]).unwrap(), i as u8, p.start, 0, true);
            }
            let mut ais: Vec<SkirmishAi> = (0..2).map(|i| SkirmishAi::new(i as u8, &sim, Difficulty::Normal)).collect();
            for _ in 0..30 * seconds {
                let mut commands = Vec::new();
                for ai in &mut ais {
                    commands.extend(ai.think(&sim));
                }
                sim.step(&commands);
                sim.take_events();
            }
            Ok::<_, String>([sim.players[0].unit_count, sim.players[1].unit_count])
        }));
        let ms = t0.elapsed().as_millis();
        match result {
            Ok(Ok(units)) => {
                played += 1;
                println!("{name:40} units {:>3} / {:>3}  {ms:>6} ms", units[0], units[1]);
            }
            Ok(Err(e)) => failed.push(format!("{name}: {e}")),
            Err(p) => {
                let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                failed.push(format!("{name}: panic {msg}"));
            }
        }
    }
    println!("{played} maps played in {:.0} s; {} skipped (campaign maps with one start, or no .tnt)", started.elapsed().as_secs_f64(), skipped.len());
    assert!(failed.is_empty(), "maps that failed: {failed:#?}");
}
