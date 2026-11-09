//! Scenario rules on synthetic data, independent of an installed copy of TA.
use super::*;
use crate::mission::{Hunter, MissionRules, Objective, Wave};

fn battle() -> (Sim, usize) {
    let data = data_of(&[unit("TANK", "CanMove=1; MaxVelocity=1;")]);
    let sim = sim_of(data, 2);
    let tank = sim.types.id("TANK").unwrap();
    (sim, tank)
}

#[test]
fn separate_hold_goals_cannot_borrow_progress_from_another_area() {
    let (mut sim, tank) = battle();
    let a = FxVec3::from_ints(200, 60, 200);
    let b = FxVec3::from_ints(800, 60, 800);
    sim.spawn_unit(tank, 0, a, 0, true).unwrap();
    sim.mission = Some(MissionRules { victory: vec![
        Objective::HoldArea { pos: a, radius: Fx::from_int(100), ticks: 3000 },
        Objective::HoldArea { pos: b, radius: Fx::from_int(100), ticks: 30 },
    ], ..Default::default() });
    for tick in [15, 30] { sim.tick = tick; sim.check_mission(); }
    assert_eq!(sim.mission.as_ref().unwrap().outcome, None, "the short goal's area was never occupied");
    sim.spawn_unit(tank, 0, b, 0, true).unwrap();
    for tick in [45, 60] { sim.tick = tick; sim.check_mission(); }
    assert_eq!(sim.mission.as_ref().unwrap().outcome, Some(true));
}

#[test]
fn enemy_hold_goals_count_their_own_area() {
    let (mut sim, tank) = battle();
    let a = FxVec3::from_ints(200, 60, 200);
    let b = FxVec3::from_ints(800, 60, 800);
    sim.spawn_unit(tank, 0, a, 0, true).unwrap();
    sim.spawn_unit(tank, 1, b, 0, true).unwrap();
    sim.mission = Some(MissionRules {
        victory: vec![Objective::HoldArea { pos: a, radius: Fx::from_int(100), ticks: 3000 }],
        defeat: vec![Objective::EnemyHoldsArea { pos: b, radius: Fx::from_int(100), ticks: 30 }],
        ..Default::default()
    });
    for tick in [15, 30] { sim.tick = tick; sim.check_mission(); }
    assert_eq!(sim.mission.as_ref().unwrap().outcome, Some(false));
}

#[test]
fn captured_wave_units_keep_their_new_owners_orders() {
    let (mut sim, tank) = battle();
    let pos = FxVec3::from_ints(800, 60, 800);
    let id = sim.spawn_unit(tank, 1, pos, 0, true).unwrap();
    sim.unit_mut(id).unwrap().owner = 0;
    sim.unit_mut(id).unwrap().orders.push_back(Order::Wait(500));
    sim.mission = Some(MissionRules { hunters: vec![Hunter { id, mark: pos, still: 2 }], ..Default::default() });
    sim.tick = 60;
    sim.check_mission();
    assert!(matches!(sim.unit(id).unwrap().orders.front(), Some(Order::Wait(500))));
    assert!(sim.mission.as_ref().unwrap().hunters.is_empty());
}

#[test]
fn carried_wave_units_are_not_retasked_by_the_hunter() {
    let (mut sim, tank) = battle();
    let pos = FxVec3::from_ints(800, 60, 800);
    let id = sim.spawn_unit(tank, 1, pos, 0, true).unwrap();
    let carrier = sim.spawn_unit(tank, 1, FxVec3::from_ints(750, 60, 800), 0, true).unwrap();
    sim.spawn_unit(tank, 0, FxVec3::from_ints(200, 60, 200), 0, true).unwrap();
    sim.unit_mut(id).unwrap().carried_by = Some(carrier);
    sim.unit_mut(id).unwrap().orders.push_back(Order::Wait(500));
    sim.mission = Some(MissionRules { hunters: vec![Hunter { id, mark: pos, still: 2 }], ..Default::default() });
    sim.tick = 60;
    sim.check_mission();
    assert!(matches!(sim.unit(id).unwrap().orders.front(), Some(Order::Wait(500))));
}

#[test]
fn hunter_stall_counters_remain_bounded_without_any_prey() {
    let (mut sim, tank) = battle();
    let pos = FxVec3::from_ints(800, 60, 800);
    let id = sim.spawn_unit(tank, 1, pos, 0, true).unwrap();
    sim.unit_mut(id).unwrap().orders.push_back(Order::Wait(500));
    sim.mission = Some(MissionRules { hunters: vec![Hunter { id, mark: pos, still: u8::MAX }], ..Default::default() });
    sim.tick = 60;
    sim.check_mission();
    assert_eq!(sim.mission.as_ref().unwrap().hunters[0].still, u8::MAX);
}

#[test]
fn scenario_checksums_cover_objectives_progress_and_future_waves() {
    let (mut sim, tank) = battle();
    let pos = FxVec3::from_ints(200, 60, 200);
    let id = sim.spawn_unit(tank, 0, pos, 0, true).unwrap();
    sim.players[1].endless = true;
    sim.mission = Some(MissionRules { victory: vec![Objective::HoldArea { pos, radius: Fx::from_int(100), ticks: 3000 }], ..Default::default() });
    sim.tick = 15;
    let before = sim.checksum();
    sim.check_mission();
    assert_ne!(sim.checksum(), before, "area progress changes the future outcome");
    let before = sim.checksum();
    sim.mission.as_mut().unwrap().victory.push(Objective::Timer(600));
    assert_ne!(sim.checksum(), before);
    let before = sim.checksum();
    sim.mission.as_mut().unwrap().waves.push(Wave { tick: 300, owner: 1, units: vec![(tank, 2)], from: pos, target: None, hunt: true, friendly: false });
    assert_ne!(sim.checksum(), before, "scheduled units must be covered before spawning");
    let before = sim.checksum();
    sim.unit_mut(id).unwrap().last_attacker = Some(id);
    assert_ne!(sim.checksum(), before, "wait-until-attacked scripts depend on attacker history");
    let before = sim.checksum();
    sim.unit_mut(id).unwrap().last_damage = Fx::ONE;
    assert_ne!(sim.checksum(), before, "unit scripts read the most recent damage");
    let mut restored = sim.clone();
    for _ in 0..360 {
        sim.step(&[]); restored.step(&[]);
        assert_eq!(sim.checksum(), restored.checksum());
    }
}

/// A wave of three kinds sent at a building (no field finds a way into one): each kind used to
/// start its rings again from the middle, and every unit of a kind stood on the middle once
/// the way to its target was not found. They come in apart now, the aircraft in flight.
#[test]
fn a_waves_units_come_in_apart_with_their_aircraft_flying() {
    let data = data_of(&[
        unit("TANK", "CanMove=1; MaxVelocity=1; FootprintX=2; FootprintZ=2;"),
        unit("KBOT", "CanMove=1; MaxVelocity=1; FootprintX=1; FootprintZ=1;"),
        unit("PLANE", "CanFly=1; CanMove=1; MaxVelocity=5; Acceleration=1; BrakeRate=1; TurnRate=2048; CruiseAlt=80; FootprintX=2; FootprintZ=2;"),
        unit("TOWER", "FootprintX=2; FootprintZ=2;"),
    ]);
    let mut sim = sim_of(data, 2);
    let id = |sim: &Sim, name: &str| sim.types.id(name).unwrap();
    let (tank, kbot, plane, tower) = (id(&sim, "TANK"), id(&sim, "KBOT"), id(&sim, "PLANE"), id(&sim, "TOWER"));
    let target = FxVec3::from_ints(200, 60, 200);
    sim.spawn_unit(tower, 0, target, 0, true).unwrap();
    // One of the player's own already standing where the wave comes in.
    let from = FxVec3::from_ints(700, 60, 700);
    sim.spawn_unit(kbot, 0, from, 0, true).unwrap();
    sim.mission = Some(MissionRules {
        waves: vec![Wave { tick: 1, owner: 1, units: vec![(tank, 6), (kbot, 8), (plane, 3)], from, target: Some(target), hunt: true, friendly: false }],
        ..Default::default()
    });
    sim.tick = 1;
    sim.check_mission();
    let units: Vec<&Unit> = sim.units().filter(|u| sim.unit_type(u).locomotion != Locomotion::Static).collect();
    assert_eq!(units.iter().filter(|u| u.owner == 1).count(), 17, "the whole wave came in");
    let flying = |u: &Unit| u.pos.y > Fx::from_int(60 + 40);
    for (i, a) in units.iter().enumerate() {
        for b in &units[i + 1..] {
            let reach = (sim.unit_type(a).footprint.0 + sim.unit_type(b).footprint.0) as i64 * 8;
            assert!(flying(a) != flying(b) || !a.pos.within_xz(b.pos, Fx::from_int(reach - 1)), "{} at {:?} inside {} at {:?}", sim.unit_type(a).name, a.pos, sim.unit_type(b).name, b.pos);
        }
        if a.type_id == plane {
            assert!(flying(a) && !a.landed, "an aircraft arrives in flight, not parked at the map's edge");
        }
        assert!(a.owner == 0 || a.pos.within_xz(from, Fx::from_int(400)), "a wave's unit came in far from its place");
    }
}

/// A wave's bombers bomb. TA's bombs fire on order only (`commandfire`), and an attack-move flew
/// them over the player's base without a bomb dropped. A wave's unit whose every weapon fires on
/// order goes for the nearest of the player's units its own side sees and can hit (never an
/// aircraft in flight with bombs), keeps on it while it stands and is seen, then takes the next;
/// seeing none, it attack-moves on as the others do.
#[test]
fn a_waves_bombers_are_sent_at_what_their_side_sees() {
    let data = data_of(&[
        unit("BOMBER", "CanFly=1; CanMove=1; MaxVelocity=5; Acceleration=1; BrakeRate=1; TurnRate=2048; CruiseAlt=80; Weapon1=BOMB;"),
        unit("TOWER", "FootprintX=2; FootprintZ=2;"),
        unit("PLANE", "CanFly=1; CanMove=1; MaxVelocity=5; Acceleration=1; BrakeRate=1; TurnRate=2048; CruiseAlt=80;"),
        unit("TANK", "CanMove=1; MaxVelocity=1; Weapon1=GUN;"),
        weapon_file("BOMB", "dropped=1; commandfire=1; range=200; areaofeffect=48;"),
        weapon_file("GUN", "range=200;"),
    ]);
    let mut sim = sim_of(data, 2);
    sim.los_enabled = true;
    let id = |sim: &Sim, name: &str| sim.types.id(name).unwrap();
    let (bomber, tower, plane, tank) = (id(&sim, "BOMBER"), id(&sim, "TOWER"), id(&sim, "PLANE"), id(&sim, "TANK"));
    let near = sim.spawn_unit(tower, 0, FxVec3::from_ints(600, 60, 300), 0, true).unwrap();
    sim.spawn_unit(tower, 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
    // The player's aircraft in flight, nearer still: bombs never hit one.
    let flying = sim.spawn_unit(plane, 0, FxVec3::from_ints(850, 60, 850), 0, true).unwrap();
    sim.put_in_flight(flying, None);
    let hunter = sim.spawn_unit(bomber, 1, FxVec3::from_ints(900, 60, 900), 0, true).unwrap();
    sim.put_in_flight(hunter, None);
    let gunner = sim.spawn_unit(tank, 1, FxVec3::from_ints(950, 60, 950), 0, true).unwrap();
    let hunters = [hunter, gunner].map(|id| Hunter { id, mark: FxVec3::ZERO, still: 0 }).to_vec();
    sim.mission = Some(MissionRules { hunters, ..Default::default() });
    let front = |sim: &Sim, id: UnitId| sim.unit(id).unwrap().orders.front().cloned();
    let look = |sim: &mut Sim, tick: u64| {
        sim.tick = tick;
        sim.check_mission();
    };
    // Nothing of the player's seen: both attack-move to its nearest forces.
    sim.vision.rebuild(&[]);
    look(&mut sim, 60);
    assert!(matches!(front(&sim, hunter), Some(Order::Fight(_))), "seeing nothing, the bomber attack-moves on: {:?}", front(&sim, hunter));
    assert!(matches!(front(&sim, gunner), Some(Order::Fight(_))));
    // Everything in its side's sight: the bomber goes for the nearest it can hit (not the
    // aircraft); the tank, whose gun fires of its own accord, attack-moves on.
    sim.vision.set_all(1, vision::LOS);
    look(&mut sim, 120);
    assert_eq!(front(&sim, hunter), Some(Order::Attack(Target::Unit(near))), "the nearest tower it can hit");
    assert!(matches!(front(&sim, gunner), Some(Order::Fight(_))), "a unit with a gun of its own keeps attack-moving");
    // On it while it stands and is seen, though another comes nearer.
    let closer = sim.spawn_unit(tower, 0, FxVec3::from_ints(880, 60, 700), 0, true).unwrap();
    look(&mut sim, 180);
    assert_eq!(front(&sim, hunter), Some(Order::Attack(Target::Unit(near))), "kept on its target");
    // Its target falls: the next, the nearest now.
    sim.unit_mut(near).unwrap().dead = true;
    look(&mut sim, 240);
    assert_eq!(front(&sim, hunter), Some(Order::Attack(Target::Unit(closer))), "the next once its target falls");
    // Lost from sight, and nothing else seen: it attack-moves on.
    sim.vision.rebuild(&[]);
    look(&mut sim, 300);
    assert!(matches!(front(&sim, hunter), Some(Order::Fight(_))), "nothing seen: on as the others go, {:?}", front(&sim, hunter));
}

#[test]
fn destroying_commanders_waits_for_commanders_in_future_enemy_waves() {
    let data = data_of(&[unit("COMMANDER", "Commander=1; CanMove=1; MaxVelocity=1;")]);
    let mut sim = sim_of(data, 2);
    let commander = sim.types.id("COMMANDER").unwrap();
    let pos = FxVec3::from_ints(800, 60, 800);
    let id = sim.spawn_unit(commander, 1, pos, 0, true).unwrap();
    sim.unit_mut(id).unwrap().dead = true;
    sim.mission = Some(MissionRules {
        victory: vec![Objective::KillEnemyCommander],
        waves: vec![Wave { tick: 300, owner: 1, units: vec![(commander, 1)], from: pos, target: None, hunt: true, friendly: false }],
        ..Default::default()
    });
    sim.tick = 15; sim.check_mission();
    assert_eq!(sim.mission.as_ref().unwrap().outcome, None, "a scheduled enemy commander is still to come");
    sim.mission.as_mut().unwrap().waves.clear();
    sim.tick = 30; sim.check_mission();
    assert_eq!(sim.mission.as_ref().unwrap().outcome, Some(true));
}
