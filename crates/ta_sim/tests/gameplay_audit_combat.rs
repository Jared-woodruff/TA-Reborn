//! Combat regressions on original, synthetic definitions; no TA install is required.

use std::sync::{Arc, atomic::{AtomicU32, Ordering}};

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Projectile, Sim, SimEvent, Target, UnitId, UnitTypes};

fn unit(name: &str, keys: &str) -> (String, String) {
    (format!("units/{name}.fbi"), format!("[UNITINFO] {{ UnitName={name}; MaxDamage=1000; SightDistance=600; {keys} }}"))
}

fn weapon(name: &str, keys: &str) -> (String, String) {
    (format!("weapons/{name}.tdf"), format!("[{name}] {{ range=400; reloadtime=0.1; weaponvelocity=400; {keys} [DAMAGE] {{ default=1; }} }}"))
}

fn game(files: &[(String, String)]) -> Sim {
    let map = TntMap { width: 64, height: 64, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; 64 * 64], features: vec![FEATURE_NONE; 64 * 64], feature_names: Vec::new(), minimap: Default::default() };
    game_on(files, map)
}

fn game_on(files: &[(String, String)], map: TntMap) -> Sim {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta-gameplay-combat-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_mod_dir(&dir, "test").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    std::fs::remove_dir_all(dir).unwrap();
    let types = Arc::new(UnitTypes::compile(&data, &|_| None));
    let players: Vec<_> = (0..2).map(|i| PlayerSetup { side: "ARM".into(), color: i, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
    let mut sim = Sim::new(types, data, &map, &MapInfo::default(), 0, &players, 7);
    for player in &mut sim.players { player.endless = true; }
    sim
}

fn spawn(sim: &mut Sim, name: &str, owner: u8, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), owner, FxVec3::from_ints(x, 0, z), 0, true).unwrap()
}

fn shots(sim: &mut Sim, shooter: UnitId, ticks: usize) -> usize {
    sim.take_events();
    for _ in 0..ticks { sim.step(&[]); }
    sim.take_events().iter().filter(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == shooter)).count()
}

fn interception_pair(anti_keys: &str, warhead_pos: FxVec3, warhead_vel: FxVec3, anti_pos: FxVec3, anti_vel: FxVec3) -> (Sim, usize, usize) {
    let mut sim = game(&[
        unit("DEFENDER", "Weapon1=ANTI;"), unit("SILO", "Weapon1=NUKE;"), unit("OBSTACLE", ""),
        weapon("ANTI", &format!("interceptor=1; stockpile=1; reloadtime=1000; {anti_keys}")),
        weapon("NUKE", "stockpile=1; targetable=1; commandfire=1; reloadtime=1000;"),
    ]);
    let defender = spawn(&mut sim, "DEFENDER", 0, 100, 100);
    let silo = spawn(&mut sim, "SILO", 1, 900, 900);
    let anti = sim.unit_type(sim.unit(defender).unwrap()).weapons[0].unwrap();
    let warhead = sim.unit_type(sim.unit(silo).unwrap()).weapons[0].unwrap();
    sim.projectiles.extend([
        Projectile { id: 100, weapon: warhead, owner: silo, owner_player: 1, pos: warhead_pos, prev_pos: warhead_pos, vel: warhead_vel, target: None, target_pos: warhead_pos, age: 3, ttl: 1000, hits: Vec::new(), dead: false },
        Projectile { id: 101, weapon: anti, owner: defender, owner_player: 0, pos: anti_pos, prev_pos: anti_pos, vel: anti_vel, target: Some(Target::Projectile(100)), target_pos: warhead_pos, age: 3, ttl: 1000, hits: Vec::new(), dead: false },
    ]);
    sim.take_events();
    (sim, anti, warhead)
}

#[test]
fn missiles_flying_apart_are_not_intercepted_by_a_distance_shortcut() {
    let (mut sim, _, _) = interception_pair("", FxVec3::from_ints(500, 500, 500), FxVec3::from_ints(100, 0, 0), FxVec3::from_ints(450, 500, 500), FxVec3::from_ints(-100, 0, 0));
    sim.step(&[]);
    assert!(!sim.take_events().iter().any(|e| matches!(e, SimEvent::Intercepted { .. })));
    assert_eq!(sim.projectiles.len(), 2);
}

#[test]
fn fast_crossing_missiles_intercept_between_tick_endpoints_in_either_order() {
    for reverse in [false, true] {
        let (mut sim, _, warhead) = interception_pair("", FxVec3::from_ints(500, 500, 500), FxVec3::from_ints(-200, 0, 0), FxVec3::from_ints(300, 500, 500), FxVec3::from_ints(100, 0, 0));
        if reverse { sim.projectiles.reverse(); }
        sim.step(&[]);
        let events = sim.take_events();
        assert_eq!(events.iter().filter(|e| matches!(e, SimEvent::Intercepted { .. })).count(), 1, "crossing flights meet during the tick (reverse={reverse})");
        assert!(!events.iter().any(|e| matches!(e, SimEvent::Explosion { weapon, .. } if *weapon == warhead)));
        assert!(sim.projectiles.is_empty());
    }
}

#[test]
fn an_interceptor_cannot_cancel_an_earlier_ground_impact() {
    for reverse in [false, true] {
        // Ground impact is at 0.2 of the tick; the flights only enter proximity at 0.3.
        let (mut sim, _, warhead) = interception_pair("", FxVec3::from_ints(500, 80, 500), FxVec3::from_ints(0, -100, 0), FxVec3::from_ints(500, 180, 500), FxVec3::from_ints(0, -300, 0));
        if reverse { sim.projectiles.reverse(); }
        sim.step(&[]);
        let events = sim.take_events();
        assert!(events.iter().any(|e| matches!(e, SimEvent::Explosion { weapon, .. } if *weapon == warhead)), "the warhead hits first (reverse={reverse})");
        assert!(!events.iter().any(|e| matches!(e, SimEvent::Intercepted { .. })));
    }
}

#[test]
fn interceptor_flights_do_not_collide_with_ordinary_units() {
    for owner in [0, 1] {
        let (mut sim, anti, _) = interception_pair("tracks=1; turnrate=32768; weaponvelocity=3000;", FxVec3::from_ints(700, 68, 300), FxVec3::ZERO, FxVec3::from_ints(300, 68, 300), FxVec3::from_ints(100, 0, 0));
        let obstacle = spawn(&mut sim, "OBSTACLE", owner, 350, 300);
        sim.step(&[]);
        assert!(sim.projectiles.iter().any(|p| p.id == 101), "an ordinary unit cannot consume an anti-missile (owner={owner})");
        assert_eq!(sim.unit(obstacle).unwrap().health, Fx::from_int(1000));
        assert!(!sim.take_events().iter().any(|e| matches!(e, SimEvent::Explosion { weapon, .. } if *weapon == anti)));
    }
}

#[test]
fn non_stockpiled_interceptors_fire_and_pay_per_shot() {
    let mut sim = game(&[
        unit("DEFENDER", "Weapon1=ANTI;"), unit("SILO", "Weapon1=NUKE;"),
        weapon("ANTI", "interceptor=1; coverage=1000; energypershot=400; metalpershot=20; reloadtime=1000;"),
        weapon("NUKE", "stockpile=1; commandfire=1; targetable=1; reloadtime=1000;"),
    ]);
    let defender = spawn(&mut sim, "DEFENDER", 0, 300, 300);
    let silo = spawn(&mut sim, "SILO", 1, 900, 900);
    let pos = FxVec3::from_ints(600, 500, 300);
    sim.projectiles.push(Projectile { id: 100, weapon: sim.unit_type(sim.unit(silo).unwrap()).weapons[0].unwrap(), owner: silo, owner_player: 1, pos, prev_pos: pos, vel: FxVec3::ZERO, target: None, target_pos: FxVec3::from_ints(300, 60, 300), age: 3, ttl: 1000, hits: Vec::new(), dead: false });
    assert_eq!(shots(&mut sim, defender, 5), 1);
    assert_eq!((sim.players[0].stats.metal_spent, sim.players[0].stats.energy_spent), (Fx::from_int(20), Fx::from_int(400)));
}

#[test]
fn orphaned_or_newly_allied_interceptor_targets_retire_without_ground_damage() {
    for allied in [false, true] {
        let (mut sim, anti, _) = interception_pair("", FxVec3::from_ints(700, 500, 300), FxVec3::ZERO, FxVec3::from_ints(300, 70, 300), FxVec3::from_ints(0, -20, 0));
        if allied { sim.set_team(1, 0); } else { sim.projectiles.retain(|p| p.id != 100); }
        sim.step(&[]);
        assert!(!sim.projectiles.iter().any(|p| p.id == 101));
        assert!(!sim.take_events().iter().any(|e| matches!(e, SimEvent::Explosion { weapon, .. } if *weapon == anti)));
    }
}

#[test]
fn all_ready_interceptor_slots_can_defend_against_a_salvo() {
    let mut sim = game(&[
        unit("DEFENDER", "Weapon1=ANTI; Weapon2=ANTI;"), unit("SILO", "Weapon1=NUKE;"),
        weapon("ANTI", "interceptor=1; stockpile=1; coverage=1000; reloadtime=1000;"),
        weapon("NUKE", "stockpile=1; commandfire=1; targetable=1; reloadtime=1000;"),
    ]);
    let defender = spawn(&mut sim, "DEFENDER", 0, 300, 300);
    let silo = spawn(&mut sim, "SILO", 1, 900, 900);
    sim.grant_stockpile(defender, 2);
    for id in [100, 101] {
        let pos = FxVec3::from_ints(700, 500, 300 + (id - 100) as i64 * 100);
        sim.projectiles.push(Projectile { id, weapon: sim.unit_type(sim.unit(silo).unwrap()).weapons[0].unwrap(), owner: silo, owner_player: 1, pos, prev_pos: pos, vel: FxVec3::ZERO, target: None, target_pos: FxVec3::from_ints(300, 60, 300), age: 3, ttl: 1000, hits: Vec::new(), dead: false });
    }
    assert_eq!(shots(&mut sim, defender, 5), 2, "each ready slot can reserve a different warhead");
    assert_eq!(sim.unit(defender).unwrap().stockpile, 0);
}

#[test]
fn a_warhead_leaving_the_map_cannot_be_intercepted_after_its_boundary_crossing() {
    let (mut sim, _, warhead) = interception_pair("", FxVec3::from_ints(1000, 500, 500), FxVec3::from_ints(100, 0, 0), FxVec3::from_ints(900, 500, 500), FxVec3::from_ints(200, 0, 0));
    sim.step(&[]);
    let events = sim.take_events();
    assert!(!events.iter().any(|e| matches!(e, SimEvent::Intercepted { .. })));
    let at = events.iter().find_map(|e| match e { SimEvent::Explosion { weapon, pos, .. } if *weapon == warhead => Some(pos.x), _ => None }).unwrap();
    assert!((at - sim.world.size_px().0).abs() < Fx::from_ratio(1, 100), "the missile retires at the boundary, not beyond it");
}

#[test]
fn a_snapshot_continues_an_in_flight_interception_deterministically() {
    let (mut sim, _, _) = interception_pair("", FxVec3::from_ints(700, 500, 300), FxVec3::from_ints(-1, 0, 0), FxVec3::from_ints(300, 500, 300), FxVec3::from_ints(10, 0, 0));
    for _ in 0..5 { sim.step(&[]); }
    let mut restored = sim.clone();
    for _ in 0..40 {
        sim.step(&[]);
        restored.step(&[]);
        assert_eq!(sim.checksum(), restored.checksum());
    }
    assert_eq!(sim.take_events().iter().filter(|e| matches!(e, SimEvent::Intercepted { .. })).count(), 1);
    assert_eq!(restored.take_events().iter().filter(|e| matches!(e, SimEvent::Intercepted { .. })).count(), 1);
}

#[test]
fn interceptors_never_fire_at_units_or_ground_even_when_ordered() {
    for command_fire in [0, 1] {
        let mut sim = game(&[
            unit("DEFENDER", "Weapon1=ANTI;"), unit("TARGET", ""),
            weapon("ANTI", &format!("interceptor=1; stockpile=1; commandfire={command_fire}; reloadtime=1000;")),
        ]);
        let defender = spawn(&mut sim, "DEFENDER", 0, 300, 300);
        let target = spawn(&mut sim, "TARGET", 1, 440, 300);
        sim.grant_stockpile(defender, 5);
        assert_eq!(shots(&mut sim, defender, 24), 0, "no automatic unit targets");
        for order in [Order::Attack(Target::Unit(target)), Order::Attack(Target::Ground(FxVec3::from_ints(440, 60, 300))), Order::ManualFire(Target::Unit(target))] {
            sim.step(&[Command::Order { player: 0, units: vec![defender], order, queue: false }]);
            assert_eq!(shots(&mut sim, defender, 24), 0, "no ordered unit/ground targets");
        }
        assert_eq!(sim.unit(defender).unwrap().stockpile, 5, "no missiles wasted");
        assert!(sim.unit(defender).unwrap().weapons[0].target.is_none());
    }
}

#[test]
fn stockpiles_pay_the_full_authored_missile_cost_over_the_build_time() {
    for (keys, metal, energy, seconds) in [
        ("targetable=1; commandfire=1;", 2000, 180000, 180),
        ("interceptor=1; coverage=2000;", 200, 10000, 120),
    ] {
        let mut sim = game(&[
            unit("SILO", "Weapon1=MISSILE;"),
            weapon("MISSILE", &format!("stockpile=1; {keys} metalpershot={metal}; energypershot={energy}; reloadtime={seconds};")),
        ]);
        let silo = spawn(&mut sim, "SILO", 0, 300, 300);
        let initial = (Fx::from_int(metal + 100), Fx::from_int(energy + 100));
        sim.players[0].base_storage = initial;
        sim.players[0].economy.metal = initial.0;
        sim.players[0].economy.energy = initial.1;
        let flow = sim.unit_flows(sim.unit(silo).unwrap());
        assert_eq!((flow.metal_use, flow.energy_use), (Fx::from_ratio(metal, seconds), Fx::from_ratio(energy, seconds)));
        for _ in 0..seconds * 30 { sim.step(&[]); }
        assert_eq!(sim.unit(silo).unwrap().stockpile, 1);
        let e = &sim.players[0].economy;
        assert_eq!((initial.0 - e.metal, initial.1 - e.energy), (Fx::from_int(metal), Fx::from_int(energy)));
        assert_eq!((sim.players[0].stats.metal_spent, sim.players[0].stats.energy_spent), (Fx::from_int(metal), Fx::from_int(energy)));
    }
}

#[test]
fn missile_production_cannot_borrow_resources_on_its_first_tick() {
    for empty_metal in [true, false] {
        let mut sim = game(&[
            unit("SILO", "Weapon1=MISSILE;"),
            weapon("MISSILE", "stockpile=1; commandfire=1; metalpershot=2000; energypershot=180000; reloadtime=180;"),
        ]);
        let silo = spawn(&mut sim, "SILO", 0, 300, 300);
        let e = &mut sim.players[0].economy;
        if empty_metal { e.metal = Fx::ZERO; } else { e.energy = Fx::ZERO; }
        for _ in 0..60 { sim.step(&[]); }
        let u = sim.unit(silo).unwrap();
        assert_eq!((u.stockpile, u.stockpile_work), (0, 0), "no free progress with an empty store");
        assert_eq!((sim.players[0].stats.metal_spent, sim.players[0].stats.energy_spent), (Fx::ZERO, Fx::ZERO));
    }
}

#[test]
fn competing_silos_pay_exactly_for_fractional_progress_and_resume_after_a_stall() {
    let mut sim = game(&[
        unit("SILO", "Weapon1=MISSILE; OnOffable=1; ActivateWhenBuilt=1;"),
        weapon("MISSILE", "stockpile=1; commandfire=1; metalpershot=31.25; energypershot=333.75; reloadtime=0.7;"),
    ]);
    let silos = [spawn(&mut sim, "SILO", 0, 300, 300), spawn(&mut sim, "SILO", 0, 600, 300)];
    let initial = (Fx::from_ratio(1, 7), Fx::from_ratio(1, 3));
    sim.players[0].economy.metal = initial.0;
    sim.players[0].economy.energy = initial.1;
    for _ in 0..30 { sim.step(&[]); }
    let (e, stats) = (&sim.players[0].economy, &sim.players[0].stats);
    assert_eq!((e.metal + stats.metal_spent, e.energy + stats.energy_spent), initial, "no overdraw lost at the end-of-tick clamp");
    let w = sim.weapon_types.get(sim.unit_type(sim.unit(silos[0]).unwrap()).weapons[0].unwrap());
    let total = (w.reload_ticks as u64) << 16;
    let expected = silos.iter().fold((Fx::ZERO, Fx::ZERO), |(m, e), &id| {
        let work = sim.unit(id).unwrap().stockpile_work;
        (m + w.metal_per_shot.share(work, total), e + w.energy_per_shot.share(work, total))
    });
    assert_eq!((stats.metal_spent, stats.energy_spent), expected, "every fraction of work is paid for");
    sim.players[0].economy.metal = Fx::from_int(1000);
    sim.players[0].economy.energy = Fx::from_int(1000);
    for _ in 0..30 {
        let full: Vec<_> = silos.iter().copied().filter(|&id| sim.unit(id).unwrap().stockpile == 1).collect();
        sim.step(&[Command::SetActive { player: 0, units: full, on: false }]);
    }
    assert!(silos.iter().all(|&id| sim.unit(id).unwrap().stockpile == 1));
    assert_eq!((sim.players[0].stats.metal_spent, sim.players[0].stats.energy_spent), (Fx::from_f64(62.5), Fx::from_f64(667.5)));
    assert!(silos.iter().all(|&id| !sim.unit_flows(sim.unit(id).unwrap()).any()), "switched-off silos show no draw");
}

#[test]
fn airborne_missiles_obey_radar_sight_alliances_and_jamming() {
    use ta_sim::{Seen, vision};
    let mut sim = game(&[unit("SILO", "Weapon1=NUKE;"), weapon("NUKE", "stockpile=1; targetable=1;")]);
    let silo = spawn(&mut sim, "SILO", 1, 900, 900);
    let at = FxVec3::from_ints(500, 500, 500);
    let mut missile = Projectile {
        id: 100, weapon: sim.unit_type(sim.unit(silo).unwrap()).weapons[0].unwrap(), owner: silo, owner_player: 1,
        pos: at, prev_pos: at, vel: FxVec3::ZERO, target: None, target_pos: at,
        age: 1, ttl: 1000, hits: Vec::new(), dead: false,
    };
    sim.los_enabled = true;
    sim.vision.rebuild(&[]);
    assert_eq!(sim.projectile_seen_by(&missile, 0), Seen::Hidden);
    assert_eq!(sim.projectile_seen_by(&missile, 1), Seen::Visible);
    sim.vision.set_all(0, vision::SONAR);
    assert_eq!(sim.projectile_seen_by(&missile, 0), Seen::Hidden, "sonar cannot detect airborne missiles");
    sim.vision.set_all(0, vision::RADAR);
    sim.remove_unit(silo);
    assert_eq!(sim.projectile_seen_by(&missile, 0), Seen::Radar, "the launcher need not survive");
    sim.vision.set_all(1, vision::RADAR_JAM);
    assert_eq!(sim.projectile_seen_by(&missile, 0), Seen::Hidden);
    sim.vision.set_all(0, vision::LOS);
    assert_eq!(sim.projectile_seen_by(&missile, 0), Seen::Visible, "sight beats jamming");
    sim.vision.rebuild(&[]);
    sim.set_team(1, 0);
    assert_eq!(sim.projectile_seen_by(&missile, 0), Seen::Visible, "allied missiles remain known");
    missile.dead = true;
    assert_eq!(sim.projectile_seen_by(&missile, 0), Seen::Hidden);
}

#[test]
fn interceptors_only_engage_enemy_targetable_missiles_inside_their_protection_radius() {
    for (targetable, owner, covered) in [(false, 1, true), (true, 0, true), (true, 1, false), (true, 1, true)] {
        let mut sim = game(&[
            unit("DEFENDER", "Weapon1=ANTI;"), unit("SILO", "Weapon1=NUKE;"),
            weapon("ANTI", "interceptor=1; stockpile=1; range=32000; coverage=100; reloadtime=1000;"),
            weapon("NUKE", &format!("targetable={}; stockpile=1; commandfire=1;", u8::from(targetable))),
        ]);
        let defender = spawn(&mut sim, "DEFENDER", 0, 300, 300);
        let silo = spawn(&mut sim, "SILO", owner, 900, 900);
        sim.grant_stockpile(defender, 1);
        let pos = FxVec3::from_ints(700, 500, 300);
        sim.projectiles.push(Projectile {
            id: 100, weapon: sim.unit_type(sim.unit(silo).unwrap()).weapons[0].unwrap(), owner: silo, owner_player: owner,
            pos, prev_pos: pos, vel: FxVec3::ZERO, target: None,
            target_pos: FxVec3::from_ints(if covered { 300 } else { 600 }, 60, 300),
            age: 0, ttl: 1000, hits: Vec::new(), dead: false,
        });
        assert_eq!(shots(&mut sim, defender, 5), usize::from(targetable && owner == 1 && covered));
    }
}

#[test]
fn automatic_targets_are_released_when_captured_or_allied() {
    for capture in [true, false] {
        let mut sim = game(&[unit("GUN", "Weapon1=SHELL;"), unit("TARGET", ""), weapon("SHELL", "ballistic=1;")]);
        let gun = spawn(&mut sim, "GUN", 0, 300, 300);
        let target = spawn(&mut sim, "TARGET", 1, 440, 300);
        assert!(shots(&mut sim, gun, 24) > 0, "the gun first engages the enemy");
        assert_eq!(sim.unit(gun).unwrap().weapons[0].target, Some(Target::Unit(target)));
        if capture { sim.transfer_unit(target, 0); } else { sim.set_team(1, 0); }
        assert_eq!(shots(&mut sim, gun, 24), 0, "a new ally must no longer be an automatic target (capture={capture})");
        assert!(sim.unit(gun).unwrap().weapons[0].target.is_none());
        // Deliberately ordering friendly fire is still allowed.
        sim.step(&[Command::Order { player: 0, units: vec![gun], order: Order::Attack(Target::Unit(target)), queue: false }]);
        assert!(shots(&mut sim, gun, 24) > 0);
    }
}

#[test]
fn an_expired_interceptor_releases_its_threat_for_another_attempt() {
    let mut sim = game(&[
        unit("DEFENDER", "Weapon1=ANTI;"), unit("SILO", "Weapon1=NUKE;"),
        weapon("ANTI", "interceptor=1; stockpile=1; commandfire=1; coverage=1000; tracks=1; turnrate=1000; flighttime=0.2;"),
        weapon("NUKE", "targetable=1; stockpile=1; commandfire=1;"),
    ]);
    let defender = spawn(&mut sim, "DEFENDER", 0, 300, 300);
    let silo = spawn(&mut sim, "SILO", 1, 900, 900);
    sim.grant_stockpile(defender, 3);
    let pos = FxVec3::from_ints(700, 500, 300);
    sim.projectiles.push(Projectile {
        id: 100, weapon: sim.types.get(sim.types.id("SILO").unwrap()).weapons[0].unwrap(), owner: silo, owner_player: 1,
        pos, prev_pos: pos, vel: FxVec3::ZERO, target: None, target_pos: FxVec3::from_ints(300, 60, 300),
        age: 0, ttl: 1000, hits: Vec::new(), dead: false,
    });
    assert!(shots(&mut sim, defender, 40) >= 2, "the first interceptor cannot reach the threat before expiring; defence must retry");
    assert!(sim.projectiles.iter().any(|p| p.id == 100), "the fixture's threat remains airborne");
}

#[test]
fn interception_prevents_a_warhead_detonating_in_the_same_tick() {
    for reverse in [false, true] {
        let mut sim = game(&[
            unit("DEFENDER", "Weapon1=ANTI;"), unit("SILO", "Weapon1=NUKE;"),
            weapon("ANTI", "interceptor=1; stockpile=1; commandfire=1; tracks=1; turnrate=1000;"),
            weapon("NUKE", "targetable=1; stockpile=1; commandfire=1;"),
        ]);
        let defender = spawn(&mut sim, "DEFENDER", 0, 300, 300);
        let silo = spawn(&mut sim, "SILO", 1, 900, 900);
        let warhead = sim.types.get(sim.types.id("SILO").unwrap()).weapons[0].unwrap();
        let anti = sim.types.get(sim.types.id("DEFENDER").unwrap()).weapons[0].unwrap();
        let pos = FxVec3::from_ints(700, 61, 300);
        let interceptor_pos = pos + FxVec3::from_ints(0, 29, 0);
        sim.projectiles.extend([
            Projectile { id: 100, weapon: warhead, owner: silo, owner_player: 1, pos, prev_pos: pos, vel: FxVec3::from_ints(0, -10, 0), target: None, target_pos: FxVec3::from_ints(700, 60, 300), age: 3, ttl: 1000, hits: Vec::new(), dead: false },
            Projectile { id: 101, weapon: anti, owner: defender, owner_player: 0, pos: interceptor_pos, prev_pos: interceptor_pos, vel: FxVec3::from_ints(0, -10, 0), target: Some(Target::Projectile(100)), target_pos: pos, age: 3, ttl: 1000, hits: Vec::new(), dead: false },
        ]);
        if reverse { sim.projectiles.reverse(); }
        sim.take_events();
        sim.step(&[]);
        let events = sim.take_events();
        assert!(events.iter().any(|e| matches!(e, SimEvent::Intercepted { .. })));
        assert!(!events.iter().any(|e| matches!(e, SimEvent::Explosion { weapon, .. } if *weapon == warhead)), "an intercepted warhead cannot also detonate (reverse={reverse})");
        assert!(sim.projectiles.is_empty());
    }
}

#[test]
fn fast_projectiles_hit_the_near_face_of_a_ridge_between_tick_endpoints() {
    for reverse in [false, true] {
        let mut map = TntMap { width: 64, height: 64, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; 64 * 64], features: vec![FEATURE_NONE; 64 * 64], feature_names: Vec::new(), minimap: Default::default() };
        for z in 0..64 { map.heights[z * 64 + 15] = 120; }
        let mut sim = game_on(&[unit("GUN", "Weapon1=SHOT;"), weapon("SHOT", "commandfire=1;")], map);
        let shooter = spawn(&mut sim, "GUN", 0, 100, 300);
        let wid = sim.types.get(sim.types.id("GUN").unwrap()).weapons[0].unwrap();
        let pos = FxVec3::from_ints(if reverse { 300 } else { 200 }, 90, 300);
        let vel = FxVec3::from_ints(if reverse { -100 } else { 100 }, 0, 0);
        assert!(pos.y > sim.world.height_at(pos.x, pos.z));
        assert!((pos + vel).y > sim.world.height_at((pos + vel).x, pos.z));
        sim.projectiles.push(Projectile { id: 100, weapon: wid, owner: shooter, owner_player: 0, pos, prev_pos: pos, vel, target: None, target_pos: pos + vel, age: 3, ttl: 1000, hits: Vec::new(), dead: false });
        sim.take_events();
        sim.step(&[]);
        assert!(sim.projectiles.is_empty(), "the shot crosses the ridge, despite both endpoints being above the ground (reverse={reverse})");
        let impact = sim.take_events().into_iter().find_map(|e| match e { SimEvent::Explosion { pos, when, .. } => Some((pos, when)), _ => None }).expect("impact event");
        let expected_x = if reverse { 248 } else { 232 };
        assert!((impact.0.x - Fx::from_int(expected_x)).abs() < Fx::ONE, "the first face is hit: {impact:?}");
        assert!(impact.1 > Fx::ZERO && impact.1 < Fx::ONE);
    }
}

#[test]
fn projectiles_hit_a_triangle_ridge_inside_a_single_map_cell() {
    let mut map = TntMap { width: 64, height: 64, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; 64 * 64], features: vec![FEATURE_NONE; 64 * 64], feature_names: Vec::new(), minimap: Default::default() };
    map.heights[10 * 64 + 10] = 120;
    map.heights[11 * 64 + 11] = 120;
    let mut sim = game_on(&[unit("GUN", "Weapon1=SHOT;"), weapon("SHOT", "commandfire=1;")], map);
    let shooter = spawn(&mut sim, "GUN", 0, 100, 300);
    let wid = sim.types.get(sim.types.id("GUN").unwrap()).weapons[0].unwrap();
    // Both endpoints are above the low corners of one cell, but the diagonal shared by
    // its two terrain triangles stands above the shot. Cell-edge checks alone miss it.
    let pos = FxVec3::from_ints(172, 100, 160);
    let vel = FxVec3::from_ints(-12, 0, 12);
    sim.projectiles.push(Projectile { id: 100, weapon: wid, owner: shooter, owner_player: 0, pos, prev_pos: pos, vel, target: None, target_pos: pos + vel, age: 3, ttl: 1000, hits: Vec::new(), dead: false });
    sim.take_events();
    sim.step(&[]);
    assert!(sim.projectiles.is_empty(), "the shot must not cross the diagonal ridge");
    let when = sim.take_events().into_iter().find_map(|e| match e { SimEvent::Explosion { when, .. } => Some(when), _ => None }).expect("impact event");
    assert!((when - Fx::from_ratio(5, 18)).abs() < Fx::from_ratio(1, 1000), "the first terrain triangle is hit at 5/18 of the tick: {when}");
}
