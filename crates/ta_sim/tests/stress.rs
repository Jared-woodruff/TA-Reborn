//! Large-battle stress tests (run with `--ignored --nocapture` in release).
//!
//! `five_thousand_unit_battle`: two armies (`TA_STRESS_UNITS` a side, 2500) march at each other
//! across a map (`TA_STRESS_MAP`, Painted Desert) and fight. `swarm_battle`: the game's swarm
//! battle (`--demo swarm`) without the game: two armies (`TA_STRESS_UNITS` a side, 1500) clash in
//! the middle of `TA_STRESS_MAP` (Highland Rift, TA Reborn's own, with 51,000 trees and rocks),
//! the second side a computer player's, with fog of war and units giving way as a skirmish has
//! them. `spread_move`: one army (`TA_STRESS_UNITS`, 5000) sent across the map as the game sends
//! a selection, each unit to its own place. All run `TA_STRESS_TICKS` ticks (a minute) and print
//! progress every `TA_STRESS_EVERY` ticks (ten seconds), the parts of ticks slower than
//! `TA_STRESS_SPIKES` ms, then the ticks' times (average and percentiles), their parts and the
//! final state's checksum.

use std::sync::Arc;
use std::time::Instant;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitTypes};

/// A setting from the environment, or its default.
fn setting<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// The game's data and a map (the install's, with TA Reborn's own mounted over it), or None
/// without an install.
fn load(map_name: &str) -> Option<(Arc<GameData>, Arc<UnitTypes>, TntMap, MapInfo)> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    vfs.mount_builtin();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map_name}.ota")).unwrap()));
    Some((data, types, map, info))
}

/// Plays the battle on (`step` takes one tick) and prints how long the ticks took, and where.
fn measure(sim: &mut Sim, mut step: impl FnMut(&mut Sim)) {
    let before = sim.stats.clone();
    let ticks: usize = setting("TA_STRESS_TICKS", 30 * 60);
    let every: usize = setting("TA_STRESS_EVERY", 300usize).max(1);
    // `TA_STRESS_SPIKES=<ms>`: what took the time in ticks slower than that.
    let spikes: f64 = setting("TA_STRESS_SPIKES", f64::MAX);
    let mut times = Vec::with_capacity(ticks);
    let started = Instant::now();
    for tick in 0..ticks {
        let last = sim.stats.clone();
        let t = Instant::now();
        step(sim);
        sim.take_events();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        times.push(ms);
        if tick % every == 0 {
            println!("t={:5.1}s units {:5} projectiles {:5} tick {:6.2} ms", tick as f64 / 30.0, sim.unit_count(), sim.projectiles.len(), ms);
        }
        if ms > spikes {
            let s = &sim.stats;
            println!(
                "  tick {tick}: {ms:.1} ms: paths {:.1} (flow fields {:.1}, routes {:.1}; {} searches, {} fields built, {} brought up to date), scripts {:.1}, collisions {:.1}, projectiles {:.1}, vision {:.1}, deaths {:.1}",
                (s.path_us - last.path_us) as f64 / 1e3,
                (s.flow_us - last.flow_us) as f64 / 1e3,
                (s.route_us - last.route_us) as f64 / 1e3,
                s.path_searches - last.path_searches,
                s.flow_builds - last.flow_builds,
                s.flow_repairs - last.flow_repairs,
                (s.scripts_ns - last.scripts_ns) as f64 / 1e6,
                (s.collisions_us - last.collisions_us) as f64 / 1e3,
                (s.projectiles_us - last.projectiles_us) as f64 / 1e3,
                (s.vision_us - last.vision_us) as f64 / 1e3,
                (s.deaths_us - last.deaths_us) as f64 / 1e3,
            );
        }
    }
    let avg = started.elapsed().as_secs_f64() * 1000.0 / ticks as f64;
    times.sort_by(f64::total_cmp);
    let at = |p: f64| times[((times.len() as f64 * p) as usize).min(times.len() - 1)];
    println!("average {avg:.2} ms/tick, p50 {:.2}, p95 {:.2}, p99 {:.2}, worst {:.2} ms", at(0.5), at(0.95), at(0.99), times[times.len() - 1]);
    // Where a tick goes, in ms (orders, movement and weapons sampled one tick in eight).
    let s = &sim.stats;
    let per = |now: u64, then: u64, scale: f64| (now - then) as f64 / scale / ticks as f64;
    println!(
        "per tick: orders {:.2} (paths {:.2}, flow fields {:.2}, routes off them {:.2}), motion {:.2}, weapons {:.2}, scripts {:.2}, collisions {:.2}, projectiles {:.2}, vision {:.2}, deaths {:.2}, economy {:.2} ms; {} path searches, {} flow fields built and {} brought up to date, {} routes off them",
        per(s.orders_ns, before.orders_ns, 1e6),
        per(s.path_us, before.path_us, 1e3),
        per(s.flow_us, before.flow_us, 1e3),
        per(s.route_us, before.route_us, 1e3),
        per(s.motion_ns, before.motion_ns, 1e6),
        per(s.weapons_ns, before.weapons_ns, 1e6),
        per(s.scripts_ns, before.scripts_ns, 1e6),
        per(s.collisions_us, before.collisions_us, 1e3),
        per(s.projectiles_us, before.projectiles_us, 1e3),
        per(s.vision_us, before.vision_us, 1e3),
        per(s.deaths_us, before.deaths_us, 1e3),
        per(s.economy_us, before.economy_us, 1e3),
        s.path_searches - before.path_searches,
        s.flow_builds - before.flow_builds,
        s.flow_repairs - before.flow_repairs,
        s.flow_routes - before.flow_routes,
    );
    // (The same orders on the same data end in the same state: a change to the simulation that
    // keeps its rules keeps this.)
    println!("final checksum {:016x}, {} units, {} features alive", sim.checksum(), sim.unit_count(), sim.features.iter().filter(|f| f.alive).count());
}

#[test]
#[ignore]
fn five_thousand_unit_battle() {
    let map_name = std::env::var("TA_STRESS_MAP").unwrap_or_else(|_| "Painted Desert".into());
    let Some((data, types, map, info)) = load(&map_name) else { return };
    let per_side: usize = setting("TA_STRESS_UNITS", 2500);
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let mut sim = Sim::new(types.clone(), data, &map, &info, 0, &players, 3);
    // `TA_STRESS_GIVE_WAY=1`: with units giving way.
    sim.local_avoidance = std::env::var("TA_STRESS_GIVE_WAY").is_ok_and(|v| v == "1");
    let (w, h) = (map.width_px() as i64, map.height_px() as i64);
    let rosters = [
        ["ARMSTUMP", "ARMFLASH", "ARMPW", "ARMROCK", "ARMHAM", "ARMSAM", "ARMJETH", "ARMBULL"],
        ["CORAK", "CORTHUD", "CORRAID", "CORGATOR", "CORLEVLR", "CORMIST", "CORSTORM", "CORREAP"],
    ];
    let t0 = Instant::now();
    let mut ids = [Vec::new(), Vec::new()];
    for side in 0..2 {
        let cols = (per_side as f64).sqrt().ceil() as i64;
        for i in 0..per_side as i64 {
            let (r, c) = (i / cols, i % cols);
            let x = if side == 0 { w / 8 + r * 28 } else { w * 7 / 8 - r * 28 };
            let z = h / 2 - cols * 14 + c * 28;
            let name = rosters[side][(i as usize) % 8];
            let t = types.id(name).unwrap();
            let pos = FxVec3::from_ints(x.clamp(32, w - 32), 0, z.clamp(32, h - 32));
            if let Some(id) = sim.spawn_unit(t, side as u8, pos, if side == 0 { 16384 } else { -16384 }, true) {
                ids[side].push(id);
            }
        }
    }
    println!("spawned {} units in {:.2}s on {map_name} ({}x{} px)", sim.unit_count(), t0.elapsed().as_secs_f64(), w, h);
    // Everyone attack-moves to the far side.
    let mut cmds = Vec::new();
    for (side, units) in ids.iter().enumerate() {
        let dest = FxVec3::from_ints(if side == 0 { w * 7 / 8 } else { w / 8 }, 0, h / 2);
        for chunk in units.chunks(64) {
            cmds.push(Command::Order { player: side as u8, units: chunk.to_vec(), order: Order::Fight(dest), queue: false });
        }
    }
    sim.step(&cmds);
    measure(&mut sim, |sim| sim.step(&[]));
}

/// The game's swarm battle (`--demo swarm` in a skirmish duel: a Commander each at the map's
/// first two starts, the second side's computer player at Normal), without the game around it.
#[test]
#[ignore]
fn swarm_battle() {
    let map_name = std::env::var("TA_STRESS_MAP").unwrap_or_else(|_| "Highland Rift".into());
    let Some((data, types, map, info)) = load(&map_name) else { return };
    let per_side: usize = setting("TA_STRESS_UNITS", 1500);
    let starts = &info.schemas[0].start_positions;
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup {
            side: ["ARM", "CORE"][i].into(),
            color: i as u8,
            start: FxVec3::new(Fx::from_f32(starts[i].1), Fx::ZERO, Fx::from_f32(starts[i].2)),
            metal: Fx::from_int(1000),
            energy: Fx::from_int(1000),
            ai: i == 1,
        })
        .collect();
    let mut sim = Sim::new(types.clone(), data, &map, &info, 0, &players, 0xC0FFEE);
    sim.commander_ends_game = true;
    sim.unit_limit = 1500;
    // `TA_STRESS_GIVE_WAY=0`: without units giving way.
    sim.local_avoidance = std::env::var("TA_STRESS_GIVE_WAY").map_or(true, |v| v == "1");
    for (i, p) in players.iter().enumerate() {
        sim.spawn_unit(types.id(["ARMCOM", "CORCOM"][i]).unwrap(), i as u8, p.start, 0, true);
    }
    let mut ai = ta_ai::SkirmishAi::new(1, &sim, ta_ai::Difficulty::Normal);
    // Lined up as the demo has them: 36 px apart, from 700 px either side of the middle.
    let rosters = [
        ["ARMSTUMP", "ARMFLASH", "ARMPW", "ARMHAM", "ARMROCK", "ARMJETH", "ARMSAM", "ARMBULL"],
        ["CORAK", "CORTHUD", "CORRAID", "CORGATOR", "CORSTORM", "CORCRASH", "CORLEVLR", "CORREAP"],
    ];
    let (cx, cz) = (map.width_px() as f32 / 2.0, map.height_px() as f32 / 2.0);
    let cols = ((per_side as f32).sqrt() * 0.8).ceil().max(1.0) as usize;
    let mut ids = [Vec::new(), Vec::new()];
    for (side, names) in rosters.iter().enumerate() {
        let sign = if side == 0 { -1.0 } else { 1.0 };
        for i in 0..per_side {
            let (row, col) = (i / cols, i % cols);
            let x = cx + sign * (700.0 + row as f32 * 36.0);
            let z = cz + (col as f32 - cols as f32 / 2.0) * 36.0;
            let p = FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z));
            ids[side].extend(sim.spawn_unit(types.id(names[i % names.len()]).unwrap(), side as u8, p, if side == 0 { 16384 } else { -16384 }, true));
        }
    }
    println!("{} + {} units on {map_name} ({} x {} px)", ids[0].len(), ids[1].len(), map.width_px(), map.height_px());
    let cmds: Vec<Command> = (0..2)
        .map(|side| {
            let dest = FxVec3::new(Fx::from_f32(if side == 0 { cx + 900.0 } else { cx - 900.0 }), Fx::ZERO, Fx::from_f32(cz));
            Command::Order { player: side as u8, units: ids[side].clone(), order: Order::Fight(dest), queue: false }
        })
        .collect();
    sim.step(&cmds);
    let mut ai_ms = 0.0;
    let mut ai_worst = 0.0f64;
    measure(&mut sim, |sim| {
        let t = Instant::now();
        let commands = ai.think(sim);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        ai_ms += ms;
        ai_worst = ai_worst.max(ms);
        sim.step(&commands);
    });
    println!("computer player: {:.2} ms a tick, {ai_worst:.2} at worst", ai_ms / setting("TA_STRESS_TICKS", 30.0 * 60.0));
    // What a copy of the game costs (the battle highlights keep a few).
    let t = Instant::now();
    let copy = (sim.clone(), ai.clone());
    println!("a copy of the game and its computer player: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    drop(copy);
}

/// One army (`TA_STRESS_UNITS`, 5000) sent across `TA_STRESS_MAP` (Highland Rift) as the game
/// sends a selection: each unit to its own place in a square round the point clicked, 40 px
/// apart. Prints the ticks' times and how long until every unit had its way planned.
#[test]
#[ignore]
fn spread_move() {
    let map_name = std::env::var("TA_STRESS_MAP").unwrap_or_else(|_| "Highland Rift".into());
    let Some((data, types, map, info)) = load(&map_name) else { return };
    let n: usize = setting("TA_STRESS_UNITS", 5000);
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let mut sim = Sim::new(types.clone(), data, &map, &info, 0, &players, 7);
    sim.local_avoidance = true;
    let names = ["ARMSTUMP", "ARMFLASH", "ARMPW", "ARMHAM", "ARMROCK", "ARMJETH", "ARMSAM", "ARMBULL"];
    let (w, h) = (map.width_px() as f32, map.height_px() as f32);
    // (In rows 36 px apart back from a fifth of the way across, as many as there is room for
    // behind.)
    let rows = ((w * 0.2 - 64.0) / 36.0).floor().max(1.0);
    let cols = ((n as f32).sqrt() * 0.8).max(n as f32 / rows).ceil().max(1.0) as usize;
    let mut ids = Vec::new();
    for i in 0..n {
        let (row, col) = (i / cols, i % cols);
        let p = FxVec3::new(Fx::from_f32(w * 0.2 - row as f32 * 36.0), Fx::ZERO, Fx::from_f32(h / 2.0 + (col as f32 - cols as f32 / 2.0) * 36.0));
        ids.extend(sim.spawn_unit(types.id(names[i % names.len()]).unwrap(), 0, p, 16384, true));
    }
    // As `Game::move_selection` gives the orders.
    let target = (w * 0.75, h / 2.0);
    let cols = (ids.len() as f32).sqrt().ceil().max(1.0) as usize;
    let rows = (ids.len() - 1) / cols;
    let mut cmds = Vec::new();
    let mut dests = Vec::new();
    for (i, &id) in ids.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        let x = target.0 + (c as f32 - (cols - 1) as f32 / 2.0) * 40.0;
        let z = target.1 + (r as f32 - rows as f32 / 2.0) * 40.0;
        let dest = FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z));
        dests.push(dest);
        cmds.push(Command::Order { player: 0, units: vec![id], order: Order::Move(dest), queue: false });
    }
    println!("{} units on {map_name} ({w} x {h} px) sent {:.0} px", ids.len(), target.0 - w * 0.2);
    let mut first = Some(cmds);
    let mut tick = 0;
    let mut planned_at = None;
    measure(&mut sim, |sim| {
        sim.step(&first.take().unwrap_or_default());
        tick += 1;
        if planned_at.is_none() {
            let waiting = ids.iter().zip(&dests).filter(|&(&id, &d)| sim.unit(id).is_some_and(|u| u.move_request != Some(d) && !u.orders.is_empty())).count();
            if waiting == 0 {
                planned_at = Some(tick);
            } else if tick % 30 == 0 {
                println!("  tick {tick}: {waiting} units still to plan their way");
            }
        }
    });
    let arrived = ids.iter().filter(|&&id| sim.unit(id).is_some_and(|u| u.orders.is_empty())).count();
    println!("every unit planned by tick {planned_at:?}; {arrived} of {} arrived", ids.len());
}

/// Crowd separation runs on all cores for big crowds; it must give exactly what a single
/// thread gives, tick for tick.
#[test]
fn parallel_crowds_match_sequential() {
    let Some((data, types, map, info)) = load("Painted Desert") else { return };
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let (w, h) = (map.width_px() as i64, map.height_px() as i64);
    let build = |parallel: bool| {
        let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 3);
        sim.parallel = parallel;
        // Two tight blocks of 900 tanks driving into each other.
        let mut cmds = Vec::new();
        for side in 0..2i64 {
            let t = types.id(["ARMSTUMP", "CORRAID"][side as usize]).unwrap();
            let mut ids = Vec::new();
            for i in 0..900i64 {
                let (r, c) = (i / 30, i % 30);
                let x = w / 2 + if side == 0 { -40 - r * 22 } else { 40 + r * 22 };
                let pos = FxVec3::from_ints(x, 0, h / 2 - 330 + c * 22);
                ids.extend(sim.spawn_unit(t, side as u8, pos, if side == 0 { 16384 } else { -16384 }, true));
            }
            let dest = FxVec3::from_ints(if side == 0 { w / 2 + 600 } else { w / 2 - 600 }, 0, h / 2);
            cmds.push(Command::Order { player: side as u8, units: ids, order: Order::Move(dest), queue: false });
        }
        sim.step(&cmds);
        sim
    };
    let (mut a, mut b) = (build(true), build(false));
    for tick in 0..90 {
        a.step(&[]);
        b.step(&[]);
        assert_eq!(a.checksum(), b.checksum(), "parallel and sequential separation diverged at tick {tick}");
    }
}
