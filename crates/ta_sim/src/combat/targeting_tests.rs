use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

fn game() -> Sim {
    game_of(&[
        ("GUN", "Weapon1=LASER; BadTargetCategory=DECOY;"),
        ("TARGET", "CanMove=1; MaxVelocity=1; Category=TARGET;"),
        ("DECOY", "CanMove=1; MaxVelocity=1; Category=DECOY;"),
    ])
}

#[test]
fn possession_turns_in_place_even_when_normal_steering_requires_movement() {
    let mut sim = game_of(&[("DRIVER", "CanMove=1; MaxVelocity=2; SteeringMode=1; TurnRate=1000;")]);
    let unit = spawn(&mut sim, "DRIVER", 0, 300, 300);
    let position = sim.unit(unit).unwrap().pos;
    let input = crate::DirectInput { throttle: 0, turn: 1, heading: 0, pitch: 0, fire: false };
    sim.step(&[crate::Command::DirectControl { player: 0, unit, input: Some(input) }]);
    let u = sim.unit(unit).unwrap();
    assert_eq!(u.heading, -sim.unit_type(u).turn_rate);
    assert_eq!(u.pos.x, position.x);
    assert_eq!(u.pos.z, position.z);
    assert!(u.speed.is_zero());
}

#[test]
fn possession_preserves_orders_limits_motion_and_resumes_them() {
    let mut sim = game_of(&[("DRIVER", "CanMove=1; MaxVelocity=2; Acceleration=0.5; BrakeRate=0.5; TurnRate=1000; SteeringMode=2;")]);
    let unit = spawn(&mut sim, "DRIVER", 0, 300, 300);
    let order = Order::Move(FxVec3::from_ints(800, 0, 300));
    sim.step(&[crate::Command::Order { player: 0, units: vec![unit], order: order.clone(), queue: false }]);
    let before = sim.unit(unit).unwrap().pos;
    let input = crate::DirectInput { throttle: 1, turn: 1, heading: 0, pitch: 0, fire: false };
    let command = crate::Command::DirectControl { player: 0, unit, input: Some(input) };
    for _ in 0..20 {
        let heading = sim.unit(unit).unwrap().heading;
        sim.step(&[command.clone()]);
        let u = sim.unit(unit).unwrap();
        let ty = sim.unit_type(u);
        assert!(crate::fixed::wrap_angle(u.heading - heading).abs() <= ty.turn_rate);
        assert!(u.speed.abs() <= ty.max_speed);
        assert_eq!(u.orders.front(), Some(&order));
    }
    assert_ne!(sim.unit(unit).unwrap().pos, before);
    sim.step(&[crate::Command::DirectControl { player: 0, unit, input: None }]);
    let u = sim.unit(unit).unwrap();
    assert!(u.direct_control.is_none());
    assert_eq!(u.orders.front(), Some(&order));
    assert!(u.path_goal.is_some());
}

#[test]
fn possession_rejects_foreign_and_invalid_input_and_expires_safely() {
    let mut sim = game_of(&[("DRIVER", "CanMove=1; MaxVelocity=2; SteeringMode=2;")]);
    let unit = spawn(&mut sim, "DRIVER", 0, 300, 300);
    let input = crate::DirectInput { throttle: -1, turn: 0, heading: 0, pitch: 0, fire: false };
    sim.step(&[crate::Command::DirectControl { player: 1, unit, input: Some(input) }]);
    assert!(sim.unit(unit).unwrap().direct_control.is_none());
    sim.step(&[crate::Command::DirectControl { player: 0, unit, input: Some(crate::DirectInput { throttle: 2, ..input }) }]);
    assert!(sim.unit(unit).unwrap().direct_control.is_none());
    let z = sim.unit(unit).unwrap().pos.z;
    sim.step(&[crate::Command::DirectControl { player: 0, unit, input: Some(input) }]);
    assert!(sim.unit(unit).unwrap().pos.z < z);
    for _ in 0..91 { sim.step(&[]); }
    assert!(sim.unit(unit).unwrap().direct_control.is_none());
    assert!(sim.unit(unit).unwrap().weapons.iter().all(|weapon| weapon.target.is_none()));
}

/// A driven unit its script holds still (a weapon unpacked: `IMMOBILE`) stays where it stands,
/// told it is wanted on the move so that its script packs up; let go, it drives off. Astern it
/// goes at half its top speed.
#[test]
fn possession_waits_for_a_held_unit_to_pack_up_and_reverses_at_half_speed() {
    let mut sim = game_of(&[("DRIVER", "CanMove=1; MaxVelocity=2; Acceleration=1; BrakeRate=1; TurnRate=1000;")]);
    let unit = spawn(&mut sim, "DRIVER", 0, 300, 300);
    sim.unit_mut(unit).unwrap().immobile = true;
    let start = sim.unit(unit).unwrap().pos;
    let drive = |sim: &mut Sim, input: crate::DirectInput, ticks: usize| {
        for _ in 0..ticks {
            sim.step(&[crate::Command::DirectControl { player: 0, unit, input: Some(input) }]);
        }
    };
    let ahead = crate::DirectInput { throttle: 1, turn: 1, heading: 0, pitch: 0, fire: false };
    drive(&mut sim, ahead, 20);
    let u = sim.unit(unit).unwrap();
    assert_eq!((u.pos, u.heading, u.speed), (start, 0, Fx::ZERO), "held, it neither moves nor turns");
    assert!(u.moving, "wanted on the move, its script is told so");
    sim.unit_mut(unit).unwrap().immobile = false;
    drive(&mut sim, crate::DirectInput { turn: 0, ..ahead }, 20);
    assert_ne!(sim.unit(unit).unwrap().pos, start, "let go, it drives off");
    drive(&mut sim, crate::DirectInput { throttle: -1, turn: 0, ..ahead }, 60);
    let u = sim.unit(unit).unwrap();
    let top = sim.unit_type(u).max_speed;
    assert_eq!(u.speed, Fx::ZERO - top / 2, "astern at half its top speed");
}

#[test]
fn possession_fires_only_on_trigger_and_replays_deterministically() {
    let make = || game_of(&[("GUN", "CanMove=1; MaxVelocity=1; Weapon1=LASER;"), ("TARGET", "")]);
    let (mut sim, mut replay) = (make(), make());
    let mut ids = Vec::new();
    for game in [&mut sim, &mut replay] {
        let unit = spawn(game, "GUN", 0, 300, 300);
        spawn(game, "TARGET", 1, 300, 500);
        ids.push(unit);
    }
    assert_eq!(ids[0], ids[1]);
    let unit = ids[0];
    let mut fired = 0;
    for tick in 0..60 {
        let input = crate::DirectInput { throttle: 0, turn: 0, heading: 0, pitch: 0, fire: tick >= 30 };
        let command = crate::Command::DirectControl { player: 0, unit, input: Some(input) };
        let decoded = command.clone();
        sim.step(&[command]);
        replay.step(&[decoded]);
        assert_eq!(sim.checksum(), replay.checksum());
        let shots = sim.take_events().iter().filter(|event| matches!(event, SimEvent::WeaponFired { unit: shooter, .. } if *shooter == unit)).count();
        if tick < 30 { assert_eq!(shots, 0); }
        fired += shots;
    }
    assert!(fired > 0);
}

/// A game of made-up units (`UnitName` and their FBI keys) armed with LASER, on flat ground.
fn game_of(units: &[(&str, &str)]) -> Sim {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ta-target-ranking-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(dir.join("units")).unwrap();
    std::fs::create_dir_all(dir.join("weapons")).unwrap();
    for &(name, keys) in units {
        std::fs::write(
            dir.join(format!("units/{name}.fbi")),
            format!("[UNITINFO] {{ UnitName={name}; MaxDamage=1000; SightDistance=600; {keys} }}"),
        )
        .unwrap();
    }
    std::fs::write(
        dir.join("weapons/laser.tdf"),
        "[LASER] { range=400; reloadtime=0.1; weaponvelocity=400; rendertype=0; [DAMAGE] { default=1; } }
         [AAGUN] { range=400; reloadtime=0.1; weaponvelocity=400; rendertype=0; toairweapon=1; [DAMAGE] { default=1; } }
         [SHORTLASER] { range=180; reloadtime=0.3; weaponvelocity=400; rendertype=0; beamweapon=1; [DAMAGE] { default=1; } }
         [MISSILE] { range=600; reloadtime=0.5; weaponvelocity=650; startvelocity=450; weaponacceleration=100; rendertype=1; guidance=1; tracks=1; selfprop=1; turnrate=33000; weapontimer=5; [DAMAGE] { default=1; } }
         [DGUN] { range=250; reloadtime=0.9; weaponvelocity=300; rendertype=3; commandfire=1; [DAMAGE] { default=1; } }
         [SLOWGUN] { range=400; reloadtime=3; weaponvelocity=400; rendertype=0; [DAMAGE] { default=1; } }",
    )
    .unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_mod_dir(&dir, "test").unwrap();
    let data = Arc::new(ta_data::GameData::load(&vfs));
    let types = Arc::new(crate::UnitTypes::compile(&data, &|_| None));
    std::fs::remove_dir_all(dir).unwrap();
    let map = ta_formats::tnt::TntMap {
        width: 64,
        height: 64,
        sea_level: 0,
        tiles: Vec::new(),
        tile_map: Vec::new(),
        heights: vec![60; 64 * 64],
        features: vec![ta_formats::tnt::FEATURE_NONE; 64 * 64],
        feature_names: Vec::new(),
        minimap: Default::default(),
        flags: Vec::new(),
    };
    let players: Vec<_> = (0..2)
        .map(|i| crate::PlayerSetup {
            side: "ARM".into(),
            color: i,
            start: FxVec3::ZERO,
            metal: Fx::from_int(1000),
            energy: Fx::from_int(1000),
            ai: i == 0,
        })
        .collect();
    Sim::new(
        types,
        data,
        &map,
        &ta_data::MapInfo::default(),
        0,
        &players,
        7,
    )
}

#[test]
fn ranking_keeps_visibility_categories_priority_retaliation_and_id_ties() {
    for case in 0..6 {
        let mut sim = game();
        let spawn = |sim: &mut Sim, name, owner, x, z| {
            sim.spawn_unit(
                sim.types.id(name).unwrap(),
                owner,
                FxVec3::from_ints(x, 0, z),
                0,
                true,
            )
            .unwrap()
        };
        let gun = spawn(&mut sim, "GUN", 0, 300, 300);
        let decoy = spawn(&mut sim, "DECOY", 1, 332, 300);
        let first = spawn(&mut sim, "TARGET", 1, 440, 300);
        spawn(&mut sim, "TARGET", 1, 300, 440); // Equal distance; the lower ID wins.
        let hidden = spawn(&mut sim, "TARGET", 1, 380, 300);
        let attacker = spawn(&mut sim, "TARGET", 1, 500, 300);
        let priority = spawn(&mut sim, "TARGET", 1, 300, 540);
        sim.unit_mut(hidden).unwrap().cloaked = true;
        let rules = crate::mission::MissionRules {
            priority_targets: match case {
                2 | 4 => vec![priority],
                3 => vec![decoy],
                5 => vec![hidden],
                _ => Vec::new(),
            },
            ai_ignored: if case == 4 {
                vec![priority]
            } else {
                Vec::new()
            },
            ..Default::default()
        };
        sim.mission = Some(rules);
        if matches!(case, 1 | 2 | 4) {
            let u = sim.unit_mut(gun).unwrap();
            u.last_attacker = Some(attacker);
            u.last_hit_tick = 0;
        }
        sim.grid.rebuild(sim.units.iter().flatten());
        sim.vision.set_all(0, crate::vision::LOS);
        let weapons = sim.weapon_types.clone();
        let weapon = sim.unit_type(sim.unit(gun).unwrap()).weapons[0].unwrap();
        let expected = match case {
            1 | 4 => attacker,
            2 => priority,
            _ => first,
        };
        assert_eq!(
            sim.find_target(gun, 0, weapons.get(weapon), 0, None, weapons.get(weapon).range),
            Some(Target::Unit(expected)),
            "target ranking case {case}"
        );
    }
}

fn spawn(sim: &mut Sim, name: &str, owner: u8, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), owner, FxVec3::from_ints(x, 0, z), 0, true).unwrap()
}

/// TA's own game data from the install, read once (none without an install).
pub(super) fn installed() -> Option<&'static (Arc<ta_data::GameData>, Arc<crate::UnitTypes>)> {
    static DATA: std::sync::OnceLock<Option<(Arc<ta_data::GameData>, Arc<crate::UnitTypes>)>> = std::sync::OnceLock::new();
    DATA.get_or_init(|| {
        let install = ta_vfs::install::discover(None).ok()?;
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").ok()?;
        let data = Arc::new(ta_data::GameData::load(&vfs));
        let types = Arc::new(crate::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
        Some((data, types))
    })
    .as_ref()
}

/// A battle of TA's own units, the Arm (player 0) against the Core (player 1), each with plenty
/// of metal and energy, on a flat 128 × 128-cell map (2,048 px a side): the ground at 60 under a
/// sea whose surface is at `sea` (60 and under: dry land), rising from x = 1,100 px to `shelf` at
/// x = 1,250 and on (a shallow shelf, or the shore). None without an install.
pub(super) fn tas_battle(sea: i32, shelf: u8) -> Option<Sim> {
    let (data, types) = installed()?;
    let mut heights = vec![60u8; 128 * 128];
    for z in 0..128 {
        for x in 0..128 {
            let px = x as i64 * 16;
            let rise = ((px - 1100).clamp(0, 150) * (shelf as i64 - 60)) / 150;
            heights[z * 128 + x] = (60 + rise) as u8;
        }
    }
    let map = ta_formats::tnt::TntMap {
        width: 128,
        height: 128,
        sea_level: sea,
        tiles: Vec::new(),
        tile_map: Vec::new(),
        heights,
        features: vec![ta_formats::tnt::FEATURE_NONE; 128 * 128],
        feature_names: Vec::new(),
        minimap: Default::default(),
        flags: Vec::new(),
    };
    let players: Vec<_> = (0..2)
        .map(|i| crate::PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(100_000), energy: Fx::from_int(1_000_000), ai: false })
        .collect();
    let mut sim = Sim::new(types.clone(), data.clone(), &map, &ta_data::MapInfo::default(), 0, &players, 11);
    for p in 0..2 {
        sim.set_starting_resources(p, Fx::from_int(100_000), Fx::from_int(1_000_000));
    }
    Some(sim)
}

/// Puts one of TA's units for `owner` at (x, z), on the ground or the water there, facing east.
pub(super) fn put_at(sim: &mut Sim, name: &str, owner: u8, x: i64, z: i64) -> UnitId {
    let ground = sim.world.height_at(Fx::from_int(x), Fx::from_int(z));
    sim.spawn_unit(sim.types.id(name).unwrap_or_else(|| panic!("{name}")), owner, FxVec3::new(Fx::from_int(x), ground, Fx::from_int(z)), 16384, true).unwrap()
}

/// Whether `unit` fired in the events since they were last taken.
pub(super) fn fired(sim: &mut Sim, unit: UnitId) -> bool {
    sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit: u, .. } if *u == unit))
}

#[test]
fn idle_turrets_retarget_after_an_enemy_leaves_firing_range() {
    for departing_x in [620, 640] {
        let mut sim = game();
        sim.los_enabled = false;
        let gun = spawn(&mut sim, "GUN", 0, 200, 300);
        let departing = spawn(&mut sim, "TARGET", 1, departing_x, 300);
        let reachable = spawn(&mut sim, "TARGET", 1, 200, 600);
        sim.unit_mut(departing).unwrap().fire_state = 0;
        sim.unit_mut(reachable).unwrap().fire_state = 0;
        sim.unit_mut(gun).unwrap().retarget(0, Some(Target::Unit(departing)));
        let mut fired = false;
        for _ in 0..60 {
            sim.step(&[]);
            fired |= sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == gun));
        }
        assert!(fired, "target at x={departing_x}");
        assert_eq!(sim.unit(gun).unwrap().weapons[0].target, Some(Target::Unit(reachable)));
    }
}

/// A lone enemy that steps just past a turret's reach and comes back is fired on as it comes back
/// into reach: the turret keeps it while it looks round for another (it found none), not passed
/// over for the two seconds (`BLOCKED_TICKS`) a target it cannot hit is.
#[test]
fn a_lone_enemy_back_in_reach_is_fired_on_at_once() {
    let mut sim = game_of(&[
        ("GUN", "Weapon1=LASER;"),
        ("TARGET", "CanMove=1; MaxVelocity=3; Acceleration=3; BrakeRate=3; TurnRate=30000; Category=TARGET;"),
    ]);
    sim.los_enabled = false;
    let gun = spawn(&mut sim, "GUN", 0, 200, 300);
    let enemy = spawn(&mut sim, "TARGET", 1, 520, 300);
    sim.unit_mut(enemy).unwrap().fire_state = 0;
    let fired = |sim: &mut Sim| sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == gun));
    let mut shots = 0;
    for _ in 0..30 {
        sim.step(&[]);
        shots += u32::from(fired(&mut sim));
    }
    assert!(shots > 0, "fired on in reach");
    // Past its reach (the laser's 400 and the target's size), within what it keeps a target to.
    let order = |x: i64| crate::Command::Order { player: 1, units: vec![enemy], order: Order::Move(FxVec3::from_ints(x, 0, 300)), queue: false };
    sim.step(&[order(630)]);
    for _ in 0..90 {
        if sim.unit(enemy).unwrap().pos.x > Fx::from_int(625) { break; }
        sim.step(&[]);
        fired(&mut sim);
    }
    assert!(sim.unit(enemy).unwrap().pos.x > Fx::from_int(625), "past reach");
    for _ in 0..10 {
        sim.step(&[]);
        assert!(!fired(&mut sim), "never fired on past its reach");
    }
    assert_eq!(sim.unit(gun).unwrap().weapons[0].target, Some(Target::Unit(enemy)), "kept, there being no other");
    sim.step(&[order(520)]);
    let back = (0..60).position(|_| {
        sim.step(&[]);
        fired(&mut sim)
    });
    assert!(back.is_some_and(|ticks| ticks <= 15), "fired on as it came back, not {back:?} ticks after");
}

#[test]
fn attack_move_closes_on_a_retained_target_outside_firing_range() {
    let mut sim = game_of(&[
        ("GUN", "Weapon1=LASER; CanMove=1; MaxVelocity=2; Acceleration=1; BrakeRate=1; TurnRate=3000;"),
        ("TARGET", ""),
    ]);
    sim.los_enabled = false;
    let gun = spawn(&mut sim, "GUN", 0, 200, 300);
    let target = spawn(&mut sim, "TARGET", 1, 620, 300);
    sim.unit_mut(target).unwrap().fire_state = 0;
    sim.unit_mut(gun).unwrap().retarget(0, Some(Target::Unit(target)));
    let types = sim.types.clone();
    let weapons = sim.weapon_types.clone();
    let dest = FxVec3::from_ints(900, 0, 300);
    let t = types.get(sim.unit(gun).unwrap().type_id);
    sim.advance_engaging(gun, t, dest, None);
    assert_eq!(sim.unit(gun).unwrap().move_request, Some(dest));
    sim.update_weapons(gun, &types, &weapons);
    assert!(sim.projectiles.is_empty());
    sim.step(&[crate::Command::Order { player: 0, units: vec![gun], order: Order::Fight(dest), queue: false }]);
    let mut fired = false;
    for _ in 0..180 {
        sim.step(&[]);
        fired |= sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == gun));
    }
    assert!(fired);
    assert!(sim.unit(gun).unwrap().pos.x < sim.unit(target).unwrap().pos.x);
}

#[test]
fn explicit_attack_orders_keep_the_target_outside_firing_range() {
    for departing_x in [620, 680] {
        let mut sim = game();
        sim.los_enabled = false;
        let gun = spawn(&mut sim, "GUN", 0, 200, 300);
        let ordered = spawn(&mut sim, "TARGET", 1, departing_x, 300);
        let reachable = spawn(&mut sim, "TARGET", 1, 200, 600);
        for id in [ordered, reachable] {
            sim.unit_mut(id).unwrap().fire_state = 0;
        }
        sim.unit_mut(gun).unwrap().orders.push_back(Order::Attack(Target::Unit(ordered)));
        let mut fired = false;
        for _ in 0..60 {
            sim.step(&[]);
            fired |= sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == gun));
        }
        assert!(!fired);
        assert_eq!(sim.unit(gun).unwrap().weapons[0].target, Some(Target::Unit(ordered)));
        assert!(matches!(sim.unit(gun).unwrap().orders.front(), Some(Order::Attack(Target::Unit(t))) if *t == ordered));
    }
}

/// TA's units take a bad target on last: when it is the only enemy in reach (a missile tower
/// with no aircraft about fires on a tank), and they turn to a better one as soon as one comes
/// into reach. With the player's `+shootall` a bad target counts as any other: the nearest first.
#[test]
fn a_bad_target_is_taken_on_last() {
    let mut sim = game_of(&[
        ("GUN", "Weapon1=LASER; BadTargetCategory=DECOY; Category=GUN;"),
        ("TARGET", "CanMove=1; MaxVelocity=1; Category=TARGET;"),
        ("DECOY", "CanMove=1; MaxVelocity=1; Category=DECOY;"),
    ]);
    sim.vision.set_all(0, crate::vision::LOS);
    sim.vision.set_all(1, crate::vision::LOS);
    let gun = spawn(&mut sim, "GUN", 0, 300, 300);
    let decoy = spawn(&mut sim, "DECOY", 1, 340, 300);
    for _ in 0..60 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(gun).unwrap().weapons[0].target, Some(Target::Unit(decoy)), "the only enemy near: taken on, bad target as it is");
    let target = spawn(&mut sim, "TARGET", 1, 400, 300);
    for _ in 0..60 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(gun).unwrap().weapons[0].target, Some(Target::Unit(target)), "a better one in reach, further off: it turns to that");
    // `+shootall`: the nearest, bad target or not.
    sim.step(&[crate::Command::SetShootAll { player: 0, on: true }]);
    assert!(sim.players[0].shoot_all);
    sim.grid.rebuild(sim.units.iter().flatten());
    let weapons = sim.weapon_types.clone();
    let w = sim.unit_type(sim.unit(gun).unwrap()).weapons[0].unwrap();
    assert_eq!(sim.find_target(gun, 0, weapons.get(w), 0, None, weapons.get(w).range), Some(Target::Unit(decoy)), "with +shootall the nearest");
    sim.step(&[crate::Command::SetShootAll { player: 0, on: false }]);
    sim.grid.rebuild(sim.units.iter().flatten());
    assert_eq!(sim.find_target(gun, 0, weapons.get(w), 0, None, weapons.get(w).range), Some(Target::Unit(target)), "and without it the better one again");
}

/// Each weapon keeps its own (TA's `wpri_`, `wsec_badTargetCategory`): a cruiser's guns never
/// pick an aircraft, its depth charges nothing but a submarine.
#[test]
fn each_weapon_keeps_its_own_bad_targets() {
    let mut sim = game_of(&[
        ("TWIN", "Weapon1=LASER; Weapon2=LASER; wpri_badTargetCategory=DECOY; wsec_badTargetCategory=TARGET; BadTargetCategory=DECOY;"),
        ("TARGET", "CanMove=1; MaxVelocity=1; Category=TARGET;"),
        ("DECOY", "CanMove=1; MaxVelocity=1; Category=DECOY;"),
    ]);
    let twin = spawn(&mut sim, "TWIN", 0, 300, 300);
    let decoy = spawn(&mut sim, "DECOY", 1, 340, 300);
    let target = spawn(&mut sim, "TARGET", 1, 380, 300);
    sim.grid.rebuild(sim.units.iter().flatten());
    sim.vision.set_all(0, crate::vision::LOS);
    let weapons = sim.weapon_types.clone();
    let w = |sim: &Sim, k: usize| sim.unit_type(sim.unit(twin).unwrap()).weapons[k].unwrap();
    let (w0, w1) = (w(&sim, 0), w(&sim, 1));
    assert_eq!(sim.find_target(twin, 0, weapons.get(w0), 0, None, weapons.get(w0).range), Some(Target::Unit(target)), "the first leaves the decoy");
    assert_eq!(sim.find_target(twin, 1, weapons.get(w1), 0, None, weapons.get(w1).range), Some(Target::Unit(decoy)), "the second leaves the target");
}

/// A weapon naming its only targets (`OnlyTargetCategory1=VTOL`: Supreme Commander's anti-air
/// towers) takes no other, by order either: told to attack a tank or the ground, its order ends
/// and it never fires; an aircraft in flight it takes on, of its own accord or told to, but not
/// one landed (which it cannot hit). TA's own anti-air (`toairweapon` alone) fires on the tank
/// when told to, as ever.
#[test]
fn a_weapon_naming_its_only_targets_takes_no_other_by_order_either() {
    let mut sim = game_of(&[
        ("TOWER", "Weapon1=AAGUN; OnlyTargetCategory1=VTOL; Category=NOTAIR;"),
        ("FLAKKER", "Weapon1=AAGUN; Category=NOTAIR;"),
        ("TANK", "CanMove=1; MaxVelocity=1; Category=NOTAIR TANK;"),
        ("JET", "CanFly=1; CanMove=1; MaxVelocity=5; Category=VTOL;"),
    ]);
    let tower = spawn(&mut sim, "TOWER", 0, 300, 300);
    let flakker = spawn(&mut sim, "FLAKKER", 0, 300, 600);
    let tank = spawn(&mut sim, "TANK", 1, 420, 450);
    let types = sim.types.clone();
    assert!(types.by_name("TOWER").unwrap().weapon_only_masks[0].is_some() && types.by_name("FLAKKER").unwrap().weapon_only_masks[0].is_none());
    let order = |units: Vec<UnitId>, target: Target| crate::Command::Order { player: 0, units, order: Order::Attack(target), queue: false };
    let (mut tower_fired, mut flakker_fired) = (false, false);
    sim.step(&[order(vec![tower, flakker], Target::Unit(tank))]);
    for _ in 0..60 {
        sim.step(&[]);
        tower_fired |= sim.projectiles.iter().any(|p| p.owner == tower);
        flakker_fired |= sim.projectiles.iter().any(|p| p.owner == flakker);
        assert_ne!(sim.unit(tower).unwrap().weapons[0].target, Some(Target::Unit(tank)), "the tower never takes the tank on");
    }
    assert!(!tower_fired && sim.unit(tower).unwrap().orders.is_empty(), "its order on the tank ended, nothing fired");
    assert!(flakker_fired, "TA's anti-air fires on the tank when told to");
    // The ground: the same.
    sim.step(&[order(vec![tower], Target::Ground(FxVec3::from_ints(380, 60, 330)))]);
    for _ in 0..30 {
        sim.step(&[]);
        tower_fired |= sim.projectiles.iter().any(|p| p.owner == tower);
    }
    assert!(!tower_fired && sim.unit(tower).unwrap().orders.is_empty(), "its order on the ground ended, nothing fired");
    // An aircraft in flight it takes, of its own accord or told to; landed, not.
    let jet = spawn(&mut sim, "JET", 1, 360, 380);
    {
        let u = sim.unit_mut(jet).unwrap();
        u.landed = false;
        u.pos.y += Fx::from_int(120);
    }
    sim.grid.rebuild(sim.units.iter().flatten());
    sim.vision.set_all(0, crate::vision::LOS);
    let weapons = sim.weapon_types.clone();
    let t = types.by_name("TOWER").unwrap();
    let w = weapons.get(t.weapons[0].unwrap());
    assert_eq!(sim.find_target(tower, 0, w, 0, None, w.range), Some(Target::Unit(jet)), "the jet in flight, of its own accord");
    assert!(sim.takes_order(t, 0, w, Target::Unit(jet), 0), "the jet in flight, by order");
    assert!(!sim.takes_order(t, 0, w, Target::Unit(tank), 0) && !sim.takes_order(t, 0, w, Target::Ground(FxVec3::from_ints(380, 60, 330)), 0));
    sim.unit_mut(jet).unwrap().landed = true;
    assert!(!sim.takes_order(t, 0, w, Target::Unit(jet), 0), "a jet landed, which it cannot hit");
}

/// A unit its script holds still (`IMMOBILE`: a Supreme Commander weapon deployed) fires as it
/// stands; told to go, it fires nothing while its script packs it up, though its weapon's aim
/// answered before; let go, it fires again as any other unit.
#[test]
fn a_unit_held_still_fires_nothing_while_it_packs_up_to_go() {
    use crate::host::value::IMMOBILE;
    use ta_formats::cob::{CobFunction, CobScript, op};
    let mut sim = game_of(&[
        ("ARTY", "Weapon1=LASER; CanMove=1; MaxVelocity=2; Acceleration=0.2; BrakeRate=0.2; TurnRate=900;"),
        ("TARGET", "Category=TARGET;"),
    ]);
    // Create: held. AimPrimary: aimed at once. StartMoving: a second's packing, then let go.
    let mut script = CobScript { pieces: vec!["base".into()], ..Default::default() };
    let mut function = |name: &str, code: &[u32]| {
        script.functions.push(CobFunction { name: name.into(), offset: script.code.len() as u32 });
        script.code.extend_from_slice(code);
    };
    let hold = |on: u32| [op::PUSH_CONSTANT, IMMOBILE as u32, op::PUSH_CONSTANT, on, op::SET_VALUE];
    function("Create", &[&hold(1)[..], &[op::PUSH_CONSTANT, 0, op::RETURN]].concat());
    function("AimPrimary", &[op::PUSH_CONSTANT, 1, op::RETURN]);
    function("StartMoving", &[&[op::PUSH_CONSTANT, 1000, op::SLEEP][..], &hold(0), &[op::PUSH_CONSTANT, 0, op::RETURN]].concat());
    let arty_type = sim.types.id("ARTY").unwrap();
    Arc::get_mut(&mut sim.types).unwrap().types[arty_type].script = Some(Arc::new(crate::cob::CobProgram::new(script, &["base".to_string()])));
    let arty = spawn(&mut sim, "ARTY", 0, 300, 300);
    spawn(&mut sim, "TARGET", 1, 450, 300);
    sim.vision.set_all(0, crate::vision::LOS);
    let mut seen = std::collections::BTreeSet::new();
    let mut shots = |sim: &Sim| sim.projectiles.iter().filter(|p| p.owner == arty && seen.insert(p.id)).count();
    let mut fired = 0;
    for _ in 0..30 {
        sim.step(&[]);
        fired += shots(&sim);
    }
    let start_at = sim.unit(arty).unwrap().pos;
    assert!(fired > 0 && sim.unit(arty).unwrap().immobile, "held, it fires as it stands");
    // Told to go (the target still in reach all the way): its second of packing, nothing fired.
    let order = crate::Command::Order { player: 0, units: vec![arty], order: Order::Move(FxVec3::from_ints(300, 0, 560)), queue: false };
    sim.step(&[order]);
    shots(&sim);
    for tick in 0..25 {
        sim.step(&[]);
        let u = sim.unit(arty).unwrap();
        assert!(u.packing_to_go() && u.pos == start_at, "tick {tick}: held, packing up to go");
        assert_eq!(shots(&sim), 0, "tick {tick}: nothing fired while it packs up");
    }
    // Let go: it sets off and fires again.
    let mut fired = 0;
    for _ in 0..60 {
        sim.step(&[]);
        fired += shots(&sim);
    }
    let u = sim.unit(arty).unwrap();
    assert!(!u.immobile && u.pos != start_at && fired > 0, "let go, off it goes, firing");
}

/// A unit its script holds still, told to go, aims anew once let go: an aim its weapon answered
/// while it stood unpacked fires nothing once it is packed and on its way, where its script
/// answers no to an aim (as the Supreme Commander factions' unpacking weapons' scripts do, aiming
/// none while their unit moves). Its gun fired, then told to go as it is about to have reloaded:
/// let go, its reload is done and its target in reach, the aim from before answered yes.
#[test]
fn a_unit_let_go_fires_nothing_on_an_aim_from_before_it_packed() {
    use crate::host::value::IMMOBILE;
    use ta_formats::cob::{CobFunction, CobScript, op};
    let mut sim = game_of(&[
        ("ARTY", "Weapon1=SLOWGUN; CanMove=1; MaxVelocity=2; Acceleration=0.2; BrakeRate=0.2; TurnRate=900;"),
        ("TARGET", "Category=TARGET;"),
    ]);
    // Create: held, its aim answering yes. AimPrimary: the static's answer. StartMoving: the aim
    // answering no from then on, a second's packing, then let go.
    let mut script = CobScript { pieces: vec!["base".into()], static_var_count: 1, ..Default::default() };
    let mut function = |name: &str, code: &[u32]| {
        script.functions.push(CobFunction { name: name.into(), offset: script.code.len() as u32 });
        script.code.extend_from_slice(code);
    };
    let hold = |on: u32| [op::PUSH_CONSTANT, IMMOBILE as u32, op::PUSH_CONSTANT, on, op::SET_VALUE];
    let answer = |yes: u32| [op::PUSH_CONSTANT, yes, op::POP_STATIC, 0];
    function("Create", &[&hold(1)[..], &answer(1), &[op::PUSH_CONSTANT, 0, op::RETURN]].concat());
    function("AimPrimary", &[op::PUSH_STATIC, 0, op::RETURN]);
    function("StartMoving", &[&answer(0)[..], &[op::PUSH_CONSTANT, 1000, op::SLEEP], &hold(0), &[op::PUSH_CONSTANT, 0, op::RETURN]].concat());
    let arty_type = sim.types.id("ARTY").unwrap();
    Arc::get_mut(&mut sim.types).unwrap().types[arty_type].script = Some(Arc::new(crate::cob::CobProgram::new(script, &["base".to_string()])));
    let arty = spawn(&mut sim, "ARTY", 0, 300, 300);
    spawn(&mut sim, "TARGET", 1, 450, 300);
    sim.vision.set_all(0, crate::vision::LOS);
    let mut seen = std::collections::BTreeSet::new();
    let mut shots = |sim: &Sim| sim.projectiles.iter().filter(|p| p.owner == arty && seen.insert(p.id)).count();
    // Held, it fires as it stands, aimed: one shot, then its three seconds' reload.
    let mut ticks = 0;
    while shots(&sim) == 0 {
        sim.step(&[]);
        ticks += 1;
        assert!(ticks < 60, "held, it fires as it stands, aimed");
    }
    for _ in 0..60 {
        sim.step(&[]);
    }
    let w = sim.unit(arty).unwrap().weapons[0];
    assert!(w.aimed && w.reload > 0, "aimed, reloading: {w:?}");
    // Told to go along the target's side, in reach all the way: packing, then let go with its
    // reload done.
    let order = crate::Command::Order { player: 0, units: vec![arty], order: Order::Move(FxVec3::from_ints(300, 0, 400)), queue: false };
    sim.step(&[order]);
    shots(&sim);
    let mut let_go = false;
    for tick in 0..90 {
        sim.step(&[]);
        let u = sim.unit(arty).unwrap();
        let_go |= !u.immobile;
        let ready = u.weapons[0].reload == 0;
        assert_eq!(shots(&sim), 0, "tick {tick}: nothing fired, packing or let go (reload done: {ready}), on an aim from before");
    }
    assert!(let_go && sim.unit(arty).unwrap().weapons[0].reload == 0, "let go, its reload done");
}

/// A weapon whose bad targets are every unit it could hit would never fire of its own accord,
/// which mods' tags of their own make (Total Mayhem's flak towers list `ALL`, which all its units
/// carry; Escalation's anti-air missiles a tag all its aircraft carry): its bad targets are
/// lifted, and it takes aircraft on. One with a unit left to take on keeps to its bad targets.
#[test]
fn a_weapon_whose_bad_targets_are_all_it_could_hit_takes_on_what_it_can() {
    let mut sim = game_of(&[
        ("FLAKKER", "Weapon1=AAGUN; BadTargetCategory=ALL; Category=ALL NOTAIR;"),
        ("PICKY", "Weapon1=AAGUN; wpri_badTargetCategory=FAST; Category=ALL NOTAIR;"),
        ("JET", "CanFly=1; CanMove=1; MaxVelocity=5; Category=ALL VTOL FAST;"),
        ("BLIMP", "CanFly=1; CanMove=1; MaxVelocity=1; Category=ALL VTOL;"),
    ]);
    let types = sim.types.clone();
    assert!(types.by_name("FLAKKER").unwrap().weapon_bad_masks[0].is_empty(), "every unit lists ALL: lifted");
    assert!(!types.by_name("PICKY").unwrap().weapon_bad_masks[0].is_empty(), "the blimp is left to it: kept");
    let flakker = spawn(&mut sim, "FLAKKER", 0, 300, 300);
    let picky = spawn(&mut sim, "PICKY", 0, 300, 500);
    let jet = spawn(&mut sim, "JET", 1, 360, 400);
    {
        let u = sim.unit_mut(jet).unwrap();
        u.landed = false;
        u.pos.y += Fx::from_int(120);
    }
    sim.grid.rebuild(sim.units.iter().flatten());
    sim.vision.set_all(0, crate::vision::LOS);
    let weapons = sim.weapon_types.clone();
    let w = |sim: &Sim, id: UnitId| sim.unit_type(sim.unit(id).unwrap()).weapons[0].unwrap();
    let (flak, aa) = (w(&sim, flakker), w(&sim, picky));
    assert_eq!(sim.find_target(flakker, 0, weapons.get(flak), 0, None, weapons.get(flak).range), Some(Target::Unit(jet)), "the flak takes the jet on");
    assert_eq!(sim.find_target(picky, 0, weapons.get(aa), 0, None, weapons.get(aa).range), Some(Target::Unit(jet)), "the jet is FAST: a bad target, but the only one near");
}

/// Only the units a player can field count: a weapon whose bad targets are every unit the sides
/// can build is lifted, though a unit nobody builds lacks them (Escalation's joke unit and
/// mission-only units lack the `ALL` its weapons list: the Raptor's Starburst Missiles never
/// fired). Once a side can build that unit, the weapon keeps to its bad targets.
#[test]
fn only_the_units_a_player_can_field_count_for_lifting_bad_targets() {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let compile = |oddity_built: bool| {
        let dir = std::env::temp_dir().join(format!("ta-fieldable-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        for sub in ["units", "weapons", "gamedata"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        for (name, keys) in [
            ("BOSS", "Builder=1; WorkerTime=100; Category=ALL NOTAIR;"),
            ("FLAKKER", "Weapon1=AAGUN; BadTargetCategory=ALL; Category=ALL NOTAIR;"),
            ("JET", "CanFly=1; CanMove=1; MaxVelocity=5; Category=ALL VTOL;"),
            ("ODDITY", "CanFly=1; CanMove=1; MaxVelocity=5; Category=VTOL;"),
        ] {
            std::fs::write(dir.join(format!("units/{name}.fbi")), format!("[UNITINFO] {{ UnitName={name}; MaxDamage=1000; {keys} }}")).unwrap();
        }
        std::fs::write(
            dir.join("weapons/laser.tdf"),
            "[AAGUN] { range=400; reloadtime=0.1; weaponvelocity=400; rendertype=0; toairweapon=1; [DAMAGE] { default=1; } }",
        )
        .unwrap();
        let extra = if oddity_built { " canbuild3=ODDITY;" } else { "" };
        std::fs::write(
            dir.join("gamedata/sidedata.tdf"),
            format!("[SIDE0] {{ name=ARM; commander=BOSS; }} [CANBUILD] {{ [BOSS] {{ canbuild1=FLAKKER; canbuild2=JET;{extra} }} }}"),
        )
        .unwrap();
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&dir, "test").unwrap();
        let data = ta_data::GameData::load(&vfs);
        let types = crate::UnitTypes::compile(&data, &|_| None);
        let _ = std::fs::remove_dir_all(&dir);
        types.by_name("FLAKKER").unwrap().weapon_bad_masks[0].is_empty()
    };
    assert!(compile(false), "nobody builds the oddity: every aircraft a player can field lists ALL, so the flak's ALL is lifted");
    assert!(!compile(true), "the oddity built: an aircraft left to the flak, so it keeps to its bad targets");
}

/// TA's own units, every one: what each of its weapons takes on first, of an aircraft in flight,
/// a tank, a building, a boat and a submarine (`--nocapture` lists them; the rest, its bad
/// targets, only when nothing of those is in reach). Held to TA: its anti-air units (the AA
/// K-bots, missile trucks and towers, flak, fighters) take aircraft first, its other armed units
/// what is on the ground or the water, submarines' torpedoes only what is in the water, the
/// cruisers' depth charges submarines. (The anti-air hovercraft as anti-air, past the slip in
/// their files: see `defs::target_tags`.)
#[test]
fn every_units_weapons_pick_what_tas_rules_let_them() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(ta_data::GameData::load(&vfs));
    let types = Arc::new(crate::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    // (No weapon of TA's has bad targets that are every unit it could hit: none is lifted.)
    for t in &types.types {
        for k in (0..3).filter(|&k| t.weapons[k].is_some()) {
            let tags = match t.def.weapon_bad_target_category[k].trim() { "" => t.def.bad_target_category.as_str(), list => list };
            assert!(types.categories(tags).is_empty() || !t.weapon_bad_masks[k].is_empty(), "{} weapon {}: its bad targets lifted", t.name, k + 1);
        }
    }
    let map = ta_formats::tnt::TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = ta_data::MapInfo::from_ota(&ta_formats::tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    let players: Vec<_> = (0..2)
        .map(|i| crate::PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 3);
    // (Coast To Coast: land in the west, a sea 85 deep in the middle.)
    let (land, sea) = (FxVec3::from_ints(400, 0, 1000), FxVec3::from_ints(1600, 0, 1000));
    assert!(sim.world.depth_at(land.x, land.z) <= Fx::ZERO && sim.world.depth_at(sea.x, sea.z) > Fx::from_int(40));
    let put = |sim: &mut Sim, name: &str, at: FxVec3| sim.spawn_unit(types.id(name).unwrap(), 1, at, 0, true).unwrap();
    let aircraft = put(&mut sim, "ARMBRAWL", land);
    {
        let u = sim.unit_mut(aircraft).unwrap();
        u.landed = false;
        u.pos.y += Fx::from_int(120);
    }
    let targets = [
        ("air", aircraft),
        ("tank", put(&mut sim, "ARMSTUMP", land + FxVec3::from_ints(100, 0, 0))),
        ("building", put(&mut sim, "ARMLLT", land + FxVec3::from_ints(0, 0, 100))),
        ("boat", put(&mut sim, "ARMPT", sea)),
        ("sub", put(&mut sim, "ARMSUB", sea + FxVec3::from_ints(150, 0, 0))),
    ];
    assert!(sim.airborne(sim.unit(aircraft).unwrap()), "the Brawler is flying");
    assert!(sim.in_water(sim.unit(targets[3].1).unwrap()) && !sim.submerged(sim.unit(targets[3].1).unwrap()), "the PT boat is on the water");
    assert!(sim.submerged(sim.unit(targets[4].1).unwrap()), "the submarine is under it");
    let weapons = sim.weapon_types.clone();
    // What a unit's weapon in `slot` (by TA's names) takes on of its own accord.
    let picks = |sim: &Sim, unit: &str, slot: usize| -> Vec<&'static str> {
        let t = types.by_name(unit).unwrap_or_else(|| panic!("{unit}"));
        let w = weapons.get(t.weapons[slot].unwrap_or_else(|| panic!("{unit} has no weapon {}", slot + 1)));
        targets.iter().filter(|&&(_, id)| sim.engages(t.id, slot, w, sim.unit(id).unwrap())).map(|&(kind, _)| kind).collect()
    };
    let mut table = Vec::new();
    for t in &types.types {
        for slot in 0..3 {
            let Some(wid) = t.weapons[slot] else { continue };
            let w = weapons.get(wid);
            // (Only by order: D-guns, bombs and nukes; anti-missiles take on missiles alone.)
            if w.command_fire || w.def.interceptor {
                continue;
            }
            let kinds = picks(&sim, &t.name, slot);
            table.push(format!("{:<9} weapon {} {:<20} {}", t.name, slot + 1, w.name, if kinds.is_empty() { "nothing".to_string() } else { kinds.join(", ") }));
        }
    }
    println!("{}", table.join("\n"));
    let mut wrong = Vec::new();
    let mut expect = |unit: &str, slot: usize, want: &[&str]| {
        let got = picks(&sim, unit, slot);
        if got != want {
            wrong.push(format!("{unit} weapon {}: picks {got:?}, TA {want:?}", slot + 1));
        }
    };
    // Anti-air: aircraft only.
    for unit in ["ARMJETH", "CORCRASH", "ARMSAM", "CORMIST", "ARMRL", "CORRL", "ARMFLAK", "CORFLAK", "ARMYORK", "CORSENT", "ARMFIG", "CORVENG", "ARMHAWK", "CORVAMP"] {
        expect(unit, 0, &["air"]);
    }
    // (The anti-air hovercraft too, though TA's files give them the ground hovercraft's tags.)
    for unit in ["ARMAH", "CORAH"] {
        expect(unit, 0, &["air"]);
    }
    // The ground's: never an aircraft.
    for unit in ["ARMPW", "ARMFLASH", "CORGATOR", "ARMLLT", "CORLLT", "ARMHLT", "CORHLT", "ARMCOM", "CORCOM", "ARMZEUS", "CORPYRO"] {
        expect(unit, 0, &["tank", "building", "boat"]);
    }
    // (TA's A.K. lists no bad targets, the Peewee VTOL: the A.K. takes on aircraft too.)
    expect("CORAK", 0, &["air", "tank", "building", "boat"]);
    // Shells: never an aircraft (they cannot reach one) nor under the water.
    for unit in ["ARMSTUMP", "CORRAID", "ARMBULL", "CORREAP", "ARMGUARD", "CORPUN"] {
        expect(unit, 0, &["tank", "building", "boat"]);
    }
    // Submarines: what is in the water.
    for unit in ["ARMSUB", "CORSUB"] {
        expect(unit, 0, &["boat", "sub"]);
    }
    // The cruisers' depth charges: submarines.
    for unit in ["ARMCRUS", "CORCRUS"] {
        expect(unit, 1, &["sub"]);
    }
    assert!(wrong.is_empty(), "{} units against TA's rules:\n{}", wrong.len(), wrong.join("\n"));

    // On an attack order: the second and third weapons that leave the order to the others for
    // these kinds of target, and keep choosing their own (`Sim::keeps_to_own_targets`).
    let keeps = |sim: &Sim, unit: &str, slot: usize| -> Vec<&'static str> {
        let t = types.by_name(unit).unwrap();
        let w = weapons.get(t.weapons[slot].unwrap());
        targets.iter().filter(|&&(_, id)| sim.keeps_to_own_targets(t, slot, w, sim.unit(id).unwrap().type_id, 0)).map(|&(kind, _)| kind).collect()
    };
    let mut table = Vec::new();
    for t in &types.types {
        for slot in 1..3 {
            let Some(wid) = t.weapons[slot] else { continue };
            let kinds = keeps(&sim, &t.name, slot);
            if !kinds.is_empty() {
                table.push(format!("{:<9} weapon {} {:<20} keeps to its own targets on an order on: {}", t.name, slot + 1, weapons.get(wid).name, kinds.join(", ")));
            }
        }
    }
    println!("\n{}", table.join("\n"));
    let mut wrong = Vec::new();
    let mut expect = |unit: &str, slot: usize, want: &[&str]| {
        let got = keeps(&sim, unit, slot);
        if got != want {
            wrong.push(format!("{unit} weapon {} keeps to its own on {got:?}, expected {want:?}", slot + 1));
        }
    };
    // The Skeeter's and the Searcher's anti-air missiles: everything but an aircraft (a submarine
    // too, though they could not reach it).
    for unit in ["ARMPT", "CORPT", "ARMLATNK", "ARMMSHIP", "CORMSHIP"] {
        expect(unit, 1, &["tank", "building", "boat", "sub"]);
    }
    // The destroyers' and cruisers' depth charges (`NOTSUB`): what TA tags as no submarine, which
    // is all but the submarines and a few amphibians.
    for unit in ["ARMROY", "CORROY", "ARMCRUS", "CORCRUS"] {
        let got = keeps(&sim, unit, 1);
        assert!(got.contains(&"boat") && !got.contains(&"sub"), "{unit}'s depth charges: {got:?}");
    }
    // The first weapon always takes the order; the Commanders' D-gun (command fire) too.
    for t in &types.types {
        if let Some(w) = t.weapons[0] {
            assert!(targets.iter().all(|&(_, id)| !sim.keeps_to_own_targets(t, 0, weapons.get(w), sim.unit(id).unwrap().type_id, 0)), "{} weapon 1", t.name);
        }
    }
    for unit in ["ARMCOM", "CORCOM"] {
        expect(unit, 2, &[]);
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Fire at will, as a player found it in TA's own units: a missile tower with no aircraft about
/// fires on a tank, and on an aircraft first when one comes; a destroyer's depth charges take on
/// an amphibious tank under the water; a scout ship's missiles a boat; a destroyer at sea shells
/// a tower on the shore.
#[test]
fn tas_units_fire_at_will_on_their_bad_targets_last() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(ta_data::GameData::load(&vfs));
    let types = Arc::new(crate::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = ta_formats::tnt::TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = ta_data::MapInfo::from_ota(&ta_formats::tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    let new_game = || {
        let players: Vec<_> = (0..2)
            .map(|i| crate::PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
            .collect();
        let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 3);
        sim.los_enabled = false;
        sim
    };
    let put = |sim: &mut Sim, name: &str, owner: u8, at: FxVec3| sim.spawn_unit(types.id(name).unwrap(), owner, at, 0, true).unwrap();
    let hold = |sim: &mut Sim, id: UnitId| {
        let u = sim.unit_mut(id).unwrap();
        u.fire_state = 0;
        u.move_state = 0;
    };
    // Where the land meets the sea along z = 1000 (Coast To Coast: land in the west).
    let shore = (400..1600).step_by(8).find(|&x| {
        let sim = new_game();
        sim.world.depth_at(Fx::from_int(x), Fx::from_int(1000)) > Fx::ZERO
    }).unwrap();
    let land = FxVec3::from_ints(shore - 160, 0, 1000);
    let sea = FxVec3::from_ints(shore + 320, 0, 1000);
    // The weapon in `slot` of `shooter`, after `ticks`, firing at will.
    let aim = |sim: &mut Sim, shooter: UnitId, slot: usize, ticks: u32| {
        for _ in 0..ticks {
            sim.step(&[]);
        }
        sim.unit(shooter).unwrap().weapons[slot].target
    };

    // The Defender, with only a tank in reach: the tank.
    let mut sim = new_game();
    let tower = put(&mut sim, "ARMRL", 0, land);
    let tank = put(&mut sim, "CORRAID", 1, land + FxVec3::from_ints(-120, 0, 60));
    hold(&mut sim, tank);
    assert_eq!(aim(&mut sim, tower, 0, 90), Some(Target::Unit(tank)), "a missile tower with no aircraft about fires on a tank");
    // ... and an aircraft first, once one flies into reach.
    let plane = put(&mut sim, "CORVENG", 1, land + FxVec3::from_ints(-60, 0, -120));
    {
        let u = sim.unit_mut(plane).unwrap();
        u.landed = false;
        u.pos.y += Fx::from_int(120);
        u.fire_state = 0;
        u.move_state = 0;
    }
    let mut turned = false;
    for _ in 0..60 {
        sim.step(&[]);
        if sim.unit(plane).is_some_and(|p| sim.airborne(p)) && sim.unit(tower).unwrap().weapons[0].target == Some(Target::Unit(plane)) {
            turned = true;
            break;
        }
    }
    assert!(turned, "and turns to the aircraft: {:?}", sim.unit(tower).unwrap().weapons[0].target);

    // The Enforcer's depth charges, an amphibious tank under the water beside it.
    let mut sim = new_game();
    let destroyer = put(&mut sim, "CORROY", 1, sea);
    let phib = put(&mut sim, "ARMCROC", 0, sea + FxVec3::from_ints(140, 0, 0));
    hold(&mut sim, phib);
    for _ in 0..30 { sim.step(&[]); }
    assert!(sim.submerged(sim.unit(phib).unwrap()), "the Triton is under the water");
    assert_eq!(aim(&mut sim, destroyer, 1, 90), Some(Target::Unit(phib)), "the destroyer's depth charges take on the amphibious tank");

    // The Skeeter's missiles, a boat beside it.
    let mut sim = new_game();
    let skeeter = put(&mut sim, "ARMPT", 0, sea);
    let boat = put(&mut sim, "CORPT", 1, sea + FxVec3::from_ints(200, 0, 0));
    hold(&mut sim, boat);
    assert_eq!(aim(&mut sim, skeeter, 1, 90), Some(Target::Unit(boat)), "a scout ship's missiles take on a boat");

    // The Enforcer at sea, a tower on the shore.
    let mut sim = new_game();
    let destroyer = put(&mut sim, "CORROY", 1, sea);
    let llt = put(&mut sim, "ARMLLT", 0, land + FxVec3::from_ints(80, 0, 0));
    hold(&mut sim, llt);
    let start = sim.unit(llt).unwrap().health;
    let mut fired = false;
    for _ in 0..600 {
        sim.step(&[]);
        fired |= sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == destroyer));
        if fired && sim.unit(llt).is_none_or(|u| u.health < start) { break; }
    }
    assert!(fired, "the destroyer shells the tower on the shore: target {:?}, blocked {:?}", sim.unit(destroyer).unwrap().weapons[0].target, sim.unit(destroyer).unwrap().weapons[0].blocked);
}

/// TA's own Targeting Facility (`ARMTARG`, "Automatic Radar Targeting") at work: a Big Bertha
/// set to fire at will shells what a radar tower shows of the enemy, far out of anything's sight;
/// with no facility, or with it switched off, it does not.
#[test]
fn tas_targeting_facility_lets_a_big_bertha_fire_at_will_on_radar() {
    let Some(mut sim) = tas_battle(0, 60) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let bertha = put_at(&mut sim, "ARMBRTHA", 0, 300, 1000);
    put_at(&mut sim, "ARMRAD", 0, 420, 1150);
    // 1,200 px off: in the Bertha's reach (4,096) and the radar's (1,440), out of all sight.
    let target = put_at(&mut sim, "CORLLT", 1, 1500, 1000);
    sim.unit_mut(target).unwrap().fire_state = 0;
    // (Built, the Bertha holds its fire: `NoAutoFire`.)
    sim.step(&[crate::Command::SetStates { player: 0, units: vec![bertha], fire: Some(2), movement: None }]);
    sim.step(&[]);
    assert_eq!(sim.seen_by(sim.unit(target).unwrap(), 0), crate::vision::Seen::Radar, "a radar contact");
    // Whether the Bertha fires within 15 s (its reload is 7, its turret turns first).
    let fires = |sim: &mut Sim| {
        sim.take_events();
        (0..450).any(|_| {
            sim.step(&[]);
            fired(sim, bertha)
        })
    };
    assert!(!fires(&mut sim), "no facility: the radar contact is let be");
    let facility = put_at(&mut sim, "ARMTARG", 0, 300, 1400);
    assert!(fires(&mut sim), "the facility at work: the Bertha fires at will on the radar contact");
    assert!(sim.players[0].radar_targeting);
    sim.step(&[crate::Command::SetActive { player: 0, units: vec![facility], on: false }]);
    sim.step(&[]);
    assert!(!fires(&mut sim), "switched off: it holds its fire again");
    assert!(!sim.players[0].radar_targeting);
}

/// A second weapon whose own bad targets name what its unit is ordered to attack (TA's Skeeter:
/// its anti-air missiles say `wsec_badTargetCategory=NOTAIR`) leaves the order to the first and
/// keeps choosing its own targets, as with no order: the laser fires at the ordered boat, the
/// missiles at the aircraft about, and with none at the nearest enemy. The unit closes on the
/// boat to its laser's reach, not its missiles'. A weapon's bad targets taken from the unit's
/// `BadTargetCategory` don't count, nor any with the player's `+shootall`; a command-fire weapon
/// fires as ordered.
#[test]
fn a_second_weapon_keeps_to_its_own_targets_on_an_attack_order() {
    let units = [
        ("SKEETER", "Weapon1=SHORTLASER; Weapon2=MISSILE; wsec_badTargetCategory=NOTAIR; Category=SHIP NOTAIR; CanMove=1; MaxVelocity=2; Acceleration=1; BrakeRate=1; TurnRate=4000; SteeringMode=2;"),
        ("TWIN", "Weapon1=SHORTLASER; Weapon2=MISSILE; BadTargetCategory=NOTAIR; Category=SHIP NOTAIR;"),
        ("BOSS", "Weapon1=SHORTLASER; Weapon3=DGUN; wspe_badTargetCategory=NOTAIR; Category=SHIP NOTAIR;"),
        ("BOAT", "CanMove=1; MaxVelocity=1; Category=SHIP NOTAIR;"),
        ("JET", "CanFly=1; CanMove=1; MaxVelocity=5; Category=VTOL;"),
    ];
    let new_game = || {
        let mut sim = game_of(&units);
        sim.los_enabled = false;
        sim
    };
    let calm = |sim: &mut Sim, id: UnitId| {
        let u = sim.unit_mut(id).unwrap();
        u.fire_state = 0;
        u.move_state = 0;
    };
    let fly = |sim: &mut Sim, id: UnitId| {
        let u = sim.unit_mut(id).unwrap();
        u.landed = false;
        u.pos.y += Fx::from_int(120);
    };
    let attack = |sim: &mut Sim, unit: UnitId, target: UnitId| {
        sim.step(&[crate::Command::Order { player: 0, units: vec![unit], order: Order::Attack(Target::Unit(target)), queue: false }]);
    };
    let targets = |sim: &Sim, id: UnitId| [0, 1, 2].map(|k| sim.unit(id).unwrap().weapons[k].target);

    // The Skeeter, the ordered boat 500 px off (out of its laser's reach, in its missiles'),
    // another boat nearer, an aircraft about.
    let mut sim = new_game();
    let skeeter = spawn(&mut sim, "SKEETER", 0, 300, 600);
    let ordered = spawn(&mut sim, "BOAT", 1, 800, 600);
    let nearer = spawn(&mut sim, "BOAT", 1, 300, 760);
    let jet = spawn(&mut sim, "JET", 1, 420, 760);
    for id in [ordered, nearer, jet] {
        calm(&mut sim, id);
    }
    fly(&mut sim, jet);
    attack(&mut sim, skeeter, ordered);
    let laser = sim.weapon_types.by_name("SHORTLASER").unwrap().id;
    let lasered = (0..360).any(|_| {
        sim.step(&[]);
        sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, weapon, .. } if *unit == skeeter && *weapon == laser))
    });
    assert!(lasered, "the Skeeter closes to its laser's reach and lasers the ordered boat: at {:?}", sim.unit(skeeter).unwrap().pos);
    let [laser, missiles, _] = targets(&sim, skeeter);
    assert_eq!(laser, Some(Target::Unit(ordered)), "the laser fires as ordered");
    assert_eq!(missiles, Some(Target::Unit(jet)), "the missiles at the aircraft about");
    // The aircraft gone: the missiles at the nearest enemy, as with no order.
    sim.remove_unit(jet);
    for _ in 0..30 {
        sim.step(&[]);
    }
    let [laser, missiles, _] = targets(&sim, skeeter);
    assert_eq!(laser, Some(Target::Unit(ordered)), "the laser keeps to the order");
    let from = sim.unit(skeeter).unwrap().pos;
    let nearest = [ordered, nearer].into_iter().min_by_key(|&b| (sim.unit(b).unwrap().pos - from).xz().length_sq_raw()).unwrap();
    assert_eq!(missiles, Some(Target::Unit(nearest)), "and the missiles at the nearest enemy");
    // With `+shootall` no target is a bad one: the missiles take the order too.
    sim.step(&[crate::Command::SetShootAll { player: 0, on: true }]);
    for _ in 0..30 {
        sim.step(&[]);
    }
    assert_eq!(targets(&sim, skeeter)[1], Some(Target::Unit(ordered)), "+shootall: the missiles as ordered");

    // The unit's `BadTargetCategory` alone: both weapons take the order.
    let mut sim = new_game();
    let twin = spawn(&mut sim, "TWIN", 0, 300, 600);
    let ordered = spawn(&mut sim, "BOAT", 1, 440, 600);
    let jet = spawn(&mut sim, "JET", 1, 300, 760);
    calm(&mut sim, ordered);
    calm(&mut sim, jet);
    fly(&mut sim, jet);
    attack(&mut sim, twin, ordered);
    for _ in 0..30 {
        sim.step(&[]);
    }
    assert_eq!(targets(&sim, twin)[..2], [Some(Target::Unit(ordered)); 2], "a weapon's bad targets from its unit's list don't keep it from the order");

    // A command-fire third weapon whose own bad targets name the target fires as ordered.
    let mut sim = new_game();
    let boss = spawn(&mut sim, "BOSS", 0, 300, 600);
    let ordered = spawn(&mut sim, "BOAT", 1, 440, 600);
    calm(&mut sim, ordered);
    sim.step(&[crate::Command::Order { player: 0, units: vec![boss], order: Order::ManualFire(Target::Unit(ordered)), queue: false }]);
    let dgun = sim.weapon_types.by_name("DGUN").unwrap().id;
    let shot = (0..60).any(|_| {
        sim.step(&[]);
        sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, weapon, .. } if *unit == boss && *weapon == dgun))
    });
    assert!(shot, "the command-fire weapon fires as ordered");
}

/// TA's Skeeter ordered to attack a Searcher, a Core Avenger flying near and another boat
/// nearer: its laser fires at the Searcher, its missiles at the Avenger.
#[test]
fn tas_skeeter_lasers_what_it_is_ordered_to_and_its_missiles_take_the_aircraft() {
    let Some(mut sim) = tas_battle(150, 60) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    sim.los_enabled = false;
    let skeeter = put_at(&mut sim, "ARMPT", 0, 600, 1000);
    let ordered = put_at(&mut sim, "CORPT", 1, 760, 1000);
    let nearer = put_at(&mut sim, "CORPT", 1, 600, 1120);
    let plane = put_at(&mut sim, "CORVENG", 1, 700, 1150);
    for id in [ordered, nearer, plane] {
        let u = sim.unit_mut(id).unwrap();
        u.fire_state = 0;
        u.move_state = 0;
    }
    {
        let u = sim.unit_mut(plane).unwrap();
        u.landed = false;
        u.pos.y += Fx::from_int(120);
    }
    sim.step(&[crate::Command::Order { player: 0, units: vec![skeeter], order: Order::Attack(Target::Unit(ordered)), queue: false }]);
    let mut at = [None; 2];
    for _ in 0..90 {
        sim.step(&[]);
        let u = sim.unit(skeeter).unwrap();
        at = [u.weapons[0].target, u.weapons[1].target];
        if at == [Some(Target::Unit(ordered)), Some(Target::Unit(plane))] && sim.unit(plane).is_some_and(|p| sim.airborne(p)) {
            break;
        }
    }
    assert_eq!(at, [Some(Target::Unit(ordered)), Some(Target::Unit(plane))], "the laser as ordered, the missiles at the aircraft");
}
