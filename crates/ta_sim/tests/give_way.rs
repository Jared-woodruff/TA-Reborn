//! Units give way (`Sim::local_avoidance`, avoid.rs): the same on every core count, and moving
//! traffic along better than TA's queueing in the situations it is for. All units belong to one
//! player here, so nobody stops to fight.
//!
//! `cargo test --release -p ta_sim --test give_way -- --nocapture` prints the timings.

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

fn new_sim(w: &World, give_way: bool) -> Sim {
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let mut sim = Sim::new(w.types.clone(), w.data.clone(), &w.map, &w.info, 0, &players, 3);
    sim.local_avoidance = give_way;
    sim
}

/// Spawns `name` at each point facing `heading`, and orders each to its own destination.
fn convoy(sim: &mut Sim, w: &World, name: &str, at: &[(i64, i64)], heading: i32, to: &[(i64, i64)]) -> Vec<UnitId> {
    let t = w.types.id(name).unwrap();
    let mut ids = Vec::new();
    let mut cmds = Vec::new();
    for (&(x, z), &(dx, dz)) in at.iter().zip(to) {
        if let Some(id) = sim.spawn_unit(t, 0, FxVec3::from_ints(x, 0, z), heading, true) {
            ids.push(id);
            cmds.push(Command::Order { player: 0, units: vec![id], order: Order::Move(FxVec3::from_ints(dx, 0, dz)), queue: false });
        }
    }
    sim.step(&cmds);
    ids
}

/// Ticks until each unit has arrived (stopped with no orders left), up to `limit`.
fn arrivals(sim: &mut Sim, ids: &[UnitId], limit: u64) -> Vec<u64> {
    let mut done: Vec<Option<u64>> = vec![None; ids.len()];
    for tick in 1..=limit {
        sim.step(&[]);
        sim.take_events();
        for (k, &id) in ids.iter().enumerate() {
            if done[k].is_none() && sim.unit(id).is_some_and(|u| u.orders.is_empty() && u.path_goal.is_none() && u.speed == Fx::ZERO) {
                done[k] = Some(tick);
            }
        }
        if done.iter().all(|d| d.is_some()) {
            break;
        }
    }
    done.into_iter().map(|d| d.unwrap_or(limit)).collect()
}

fn mean(v: &[u64]) -> f64 {
    v.iter().sum::<u64>() as f64 / v.len().max(1) as f64
}

fn centre(w: &World) -> (i64, i64) {
    (w.map.width_px() as i64 / 2, w.map.height_px() as i64 / 2)
}

/// Two columns of eight tanks meeting head on, each going to where the other started.
fn head_on(w: &World, give_way: bool) -> Vec<u64> {
    let mut sim = new_sim(w, give_way);
    let (cx, cz) = centre(w);
    let east: Vec<(i64, i64)> = (0..8).map(|i| (cx - 120 - i * 36, cz)).collect();
    let west: Vec<(i64, i64)> = (0..8).map(|i| (cx + 120 + i * 36, cz)).collect();
    let east_to: Vec<(i64, i64)> = (0..8).map(|i| (cx + 420 + i * 36, cz)).collect();
    let west_to: Vec<(i64, i64)> = (0..8).map(|i| (cx - 420 - i * 36, cz)).collect();
    let mut ids = convoy(&mut sim, w, "ARMSTUMP", &east, 16384, &east_to);
    ids.extend(convoy(&mut sim, w, "ARMSTUMP", &west, -16384, &west_to));
    arrivals(&mut sim, &ids, 30 * 90)
}

/// Six fast tanks behind a slow one, all going the same way.
fn overtaking(w: &World, give_way: bool) -> Vec<u64> {
    let mut sim = new_sim(w, give_way);
    let (cx, cz) = centre(w);
    convoy(&mut sim, w, "ARMBULL", &[(cx - 200, cz)], 16384, &[(cx + 700, cz)]);
    let fast: Vec<(i64, i64)> = (0..6).map(|i| (cx - 260 - i * 34, cz)).collect();
    let fast_to: Vec<(i64, i64)> = (0..6).map(|i| (cx + 500 - i * 34, cz)).collect();
    let ids = convoy(&mut sim, w, "ARMFLASH", &fast, 16384, &fast_to);
    arrivals(&mut sim, &ids, 30 * 90)
}

/// A block of twenty-four converging on points round one spot from all sides.
fn converging(w: &World, give_way: bool) -> Vec<u64> {
    let mut sim = new_sim(w, give_way);
    let (cx, cz) = centre(w);
    let mut ids = Vec::new();
    for (k, &(ox, oz, heading)) in [(-400i64, 0i64, 16384i32), (400, 0, -16384), (0, -400, 0), (0, 400, 32767)].iter().enumerate() {
        let at: Vec<(i64, i64)> = (0..6).map(|i| (cx + ox + if ox == 0 { (i - 3) * 36 } else { 0 }, cz + oz + if oz == 0 { (i - 3) * 36 } else { 0 })).collect();
        // Their destinations: a grid round the middle, each group through the others.
        let to: Vec<(i64, i64)> = (0..6).map(|i| (cx - ox / 2 + ((i + k as i64) % 3 - 1) * 40, cz - oz / 2 + ((i + k as i64) / 3 % 2) * 40)).collect();
        ids.extend(convoy(&mut sim, w, "CORRAID", &at, heading, &to));
    }
    arrivals(&mut sim, &ids, 30 * 90)
}

#[test]
fn giving_way_moves_traffic_along() {
    let Some(w) = world() else { return };
    for (name, run) in [("head on", head_on as fn(&World, bool) -> Vec<u64>), ("overtaking", overtaking), ("converging", converging)] {
        let (off, on) = (run(&w, false), run(&w, true));
        let (m_off, m_on) = (mean(&off), mean(&on));
        let (worst_off, worst_on) = (*off.iter().max().unwrap(), *on.iter().max().unwrap());
        println!("{name:11}: as in TA mean {:5.1} s, last {:5.1} s | give way mean {:5.1} s, last {:5.1} s", m_off / 30.0, worst_off as f64 / 30.0, m_on / 30.0, worst_on as f64 / 30.0);
        // Never much worse than TA's own movement.
        assert!(m_on <= m_off * 1.1 + 15.0, "{name}: giving way is slower ({m_on} > {m_off})");
        assert!(worst_on < 30 * 90 || worst_off >= 30 * 90, "{name}: a unit never arrived when giving way");
    }
}

/// With the rule on, planning runs on all cores for big crowds; it must give exactly what a
/// single thread gives, tick for tick.
#[test]
fn giving_way_is_the_same_on_every_core_count() {
    let Some(w) = world() else { return };
    let (width, h) = (w.map.width_px() as i64, w.map.height_px() as i64);
    let build = |parallel: bool| {
        let mut sim = new_sim(&w, true);
        sim.parallel = parallel;
        let mut cmds = Vec::new();
        for side in 0..2i64 {
            let t = w.types.id(["ARMSTUMP", "CORRAID"][side as usize]).unwrap();
            let mut ids = Vec::new();
            for i in 0..900i64 {
                let (r, c) = (i / 30, i % 30);
                let x = width / 2 + if side == 0 { -40 - r * 22 } else { 40 + r * 22 };
                ids.extend(sim.spawn_unit(t, side as u8, FxVec3::from_ints(x, 0, h / 2 - 330 + c * 22), if side == 0 { 16384 } else { -16384 }, true));
            }
            let dest = FxVec3::from_ints(if side == 0 { width / 2 + 600 } else { width / 2 - 600 }, 0, h / 2);
            cmds.push(Command::Order { player: side as u8, units: ids, order: Order::Move(dest), queue: false });
        }
        sim.step(&cmds);
        sim
    };
    let (mut a, mut b) = (build(true), build(false));
    for tick in 0..90 {
        a.step(&[]);
        b.step(&[]);
        assert_eq!(a.checksum(), b.checksum(), "parallel and sequential giving way diverged at tick {tick}");
    }
}
