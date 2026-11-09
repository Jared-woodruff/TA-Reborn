//! Group orders as formations (the rule "modern movement", `formation.rs`): a mixed group sent
//! somewhere together forms up in rows, marches there side by side at its slowest unit's pace
//! and arrives in shape, instead of streaming down one lane, ramming the slower and piling up
//! where it was sent. Measured against TA's movement on Painted Desert (skipped without a TA
//! install).
//!
//! `cargo test --release -p ta_sim --test formations -- --nocapture` prints the figures.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitId, UnitTypes};

struct World {
    types: Arc<UnitTypes>,
    data: Arc<GameData>,
    map: TntMap,
    info: MapInfo,
}

fn world() -> Option<World> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read("maps/Painted Desert.tnt").ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Painted Desert.ota").ok()?));
    Some(World { types, data, map, info })
}

/// A game between ARM and CORE with units giving way, and group orders as formations or not.
fn new_sim(w: &World, formations: bool) -> Sim {
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let mut sim = Sim::new(w.types.clone(), w.data.clone(), &w.map, &w.info, 0, &players, 3);
    sim.local_avoidance = true;
    sim.group_formations = formations;
    sim
}

fn centre(w: &World) -> (i64, i64) {
    (w.map.width_px() as i64 / 2, w.map.height_px() as i64 / 2)
}

/// A crowd of `kinds` in turn, `n` of them, round `(x, z)` (rows of six, 40 px apart, every
/// other row shifted half a place, facing east), for player 0.
fn crowd(sim: &mut Sim, kinds: &[&str], n: usize, (x, z): (i64, i64)) -> Vec<UnitId> {
    let types: Vec<usize> = kinds.iter().map(|k| sim.types.id(k).unwrap_or_else(|| panic!("no {k}"))).collect();
    (0..n as i64)
        .filter_map(|i| {
            let (r, c) = (i / 6, i % 6);
            let at = FxVec3::from_ints(x - r * 40, 0, z - 100 + c * 40 + (r % 2) * 20);
            sim.spawn_unit(types[i as usize % types.len()], 0, at, 16384, true)
        })
        .collect()
}

/// What a march looked like, tick by tick.
#[derive(Debug, Default)]
struct March {
    /// Pairs of units overlapping (bodies as the crowd has them, 4/5 of the radius): the mean
    /// over the ticks and the most at once.
    overlaps_mean: f64,
    overlaps_most: usize,
    /// The group's width across the way it goes (px; the spread of its units across it, two
    /// standard deviations each side) at its narrowest, from a fifth of the way on.
    narrowest: f64,
    /// Ticks to the first arrival and the last.
    first: u64,
    last: u64,
    /// The closest two units stood at the end (px, between their bodies' edges).
    closest_end: f64,
}

fn body(sim: &Sim, u: &ta_sim::Unit) -> f64 {
    sim.unit_type(u).radius.to_f64() * 0.8
}

/// Sends `ids` east by `dx` px as one order and watches them for up to `limit` ticks.
fn march(sim: &mut Sim, ids: &[UnitId], dest: FxVec3, limit: u64) -> March {
    let start = ids.iter().filter_map(|&id| sim.unit(id)).map(|u| u.pos.x.to_f64()).sum::<f64>() / ids.len() as f64;
    let trip = dest.x.to_f64() - start;
    sim.step(&[Command::Order { player: 0, units: ids.to_vec(), order: Order::Move(dest), queue: false }]);
    let mut m = March { narrowest: f64::MAX, ..Default::default() };
    let (mut overlaps, mut ticks) = (0usize, 0u64);
    let mut done: Vec<Option<u64>> = vec![None; ids.len()];
    for tick in 1..=limit {
        sim.step(&[]);
        sim.take_events();
        let units: Vec<&ta_sim::Unit> = ids.iter().filter_map(|&id| sim.unit(id)).collect();
        let mut now = 0;
        for (i, a) in units.iter().enumerate() {
            for b in &units[i + 1..] {
                if a.pos.distance_xz(b.pos).to_f64() < body(sim, a) + body(sim, b) {
                    now += 1;
                }
            }
        }
        overlaps += now;
        ticks += 1;
        m.overlaps_most = m.overlaps_most.max(now);
        let mean_x = units.iter().map(|u| u.pos.x.to_f64()).sum::<f64>() / units.len() as f64;
        if (mean_x - start) / trip > 0.2 && done.iter().all(|d| d.is_none()) {
            let zs: Vec<f64> = units.iter().map(|u| u.pos.z.to_f64()).collect();
            let mz = zs.iter().sum::<f64>() / zs.len() as f64;
            let sd = (zs.iter().map(|z| (z - mz).powi(2)).sum::<f64>() / zs.len() as f64).sqrt();
            m.narrowest = m.narrowest.min(4.0 * sd);
        }
        for (k, &id) in ids.iter().enumerate() {
            if done[k].is_none() && sim.unit(id).is_some_and(|u| u.orders.is_empty() && u.path_goal.is_none() && u.speed == Fx::ZERO) {
                done[k] = Some(tick);
            }
        }
        if std::env::var("TA_TRACE").is_ok() && tick % 30 == 0 {
            for f in &sim.formations {
                println!("t {:3}s formation {} {:?}, out {} of {}, overlapping now {now}", tick / 30, f.id, f.phase, f.mates.iter().filter(|m| m.out).count(), f.mates.len());
            }
            for (i, a) in units.iter().enumerate() {
                for b in &units[i + 1..] {
                    let d = a.pos.distance_xz(b.pos).to_f64();
                    if d < body(sim, a) + body(sim, b) {
                        println!("   overlap {} ({:.0},{:.0}) speed {:.2} and {} ({:.0},{:.0}) speed {:.2}: {:.1} apart, bodies {:.1}", sim.unit_type(a).name, a.pos.x.to_f64(), a.pos.z.to_f64(), a.speed.to_f64(), sim.unit_type(b).name, b.pos.x.to_f64(), b.pos.z.to_f64(), b.speed.to_f64(), d, body(sim, a) + body(sim, b));
                    }
                }
            }
            for &id in ids.iter().take(0) {
                if let Some(u) = sim.unit(id) {
                    println!("   {:9} at ({:5.0},{:5.0}) speed {:4.2} path {:2} goal {:?} orders {:?}", sim.unit_type(u).name, u.pos.x.to_f64(), u.pos.z.to_f64(), u.speed.to_f64(), u.path.len(), u.path_goal.map(|g| (g.x.to_int(), g.z.to_int())), u.orders.front());
                }
            }
        }
        if done.iter().all(|d| d.is_some()) {
            break;
        }
    }
    m.overlaps_mean = overlaps as f64 / ticks.max(1) as f64;
    let arrived: Vec<u64> = done.iter().map(|d| d.unwrap_or(limit)).collect();
    m.first = *arrived.iter().min().unwrap();
    m.last = *arrived.iter().max().unwrap();
    let units: Vec<&ta_sim::Unit> = ids.iter().filter_map(|&id| sim.unit(id)).collect();
    m.closest_end = f64::MAX;
    for (i, a) in units.iter().enumerate() {
        for b in &units[i + 1..] {
            m.closest_end = m.closest_end.min(a.pos.distance_xz(b.pos).to_f64() - body(sim, a) - body(sim, b));
        }
    }
    m
}

const MIXED: [&str; 6] = ["ARMFLASH", "ARMSTUMP", "ARMPW", "ARMHAM", "ARMSAM", "ARMCK"];

/// Thirty mixed units (fast and slow tanks, kbots, artillery, missile trucks, constructors)
/// sent 1,400 px east as one order.
#[test]
fn a_mixed_group_marches_in_formation() {
    let Some(w) = world() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let (cx, cz) = centre(&w);
    let dest = FxVec3::from_ints(cx + 700, 0, cz);
    let mut results = Vec::new();
    for formations in [false, true] {
        let mut sim = new_sim(&w, formations);
        let ids = crowd(&mut sim, &MIXED, 30, (cx - 700, cz));
        let m = march(&mut sim, &ids, dest, 30 * 120);
        println!(
            "{:13}: overlapping pairs mean {:5.2}, most {:3} | narrowest {:6.0} px | arrived {:5.1} s to {:5.1} s | closest at the end {:5.1} px",
            if formations { "formations" } else { "as in TA" },
            m.overlaps_mean,
            m.overlaps_most,
            m.narrowest,
            m.first as f64 / 30.0,
            m.last as f64 / 30.0,
            m.closest_end
        );
        if formations {
            // Front to back: the short-ranged, the tanks, the artillery, the missile trucks and
            // the constructors (each kind's mean place along the way they went).
            let x = |name: &str| -> f64 {
                let xs: Vec<f64> = ids.iter().filter_map(|&id| sim.unit(id)).filter(|u| sim.unit_type(u).name.eq_ignore_ascii_case(name)).map(|u| u.pos.x.to_f64()).collect();
                xs.iter().sum::<f64>() / xs.len().max(1) as f64
            };
            let order: Vec<(&str, f64)> = MIXED.iter().map(|&n| (n, x(n))).collect();
            println!("               front to back: {}", order.iter().map(|(n, x)| format!("{n} {x:.0}")).collect::<Vec<_>>().join(", "));
            assert!(x("ARMFLASH") > x("ARMHAM") && x("ARMPW") > x("ARMSAM") && x("ARMSTUMP") > x("ARMCK"), "the short-ranged in front, artillery and builders behind: {order:?}");
        }
        results.push(m);
    }
    let (ta, modern) = (&results[0], &results[1]);
    assert!(modern.last < 30 * 120, "the formation never got there");
    assert!(modern.overlaps_mean <= ta.overlaps_mean / 2.0 + 0.5, "formations overlap less: {} against {}", modern.overlaps_mean, ta.overlaps_mean);
    assert!(modern.last - modern.first <= 30 * 4, "the formation arrives together ({} ticks apart)", modern.last - modern.first);
    assert!(modern.closest_end > 0.0, "nobody on top of anybody at the end ({:.1} px)", modern.closest_end);
}

/// The mixed group attack-moves into a line of Raiders and two light laser towers: it goes all
/// the way in, as one body at its constructors' pace (as in Supreme Commander), and fights it
/// out.
#[test]
fn a_formation_attack_moves_into_a_line() {
    let Some(w) = world() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let (cx, cz) = centre(&w);
    for formations in [false, true] {
        let mut sim = new_sim(&w, formations);
        let ids = crowd(&mut sim, &MIXED, 30, (cx - 700, cz));
        let raid = sim.types.id("CORRAID").unwrap();
        let llt = sim.types.id("CORLLT").unwrap();
        let mut enemy = Vec::new();
        for k in 0..8 {
            enemy.extend(sim.spawn_unit(raid, 1, FxVec3::from_ints(cx + 400, 0, cz - 210 + k * 60), -16384, true));
        }
        for dz in [-150, 150] {
            enemy.extend(sim.spawn_unit(llt, 1, FxVec3::from_ints(cx + 480, 0, cz + dz), -16384, true));
        }
        sim.step(&[Command::Order { player: 0, units: ids.clone(), order: Order::Fight(FxVec3::from_ints(cx + 700, 0, cz)), queue: false }]);
        let mut cleared = None;
        for tick in 1..=30 * 180u64 {
            sim.step(&[]);
            sim.take_events();
            if std::env::var("TA_TRACE").is_ok() && tick % 90 == 0 {
                let alive = enemy.iter().filter(|&&e| sim.unit(e).is_some_and(|u| !u.dead)).count();
                println!("{} t {:3}s: enemy left {alive}; formations {:?}", if formations { "formations" } else { "as in TA" }, tick / 30, sim.formations.iter().map(|f| (f.id, f.phase, f.mates.iter().filter(|m| m.out).count(), f.mates.len())).collect::<Vec<_>>());
                for &id in ids.iter().step_by(5) {
                    if let Some(u) = sim.unit(id) {
                        println!("   {:9} x {:5.0} speed {:4.2} goal {:?} order {:?} target {:?}", sim.unit_type(u).name, u.pos.x.to_f64(), u.speed.to_f64(), u.path_goal.map(|g| g.x.to_int()), u.orders.front().map(|o| format!("{o:?}").chars().take(30).collect::<String>()), u.weapons[0].target);
                    }
                }
            }
            if cleared.is_none() && enemy.iter().all(|&e| sim.unit(e).is_none_or(|u| u.dead)) {
                cleared = Some(tick);
                break;
            }
        }
        let ours = ids.iter().filter(|&&e| sim.unit(e).is_some_and(|u| !u.dead)).count();
        println!("{:13}: the enemy's 10 gone at {:?} s, {ours} of ours left", if formations { "formations" } else { "as in TA" }, cleared.map(|t| t / 30));
        if formations {
            assert!(cleared.is_some(), "the formation fought its way in");
        }
    }
}

/// Formations are planned and paced the same on one core and on many, tick for tick, in a big
/// battle (900 a side sent at each other as two group orders, rows for each side's first 300
/// and the shape they stand in for the rest).
#[test]
fn formations_are_the_same_on_every_core_count() {
    let Some(w) = world() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let (width, h) = (w.map.width_px() as i64, w.map.height_px() as i64);
    let build = |parallel: bool| {
        let mut sim = new_sim(&w, true);
        sim.parallel = parallel;
        let mut cmds = Vec::new();
        for side in 0..2i64 {
            let kinds: [&str; 3] = if side == 0 { ["ARMSTUMP", "ARMFLASH", "ARMHAM"] } else { ["CORRAID", "CORAK", "CORTHUD"] };
            let types: Vec<usize> = kinds.iter().map(|k| w.types.id(k).unwrap()).collect();
            let mut ids = Vec::new();
            for i in 0..900i64 {
                let (r, c) = (i / 30, i % 30);
                let x = width / 2 + if side == 0 { -200 - r * 24 } else { 200 + r * 24 };
                ids.extend(sim.spawn_unit(types[i as usize % 3], side as u8, FxVec3::from_ints(x, 0, h / 2 - 360 + c * 24), if side == 0 { 16384 } else { -16384 }, true));
            }
            let dest = FxVec3::from_ints(if side == 0 { width / 2 + 500 } else { width / 2 - 500 }, 0, h / 2);
            cmds.push(Command::Order { player: side as u8, units: ids[..300].to_vec(), order: Order::Fight(dest), queue: false });
            cmds.push(Command::Order { player: side as u8, units: ids[300..].to_vec(), order: Order::Fight(dest), queue: false });
        }
        sim.step(&cmds);
        sim
    };
    let (mut a, mut b) = (build(true), build(false));
    let started = std::time::Instant::now();
    for tick in 0..300 {
        a.step(&[]);
        b.step(&[]);
        assert_eq!(a.checksum(), b.checksum(), "parallel and sequential formations diverged at tick {tick}");
    }
    println!("1,800 units in four formations: 300 ticks twice in {:.2} s; {} formations", started.elapsed().as_secs_f64(), a.formations.len());
}

/// Two group moves, the second queued and turning: two formations, the second laid out from where
/// the first ends (so that the ways from one to the other hardly cross), and every unit gets to
/// the end of both.
#[test]
fn queued_group_moves_march_leg_after_leg() {
    let Some(w) = world() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let (cx, cz) = centre(&w);
    let mut sim = new_sim(&w, true);
    let ids = crowd(&mut sim, &["ARMSTUMP", "ARMPW"], 12, (cx - 500, cz));
    let first = FxVec3::from_ints(cx, 0, cz);
    let second = FxVec3::from_ints(cx, 0, cz + 500);
    sim.step(&[
        Command::Order { player: 0, units: ids.clone(), order: Order::Move(first), queue: false },
        Command::Order { player: 0, units: ids.clone(), order: Order::Move(second), queue: true },
    ]);
    assert_eq!(sim.formations.len(), 2, "a formation for each leg");
    let legs: Vec<Vec<FxVec3>> = sim.formations.iter().map(|f| f.mates.iter().map(|m| m.spot).collect()).collect();
    for tick in 0..30 * 120 {
        sim.step(&[]);
        sim.take_events();
        if ids.iter().all(|&id| sim.unit(id).is_some_and(|u| u.orders.is_empty() && u.path_goal.is_none())) {
            println!("both legs marched in {:.1} s", tick as f64 / 30.0);
            break;
        }
    }
    for &id in &ids {
        let u = sim.unit(id).unwrap();
        assert!(u.orders.is_empty(), "{} still has orders: {:?}", sim.unit_type(u).name, u.orders);
        let end = legs[1].iter().map(|s| s.distance_xz(u.pos)).min().unwrap();
        assert!(end < Fx::from_int(40), "{} ended {:.0} px from any place of the second leg", sim.unit_type(u).name, end.to_f64());
    }
    // The ways from the first leg's places to the second's hardly cross (each unit's place in the
    // second laid out from where it stands at the end of the first).
    let cross = |a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)| {
        let side = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0);
        side(a, b, c) * side(a, b, d) < 0.0 && side(c, d, a) * side(c, d, b) < 0.0
    };
    let xz = |p: &FxVec3| (p.x.to_f64(), p.z.to_f64());
    // (Units of a kind; the kinds change rows as the group turns, the Peewees taking the front
    // whichever way it faces, and some cross for it.)
    let kind = |k: usize| sim.unit(ids[k]).map(|u| u.type_id);
    let (mut crossings, mut pairs, mut all) = (0, 0, 0);
    for i in 0..legs[0].len() {
        for j in i + 1..legs[0].len() {
            let crossed = cross(xz(&legs[0][i]), xz(&legs[1][i]), xz(&legs[0][j]), xz(&legs[1][j]));
            all += crossed as usize;
            if kind(i) == kind(j) {
                pairs += 1;
                crossings += crossed as usize;
            }
        }
    }
    println!("ways from the first leg to the second crossing: {crossings} of {pairs} pairs of a kind ({all} in all)");
    assert!(crossings * 20 <= pairs, "the ways of units of a kind from one leg to the next cross ({crossings} of {pairs} pairs)");
}
