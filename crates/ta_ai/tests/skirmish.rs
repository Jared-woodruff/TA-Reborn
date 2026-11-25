//! AI vs AI on real game data (skipped without a TA install).
//!
//! `TA_TEST_MAP` picks the map (default Sherwood), `TA_TEST_MINUTES` the length (default
//! 12), `TA_TEST_DIFFICULTY` (easy, normal, hard) the even players' level and
//! `TA_TEST_DIFFICULTY2` the odd players' (default: the same); `TA_TEST_PERSONA` (balanced,
//! rush, turtle, tech, air, naval, random) the even players' persona and `TA_TEST_PERSONA2` the
//! odd players' (default: the same). `TA_TEST_SWAP=1` rotates the
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
//! Diagnostics: `TA_TEST_TRACE=1` prints every building and Commander lost and what hit it last
//! (and any unit finished a second time, with the builders at it),
//! `TA_TEST_DUMP=<minute>` lists every unit (position, order, progress) at that minute and
//! `TA_TEST_LOG=1` prints the AI's debug log (jobs, waves, raid responses).
//! `TA_TEST_UNTIL_DECIDED=1` stops once one team is left (else the winner plays on to the end).
//!
//! It reports, beside the economy, each player's tech tree as used (`audit_kind`): strategic and
//! tactical launches and missiles shot down, the experimentals finished, every five minutes the
//! big projects under way (how far along, how many at work on each; the strategic silos' missiles),
//! every ten the army by tier (and what was finished in those ten minutes), and who won (the one
//! left, else on points).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use ta_ai::{Difficulty, Persona, SkirmishAi};
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

fn persona(var: &str) -> Option<Persona> {
    Persona::from_key(&std::env::var(var).ok()?)
}

fn joined<T: std::fmt::Display>(v: &[T]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" / ")
}

/// A unit type's tier: its LEVELn (or a mod's LEVn) tag, 1 to 3.
fn tier(t: &ta_sim::UnitType) -> u8 {
    t.def.category.split_whitespace().filter_map(|token| {
        let token = token.to_ascii_uppercase();
        token.strip_prefix("LEVEL").or_else(|| token.strip_prefix("LEV"))?.parse::<u8>().ok().filter(|l| (1..=5).contains(l)).map(|l| l.min(3))
    }).max().unwrap_or(1)
}

/// What the tech tree audit counts a finished unit as, by its properties (a mod's units too):
/// factories by tier, strategic buildings, artillery, experimentals and the army by tier; none
/// for the rest (constructors, economy, small towers, the Commander).
fn audit_kind(sim: &Sim, type_id: usize) -> Option<&'static str> {
    let t = sim.types.get(type_id);
    if t.def.is_commander {
        return None;
    }
    let weapons: Vec<&ta_sim::WeaponType> = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w)).collect();
    let cost = (t.cost_metal + t.cost_energy / 60).to_int();
    let nuclear = |w: &&ta_sim::WeaponType| w.def.stockpile && w.command_fire && w.def.targetable_classes() & ta_sim::STRATEGIC_CLASS != 0;
    let anti = |w: &&ta_sim::WeaponType| w.def.interceptor && w.def.interceptor_classes() & ta_sim::STRATEGIC_CLASS != 0;
    let gun = |w: &&ta_sim::WeaponType| w.hits_ground && !w.def.stockpile && !w.def.interceptor && !w.command_fire && w.default_damage > Fx::ZERO;
    let tier = tier(t);
    if t.locomotion == Locomotion::Static {
        if sim.types.makes_units(t) {
            return Some(["factory T1", "factory T2", "factory T3"][tier as usize - 1]);
        }
        if weapons.iter().any(nuclear) {
            return Some("nuclear silo");
        }
        if weapons.iter().any(|w| w.def.stockpile && w.command_fire) {
            return Some("other silo");
        }
        if weapons.iter().any(anti) {
            return Some("anti-nuke");
        }
        if weapons.iter().any(|w| gun(w) && w.range >= Fx::from_int(2000)) {
            return Some("long-range gun");
        }
        if weapons.iter().any(gun) && cost >= 2000 {
            return Some("heavy defence");
        }
        return None;
    }
    let armed = weapons.iter().any(|w| gun(w) || w.hits_air && !w.def.interceptor && w.default_damage > Fx::ZERO);
    // (One that builds units too, Forged Alliance's Fatboy, is an experimental all the same; one
    // that puts up buildings, its support commanders, a constructor.)
    if armed && cost >= 5000 && !constructs(sim, t) {
        return Some("experimental");
    }
    if t.is_builder {
        return None;
    }
    if weapons.iter().any(anti) {
        return Some("mobile anti-nuke");
    }
    if !armed {
        return None;
    }
    let artillery = weapons.iter().any(|w| gun(w) && matches!(w.trajectory, ta_sim::Trajectory::Ballistic | ta_sim::Trajectory::Guided) && w.range >= Fx::from_int(600));
    Some(match (t.locomotion, tier) {
        (Locomotion::Ground | Locomotion::Hover, _) if artillery => "mobile artillery",
        (Locomotion::Air, 1) => "aircraft T1",
        (Locomotion::Air, 2) => "aircraft T2",
        (Locomotion::Air, _) => "aircraft T3",
        (Locomotion::Naval, 1) => "ships T1",
        (Locomotion::Naval, 2) => "ships T2",
        (Locomotion::Naval, _) => "ships T3",
        (_, 1) => "army T1",
        (_, 2) => "army T2",
        _ => "army T3",
    })
}

/// A builder that puts up buildings (a constructor, the factions' support commanders among them),
/// not one that turns out units (Forged Alliance's Fatboy).
fn constructs(sim: &Sim, t: &ta_sim::UnitType) -> bool {
    t.is_builder && t.ai_build_options.iter().any(|&o| sim.types.get(o).locomotion == Locomotion::Static)
}

/// A unit type's place in the army: 0 to 2 its tier (`tier`), 3 an experimental (armed, mobile,
/// costing 5,000 or more: TA's Krogoth, Forged Alliance's Monkeylord, and its Fatboy, a factory
/// on tracks); none for what is no part of the army (buildings, constructors, the Commanders,
/// unarmed units).
fn army_tier(sim: &Sim, type_id: usize) -> Option<usize> {
    let t = sim.types.get(type_id);
    if t.locomotion == Locomotion::Static || t.def.is_commander || constructs(sim, t) {
        return None;
    }
    let armed = t.weapons.iter().flatten().any(|&w| {
        let w = sim.weapon_types.get(w);
        !w.def.interceptor && w.default_damage > Fx::ZERO
    });
    if !armed {
        return None;
    }
    if (t.cost_metal + t.cost_energy / 60).to_int() >= 5000 {
        return Some(3);
    }
    (!t.is_builder).then(|| tier(t) as usize - 1)
}

/// A player's finished army by tier (`army_tier`): how many, and what they cost.
fn army_by_tier(sim: &Sim, player: u8) -> ([u32; 4], [i64; 4]) {
    let (mut count, mut cost) = ([0u32; 4], [0i64; 4]);
    for u in sim.units().filter(|u| u.owner == player && !u.dead && u.is_complete()) {
        if let Some(k) = army_tier(sim, u.type_id) {
            let t = sim.unit_type(u);
            count[k] += 1;
            cost[k] += (t.cost_metal + t.cost_energy / 60).to_int();
        }
    }
    (count, cost)
}

/// A player's big projects under way: experimentals (in a factory or in the field), missile silos
/// and anti-missile defences going up, each with how far along it is and how many are at work on
/// it (its factory among them); and its strategic silos standing, with the missiles they hold and
/// how far along the next is.
fn projects_under_way(sim: &Sim, player: u8) -> String {
    let mut out: Vec<String> = Vec::new();
    for u in sim.units().filter(|u| u.owner == player && !u.dead) {
        let t = sim.unit_type(u);
        let missiles = t.weapons.iter().flatten().any(|&w| {
            let w = sim.weapon_types.get(w);
            w.def.stockpile && (w.command_fire || w.def.interceptor)
        });
        if u.is_complete() {
            let strategic = t.weapons.iter().flatten().any(|&w| {
                let w = sim.weapon_types.get(w);
                w.def.stockpile && w.command_fire && w.def.targetable_classes() & ta_sim::STRATEGIC_CLASS != 0
            });
            if t.locomotion == Locomotion::Static && strategic {
                out.push(format!("{} standing, {} missiles and the next {:.0}%", t.name, u.stockpile, u.stockpile_progress.to_f32() * 100.0));
            }
            continue;
        }
        if army_tier(sim, u.type_id) != Some(3) && !(t.locomotion == Locomotion::Static && missiles) {
            continue;
        }
        let hands = sim.units().filter(|h| h.owner == player && h.build.is_some_and(|b| b.target == u.id)).count();
        out.push(format!("{} {:.0}% ({} at it)", t.name, u.build_progress.to_f32() * 100.0, hands));
    }
    out.join(", ")
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
    // (As the game plays it: the game's features and the map's own alone.)
    let mut sim = Sim::new(types.clone(), data.for_map(&vfs, &map_name), &map, &info, 0, &players, seed);
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
    let first_persona = persona("TA_TEST_PERSONA").unwrap_or_default();
    let personas: Vec<Persona> = (0..n).map(|i| if i % 2 == 0 { first_persona } else { persona("TA_TEST_PERSONA2").unwrap_or(first_persona) }).collect();
    let mut ais: Vec<SkirmishAi> = (0..n).map(|i| SkirmishAi::with_persona(i as u8, &sim, levels[i], personas[i])).collect();
    for kv in std::env::var("TA_TEST_TWEAKS").unwrap_or_default().split(',').filter(|kv| !kv.trim().is_empty()) {
        let (k, v) = kv.split_once('=').expect("TA_TEST_TWEAKS: name=value,...");
        let v: i64 = v.trim().parse().expect("TA_TEST_TWEAKS: a number");
        for ai in &mut ais {
            assert!(ai.tweak(k.trim(), v), "unknown AI parameter {k}");
        }
    }
    let lineup: Vec<String> = (0..n).map(|i| format!("P{i} {} {:?} {:?}{}", players[i].side, levels[i], ais[i].persona, if teams { format!(" (team {})", i % 2) } else { String::new() })).collect();
    println!("{map_name}: {}", lineup.join(" vs "));

    let started = Instant::now();
    let mut ai_time = 0.0;
    let (mut shots, mut deaths, mut built) = (0u32, 0u32, vec![0u32; n]);
    let (mut launches, mut interceptions) = (0u32, 0u32);
    // TA Reborn's shields (a mod's: none of TA's), per player whose shields they are: hits, the damage
    // they took, collapses.
    let (mut shield_hits, mut shield_taken, mut shield_collapses) = (vec![0u32; n], vec![0i64; n], vec![0u32; n]);
    // The tech tree audit: per player, what was made of each kind (`audit_kind`) and the minute
    // of the first; strategic launches, long-range gun shots and missiles shot down.
    let mut made: Vec<BTreeMap<&'static str, (u32, u64)>> = vec![Default::default(); n];
    let mut launched = vec![0u32; n];
    let mut long_shots = vec![0u32; n];
    let mut shot_down = vec![0u32; n];
    // Missiles of the other classes launched (the Supreme Commander factions' tactical ones),
    // per player: `launched` counts the strategic ones (TA's every silo's, nukes and FA's
    // strategic launchers').
    let mut tactical = vec![0u32; n];
    // Of the missiles each player shot down, the strategic ones (a strategic projectile ended
    // where the interception was).
    let mut strategic_down = vec![0u32; n];
    // The army by tier (`army_tier`: first, second, third, experimental), per player: what was
    // finished in each ten minutes; the experimentals finished (minute, player, name); and the
    // minute each player was defeated.
    let decades = (minutes as usize).div_ceil(10).max(1);
    let mut by_tier: Vec<Vec<[u32; 4]>> = vec![vec![[0; 4]; decades]; n];
    let mut experimentals_done: Vec<(u64, u8, String)> = Vec::new();
    let mut defeated_at: Vec<Option<u64>> = vec![None; n];
    // How each played (its persona): the minute its army first pushed at an enemy's start (three
    // armed units or more within 1,500 px of it, looked at every 30 s), and the armed buildings,
    // armed aircraft and armed ships it finished.
    let starts: Vec<FxVec3> = players.iter().map(|p| p.start).collect();
    let mut first_push: Vec<Option<u64>> = vec![None; n];
    let (mut towers, mut planes, mut ships) = (vec![0u32; n], vec![0u32; n], vec![0u32; n]);
    let mut peak_units = 0;
    let mut dead_ids = HashSet::new();
    let mut dead_types: BTreeMap<String, u32> = Default::default();
    let mut firsts: BTreeMap<String, (u64, u8)> = Default::default();
    // Upgrades in place (player, what into: how many, the first's minute) and enhancements
    // installed (a mod's: TA's units have neither).
    let mut upgraded: BTreeMap<(u8, String), (u32, u64)> = Default::default();
    let mut enhanced: Vec<(u64, u8, String, String)> = Vec::new();
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
    // Ticks played (fewer than asked when the battle is decided sooner and `TA_TEST_UNTIL_DECIDED`).
    let mut played = 0u64;
    let until_decided = std::env::var("TA_TEST_UNTIL_DECIDED").is_ok_and(|v| v == "1");
    for tick in 0..30 * 60 * minutes {
        played = tick + 1;
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
                SimEvent::WeaponFired { unit, weapon, .. } => {
                    shots += 1;
                    let w = sim.weapon_types.get(weapon);
                    let by = owners.get(&unit).copied().or_else(|| sim.unit(unit).map(|u| u.owner)).map(|o| o as usize);
                    if w.def.stockpile && w.def.command_fire {
                        launches += 1;
                        let strategic = w.def.targetable_classes() & ta_sim::STRATEGIC_CLASS != 0;
                        if let Some(p) = by {
                            if strategic {
                                launched[p] += 1;
                            } else {
                                tactical[p] += 1;
                            }
                        }
                        // (The tactical ones are many: counted, not each told.)
                        if strategic {
                            println!("tick {tick} ({} min): strategic launch ({}) by P{by:?}", tick / 1800, w.name);
                        }
                    } else if w.range >= Fx::from_int(2000) && !w.def.interceptor && let Some(p) = by {
                        long_shots[p] += 1;
                    }
                }
                SimEvent::ShieldHit { unit, damage, .. } => {
                    if let Some(p) = sim.unit(unit).map(|u| u.owner as usize).filter(|&p| p < n) {
                        shield_hits[p] += 1;
                        shield_taken[p] += damage.to_int();
                    }
                }
                SimEvent::ShieldChanged { unit, collapsed: true, .. } => {
                    if let Some(p) = sim.unit(unit).map(|u| u.owner as usize).filter(|&p| p < n) {
                        shield_collapses[p] += 1;
                    }
                }
                SimEvent::Intercepted { defender, pos, .. } => {
                    interceptions += 1;
                    if let Some(c) = shot_down.get_mut(defender as usize) {
                        *c += 1;
                    }
                    let strategic = sim.spent.iter().any(|(p, _)| p.pos.within_xz(pos, Fx::from_int(96)) && sim.weapon_types.get(p.weapon).def.targetable_classes() & ta_sim::STRATEGIC_CLASS != 0);
                    if strategic && let Some(c) = strategic_down.get_mut(defender as usize) {
                        *c += 1;
                    }
                    println!("tick {tick} ({} min): {}missile intercepted by P{defender}", tick / 1800, if strategic { "strategic " } else { "" });
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
                    if trace && (t.locomotion == Locomotion::Static || t.def.is_commander) {
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
                SimEvent::UnitUpgraded { to, type_id, .. } => {
                    let Some(u) = sim.unit(to) else { continue };
                    let e = upgraded.entry((u.owner, sim.types.get(type_id).name.clone())).or_insert((0, tick / 1800));
                    e.0 += 1;
                }
                SimEvent::UnitEnhanced { id, index, on: true } => {
                    let Some(u) = sim.unit(id) else { continue };
                    let t = sim.unit_type(u);
                    enhanced.push((tick / 1800, u.owner, t.name.clone(), t.def.enhancements.get(index as usize).map_or_else(String::new, |e| e.name.clone())));
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
                    {
                        let t = sim.unit_type(u);
                        if t.weapons.iter().any(|w| w.is_some()) && !t.def.is_commander {
                            let o = u.owner as usize;
                            match t.locomotion {
                                Locomotion::Static => towers[o] += 1,
                                Locomotion::Air => planes[o] += 1,
                                Locomotion::Naval => ships[o] += 1,
                                _ => {}
                            }
                        }
                    }
                    if let Some(kind) = audit_kind(&sim, u.type_id) {
                        let e = made[u.owner as usize].entry(kind).or_insert((0, tick / 1800));
                        e.0 += 1;
                    }
                    if let Some(k) = army_tier(&sim, u.type_id) {
                        by_tier[u.owner as usize][((tick / 1800) as usize / 10).min(decades - 1)][k] += 1;
                        if k == 3 {
                            println!("tick {tick} ({} min): P{} finishes an experimental, {}", tick / 1800, u.owner, sim.unit_type(u).name);
                            experimentals_done.push((tick / 1800, u.owner, sim.unit_type(u).name.clone()));
                        }
                    }
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
        if tick % 900 == 0 {
            for p in 0..n {
                if first_push[p].is_some() {
                    continue;
                }
                let near = sim.units().filter(|u| {
                    let t = sim.unit_type(u);
                    u.owner == p as u8 && u.is_complete() && t.locomotion != Locomotion::Static && !t.is_builder && t.weapons.iter().any(|w| w.is_some())
                        && (0..n).any(|e| sim.is_enemy(p as u8, e as u8) && u.pos.within_xz(starts[e], Fx::from_int(1500)))
                }).count();
                if near >= 3 {
                    first_push[p] = Some(tick / 1800);
                }
            }
        }
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
        // Every five minutes the big projects under way (experimentals, in a factory or in the
        // field, and missile silos and anti-missile defences going up): how far along, and how
        // many hands at each; every ten, each army by tier.
        if tick % (5 * 1800) == 0 && tick > 0 {
            for p in 0..n {
                let line = projects_under_way(&sim, p as u8);
                if !line.is_empty() {
                    println!("t={:2}min  P{p} under way: {line}", tick / 1800);
                }
            }
        }
        if tick % (10 * 1800) == 0 && tick > 0 {
            for p in 0..n {
                let (count, cost) = army_by_tier(&sim, p as u8);
                let total: i64 = cost.iter().sum::<i64>().max(1);
                let decade = &by_tier[p][(tick / 1800 / 10) as usize - 1];
                println!(
                    "t={:2}min  P{p} army T1 {} / T2 {} / T3 {} / experimental {} (by cost {}% / {}% / {}% / {}%); finished in the last ten minutes T1 {} / T2 {} / T3 {} / experimental {}",
                    tick / 1800,
                    count[0],
                    count[1],
                    count[2],
                    count[3],
                    cost[0] * 100 / total,
                    cost[1] * 100 / total,
                    cost[2] * 100 / total,
                    cost[3] * 100 / total,
                    decade[0],
                    decade[1],
                    decade[2],
                    decade[3]
                );
            }
        }
        if tick % 300 == 0 {
            for (p, at) in defeated_at.iter_mut().enumerate() {
                if at.is_none() && sim.players[p].defeated {
                    *at = Some(tick / 1800);
                    println!("tick {tick} ({} min): P{p} is defeated", tick / 1800);
                }
            }
            // (`TA_TEST_UNTIL_DECIDED=1`: the battle decided, one team left, no more to see. Else
            // the winner plays on alone, to the end: how far up the tree it gets.)
            let standing: Vec<u8> = (0..n as u8).filter(|&p| !sim.players[p as usize].defeated).collect();
            if until_decided && (standing.len() <= 1 || standing.iter().all(|&p| !sim.is_enemy(standing[0], p))) {
                println!("the battle is over at minute {}", tick / 1800);
                break;
            }
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
    let minutes_played = played / 1800;
    println!(
        "simulated {minutes_played} min in {elapsed:.2}s ({:.0}x real time, AI {ai_time:.2}s); built {built:?}, shots {shots}, deaths {deaths}, peak units {peak_units}",
        (played / 30) as f64 / elapsed,
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
    if played > 3 * 1800 {
        let span = (played - 3 * 1800) as f32 / 100.0;
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
    println!("missile launches {launches} (strategic {}, tactical {}), interceptions {interceptions} (strategic {})", launched.iter().sum::<u32>(), tactical.iter().sum::<u32>(), strategic_down.iter().sum::<u32>());
    if sim.types.shields {
        let each: Vec<String> = (0..n).map(|p| format!("P{p} hits {} ({} damage taken), collapses {}", shield_hits[p], shield_taken[p], shield_collapses[p])).collect();
        println!("shields: {}", each.join("; "));
    }
    for p in 0..n {
        let kinds: Vec<String> = made[p].iter().map(|(k, (count, first))| format!("{k} {count} (min {first})")).collect();
        println!("P{p} tech: {}; strategic launches {}, tactical launches {}, long-range shots {}, missiles shot down {} (strategic {})", kinds.join(", "), launched[p], tactical[p], long_shots[p], shot_down[p], strategic_down[p]);
    }
    for p in 0..n {
        let decades: Vec<String> = by_tier[p]
            .iter()
            .enumerate()
            .map(|(k, d)| {
                let all = d.iter().sum::<u32>().max(1);
                format!("{}-{}: {}/{}/{}/{} (T2 {}%, T3 {}%)", k * 10, k * 10 + 10, d[0], d[1], d[2], d[3], d[1] * 100 / all, d[2] * 100 / all)
            })
            .collect();
        println!("P{p} army finished by tier (T1/T2/T3/experimental) per ten minutes: {}", decades.join(" | "));
        let x: Vec<String> = experimentals_done.iter().filter(|e| e.1 == p as u8).map(|e| format!("{} (min {})", e.2, e.0)).collect();
        println!("P{p} experimentals finished: {}", if x.is_empty() { "none".to_string() } else { x.join(", ") });
    }
    // Who won: the one left standing, else what each owns (cost, the Commander left out, as the
    // ladder counts it).
    let standing: Vec<usize> = (0..n).filter(|&p| !sim.players[p].defeated).collect();
    let worth: Vec<i64> = (0..n)
        .map(|p| sim.units().filter(|u| u.owner == p as u8 && !u.dead && !sim.unit_type(u).def.is_commander).map(|u| (sim.unit_type(u).cost_metal + sim.unit_type(u).cost_energy / 60).to_int()).sum())
        .collect();
    match standing.as_slice() {
        [w] => println!("winner: P{w} ({:?}); defeated at minute {defeated_at:?}", sides[*w % 2].0),
        _ => {
            let w = (0..n).max_by_key(|&p| (worth[p], std::cmp::Reverse(p))).unwrap_or(0);
            println!("winner on points: P{w} ({:?}), worth {worth:?}; defeated at minute {defeated_at:?}", sides[w % 2].0);
        }
    }
    println!("first advanced/strategic builds (minute, player): {firsts:?}");
    if !upgraded.is_empty() {
        println!("upgrades in place ((player, into): (how many, first minute)): {upgraded:?}");
    }
    for (minute, p, unit, name) in &enhanced {
        println!("enhancement at {minute:2} min: P{p} {unit} installs {name}");
    }
    for p in 0..n {
        println!("P{p} {:?}: first push at minute {:?}, armed buildings {}, armed aircraft {}, armed ships {}", ais[p].persona, first_push[p], towers[p], planes[p], ships[p]);
    }
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
    // Adjacency (a mod's: the Supreme Commander factions'): the buildings taking their
    // neighbours' bonuses at the end, how many links, and what the bonuses come to on average.
    for p in 0..n {
        let taking: Vec<&ta_sim::Unit> = sim.units().filter(|u| u.owner == p as u8 && u.adjacency.is_some()).collect();
        if taking.is_empty() {
            continue;
        }
        let links: usize = taking.iter().map(|u| u.adjacency.as_ref().map_or(0, |a| a.from.len())).sum();
        let mut by: Vec<String> = Vec::new();
        for a in ta_sim::AdjacencyAffect::ALL {
            let changed: Vec<f64> = taking.iter().filter_map(|u| u.adjacency.as_ref()).map(|j| j.change[a.index()].to_f64()).filter(|&c| c != 0.0).collect();
            if !changed.is_empty() {
                by.push(format!("{} {} ({:+.1}% on average)", a.key(), changed.len(), changed.iter().sum::<f64>() / changed.len() as f64 * 100.0));
            }
        }
        let mut pairs: Vec<String> = Vec::new();
        for u in &taking {
            for (g, _) in &u.adjacency.as_ref().unwrap().from {
                if let Some(g) = sim.unit(*g) {
                    pairs.push(format!("{}>{}", sim.unit_type(g).name, sim.unit_type(u).name));
                }
            }
        }
        pairs.sort();
        let mut kinds: Vec<String> = Vec::new();
        for pair in &pairs {
            if kinds.last().is_none_or(|k| !k.starts_with(&format!("{pair} "))) {
                kinds.push(format!("{pair} x{}", pairs.iter().filter(|q| *q == pair).count()));
            }
        }
        println!("P{p} adjacency: {} buildings take {links} neighbours' bonuses: {}; {}", taking.len(), by.join(", "), kinds.join(", "));
    }
    assert_eq!(dupes, 0, "a unit must be destroyed once");
    assert_eq!(refinished, 0, "a unit must be finished once");
    assert!(built.iter().all(|&b| b > 10), "every AI should build (built {built:?})");
    assert!(shots > 50, "armies should have fought");
    if let Some((_, metal, _)) = income_at.iter().find(|(m, ..)| *m == 10) {
        assert!(metal.iter().all(|&m| m >= 8.0), "every AI should make at least 8 metal/s by minute 10 ({metal:?})");
    }
}
