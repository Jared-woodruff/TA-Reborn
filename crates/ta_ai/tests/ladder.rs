//! AI ladder (skipped without a TA install; run with
//! `cargo test --release -p ta_ai --test ladder -- --ignored --nocapture`): one difficulty
//! against another on ten maps from both starts and on both sides, `TA_LADDER_MINUTES` long
//! (default 25), the games spread over every core. `TA_LADDER_A` / `TA_LADDER_B` pick the
//! levels (default hard / normal); `TA_LADDER_MAPS=all` plays every map with two starts,
//! `TA_LADDER_MAPS=sea` the water and island maps, and a comma-separated list of names those
//! maps. A game is won outright when the other side is defeated, else on points: what each
//! side owns (cost); `TA_LADDER_COMMANDER=1` plays with the Commander rule (the skirmish
//! default) instead of to the last unit. `TA_LADDER_SEEDS=n` plays every game under n seeds
//! (odd ones with B as player 0, so an even count is fairest).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ta_ai::{Difficulty, SkirmishAi};
use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Fx, FxVec3, Locomotion, PlayerSetup, Sim, SimEvent, UnitId, UnitTypes};

const MAPS: [&str; 10] = [
    "Sherwood",
    "Painted Desert",
    "Great Divide",
    "Gasplant Plain",
    "Rock Alley",
    "Yerrot Mountains",
    "Eastside Westside",
    "Full Moon",
    "The Cold Place",
    "Red River",
];

/// Water and island maps: navies, hovercraft, amphibians and transports decide these (on
/// Biggie Biggs, OOOOweeee and Slate Gordon nothing on the ground or the water gets from one
/// start to the other).
const SEA_MAPS: [&str; 10] = [
    "Anteer Strait",
    "Biggie Biggs",
    "Dire Straits",
    "East Indeez",
    "OOOOweeee",
    "Pillopeens",
    "Sail Away",
    "Seven Islands",
    "Slate Gordon",
    "The Bayou",
];

/// `TA_TEST_LOG`: the AIs' debug lines (best with `TA_LADDER_GAME`, one game at a time).
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

fn level(var: &str, default: Difficulty) -> Difficulty {
    match std::env::var(var).map(|v| v.to_ascii_lowercase()).as_deref() {
        Ok("easy") => Difficulty::Easy,
        Ok("normal") => Difficulty::Normal,
        Ok("hard") => Difficulty::Hard,
        _ => default,
    }
}

/// What a player owns, by cost (metal plus energy / 60), the Commander left out: ARM's
/// costs 6,300 more than CORE's, which would decide close games on points by side alone.
fn worth(sim: &Sim, player: u8) -> i64 {
    sim.units().filter(|u| u.owner == player && !u.dead && !sim.unit_type(u).def.is_commander).map(|u| {
        let t = sim.unit_type(u);
        (t.cost_metal + t.cost_energy / 60).to_int()
    }).sum()
}

struct Game {
    map: usize,
    game: usize,
    seed: u64,
    winner: usize,
    /// Minute the loser was defeated.
    decided: Option<u64>,
    worth: [i64; 2],
    metal: [i64; 2],
    kills: [u64; 2],
    lost: [u64; 2],
    air: Air,
    /// What each side still has at the end: buildings, ground and hover units, ships,
    /// aircraft (the Commander counted apart).
    left: [[u32; 5]; 2],
    /// Per side, by domain (buildings, ground and hover, ships, aircraft): worth owned at
    /// the end (the Commander left out, as in `worth`), and worth of the enemy units killed
    /// by that domain.
    owned: [[i64; 4]; 2],
    killed_by: [[i64; 4]; 2],
    /// Per side: worth of idle hovercraft and of idle other ground units at the end.
    idle: [[i64; 2]; 2],
}

/// 0 buildings, 1 ground and hover units, 2 ships, 3 aircraft.
fn domain(sim: &Sim, type_id: usize) -> usize {
    match sim.types.get(type_id).locomotion {
        Locomotion::Static => 0,
        Locomotion::Naval => 2,
        Locomotion::Air => 3,
        _ => 1,
    }
}

/// A side's units by kind: buildings, ground and hover units, ships, aircraft, Commander.
fn remnants(sim: &Sim, player: u8) -> [u32; 5] {
    let mut out = [0; 5];
    for u in sim.units().filter(|u| u.owner == player && !u.dead) {
        let t = sim.unit_type(u);
        let k = if t.def.is_commander {
            4
        } else {
            match t.locomotion {
                Locomotion::Static => 0,
                Locomotion::Naval => 2,
                Locomotion::Air => 3,
                _ => 1,
            }
        };
        out[k] += 1;
    }
    out
}

/// What each side's warplanes cost and did.
#[derive(Default, Clone, Copy)]
struct Air {
    /// Metal and energy / 60 in finished aircraft.
    spent: [i64; 2],
    lost: [u32; 2],
    /// Units finished off by our aircraft, and their worth.
    kills: [u32; 2],
    value: [i64; 2],
    strikes: [u32; 2],
    /// Strikes at the enemy Commander (the island air doctrine).
    commander_strikes: [u32; 2],
}

/// Armed aircraft (bombers, fighters, gunships), not transports or scouts.
fn warplane(sim: &Sim, type_id: usize) -> bool {
    let t = sim.types.get(type_id);
    t.locomotion == Locomotion::Air && t.transport_capacity == 0 && t.weapons.iter().any(|w| w.is_some())
}

/// A pair per player as a pair per side (A, B): swapped when B is player 0.
fn ab<T: Copy>(flip: bool, x: [T; 2]) -> [T; 2] {
    if flip { [x[1], x[0]] } else { x }
}

/// Cost of a unit type (metal plus energy / 60).
fn cost(sim: &Sim, type_id: usize) -> i64 {
    let t = sim.types.get(type_id);
    (t.cost_metal + t.cost_energy / 60).to_int()
}

#[test]
#[ignore]
fn ladder() {
    if std::env::var("TA_TEST_LOG").is_ok() && log::set_logger(&PrintLog).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
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
    let (a, b) = (level("TA_LADDER_A", Difficulty::Hard), level("TA_LADDER_B", Difficulty::Normal));
    let minutes: u64 = std::env::var("TA_LADDER_MINUTES").ok().and_then(|v| v.parse().ok()).unwrap_or(25);
    let names: Vec<String> = match std::env::var("TA_LADDER_MAPS").ok().as_deref().map(str::trim) {
        Some("all") => {
            let mut all: Vec<String> = vfs.list("maps", Some("ota"), false).into_iter().filter_map(|p| p.strip_prefix("maps/").and_then(|f| f.strip_suffix(".ota")).map(str::to_string)).collect();
            all.sort();
            all.dedup();
            all
        }
        Some("sea") => SEA_MAPS.iter().map(|m| m.to_string()).collect(),
        Some(list) if !list.is_empty() => list.split(',').map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect(),
        _ => MAPS.iter().map(|m| m.to_string()).collect(),
    };
    // Maps as bytes (each game parses its own copy on its thread).
    let maps: Vec<(String, Vec<u8>, Vec<u8>)> = names
        .into_iter()
        .filter_map(|name| {
            let (tnt, ota) = (vfs.read_opt(&format!("maps/{name}.tnt"))?, vfs.read_opt(&format!("maps/{name}.ota"))?);
            let info = MapInfo::from_ota(&tdf::parse_bytes(&ota));
            info.schemas.first().is_some_and(|s| s.start_positions.len() >= 2).then(|| (name, tnt.to_vec(), ota.to_vec()))
        })
        .collect();
    // `TA_LADDER_GAME=n` plays only the n-th game of each map (0: A as ARM at start 0, 1: at
    // start 1, 2 and 3: A as CORE).
    let only: Option<usize> = std::env::var("TA_LADDER_GAME").ok().and_then(|v| v.parse().ok());
    // `TA_LADDER_SEEDS=n`: every game under n random seeds (more games, less noise).
    let seeds: u64 = std::env::var("TA_LADDER_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(1).max(1);
    // `TA_LADDER_A_TWEAKS=micro=0,army_cap=120` (and `_B_`): parameters changed for one side.
    let tweaks = |var: &str| -> Vec<(String, i64)> {
        std::env::var(var)
            .unwrap_or_default()
            .split(',')
            .filter_map(|kv| kv.split_once('=').and_then(|(k, v)| Some((k.trim().to_string(), v.trim().parse().ok()?))))
            .collect()
    };
    let (tweaks_a, tweaks_b) = (tweaks("TA_LADDER_A_TWEAKS"), tweaks("TA_LADDER_B_TWEAKS"));
    let jobs: Vec<(usize, usize, u64)> = (0..maps.len())
        .flat_map(|m| (0..4).flat_map(move |g| (0..seeds).map(move |sd| (m, g, sd))))
        .filter(|&(_, g, _)| only.is_none_or(|o| o == g))
        .collect();
    let next = AtomicUsize::new(0);
    // Time spent thinking by the AIs, in microseconds.
    let ai_us = AtomicU64::new(0);
    let results = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(m, game, seed)) = jobs.get(k) else { break };
                    let (_, tnt, ota) = &maps[m];
                    let map = TntMap::parse(tnt).unwrap();
                    let info = MapInfo::from_ota(&tdf::parse_bytes(ota));
                    let starts = info.schemas[0].start_positions.clone();
                    // The games swap the starts and who plays ARM; odd seeds also who is player
                    // 0 (`flip`: B is), since going first is worth a little (units act in turn).
                    let (swap, core_a) = (game % 2, game / 2 == 1);
                    let flip = seed % 2 == 1;
                    // The side (0 for A, 1 for B) of player `p`.
                    let who = |p: usize| p ^ flip as usize;
                    let side = |j: usize| if (j == 0) != core_a { "ARM" } else { "CORE" };
                    let players: Vec<PlayerSetup> = (0..2)
                        .map(|p| {
                            let j = who(p);
                            let s = &starts[(j + swap) % 2];
                            PlayerSetup { side: side(j).into(), color: p as u8, start: FxVec3::new(Fx::from_f32(s.1), Fx::ZERO, Fx::from_f32(s.2)), metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: true }
                        })
                        .collect();
                    let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 7 + seed * 1000);
                    sim.unit_limit = 1500;
                    sim.local_avoidance = std::env::var("TA_TEST_GIVE_WAY").is_ok_and(|v| v == "1");
                    sim.group_formations = sim.local_avoidance;
                    // `TA_LADDER_COMMANDER=1`: the skirmish default, losing the Commander loses.
                    sim.commander_ends_game = std::env::var("TA_LADDER_COMMANDER").is_ok_and(|v| v == "1");
                    for (i, p) in players.iter().enumerate() {
                        let com = if side(who(i)) == "ARM" { "ARMCOM" } else { "CORCOM" };
                        sim.spawn_unit(types.id(com).unwrap(), i as u8, p.start, 0, true);
                    }
                    let levels = ab(flip, [a, b]);
                    let mut ais = [SkirmishAi::new(0, &sim, levels[0]), SkirmishAi::new(1, &sim, levels[1])];
                    for (ai, list) in ais.iter_mut().zip(ab(flip, [&tweaks_a, &tweaks_b])) {
                        for (k, v) in list {
                            assert!(ai.tweak(k, *v), "unknown AI parameter {k}");
                        }
                    }
                    let mut decided = None;
                    let mut thinking = std::time::Duration::ZERO;
                    // Every unit's side and type (for kills after it is gone).
                    let mut known: std::collections::HashMap<UnitId, (u8, usize)> = Default::default();
                    let mut air = Air::default();
                    let mut killed_by = [[0i64; 4]; 2];
                    for tick in 0..30 * 60 * minutes {
                        let mut commands = Vec::new();
                        let t0 = Instant::now();
                        for ai in &mut ais {
                            commands.extend(ai.think(&sim));
                        }
                        thinking += t0.elapsed();
                        sim.step(&commands);
                        for e in sim.take_events() {
                            match e {
                                SimEvent::UnitCreated { id, type_id } => {
                                    if let Some(u) = sim.unit(id) {
                                        known.insert(id, (u.owner, type_id));
                                    }
                                }
                                SimEvent::UnitCompleted { id, type_id } => {
                                    if let (Some(u), true) = (sim.unit(id), warplane(&sim, type_id)) {
                                        air.spent[u.owner as usize & 1] += cost(&sim, type_id);
                                    }
                                }
                                SimEvent::UnitDestroyed { id, type_id, killer, .. } => {
                                    if let Some(&(owner, _)) = known.get(&id) && warplane(&sim, type_id) {
                                        air.lost[owner as usize & 1] += 1;
                                    }
                                    if let (Some(&(k_owner, k_type)), Some(&(v_owner, _))) = (killer.and_then(|k| known.get(&k)), known.get(&id)) && k_owner != v_owner {
                                        killed_by[k_owner as usize & 1][domain(&sim, k_type)] += cost(&sim, type_id);
                                    }
                                    if let Some(&(k_owner, k_type)) = killer.and_then(|k| known.get(&k)) && warplane(&sim, k_type) && known.get(&id).is_some_and(|&(o, _)| o != k_owner) {
                                        air.kills[k_owner as usize & 1] += 1;
                                        air.value[k_owner as usize & 1] += cost(&sim, type_id);
                                    }
                                }
                                _ => {}
                            }
                        }
                        if tick % 300 == 0 && let Some(loser) = (0..2).find(|&p| sim.players[p].defeated) {
                            decided = Some((1 - loser, tick / 1800));
                            break;
                        }
                    }
                    ai_us.fetch_add(thinking.as_micros() as u64, Ordering::Relaxed);
                    if std::env::var("TA_LADDER_ORDERS").is_ok() {
                        // Ground units at the end: worth by first order, home (within 1500 px of
                        // the start) or away.
                        for p in 0..2u8 {
                            let start = players[p as usize].start;
                            let mut by: std::collections::BTreeMap<String, i64> = Default::default();
                            for u in sim.units().filter(|u| u.owner == p && !u.dead && u.is_complete()) {
                                let t = sim.unit_type(u);
                                if !matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover) || t.def.is_commander || t.is_builder || t.max_speed == Fx::ZERO {
                                    continue;
                                }
                                let order = match u.orders.front() {
                                    None => "idle".to_string(),
                                    Some(o) => format!("{o:?}").split(['(', ' ', '{']).next().unwrap_or("?").to_string(),
                                };
                                let place = if u.pos.within_xz(start, Fx::from_int(1500)) { "home" } else { "away" };
                                let carried = if u.carried_by.is_some() { " carried" } else { "" };
                                *by.entry(format!("{order} {place}{carried}")).or_default() += cost(&sim, u.type_id);
                            }
                            let list: Vec<String> = by.iter().map(|(k, v)| format!("{k}={v}")).collect();
                            println!("orders {} P{p}: {}", maps[m].0, list.join(", "));
                            // The ground domain's worth by type, largest first.
                            let mut types: std::collections::BTreeMap<String, i64> = Default::default();
                            for u in sim.units().filter(|u| u.owner == p && !u.dead && domain(&sim, u.type_id) == 1) {
                                *types.entry(sim.unit_type(u).name.clone()).or_default() += cost(&sim, u.type_id);
                            }
                            let mut top: Vec<(String, i64)> = types.into_iter().collect();
                            top.sort_by_key(|t| -t.1);
                            let list: Vec<String> = top.iter().take(6).map(|(k, v)| format!("{k}={v}")).collect();
                            println!("ground {} P{p}: {}", maps[m].0, list.join(", "));
                        }
                    }
                    if std::env::var("TA_TEST_LOG").is_ok() {
                        for p in 0..2u8 {
                            let mut kinds: std::collections::BTreeMap<String, u32> = Default::default();
                            for u in sim.units().filter(|u| u.owner == p && !u.dead) {
                                *kinds.entry(sim.unit_type(u).name.clone()).or_default() += 1;
                            }
                            let list: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} x{n}")).collect();
                            println!("end P{p}: {}", list.join(", "));
                        }
                    }
                    air.strikes = [ais[0].air_strikes, ais[1].air_strikes];
                    air.commander_strikes = [ais[0].commander_strikes, ais[1].commander_strikes];
                    let air = Air { spent: ab(flip, air.spent), lost: ab(flip, air.lost), kills: ab(flip, air.kills), value: ab(flip, air.value), strikes: ab(flip, air.strikes), commander_strikes: ab(flip, air.commander_strikes) };
                    let w = ab(flip, [worth(&sim, 0), worth(&sim, 1)]);
                    let stats = |p: usize| &sim.players[p].stats;
                    let result = Game {
                        map: m,
                        game,
                        seed,
                        winner: decided.map(|(w, _)| who(w)).unwrap_or(if w[0] >= w[1] { 0 } else { 1 }),
                        decided: decided.map(|(_, m)| m),
                        worth: w,
                        metal: ab(flip, [stats(0).metal_produced.to_int(), stats(1).metal_produced.to_int()]),
                        kills: ab(flip, [stats(0).kills, stats(1).kills]),
                        lost: ab(flip, [stats(0).lost, stats(1).lost]),
                        air,
                        left: ab(flip, [remnants(&sim, 0), remnants(&sim, 1)]),
                        owned: ab(flip, [0u8, 1].map(|p| {
                            let mut o = [0i64; 4];
                            for u in sim.units().filter(|u| u.owner == p && !u.dead && !sim.unit_type(u).def.is_commander) {
                                o[domain(&sim, u.type_id)] += cost(&sim, u.type_id);
                            }
                            o
                        })),
                        killed_by: ab(flip, killed_by),
                        idle: ab(flip, [0u8, 1].map(|p| {
                            let mut o = [0i64; 2];
                            for u in sim.units().filter(|u| u.owner == p && !u.dead && u.orders.is_empty() && u.is_complete()) {
                                let t = sim.unit_type(u);
                                match t.locomotion {
                                    Locomotion::Hover => o[0] += cost(&sim, u.type_id),
                                    Locomotion::Ground if !t.def.is_commander && t.max_speed > Fx::ZERO && !t.is_builder => o[1] += cost(&sim, u.type_id),
                                    _ => {}
                                }
                            }
                            o
                        })),
                    };
                    results.lock().unwrap().push(result);
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|g| (g.map, g.game));
    let (mut wins, mut ko, mut metal, mut kills) = ([0; 2], [0; 2], [0i64; 2], [0u64; 2]);
    let mut air = Air::default();
    let (mut owned, mut killed_by) = ([[0i64; 4]; 2], [[0i64; 4]; 2]);
    let mut idle = [[0i64; 2]; 2];
    let mut arm_wins = 0;
    for g in &results {
        let (swap, core_a) = (g.game % 2, g.game / 2 == 1);
        wins[g.winner] += 1;
        arm_wins += ((g.winner == 0) != core_a) as u32;
        ko[g.winner] += g.decided.is_some() as u32;
        for p in 0..2 {
            metal[p] += g.metal[p];
            kills[p] += g.kills[p];
            air.spent[p] += g.air.spent[p];
            air.lost[p] += g.air.lost[p];
            air.kills[p] += g.air.kills[p];
            air.value[p] += g.air.value[p];
            air.strikes[p] += g.air.strikes[p];
            air.commander_strikes[p] += g.air.commander_strikes[p];
            for d in 0..4 {
                owned[p][d] += g.owned[p][d];
                killed_by[p][d] += g.killed_by[p][d];
            }
            idle[p][0] += g.idle[p][0];
            idle[p][1] += g.idle[p][1];
        }
        let how = match g.decided {
            Some(m) => format!("defeated the other at minute {m}"),
            None => {
                let l = g.left[1 - g.winner];
                format!("ahead {} to {} (loser keeps {}b {}g {}s {}a{})", g.worth[g.winner], g.worth[1 - g.winner], l[0], l[1], l[2], l[3], if l[4] > 0 { " +com" } else { "" })
            }
        };
        println!(
            "{:18} {} start {swap} seed {}: {:?} {} {how:56} metal {:>6} / {:>6}, kills {:>3} / {:>3}, lost {:>3} / {:>3}",
            maps[g.map].0,
            if core_a { "A=CORE" } else { "A=ARM " },
            g.seed,
            [a, b][g.winner],
            ["(A)", "(B)"][g.winner],
            g.metal[0],
            g.metal[1],
            g.kills[0],
            g.kills[1],
            g.lost[0],
            g.lost[1]
        );
    }
    let games = results.len().max(1);
    println!(
        "metal produced {} / {}, kills {} / {} ({} per game), AI time {:.1} s ({:.0} ms per game)",
        metal[0],
        metal[1],
        kills[0],
        kills[1],
        (kills[0] + kills[1]) as usize / games,
        ai_us.load(Ordering::Relaxed) as f64 / 1e6,
        ai_us.load(Ordering::Relaxed) as f64 / 1e3 / games as f64
    );
    println!(
        "aircraft: spent {} / {}, lost {} / {}, strikes {} / {} ({} / {} at the Commander), their kills {} / {} worth {} / {}",
        air.spent[0], air.spent[1], air.lost[0], air.lost[1], air.strikes[0], air.strikes[1], air.commander_strikes[0], air.commander_strikes[1], air.kills[0], air.kills[1], air.value[0], air.value[1]
    );
    for (p, name) in [(0, "A"), (1, "B")] {
        let o = owned[p];
        let k = killed_by[p];
        println!(
            "{name}: owns at the end {} buildings, {} ground ({} idle hover, {} idle other), {} ships, {} air; killed by its buildings {}, ground {}, ships {}, air {}",
            o[0] / games as i64,
            o[1] / games as i64,
            idle[p][0] / games as i64,
            idle[p][1] / games as i64,
            o[2] / games as i64,
            o[3] / games as i64,
            k[0] / games as i64,
            k[1] / games as i64,
            k[2] / games as i64,
            k[3] / games as i64
        );
    }
    // Points margin per game, A's view (100 when A defeated B): steadier than wins alone.
    let margins: Vec<f64> = results
        .iter()
        .map(|g| match g.decided {
            Some(_) => if g.winner == 0 { 100.0 } else { -100.0 },
            None => (g.worth[0] - g.worth[1]) as f64 * 100.0 / (g.worth[0] + g.worth[1]).max(1) as f64,
        })
        .collect();
    let mean = margins.iter().sum::<f64>() / games as f64;
    let var = margins.iter().map(|m| (m - mean) * (m - mean)).sum::<f64>() / (games as f64 - 1.0).max(1.0);
    println!("margin for A {mean:+.1}% (standard error {:.1})", (var / games as f64).sqrt());
    println!("ARM won {arm_wins} of {games}");
    println!("{a:?} (A) {} wins ({} outright), {b:?} (B) {} wins ({} outright)", wins[0], ko[0], wins[1], ko[1]);
}
