//! A copy of a game (the simulation and its computer players, as the battle highlights keep
//! them) plays on exactly as the game itself: two AIs fight on real data, the game is copied
//! a few minutes in, and both play on with their own AIs' orders, their checksums compared
//! as they go. Skipped without a TA install. `TA_TEST_MAP` picks the map (Sherwood).

use std::sync::Arc;

use ta_ai::{Difficulty, SkirmishAi};
use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Fx, FxVec3, PlayerSetup, Sim, UnitTypes};

/// Counts the bytes allocated, to weigh a copy.
struct Counting;

static ALLOCATED: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size() as isize, std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        ALLOCATED.fetch_sub(layout.size() as isize, std::sync::atomic::Ordering::Relaxed);
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn step(sim: &mut Sim, ais: &mut [SkirmishAi]) {
    let mut commands = Vec::new();
    for ai in ais.iter_mut() {
        commands.extend(ai.think(sim));
    }
    sim.step(&commands);
    sim.take_events();
}

#[test]
fn a_copied_game_plays_on_exactly_as_the_game() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map_name = std::env::var("TA_TEST_MAP").unwrap_or_else(|_| "Sherwood".into());
    let map = TntMap::parse(&vfs.read_opt(&format!("maps/{map_name}.tnt")).expect("the map")).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read_opt(&format!("maps/{map_name}.ota")).expect("its OTA")));
    let starts = &info.schemas[0].start_positions;
    let sides = [("ARM", "ARMCOM"), ("CORE", "CORCOM")];
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup {
            side: sides[i].0.to_string(),
            color: i as u8,
            start: FxVec3::new(Fx::from_f32(starts[i].1), Fx::ZERO, Fx::from_f32(starts[i].2)),
            metal: Fx::from_int(1000),
            energy: Fx::from_int(1000),
            ai: true,
        })
        .collect();
    let mut sim = Sim::new(types.clone(), data.for_map(&vfs, &map_name), &map, &info, 0, &players, 11);
    // The rule "modern movement" (units give way, group orders as formations) unless
    // `TA_TEST_GIVE_WAY=0`: its state must copy too.
    sim.local_avoidance = std::env::var("TA_TEST_GIVE_WAY").map_or(true, |v| v != "0");
    sim.group_formations = sim.local_avoidance;
    for (i, p) in players.iter().enumerate() {
        sim.spawn_unit(types.id(sides[i].1).expect("the commander"), i as u8, p.start, 0, true);
    }
    let mut ais: Vec<SkirmishAi> = (0..2).map(|i| SkirmishAi::new(i as u8, &sim, Difficulty::Hard)).collect();
    // Eight minutes in: bases up, armies out and fighting.
    for _ in 0..30 * 60 * 8 {
        step(&mut sim, &mut ais);
    }
    let before = ALLOCATED.load(std::sync::atomic::Ordering::Relaxed);
    let started = std::time::Instant::now();
    let (mut copy, mut copy_ais) = (sim.clone(), ais.clone());
    let (bytes, took) = (ALLOCATED.load(std::sync::atomic::Ordering::Relaxed) - before, started.elapsed());
    println!("{map_name}: a copy is {:.1} MB, made in {:.2} ms (map {} x {} cells)", bytes as f64 / 1e6, took.as_secs_f64() * 1000.0, sim.world.width, sim.world.height);
    assert_eq!(copy.checksum(), sim.checksum());
    let units = sim.unit_count();
    for tick in 0..30 * 180 {
        step(&mut sim, &mut ais);
        step(&mut copy, &mut copy_ais);
        if tick % 30 == 0 {
            assert_eq!(copy.checksum(), sim.checksum(), "the copy went its own way {tick} ticks after the copy");
        }
    }
    assert_eq!(copy.checksum(), sim.checksum());
    println!("{map_name}: {units} units when copied, {} three minutes on, the copy identical", sim.unit_count());
}
