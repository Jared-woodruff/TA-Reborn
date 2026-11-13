//! Campaign missions against the real game data (skipped without a TA install), and the
//! mission editor's unit keys on made-up missions (no install needed).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ta_data::mission::{self, Mission};
use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::mission::{MissionSetup, Objective};
use ta_sim::{Cheat, Command, Fx, FxVec3, Order, PlayerSetup, Sim, SimEvent, Target, UnitId, UnitTypes};

struct Loaded {
    sim: Sim,
    mission: Mission,
    /// The placed units' script names (upper case).
    idents: rustc_hash::FxHashMap<String, UnitId>,
    /// All the placing found out (groups, control groups).
    setup: MissionSetup,
}

fn load(file: &str, difficulty: &str) -> Option<Loaded> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    load_from(&vfs, data, types, file, difficulty)
}

fn load_from(vfs: &ta_vfs::Vfs, data: Arc<GameData>, types: Arc<UnitTypes>, file: &str, difficulty: &str) -> Option<Loaded> {
    let own = mission::campaigns(vfs).into_iter().find(|c| c.missions.iter().any(|m| m.file.eq_ignore_ascii_case(file))).map(|c| c.side).unwrap_or_else(|| "ARM".into());
    let other = if own.eq_ignore_ascii_case("CORE") { "ARM" } else { "CORE" };
    let mission = Mission::load(vfs, file)?;
    let map = TntMap::parse(&vfs.read(&mission.map_path()).ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{file}")).ok()?));
    let schema = mission.schema(difficulty)?.clone();
    let players_needed = schema.units.iter().map(|u| u.player).max().unwrap_or(2).max(2) as usize;
    let players: Vec<PlayerSetup> = (0..players_needed)
        .map(|i| PlayerSetup {
            side: if i == 0 { own.clone() } else { other.into() },
            color: i as u8,
            start: FxVec3::ZERO,
            metal: Fx::from_f64(if i == 0 { schema.human_metal } else { schema.computer_metal }),
            energy: Fx::from_f64(if i == 0 { schema.human_energy } else { schema.computer_energy }),
            ai: i != 0,
        })
        .collect();
    let schema_index = mission.schemas.iter().position(|s| s.kind.eq_ignore_ascii_case(difficulty)).unwrap_or(0);
    let mut sim = Sim::new(types, data, &map, &info, schema_index, &players, 7);
    for player in 0..players_needed { sim.set_team(player as u8, (player > 0) as u8); }
    let allowed = mission::use_only_units(vfs, &mission.use_only);
    let setup = ta_sim::mission::setup_mission(&mut sim, &mission, &schema, allowed.as_deref());
    println!("{}: {} units placed ({} skipped), {} idents", mission.name, sim.unit_count(), setup.skipped.len(), setup.idents.len());
    Some(Loaded { sim, mission, idents: setup.idents.clone(), setup })
}

/// The computer players of a mission, as the game runs them.
fn computer_players(sim: &Sim) -> Vec<ta_ai::SkirmishAi> {
    (1..sim.players.len() as u8).map(|p| ta_ai::SkirmishAi::new(p, sim, ta_ai::Difficulty::Normal)).collect()
}

/// One tick with the computer players' commands (and any of ours); the events it made.
fn tick(sim: &mut Sim, ais: &mut [ta_ai::SkirmishAi], ours: &[Command]) -> Vec<SimEvent> {
    let mut cmds = ours.to_vec();
    for ai in ais.iter_mut() {
        cmds.extend(ai.think(sim));
    }
    sim.step(&cmds);
    sim.take_events()
}

fn ended(events: &[SimEvent]) -> Option<bool> {
    events.iter().find_map(|e| match e {
        SimEvent::MissionEnded { victory } => Some(*victory),
        _ => None,
    })
}

/// The first of the player's units of a type.
fn find(sim: &Sim, owner: u8, name: &str) -> UnitId {
    sim.units().find(|u| u.owner == owner && sim.unit_type(u).name.eq_ignore_ascii_case(name)).map(|u| u.id).unwrap_or_else(|| panic!("no {name} of player {owner}"))
}

#[test]
fn a_hero_returns_is_winnable() {
    let Some(Loaded { mut sim, mission, .. }) = load("AC01.ota", "Easy") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let _ = mission;
    let human: Vec<_> = sim.units().filter(|u| u.owner == 0).map(|u| u.id).collect();
    let scripted = sim.units().filter(|u| u.owner == 1 && !u.orders.is_empty()).count();
    assert!(!human.is_empty(), "the human gets units");
    assert!(scripted > 0, "enemy units carry their InitialMission scripts");
    assert!(sim.allowed.is_some(), "AC01 restricts the buildable units");
    // Enemy patrols should start moving.
    let start: Vec<(ta_sim::UnitId, FxVec3)> = sim.units().filter(|u| u.owner == 1).map(|u| (u.id, u.pos)).collect();
    for tick in 0..30 * 10 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::MissionEnded { victory } = e {
                let humans: Vec<String> = sim.units().filter(|u| u.owner == 0).map(|u| format!("{}@({:.0},{:.0})", sim.unit_type(u).name, u.pos.x.to_f32(), u.pos.z.to_f32())).collect();
                println!("tick {tick}: mission ended victory={victory}; rules {:?}; human units {humans:?}", sim.mission);
            }
        }
    }
    let moved = start.iter().filter(|(id, p)| sim.unit(*id).is_some_and(|u| !u.pos.within_xz(*p, Fx::from_int(40)))).count();
    println!("{moved} of {} enemy units moved in 10 s", start.len());
    assert!(moved > 0, "scripted patrols move");
    assert!(sim.mission.as_ref().unwrap().outcome.is_none(), "nothing decided yet");

    // Attack-move the task force to the objective (992, 656), fighting through the patrols.
    let goal = FxVec3::from_ints(992, 0, 656);
    let army: Vec<_> = human.iter().copied().filter(|&id| sim.unit(id).is_some_and(|u| sim.unit_type(u).locomotion != ta_sim::Locomotion::Static)).collect();
    let runner = army[0];
    sim.step(&[Command::Order { player: 0, units: army.clone(), order: Order::Fight(goal), queue: false }]);
    let mut result = None;
    for tick in 0..30 * 180 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::MissionEnded { victory } = e {
                result = Some((victory, tick));
            }
        }
        if result.is_some() {
            break;
        }
    }
    let survivors = army.iter().filter(|&&id| sim.unit(id).is_some()).count();
    println!("mission result {result:?}; {survivors} of {} survived", army.len());
    let _ = runner;
    assert_eq!(result.map(|r| r.0), Some(true), "the task force should reach the Gate");
}

#[test]
fn commander_captures_enemy_unit() {
    let Some(Loaded { mut sim, .. }) = load("AC02.ota", "Easy") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let com = sim.units().find(|u| u.owner == 0 && sim.unit_type(u).def.is_commander).map(|u| (u.id, u.pos)).expect("commander");
    // Spawn an enemy Flash right next to the commander and capture it.
    let flash = sim.types.id("ARMFLASH").unwrap();
    let site = sim.find_build_site(flash, com.1 + FxVec3::from_ints(60, 0, 0), 10).unwrap();
    let victim = sim.spawn_unit(flash, 1, site, 0, true).unwrap();
    // Hold fire across our forces (as a player capturing something would), then capture.
    let ours: Vec<ta_sim::UnitId> = sim.units().filter(|u| u.owner == 0).map(|u| u.id).collect();
    sim.step(&[
        Command::Order { player: 0, units: ours, order: Order::SetStates { fire: 0, movement: 0 }, queue: false },
        Command::Order { player: 0, units: vec![com.0], order: Order::Capture(victim), queue: true },
    ]);
    let mut captured = None;
    for tick in 0..30 * 20 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::UnitCaptured { id, to, .. } = e && id == victim && to == 0 {
                captured = Some(tick);
            }
        }
        if captured.is_some() {
            break;
        }
    }
    println!("captured after {captured:?} ticks");
    assert!(captured.is_some(), "the commander should capture the Flash");
    assert_eq!(sim.unit(victim).map(|u| u.owner), Some(0));
    let _ = Target::Unit(victim);
}

/// Mesa Run opens with the Commander aboard a Valkyrie (`i air7`), whose script flies it in,
/// sets it down (`u 606 2116`) and flies on into the missile towers (`m 1600 600`). Without
/// the `u` the Commander rode along and the mission was lost in 19 seconds. (Set down, the
/// Commander is in a Guardian's reach: the player has to run, as the mission's name says.)
#[test]
fn mesa_run_valkyrie_sets_the_commander_down() {
    let Some(Loaded { mut sim, idents, .. }) = load("Mesa Run.ota", "Medium") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let com = find(&sim, 0, "CORCOM");
    let valkyrie = idents["AIR7"];
    assert_eq!(sim.unit(com).unwrap().carried_by, Some(valkyrie), "the Commander starts aboard its Valkyrie");
    let mut ais = computer_players(&sim);
    let mut down = None;
    for t in 0..30 * 30 {
        let events = tick(&mut sim, &mut ais, &[]);
        assert_eq!(ended(&events), None, "Mesa Run decided at tick {t}");
        let u = sim.unit(com).expect("the Commander lives");
        if u.carried_by.is_none() {
            down = Some((t, u.pos, u.health));
            break;
        }
    }
    let (t, pos, health) = down.expect("the Valkyrie sets the Commander down");
    println!("the Commander is set down at tick {t}, at ({}, {}), health {}", pos.x.to_int(), pos.z.to_int(), health.to_int());
    assert!(pos.within_xz(FxVec3::from_ints(606, 0, 2116), Fx::from_int(96)), "where the script says");
    assert_eq!(health, sim.unit_type(sim.unit(com).unwrap()).max_health, "unharmed");
    // The empty Valkyrie flies on, as scripted.
    for _ in 0..30 * 2 {
        tick(&mut sim, &mut ais, &[]);
    }
    let v = sim.unit(valkyrie).expect("the Valkyrie");
    assert!(v.cargo.is_empty());
    assert_eq!(v.orders.front(), Some(&Order::Move(FxVec3::from_ints(1600, 0, 600))));
}

/// Force the Strait is won by getting the transport ship east past x = 6000 (TA's
/// `UnitTypePassesX`), which the mission didn't read: it could only be lost.
#[test]
fn force_the_strait_can_be_won() {
    let Some(Loaded { mut sim, mission, .. }) = load("Force the Strait.ota", "Medium") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let goal = ta_sim::mission::Objective::UnitTypePasses {
        unit: sim.types.id("ARMTSHIP").unwrap(),
        line: ta_sim::mission::Crossing { at: Fx::from_int(6000), toward: ta_data::mission::Toward::East },
    };
    assert_eq!(sim.mission.as_ref().unwrap().victory, vec![goal], "{:?}", mission.victory);
    // Through the strait (the enemy's guns can't hurt us: this is about the goal), to the
    // deepest water a little past the line.
    sim.cheats_allowed = true;
    let ship = find(&sim, 0, "ARMTSHIP");
    let x = Fx::from_int(6400);
    let z = (1..sim.world.size_px().1.to_int() / 64).map(|k| Fx::from_int(k * 64)).min_by_key(|&z| sim.world.height_at(x, z)).unwrap();
    let mut ais = computer_players(&sim);
    let go = [Command::Cheat { player: 0, cheat: Cheat::Invulnerable(true) }, Command::Order { player: 0, units: vec![ship], order: Order::Move(FxVec3::new(x, Fx::ZERO, z)), queue: false }];
    let mut result = None;
    for t in 0..30 * 600 {
        let events = tick(&mut sim, &mut ais, if t == 0 { &go } else { &[] });
        if let Some(v) = ended(&events) {
            result = Some((v, t));
            break;
        }
    }
    let at = sim.unit(ship).map(|u| (u.pos.x.to_int(), u.pos.z.to_int()));
    println!("result {result:?}; the ship at {at:?}");
    assert_eq!(result.map(|r| r.0), Some(true), "the ship gets through and the mission is won");
}

/// Lightning Recon is lost when an enemy unit gets west of x = 4500, the line before the
/// player's base; Surprise at Labrodorite when one gets west of x = 25 (off the map's west
/// edge: "do not let any Arm units escape to the west"). Neither was read.
#[test]
fn enemies_passing_x_lose_the_mission() {
    for (file, line) in [("Lightning Recon.ota", 4500), ("Surprise at Labrodorite.ota", 25)] {
        let Some(Loaded { mut sim, .. }) = load(file, "Medium") else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let lose = ta_sim::mission::Objective::AnyUnitPasses(ta_sim::mission::Crossing { at: Fx::from_int(line), toward: ta_data::mission::Toward::West });
        assert!(sim.mission.as_ref().unwrap().defeat.contains(&lose), "{file}: {:?}", sim.mission.as_ref().unwrap().defeat);
        // Our units hold their fire; an enemy scout of the east runs west, past the line.
        let ours: Vec<UnitId> = sim.units().filter(|u| u.owner == 0).map(|u| u.id).collect();
        let scout = sim.units().filter(|u| u.owner == 1 && sim.unit_type(u).locomotion == ta_sim::Locomotion::Ground).max_by_key(|u| (u.pos.x, u.id)).map(|u| u.id).unwrap();
        let (sx, sz) = sim.unit(scout).map(|u| (u.pos.x, u.pos.z)).unwrap();
        let to = FxVec3::new(Fx::from_int(line - 400).max(Fx::from_int(8)), Fx::ZERO, sz);
        println!("{file}: {} at ({}, {}) runs to x {}", sim.unit_type(sim.unit(scout).unwrap()).name, sx.to_int(), sz.to_int(), to.x.to_int());
        let orders = [
            Command::Order { player: 0, units: ours, order: Order::SetStates { fire: 0, movement: 0 }, queue: false },
            Command::Order { player: 1, units: vec![scout], order: Order::Move(to), queue: false },
        ];
        let mut result = None;
        for t in 0..30 * 300 {
            sim.step(if t == 0 { &orders } else { &[] });
            if let Some(v) = ended(&sim.take_events()) {
                result = Some((v, t));
                break;
            }
        }
        let past = sim.unit(scout).map(|u| u.pos.x.to_int());
        println!("{file}: {result:?}, the scout at x {past:?}");
        assert_eq!(result.map(|r| r.0), Some(false), "{file}: the enemy got past x = {line}");
        assert!(past.is_some_and(|x| x < line), "{file}: lost when the scout crossed");
    }
}

/// Factories build what their scripts say, one order after another: Jungle Journey's Kbot
/// Lab three Crashers and then three Thuds, its advanced lab three Cans (`b corcrash 3, b
/// corthud 3`), and nothing more.
#[test]
fn factories_build_what_their_scripts_ask() {
    let Some(Loaded { mut sim, .. }) = load("Jungle Journey.ota", "Medium") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    sim.cheats_allowed = true;
    let (lab, adv) = (find(&sim, 0, "CORLAB"), find(&sim, 0, "CORALAB"));
    assert!(sim.runs_mission_script(lab) && sim.runs_mission_script(adv));
    let mut built: Vec<(UnitId, String)> = Vec::new();
    let mut ais = computer_players(&sim);
    let fast = [Command::Cheat { player: 0, cheat: Cheat::Unlimited(true) }];
    for t in 0..30 * 600 {
        for e in tick(&mut sim, &mut ais, if t == 0 { &fast } else { &[] }) {
            if let SimEvent::UnitCompleted { id, type_id } = e
                && let Some(f) = sim.unit(id).and_then(|u| u.factory)
            {
                built.push((f, sim.types.get(type_id).name.clone()));
            }
        }
        if !sim.runs_mission_script(lab) && !sim.runs_mission_script(adv) {
            break;
        }
    }
    let of = |f: UnitId| built.iter().filter(|b| b.0 == f).map(|b| b.1.to_ascii_uppercase()).collect::<Vec<_>>();
    println!("lab {:?}; advanced lab {:?}", of(lab), of(adv));
    assert_eq!(of(lab), ["CORCRASH", "CORCRASH", "CORCRASH", "CORTHUD", "CORTHUD", "CORTHUD"]);
    assert_eq!(of(adv), ["CORCAN", "CORCAN", "CORCAN"]);
    assert!(sim.unit(lab).unwrap().factory_queue.is_empty(), "and nothing more");
}

/// Hovercraft transports set their units down one at a time, a `u` and a point each: the
/// Core Contingency's last Arm mission lands its army so (`u 3077 3758, u 3077 3868, u 3077
/// 3978, u 3177 3788, u 3277 3798, u 3377 3818`). (Our side can't be hurt here: the landing
/// is under fire.)
#[test]
fn transports_set_down_one_unit_a_point() {
    let Some(Loaded { mut sim, idents, .. }) = load("EXP1AC12.ota", "Medium") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    sim.cheats_allowed = true;
    let hover = idents["JOHNB"];
    let aboard: Vec<UnitId> = sim.unit(hover).unwrap().cargo.clone();
    assert_eq!(aboard.len(), 6);
    let mut ais = computer_players(&sim);
    let mut drops = Vec::new();
    let safe = [Command::Cheat { player: 0, cheat: Cheat::Invulnerable(true) }];
    for t in 0..30 * 240 {
        for e in tick(&mut sim, &mut ais, if t == 0 { &safe } else { &[] }) {
            if let SimEvent::UnitUnloaded { transport, unit } = e
                && transport == hover
                && let Some(u) = sim.unit(unit)
            {
                drops.push((u.pos.x.to_int(), u.pos.z.to_int(), sim.unit(hover).map_or(0, |h| h.cargo.len())));
            }
        }
        if !sim.runs_mission_script(hover) {
            break;
        }
    }
    println!("set down at {drops:?}");
    assert_eq!(drops.len(), 6, "all six are set down");
    // Each at its own point: they spread over the 300 pixels the points span, east to west,
    // where a single unloading would have put them all down round the first.
    let (lo, hi) = (drops.iter().map(|p| p.0).min().unwrap(), drops.iter().map(|p| p.0).max().unwrap());
    assert!(hi - lo > 200, "spread from x {lo} to {hi}");
    assert_eq!(drops.iter().map(|d| d.2).collect::<Vec<_>>(), [5, 4, 3, 2, 1, 0], "one a point");
}

/// A shipyard set to blow itself up (`wa target1, w 20, d`) waits for its guard to be
/// attacked, then 20 seconds (TA's waits are seconds), then goes up at once.
#[test]
fn scripts_wait_seconds_and_self_destruct() {
    let Some(Loaded { mut sim, idents, .. }) = load("Crossing the Long Lakes.ota", "Medium") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    sim.cheats_allowed = true;
    let (yard, guard) = (idents["SYTWO"], idents["TARGET1"]);
    let mut ais = computer_players(&sim);
    for _ in 0..30 * 5 {
        tick(&mut sim, &mut ais, &[]);
    }
    assert!(sim.unit(yard).is_some(), "the shipyard waits for its guard to be attacked");
    let attack = [Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![guard]) }];
    let mut gone = None;
    for t in 0..30 * 40 {
        tick(&mut sim, &mut ais, if t == 0 { &attack } else { &[] });
        if gone.is_none() && sim.unit(yard).is_none_or(|u| u.dead) {
            gone = Some(t);
        }
    }
    println!("the shipyard blew up {gone:?} ticks after its guard fell");
    let t = gone.expect("the shipyard blows itself up");
    assert!((20 * 30..20 * 30 + 15).contains(&t), "20 seconds after");
}

/// Units placed with `Immunity=1` take no harm: the Core campaign's first mission has its
/// Commander "deactivated" (holding its fire for an hour) and immune, while the player's
/// strike force cleans the Arm out. It was lost in a minute.
#[test]
fn immune_units_take_no_harm() {
    let Some(Loaded { mut sim, .. }) = load("CC01.ota", "Medium") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let com = find(&sim, 0, "CORCOM");
    assert!(sim.is_immune(com));
    let max = sim.unit_type(sim.unit(com).unwrap()).max_health;
    let mut ais = computer_players(&sim);
    let mut hit = false;
    for t in 0..30 * 120 {
        let events = tick(&mut sim, &mut ais, &[]);
        assert_eq!(ended(&events), None, "decided at tick {t}");
        let u = sim.unit(com).expect("the Commander");
        hit |= u.last_attacker.is_some();
        assert_eq!(u.health, max, "unharmed at tick {t}");
    }
    println!("the Commander was {}attacked", if hit { "" } else { "never " });
}

/// Scripts keep a game deterministic: a mission set up twice plays the same, tick for tick,
/// and so does a copy taken while the scripts run (saved games and replays, multiplayer).
#[test]
fn missions_with_scripts_play_the_same_twice() {
    for file in ["Mesa Run.ota", "Crossing the Long Lakes.ota", "EXP1AC12.ota"] {
        let (Some(a), Some(b)) = (load(file, "Medium"), load(file, "Medium")) else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let (mut a, mut b) = (a.sim, b.sim);
        let (mut ais_a, mut ais_b) = (computer_players(&a), computer_players(&b));
        let mut copy = None;
        for t in 0..30 * 100 {
            tick(&mut a, &mut ais_a, &[]);
            tick(&mut b, &mut ais_b, &[]);
            if t % 300 == 0 {
                assert_eq!(a.checksum(), b.checksum(), "{file}: set up twice, apart at tick {t}");
            }
            if t == 30 * 20 {
                copy = Some((a.clone(), ais_a.clone()));
            }
        }
        let (mut c, mut ais_c) = copy.unwrap();
        for _ in 30 * 20 + 1..30 * 100 {
            tick(&mut c, &mut ais_c, &[]);
        }
        assert_eq!(c.checksum(), a.checksum(), "{file}: the copy went its own way");
    }
}

/// Loads every campaign mission and runs 30 s with the AI. A mission decided before the
/// player does anything points at a misread objective.
#[test]
#[ignore]
fn every_mission_loads_and_runs() {
    let Some(install) = ta_vfs::install::discover(None).ok() else { return };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let mut early = Vec::new();
    let mut total = 0;
    for c in mission::campaigns(&vfs) {
        for entry in &c.missions {
            let Some(Loaded { mut sim, .. }) = load(&entry.file, "Medium") else {
                println!("  !! {} failed to load", entry.file);
                continue;
            };
            total += 1;
            let mut ais: Vec<ta_ai::SkirmishAi> =
                (1..sim.players.len() as u8).map(|p| ta_ai::SkirmishAi::new(p, &sim, ta_ai::Difficulty::Normal)).collect();
            let t0 = std::time::Instant::now();
            let mut ended = None;
            for tick in 0..30 * 30 {
                let mut cmds = Vec::new();
                for ai in &mut ais {
                    cmds.extend(ai.think(&sim));
                }
                sim.step(&cmds);
                for e in sim.take_events() {
                    if let SimEvent::MissionEnded { victory } = e {
                        ended = Some((victory, tick));
                    }
                }
                if ended.is_some() {
                    break;
                }
            }
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            println!("  {:10} {:>4} units, 30 s in {ms:6.0} ms{}", entry.file, sim.unit_count(), ended.map_or(String::new(), |e| format!("  ENDED {e:?}")));
            if ended.is_some() {
                early.push(entry.file.clone());
            }
        }
    }
    println!("{total} missions; decided early: {early:?}");
}

#[test]
#[ignore = "checks every campaign difficulty against the locally installed original game"]
fn every_campaign_difficulty_has_playable_objectives_at_the_start() {
    let install = ta_vfs::install::discover(None).expect("the campaign audit needs an installed game");
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let campaigns = mission::campaigns(&vfs);
    assert!(!campaigns.is_empty());
    let mut total = 0;
    for campaign in campaigns {
        for entry in &campaign.missions {
            for difficulty in ["Easy", "Medium", "Hard"] {
                let Loaded { mut sim, mission, .. } = load_from(&vfs, data.clone(), types.clone(), &entry.file, difficulty)
                    .unwrap_or_else(|| panic!("{} {difficulty} must load", entry.file));
                assert!(mission.schema(difficulty).is_some(), "{} has no schema or fallback for {difficulty}", entry.file);
                let rules = sim.mission.as_ref().unwrap();
                assert!(!rules.victory.is_empty(), "{} has no winning condition", entry.file);
                assert_eq!(rules.victory.len(), mission.victory.len(), "{} {difficulty} has an unresolved victory unit type", entry.file);
                assert!(mission.defeat.iter().all(|goal| Objective::from_goal(goal, &types).is_some()), "{} {difficulty} has an unresolved defeat unit type", entry.file);
                assert!(sim.units().any(|u| u.owner == 0));
                for unit in sim.units() {
                    assert!(unit.cargo.len() <= sim.unit_type(unit).transport_capacity as usize, "{} {difficulty} overloaded a transport", entry.file);
                    if let Some(carrier) = unit.carried_by {
                        let carrier = sim.unit(carrier).unwrap();
                        assert!(carrier.owner == unit.owner && sim.unit_type(carrier).can_carry(sim.unit_type(unit)), "{} {difficulty} placed invalid cargo", entry.file);
                    }
                }
                for _ in 0..15 { sim.step(&[]); }
                assert_eq!(sim.mission.as_ref().unwrap().outcome, None, "{} {difficulty} must not end immediately after loading", entry.file);
                total += 1;
            }
        }
    }
    println!("{total} campaign/difficulty combinations have playable initial objectives");
}

/// Plays a mission (`TA_MISSION`, default every one) for `TA_MINUTES` with nobody at the
/// player's controls, and prints what the scripted factories built, how many scripts still
/// run and how it ended.
#[test]
#[ignore]
fn missions_play_their_scripts() {
    let Some(install) = ta_vfs::install::discover(None).ok() else { return };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let files: Vec<String> = match std::env::var("TA_MISSION") {
        Ok(f) => vec![f],
        Err(_) => mission::campaigns(&vfs).iter().flat_map(|c| c.missions.iter().map(|m| m.file.clone())).collect(),
    };
    let minutes: u64 = std::env::var("TA_MINUTES").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
    for file in files {
        let Some(Loaded { mut sim, .. }) = load(&file, "Medium") else { continue };
        let factories: Vec<UnitId> = sim.units().filter(|u| sim.unit_type(u).is_factory && sim.runs_mission_script(u.id)).map(|u| u.id).collect();
        let scripts = sim.mission.as_ref().map_or(0, |m| m.scripts.len());
        let mut ais = computer_players(&sim);
        let mut built: std::collections::BTreeMap<String, u32> = Default::default();
        let mut result = None;
        for t in 0..30 * 60 * minutes {
            for e in tick(&mut sim, &mut ais, &[]) {
                match e {
                    SimEvent::UnitCompleted { id, type_id } if sim.unit(id).and_then(|u| u.factory).is_some_and(|f| factories.contains(&f)) => {
                        *built.entry(sim.types.get(type_id).name.clone()).or_default() += 1;
                    }
                    SimEvent::MissionEnded { victory } => result = Some((victory, t / 30)),
                    _ => {}
                }
            }
            if result.is_some() {
                break;
            }
        }
        let left = sim.mission.as_ref().map_or(0, |m| m.scripts.len());
        println!("{file}: {scripts} scripts, {left} still running; {} scripted factories built {built:?}; ended {result:?}", factories.len());
    }
}

/// Prints, each second, where the units named in `TA_TRACE_UNITS` (idents, or unit types for
/// their first one) are and what they do (`TA_MISSION`, `TA_SECONDS`).
#[test]
#[ignore]
fn trace_mission_units() {
    let file = std::env::var("TA_MISSION").unwrap_or_else(|_| "Mesa Run.ota".into());
    let Some(Loaded { mut sim, idents, .. }) = load(&file, "Medium") else { return };
    let names = std::env::var("TA_TRACE_UNITS").unwrap_or_else(|_| "AIR7,CORCOM".into());
    let watched: Vec<(String, UnitId)> = names
        .split(',')
        .filter_map(|n| {
            let n = n.trim().to_ascii_uppercase();
            let id = idents.get(&n).copied().or_else(|| sim.units().find(|u| sim.unit_type(u).name.eq_ignore_ascii_case(&n)).map(|u| u.id))?;
            Some((n, id))
        })
        .collect();
    let seconds: u64 = std::env::var("TA_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
    let mut ais = computer_players(&sim);
    for t in 0..30 * seconds {
        let owners: std::collections::HashMap<UnitId, (u8, String)> = sim.units().map(|u| (u.id, (u.owner, sim.unit_type(u).name.clone()))).collect();
        for e in tick(&mut sim, &mut ais, &[]) {
            match e {
                SimEvent::UnitDestroyed { id, .. } if watched.iter().any(|w| w.1 == id) => println!("tick {t}: {:?} destroyed", owners.get(&id)),
                SimEvent::UnitUnloaded { transport, unit } => println!("tick {t}: {:?} sets {:?} down", owners.get(&transport), owners.get(&unit)),
                SimEvent::OrderFailed { unit, reason } => println!("tick {t}: {:?}: {reason}", owners.get(&unit)),
                SimEvent::MissionEnded { victory } => println!("tick {t}: mission ended, victory {victory}"),
                _ => {}
            }
        }
        if t % 30 == 0 {
            for (n, id) in &watched {
                if let Some(u) = sim.unit(*id) {
                    let name = |id: UnitId| sim.unit(id).map(|a| format!("{}@({}, {})", sim.unit_type(a).name, a.pos.x.to_int(), a.pos.z.to_int()));
                    let attacker = u.last_attacker.and_then(name);
                    let aim = u.weapons[0].target.and_then(|t| match t {
                        Target::Unit(id) => name(id),
                        other => Some(format!("{other:?}")),
                    });
                    println!(
                        "{t:5} {n}: ({}, {}, y {}) hp {} carried {:?} cargo {} scripted {} orders {:?} hit by {attacker:?} aiming at {aim:?}",
                        u.pos.x.to_int(),
                        u.pos.z.to_int(),
                        u.pos.y.to_int(),
                        u.health.to_int(),
                        u.carried_by.map(|c| c.slot),
                        u.cargo.len(),
                        sim.runs_mission_script(*id),
                        u.orders.iter().take(3).collect::<Vec<_>>()
                    );
                }
            }
        }
    }
}

#[test]
#[ignore]
fn trace_mission_deaths() {
    let file = std::env::var("TA_MISSION").unwrap_or_else(|_| "AC09.ota".into());
    let Some(Loaded { mut sim, .. }) = load(&file, "Medium") else { return };
    let mut ais: Vec<ta_ai::SkirmishAi> = (1..sim.players.len() as u8).map(|p| ta_ai::SkirmishAi::new(p, &sim, ta_ai::Difficulty::Normal)).collect();
    if let Some(m) = &sim.mission {
        println!("win {:?}; lose {:?}; {} scripts", m.victory, m.defeat, m.scripts.len());
    }
    for tick in 0..30 * 30 {
        let mut cmds = Vec::new();
        for ai in &mut ais {
            cmds.extend(ai.think(&sim));
        }
        let owners: std::collections::HashMap<ta_sim::UnitId, (u8, String, Option<ta_sim::UnitId>)> =
            sim.units().map(|u| (u.id, (u.owner, sim.unit_type(u).name.clone(), u.last_attacker))).collect();
        sim.step(&cmds);
        for e in sim.take_events() {
            match e {
                SimEvent::UnitDestroyed { id, .. } => {
                    let (o, n, by) = owners.get(&id).cloned().unwrap_or_default();
                    let killer = by.and_then(|k| owners.get(&k)).map(|(ko, kn, _)| format!("{kn}/p{ko}"));
                    println!("tick {tick}: {n} (player {o}) destroyed by {killer:?}");
                }
                SimEvent::MissionEnded { victory } => {
                    println!("tick {tick}: mission ended, victory {victory}");
                    return;
                }
                _ => {}
            }
        }
    }
}

// ---- The mission editor's unit keys in TA's missions -------------------------------------------

/// Core mission 1's Commander, "deactivated" and immune, is marked `AiIgnore`: the Arm's
/// units never aim at it, and its computer player never sends anything after it.
#[test]
fn the_deactivated_commander_is_ignored() {
    let Some(Loaded { mut sim, .. }) = load("CC01.ota", "Medium") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let com = find(&sim, 0, "CORCOM");
    assert!(sim.ai_ignores(com) && sim.is_immune(com));
    let mut ais = computer_players(&sim);
    for t in 0..30 * 120 {
        tick(&mut sim, &mut ais, &[]);
        for u in sim.units().filter(|u| u.owner != 0) {
            assert!(!u.weapons.iter().any(|w| w.target == Some(Target::Unit(com))), "tick {t}: {} aims at the Commander", sim.unit_type(u).name);
            assert!(!u.orders.iter().any(|o| matches!(o, Order::Attack(Target::Unit(c)) if *c == com)), "tick {t}: {} is sent at the Commander", sim.unit_type(u).name);
        }
    }
}

/// Miniement Landing: "your Shipyard is a high priority target" (`AiPriorityTarget` on it
/// and a Guardian); the Core's escorts guard the transports by type (`g CORTSHIP`), where
/// they used to leave for the map's corner at once.
#[test]
fn miniement_landing_marks_the_shipyard_and_escorts_the_transports() {
    let Some(Loaded { mut sim, .. }) = load("Miniement Landing.ota", "Medium") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let yard = find(&sim, 0, "ARMSY");
    assert!(sim.is_priority_target(yard));
    let marked: Vec<String> = sim.priority_targets().iter().filter_map(|&id| sim.unit(id)).map(|u| format!("{}@{},{}", sim.unit_type(u).name, u.pos.x.to_int(), u.pos.z.to_int())).collect();
    assert_eq!(marked.len(), 2, "{marked:?}");
    sim.step(&[]);
    let transports: Vec<UnitId> = sim.units().filter(|u| sim.unit_type(u).name.eq_ignore_ascii_case("CORTSHIP")).map(|u| u.id).collect();
    let escorts: Vec<UnitId> = sim.units().filter(|u| u.owner == 1 && sim.unit_type(u).name.eq_ignore_ascii_case("CORROY") && u.pos.x < Fx::from_int(700)).map(|u| u.id).collect();
    assert_eq!(escorts.len(), 5);
    let name = |id: UnitId| sim.unit(id).map(|u| format!("{}@{},{}", sim.unit_type(u).name, u.pos.x.to_int(), u.pos.z.to_int()));
    for e in escorts {
        let front = sim.unit(e).unwrap().orders.front().cloned();
        let guarded = match front {
            Some(Order::Guard(t)) => Some(t),
            _ => None,
        };
        assert!(guarded.is_some_and(|t| transports.contains(&t)), "{:?} guards {:?} ({front:?})", name(e), guarded.and_then(name));
    }
}

/// `MoveUnitToRadius` naming a building is "build one there": Guardian Hill's "build a
/// Punisher on top of Guardian Hill" (a finished one; a nanoframe isn't one).
#[test]
fn a_building_raised_in_the_area_meets_its_goal() {
    let Some(Loaded { mut sim, .. }) = load("Guardian Hill.ota", "Medium") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let pun = sim.types.id("CORPUN").unwrap();
    let site = sim.find_build_site(pun, FxVec3::from_ints(1570, 0, 3480), 12).expect("room on the hill");
    // (The hill's Arm guns can't hurt it: this is about the goal.)
    sim.cheats_allowed = true;
    sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Invulnerable(true) }]);
    let frame = sim.spawn_unit(pun, 0, site, 0, false).unwrap();
    for _ in 0..30 * 2 {
        sim.step(&[]);
        assert_eq!(ended(&sim.take_events()), None, "a nanoframe is no Punisher");
    }
    sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![frame]) }]);
    sim.spawn_unit(pun, 0, site, 0, true).unwrap();
    let result = (0..30 * 2).find_map(|_| {
        sim.step(&[]);
        ended(&sim.take_events())
    });
    assert_eq!(result, Some(true), "the Punisher on the hill wins");
}

/// Attack on Barrack: "destroy the Advanced Kbot Lab and all the Dominators", both goals
/// (`KillUnitType=CORALAB, 1`, `KillAllOfType=CORHRK`); the Core builds its Dominators first
/// (`BuildPriority=CORHRK`).
#[test]
fn attack_on_barrack_takes_both_goals() {
    let Some(Loaded { mut sim, idents, .. }) = load("Attack on Barrack.ota", "Medium") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let hrk = sim.types.id("CORHRK").unwrap();
    assert_eq!(sim.build_priority(1).collect::<Vec<_>>(), vec![hrk]);
    sim.cheats_allowed = true;
    let run = |sim: &mut Sim, kill: Vec<UnitId>, ticks: u32| {
        sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(kill) }]);
        (0..ticks).find_map(|_| {
            sim.step(&[]);
            ended(&sim.take_events())
        })
    };
    assert_eq!(run(&mut sim, vec![idents["LAB"]], 60), None, "the lab alone");
    let dominators: Vec<UnitId> = sim.units().filter(|u| u.type_id == hrk).map(|u| u.id).collect();
    assert_eq!(run(&mut sim, dominators, 60), Some(true), "and every Dominator");
}

/// Sink the Battleship: the Titans start in control group 1 (`InitialGroup=1`; "keep your
/// Titans concentrated"), the limit is 250 units each, and sinking the battleship, which the
/// mission marks critical (the enemy's), wins.
#[test]
fn sink_the_battleship_groups_the_titans_and_is_won_by_sinking_it() {
    let Some(Loaded { mut sim, idents, setup, .. }) = load("Sink the Battleship.ota", "Easy") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    assert_eq!(sim.unit_limit, 250);
    let titans: Vec<String> = setup.control_groups.iter().map(|&(g, id)| format!("{g}:{}", sim.unit_type(sim.unit(id).unwrap()).name)).collect();
    assert_eq!(titans, ["1:CORTITAN"; 4]);
    sim.cheats_allowed = true;
    sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![idents["HARRY"]]) }]);
    let result = (0..60).find_map(|_| {
        sim.step(&[]);
        ended(&sim.take_events())
    });
    assert_eq!(result, Some(true));
}

/// Mesa Run's Roach starts aboard "the specified transport" by type (`i CORVALK`: the
/// nearest of the player's Valkyries), as TA's editor allows ("i ARMSHIP").
#[test]
fn passengers_board_a_transport_named_by_type() {
    let Some(Loaded { sim, idents, .. }) = load("Mesa Run.ota", "Medium") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let named: Vec<UnitId> = idents.values().copied().collect();
    let roaches: Vec<&ta_sim::Unit> = sim.units().filter(|u| u.owner == 0 && sim.unit_type(u).name.eq_ignore_ascii_case("CORROACH")).collect();
    // Two ride Valkyries called T2 and T3; the third rides the one with no name, beside it.
    let unnamed = roaches.iter().filter_map(|r| r.carried_by).filter(|t| !named.contains(t)).count();
    assert!(roaches.iter().all(|r| r.carried_by.is_some()), "every Roach aboard");
    assert_eq!(unnamed, 1);
}

/// Skirmish on the Sand's reserve waits `wa patrol`: on the patrol's three Peewees (the group
/// `patrol`), not only the one called so.
#[test]
fn skirmish_on_the_sand_reserve_waits_on_the_whole_patrol() {
    let Some(Loaded { mut sim, idents, setup, .. }) = load("Skirmish on the Sand.ota", "Medium") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let patrol = setup.groups["PATROL"].clone();
    assert_eq!(patrol.len(), 3);
    let other = patrol.iter().copied().find(|&p| p != idents["PATROL"]).unwrap();
    // (Rocketeers, Jethros and Hammers in a block west of the patrol.)
    let reserve: Vec<UnitId> =
        sim.units().filter(|u| u.owner == 1 && u.pos.x < Fx::from_int(600) && u.pos.z > Fx::from_int(780) && u.pos.z < Fx::from_int(860) && sim.runs_mission_script(u.id)).map(|u| u.id).collect();
    assert_eq!(reserve.len(), 9);
    sim.cheats_allowed = true;
    for _ in 0..30 {
        sim.step(&[]);
    }
    let waiting = |sim: &Sim| reserve.iter().filter(|&&r| sim.runs_mission_script(r)).count();
    assert_eq!(waiting(&sim), reserve.len(), "all wait");
    sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![other]) }]);
    for _ in 0..10 {
        sim.step(&[]);
    }
    assert_eq!(waiting(&sim), 0, "a Peewee of the patrol falls: the reserve moves");
}

// ---- The mission editor's unit keys on made-up missions (no install needed) -----------------

/// Game data from TDF files, loaded as a mod's folder is: a gun tower, a target, tanks and a
/// plant that makes them.
fn made_up_data() -> GameData {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let unit = |name: &str, keys: &str| {
        (format!("units/{}.fbi", name.to_ascii_lowercase()), format!("[UNITINFO]\n{{\nUnitName={name};\nObjectname={name};\nMaxDamage=1000;\nSightDistance=600;\n{keys}\n}}\n"))
    };
    let files = [
        unit("GUN", "Weapon1=GUN;"),
        // (A long-range gun as TA builds them: on hold fire.)
        unit("BERTHA", "Weapon1=GUN;\nNoAutoFire=1;\nStandingFireOrder=0;"),
        unit("TARGET", ""),
        unit("TANK", "Weapon1=GUN;\nMaxVelocity=1;\nCanMove=1;"),
        unit("TANK2", "Weapon1=GUN;\nMaxVelocity=1;\nCanMove=1;"),
        unit("LIFT", "CanFly=1;\nCanMove=1;\nMaxVelocity=1;\nCruiseAlt=60;\nTransportCapacity=1;\nTransportSize=1;"),
        unit("BIG", "MaxVelocity=1;\nCanMove=1;\nFootprintX=6;\nFootprintZ=6;"),
        unit("PLANT", "Builder=1;\nWorkerTime=100;\nFootprintX=4;\nFootprintZ=4;"),
        // (A gun TA builds on hold fire, though its file says fire at will: the Annihilator.)
        unit("ANNI", "Weapon1=GUN;\nNoAutoFire=1;\nStandingFireOrder=2;\nBuildTime=300;\nFootprintX=2;\nFootprintZ=2;"),
        unit("NANO", "Builder=1;\nWorkerTime=300;\nBuildDistance=300;\nMaxVelocity=1;\nCanMove=1;"),
        ("weapons/gun.tdf".into(), "[GUN]\n{\nrange=400;\nreloadtime=3;\nweaponvelocity=400;\n[DAMAGE]\n{\ndefault=1;\n}\n}\n".into()),
        (
            "download/plant.tdf".into(),
            "[MENUENTRY1]\n{\nUNITMENU=PLANT;\nMENU=1;\nBUTTON=0;\nUNITNAME=TANK;\n}\n[MENUENTRY2]\n{\nUNITMENU=PLANT;\nMENU=1;\nBUTTON=1;\nUNITNAME=TANK2;\n}\n".into(),
        ),
        ("download/nano.tdf".into(), "[MENUENTRY1]\n{\nUNITMENU=NANO;\nMENU=1;\nBUTTON=0;\nUNITNAME=ANNI;\n}\n".into()),
    ];
    let dir = std::env::temp_dir().join(format!("ta-mission-keys-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    for (path, text) in &files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_mod_dir(&dir, "test").unwrap();
    let data = GameData::load(&vfs);
    std::fs::remove_dir_all(&dir).ok();
    data
}

/// A made-up mission on a flat 1024 × 1024 map: the player (TA player 1) against one computer
/// player (2). `units` are (type, player, x, z, more keys); `header` holds the goals.
fn made_up_mission(header: &str, units: &[(&str, u32, i32, i32, &str)]) -> (Sim, MissionSetup) {
    let data = Arc::new(made_up_data());
    let types = Arc::new(UnitTypes::compile(&data, &|_| None));
    let cells = 64 * 64;
    let map = TntMap { width: 64, height: 64, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; cells], features: vec![FEATURE_NONE; cells], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    let players: Vec<PlayerSetup> = (0..2).map(|i| PlayerSetup { side: "ARM".into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: i == 1 }).collect();
    let mut sim = Sim::new(types, data, &map, &MapInfo::default(), 0, &players, 7);
    let placed: String = units.iter().enumerate().map(|(i, (name, player, x, z, more))| format!("[unit{i}]{{Unitname={name};XPos={x};ZPos={z};Player={player};{more}}}")).collect();
    let mission = Mission::from_ota("Made Up.ota", &tdf::parse(&format!("[GlobalHeader]{{{header}[Schema 0]{{Type=Easy;[units]{{{placed}}}}}}}")));
    let setup = ta_sim::mission::setup_mission(&mut sim, &mission, &mission.schemas[0], None);
    sim.cheats_allowed = true;
    (sim, setup)
}

#[test]
fn mission_attack_scripts_resolve_complete_idents_before_unit_types() {
    for ident in ["Power Mike", "TANK2"] {
        let script = format!("InitialMission=a {ident};");
        let (mut sim, _) = made_up_mission("", &[
            ("TANK", 1, 100, 100, &script),
            ("TANK2", 2, 200, 100, "Ident=decoy;"),
            ("TARGET", 2, 300, 100, &format!("Ident={ident};")),
        ]);
        let units = placed_units(&sim);
        sim.step(&[]);
        assert!(matches!(sim.unit(units[0]).unwrap().orders.front(), Some(Order::Attack(Target::Unit(target))) if *target == units[2]), "a {ident} must attack that named unit");
    }
}

#[test]
fn invalid_mission_player_numbers_cannot_wrap_into_another_players_army() {
    let (sim, _) = made_up_mission("", &[
        ("TANK", 1, 100, 100, ""),
        ("TARGET", 2, 300, 100, ""),
        ("TANK2", 257, 500, 100, ""),
    ]);
    assert_eq!(sim.unit_count(), 2, "player 257 is outside the mission, not player 1");
    assert_eq!(sim.players[0].unit_count, 1);
}

#[test]
fn fractional_campaign_timers_do_not_end_a_mission_ahead_of_time() {
    for (header, victory) in [("VictoryTimerRunsOut=1.5;", true), ("DeathTimerRunsOut=1.5;", false)] {
        let (mut sim, _) = made_up_mission(header, &[("TANK", 1, 100, 100, "")]);
        for _ in 0..44 { sim.step(&[]); }
        assert_eq!(sim.mission.as_ref().unwrap().outcome, None, "a 1.5-second timer must last 45 ticks");
        sim.step(&[]);
        assert_eq!(sim.mission.as_ref().unwrap().outcome, Some(victory));
    }
}

#[test]
fn mission_passengers_share_available_transports_without_exceeding_capacity() {
    for name in ["LIFT", "shuttles"] {
        let aboard = format!("InitialMission=i {name},w 2,m 800 800;");
        let (sim, _) = made_up_mission("", &[
            ("LIFT", 1, 100, 100, "InitialGroup=shuttles;"),
            ("LIFT", 1, 500, 100, "InitialGroup=shuttles;"),
            ("TANK", 1, 150, 100, &aboard),
            ("TANK", 1, 175, 100, &aboard),
            ("TANK", 1, 200, 100, &aboard),
        ]);
        assert!(sim.units().all(|u| u.cargo.len() <= sim.unit_type(u).transport_capacity as usize), "i {name} must respect each transport's capacity");
        assert_eq!(sim.units().filter(|u| u.carried_by.is_some()).count(), 2, "both transports take one passenger");
        let waiting = sim.units().find(|u| sim.unit_type(u).name == "TANK" && u.carried_by.is_none()).unwrap();
        assert!(!waiting.orders.is_empty(), "a passenger without a free transport still runs its own script");
    }
}

#[test]
fn named_mission_transports_must_be_friendly_and_able_to_carry_the_passenger() {
    for (carrier, player, passenger) in [("TARGET", 1, "TANK"), ("LIFT", 2, "TANK"), ("LIFT", 1, "BIG")] {
        let (sim, _) = made_up_mission("", &[
            (carrier, player, 100, 100, "Ident=bus;"),
            (passenger, 1, 200, 100, "InitialMission=i bus,w 2,m 800 800;"),
        ]);
        assert!(sim.units().all(|u| u.carried_by.is_none()), "a {passenger} cannot board player {player}'s {carrier}");
    }
}

/// The placed units, in the order they were placed.
fn placed_units(sim: &Sim) -> Vec<UnitId> {
    let mut ids: Vec<UnitId> = sim.units().map(|u| u.id).collect();
    ids.sort();
    ids
}

fn aim(sim: &Sim, id: UnitId) -> Option<Target> {
    sim.unit(id).and_then(|u| u.weapons[0].target)
}

/// "AI Should Ignore Unit ... the AI will act as though the unit does not exist" and
/// "Priority Target Of AI will make free units looking for something to attack head for
/// that unit first" (TA's editor): a computer player's gun passes over the nearest of the
/// player's units, ignored, for the farthest, a priority target, then takes the next; the
/// player's own gun still shoots a computer player's unit marked to be ignored.
#[test]
fn computer_guns_ignore_and_put_first_what_the_mission_says() {
    let (mut sim, _) = made_up_mission(
        "DestroyAllUnits=1;",
        &[
            ("GUN", 2, 300, 300, ""),
            ("TARGET", 1, 350, 300, "AiIgnore=1;"),
            ("TARGET", 1, 420, 300, ""),
            ("TARGET", 1, 520, 300, "AiPriorityTarget=1;"),
            ("GUN", 1, 300, 900, ""),
            ("TARGET", 2, 350, 900, "AiIgnore=1;"),
        ],
    );
    let [gun, ignored, plain, priority, ours, theirs] = placed_units(&sim)[..] else { panic!("six units") };
    assert!(sim.ai_ignores(ignored) && sim.ai_ignores(theirs) && !sim.ai_ignores(plain));
    assert!(sim.is_priority_target(priority) && !sim.is_priority_target(plain));
    for _ in 0..20 {
        sim.step(&[]);
    }
    assert_eq!(aim(&sim, gun), Some(Target::Unit(priority)), "the priority target first, though the farthest");
    assert_eq!(aim(&sim, ours), Some(Target::Unit(theirs)), "the player's guns are no computer player's");
    // The priority target gone: the next nearest, never the one to ignore.
    sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![priority]) }]);
    for _ in 0..20 {
        sim.step(&[]);
    }
    assert_eq!(aim(&sim, gun), Some(Target::Unit(plain)));
    sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![plain]) }]);
    for _ in 0..40 {
        sim.step(&[]);
        assert_eq!(aim(&sim, gun), None, "only the ignored unit is left in reach");
    }
}

/// "If a unit designated a mission critical unit is destroyed the mission is lost" (TA's
/// editor): the player's. A computer player's critical unit has no part in the goals.
#[test]
fn losing_a_critical_unit_loses_the_mission() {
    let (mut sim, _) = made_up_mission(
        "DestroyAllUnits=1;",
        &[("TARGET", 1, 300, 300, "MissionCriticalUnit=1;"), ("TARGET", 1, 400, 300, ""), ("TARGET", 2, 800, 800, "MissionCriticalUnit=1;"), ("GUN", 2, 900, 900, "")],
    );
    let [ours, _, theirs, _] = placed_units(&sim)[..] else { panic!("four units") };
    assert!(sim.mission.as_ref().unwrap().defeat.contains(&Objective::UnitLost(ours)));
    let run = |sim: &mut Sim, kill: UnitId| {
        sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![kill]) }]);
        (0..60).find_map(|_| {
            sim.step(&[]);
            ended(&sim.take_events())
        })
    };
    assert_eq!(run(&mut sim, theirs), None, "the computer player's critical unit");
    assert_eq!(run(&mut sim, ours), Some(false), "the player's");
}

/// "All the victory conditions must be met for a victory to occur" (TA's editor): Brawler
/// Buildup's "destroy all enemy Brawlers while ensuring your metal production continues for
/// at least twenty minutes", here two seconds. (Scenarios keep any one goal winning.)
#[test]
fn every_victory_goal_must_be_met() {
    let units = [("TARGET", 1, 300, 300, ""), ("TARGET", 2, 600, 600, "")];
    let goals = "KillAllOfType=TARGET;VictoryTimerRunsOut=2;";
    let outcome = |sim: &mut Sim, ticks: u32, cmds: &[Command]| {
        sim.step(cmds);
        (0..ticks).find_map(|_| {
            sim.step(&[]);
            ended(&sim.take_events()).map(|v| (v, sim.tick))
        })
    };
    let (mut sim, _) = made_up_mission(goals, &units);
    let enemy = placed_units(&sim)[1];
    assert_eq!(outcome(&mut sim, 30, &[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![enemy]) }]), None, "the enemy destroyed: not yet two seconds");
    let won = outcome(&mut sim, 60, &[]).expect("decided");
    assert!(won.0 && (60..=75).contains(&won.1), "won at two seconds: {won:?}");
    // The time alone isn't enough.
    let (mut sim, _) = made_up_mission(goals, &units);
    assert_eq!(outcome(&mut sim, 30 * 5, &[]), None);
    // Any one goal (the scenarios' rule): the time alone wins.
    let (mut sim, _) = made_up_mission(goals, &units);
    sim.mission.as_mut().unwrap().all_victory = false;
    assert_eq!(outcome(&mut sim, 30 * 5, &[]).map(|w| w.0), Some(true));
}

/// `MoveUnitToRadius` naming a building: one finished there (a nanoframe is none); `ANYTYPE`:
/// a unit that went there (a building standing on the spot doesn't count).
#[test]
fn a_building_goal_takes_a_finished_building() {
    let decided = |sim: &mut Sim| {
        (0..30).find_map(|_| {
            sim.step(&[]);
            ended(&sim.take_events())
        })
    };
    let (mut sim, _) = made_up_mission("MoveUnitToRadius=GUN, 500, 500, 100;", &[("TARGET", 1, 100, 100, ""), ("TARGET", 2, 900, 900, "")]);
    let gun = sim.types.id("GUN").unwrap();
    let frame = sim.spawn_unit(gun, 0, FxVec3::from_ints(500, 0, 500), 0, false).unwrap();
    assert_eq!(decided(&mut sim), None, "a nanoframe");
    sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![frame]) }]);
    sim.spawn_unit(gun, 0, FxVec3::from_ints(520, 0, 480), 0, true).unwrap();
    assert_eq!(decided(&mut sim), Some(true), "the gun raised there");
    let (mut sim, _) = made_up_mission("MoveUnitToRadius=ANYTYPE, 500, 500, 100;", &[("GUN", 1, 500, 500, ""), ("TARGET", 2, 900, 900, "")]);
    assert_eq!(decided(&mut sim), None, "standing there from the start");
    let tank = sim.types.id("TANK").unwrap();
    sim.spawn_unit(tank, 0, FxVec3::from_ints(450, 0, 500), 0, true).unwrap();
    assert_eq!(decided(&mut sim), Some(true), "a tank there");
}

/// `maxunits`: every player's limit ("these options work just like the options in
/// multiplayer play"). Placed units count, and may go past it.
#[test]
fn the_unit_limit_holds_every_player() {
    let (sim, _) = made_up_mission("maxunits=2;DestroyAllUnits=1;", &[("TARGET", 1, 300, 300, ""), ("TARGET", 1, 400, 300, ""), ("TARGET", 1, 500, 300, ""), ("TARGET", 2, 800, 800, "")]);
    assert_eq!(sim.unit_limit, 2);
    assert!(sim.at_unit_limit(0) && !sim.at_unit_limit(1));
    let (sim, _) = made_up_mission("DestroyAllUnits=1;", &[("TARGET", 1, 300, 300, "")]);
    assert_eq!(sim.unit_limit, u32::MAX, "no limit given, none set");
}

/// "Initial Group works much like setting a unique name, except that it allows a group of
/// units to be referenced rather than just one" (TA's editor): `wa patrol` waits on every
/// unit of the group of that name (and the one called so); `g TYPE` guards the nearest of
/// the type, its own side's first; the player's digits are control groups.
#[test]
fn groups_and_types_name_units_in_scripts() {
    let (mut sim, setup) = made_up_mission(
        "DestroyAllUnits=1;",
        &[
            ("TANK", 1, 300, 300, "InitialGroup=1;"),
            ("TANK", 1, 340, 300, "InitialGroup=1;"),
            ("TARGET", 2, 700, 300, "Ident=patrol;InitialGroup=patrol;"),
            ("TARGET", 2, 800, 300, "InitialGroup=patrol;"),
            ("TANK", 2, 900, 900, "InitialMission=wa patrol, m 900 600;"),
            ("TARGET", 1, 950, 950, ""),
            ("TARGET", 2, 600, 900, ""),
            ("TANK", 2, 1000, 900, "InitialMission=g TARGET, m 100 100;"),
        ],
    );
    let ids = placed_units(&sim);
    assert_eq!(setup.control_groups, vec![(1, ids[0]), (1, ids[1])]);
    assert_eq!(setup.groups["PATROL"], vec![ids[2], ids[3]]);
    let (waiter, guard, their_target) = (ids[4], ids[7], ids[6]);
    for _ in 0..30 {
        sim.step(&[]);
    }
    assert!(matches!(sim.unit(waiter).unwrap().orders.front(), Some(Order::Wait(_))), "waiting on the patrol");
    // The guard: the computer player's TARGET, though the player's is nearer.
    assert_eq!(sim.unit(guard).unwrap().orders.front(), Some(&Order::Guard(their_target)));
    // The group's other unit falls: the wait is over.
    sim.step(&[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![ids[3]]) }]);
    for _ in 0..5 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(waiter).unwrap().orders.front(), Some(&Order::Move(FxVec3::from_ints(900, 0, 600))));
}

/// "Tell AI to Build this Unit with Priority means of all the units specified this
/// particular unit should be built first" (TA's editor): a computer player's plant turns
/// out the type its units are marked with.
#[test]
fn computer_players_build_the_priority_type_first() {
    for mark in ["TANK2", "TANK"] {
        let more = format!("BuildPriority={mark};");
        let (mut sim, _) = made_up_mission("DestroyAllUnits=1;", &[("PLANT", 2, 300, 300, ""), ("TANK", 2, 600, 300, &more), ("TARGET", 1, 900, 900, "")]);
        assert_eq!(sim.build_priority(1).collect::<Vec<_>>(), vec![sim.types.id(mark).unwrap()]);
        let plant = placed_units(&sim)[0];
        let mut ai = ta_ai::SkirmishAi::new(1, &sim, ta_ai::Difficulty::Normal);
        let mut first = None;
        for _ in 0..30 * 30 {
            let cmds = ai.think(&sim);
            first = first.or(cmds.iter().find_map(|c| match c {
                Command::Produce { factory, type_id, .. } if *factory == plant => Some(sim.types.get(*type_id).name.clone()),
                _ => None,
            }));
            sim.step(&cmds);
            if first.is_some() {
                break;
            }
        }
        assert_eq!(first.as_deref(), Some(mark), "marked {mark}");
    }
}

/// Killing the enemy Commander where the enemy has none (the Core Contingency's last mission
/// sets that goal against an Arm with only decoys): there is none to kill, so the goal is met
/// once the rest are; under the scenarios' any-goal rule it never wins on its own.
#[test]
fn no_enemy_commander_is_none_to_kill() {
    let units = [("TARGET", 1, 300, 300, ""), ("TARGET", 2, 600, 600, "")];
    let outcome = |sim: &mut Sim, ticks: u32, cmds: &[Command]| {
        sim.step(cmds);
        (0..ticks).find_map(|_| {
            sim.step(&[]);
            ended(&sim.take_events())
        })
    };
    let (mut sim, _) = made_up_mission("KillEnemyCommander=1;DestroyAllUnits=1;", &units);
    assert_eq!(outcome(&mut sim, 60, &[]), None, "the enemy still has a unit");
    let enemy = placed_units(&sim)[1];
    assert_eq!(outcome(&mut sim, 60, &[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![enemy]) }]), Some(true), "nothing of the enemy left");
    let (mut sim, _) = made_up_mission("KillEnemyCommander=1;", &units);
    sim.mission.as_mut().unwrap().all_victory = false;
    assert_eq!(outcome(&mut sim, 60, &[]), None, "any one goal: no Commander has fallen");
}

/// TA builds its long-range guns on hold fire (`NoAutoFire=1`) for a player to set going. Every
/// computer player of a mission has a `SkirmishAi`, which sets its own going: at once where no
/// script runs them, else once the script is over ("if a Unit is given no initial commands, or
/// runs out of commands, the AI will take over", TA's editor), a script's `o` (move order, then
/// fire order) holding or freeing them meanwhile (CC15's Berthas wait, unfiring, to be
/// captured). The player's own keep their file's fire orders (a Bertha's: hold fire) for the
/// player to change.
#[test]
fn a_computer_player_sets_its_mission_guns_going_once_their_scripts_end() {
    let (mut sim, _) = made_up_mission(
        "DestroyAllUnits=1;",
        &[
            ("BERTHA", 2, 300, 300, ""),
            ("BERTHA", 2, 500, 300, "InitialMission=w 2;"),
            ("BERTHA", 2, 700, 300, "InitialMission=o 0 0, w 2, o 0 1;"),
            ("BERTHA", 1, 300, 900, ""),
            ("TARGET", 1, 500, 900, ""),
        ],
    );
    let [free, waiting, ordered, ours, _] = placed_units(&sim)[..] else { panic!("five units") };
    let fire = |sim: &Sim, id: UnitId| sim.unit(id).unwrap().fire_state;
    let mut ai = ta_ai::SkirmishAi::new(1, &sim, ta_ai::Difficulty::Normal);
    let mut run = |sim: &mut Sim, ticks: u32| {
        for _ in 0..ticks {
            let cmds = ai.think(sim);
            sim.step(&cmds);
        }
    };
    run(&mut sim, 30);
    assert_eq!(fire(&sim, free), 2, "the computer player sets its unscripted gun going");
    assert_eq!(fire(&sim, waiting), 0, "a gun its script runs holds its fire");
    assert_eq!(fire(&sim, ordered), 0, "`o 0 0`: hold position, hold fire");
    run(&mut sim, 45);
    assert_eq!(fire(&sim, waiting), 2, "its script over, the computer player sets it going");
    assert_eq!(fire(&sim, ordered), 1, "`o 0 1`: hold position, return fire");
    run(&mut sim, 60);
    assert_eq!(fire(&sim, ordered), 1, "as its script left it: the computer player sets going only guns holding their fire");
    assert_eq!(fire(&sim, ours), 0, "the player's waits for the player");
}

/// TA sets its `NoAutoFire` guns to hold fire as they are built. The units a mission places for
/// the player were never built: they keep their file's fire orders (the Annihilator's file says
/// fire at will, the Big Bertha's hold fire). A computer player's placed gun waits for its
/// computer player to set it going, and one the player builds comes out on hold fire.
#[test]
fn the_players_placed_guns_keep_their_files_fire_orders() {
    let (mut sim, _) = made_up_mission(
        "DestroyAllUnits=1;",
        &[("ANNI", 1, 300, 900, ""), ("BERTHA", 1, 500, 900, ""), ("ANNI", 2, 300, 300, ""), ("NANO", 1, 800, 900, "")],
    );
    let [ours, bertha, theirs, nano] = placed_units(&sim)[..] else { panic!("four units") };
    let fire = |sim: &Sim, id: UnitId| sim.unit(id).unwrap().fire_state;
    assert_eq!(fire(&sim, ours), 2, "the player's placed Annihilator fires at will, as its file says");
    assert_eq!(fire(&sim, bertha), 0, "the player's placed Bertha holds its fire, as its file says");
    assert_eq!(fire(&sim, theirs), 0, "the computer player's waits for its computer player");
    let mut ai = ta_ai::SkirmishAi::new(1, &sim, ta_ai::Difficulty::Normal);
    for _ in 0..30 {
        let cmds = ai.think(&sim);
        sim.step(&cmds);
    }
    assert_eq!(fire(&sim, theirs), 2, "its computer player sets it going");
    let anni = sim.types.id("ANNI").unwrap();
    let site = FxVec3::from_ints(800, 0, 700);
    sim.step(&[Command::Order { player: 0, units: vec![nano], order: Order::Build { type_id: anni, pos: site }, queue: false }]);
    let built = (0..900)
        .find_map(|_| {
            sim.step(&[]);
            sim.units().find(|u| u.owner == 0 && u.type_id == anni && u.id != ours && u.is_complete()).map(|u| u.id)
        })
        .expect("the player's constructor finishes an Annihilator");
    assert_eq!(fire(&sim, built), 0, "one the player builds holds its fire");
    assert_eq!(fire(&sim, ours), 2, "the placed one still fires at will");
}

/// Since TA builds its `NoAutoFire` guns on hold fire, the Annihilators placed for the player in
/// Com Pattern Five ("established Annihilator energy cannons ... lure the enemy units into our
/// attack zone") and A Shortage of Water were silent until the player set them going; their
/// file says fire at will, and placed, never built, they keep it. The Big Berthas of Go With the
/// Floe and Fighting Under Fire, and Air Assault's Intimidator, hold their fire as their files
/// say.
#[test]
fn the_campaigns_placed_guns_keep_their_files_fire_orders() {
    if load("AC01.ota", "Easy").is_none() {
        eprintln!("TA install not found; skipping");
        return;
    }
    for (file, unit, fire) in [
        ("Com Pattern Five.ota", "ARMANNI", 2),
        ("A Shortage of Water.ota", "ARMANNI", 2),
        ("EXP1AC11.ota", "ARMBRTHA", 0),
        ("Fighting Under Fire.ota", "ARMBRTHA", 0),
        ("Air Assault.ota", "CORINT", 0),
    ] {
        for difficulty in ["Easy", "Medium", "Hard"] {
            let Loaded { mut sim, .. } = load(file, difficulty).unwrap_or_else(|| panic!("{file} {difficulty}"));
            let mut ais = computer_players(&sim);
            for _ in 0..30 {
                tick(&mut sim, &mut ais, &[]);
            }
            let guns: Vec<u8> = sim.units().filter(|u| u.owner == 0 && sim.unit_type(u).name.eq_ignore_ascii_case(unit)).map(|u| u.fire_state).collect();
            assert!(!guns.is_empty(), "{file} {difficulty}: the player's {unit}");
            assert!(guns.iter().all(|&f| f == fire), "{file} {difficulty}: the player's {unit} on {guns:?}, its file says {fire}");
        }
    }
}

/// A mission script's `o` gives the move order first, then the fire order, as TA's missions
/// write it: `o 1 2` (maneuver, fire at will: the units' own defaults) on the Avengers patrolling
/// the Bromid Maze; `o 0 1` (hold position, return fire) on Spider Technology's A.K.s; the
/// Motien Ramp's Intimidator returns fire, then opens fire on the player's Big Bertha. A placed
/// enemy Doomsday Machine no script runs is set firing by its computer player at once
/// (Breakthrough to Central Consciousness).
#[test]
fn mission_scripts_give_the_move_order_then_the_fire_order() {
    let state = |file: &str, unit: &str, script: &dyn Fn(&[mission::MissionCommand]) -> bool, ticks: u32| {
        let Some(Loaded { mut sim, mission, .. }) = load(file, "Medium") else { return None };
        let mut ais = computer_players(&sim);
        for _ in 0..ticks {
            tick(&mut sim, &mut ais, &[]);
        }
        // The first of the computer players' units of the type whose script matches: the one of
        // its type standing nearest where it was placed.
        let pu = mission.schema("Medium")?.units.iter().find(|pu| pu.player > 1 && pu.name.eq_ignore_ascii_case(unit) && script(&pu.script))?;
        let at = FxVec3::new(Fx::from_f32(pu.x), Fx::ZERO, Fx::from_f32(pu.z));
        let u = sim.units().filter(|u| u.owner as u32 == pu.player - 1 && sim.unit_type(u).name.eq_ignore_ascii_case(unit)).min_by_key(|u| (u.pos - at).xz().length_sq_raw())?;
        Some((u.move_state, u.fire_state))
    };
    use mission::MissionCommand::{Patrol, States, Wait};
    if load("AC10.ota", "Medium").is_none() {
        eprintln!("TA install not found; skipping");
        return;
    }
    // (`o 1 2, p 536 1708`)
    let avenger = state("AC10.ota", "CORVENG", &|s| matches!(s, [States { .. }, Patrol(..), ..]), 2).expect("AC10's Avengers");
    assert_eq!(avenger, (1, 2), "AC10's patrolling Avengers: maneuver, fire at will");
    // (`o 0 1, w 60, p 1848 987`)
    let ak = state("AC03.ota", "CORAK", &|s| matches!(s, [States { .. }, Wait(60), Patrol(1848.0, 987.0), ..]), 2).expect("AC03's A.K.s");
    assert_eq!(ak, (0, 1), "AC03's A.K.s: hold position, return fire");
    // (`o 0 1, w 3000, o 0 2, a ARMBRTHA`)
    let intimidator = state("AC19.ota", "CORINT", &|s| matches!(s, [States { .. }, Wait(3000), States { .. }, ..]), 2).expect("AC19's Intimidator");
    assert_eq!(intimidator.1, 1, "AC19's Intimidator returns fire before it opens up");
    let doomsday = state("AC24.ota", "CORDOOM", &|s| s.is_empty(), 30).expect("AC24's Doomsday Machine");
    assert_eq!(doomsday.1, 2, "AC24's Doomsday Machine is set firing by its computer player");
}
