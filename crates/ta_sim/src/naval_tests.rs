//! Naval sensors, underwater geometry, weapons and transport lifecycle checks.

use super::*;

#[test]
fn possession_turns_a_stationary_ship_on_the_spot() {
    let mut sim = sea();
    let ship = put(&mut sim, "SHIP", 0, 300, 300);
    let position = sim.unit(ship).unwrap().pos;
    let input = crate::DirectInput { throttle: 0, turn: -1, heading: 0, pitch: 0, fire: false };
    sim.step(&[Command::DirectControl { player: 0, unit: ship, input: Some(input) }]);
    let u = sim.unit(ship).unwrap();
    assert_eq!(u.heading, sim.unit_type(u).turn_rate);
    assert_eq!(u.pos.x, position.x);
    assert_eq!(u.pos.z, position.z);
    assert!(u.speed.is_zero());
}

#[test]
fn possession_steers_ships_without_driving_them_onto_land() {
    let mut sim = sea();
    let ship = put(&mut sim, "SHIP", 0, 650, 300);
    sim.unit_mut(ship).unwrap().heading = 16384;
    assert_eq!(sim.unit_type(sim.unit(ship).unwrap()).locomotion, Locomotion::Naval);
    let input = crate::DirectInput { throttle: 1, turn: 0, heading: 16384, pitch: 0, fire: false };
    for _ in 0..180 { sim.step(&[Command::DirectControl { player: 0, unit: ship, input: Some(input) }]); }
    let edge = sim.unit(ship).unwrap().pos.x;
    assert!(edge > Fx::from_int(650) && edge < Fx::from_int(768));
    for _ in 0..40 { sim.step(&[Command::DirectControl { player: 0, unit: ship, input: Some(crate::DirectInput { throttle: -1, ..input }) }]); }
    assert!(sim.unit(ship).unwrap().pos.x < edge);
}

fn sea() -> Sim {
    let data = data_of(&[
        unit("SHIP", "CanMove=1; Floater=1; MinWaterDepth=5; MaxVelocity=3; Acceleration=1; BrakeRate=1; TurnRate=4096; FootprintX=2; FootprintZ=2; Weapon1=GUN;"),
        unit("SUB", "CanMove=1; Floater=1; MinWaterDepth=20; WaterLine=30; MaxVelocity=3; Acceleration=1; BrakeRate=1; TurnRate=4096; FootprintX=2; FootprintZ=2; Weapon1=TORPEDO;"),
        unit("TORPEDOBOAT", "CanMove=1; Floater=1; MinWaterDepth=5; MaxVelocity=3; FootprintX=2; FootprintZ=2; Weapon1=TORPEDO;"),
        unit("TANK", "CanMove=1; MaxVelocity=2; MaxWaterDepth=0; FootprintX=1; FootprintZ=1;"),
        unit("PLANE", "CanMove=1; CanFly=1; MaxVelocity=3; FootprintX=1; FootprintZ=1;"),
        unit("SUBMISSILE", "CanMove=1; Floater=1; MinWaterDepth=20; WaterLine=30; MaxVelocity=2; FootprintX=2; FootprintZ=2; Weapon1=SUBROCKET;"),
        unit("NOFIRE", "CanMove=1; Floater=1; MinWaterDepth=20; WaterLine=30; MaxVelocity=2; FootprintX=2; FootprintZ=2; Weapon1=NOFIRETORP;"),
        unit("WATERLASER", "CanMove=1; Floater=1; MinWaterDepth=20; WaterLine=30; MaxVelocity=2; FootprintX=2; FootprintZ=2; Weapon1=WATERBEAM;"),
        unit("TRANSPORT", "CanMove=1; Floater=1; MinWaterDepth=10; MaxVelocity=3; Acceleration=1; BrakeRate=1; TurnRate=4096; FootprintX=2; FootprintZ=2; TransportCapacity=3; TransportSize=3;"),
        unit("LARGESHIP", "CanMove=1; Floater=1; MinWaterDepth=10; MaxVelocity=1; FootprintX=16; FootprintZ=16;"),
        weapon_file("GUN", "turret=1; range=400; weaponvelocity=300;"),
        weapon_file("TORPEDO", "waterweapon=1; selfprop=1; guidance=1; tracks=1; turnrate=30000; turret=1; range=400; weaponvelocity=120;"),
        weapon_file("SUBROCKET", "firesubmersed=1; vlaunch=1; selfprop=1; guidance=1; turnrate=30000; weaponvelocity=300; startvelocity=60; weaponacceleration=300; weapontimer=2; turret=1;"),
        weapon_file("NOFIRETORP", "waterweapon=1; firesubmersed=0; selfprop=1; guidance=1; tracks=1; turret=1; weaponvelocity=120;"),
        weapon_file("WATERBEAM", "waterweapon=1; rendertype=0; beamweapon=1; turret=1; weaponvelocity=300; range=400;"),
        weapon_file("SLOWTORPEDO", "waterweapon=1; selfprop=1; guidance=1; tracks=1; turnrate=90; turret=1; weaponvelocity=120; range=400;"),
    ]);
    let mut map = flat_map();
    map.sea_level = 120;
    map.heights.fill(20);
    // A dry island in the east.
    for z in 0..64 { for x in 48..64 { map.heights[z * 64 + x] = 150; } }
    let mut sim = sim_on_map(data, 2, &map);
    sim.los_enabled = false;
    sim
}

fn put(sim: &mut Sim, name: &str, owner: u8, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), owner, FxVec3::from_ints(x, 20, z), 0, true).unwrap()
}

/// A sea 100 px deep, with a shelf 20 px deep from x = 640 on; a boat armed as TA's Skeeter (a
/// laser, anti-air missiles), a torpedo boat, an underwater building (as TA's: water under it,
/// no `WaterLine`, no `Floater`) and an amphibious crawler, the last two 42 px tall (the Arm's
/// underwater metal extractor's height: out of the shelf's 20 px of water).
fn shallows() -> Sim {
    let data = data_of(&[
        unit("SKEETER", "CanMove=1; Floater=1; MinWaterDepth=6; MaxVelocity=3; Acceleration=1; BrakeRate=1; TurnRate=4096; FootprintX=2; FootprintZ=2; Weapon1=LASER; Weapon2=MISSILE; wsec_badTargetCategory=NOTAIR; Category=SHIP NOTAIR;"),
        unit("TORPEDOBOAT", "CanMove=1; Floater=1; MinWaterDepth=5; MaxVelocity=3; FootprintX=2; FootprintZ=2; Weapon1=TORPEDO;"),
        unit("UWMEX", "MinWaterDepth=19; FootprintX=3; FootprintZ=3; ExtractsMetal=0.001; MaxDamage=100000; Category=METAL NOTAIR NOTSUB;"),
        unit("CRAWLER", "CanMove=1; MaxVelocity=1; MaxWaterDepth=255; FootprintX=3; FootprintZ=3; MaxDamage=100000; Category=KBOT NOTAIR;"),
        weapon_file("LASER", "rendertype=0; beamweapon=1; turret=1;"),
        weapon_file("MISSILE", "rendertype=1; guidance=1; tracks=1; selfprop=1; turnrate=30000; turret=1;"),
        weapon_file("TORPEDO", "waterweapon=1; selfprop=1; guidance=1; tracks=1; turnrate=30000; turret=1; weaponvelocity=120;"),
    ]);
    let mut map = flat_map();
    map.sea_level = 120;
    map.heights.fill(20);
    for z in 0..64 { for x in 40..64 { map.heights[z * 64 + x] = 100; } }
    let mut sim = sim_on_map(data, 2, &map);
    sim.los_enabled = false;
    let types = Arc::get_mut(&mut sim.types).unwrap();
    for name in ["UWMEX", "CRAWLER"] {
        let kind = types.id(name).unwrap();
        types.types[kind].model = Arc::new(crate::defs::ModelDef { height: Fx::from_int(42), ..Default::default() });
    }
    sim
}

/// Whether `shooter` fires within `ticks`, and whether `victim` loses health meanwhile.
fn fires_and_hurts(sim: &mut Sim, shooter: UnitId, victim: UnitId, ticks: u32) -> (bool, bool) {
    let start = sim.unit(victim).unwrap().health;
    let mut fired = false;
    for _ in 0..ticks {
        sim.step(&[]);
        fired |= sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == shooter));
    }
    (fired, sim.unit(victim).is_none_or(|u| u.health < start))
}

/// What is wholly under the water only weapons that go under it (`waterweapon`) reach, of their
/// own accord or by an order (TA's Skeeter never fires on an underwater metal extractor); what
/// reaches out of it surface weapons and torpedoes alike. TA's underwater buildings (water under
/// them, no `WaterLine`, no `Floater`) are wholly under it however tall their models: the Arm's
/// extractor, 42 px tall, stands in 19 px of water and up.
#[test]
fn surface_weapons_never_take_on_what_is_wholly_under_the_water() {
    let calm = |sim: &mut Sim, id: UnitId| {
        let u = sim.unit_mut(id).unwrap();
        u.fire_state = 0;
        u.move_state = 0;
    };
    // An underwater building on the shelf, its model 22 px out of the water.
    let mut sim = shallows();
    let mex = put(&mut sim, "UWMEX", 1, 800, 300);
    let u = sim.unit(mex).unwrap();
    assert!(u.pos.y + sim.unit_type(u).model.height > sim.world.sea_level, "its model reaches out of the water");
    assert!(sim.submerged(u) && sim.in_water(u), "an underwater building is wholly under the water");
    let skeeter = put(&mut sim, "SKEETER", 0, 600, 300);
    assert_eq!(fires_and_hurts(&mut sim, skeeter, mex, 90), (false, false), "the Skeeter never fires on it at will");
    sim.step(&[Command::Order { player: 0, units: vec![skeeter], order: Order::Attack(Target::Unit(mex)), queue: false }]);
    assert_eq!(fires_and_hurts(&mut sim, skeeter, mex, 60), (false, false), "nor by an order");
    assert!(sim.unit(skeeter).unwrap().orders.is_empty() && sim.unit(skeeter).unwrap().weapons.iter().all(|w| w.target.is_none()), "the order ends: nothing it carries reaches the target");
    let torpedoes = put(&mut sim, "TORPEDOBOAT", 0, 600, 420);
    assert_eq!(fires_and_hurts(&mut sim, torpedoes, mex, 150), (true, true), "a torpedo boat sinks it");

    // An amphibious crawler on the shelf, its model 22 px out of the water: partly under it.
    let mut sim = shallows();
    let crawler = put(&mut sim, "CRAWLER", 1, 800, 300);
    calm(&mut sim, crawler);
    assert!(!sim.submerged(sim.unit(crawler).unwrap()) && sim.in_water(sim.unit(crawler).unwrap()));
    let skeeter = put(&mut sim, "SKEETER", 0, 600, 300);
    assert_eq!(fires_and_hurts(&mut sim, skeeter, crawler, 90), (true, true), "partly out of the water: the Skeeter hits it");
    let mut sim = shallows();
    let crawler = put(&mut sim, "CRAWLER", 1, 800, 300);
    calm(&mut sim, crawler);
    let torpedoes = put(&mut sim, "TORPEDOBOAT", 0, 600, 300);
    assert_eq!(fires_and_hurts(&mut sim, torpedoes, crawler, 150), (true, true), "and so does a torpedo boat");

    // The crawler in the deep, 100 px of water over its 42: wholly under it.
    let mut sim = shallows();
    let crawler = put(&mut sim, "CRAWLER", 1, 400, 300);
    calm(&mut sim, crawler);
    assert!(sim.submerged(sim.unit(crawler).unwrap()));
    let skeeter = put(&mut sim, "SKEETER", 0, 200, 300);
    assert_eq!(fires_and_hurts(&mut sim, skeeter, crawler, 90), (false, false), "wholly under the water: the Skeeter never fires on it");
    let torpedoes = put(&mut sim, "TORPEDOBOAT", 0, 200, 420);
    assert_eq!(fires_and_hurts(&mut sim, torpedoes, crawler, 150), (true, true), "a torpedo boat does, and hits");
}

#[test]
fn submerged_contacts_need_sonar_even_inside_optical_sight() {
    let mut sim = sea();
    sim.los_enabled = true;
    let sub = put(&mut sim, "SUB", 1, 350, 300);
    let ship = put(&mut sim, "SHIP", 1, 450, 300);
    sim.vision.set_all(0, vision::LOS | vision::RADAR);
    assert_eq!(sim.seen_by(sim.unit(ship).unwrap(), 0), Seen::Visible);
    assert_eq!(sim.seen_by(sim.unit(sub).unwrap(), 0), Seen::Hidden, "surface sight/radar cannot reveal a submerged contact");
    sim.vision.set_all(0, vision::SONAR);
    assert_eq!(sim.seen_by(sim.unit(sub).unwrap(), 0), Seen::Visible, "sonar plus sight identifies the contact");
    sim.vision.set_all(1, vision::SONAR_JAM);
    assert_eq!(sim.seen_by(sim.unit(sub).unwrap(), 0), Seen::Hidden, "sonar jamming still protects a submerged contact inside surface LOS");
    assert_eq!(sim.seen_by(sim.unit(sub).unwrap(), 1), Seen::Visible);
}

#[test]
fn radar_stealth_does_not_hide_a_submarine_from_sonar() {
    let mut sim = sea();
    sim.los_enabled = true;
    let sub = put(&mut sim, "SUB", 1, 350, 300);
    let kind = sim.unit(sub).unwrap().type_id;
    Arc::get_mut(&mut sim.types).unwrap().types[kind].def.stealth = true;
    sim.vision.set_all(0, vision::SONAR);
    assert_eq!(sim.seen_by(sim.unit(sub).unwrap(), 0), Seen::Radar, "radar stealth does not replace sonar jamming");
    sim.vision.set_all(1, vision::SONAR_JAM);
    assert_eq!(sim.seen_by(sim.unit(sub).unwrap(), 0), Seen::Hidden);
}

#[test]
fn submarine_target_and_piece_geometry_use_the_drawn_waterline() {
    let mut sim = sea();
    let sub = put(&mut sim, "SUB", 0, 350, 300);
    let u = sim.unit(sub).unwrap();
    let t = sim.unit_type(u);
    let point = sim.target_point(Target::Unit(sub)).unwrap();
    assert_eq!(point.y, u.pos.y - Fx::from_int(30) + t.model.height.max(Fx::from_int(8)) / 2, "collision/aim geometry must match the rendered submarine");
    // A root without geometry isolates its world origin from authored offsets.
    let mut model = crate::defs::ModelDef::default();
    model.pieces.push(crate::defs::PieceDef { name: "root".into(), parent: None, offset: FxVec3::ZERO, top: Fx::ZERO });
    let mut ty = t.clone();
    ty.model = Arc::new(model);
    let root = piece_world_position(u, &ty, None, 0);
    assert_eq!(root.y, sim.world.sea_level - Fx::from_int(30));
}

#[test]
fn torpedo_launchers_do_not_acquire_aircraft_or_dry_land_targets() {
    for name in ["PLANE", "TANK"] {
        let mut sim = sea();
        let shooter = put(&mut sim, "TORPEDOBOAT", 0, 700, 300);
        let victim = put(&mut sim, name, 1, 820, 300);
        if name == "PLANE" { sim.unit_mut(victim).unwrap().pos.y = Fx::from_int(220); }
        for _ in 0..20 { sim.step(&[]); }
        assert!(sim.unit(shooter).unwrap().weapons.iter().all(|w| w.target.is_none()), "a torpedo cannot acquire {name}");
        assert!(!sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == shooter)));
    }
}

#[test]
fn torpedoes_stay_in_the_water_when_chasing_surface_ships() {
    let mut sim = sea();
    let shooter = put(&mut sim, "SUB", 0, 300, 300);
    let victim = put(&mut sim, "SHIP", 1, 550, 300);
    sim.unit_mut(victim).unwrap().fire_state = 0;
    sim.unit_mut(victim).unwrap().move_state = 0;
    sim.give_orders(shooter, [Order::Attack(Target::Unit(victim))]);
    let mut observed = false;
    for _ in 0..150 {
        sim.step(&[]);
        for p in sim.projectiles.iter().filter(|p| p.owner == shooter && p.age > 3) {
            observed = true;
            assert!(p.pos.y < sim.world.sea_level, "a torpedo must not climb into the air to reach a surface ship: {:?}", p.pos);
        }
    }
    assert!(observed, "the submarine must fire");
    assert!(sim.unit(victim).unwrap().health < sim.unit_type(sim.unit(victim).unwrap()).max_health, "the torpedoes must actually hit");
}

#[test]
fn fire_submersed_permissions_are_separate_from_water_travel() {
    for (name, allowed) in [("SUBMISSILE", true), ("NOFIRE", false)] {
        let mut sim = sea();
        let shooter = put(&mut sim, name, 0, 300, 300);
        let victim = put(&mut sim, "SHIP", 1, 550, 300);
        sim.unit_mut(victim).unwrap().fire_state = 0;
        sim.unit_mut(victim).unwrap().move_state = 0;
        sim.give_orders(shooter, [Order::Attack(Target::Unit(victim))]);
        for _ in 0..30 { sim.step(&[]); }
        let fired = sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == shooter));
        assert_eq!(fired, allowed, "{name}: honor the authored firesubmersed permission");
    }
}

#[test]
fn submerged_launch_permission_keeps_a_rising_missile_alive_until_it_surfaces() {
    let mut sim = sea();
    let shooter = put(&mut sim, "SUBMISSILE", 0, 300, 300);
    let victim = put(&mut sim, "SHIP", 1, 550, 300);
    sim.unit_mut(victim).unwrap().fire_state = 0;
    sim.unit_mut(victim).unwrap().move_state = 0;
    sim.give_orders(shooter, [Order::Attack(Target::Unit(victim))]);
    for _ in 0..20 { sim.step(&[]); }
    assert!(sim.projectiles.iter().any(|p| p.owner == shooter && p.age >= 10 && p.pos.y > sim.world.sea_level), "the underwater launch must emerge before normal surface collision applies");
}

#[test]
fn submarine_draft_cannot_be_relaxed_by_a_shallower_movement_class() {
    let sim = sea();
    let mut ty = sim.types.by_name("SUB").unwrap().clone();
    ty.move_class = Some(0);
    let classes = [ta_data::MoveClass { name: "BOAT".into(), min_water_depth: 5.0, max_water_depth: 255.0, max_slope: 255.0, ..Default::default() }];
    let spec = MoveSpec::for_type(&ty, &classes);
    let mut map = flat_map();
    map.sea_level = 120;
    map.heights.fill(100);
    let mut world = World::new(&map);
    world.prepare(&spec);
    assert!(!world.terrain_passable(&spec, (20, 20)), "a submarine whose model sits 30 px below the surface cannot sail in 20 px of water");
}

#[test]
fn an_above_water_submarine_frame_is_not_protected_as_submerged() {
    let mut sim = sea();
    let sub = put(&mut sim, "SUB", 0, 300, 300);
    sim.unit_mut(sub).unwrap().pos.y = sim.world.sea_level + Fx::from_int(60);
    assert!(!sim.submerged(sim.unit(sub).unwrap()), "a submarine above the water on a construction pad is exposed");
}

#[test]
fn a_water_capable_beam_can_still_hit_aircraft() {
    let mut sim = sea();
    put(&mut sim, "WATERLASER", 0, 300, 300);
    let victim = put(&mut sim, "PLANE", 1, 500, 300);
    sim.unit_mut(victim).unwrap().pos.y = Fx::from_int(220);
    let start = sim.unit(victim).unwrap().health;
    for _ in 0..60 { sim.step(&[]); }
    assert!(sim.unit(victim).unwrap().health < start, "a water-capable beam is not a torpedo and can still hit aircraft");
}

#[test]
fn invalid_torpedo_attack_orders_finish_without_leaving_a_pursuit() {
    for ground in [false, true] {
        let mut sim = sea();
        let shooter = put(&mut sim, "TORPEDOBOAT", 0, 700, 300);
        let victim = put(&mut sim, "TANK", 1, 820, 300);
        let at = sim.unit(victim).unwrap().pos;
        let target = if ground { Target::Ground(at) } else { Target::Unit(victim) };
        sim.give_orders(shooter, [Order::Attack(target), Order::Wait(777)]);
        for _ in 0..10 { sim.step(&[]); }
        let u = sim.unit(shooter).unwrap();
        assert!(matches!(u.orders.front(), Some(Order::Wait(_))) && u.path_goal.is_none() && u.move_request.is_none());
        assert!(!sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == shooter)));
    }
}

#[test]
fn a_ship_unloads_every_passenger_at_a_reachable_shore() {
    let mut sim = sea();
    let ship = put(&mut sim, "TRANSPORT", 0, 700, 300);
    let passengers: Vec<_> = (0..3).map(|_| put(&mut sim, "TANK", 0, 820, 300)).collect();
    for &p in &passengers { sim.attach(ship, p, None); }
    sim.give_orders(ship, [Order::Unload(FxVec3::from_ints(820, 150, 300))]);
    for _ in 0..200 { sim.step(&[]); }
    assert!(sim.unit(ship).unwrap().cargo.is_empty() && sim.unit(ship).unwrap().orders.is_empty());
    for id in passengers {
        let u = sim.unit(id).unwrap();
        assert!(u.carried_by.is_none() && sim.world.depth_at(u.pos.x, u.pos.z) <= Fx::ZERO);
    }
}

#[test]
fn a_failed_ship_unload_keeps_its_cargo_aboard() {
    let mut sim = sea();
    let ship = put(&mut sim, "TRANSPORT", 0, 400, 300);
    let passenger = put(&mut sim, "TANK", 0, 820, 300);
    sim.attach(ship, passenger, None);
    sim.give_orders(ship, [Order::Unload(FxVec3::from_ints(400, 120, 300))]);
    for _ in 0..10 { sim.step(&[]); }
    assert_eq!(sim.unit(passenger).unwrap().carried_by, Some(ship));
    assert_eq!(sim.unit(ship).unwrap().cargo, vec![passenger]);
    assert!(sim.take_events().iter().any(|e| matches!(e, SimEvent::OrderFailed { unit, .. } if *unit == ship)));
}

#[test]
fn a_torpedo_contact_at_the_edge_of_a_large_hull_is_found_and_deals_damage() {
    let mut sim = sea();
    let source = put(&mut sim, "TORPEDOBOAT", 0, 100, 300);
    let victim = put(&mut sim, "LARGESHIP", 1, 384, 300);
    sim.grid.rebuild(sim.units.iter().flatten());
    let weapon = sim.weapon_types.by_name("TORPEDO").unwrap().id;
    let pos = FxVec3::from_ints(287, 118, 300);
    sim.projectiles.push(crate::Projectile { id: 500, weapon, owner: source, owner_player: 0, pos, prev_pos: pos, vel: FxVec3::from_ints(4, 0, 0), target: Some(Target::Unit(victim)), target_pos: FxVec3::from_ints(384, 118, 300), age: 5, ttl: 100, hits: vec![], dead: false });
    let before = sim.unit(victim).unwrap().health;
    sim.update_projectiles();
    assert!(sim.projectiles.is_empty(), "the segment enters the hull even though its centre lies beyond the old 48 px query padding");
    assert!(sim.unit(victim).unwrap().health < before, "direct impact damage must also find the large hull at its leading edge");
}

#[test]
fn naval_weapons_acquire_large_hulls_whose_edges_are_within_range() {
    let mut sim = sea();
    let source = put(&mut sim, "TORPEDOBOAT", 0, 100, 300);
    put(&mut sim, "LARGESHIP", 1, 560, 300);
    for _ in 0..20 { sim.step(&[]); }
    assert!(sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == source)), "range is measured to the hull's edge, not its centre");
}

#[test]
fn slow_turning_torpedoes_honor_the_authored_turn_rate() {
    let mut sim = sea();
    let source = put(&mut sim, "TORPEDOBOAT", 0, 100, 300);
    let victim = put(&mut sim, "SHIP", 1, 500, 300);
    sim.grid.rebuild(sim.units.iter().flatten());
    let weapon = sim.weapon_types.by_name("SLOWTORPEDO").unwrap().id;
    let turn = sim.weapon_types.get(weapon).turn_rate;
    assert_eq!(turn, 3);
    let pos = FxVec3::from_ints(300, 110, 300);
    sim.projectiles.push(crate::Projectile { id: 500, weapon, owner: source, owner_player: 0, pos, prev_pos: pos, vel: FxVec3::from_ints(0, 0, 4), target: Some(Target::Unit(victim)), target_pos: FxVec3::from_ints(500, 118, 300), age: 5, ttl: 100, hits: vec![], dead: false });
    sim.update_projectiles();
    let p = &sim.projectiles[0];
    assert!(heading_of(p.vel).abs() <= turn + 2, "the generic missile's 200-unit floor must not override a torpedo's slow turn rate: {}", heading_of(p.vel));
}
