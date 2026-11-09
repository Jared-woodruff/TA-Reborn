//! Air transports setting their cargo down on open ground: one alone, and several sent to the
//! same place, as a player sends them (`TA_TRACE=1` prints what they do).

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, SimEvent, UnitId, UnitTypes};

fn sim_on(map_name: &str) -> Option<Sim> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map_name}.ota")).ok()?));
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: if i == 0 { "ARM".into() } else { "CORE".into() }, color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    Some(Sim::new(types, data, &map, &info, 0, &players, 99))
}

/// The ground at `(x, z)` (px).
fn ground(sim: &Sim, x: i64, z: i64) -> FxVec3 {
    FxVec3::new(Fx::from_int(x), sim.world.height_at(Fx::from_int(x), Fx::from_int(z)), Fx::from_int(z))
}

/// Whether the ground for `r` px round `(x, z)` is dry and level (within 12 px).
fn open_ground(sim: &Sim, x: i64, z: i64, r: i64) -> bool {
    let sea = sim.world.sea_level;
    let mid = sim.world.height_at(Fx::from_int(x), Fx::from_int(z));
    (-r..=r).step_by(16).all(|dz| {
        (-r..=r).step_by(16).all(|dx| {
            let h = sim.world.height_at(Fx::from_int(x + dx), Fx::from_int(z + dz));
            h > sea + Fx::from_int(4) && (h - mid).abs() <= Fx::from_int(12)
        })
    })
}

/// Two places of open ground at least `apart` px apart: where the transports start, and where
/// they set their cargo down.
fn two_places(sim: &Sim, apart: i64) -> Option<((i64, i64), (i64, i64))> {
    let (w, h) = (sim.world.width as i64 * 16, sim.world.height as i64 * 16);
    let mut open = Vec::new();
    for z in (200..h - 200).step_by(64) {
        for x in (200..w - 200).step_by(64) {
            if open_ground(sim, x, z, 112) {
                open.push((x, z));
            }
        }
    }
    let far = |a: (i64, i64), b: (i64, i64)| (a.0 - b.0).pow(2) + (a.1 - b.1).pow(2) >= apart * apart;
    open.iter().find_map(|&a| open.iter().find(|&&b| far(a, b)).map(|&b| (a, b)))
}

/// `n` of `transport`, each loaded with a `cargo`, then all told at once to set it down at the
/// same place: whether every one did within a minute, and none failed.
fn drop_together(sim: &mut Sim, transport: &str, cargo: &str, n: usize, apart: i64) -> Result<f32, String> {
    let trace = std::env::var("TA_TRACE").is_ok();
    let id = |sim: &Sim, name: &str| sim.types.id(name).unwrap();
    let ((sx, sz), (dx, dz)) = two_places(sim, apart).ok_or("no open ground")?;
    let mut pairs: Vec<(UnitId, UnitId)> = Vec::new();
    for k in 0..n as i64 {
        let p = ground(sim, sx, sz + 64 * k - 64);
        let t = sim.spawn_unit(id(sim, transport), 1, p, 0, true).unwrap();
        let c = sim.spawn_unit(id(sim, cargo), 1, p + FxVec3::from_ints(50, 0, 0), 0, true).unwrap();
        sim.step(&[Command::Order { player: 1, units: vec![t], order: Order::Load(c), queue: false }]);
        pairs.push((t, c));
    }
    for _ in 0..30 * 40 {
        sim.step(&[]);
        if pairs.iter().all(|&(t, c)| sim.unit(c).is_some_and(|c| c.carried_by == Some(t))) {
            break;
        }
    }
    if !pairs.iter().all(|&(t, c)| sim.unit(c).is_some_and(|c| c.carried_by == Some(t))) {
        return Err(format!("{transport}: not all loaded"));
    }
    let dest = ground(sim, dx, dz);
    let units: Vec<UnitId> = pairs.iter().map(|&(t, _)| t).collect();
    sim.step(&[Command::Order { player: 1, units: units.clone(), order: Order::Unload(dest), queue: false }]);
    for tick in 1..=30 * 60u32 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::OrderFailed { unit, reason } = e && units.contains(&unit) {
                return Err(format!("{n} x {transport}: one failed at tick {tick}: {reason}"));
            }
        }
        if trace && tick % 15 == 0 {
            for &(t, _) in &pairs {
                let u = sim.unit(t).unwrap();
                println!(
                    "  {:.1} s {:?}: {:>5.0} off the place, {:>4.0} up, speed {:.2}, cargo {}, orders {}",
                    tick as f32 / 30.0, t.slot, (u.pos - dest).xz().length_xz().to_f32(), (u.pos.y - sim.world.height_at(u.pos.x, u.pos.z)).to_f32(), u.speed.to_f32(), u.cargo.len(), u.orders.len()
                );
            }
        }
        if pairs.iter().all(|&(t, _)| sim.unit(t).is_some_and(|u| u.cargo.is_empty() && u.orders.is_empty())) {
            // Every passenger on the ground near the place, standing.
            for &(_, c) in &pairs {
                let c = sim.unit(c).unwrap();
                let off = (c.pos - dest).xz().length_xz().to_f32();
                if c.carried_by.is_some() || off > 6.0 * 32.0 {
                    return Err(format!("{n} x {transport}: a passenger was set down {off:.0} px off"));
                }
            }
            return Ok(tick as f32 / 30.0);
        }
    }
    let left = pairs.iter().filter(|&&(t, _)| sim.unit(t).is_some_and(|u| !u.cargo.is_empty())).count();
    Err(format!("{n} x {transport}: {left} still carrying after a minute"))
}

/// The transports' own flight: TA's take speed up slowly and brake hard (CORVALK: 0.02 and 4 a
/// tick), and arriving over a place they meant to set down at they overshot it at full speed and
/// swung back, over and over, never low enough to let go (wider than the 10 px they let go
/// within, for ever): the cargo hung under a transport circling the place.
#[test]
fn air_transports_set_their_cargo_down_where_they_are_sent() {
    for (transport, cargo) in [("CORVALK", "CORRAID"), ("ARMATLAS", "ARMSTUMP")] {
        // (From far enough to come at full speed.)
        for (n, apart) in [(1usize, 1600), (3, 1600)] {
            let Some(mut sim) = sim_on("Painted Desert") else {
                eprintln!("TA install not found; skipping");
                return;
            };
            match drop_together(&mut sim, transport, cargo, n, apart) {
                Ok(seconds) => println!("{n} x {transport} with {cargo}, from {apart} px: all set down after {seconds:.1} s"),
                Err(e) => panic!("{e} (from {apart} px)"),
            }
        }
    }
}
