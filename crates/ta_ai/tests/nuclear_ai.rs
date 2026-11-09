//! Nuclear strikes on a computer player's Commander as it goes about its base, on real game
//! data (skipped without a TA install).
//!
//! `TA_TEST_MAP` picks the map (Painted Desert), `TA_TEST_FIRST` the minute of the first
//! strike (3), `TA_TEST_GROUND=1` aims at the ground where the Commander stands instead of at
//! the Commander, `TA_TEST_ANTI=1` gives the computer a Fortitude with two interceptors beside
//! its Commander as the first strike is ordered (those two print what happens, unjudged).

use std::sync::Arc;

use ta_ai::{Difficulty, SkirmishAi};
use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, SimEvent, Target, UnitTypes};

/// Our silo strikes at the computer's Commander, as it builds and repairs round its base among
/// its buildings, every 40 seconds: each missile comes down where the Commander stood as it was
/// launched (never after it, as TA's), on the Commander at its work there (the first leaves it
/// what TA's 2,900 to a Commander leave, or less, the buildings round it blowing up too), and the
/// second at the latest destroys it. Steered straight at it from the top of their climb, they
/// came in low and went off against the buildings in front of it, 100 to 300 px short.
#[test]
fn nukes_destroy_a_computer_players_commander_about_its_base() {
    let Ok(install) = ta_vfs::install::discover(None) else { eprintln!("TA install not found; skipping"); return };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map_name = std::env::var("TA_TEST_MAP").unwrap_or_else(|_| "Painted Desert".into());
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map_name}.ota")).unwrap()));
    let starts = &info.schemas[0].start_positions;
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: if i == 0 { "ARM".into() } else { "CORE".into() }, color: i as u8, start: FxVec3::new(Fx::from_f32(starts[i].1), Fx::ZERO, Fx::from_f32(starts[i].2)), metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: i == 1 })
        .collect();
    let mut sim = Sim::new(types.clone(), data, &map, &info, 0, &players, 7);
    // (Our silo's side sees the Commander it aims at.)
    sim.los_enabled = false;
    sim.spawn_unit(types.id("ARMCOM").unwrap(), 0, players[0].start, 0, true).unwrap();
    let core = sim.spawn_unit(types.id("CORCOM").unwrap(), 1, players[1].start, 0, true).unwrap();
    let silo_type = types.id("ARMSILO").unwrap();
    let silo_at = sim.find_build_site(silo_type, players[0].start + FxVec3::from_ints(200, 0, 0), 40).unwrap();
    let silo = sim.spawn_unit(silo_type, 0, silo_at, 0, true).unwrap();
    sim.grant_stockpile(silo, 10);
    // (Our side holds out however the computer attacks: the strikes are what is looked at.)
    sim.players[0].cheats.invulnerable = true;
    let mut ai = SkirmishAi::new(1, &sim, Difficulty::Normal);
    let by_ground = std::env::var("TA_TEST_GROUND").is_ok_and(|v| v == "1");
    let anti = std::env::var("TA_TEST_ANTI").is_ok_and(|v| v == "1");
    let first: u64 = 30 * 60 * std::env::var("TA_TEST_FIRST").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let weapon = sim.unit_type(sim.unit(silo).unwrap()).weapons[0].unwrap();
    let (mut ordered, mut blasts) = (0, Vec::new());
    for tick in 0..first + 30 * 60 * 6 {
        let mut commands = ai.think(&sim);
        if tick == first && anti {
            let t = types.id("CORFMD").unwrap();
            let c = sim.unit(core).unwrap().pos;
            if let Some(p) = sim.find_build_site(t, c + FxVec3::from_ints(0, 0, 300), 60) && let Some(fmd) = sim.spawn_unit(t, 1, p, 0, true) {
                sim.grant_stockpile(fmd, 2);
            }
        }
        if tick >= first && (tick - first).is_multiple_of(30 * 40) && ordered < 4 && let Some(c) = sim.unit(core).filter(|c| !c.dead) {
            println!("tick {tick}: strike {ordered} at the Commander ({} health, at {:?})", c.health.to_f32(), c.pos.to_f32());
            let target = if by_ground { Target::Ground(c.pos) } else { Target::Unit(core) };
            commands.push(Command::Order { player: 0, units: vec![silo], order: Order::Attack(target), queue: false });
            ordered += 1;
        }
        sim.step(&commands);
        for e in sim.take_events() {
            match e {
                SimEvent::Explosion { weapon: w, pos, .. } if w == weapon => {
                    let c = sim.unit(core).filter(|c| !c.dead);
                    let off = c.map(|c| c.pos.distance_xz(pos).to_f32());
                    println!("tick {tick}: the missile went off at {:?}, {off:?} px from the Commander, left {:?} health", pos.to_f32(), c.map(|c| c.health.to_f32()));
                    blasts.push(off);
                }
                SimEvent::Intercepted { pos, .. } => println!("tick {tick}: intercepted at {:?}", pos.to_f32()),
                _ => {}
            }
        }
        if sim.unit(core).is_none_or(|u| u.dead) {
            break;
        }
    }
    if anti || by_ground {
        return;
    }
    assert!(sim.unit(core).is_none_or(|u| u.dead), "two nukes destroy the computer's Commander: {blasts:?}");
    assert!(blasts.len() <= 2, "the second at the latest: {blasts:?}");
    for off in blasts.iter().flatten() {
        assert!(*off <= 40.0, "each comes down on the Commander: {blasts:?}");
    }
}
