//! Bombers (skipped without a TA install): bombs and shells don't hit aircraft in flight,
//! so a strike over one target doesn't bring itself down with its own bombs; command-fire
//! bombs in the first weapon slot drop on attack orders even beside another weapon (the
//! Phoenix), while the Commander's D-gun still waits for manual fire.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::mission::{MissionRules, Wave};
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, SimEvent, Target, UnitTypes};

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

#[test]
fn a_bomber_strike_does_not_bring_itself_down() {
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    // This tests strike physics with full intelligence, independently of scouting.
    sim.los_enabled = false;
    // Ten Thunders in a tight group against an unarmed building (nothing shoots back).
    let (bomber, solar) = (sim.types.id("ARMTHUND").unwrap(), sim.types.id("CORSOLAR").unwrap());
    let target = sim.spawn_unit(solar, 1, FxVec3::from_ints(3000, 0, 1500), 0, true).unwrap();
    let planes: Vec<_> = (0..10).map(|k| sim.spawn_unit(bomber, 0, FxVec3::from_ints(900 + (k % 5) * 40, 0, 1400 + (k / 5) * 40), 0, true).unwrap()).collect();
    let health0 = sim.unit(target).unwrap().health;
    sim.step(&[Command::Order { player: 0, units: planes.clone(), order: Order::Attack(Target::Unit(target)), queue: false }]);
    for _ in 0..30 * 25 {
        sim.step(&[]);
        sim.take_events();
    }
    let hurt = planes.iter().filter(|&&p| sim.unit(p).is_none_or(|u| u.dead || u.health < sim.unit_type(u).max_health)).count();
    assert_eq!(hurt, 0, "bombers were hit by their own strike's bombs");
    assert!(sim.unit(target).is_none_or(|u| u.dead || u.health < health0), "the strike should have bombed its target");
}

/// Explosions of weapon `name` over `ticks` ticks.
fn blasts(sim: &mut Sim, name: &str, ticks: u64) -> usize {
    let id = sim.weapon_types.types.iter().find(|w| w.def.name.eq_ignore_ascii_case(name)).map(|w| w.id).expect("weapon");
    let mut n = 0;
    for _ in 0..ticks {
        sim.step(&[]);
        n += sim.take_events().iter().filter(|e| matches!(e, SimEvent::Explosion { weapon, .. } if *weapon == id)).count();
    }
    n
}

#[test]
fn advanced_bombers_drop_bombs_on_attack_orders() {
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    // This tests weapon-slot selection with full intelligence, independently of scouting.
    sim.los_enabled = false;
    let (bomber, solar) = (sim.types.id("ARMPNIX").unwrap(), sim.types.id("CORSOLAR").unwrap());
    let target = sim.spawn_unit(solar, 1, FxVec3::from_ints(3000, 0, 1500), 0, true).unwrap();
    let planes: Vec<_> = (0..2).map(|k| sim.spawn_unit(bomber, 0, FxVec3::from_ints(900, 0, 1400 + k * 60), 0, true).unwrap()).collect();
    sim.step(&[Command::Order { player: 0, units: planes, order: Order::Attack(Target::Unit(target)), queue: false }]);
    let bombs = blasts(&mut sim, "ARMADVBOMB", 30 * 25);
    assert!(bombs >= 2, "Phoenixes ordered to attack should bomb (bombs: {bombs})");
}

#[test]
fn the_commander_keeps_its_dgun_for_manual_fire() {
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let (com, foe) = (sim.types.id("ARMCOM").unwrap(), sim.types.id("CORSOLAR").unwrap());
    let c = sim.spawn_unit(com, 0, FxVec3::from_ints(2000, 0, 1500), 16384, true).unwrap();
    let target = sim.spawn_unit(foe, 1, FxVec3::from_ints(2150, 0, 1500), 0, true).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![c], order: Order::Attack(Target::Unit(target)), queue: false }]);
    let dgun = blasts(&mut sim, "ARM_DISINTEGRATOR", 30 * 8);
    assert_eq!(dgun, 0, "an attack order must not fire the D-gun");
    assert!(sim.unit(target).is_none_or(|u| u.dead || u.health < sim.unit_type(u).max_health), "the laser should have fired");
}

/// A scenario's wave of Shadows bombs the player's base. TA's bombs fire on order only
/// (`commandfire=1`) and a wave's units only attack-move, so the Shadows flew over the base,
/// circled and never dropped a bomb; a wave's unit whose every weapon fires on order now goes for
/// the nearest of the player's units its side sees (`ta_sim::mission`'s hunt).
#[test]
fn a_waves_bombers_bomb_what_they_see() {
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    // (In the fog: they go for what their side sees.)
    sim.los_enabled = true;
    let (shadow, solar) = (sim.types.id("CORSHAD").unwrap(), sim.types.id("ARMSOLAR").unwrap());
    let base: Vec<_> = (0..4).map(|k| sim.spawn_unit(solar, 0, FxVec3::from_ints(1500 + k * 120, 0, 1500), 0, true).unwrap()).collect();
    let wave = Wave { tick: 1, owner: 1, units: vec![(shadow, 3)], from: FxVec3::from_ints(3600, 0, 1500), target: None, hunt: true, friendly: false };
    sim.mission = Some(MissionRules { waves: vec![wave], ..Default::default() });
    let bombs = blasts(&mut sim, "CORBOMB", 30 * 60);
    assert!(bombs > 0, "the wave's Shadows should bomb the base they see");
    let hurt = base.iter().filter(|&&b| sim.unit(b).is_none_or(|u| u.dead || u.health < sim.unit_type(u).max_health)).count();
    assert!(hurt > 0, "the bombs should land on the base ({bombs} dropped)");
}
