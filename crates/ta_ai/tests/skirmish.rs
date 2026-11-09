//! AI vs AI on real game data (skipped without a TA install).
//!
//! `TA_TEST_MAP` picks the map (default Sherwood), `TA_TEST_MINUTES` the length (default
//! 12), `TA_TEST_DIFFICULTY` (easy, normal, hard) the even players' level and
//! `TA_TEST_DIFFICULTY2` the odd players' (default: the same). `TA_TEST_SWAP=1` rotates the
//! start positions; `TA_TEST_PLAYERS=n` plays n AIs (sides alternate ARM/CORE) and
//! `TA_TEST_TEAMS=1` puts them on two alternating teams (e.g. a 2v2). `TA_TEST_SEED=n` changes
//! the game's random seed (7), and with it the AIs' choices; `TA_TEST_TWEAKS=expanders=0`
//! changes every AI's level parameters (`SkirmishAi::tweak`); `TA_TEST_GIVE_WAY=1` plays with
//! the rule "units give way"; `TA_TEST_MUTATORS=no_rush=5,resources=8` with a skirmish's
//! mutators (`Mutators::parse`).
//!
//! `TA_TEST_MOD=<folder>` plays with a mod (a folder with its mod.toml, as the launcher
//! installs them) mounted over the game; players then alternate the data's first two sides
//! (`TA_TEST_SIDES=TAL,CORE` picks them by name).
//! Diagnostics: `TA_TEST_TRACE=1` prints every building lost and what hit it last (and any
//! unit finished a second time, with the builders at it),
//! `TA_TEST_DUMP=<minute>` lists every unit (position, order, progress) at that minute and
//! `TA_TEST_LOG=1` prints the AI's debug log (jobs, waves, raid responses).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use ta_ai::{Difficulty, SkirmishAi};
use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Fx, FxVec3, Locomotion, PlayerSetup, Sim, SimEvent, UnitId, UnitTypes};

/// Prints the AI's debug log.
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

fn difficulty(var: &str) -> Option<Difficulty> {
    match std::env::var(var).ok()?.to_ascii_lowercase().as_str() {
        "easy" => Some(Difficulty::Easy),
        "hard" => Some(Difficulty::Hard),
        "normal" => Some(Difficulty::Normal),
        _ => None,
    }
}

fn joined<T: std::fmt::Display>(v: &[T]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" / ")
}

#[test]
fn ai_vs_ai_plays_a_game() {
    if std::env::var("TA_TEST_LOG").is_ok() && log::set_logger(&PrintLog).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    // (TA Reborn's own maps too, and `TA_TEST_MOD`'s mod over them as the game mounts it: what
    // it replaces of the install left out.)
    let module = std::env::var("TA_TEST_MOD").ok().map(|dir| {
        let dir = std::path::PathBuf::from(dir);
        assert!(dir.join("mod.toml").is_file(), "TA_TEST_MOD: a mod's folder with its mod.toml");
        ta_vfs::mods::read_mod(&dir).expect("a valid mod.toml").expect("a mod's folder")
    });
    let mounted = ta_vfs::mods::mount(Some(&install.root), ta_vfs::builtin_dir().as_deref(), &module.iter().collect::<Vec<_>>(), &|m| m.manifest.id.clone());
    mounted.install.unwrap();
    mounted.mods.unwrap();
    if let Some(m) = &module {
        println!("mod: {} {}", m.manifest.name, m.manifest.version);
        if !mounted.replaced.is_empty() {
            println!("it replaces the install's {}", mounted.replaced.join(", "));
        }
    }
    let vfs = mounted.vfs;
    let data = Arc::new(GameData::load(&vfs));
    // The sides played, and their commanders: ARM and CORE, or the data's first two (a mod's).
    let wanted: Vec<String> = std::env::var("TA_TEST_SIDES").map(|s| s.split(',').map(|x| x.trim().to_ascii_uppercase()).collect()).unwrap_or_default();
    let mut sides: Vec<(String, String)> = Vec::new();
    for s in &data.sides {
        if (wanted.is_empty() || wanted.contains(&s.name)) && !sides.iter().any(|(n, _)| *n == s.name) {
            sides.push((s.name.clone(), s.commander.clone()));
        }
    }
    if wanted.is_empty() && data.sides.iter().any(|s| s.name == "ARM") && data.sides.iter().any(|s| s.name == "CORE") {
        sides.retain(|(n, _)| n == "ARM" || n == "CORE");
        sides.sort_by_key(|(n, _)| n != "ARM");
    }
    assert!(sides.len() >= 2, "two sides to play: {sides:?}");
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map_name = std::env::var("TA_TEST_MAP").unwrap_or_else(|_| "Sherwood".into());
    let minutes: u64 = std::env::var("TA_TEST_MINUTES").ok().and_then(|v| v.parse().ok()).unwrap_or(12);
    let first = difficulty("TA_TEST_DIFFICULTY").unwrap_or(Difficulty::Normal);
    let second = difficulty("TA_TEST_DIFFICULTY2").unwrap_or(first);
    let (Some(tnt), Some(ota)) = (vfs.read_opt(&format!("maps/{map_name}.tnt")), vfs.read_opt(&format!("maps/{map_name}.ota"))) else {
        panic!("map '{map_name}' not found (see `ta-tools find maps --ext tnt`)");
    };
    let map = TntMap::parse(&tnt).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&ota));
    let starts = &info.schemas[0].start_positions;
    let n: usize = std::env::var("TA_TEST_PLAYERS").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    assert!(n >= 2 && starts.len() >= n, "map '{map_name}' has fewer than {n} start positions");
    let swap = std::env::var("TA_TEST_SWAP").is_ok() as usize;
    let teams = std::env::var("TA_TEST_TEAMS").is_ok();
    let players: Vec<PlayerSetup> = (0..n)
        .map(|i| {
            let s = &starts[(i + swap) % n];
            PlayerSetup {
                side: sides[i % 2].0.clone(),
                color: i as u8,
                start: FxVec3::new(Fx::from_f32(s.1), Fx::ZERO, Fx::from_f32(s.2)),
                metal: Fx::from_int(1000),
                energy: Fx::from_int(1000),
                ai: true,
            }
        })
        .collect();
    let seed: u64 = std::env::var("TA_TEST_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
    let mut sim = Sim::new(types.clone(), data, &map, &info, 0, &players, seed);
    sim.unit_limit = 1500;
    // `TA_TEST_GIVE_WAY=1`: units give way (the skirmish rule; off by default here, so the
    // baselines stay comparable).
    sim.local_avoidance = std::env::var("TA_TEST_GIVE_WAY").is_ok_and(|v| v == "1");
    sim.group_formations = sim.local_avoidance;
    if let Ok(spec) = std::env::var("TA_TEST_MUTATORS") {
        sim.set_mutators(ta_sim::Mutators::parse(&spec));
    }
    for (i, p) in players.iter().enumerate() {
        sim.spawn_unit(types.id(&sides[i % 2].1).expect("the side's commander"), i as u8, p.start, 0, true);
        if teams {
            sim.set_team(i as u8, (i % 2) as u8);
        }
    }
    let levels: Vec<Difficulty> = (0..n).map(|i| if i % 2 == 0 { first } else { second }).collect();
    let mut ais: Vec<SkirmishAi> = (0..n).map(|i| SkirmishAi::new(i as u8, &sim, levels[i])).collect();
    for kv in std::env::var("TA_TEST_TWEAKS").unwrap_or_default().split(',').filter(|kv| !kv.trim().is_empty()) {
        let (k, v) = kv.split_once('=').expect("TA_TEST_TWEAKS: name=value,...");
        let v: i64 = v.trim().parse().expect("TA_TEST_TWEAKS: a number");
        for ai in &mut ais {
            assert!(ai.tweak(k.trim(), v), "unknown AI parameter {k}");
        }
    }
    let lineup: Vec<String> = (0..n).map(|i| format!("P{i} {} {:?}{}", players[i].side, levels[i], if teams { format!(" (team {})", i % 2) } else { String::new() })).collect();
    println!("{map_name}: {}", lineup.join(" vs "));

    let started = Instant::now();
    let mut ai_time = 0.0;
    let (mut shots, mut deaths, mut built) = (0u32, 0u32, vec![0u32; n]);
    let (mut launches, mut interceptions) = (0u32, 0u32);
    let mut peak_units = 0;
    let mut dead_ids = HashSet::new();
    let mut dead_types: BTreeMap<String, u32> = Default::default();
    let mut firsts: BTreeMap<String, (u64, u8)> = Default::default();
    let mut owners: HashMap<UnitId, u8> = Default::default();
    let mut extractor_losses = vec![0u32; n];
    let mut commander_lost: Vec<Option<u64>> = vec![None; n];
    let mut income_at: Vec<(u64, Vec<f32>, Vec<f32>)> = Vec::new();
    let mut dupes = 0;
    // How many times each unit was finished: once (one taken apart by its own side and built
    // back up is not finished again).
    let mut finished: HashMap<UnitId, u32> = Default::default();
    // Units ordered, per order kind (a proxy for the path searches the AI causes).
    let mut ordered: BTreeMap<String, u64> = Default::default();
    // Metal in finished units per player: economy, defences, constructors, army.
    let mut spent = vec![[0f32; 4]; n];
    let mut ally_kills = 0u32;
    // Ticks (from minute 3, past the opening) per player: out of metal (under 2% of the
    // store), out of metal with the energy store over half full (power idle while metal holds
    // everything up), the energy store brimming (income thrown away), and both stores over
    // half full (production not keeping up).
    let mut dry = vec![[0u64; 4]; n];
    // Units started in a factory, and those destroyed there unfinished, per player.
    let mut in_yard: HashSet<UnitId> = HashSet::new();
    let mut yard_losses = vec![0u32; n];
    let trace = std::env::var("TA_TEST_TRACE").is_ok();
    let dump_at: Option<u64> = std::env::var("TA_TEST_DUMP").ok().and_then(|v| v.parse().ok());
    // Last hit on every unit before this tick (name, owner of the attacker, victim finished).
    let mut last_hit: HashMap<UnitId, (String, u8, bool)> = Default::default();
    for tick in 0..30 * 60 * minutes {
        if trace || teams {
            last_hit.clear();
            for u in sim.units() {
                if let Some(a) = u.last_attacker.and_then(|a| sim.unit(a)) {
                    last_hit.insert(u.id, (sim.unit_type(a).name.clone(), a.owner, u.is_complete()));
                }
            }
        }
        let t0 = Instant::now();
        let mut commands = Vec::new();
        for ai in &mut ais {
            commands.extend(ai.think(&sim));
        }
        ai_time += t0.elapsed().as_secs_f64();
        for c in &commands {
            let (kind, count) = match c {
                ta_sim::Command::Order { units, order, .. } => (format!("{order:?}").split(['(', ' ']).next().unwrap_or("").to_string(), units.len() as u64),
                ta_sim::Command::SetActive { units, .. } => ("SetActive".into(), units.len() as u64),
                other => (format!("{other:?}").split([' ', '{']).next().unwrap_or("").to_string(), 1),
            };
            *ordered.entry(kind).or_insert(0) += count;
        }
        sim.step(&commands);
        for e in sim.take_events() {
            match e {
                SimEvent::WeaponFired { weapon, .. } => {
                    shots += 1;
                    let d = &sim.weapon_types.get(weapon).def;
                    if d.stockpile && d.command_fire {
                        launches += 1;
                        println!("tick {tick} ({} min): strategic launch ({})", tick / 1800, sim.weapon_types.get(weapon).name);
                    }
                }
                SimEvent::Intercepted { .. } => {
                    interceptions += 1;
                    println!("tick {tick} ({} min): missile intercepted", tick / 1800);
                }
                SimEvent::UnitCreated { id, .. } => {
                    if let Some(u) = sim.unit(id) {
                        owners.insert(id, u.owner);
                        if u.factory.is_some() {
                            in_yard.insert(id);
                        }
                    }
                }
                SimEvent::UnitDestroyed { id, type_id, pos, .. } => {
                    if !dead_ids.insert(id) {
                        dupes += 1;
                    }
                    if in_yard.remove(&id) && let Some(&o) = owners.get(&id) {
                        yard_losses[o as usize] += 1;
                    }
                    let t = sim.types.get(type_id);
                    *dead_types.entry(t.name.clone()).or_insert(0u32) += 1;
                    deaths += 1;
                    let Some(&o) = owners.get(&id) else { continue };
                    let by = last_hit.get(&id);
                    if by.is_some_and(|b| b.1 != o && sim.allied(b.1, o)) {
                        ally_kills += 1;
                    }
                    if trace && t.locomotion == Locomotion::Static {
                        let tag = if by.is_some_and(|b| !b.2) { "nanoframe" } else { "" };
                        println!("  {:5.1} min: P{o} {} {tag} lost at {pos:?} (last hit by {:?})", tick as f32 / 1800.0, t.name, by.map(|b| (&b.0, b.1)));
                    }
                    if t.extracts_metal > Fx::ZERO {
                        extractor_losses[o as usize] += 1;
                    }
                    if t.def.is_commander && commander_lost[o as usize].is_none() {
                        commander_lost[o as usize] = Some(tick / 1800);
                    }
                }
                SimEvent::UnitCompleted { id, .. } => {
                    in_yard.remove(&id);
                    let Some(u) = sim.unit(id) else { continue };
                    let times = finished.entry(id).or_insert(0);
                    *times += 1;
                    if *times > 1 && trace {
                        // (The builders at it: taking it apart, repairing, guarding, building.)
                        let hands: Vec<String> = sim
                            .units()
                            .filter(|h| h.id != id && (h.build.is_some_and(|b| b.target == id) || matches!(h.orders.front(), Some(ta_sim::Order::ReclaimUnit(t) | ta_sim::Order::Repair(t) | ta_sim::Order::Guard(t)) if *t == id)))
                            .map(|h| format!("{} {:?}", sim.unit_type(h).name, h.orders.front()))
                            .collect();
                        println!("  {:5.1} min: P{} {} finished again ({times} times); at it: {}", tick as f32 / 1800.0, u.owner, sim.unit_type(u).name, hands.join(", "));
                    }
                    built[u.owner as usize] += 1;
                    let t = sim.unit_type(u);
                    let kind = if t.locomotion == Locomotion::Static {
                        t.weapons.iter().any(|w| w.is_some()) as usize
                    } else if t.is_builder {
                        2
                    } else {
                        3
                    };
                    if !t.def.is_commander {
                        spent[u.owner as usize][kind] += t.cost_metal.to_f32();
                    }
                    // First completion of each advanced or strategic type, and when.
                    let c = t.def.category.to_ascii_uppercase();
                    if c.contains("LEVEL2") || c.contains("LEVEL3") || t.needs_geo {
                        firsts.entry(t.name.clone()).or_insert((tick / 1800, u.owner));
                    }
                }
                _ => {}
            }
        }
        peak_units = peak_units.max(sim.unit_count());
        if tick >= 3 * 1800 {
            for (p, d) in dry.iter_mut().enumerate() {
                let e = &sim.players[p].economy;
                let out_of_metal = e.metal * 50 < e.metal_capacity;
                let half = |v: Fx, cap: Fx| v * 2 >= cap;
                d[0] += out_of_metal as u64;
                d[1] += (out_of_metal && half(e.energy, e.energy_capacity)) as u64;
                d[2] += (e.energy * 100 >= e.energy_capacity * 99) as u64;
                d[3] += (half(e.metal, e.metal_capacity) && half(e.energy, e.energy_capacity)) as u64;
            }
        }
        if dump_at.is_some_and(|m| tick == m * 1800) {
            for u in sim.units() {
                println!(
                    "  P{} {:9} ({:5}, {:5}) hp {:5.0} build {:3.0}% active {} front {:?} busy {}",
                    u.owner,
                    sim.unit_type(u).name,
                    u.pos.x.to_int(),
                    u.pos.z.to_int(),
                    u.health.to_f32(),
                    u.build_progress.to_f32() * 100.0,
                    u.activated as u8,
                    u.orders.front(),
                    u.build.is_some() as u8
                );
            }
        }
        if tick % (30 * 60) == 0 {
            let summary: Vec<String> = (0..n)
                .map(|p| {
                    let e = &sim.players[p].economy;
                    // Mobile armed units and their cost.
                    let army: Vec<&ta_sim::Unit> = sim
                        .units()
                        .filter(|u| {
                            let t = sim.unit_type(u);
                            u.owner == p as u8 && u.is_complete() && t.locomotion != Locomotion::Static && !t.is_builder && t.weapons.iter().any(|w| w.is_some())
                        })
                        .collect();
                    let cost: f32 = army.iter().map(|u| (sim.unit_type(u).cost_metal + sim.unit_type(u).cost_energy / 60).to_f32()).sum();
                    format!(
                        "P{p}: {:3} units, army {:3} ({:5.0}), M {:4.0}(+{:4.1}) E {:5.0}(+{:6.1})",
                        sim.players[p].unit_count,
                        army.len(),
                        cost,
                        e.metal.to_f32(),
                        e.metal_income.to_f32(),
                        e.energy.to_f32(),
                        e.energy_income.to_f32()
                    )
                })
                .collect();
            println!("t={:2}min  {}", tick / 1800, summary.join(" | "));
        }
        // Incomes averaged over the 20 s before each checkpoint (they jitter per tick).
        let minute = tick / 1800 + 1;
        if [5, 10, 15, 20, 30, 40, 50, 60].contains(&minute) && tick % 1800 >= 1800 - 30 * 20 {
            if income_at.last().is_none_or(|(m, ..)| *m != minute) {
                income_at.push((minute, vec![0.0; n], vec![0.0; n]));
            }
            let last = income_at.last_mut().unwrap();
            for p in 0..n {
                last.1[p] += sim.players[p].economy.metal_income.to_f32() / 600.0;
                last.2[p] += sim.players[p].economy.energy_income.to_f32() / 600.0;
            }
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    println!(
        "simulated {minutes} min in {elapsed:.2}s ({:.0}x real time, AI {ai_time:.2}s); built {built:?}, shots {shots}, deaths {deaths}, peak units {peak_units}",
        (minutes * 60) as f64 / elapsed,
    );
    for (m, metal, energy) in &income_at {
        let metal: Vec<String> = metal.iter().map(|v| format!("{v:.1}")).collect();
        let energy: Vec<String> = energy.iter().map(|v| format!("{v:.0}")).collect();
        println!("income at {m:2} min (last 20 s avg): metal {}, energy {}", joined(&metal), joined(&energy));
    }
    println!("units ordered: {ordered:?}");
    for (p, s) in spent.iter().enumerate() {
        let total: f32 = s.iter().sum::<f32>().max(1.0);
        println!(
            "P{p} metal in finished units: economy {:.0} ({:.0}%), defence {:.0} ({:.0}%), constructors {:.0} ({:.0}%), army {:.0} ({:.0}%)",
            s[0],
            s[0] * 100.0 / total,
            s[1],
            s[1] * 100.0 / total,
            s[2],
            s[2] * 100.0 / total,
            s[3],
            s[3] * 100.0 / total
        );
    }
    println!("extractor losses {extractor_losses:?}, commander lost at minute {commander_lost:?}");
    if minutes > 3 {
        let span = ((minutes - 3) * 1800) as f32 / 100.0;
        for (p, d) in dry.iter().enumerate() {
            println!(
                "P{p} from minute 3: out of metal {:.0}% of the time, out of metal with energy banked (store half full) {:.0}%, energy store brimming {:.0}%, both stores half full {:.0}%; units lost unfinished in a factory {}",
                d[0] as f32 / span,
                d[1] as f32 / span,
                d[2] as f32 / span,
                d[3] as f32 / span,
                yard_losses[p]
            );
        }
    }
    if teams {
        println!("units killed by an ally {ally_kills}");
    }
    println!("strategic launches {launches}, interceptions {interceptions}");
    println!("first advanced/strategic builds (minute, player): {firsts:?}");
    println!("{:?}", sim.stats);
    // (The same game ends in the same state: a change to the simulation or the AI that keeps
    // their rules keeps this.)
    println!("final checksum {:016x}", sim.checksum());
    println!("unique deaths {}, duplicate destroy events {dupes}; by type {dead_types:?}", dead_ids.len());
    let refinished = finished.values().filter(|&&c| c > 1).count();
    println!("units finished more than once {refinished}");
    for p in 0..n {
        let mut names: Vec<String> = sim.units().filter(|u| u.owner == p as u8).map(|u| sim.unit_type(u).name.clone()).collect();
        names.sort();
        let mut counted: Vec<String> = Vec::new();
        for name in &names {
            if counted.last().is_none_or(|c| !c.starts_with(&format!("{name} "))) {
                counted.push(format!("{name} x{}", names.iter().filter(|m| *m == name).count()));
            }
        }
        println!("P{p} ({} units{}): {}", sim.players[p].unit_count, if sim.players[p].defeated { ", defeated" } else { "" }, counted.join(", "));
    }
    assert_eq!(dupes, 0, "a unit must be destroyed once");
    assert_eq!(refinished, 0, "a unit must be finished once");
    assert!(built.iter().all(|&b| b > 10), "every AI should build (built {built:?})");
    assert!(shots > 50, "armies should have fought");
    if let Some((_, metal, _)) = income_at.iter().find(|(m, ..)| *m == 10) {
        assert!(metal.iter().all(|&m| m >= 8.0), "every AI should make at least 8 metal/s by minute 10 ({metal:?})");
    }
}
