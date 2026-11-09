//! Land movement, combat orders and deterministic state regressions.

use super::*;

fn battlefield() -> Sim {
    battlefield_on_map(&flat_map())
}

fn battlefield_on_map(map: &TntMap) -> Sim {
    let data = data_of(&[
        unit("TANK", "CanMove=1; MaxVelocity=4; Acceleration=1; BrakeRate=1; TurnRate=4096; SteeringMode=2; MaxSlope=12; MaxWaterDepth=0; FootprintX=1; FootprintZ=1; Weapon1=GUN;"),
        unit("FAST", "CanMove=1; MaxVelocity=48; Acceleration=48; BrakeRate=48; TurnRate=4096; SteeringMode=2; MaxSlope=12; MaxWaterDepth=0; FootprintX=1; FootprintZ=1;"),
        unit("GIANT", "CanMove=1; MaxVelocity=1; FootprintX=16; FootprintZ=16;"),
        unit("AMPHIB", "CanMove=1; MaxVelocity=4; Acceleration=1; BrakeRate=1; TurnRate=4096; SteeringMode=2; MaxSlope=12; MaxWaterDepth=255; FootprintX=1; FootprintZ=1;"),
        unit("HOVER", "CanMove=1; CanHover=1; MaxVelocity=4; Acceleration=1; BrakeRate=1; TurnRate=4096; SteeringMode=2; MaxSlope=12; FootprintX=1; FootprintZ=1;"),
        unit("BOAT", "CanMove=1; Floater=1; MinWaterDepth=10; MaxVelocity=4; Acceleration=1; BrakeRate=1; TurnRate=4096; SteeringMode=2; FootprintX=1; FootprintZ=1;"),
        unit("MANUAL", "CanMove=1; MaxVelocity=4; Acceleration=1; BrakeRate=1; TurnRate=4096; SteeringMode=2; FootprintX=1; FootprintZ=1; Weapon1=MANUALGUN;"),
        weapon_file("GUN", "range=160; reloadtime=0.2; weaponvelocity=300; turret=1;"),
        weapon_file("MANUALGUN", "range=160; reloadtime=0.2; weaponvelocity=300; turret=1; commandfire=1;"),
    ]);
    let mut sim = sim_on_map(data, 2, map);
    sim.los_enabled = false;
    sim
}

fn put(sim: &mut Sim, name: &str, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), 0, FxVec3::from_ints(x, 60, z), 16384, true).unwrap()
}

#[test]
fn unreachable_move_finishes_at_the_reachable_side_and_advances_the_queue() {
    let mut sim = battlefield();
    for z in 0..64 { sim.world.occupy(Occupant::Feature(900), (32, z), (1, 1), &[]); }
    let tank = put(&mut sim, "TANK", 300, 300);
    sim.give_orders(tank, [Order::Move(FxVec3::from_ints(800, 60, 300)), Order::Wait(777)]);
    for _ in 0..500 {
        sim.step(&[]);
        if matches!(sim.unit(tank).unwrap().orders.front(), Some(Order::Wait(_))) { break; }
    }
    let u = sim.unit(tank).unwrap();
    assert!(matches!(u.orders.front(), Some(Order::Wait(_))), "unreachable Move must finish at its closest reachable point: {:?}, {:?}", u.pos, u.path_goal);
    assert!(u.pos.x < Fx::from_int(512));
}

#[test]
fn cached_routes_keep_each_units_exact_destination() {
    let mut sim = battlefield();
    let a = put(&mut sim, "TANK", 160, 160);
    let b = put(&mut sim, "TANK", 170, 160);
    let goal_a = FxVec3::from_ints(801, 60, 300);
    let goal_b = FxVec3::from_ints(805, 60, 300);
    sim.move_to_point(a, goal_a);
    sim.move_to_point(b, goal_b);
    assert_eq!(sim.stats.path_searches, 1, "the second route should reuse the first search");
    assert_eq!(sim.unit(b).unwrap().path_goal, Some(goal_b), "sharing anchor cells must preserve each exact requested position");
    sim.remove_unit(a);
    sim.give_orders(b, [Order::Move(goal_b)]);
    for _ in 0..300 { sim.step(&[]); }
    let u = sim.unit(b).unwrap();
    assert!(u.orders.is_empty() && u.pos.within_xz(goal_b, Fx::from_int(4)), "the last leg must reach the requested point rather than the anchor centre: {:?}", u.pos);
}

#[test]
fn an_empty_unreachable_route_finishes_and_cannot_poison_a_neighbours_cache() {
    let mut sim = battlefield();
    let trapped = put(&mut sim, "TANK", 168, 168);
    let free = put(&mut sim, "TANK", 184, 184);
    for c in [(9, 10), (11, 10), (10, 9), (10, 11)] { sim.world.occupy(Occupant::Feature(900), c, (1, 1), &[]); }
    let goal = FxVec3::from_ints(800, 60, 800);
    sim.move_to_point(trapped, goal);
    assert!(sim.unit(trapped).unwrap().path.is_empty());
    sim.move_to_point(free, goal);
    assert!(!sim.unit(free).unwrap().path.is_empty(), "a failed empty route from a nearby blocked start is not this unit's route");
    sim.give_orders(trapped, [Order::Move(goal), Order::Wait(777)]);
    for _ in 0..40 { sim.step(&[]); }
    assert!(matches!(sim.unit(trapped).unwrap().orders.front(), Some(Order::Wait(_))), "a unit in a sealed pocket must finish its unreachable Move");
}

#[test]
fn ground_motion_cannot_cut_a_corner_blocked_on_both_sides() {
    let mut sim = battlefield();
    let tank = put(&mut sim, "TANK", 23, 23);
    sim.world.occupy(Occupant::Feature(900), (1, 0), (1, 1), &[]);
    sim.world.occupy(Occupant::Feature(901), (0, 1), (1, 1), &[]);
    let t = sim.types.by_name("TANK").unwrap().clone();
    let spec = sim.move_spec(t.id);
    let u = sim.units[tank.slot as usize].as_mut().unwrap();
    u.heading = 8192;
    u.speed = Fx::from_int(4);
    u.path_goal = Some(FxVec3::from_ints(100, 60, 100));
    Sim::ground_step(u, &t, spec, &sim.world, Default::default(), 10);
    assert_eq!(World::pos_to_anchor(u.pos, t.footprint), (0, 0), "the diagonal step crossed a closed corner: {:?}", u.pos);
}

#[test]
fn movement_checks_the_thin_cell_crossed_between_two_nearby_boundaries() {
    let mut sim = battlefield();
    let tank = put(&mut sim, "TANK", 23, 23);
    sim.world.occupy(Occupant::Feature(900), (1, 0), (1, 1), &[]);
    let t = sim.types.by_name("TANK").unwrap().clone();
    let spec = sim.move_spec(t.id);
    let u = sim.units[tank.slot as usize].as_mut().unwrap();
    u.pos.x += Fx::from_ratio(1, 4);
    u.heading = 8192;
    u.speed = Fx::from_int(2);
    u.path_goal = Some(FxVec3::from_ints(100, 60, 100));
    Sim::ground_step(u, &t, spec, &sim.world, Default::default(), 10);
    assert_eq!(World::pos_to_anchor(u.pos, t.footprint).0, 0, "the diagonal crosses the blocked x-side cell briefly before the z boundary: {:?}", u.pos);
}

#[test]
fn a_fast_unit_cannot_skip_over_a_blocking_cell() {
    let mut sim = battlefield();
    let tank = put(&mut sim, "FAST", 300, 300);
    sim.world.occupy(Occupant::Feature(900), (20, 18), (1, 1), &[]);
    let t = sim.types.by_name("FAST").unwrap().clone();
    let spec = sim.move_spec(t.id);
    let u = sim.units[tank.slot as usize].as_mut().unwrap();
    u.speed = t.max_speed;
    u.path_goal = Some(FxVec3::from_ints(800, 60, 300));
    Sim::ground_step(u, &t, spec, &sim.world, Default::default(), 10);
    assert!(u.pos.x < Fx::from_int(320), "one tick must not jump a blocking cell: {:?}", u.pos);
}

#[test]
fn escaping_an_obstacle_never_steps_outside_the_map() {
    let mut sim = battlefield();
    let tank = put(&mut sim, "TANK", 8, 300);
    sim.world.occupy(Occupant::Feature(900), (0, 18), (1, 1), &[]);
    let t = sim.types.by_name("TANK").unwrap().clone();
    let spec = sim.move_spec(t.id);
    let u = sim.units[tank.slot as usize].as_mut().unwrap();
    u.heading = -16384;
    u.speed = Fx::from_int(4);
    u.path_goal = Some(FxVec3::from_ints(-100, 60, 300));
    Sim::ground_step(u, &t, spec, &sim.world, Default::default(), 10);
    assert!(sim.world.terrain_passable(&spec, World::pos_to_anchor(u.pos, t.footprint)), "a unit trapped under an obstacle still respects terrain/map bounds: {:?}", u.pos);
}

#[test]
fn escaping_one_obstacle_does_not_enter_another() {
    let mut sim = battlefield();
    let tank = put(&mut sim, "TANK", 311, 300);
    sim.world.occupy(Occupant::Feature(900), (18, 18), (1, 1), &[]);
    sim.world.occupy(Occupant::Feature(901), (19, 18), (1, 1), &[]);
    let t = sim.types.by_name("TANK").unwrap().clone();
    let spec = sim.move_spec(t.id);
    let u = sim.units[tank.slot as usize].as_mut().unwrap();
    u.speed = Fx::from_int(4);
    u.path_goal = Some(FxVec3::from_ints(800, 60, 300));
    Sim::ground_step(u, &t, spec, &sim.world, Default::default(), 10);
    assert_eq!(World::pos_to_anchor(u.pos, t.footprint), (18, 18), "escaping the first obstacle cannot ignore the second");
}

#[test]
fn land_amphibious_hover_and_naval_units_respect_their_shoreline_constraints() {
    let mut map = flat_map();
    map.sea_level = 40;
    for z in 0..64 { for x in 0..64 { map.heights[z * 64 + x] = (20 + (x as i32 - 20).clamp(0, 10) * 4) as u8; } }
    for name in ["TANK", "AMPHIB", "HOVER", "BOAT"] {
        let mut sim = battlefield_on_map(&map);
        let (from, to) = if name == "BOAT" { (200, 800) } else { (800, 200) };
        let unit = put(&mut sim, name, from, 300);
        let kind = sim.unit(unit).unwrap().type_id;
        let spec = sim.move_spec(kind);
        assert!(sim.world.terrain_passable(&spec, World::pos_to_anchor(sim.unit(unit).unwrap().pos, (1, 1))), "{name} must have a valid initial terrain region");
        sim.give_orders(unit, [Order::Move(FxVec3::from_ints(to, 60, 300))]);
        for _ in 0..600 {
            sim.step(&[]);
            let u = sim.unit(unit).unwrap();
            assert!(sim.world.terrain_passable(&spec, World::pos_to_anchor(u.pos, (1, 1))), "{name} entered invalid terrain at {:?}", u.pos);
            let ground = sim.world.height_at(u.pos.x, u.pos.z);
            let surface = if matches!(name, "HOVER" | "BOAT") { ground.max(sim.world.sea_level) } else { ground };
            assert_eq!(u.pos.y, surface, "{name} must follow its ground/water surface");
            if u.orders.is_empty() { break; }
        }
        let u = sim.unit(unit).unwrap();
        assert!(u.orders.is_empty(), "{name} must finish at its destination or reachable shoreline");
        if matches!(name, "AMPHIB" | "HOVER") { assert!((u.pos.x - Fx::from_int(to)).abs() < Fx::from_int(32)); }
        else { assert!((u.pos.x - Fx::from_int(to)).abs() > Fx::from_int(100)); }
    }
}

#[test]
fn large_ground_units_separate_across_more_than_one_crowd_bucket() {
    let mut sim = battlefield();
    let a = put(&mut sim, "GIANT", 319, 400);
    let b = put(&mut sim, "GIANT", 449, 400);
    let before = sim.unit(a).unwrap().pos.distance_xz(sim.unit(b).unwrap().pos);
    sim.resolve_collisions();
    let after = sim.unit(a).unwrap().pos.distance_xz(sim.unit(b).unwrap().pos);
    assert!(after > before, "overlapping large footprints must collide across bucket boundaries ({before:?} -> {after:?})");
}

#[test]
fn fight_order_resumes_movement_after_its_contact_is_gone() {
    let mut sim = battlefield();
    let tank = put(&mut sim, "TANK", 200, 300);
    let target = sim.spawn_unit(sim.types.id("TANK").unwrap(), 1, FxVec3::from_ints(280, 60, 300), 0, true).unwrap();
    sim.unit_mut(target).unwrap().fire_state = 0;
    sim.give_orders(tank, [Order::Fight(FxVec3::from_ints(800, 60, 300))]);
    for _ in 0..30 { sim.step(&[]); }
    assert!(sim.unit(tank).unwrap().weapons.iter().any(|w| w.target == Some(Target::Unit(target))));
    sim.remove_unit(target);
    for _ in 0..150 { sim.step(&[]); }
    assert!(sim.unit(tank).unwrap().pos.x > Fx::from_int(500), "a paused fight route must resume when its weapon contact ends");
}

#[test]
fn completed_manual_fire_clears_its_approach_before_a_queued_wait() {
    let mut sim = battlefield();
    let tank = put(&mut sim, "MANUAL", 200, 300);
    sim.give_orders(tank, [Order::ManualFire(Target::Ground(FxVec3::from_ints(350, 60, 300))), Order::Wait(777)]);
    sim.step(&[]);
    assert!(sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == tank)), "the shot must complete the manual-fire order");
    let u = sim.unit(tank).unwrap();
    assert!(matches!(u.orders.front(), Some(Order::Wait(_))));
    assert!(u.path_goal.is_none() && u.path.is_empty() && u.move_request.is_none(), "a completed firing order cannot keep its old approach during Wait");
    let stopped = u.pos;
    for _ in 0..20 { sim.step(&[]); }
    assert_eq!(sim.unit(tank).unwrap().pos, stopped);
}

#[test]
fn a_spawned_vehicle_already_follows_the_terrain_under_its_tracks() {
    let mut map = flat_map();
    for z in 0..64 { for x in 0..64 { map.heights[z * 64 + x] = 30 + x as u8; } }
    let mut sim = battlefield_on_map(&map);
    let tank = put(&mut sim, "TANK", 300, 300);
    let u = sim.unit(tank).unwrap();
    assert!(u.pitch < 0, "the east-facing vehicle's tracks climb the eastward slope on its first frame");
    assert_eq!(u.roll, 0);
}

#[test]
fn ground_standing_orders_and_route_state_change_the_checksum() {
    let mut sim = battlefield();
    let tank = put(&mut sim, "TANK", 200, 300);
    let expected = sim.checksum();
    for field in 0..12 {
        let mut changed = sim.clone();
        let u = changed.unit_mut(tank).unwrap();
        match field {
            0 => u.fire_state = 0,
            1 => u.move_state = 0,
            2 => u.path.push((20, 20)),
            3 => u.path_partial = true,
            4 => u.stuck_ticks += 1,
            5 => u.approach_best = Fx::from_int(99),
            6 => u.approach_idle += 1,
            7 => u.head_on_idle += 1,
            8 => u.squeeze_until += 1,
            9 => u.face = Some(100),
            10 => u.idle_anchor = Some(FxVec3::from_ints(100, 60, 100)),
            11 => u.formations.push((77, FxVec3::from_ints(100, 60, 100))),
            _ => unreachable!(),
        }
        assert_ne!(changed.checksum(), expected, "ground state {field} must affect the lockstep checksum");
    }
}

#[test]
fn a_temporarily_stalled_unit_rejoins_its_formations_pace() {
    let mut sim = battlefield();
    sim.group_formations = true;
    let a = put(&mut sim, "TANK", 200, 300);
    let b = put(&mut sim, "TANK", 200, 350);
    sim.step(&[Command::Formation { player: 0, units: vec![a, b], order: Order::Move(FxVec3::from_ints(800, 60, 300)), queue: false, shape: crate::formation::Shape::Keep }]);
    sim.unit_mut(a).unwrap().paralyzed_until = sim.tick + 75;
    for _ in 0..75 { sim.step(&[]); }
    assert!(sim.formations.iter().flat_map(|f| &f.mates).find(|m| m.id == a).unwrap().out, "the stalled member must first be let go");
    for _ in 0..15 { sim.step(&[]); }
    assert!(!sim.formations.iter().flat_map(|f| &f.mates).find(|m| m.id == a).unwrap().out, "once it makes progress within the formation schedule it must rejoin the pace");
}

#[test]
fn formation_schedule_and_retained_steering_change_the_checksum() {
    let mut sim = battlefield();
    sim.group_formations = true;
    let a = put(&mut sim, "TANK", 200, 300);
    let b = put(&mut sim, "TANK", 200, 350);
    sim.step(&[Command::Formation { player: 0, units: vec![a, b], order: Order::Move(FxVec3::from_ints(800, 60, 300)), queue: false, shape: crate::formation::Shape::Keep }]);
    let expected = sim.checksum();
    assert_eq!(sim.clone().checksum(), expected);
    for field in 0..9 {
        let mut changed = sim.clone();
        match field {
            0 => changed.formations[0].phase = crate::formation::Phase::Forming { until: 999 },
            1 => changed.formations[0].facing += 1,
            2 => changed.formations[0].mates[0].out = true,
            3 => changed.local_avoidance = !changed.local_avoidance,
            4 => changed.group_formations = false,
            5 => changed.next_formation += 1,
            6 => changed.steers.push(crate::avoid::Steer { bend: 100, speed_cap: Some(Fx::ONE) }),
            7 => changed.formations[0].mates[0].form = Some(FxVec3::ZERO),
            8 => changed.formations[0].mates[0].spot.x += Fx::ONE,
            _ => unreachable!(),
        }
        assert_ne!(changed.checksum(), expected, "formation/steering field {field} must affect the lockstep checksum");
    }
}
