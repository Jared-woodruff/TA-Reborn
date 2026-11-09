//! Units bunched up shook: every tick the crowd's separation (crowd.rs) pushed them apart and a
//! mover's own step (`Sim::ground_step`) took it back in, so they went to and fro and turned back
//! and forth on the spot. Measured here as the way a unit goes and comes back (its path less how
//! far it got) and the turning it undoes, in a crowd gathered at one spot and in a horde pressing
//! into a line that holds, against the real game data (skipped without a TA install).
//!
//! `cargo test --release -p ta_sim --test crowd_jitter -- --nocapture` prints the figures.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitId, UnitTypes};

const TICKS: u64 = 30;

/// A game on Painted Desert between ARM (player 0) and CORE (player 1), no units, and the
/// first start position; None without an install.
fn game() -> Option<(Sim, FxVec3)> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let tnt = TntMap::parse(&vfs.read("maps/Painted Desert.tnt").ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Painted Desert.ota").ok()?));
    let (_, x, z) = info.schemas[0].start_positions[0];
    let start = FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z));
    let players: Vec<PlayerSetup> = ["ARM", "CORE"]
        .iter()
        .enumerate()
        .map(|(i, side)| PlayerSetup { side: (*side).into(), color: i as u8, start, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let mut sim = Sim::new(types, data, &tnt, &info, 0, &players, 77);
    sim.local_avoidance = std::env::var("TA_TEST_GIVE_WAY").map_or(true, |v| v != "0");
    Some((sim, start))
}

/// Spawns `rows` x `cols` of the kinds in turn, `spacing` apart from `at`, for `player`.
fn block(sim: &mut Sim, player: u8, kinds: &[&str], at: FxVec3, rows: i64, cols: i64, spacing: i64, heading: i32) -> Vec<UnitId> {
    let types: Vec<usize> = kinds.iter().map(|k| sim.types.id(k).unwrap_or_else(|| panic!("no {k}"))).collect();
    let mut ids = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let t = types[(r * cols + c) as usize % types.len()];
            if let Some(id) = sim.spawn_unit(t, player, at + FxVec3::from_ints(c * spacing, 0, r * spacing), heading, true) {
                ids.push(id);
            }
        }
    }
    ids
}

/// To-and-fro over `ticks`: per unit, the way it went less how far it got (px a second) and
/// the turning it undid (degrees a second); the mean and the 90th percentile of each.
#[derive(Debug)]
struct Jitter {
    shake: f64,
    shake_p90: f64,
    turn: f64,
    turn_p90: f64,
}

fn measure(sim: &mut Sim, ids: &[UnitId], ticks: u64) -> Jitter {
    let at = |sim: &Sim, id: UnitId| sim.unit(id).map(|u| (u.pos.x.to_f32() as f64, u.pos.z.to_f32() as f64, u.heading));
    let first: Vec<_> = ids.iter().map(|&id| at(sim, id)).collect();
    let mut last = first.clone();
    let mut path = vec![0.0f64; ids.len()];
    let mut turned = vec![0i64; ids.len()];
    let mut net_turn = vec![0i64; ids.len()];
    for _ in 0..ticks {
        sim.step(&[]);
        sim.take_events();
        for (k, &id) in ids.iter().enumerate() {
            let (Some((x0, z0, h0)), Some((x, z, h))) = (last[k], at(sim, id)) else {
                last[k] = None;
                continue;
            };
            path[k] += ((x - x0).powi(2) + (z - z0).powi(2)).sqrt();
            let dh = ta_sim::fixed::wrap_angle(h - h0) as i64;
            turned[k] += dh.abs();
            net_turn[k] += dh;
            last[k] = Some((x, z, h));
        }
    }
    let seconds = ticks as f64 / TICKS as f64;
    let degrees = 360.0 / ta_sim::fixed::FULL_TURN as f64;
    let (mut shakes, mut turns) = (Vec::new(), Vec::new());
    for k in 0..ids.len() {
        let (Some((x0, z0, _)), Some((x, z, _))) = (first[k], last[k]) else { continue };
        let net = ((x - x0).powi(2) + (z - z0).powi(2)).sqrt();
        shakes.push((path[k] - net).max(0.0) / seconds);
        turns.push((turned[k] - net_turn[k].abs()) as f64 * degrees / seconds);
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let p90 = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.get(v.len() * 9 / 10).copied().unwrap_or(0.0)
    };
    let (shake, turn) = (mean(&shakes), mean(&turns));
    Jitter { shake, shake_p90: p90(&mut shakes), turn, turn_p90: p90(&mut turns) }
}

fn run(sim: &mut Sim, ticks: u64) {
    for _ in 0..ticks {
        sim.step(&[]);
        sim.take_events();
    }
}

/// 180 tanks and kbots sent to one spot: pressing into the crowd there until they count as
/// arrived, then standing in it.
fn gathered() -> Option<(Jitter, Jitter)> {
    let (mut sim, start) = game()?;
    let ids = block(&mut sim, 0, &["ARMSTUMP", "ARMPW", "ARMBULL", "ARMZEUS"], start, 12, 15, 40, 0);
    let spot = start + FxVec3::from_ints(300, 0, 900);
    sim.step(&[Command::Order { player: 0, units: ids.clone(), order: Order::Move(spot), queue: false }]);
    run(&mut sim, 12 * TICKS);
    let pressing = measure(&mut sim, &ids, 6 * TICKS);
    run(&mut sim, 20 * TICKS);
    let standing = measure(&mut sim, &ids, 10 * TICKS);
    Some((pressing, standing))
}

/// 160 of the Arm's sent on through a line of 50 of the Core's holding their ground (neither
/// side firing): the front presses into the line and the rest into the front, as a horde at a
/// battle's front does.
fn pressed() -> Option<Jitter> {
    let (mut sim, start) = game()?;
    let line = block(&mut sim, 1, &["CORRAID", "CORAK"], start + FxVec3::from_ints(-600, 0, 700), 1, 50, 36, 32768);
    let horde = block(&mut sim, 0, &["ARMSTUMP", "ARMPW", "ARMBULL"], start + FxVec3::from_ints(0, 0, 200), 10, 16, 40, 0);
    let goal = start + FxVec3::from_ints(300, 0, 1600);
    sim.step(&[
        Command::SetStates { player: 1, units: line.clone(), fire: Some(0), movement: Some(0) },
        Command::SetStates { player: 0, units: horde.clone(), fire: Some(0), movement: None },
    ]);
    sim.step(&[Command::Order { player: 0, units: horde.clone(), order: Order::Move(goal), queue: false }]);
    run(&mut sim, 20 * TICKS);
    Some(measure(&mut sim, &horde, 10 * TICKS))
}

/// Before: pressing in at the spot units went to and fro 12.5 px a second and turned back and
/// forth 31 degrees a second, standing there they shook 4.7 px a second, and against the line
/// 8.3 px and 17 degrees; now about 2 px and 12 degrees, 0.2 px, and 1.3 px and 7 degrees.
#[test]
fn crowds_hold_still() {
    let Some((pressing, standing)) = gathered() else { return };
    let Some(press) = pressed() else { return };
    println!("gathered at a spot, pressing in: {pressing:.2?}");
    println!("gathered at a spot, standing:    {standing:.2?}");
    println!("pressed into a line that holds:  {press:.2?}");
    assert!(pressing.shake < 4.0 && pressing.turn < 20.0, "pressing in at the spot: {pressing:.2?}");
    assert!(standing.shake < 1.0 && standing.turn < 1.0, "standing at the spot: {standing:.2?}");
    assert!(press.shake < 3.0 && press.turn < 12.0, "against the line: {press:.2?}");
}

/// (Looking into it: a few units of the gathered crowd tick by tick.)
#[test]
#[ignore]
fn trace_a_crowd() {
    let Some((mut sim, start)) = game() else { return };
    let ids = block(&mut sim, 0, &["ARMSTUMP", "ARMPW", "ARMBULL", "ARMZEUS"], start, 12, 15, 40, 0);
    let spot = start + FxVec3::from_ints(300, 0, 900);
    sim.step(&[Command::Order { player: 0, units: ids.clone(), order: Order::Move(spot), queue: false }]);
    let watch = [ids[0], ids[50], ids[95], ids[170]];
    for phase in [12 * TICKS, 38 * TICKS] {
        while sim.tick < phase {
            sim.step(&[]);
            sim.take_events();
        }
        for &id in &watch {
            let name = { let u = sim.unit(id).unwrap(); sim.unit_type(u).name.clone() };
            println!("-- {name} {id:?} from tick {}", sim.tick);
            for _ in 0..12 {
                let before = sim.unit(id).map(|u| (u.pos, u.heading)).unwrap();
                sim.step(&[]);
                sim.take_events();
                let u = sim.unit(id).unwrap();
                let d = u.pos - before.0;
                println!(
                    "  tick {:5} step ({:6.2}, {:6.2}) heading {:7.1} (turned {:6.2}) speed {:5.2} goal {} jostled {} approach_idle {}",
                    sim.tick,
                    d.x.to_f32(),
                    d.z.to_f32(),
                    u.heading as f32 * 360.0 / 65536.0,
                    ta_sim::fixed::wrap_angle(u.heading - before.1) as f32 * 360.0 / 65536.0,
                    u.speed.to_f32(),
                    u.path_goal.is_some(),
                    u.jostled_tick + 1 >= sim.tick,
                    u.approach_idle
                );
            }
        }
    }
}
