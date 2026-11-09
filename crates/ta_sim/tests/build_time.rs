//! Construction counts whole units of work (exact for any build time): stock units build in
//! as many ticks as when progress was a 16.16 fraction a tick, give or take the one tick that
//! fraction's rounding cost. Skipped when no TA install is found.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitType, UnitTypes};

fn load() -> Option<(ta_vfs::Vfs, Arc<GameData>, Arc<UnitTypes>)> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    Some((vfs, data, types))
}

/// Ticks a build took at full pace when each tick added `worker_rate / build_time` (16.16,
/// rounded down) until the progress reached one; none when that step was zero.
fn ticks_before(builder: &UnitType, unit: &UnitType) -> Option<u64> {
    let step = (builder.worker_rate / unit.build_time).0 as u64;
    (step > 0).then(|| 65536u64.div_ceil(step))
}

/// Ticks a build takes at full pace now.
fn ticks_now(builder: &UnitType, unit: &UnitType) -> u64 {
    unit.build_work.div_ceil(builder.work_rate)
}

/// TA's own time: `BuildTime / WorkerTime` seconds, in whole ticks (the last one partly used).
fn ticks_exact(builder: &UnitType, unit: &UnitType) -> u64 {
    (unit.def.build_time * 30.0 / builder.def.worker_time).ceil() as u64
}

#[test]
fn stock_units_build_in_their_build_time() {
    let Some((_, _, types)) = load() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let by = |n: &str| types.by_name(n).unwrap_or_else(|| panic!("{n}"));
    for (builder, unit) in [("ARMVP", "ARMSTUMP"), ("ARMCOM", "ARMSOLAR"), ("CORGANT", "CORKROG"), ("CORCOM", "CORSOLAR"), ("ARMAVP", "ARMBULL")] {
        let (b, u) = (by(builder), by(unit));
        let (before, now) = (ticks_before(b, u), ticks_now(b, u));
        println!("{unit} by {builder}: {now} ticks (before {before:?}; build time {}, worker time {})", u.def.build_time, b.def.worker_time);
        assert_eq!(now, ticks_exact(b, u), "{unit} by {builder}");
    }
    // Every builder and everything it builds: exactly TA's time now; the 16.16 step's rounding
    // (always down) used to add up to a percent or so.
    let (mut pairs, mut worst, mut never) = (0, (0.0, String::new()), Vec::new());
    let mut slower: Vec<(f64, String)> = Vec::new();
    for b in types.types.iter().filter(|t| t.is_builder) {
        for &o in &b.build_options {
            let u = types.get(o);
            let now = ticks_now(b, u);
            pairs += 1;
            assert_eq!(now, ticks_exact(b, u), "{} by {}", u.name, b.name);
            let Some(before) = ticks_before(b, u) else {
                never.push(format!("{} by {}: {now} ticks", u.name, b.name));
                continue;
            };
            assert!(before + 1 >= now, "{} by {}: never slower than before", u.name, b.name);
            let cost = (before as f64 - now as f64) / now as f64;
            slower.push((cost, format!("{} by {}: {now} ticks, before {before}", u.name, b.name)));
            if cost > worst.0 {
                worst = (cost, format!("{} by {}: {now} ticks, before {before}", u.name, b.name));
            }
        }
    }
    slower.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (cost, what) in slower.iter().take(12) {
        println!("  {:.2}% {what}", cost * 100.0);
    }
    println!("{pairs} builder and unit pairs; the most the old rounding cost: {:.2}% ({}); never finished before: {never:?}", worst.0 * 100.0, worst.1);
    assert!(pairs > 500);
}

/// Every stock category token has a bit of its own (TA names 54, near the 64 a 64-bit mask
/// held: a mod's few more were ignored).
#[test]
fn stock_category_tokens_all_count() {
    let Some((_, data, types)) = load() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let tokens: std::collections::BTreeSet<String> = data.units.iter().flat_map(|u| u.category.split_whitespace().map(|t| t.to_ascii_uppercase())).collect();
    println!("{} distinct category tokens", tokens.len());
    assert!(tokens.len() <= ta_sim::defs::CategorySet::CAPACITY);
    for t in &tokens {
        assert!(!types.categories(t).is_empty(), "{t}");
    }
}

/// In the game (the Commander's arm, its nano stance, the economy) a solar collector takes
/// exactly its build time of work.
#[test]
fn a_commander_builds_a_solar_collector_in_its_build_time() {
    let Some((vfs, data, types)) = load() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let map = TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    let (_, x, z) = info.schemas[0].start_positions[0];
    let start = FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z));
    let players = vec![PlayerSetup { side: "ARM".into(), color: 0, start, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }];
    let mut sim = Sim::new(types.clone(), data, &map, &info, 0, &players, 1234);
    sim.set_starting_resources(0, Fx::from_int(100_000), Fx::from_int(100_000));
    let commander = sim.spawn_unit(types.id("ARMCOM").unwrap(), 0, start, 0, true).unwrap();
    let solar = types.id("ARMSOLAR").unwrap();
    let site = sim.find_build_site(solar, start + FxVec3::from_ints(96, 0, 0), 30).expect("a site");
    sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Build { type_id: solar, pos: site }, queue: false }]);
    let (mut working, mut done) = (0, false);
    for _ in 0..30 * 60 {
        let before = sim.units().find(|u| u.type_id == solar).map_or(0, |u| u.build_work);
        sim.step(&[]);
        let Some(u) = sim.units().find(|u| u.type_id == solar) else { continue };
        working += (u.build_work > before) as u64;
        if u.is_complete() {
            done = true;
            break;
        }
    }
    assert!(done, "the solar collector never finished");
    assert_eq!(working, ticks_now(types.get(types.id("ARMCOM").unwrap()), types.get(solar)));
}
