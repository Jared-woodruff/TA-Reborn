//! A Conquest Commander's perks against the real game data (skipped without a TA install): each
//! changes what it says, for its own player only.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, Perks, PlayerSetup, Sim, Target, UnitId, UnitTypes};

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
            metal: Fx::from_int(5000),
            energy: Fx::from_int(5000),
            ai: false,
        })
        .collect();
    Some(Sim::new(types, data, &map, &info, 0, &players, 99))
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

fn run(sim: &mut Sim, ticks: u32) {
    for _ in 0..ticks {
        sim.step(&[]);
        sim.take_events();
    }
}

/// Holds `units`' fire (and their ground), so a test's victims stay victims.
fn hold(sim: &mut Sim, player: u8, units: Vec<UnitId>) {
    sim.step(&[Command::SetStates { player, units, fire: Some(0), movement: Some(0) }]);
}

/// A battlefield with our Commander on flat ground, and `perks` given to `who`.
fn field(who: u8, perks: Perks) -> Option<(Sim, FxVec3, UnitId)> {
    let mut sim = sim_on("Coast To Coast")?;
    sim.set_perks(who, perks);
    let at = flat_spot(&sim, 240)?;
    let commander = sim.spawn_unit(sim.types.id("ARMCOM")?, 0, at, 0, true)?;
    Some((sim, at, commander))
}

/// Health our Commander loses to an AK firing at it for ten seconds.
fn commander_hurt(perks: Perks) -> Option<Fx> {
    let (mut sim, at, commander) = field(0, perks)?;
    let ak = sim.spawn_unit(sim.types.id("CORAK")?, 1, at + FxVec3::from_ints(110, 0, 0), -16384, true)?;
    hold(&mut sim, 0, vec![commander]);
    sim.step(&[Command::Order { player: 1, units: vec![ak], order: Order::Attack(Target::Unit(commander)), queue: false }]);
    run(&mut sim, 300);
    Some(sim.unit_type(sim.unit(commander)?).max_health - sim.unit(commander)?.health)
}

/// Health a metal storage loses to our Commander's laser in ten seconds, with `perks` for
/// player `who`.
fn storage_hurt(who: u8, perks: Perks) -> Option<Fx> {
    let (mut sim, at, commander) = field(who, perks)?;
    let storage = sim.spawn_unit(sim.types.id("CORMSTOR")?, 1, at + FxVec3::from_ints(120, 0, 0), 0, true)?;
    sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Attack(Target::Unit(storage)), queue: false }]);
    run(&mut sim, 300);
    Some(sim.unit_type(sim.unit(storage)?).max_health - sim.unit(storage)?.health)
}

/// Ticks our Commander takes to build a solar collector beside itself.
fn solar_ticks(perks: Perks) -> Option<u32> {
    let (mut sim, at, commander) = field(0, perks)?;
    let solar = sim.types.id("ARMSOLAR")?;
    sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Build { type_id: solar, pos: at + FxVec3::from_ints(96, 0, 0) }, queue: false }]);
    for tick in 1..=30 * 120 {
        sim.step(&[]);
        sim.take_events();
        if sim.units().any(|u| u.type_id == solar && u.owner == 0 && u.is_complete()) {
            return Some(tick);
        }
    }
    None
}

/// Experience a Stumpy of ours earns killing an AK that holds its fire.
fn kill_experience(perks: Perks) -> Option<Fx> {
    let (mut sim, at, _) = field(0, perks)?;
    let stumpy = sim.spawn_unit(sim.types.id("ARMSTUMP")?, 0, at + FxVec3::from_ints(0, 0, 60), 0, true)?;
    let ak = sim.spawn_unit(sim.types.id("CORAK")?, 1, at + FxVec3::from_ints(150, 0, 60), -16384, true)?;
    hold(&mut sim, 1, vec![ak]);
    sim.step(&[Command::Order { player: 0, units: vec![stumpy], order: Order::Attack(Target::Unit(ak)), queue: false }]);
    for _ in 0..30 * 60 {
        sim.step(&[]);
        sim.take_events();
        if sim.unit(ak).is_none_or(|u| u.dead) {
            break;
        }
    }
    Some(sim.unit(stumpy)?.xp)
}

#[test]
fn armour_spares_the_commander_and_marksman_sharpens_its_aim() {
    let Some(plain) = commander_hurt(Perks::default()) else {
        eprintln!("no TA install: skipped");
        return;
    };
    let armoured = commander_hurt(Perks { armour: true, ..Default::default() }).unwrap();
    assert!(plain > Fx::ZERO, "the AK reached the Commander");
    let ratio = armoured.to_f64() / plain.to_f64();
    assert!((0.62..=0.78).contains(&ratio), "armour takes 30% off: {armoured:?} of {plain:?} ({ratio:.2})");

    let plain = storage_hurt(0, Perks::default()).unwrap();
    let marksman = storage_hurt(0, Perks { marksman: true, ..Default::default() }).unwrap();
    let ratio = marksman.to_f64() / plain.to_f64();
    assert!(plain > Fx::ZERO && (1.15..=1.35).contains(&ratio), "a quarter harder: {marksman:?} against {plain:?} ({ratio:.2})");
    // (The enemy's perks are its own.)
    assert_eq!(storage_hurt(1, Perks { marksman: true, armour: true, ..Default::default() }).unwrap(), plain);
}

#[test]
fn engineer_builds_faster_and_instructor_teaches_faster() {
    let Some(plain) = solar_ticks(Perks::default()) else {
        eprintln!("no TA install: skipped");
        return;
    };
    let engineer = solar_ticks(Perks { engineer: true, ..Default::default() }).unwrap();
    let ratio = engineer as f64 / plain as f64;
    assert!((0.6..=0.75).contains(&ratio), "half as fast again: {engineer} ticks against {plain} ({ratio:.2})");

    let plain = kill_experience(Perks::default()).unwrap();
    let taught = kill_experience(Perks { instructor: true, ..Default::default() }).unwrap();
    assert!(plain > Fx::ZERO, "the Stumpy made its kill");
    assert_eq!(taught, plain * 3 / 2);
}

#[test]
fn capacitors_halve_the_d_gun() {
    let Some((sim, _, commander)) = field(0, Perks { capacitors: true, ..Default::default() }) else {
        eprintln!("no TA install: skipped");
        return;
    };
    let t = sim.unit_type(sim.unit(commander).unwrap());
    let dgun = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w)).find(|w| w.command_fire).expect("the Commander's D-gun");
    let laser = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w)).find(|w| !w.command_fire).expect("its laser");
    assert!(dgun.energy_per_shot > Fx::ZERO);
    assert_eq!(sim.shot_energy(commander, dgun), dgun.energy_per_shot / 2);
    assert_eq!(sim.shot_energy(commander, laser), laser.energy_per_shot, "only the D-gun");
    let (plain, _, other) = field(1, Perks { capacitors: true, ..Default::default() }).unwrap();
    assert_eq!(plain.shot_energy(other, dgun), dgun.energy_per_shot, "not another player's Commander");
}
