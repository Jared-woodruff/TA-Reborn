//! Cheats against the real game data (skipped without a TA install).

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::mission::MissionRules;
use ta_sim::{Cheat, Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitId, UnitTypes};

fn sim_on(map_name: &str) -> Option<Sim> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map_name}.ota")).ok()?));
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup {
            side: if i == 0 { "ARM".into() } else { "CORE".into() },
            color: i as u8,
            start: FxVec3::ZERO,
            metal: Fx::from_int(1000),
            energy: Fx::from_int(1000),
            ai: false,
        })
        .collect();
    let mut sim = Sim::new(types, data, &map, &info, 0, &players, 99);
    sim.cheats_allowed = true;
    Some(sim)
}

/// Dry, nearly flat ground: an east-west strip varying by at most 3 px.
fn flat_spot(sim: &Sim, len: i64) -> Option<FxVec3> {
    let (w, h) = sim.world.size_px();
    let sea = sim.world.sea_level;
    for z in (64..h.to_int() - 64).step_by(32) {
        for x in (64..w.to_int() - len - 64).step_by(32) {
            let hs: Vec<Fx> = (0..=len).step_by(16).map(|dx| sim.world.height_at(Fx::from_int(x + dx), Fx::from_int(z))).collect();
            let (lo, hi) = (hs.iter().min()?, hs.iter().max()?);
            if *lo > sea && *hi - *lo <= Fx::from_int(3) {
                return Some(FxVec3::from_ints(x + 20, 0, z));
            }
        }
    }
    None
}

fn cheat(sim: &mut Sim, player: u8, cheat: Cheat) {
    sim.step(&[Command::Cheat { player, cheat }]);
}

fn run(sim: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        sim.step(&[]);
        sim.take_events();
    }
}

/// Health a player-0 metal storage loses to a player-1 AK firing at it for ten seconds, with
/// the cheats `setup` gives first.
fn storage_under_fire(setup: &[(u8, Cheat)]) -> Option<(Fx, Fx, Sim, UnitId)> {
    let mut sim = sim_on("Coast To Coast")?;
    let a = flat_spot(&sim, 200)?;
    let storage = sim.spawn_unit(sim.types.id("ARMMSTOR")?, 0, a, 0, true)?;
    let ak = sim.spawn_unit(sim.types.id("CORAK")?, 1, a + FxVec3::from_ints(110, 0, 0), -16384, true)?;
    for (player, c) in setup {
        cheat(&mut sim, *player, c.clone());
    }
    sim.step(&[Command::Order { player: 1, units: vec![ak], order: Order::Attack(ta_sim::Target::Unit(storage)), queue: false }]);
    run(&mut sim, 300);
    let max = sim.unit_type(sim.unit(storage)?).max_health;
    let health = sim.unit(storage)?.health;
    Some((max - health, max, sim, storage))
}

#[test]
fn double_and_half_damage_and_god_mode() {
    let Some((normal, max, _, _)) = storage_under_fire(&[]) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    assert!(normal > Fx::ZERO, "the AK hurts the storage");
    let (double, _, _, _) = storage_under_fire(&[(1, Cheat::Damage(1))]).unwrap();
    let (half, _, _, _) = storage_under_fire(&[(1, Cheat::Damage(-1))]).unwrap();
    println!("damage in 10 s: normal {normal:?}, double {double:?}, half {half:?} (of {max:?})");
    assert!(double < max, "the storage outlives the double-damage run");
    let near = |a: Fx, b: Fx| (a - b).abs() <= b / 50 + Fx::ONE;
    assert!(near(double, normal * 2), "+doubleshot doubles the damage");
    assert!(near(half, normal / 2), "+halfshot halves it");

    let (god, _, mut sim, storage) = storage_under_fire(&[(0, Cheat::Invulnerable(true))]).unwrap();
    assert_eq!(god, Fx::ZERO, "god mode: no harm");
    assert!(sim.players[0].cheats.used && !sim.players[1].cheats.used);
    // Off again, it takes damage; healed, it is whole.
    cheat(&mut sim, 0, Cheat::Invulnerable(false));
    run(&mut sim, 150);
    let max = sim.unit_type(sim.unit(storage).unwrap()).max_health;
    assert!(sim.unit(storage).unwrap().health < max);
    cheat(&mut sim, 0, Cheat::Heal(vec![storage]));
    let h = sim.unit(storage).unwrap().health;
    assert!(h > max - Fx::from_int(20), "healed ({h:?} of {max:?}; the AK keeps firing)");
    // Its own blast is not stopped by god mode.
    cheat(&mut sim, 0, Cheat::Invulnerable(true));
    sim.step(&[Command::SelfDestruct { player: 0, units: vec![storage] }]);
    run(&mut sim, 200);
    assert!(sim.unit(storage).is_none_or(|u| u.dead), "a self-destruct still goes off");
}

#[test]
fn cheats_are_ignored_unless_allowed() {
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    // (A player with nothing left is defeated, and cheats no more.)
    let a = flat_spot(&sim, 200).unwrap();
    sim.spawn_unit(sim.types.id("ARMCOM").unwrap(), 0, a, 0, true).unwrap();
    sim.cheats_allowed = false;
    sim.players[0].economy.metal = Fx::from_int(10);
    cheat(&mut sim, 0, Cheat::Resources);
    cheat(&mut sim, 0, Cheat::Invulnerable(true));
    assert!(sim.players[0].economy.metal < Fx::from_int(100), "no free metal");
    assert!(!sim.players[0].cheats.invulnerable && !sim.players[0].cheats.used);
    sim.cheats_allowed = true;
    cheat(&mut sim, 0, Cheat::Resources);
    let e = &sim.players[0].economy;
    assert_eq!(e.metal, e.metal_capacity, "+atm fills the storage");
    assert_eq!(e.energy, e.energy_capacity);
    assert!(sim.players[0].cheats.used);
}

#[test]
fn instant_build_and_unlimited_resources() {
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 200).unwrap();
    let com = sim.spawn_unit(sim.types.id("ARMCOM").unwrap(), 0, a, 0, true).unwrap();
    let solar = sim.types.id("ARMSOLAR").unwrap();
    let build = |sim: &Sim, dx: i64| Command::Order {
        player: 0,
        units: vec![com],
        order: Order::Build { type_id: solar, pos: sim.snap_to_grid(solar, a + FxVec3::from_ints(dx, 0, 80)) },
        queue: false,
    };
    let finished = |sim: &Sim| sim.units().filter(|u| u.type_id == solar && u.is_complete() && !u.dead).count();

    // Normally a solar collector takes the Commander a while.
    let order = build(&sim, 0);
    sim.step(&[order]);
    run(&mut sim, 90);
    assert_eq!(finished(&sim), 0, "three seconds are not enough without the cheat");

    // With the cheat the next one is up as soon as the Commander starts, and costs nothing.
    cheat(&mut sim, 0, Cheat::InstantBuild(true));
    let metal = sim.players[0].economy.metal;
    let order = build(&sim, 96);
    sim.step(&[order]);
    let mut ticks = 0;
    while finished(&sim) < 1 && ticks < 150 {
        run(&mut sim, 1);
        ticks += 1;
    }
    assert!(finished(&sim) >= 1, "instant build finishes it");
    assert!(sim.players[0].economy.metal >= metal, "for nothing");

    cheat(&mut sim, 0, Cheat::Unlimited(true));
    run(&mut sim, 3);
    let e = &sim.players[0].economy;
    assert_eq!((e.metal, e.energy), (e.metal_capacity, e.energy_capacity), "unlimited: storage stays full");
}

#[test]
fn spawn_promote_destroy_unlock_and_win() {
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 200).unwrap();
    sim.spawn_unit(sim.types.id("ARMCOM").unwrap(), 0, a, 0, true).unwrap();
    let ak = sim.types.id("CORAK").unwrap();
    let solar = sim.types.id("CORSOLAR").unwrap();
    let there = a + FxVec3::from_ints(400, 0, 0);
    cheat(&mut sim, 0, Cheat::Spawn { type_id: ak, owner: 1, pos: there, count: 7 });
    cheat(&mut sim, 0, Cheat::Spawn { type_id: solar, owner: 1, pos: there + FxVec3::from_ints(0, 0, 200), count: 3 });
    let aks: Vec<UnitId> = sim.units().filter(|u| u.type_id == ak && u.owner == 1).map(|u| u.id).collect();
    assert_eq!(aks.len(), 7, "seven AKs for the enemy");
    assert_eq!(sim.units().filter(|u| u.type_id == solar && u.is_complete()).count(), 3, "three finished solar collectors");
    for &id in &aks {
        let p = sim.unit(id).unwrap().pos;
        assert!(p.within_xz(there, Fx::from_int(200)), "placed around the spot");
    }

    cheat(&mut sim, 0, Cheat::Promote(aks[..2].to_vec()));
    assert!(aks[..2].iter().all(|&id| sim.unit(id).unwrap().rank() == 3), "promoted to elite");
    assert_eq!(sim.unit(aks[2]).unwrap().rank(), 0);

    cheat(&mut sim, 0, Cheat::Destroy(aks[..3].to_vec()));
    run(&mut sim, 2);
    assert!(aks[..3].iter().all(|&id| sim.unit(id).is_none_or(|u| u.dead)), "destroyed");
    assert!(sim.unit(aks[3]).is_some_and(|u| !u.dead));

    sim.allowed = Some(vec![false; sim.types.types.len()]);
    assert!(!sim.is_allowed(solar));
    cheat(&mut sim, 0, Cheat::Unlock);
    assert!(sim.is_allowed(solar), "restrictions lifted");

    sim.mission = Some(MissionRules { human: 0, ..Default::default() });
    cheat(&mut sim, 0, Cheat::Win);
    assert!(sim.players[1].defeated && !sim.players[0].defeated);
    assert_eq!(sim.mission.as_ref().unwrap().outcome, Some(true), "the mission is won");
    run(&mut sim, 200);
    assert_eq!(sim.units().filter(|u| u.owner == 1 && !u.dead).count(), 0, "the enemy's forces blow up");
    assert!(sim.units().any(|u| u.owner == 0 && !u.dead));
}
