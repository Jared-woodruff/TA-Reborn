//! Boom transports (hovercraft and ships) swing their boom towards the unit they pick up and
//! set cargo down where the unload order says, on every side (skipped without a TA install).

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, SimEvent, UnitTypes};

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

/// Dry, nearly flat ground around a point: every 16 px within `r` varies by 10 px at most.
fn flat_area(sim: &Sim, r: i64) -> Option<FxVec3> {
    let (w, h) = sim.world.size_px();
    let sea = sim.world.sea_level;
    let ht = |x: i64, z: i64| sim.world.height_at(Fx::from_int(x), Fx::from_int(z));
    for z in (r + 64..h.to_int() - r - 64).step_by(48) {
        for x in (r + 64..w.to_int() - r - 64).step_by(48) {
            let hs: Vec<Fx> = (-r..=r).step_by(16).flat_map(|dz| (-r..=r).step_by(16).map(move |dx| (dx, dz))).map(|(dx, dz)| ht(x + dx, z + dz)).collect();
            let (lo, hi) = (*hs.iter().min()?, *hs.iter().max()?);
            if lo > sea + Fx::from_int(8) && hi - lo <= Fx::from_int(10) {
                return Some(FxVec3::from_ints(x, 0, z));
            }
        }
    }
    None
}

/// A hovercraft on flat ground takes a tank aboard from ahead of it, then is told to set it
/// down 80 px away in direction `offset`: how far from that point did the boom put it, and
/// where was the boom's hook (relative to the transport) when it took the tank?
fn hover_drop(offset: (i64, i64)) -> Option<(FxVec3, FxVec3)> {
    let mut sim = sim_on("Painted Desert")?;
    let at = flat_area(&sim, 128).expect("flat ground");
    let (hover_t, tank_t) = (sim.types.id("ARMTHOVR")?, sim.types.id("ARMFLASH")?);
    let hover = sim.spawn_unit(hover_t, 0, at, 0, true)?;
    let tank = sim.spawn_unit(tank_t, 0, at + FxVec3::from_ints(0, 0, -70), 0, true)?;
    sim.step(&[Command::Order { player: 0, units: vec![hover], order: Order::Load(tank), queue: false }]);
    let mut hook = None;
    for _ in 0..30 * 40 {
        sim.step(&[]);
        if sim.take_events().iter().any(|e| matches!(e, SimEvent::UnitLoaded { unit, .. } if *unit == tank)) {
            // The model's `link` piece hangs from the end of the boom.
            let u = sim.unit(hover)?;
            hook = Some(ta_sim::unit::piece_world_position(u, sim.unit_type(u), u.cob.as_ref(), 8) - u.pos);
            break;
        }
    }
    let hook = hook.expect("the hovercraft should take the tank aboard");
    for _ in 0..30 {
        sim.step(&[]);
    }
    let dest = sim.unit(hover)?.pos + FxVec3::from_ints(offset.0, 0, offset.1);
    sim.step(&[Command::Order { player: 0, units: vec![hover], order: Order::Unload(dest), queue: false }]);
    for _ in 0..30 * 40 {
        sim.step(&[]);
        if sim.take_events().iter().any(|e| matches!(e, SimEvent::UnitUnloaded { unit, .. } if *unit == tank)) {
            break;
        }
    }
    let t = sim.unit(tank)?;
    assert!(t.carried_by.is_none(), "the tank should be set down");
    Some((t.pos - dest, hook))
}

#[test]
fn hover_transport_sets_cargo_down_where_ordered() {
    for offset in [(0, 80), (80, 0), (0, -80), (-80, 0)] {
        let Some((off, hook)) = hover_drop(offset) else {
            eprintln!("TA install not found; skipping");
            return;
        };
        println!("drop {offset:?}: {:.0} px off; hook at ({}, {}) when loading", off.length_xz().to_f32(), hook.x.to_int(), hook.z.to_int());
        assert!(off.length_xz() < Fx::from_int(32), "set down {:.0} px from where it was ordered ({offset:?})", off.length_xz().to_f32());
        // The tank stood north (-z) of the hovercraft: the boom reached that way for it.
        assert!(hook.z < Fx::from_int(-20), "the boom should reach towards the tank it picks up, not away");
    }
}
