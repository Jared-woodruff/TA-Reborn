//! The simulation's and the computer players' memory over a long battle (skipped without a TA
//! install), after a player's report of 2026-11-11: "I made the radar plane and used it to spy
//! the enemy CPU... it used a ton of RAM and kept increasing".
//!
//! Two computer players fight on Sherwood (`TA_TEST_MAP`) under a low unit limit
//! (`TA_TEST_UNIT_LIMIT`, 90), so that after the first minutes the battle is as big as it gets:
//! units lost and built again, never more of them. From then on player 0's radar planes patrol
//! over player 1's start and back, sent off again when idle and replaced when shot down, for
//! `TA_TEST_MINUTES` (14) minutes. Through that stretch the memory must stay as it was: what is
//! kept of units, shots and sightings gone must be let go. `TA_TEST_LOG=1` prints the memory
//! each minute.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use ta_ai::{Difficulty, SkirmishAi};
use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitId, UnitTypes};

/// The system's allocator, counting the bytes allocated and not yet freed.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call goes on to the system's allocator as it came; only a count is kept.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as the caller's.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as the caller's.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        // SAFETY: as the caller's.
        unsafe { System.dealloc(p, layout) };
        LIVE.fetch_sub(layout.size(), Relaxed);
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: as the caller's.
        let q = unsafe { System.realloc(p, layout, new_size) };
        if !q.is_null() {
            LIVE.fetch_add(new_size, Relaxed);
            LIVE.fetch_sub(layout.size(), Relaxed);
        }
        q
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

fn env_number<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

fn mb(bytes: usize) -> f64 {
    bytes as f64 / (1 << 20) as f64
}

#[test]
fn a_long_battle_with_radar_planes_over_the_enemy_keeps_to_its_memory() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mounted = ta_vfs::mods::mount(Some(&install.root), ta_vfs::builtin_dir().as_deref(), &[], &|m| m.manifest.id.clone());
    mounted.install.unwrap();
    let vfs = mounted.vfs;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map_name = std::env::var("TA_TEST_MAP").unwrap_or_else(|_| "Sherwood".into());
    let (Some(tnt), Some(ota)) = (vfs.read_opt(&format!("maps/{map_name}.tnt")), vfs.read_opt(&format!("maps/{map_name}.ota"))) else {
        panic!("map '{map_name}' not found");
    };
    let map = TntMap::parse(&tnt).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&ota));
    let starts = &info.schemas[0].start_positions;
    let sides = [("ARM", "ARMCOM"), ("CORE", "CORCOM")];
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup {
            side: sides[i].0.into(),
            color: i as u8,
            start: FxVec3::new(Fx::from_f32(starts[i].1), Fx::ZERO, Fx::from_f32(starts[i].2)),
            metal: Fx::from_int(1000),
            energy: Fx::from_int(1000),
            ai: true,
        })
        .collect();
    let mut sim = Sim::new(types.clone(), data, &map, &info, 0, &players, 7);
    sim.unit_limit = env_number("TA_TEST_UNIT_LIMIT", 90u32);
    for (i, p) in players.iter().enumerate() {
        sim.spawn_unit(types.id(sides[i].1).expect("the side's commander"), i as u8, p.start, 0, true);
    }
    let mut ais: Vec<SkirmishAi> = (0..2).map(|i| SkirmishAi::new(i as u8, &sim, Difficulty::Normal)).collect();
    let plane = types.id("ARMAWAC").expect("the Arm's radar plane (Core Contingency)");
    let (home, target) = (players[0].start, players[1].start);
    let log = std::env::var("TA_TEST_LOG").is_ok();

    // The battle grows to its limit.
    let warm_up = env_number("TA_TEST_WARM_UP", 14u64);
    let mut step = |sim: &mut Sim, ais: &mut [SkirmishAi], extra: Vec<Command>| {
        let mut commands = extra;
        for ai in ais.iter_mut() {
            commands.extend(ai.think(sim));
        }
        sim.step(&commands);
        sim.take_events();
    };
    for _ in 0..30 * 60 * warm_up {
        step(&mut sim, &mut ais, Vec::new());
    }
    let counts = |sim: &Sim| (0..2).map(|i| sim.units().filter(|u| !u.dead && u.owner == i).count()).collect::<Vec<_>>();
    println!("{map_name}: after {warm_up} minutes {:?} units (limit {}), {:.1} MB", counts(&sim), sim.unit_limit, mb(LIVE.load(Relaxed)));

    // Then radar planes over the enemy, for as long again.
    let minutes = env_number("TA_TEST_MINUTES", 14u64);
    let mut planes: Vec<UnitId> = Vec::new();
    let (mut sent, mut samples, mut contacts) = (0usize, Vec::new(), 0usize);
    for tick in 0..30 * 60 * minutes {
        let mut extra = Vec::new();
        if tick % 300 == 0 {
            planes.retain(|&id| sim.unit(id).is_some_and(|u| !u.dead && u.owner == 0));
            while planes.len() < 3 {
                let at = home + FxVec3::from_ints(64 * (sent % 5) as i64 - 128, 0, 96);
                let Some(id) = sim.spawn_airborne(plane, 0, at, 0, None) else { break };
                planes.push(id);
                sent += 1;
            }
            let idle: Vec<UnitId> = planes.iter().copied().filter(|&id| sim.unit(id).is_some_and(|u| u.orders.is_empty())).collect();
            if !idle.is_empty() {
                extra.push(Command::Order { player: 0, units: idle.clone(), order: Order::Patrol(target), queue: false });
                extra.push(Command::Order { player: 0, units: idle, order: Order::Patrol(home), queue: true });
            }
        }
        step(&mut sim, &mut ais, extra);
        if tick % 300 == 150 {
            // (What player 0's sensors show of the enemy: the planes are over its base.)
            contacts = contacts.max(sim.units().filter(|u| !u.dead && u.owner == 1 && sim.seen_by(u, 0) != ta_sim::Seen::Hidden).count());
        }
        if tick % 1800 == 1799 {
            let live = LIVE.load(Relaxed);
            samples.push(live);
            if log {
                println!("  minute {:>2}: {:.1} MB, units {:?}, radar planes sent {sent}", tick / 1800 + 1, mb(live), counts(&sim));
            }
        }
    }
    assert!(sent >= 3, "radar planes flew");
    assert!(contacts >= 20, "the radar planes showed the enemy's base ({contacts} of its units at most)");
    // (The first two minutes as the planes come in, against the last two.)
    let (first, last) = ((samples[0] + samples[1]) / 2, (samples[samples.len() - 2] + samples[samples.len() - 1]) / 2);
    let growth = mb(last) - mb(first);
    println!("{map_name}: {minutes} minutes with radar planes over the enemy ({sent} sent, {contacts} enemy units in sight or on radar at most): {:.1} MB to {:.1} MB ({growth:+.1} MB)", mb(first), mb(last));
    let allowed = 8.0 + mb(first) * 0.06;
    assert!(growth < allowed, "the battle's memory grew {growth:.1} MB in {minutes} minutes at its size limit (allowed {allowed:.1} MB): something is kept of what is gone");
}
