//! Air movement, transport, combat and state regressions without original assets.

use super::*;

fn airfield() -> Sim {
    airfield_on_map(&flat_map())
}

fn airfield_on_map(map: &TntMap) -> Sim {
    let data = data_of(&[
        unit("FIGHTER", "CanFly=1; CanMove=1; MaxVelocity=5; Acceleration=1; BrakeRate=1; TurnRate=2048; CruiseAlt=80; Weapon1=GUN; FootprintX=1; FootprintZ=1;"),
        unit("GUNSHIP", "CanFly=1; CanMove=1; HoverAttack=1; MaxVelocity=4; Acceleration=1; BrakeRate=1; TurnRate=2048; CruiseAlt=80; Weapon1=FIXED; FootprintX=1; FootprintZ=1;"),
        unit("TRANSPORT", "CanFly=1; CanMove=1; MaxVelocity=4; Acceleration=1; BrakeRate=1; TurnRate=2048; CruiseAlt=80; TransportCapacity=3; TransportSize=3; FootprintX=1; FootprintZ=1;"),
        unit("TANK", "CanMove=1; MaxVelocity=1; MaxWaterDepth=0; FootprintX=1; FootprintZ=1;"),
        unit("BUILDER", "Builder=1; WorkerTime=30; CanMove=1; MaxVelocity=1; BuildDistance=128; FootprintX=1; FootprintZ=1;"),
        unit("BUILDING", "FootprintX=6; FootprintZ=6; BuildTime=1000; BuildCostMetal=100; BuildCostEnergy=1000;"),
        unit("PAD", "IsAirBase=1; WorkerTime=300; FootprintX=4; FootprintZ=4;"),
        unit("CARRIER", "IsAirBase=1; WorkerTime=300; CanMove=1; MaxVelocity=2; Acceleration=1; BrakeRate=1; TurnRate=2048; Floater=1; MinWaterDepth=20; FootprintX=4; FootprintZ=4;"),
        // (As TA's seaplanes say it: the depth first denied, then given.)
        unit("SEAPLANE", "CanFly=1; CanMove=1; MaxVelocity=5; Acceleration=1; BrakeRate=1; TurnRate=2048; CruiseAlt=80; FootprintX=2; FootprintZ=2; MaxWaterDepth=0; maxwaterdepth=255; amphibious=1;"),
        // A plane turning slowly: at its slowest it turns round a circle 35 px across.
        unit("GLIDER", "CanFly=1; CanMove=1; MaxVelocity=5; Acceleration=1; BrakeRate=1; TurnRate=150; CruiseAlt=80; Weapon1=GUN; FootprintX=1; FootprintZ=1;"),
        // A construction aircraft turning as slowly as TA's (ARMCA's 90).
        unit("AIRCON", "CanFly=1; CanMove=1; Builder=1; WorkerTime=60; BuildDistance=40; MaxVelocity=6.9; Acceleration=0.06; BrakeRate=1.5; TurnRate=90; CruiseAlt=70; FootprintX=1; FootprintZ=1;"),
        weapon_file("GUN", "range=300; reloadtime=0.2; weaponvelocity=300; turret=1;"),
        weapon_file("FIXED", "range=300; reloadtime=0.2; weaponvelocity=300; tolerance=1000;"),
    ]);
    let mut sim = sim_on_map(data, 2, map);
    for p in &mut sim.players { p.endless = true; }
    sim.los_enabled = false;
    sim
}

fn put(sim: &mut Sim, name: &str, owner: u8, x: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), owner, FxVec3::from_ints(x, 60, 300), 0, true).unwrap()
}

#[test]
fn an_air_unload_order_delivers_every_passenger() {
    let mut sim = airfield();
    let transport = put(&mut sim, "TRANSPORT", 0, 300);
    let passengers: Vec<_> = (0..3).map(|_| put(&mut sim, "TANK", 0, 300)).collect();
    for &p in &passengers { sim.attach(transport, p, None); }
    let height = sim.types.by_name("TANK").unwrap().model.height;
    sim.unit_mut(transport).unwrap().pos.y = Fx::from_int(60 + 6) + height;
    let at = FxVec3::from_ints(300, 60, 300);
    sim.give_orders(transport, [Order::Unload(at)]);
    for _ in 0..120 { sim.step(&[]); }
    assert!(sim.unit(transport).unwrap().cargo.is_empty(), "one unload command must deliver all cargo");
    assert!(passengers.iter().all(|&p| sim.unit(p).unwrap().carried_by.is_none()));
    assert!(sim.unit(transport).unwrap().orders.is_empty());
}

#[test]
fn a_failed_air_drop_does_not_run_end_transport_with_cargo_aboard() {
    use ta_formats::cob::{CobFunction, CobScript, op};
    let mut map = flat_map();
    map.sea_level = 150;
    let mut sim = airfield_on_map(&map);
    let kind = sim.types.id("TRANSPORT").unwrap();
    Arc::get_mut(&mut sim.types).unwrap().types[kind].script = Some(Arc::new(crate::cob::CobProgram::new(CobScript {
        static_var_count: 1,
        functions: vec![CobFunction { name: "EndTransport".into(), offset: 0 }],
        code: vec![op::PUSH_STATIC, 0, op::PUSH_CONSTANT, 1, op::ADD, op::POP_STATIC, 0, op::PUSH_CONSTANT, 0, op::RETURN],
        ..Default::default()
    }, &[])));
    let transport = put(&mut sim, "TRANSPORT", 0, 300);
    let passenger = put(&mut sim, "TANK", 0, 300);
    sim.attach(transport, passenger, None);
    let height = sim.types.by_name("TANK").unwrap().model.height;
    sim.unit_mut(transport).unwrap().pos.y = Fx::from_int(156) + height;
    let at = FxVec3::from_ints(300, 150, 300);
    sim.give_orders(transport, [Order::Unload(at)]);
    let t = sim.types.by_name("TRANSPORT").unwrap().clone();
    sim.update_unload_order(transport, &t, at);
    sim.run_scripts(&sim.types.clone());
    assert_eq!(sim.unit(passenger).unwrap().carried_by, Some(transport));
    assert_eq!(sim.unit(transport).unwrap().cob.as_ref().unwrap().statics[0], 0, "a failed drop must keep the loaded script state");
}

#[test]
fn an_invalid_air_pickup_clears_its_old_approach() {
    let mut sim = airfield();
    let transport = put(&mut sim, "TRANSPORT", 0, 100);
    let passenger = put(&mut sim, "TANK", 0, 800);
    sim.step(&[Command::Order { player: 0, units: vec![transport], order: Order::Load(passenger), queue: false }]);
    assert!(sim.unit(transport).unwrap().path_goal.is_some());
    sim.remove_unit(passenger);
    sim.step(&[]);
    let t = sim.unit(transport).unwrap();
    assert!(t.orders.is_empty() && t.path_goal.is_none(), "a cancelled pickup cannot leave the aircraft chasing its old target");
}

#[test]
fn loading_a_constructor_ends_its_old_work_and_idle_return_route() {
    let mut sim = airfield();
    let transport = put(&mut sim, "TRANSPORT", 0, 300);
    let builder = put(&mut sim, "BUILDER", 0, 300);
    let frame = sim.spawn_unit(sim.types.id("BUILDING").unwrap(), 0, FxVec3::from_ints(400, 60, 300), 0, false).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Repair(frame), queue: false }]);
    sim.step(&[]);
    assert!(sim.unit(builder).unwrap().build.is_some() && sim.unit(builder).unwrap().in_build_stance);
    sim.unit_mut(builder).unwrap().idle_anchor = Some(FxVec3::from_ints(100, 60, 100));
    sim.attach(transport, builder, None);
    let u = sim.unit(builder).unwrap();
    assert!(u.build.is_none() && !u.in_build_stance && u.idle_anchor.is_none(), "boarding cancels the old work and its automatic return route");
    assert!(sim.detach(transport, builder, FxVec3::from_ints(300, 60, 300)));
    assert!(!sim.unit_flows(sim.unit(builder).unwrap()).any(), "an unloaded idle constructor must not display stale construction draw");
}

#[test]
fn idle_aircraft_do_not_land_inside_ordinary_buildings() {
    let mut sim = airfield();
    let building = put(&mut sim, "BUILDING", 0, 300);
    let fighter = put(&mut sim, "FIGHTER", 0, 300);
    sim.unit_mut(fighter).unwrap().pos.y = Fx::from_int(140);
    for _ in 0..240 { sim.step(&[]); }
    let u = sim.unit(fighter).unwrap();
    let cell = World::cell_of(u.pos);
    assert!(!u.landed || sim.world.occupant(cell.0, cell.1) != Occupant::Unit(building.slot), "landing must avoid the building's occupied footprint");
}

#[test]
fn a_landing_approach_finishes_inside_the_clear_footprint() {
    let mut sim = airfield();
    let kind = sim.types.id("FIGHTER").unwrap();
    Arc::get_mut(&mut sim.types).unwrap().types[kind].footprint = (2, 2);
    let building = put(&mut sim, "BUILDING", 0, 300);
    let origin = sim.unit(building).unwrap().occupied_origin.unwrap();
    let clear = sim.world.anchor_to_pos((origin.0 + 7, origin.1 + 3), (2, 2));
    let fighter = put(&mut sim, "FIGHTER", 0, 300);
    let u = sim.unit_mut(fighter).unwrap();
    u.pos = clear + FxVec3::from_ints(-2, 80, 0);
    u.path_goal = Some(clear);
    u.move_request = Some(clear);
    u.idle_ticks = 100;
    for _ in 0..120 { sim.step(&[]); }
    let u = sim.unit(fighter).unwrap();
    assert!(u.landed && u.pos.within_xz(clear, Fx::ONE), "parking must reach the valid footprint before the goal is cleared: {:?}", u.pos);
}

#[test]
fn repair_pads_mend_landed_gunships_and_unarmed_aircraft() {
    for name in ["GUNSHIP", "TRANSPORT"] {
        let mut sim = airfield();
        let pad = put(&mut sim, "PAD", 0, 300);
        let aircraft = put(&mut sim, name, 0, 300);
        sim.unit_mut(aircraft).unwrap().health = Fx::from_int(500);
        sim.unit_mut(aircraft).unwrap().landed = true;
        sim.unit_mut(aircraft).unwrap().air_pad = Some((pad, None));
        sim.grid.rebuild(sim.units.iter().flatten());
        let t = sim.types.by_name("PAD").unwrap().clone();
        sim.tick = 5;
        sim.update_air_base(pad, &t);
        assert!(sim.unit(aircraft).unwrap().health > Fx::from_int(500), "{name} is still an aircraft that the pad can repair");
    }
}

#[test]
fn aircraft_land_on_repair_carriers_and_stay_on_the_moving_deck() {
    let mut map = flat_map();
    map.sea_level = 150;
    let mut sim = airfield_on_map(&map);
    let carrier = put(&mut sim, "CARRIER", 0, 300);
    let fighter = put(&mut sim, "FIGHTER", 0, 300);
    sim.unit_mut(fighter).unwrap().pos.y = Fx::from_int(230);
    sim.unit_mut(fighter).unwrap().health = Fx::from_int(500);
    sim.unit_mut(fighter).unwrap().fire_state = 0;
    for _ in 0..240 { sim.step(&[]); }
    assert!(sim.unit(fighter).unwrap().landed, "a repair carrier is a landing platform over water");
    assert!(sim.unit(fighter).unwrap().health > Fx::from_int(500), "mobile airbases must provide repairs");
    sim.step(&[Command::Order { player: 0, units: vec![carrier], order: Order::Move(FxVec3::from_ints(600, 150, 300)), queue: false }]);
    for _ in 0..60 { sim.step(&[]); }
    assert!(sim.unit(fighter).unwrap().pos.within_xz(sim.unit(carrier).unwrap().pos, Fx::from_int(2)), "a parked aircraft rides the carrier deck");
    sim.transfer_unit(carrier, 1);
    sim.step(&[]);
    assert!(!sim.unit(fighter).unwrap().landed, "loss of an allied landing platform releases the aircraft");
}

#[test]
fn aircraft_finish_an_outside_map_move_at_the_reachable_edge() {
    let mut sim = airfield();
    let fighter = put(&mut sim, "FIGHTER", 0, 700);
    sim.unit_mut(fighter).unwrap().fire_state = 0;
    sim.give_orders(fighter, [Order::Move(FxVec3::from_ints(1300, 60, 300)), Order::Wait(900)]);
    for _ in 0..450 { sim.step(&[]); }
    assert!(matches!(sim.unit(fighter).unwrap().orders.front(), Some(Order::Wait(_))), "an unreachable point beyond the edge cannot stall later orders");
}

#[test]
fn idle_fighters_engage_a_detected_threat_under_maneuver_orders() {
    let mut sim = airfield();
    let fighter = put(&mut sim, "FIGHTER", 0, 300);
    // A forward gun, with the threat off its nose and no player attack order.
    let fixed = sim.weapon_types.by_name("FIXED").unwrap().id;
    let kind = sim.types.id("FIGHTER").unwrap();
    Arc::get_mut(&mut sim.types).unwrap().types[kind].weapons[0] = Some(fixed);
    sim.unit_mut(fighter).unwrap().pos.y = Fx::from_int(140);
    sim.unit_mut(fighter).unwrap().move_state = 1;
    let enemy = put(&mut sim, "TRANSPORT", 1, 450);
    sim.unit_mut(enemy).unwrap().pos.y = Fx::from_int(140);
    sim.give_orders(enemy, [Order::Wait(1000)]);
    let mut fired = false;
    for _ in 0..180 {
        sim.step(&[]);
        fired |= sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == fighter));
    }
    assert!(fired, "fire-at-will fighters must turn into an attack run for an acquired threat");
}

#[test]
fn a_hovering_aircraft_turns_its_fixed_weapon_toward_its_target() {
    let mut sim = airfield();
    let gunship = put(&mut sim, "GUNSHIP", 0, 300);
    sim.unit_mut(gunship).unwrap().pos.y = Fx::from_int(140);
    let target = put(&mut sim, "TANK", 1, 450);
    sim.step(&[Command::Order { player: 0, units: vec![gunship], order: Order::Attack(Target::Unit(target)), queue: false }]);
    let mut fired = false;
    for _ in 0..90 {
        sim.step(&[]);
        fired |= sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, .. } if *unit == gunship));
    }
    assert!(fired, "a gunship already in range must turn rather than wait forever with its gun pointing away");
}

#[test]
fn hovering_aircraft_do_not_turn_toward_hidden_live_positions() {
    let mut sim = airfield();
    let gunship = put(&mut sim, "GUNSHIP", 0, 300);
    let enemy = put(&mut sim, "TANK", 1, 450);
    sim.unit_mut(gunship).unwrap().pos.y = Fx::from_int(140);
    sim.unit_mut(gunship).unwrap().weapons[0].target = Some(Target::Unit(enemy));
    sim.los_enabled = true;
    sim.vision.rebuild(&[]);
    let t = sim.types.by_name("GUNSHIP").unwrap().clone();
    sim.update_air(gunship, &t);
    assert_eq!(sim.unit(gunship).unwrap().heading, 0, "a stale weapon target cannot reveal a hidden unit's new position");
}

#[test]
fn losing_a_reserved_carrier_cancels_the_deck_approach() {
    let mut map = flat_map();
    map.sea_level = 150;
    let mut sim = airfield_on_map(&map);
    let carrier = put(&mut sim, "CARRIER", 0, 300);
    let fighter = put(&mut sim, "FIGHTER", 0, 440);
    let goal = sim.unit(carrier).unwrap().pos;
    let u = sim.unit_mut(fighter).unwrap();
    u.pos.y = Fx::from_int(230);
    u.air_pad = Some((carrier, None));
    u.path_goal = Some(goal);
    sim.transfer_unit(carrier, 1);
    sim.step(&[]);
    let u = sim.unit(fighter).unwrap();
    assert!(u.air_pad.is_none() && u.path_goal.is_none() && !u.landed, "a lost platform must release its old approach as well as its reservation");
}

#[test]
fn newly_loaded_cargo_is_not_damaged_through_a_stale_combat_grid() {
    let mut sim = airfield();
    let transport = put(&mut sim, "TRANSPORT", 0, 300);
    let passenger = put(&mut sim, "TANK", 0, 300);
    let attacker = put(&mut sim, "TANK", 1, 100);
    sim.grid.rebuild(sim.units.iter().flatten());
    sim.attach(transport, passenger, None);
    let pos = sim.unit(passenger).unwrap().pos;
    let weapon = sim.weapon_types.by_name("GUN").unwrap().clone();
    let health = sim.unit(passenger).unwrap().health;
    sim.explode(&weapon, pos, attacker, Some(passenger), false, false, None);
    assert_eq!(sim.unit(passenger).unwrap().health, health, "cargo attached after the grid snapshot is still inaccessible");
    sim.unit_mut(transport).unwrap().pos.y += Fx::from_int(100);
    let from = pos + FxVec3::from_ints(-32, 4, 0);
    sim.projectiles.push(crate::Projectile { id: 99, weapon: weapon.id, owner: attacker, owner_player: 1, pos: from, prev_pos: from, vel: FxVec3::from_ints(64, 0, 0), target: Some(Target::Unit(passenger)), target_pos: pos, age: 0, ttl: 100, hits: Vec::new(), dead: false });
    sim.update_projectiles();
    assert_eq!(sim.unit(passenger).unwrap().health, health, "direct projectiles must also ignore newly loaded cargo");
}

#[test]
fn air_checksums_include_flight_phase_and_transport_state() {
    let mut sim = airfield();
    let aircraft = put(&mut sim, "TRANSPORT", 0, 300);
    let passenger = put(&mut sim, "TANK", 0, 350);
    let pad = put(&mut sim, "PAD", 0, 500);
    let expected = sim.checksum();
    for field in 0..13 {
        let mut changed = sim.clone();
        let u = changed.unit_mut(aircraft).unwrap();
        match field {
            0 => u.speed = Fx::ONE,
            1 => u.landed = true,
            2 => u.air_pass = true,
            3 => u.pass_runout = true,
            4 => u.pass_ticks = 1,
            5 => u.idle_ticks = 1,
            6 => u.cruise_alt = Some(Fx::from_int(120)),
            7 => u.hover_alt = Some(Fx::from_int(80)),
            8 => u.cargo.push(passenger),
            9 => u.transport_job = Some((passenger, 1)),
            10 => u.air_pad = Some((pad, None)),
            11 => u.path_goal = Some(FxVec3::from_ints(700, 140, 300)),
            _ => u.repair_trip = true,
        }
        assert_ne!(changed.checksum(), expected, "flight/cargo divergence must be detected (field {field})");
    }
}

#[test]
fn holding_position_or_fire_does_not_start_an_idle_fighter_sortie() {
    for (fire, movement) in [(2, 0), (0, 1)] {
        let mut sim = airfield();
        let fighter = put(&mut sim, "FIGHTER", 0, 300);
        sim.unit_mut(fighter).unwrap().pos.y = Fx::from_int(140);
        sim.unit_mut(fighter).unwrap().fire_state = fire;
        sim.unit_mut(fighter).unwrap().move_state = movement;
        let enemy = put(&mut sim, "TRANSPORT", 1, 450);
        sim.unit_mut(enemy).unwrap().pos.y = Fx::from_int(140);
        sim.give_orders(enemy, [Order::Wait(1000)]);
        for _ in 0..40 { sim.step(&[]); }
        let u = sim.unit(fighter).unwrap();
        assert!(u.orders.is_empty() && !u.air_pass && u.pos.x == Fx::from_int(300), "standing orders remain effective ({fire}, {movement})");
    }
}

#[test]
fn scripted_deck_height_and_reservations_keep_planes_out_of_the_building() {
    use crate::defs::{ModelDef, PieceDef};
    use ta_formats::cob::{CobFunction, CobScript, op};
    let mut sim = airfield();
    let kind = sim.types.id("PAD").unwrap();
    let t = &mut Arc::get_mut(&mut sim.types).unwrap().types[kind];
    t.model = Arc::new(ModelDef { pieces: vec![PieceDef { name: "deck".into(), parent: None, offset: FxVec3::from_ints(0, 10, 0), top: Fx::from_int(20) }], height: Fx::from_int(30), reach: Fx::from_int(30) });
    t.script = Some(Arc::new(crate::cob::CobProgram::new(CobScript { pieces: vec!["deck".into()], functions: vec![CobFunction { name: "QueryLandingPad".into(), offset: 0 }], code: vec![op::PUSH_CONSTANT, 0, op::RETURN], ..Default::default() }, &["deck".into()])));
    let pad = put(&mut sim, "PAD", 0, 300);
    let a = put(&mut sim, "FIGHTER", 0, 300);
    let b = put(&mut sim, "FIGHTER", 0, 300);
    for id in [a, b] { sim.unit_mut(id).unwrap().pos.y = Fx::from_int(140); }
    for _ in 0..300 { sim.step(&[]); }
    assert_eq!(sim.unit(a).unwrap().air_pad, Some((pad, Some(0))));
    assert_eq!(sim.unit(a).unwrap().pos.y, sim.unit(pad).unwrap().pos.y + Fx::from_int(30));
    assert!(sim.unit(a).unwrap().landed);
    assert_ne!(sim.unit(b).unwrap().air_pad, sim.unit(a).unwrap().air_pad, "one physical pad has one reservation");
    assert!(!sim.unit(a).unwrap().pos.within_xz(sim.unit(b).unwrap().pos, Fx::from_int(8)));
    sim.step(&[Command::Order { player: 0, units: vec![a], order: Order::Move(FxVec3::from_ints(700, 60, 300)), queue: false }]);
    assert!(sim.unit(a).unwrap().air_pad.is_none() && !sim.unit(a).unwrap().landed, "flight orders release the reservation");
}

#[test]
fn a_snapshot_continues_a_parked_carrier_and_loaded_air_transport_identically() {
    let mut map = flat_map();
    map.sea_level = 150;
    let mut sim = airfield_on_map(&map);
    let carrier = put(&mut sim, "CARRIER", 0, 300);
    let fighter = put(&mut sim, "FIGHTER", 0, 300);
    sim.unit_mut(fighter).unwrap().pos.y = Fx::from_int(230);
    for _ in 0..240 { sim.step(&[]); }
    assert!(sim.unit(fighter).unwrap().landed && sim.unit(fighter).unwrap().air_pad.is_some());
    let transport = put(&mut sim, "TRANSPORT", 0, 600);
    let passenger = put(&mut sim, "TANK", 0, 600);
    sim.attach(transport, passenger, None);
    let mut restored = sim.clone();
    let command = Command::Order { player: 0, units: vec![carrier], order: Order::Move(FxVec3::from_ints(300, 150, 700)), queue: false };
    sim.step(std::slice::from_ref(&command));
    restored.step(&[command]);
    for _ in 0..120 {
        sim.step(&[]);
        restored.step(&[]);
        assert_eq!(sim.checksum(), restored.checksum());
    }
    assert_eq!(sim.unit(passenger).unwrap().carried_by, Some(transport));
}

/// A 64 × 64-cell map whose heights `height(x, z)` gives (sea at `sea`).
fn shaped_map(sea: i32, height: impl Fn(usize, usize) -> u8) -> TntMap {
    let mut map = flat_map();
    map.sea_level = sea;
    for z in 0..64 {
        for x in 0..64 {
            map.heights[z * 64 + x] = height(x, z);
        }
    }
    map
}

/// Level ground at 60 west of cell 20, a hillside rising 6 px a cell to cell 40, a plateau at
/// 180 beyond (each 16 px cell's corners: the one-cell FIGHTER may vary by 3).
fn hillside() -> TntMap {
    shaped_map(0, |x, _| if x < 20 { 60 } else if x < 40 { 60 + 6 * (x as u8 - 20) } else { 180 })
}

/// Steps an idle aircraft until it has landed and settled (or `ticks` run out).
fn settle(sim: &mut Sim, id: UnitId, ticks: u32) {
    for _ in 0..ticks {
        sim.step(&[]);
        if sim.unit(id).unwrap().landed {
            break;
        }
    }
    // (It touches down within a pixel of where it rests.)
    for _ in 0..5 { sim.step(&[]); }
}

/// Whether a landed aircraft rests where it may: flat ground under it, at its height.
fn rests_level(sim: &Sim, id: UnitId) -> bool {
    let u = sim.unit(id).unwrap();
    u.landed && sim.world.air_landing_height(u.pos, &sim.unit_type(u).landing()) == Some(u.pos.y)
}

#[test]
fn idle_aircraft_land_on_the_nearest_level_ground_not_on_a_hillside() {
    // Left over the hillside: down to its foot (west of 312 px) or up on the plateau (east of
    // 648), whichever is nearer; of two as near, the western.
    for (x, foot) in [(440, true), (560, false), (480, true)] {
        let mut sim = airfield_on_map(&hillside());
        let fighter = put(&mut sim, "FIGHTER", 0, x);
        settle(&mut sim, fighter, 900);
        let u = sim.unit(fighter).unwrap();
        assert!(rests_level(&sim, fighter), "from {x} it must land on level ground: {:?}, landed {}", u.pos, u.landed);
        let px = sim.unit(fighter).unwrap().pos.x.to_int();
        assert!(if foot { (280..=312).contains(&px) } else { (648..=680).contains(&px) }, "from {x}: the nearest level ground, not {px}");
    }
}

#[test]
fn with_no_level_ground_near_an_idle_aircraft_stays_in_the_air() {
    // A hillside everywhere: 4 px a cell, more than the FIGHTER's 3 under its one cell.
    let mut sim = airfield_on_map(&shaped_map(0, |x, _| 2 + 4 * x as u8));
    let fighter = put(&mut sim, "FIGHTER", 0, 300);
    for _ in 0..900 { sim.step(&[]); }
    let u = sim.unit(fighter).unwrap();
    let ground = sim.world.height_at(u.pos.x, u.pos.z);
    assert!(!u.landed && u.path_goal.is_none(), "nowhere to land: it hovers ({:?}, goal {:?})", u.pos, u.path_goal);
    assert!((u.pos.y - ground - Fx::from_int(80)).abs() <= Fx::from_int(2), "at its cruising height over the hillside: {:?} over {ground:?}", u.pos);
}

#[test]
fn aircraft_sent_to_one_place_take_separate_ground_to_land_on() {
    let mut sim = airfield();
    let fighters: Vec<_> = (0..4).map(|_| put(&mut sim, "FIGHTER", 0, 300)).collect();
    for _ in 0..1200 { sim.step(&[]); }
    for (i, &a) in fighters.iter().enumerate() {
        assert!(rests_level(&sim, a), "fighter {i} landed: {:?}", sim.unit(a).unwrap().pos);
        for &b in &fighters[i + 1..] {
            let (pa, pb) = (sim.unit(a).unwrap().pos, sim.unit(b).unwrap().pos);
            assert!(!pa.within_xz(pb, Fx::from_int(15)), "each takes ground of its own, not the crowd's to push apart: {pa:?} {pb:?}");
        }
    }
}

#[test]
fn a_plane_reaches_a_place_to_land_nearer_than_its_turning_circle() {
    // The second finds the ground under it taken and moves over a cell or two: flying as it
    // does to a target it would circle that place for ever.
    let mut sim = airfield();
    let planes = [put(&mut sim, "GLIDER", 0, 300), put(&mut sim, "GLIDER", 0, 300)];
    sim.unit_mut(planes[1]).unwrap().heading = 16384;
    for _ in 0..900 { sim.step(&[]); }
    for id in planes {
        assert!(rests_level(&sim, id), "both land: {:?}, goal {:?}", sim.unit(id).unwrap().pos, sim.unit(id).unwrap().path_goal);
    }
    assert!(!sim.unit(planes[0]).unwrap().pos.within_xz(sim.unit(planes[1]).unwrap().pos, Fx::from_int(15)));
}

#[test]
fn seaplanes_set_down_on_the_water_and_other_aircraft_do_not() {
    let mut map = flat_map();
    map.sea_level = 150;
    let mut sim = airfield_on_map(&map);
    let seaplane = put(&mut sim, "SEAPLANE", 0, 300);
    let fighter = put(&mut sim, "FIGHTER", 0, 700);
    for _ in 0..900 { sim.step(&[]); }
    let (s, f) = (sim.unit(seaplane).unwrap(), sim.unit(fighter).unwrap());
    assert!(s.landed && s.pos.y == Fx::from_int(150), "a seaplane (Amphibious) lands on the sea's surface: {:?}", s.pos);
    assert!(!f.landed && f.pos.y > Fx::from_int(200), "a fighter stays up over the sea: {:?}", f.pos);
}

#[test]
fn flying_to_its_place_to_land_an_aircraft_climbs_over_a_ridge_not_through_it() {
    // A hillside rising 5 px a cell (nowhere to land), a ridge peaking at 220 (cells 28 to 30)
    // and level ground at 60 beyond it; low over the hillside, it must go over the ridge.
    let map = shaped_map(0, |x, _| match x { 0..=27 => 30 + 5 * x as u8, 29 => 220, 28 | 30 => 200, _ => 60 });
    let mut sim = airfield_on_map(&map);
    let fighter = put(&mut sim, "FIGHTER", 0, 424);
    sim.set_cruise_alt(fighter, Some(Fx::from_int(20)));
    let mut lowest = Fx::MAX;
    for _ in 0..900 {
        sim.step(&[]);
        let u = sim.unit(fighter).unwrap();
        if u.landed { break; }
        // (Over the ridge's faces, short of where it comes down.)
        if (432..496).contains(&u.pos.x.to_int()) {
            lowest = lowest.min(u.pos.y - sim.world.height_at(u.pos.x, u.pos.z));
        }
    }
    assert!(rests_level(&sim, fighter) && sim.unit(fighter).unwrap().pos.x.to_int() >= 504, "it lands beyond the ridge: {:?}", sim.unit(fighter).unwrap().pos);
    assert!(lowest >= Fx::from_int(6), "over the ridge it keeps clear of the ground ({lowest:?} at the lowest)");
}

#[test]
fn aircraft_keep_their_wings_off_the_foot_of_a_cliff() {
    // Level ground at 60, a cliff up to 200 from 480 px: a fighter whose wings reach 40 px
    // lands no nearer the cliff than that (and half a cell).
    let mut sim = airfield_on_map(&shaped_map(0, |x, _| if x < 30 { 60 } else { 200 }));
    let kind = sim.types.id("FIGHTER").unwrap();
    let t = &mut Arc::get_mut(&mut sim.types).unwrap().types[kind];
    t.model = Arc::new(crate::defs::ModelDef { pieces: Vec::new(), height: Fx::from_int(10), reach: Fx::from_int(40) });
    let fighter = put(&mut sim, "FIGHTER", 0, 456);
    settle(&mut sim, fighter, 900);
    let u = sim.unit(fighter).unwrap();
    assert!(rests_level(&sim, fighter) && u.pos.x.to_int() <= 432, "its wings would cut into the cliff at {:?}", u.pos);
}

/// A plane with nothing to do over the sea, where it cannot set down, circles round where it came
/// to rest (as TA's do), neither hanging still there nor flying off to land on the nearest shore.
#[test]
fn planes_with_nowhere_to_set_down_circle_where_they_came_to_rest() {
    let mut map = flat_map();
    map.sea_level = 150;
    let mut sim = airfield_on_map(&map);
    let fighter = put(&mut sim, "FIGHTER", 0, 300);
    {
        let u = sim.unit_mut(fighter).unwrap();
        u.landed = false;
        u.pos.y = Fx::from_int(150 + 80);
    }
    let spot = FxVec3::from_ints(700, 0, 700);
    sim.give_orders(fighter, [Order::Move(spot)]);
    for _ in 0..600 { sim.step(&[]); }
    let (mut moved, mut farthest, mut last) = (Fx::ZERO, Fx::ZERO, sim.unit(fighter).unwrap().pos);
    for _ in 0..600 {
        sim.step(&[]);
        let u = sim.unit(fighter).unwrap();
        assert!(u.orders.is_empty() && !u.landed, "idle and in the air");
        moved += (u.pos - last).length_xz();
        farthest = farthest.max((u.pos - spot).length_xz());
        last = u.pos;
    }
    let u = sim.unit(fighter).unwrap();
    assert!(moved > Fx::from_int(1000), "it keeps flying: {moved:?} in twenty seconds");
    assert!(farthest < Fx::from_int(300), "round the spot it was sent to: as far as {farthest:?} from it");
    assert!(u.orbit.is_some(), "circling");
    // Told to go somewhere, it goes.
    let there = FxVec3::from_ints(200, 0, 200);
    sim.give_orders(fighter, [Order::Move(there)]);
    for _ in 0..300 { sim.step(&[]); }
    assert!(sim.unit(fighter).unwrap().pos.within_xz(there, Fx::from_int(200)), "and goes where it is sent");
}

/// Hovering aircraft other than gunships (scouts, builders, transports) with nothing to do over
/// the sea circle where they came to rest, as planes do, rather than hang there on top of each
/// other: several sent to one place keep apart, each round its own size of circle, and a
/// slow-turning builder loiters slowly. A gunship hovers where it is.
#[test]
fn hovering_aircraft_with_nowhere_to_set_down_circle_apart() {
    let mut map = flat_map();
    map.sea_level = 150;
    let mut sim = airfield_on_map(&map);
    let spot = FxVec3::from_ints(700, 0, 700);
    let fleet: Vec<UnitId> = ["TRANSPORT", "TRANSPORT", "AIRCON", "AIRCON", "GUNSHIP"].iter().map(|name| {
        let id = put(&mut sim, name, 0, 300);
        let u = sim.unit_mut(id).unwrap();
        u.landed = false;
        u.pos.y = Fx::from_int(150 + 80);
        id
    }).collect();
    for &id in &fleet {
        sim.give_orders(id, [Order::Move(spot)]);
    }
    for _ in 0..900 { sim.step(&[]); }
    let (circling, gunship) = (&fleet[..4], fleet[4]);
    let mut moved = [Fx::ZERO; 5];
    let mut last: Vec<FxVec3> = fleet.iter().map(|&id| sim.unit(id).unwrap().pos).collect();
    // (Ticks each pair spends within 24 px of each other: passing, a moment; stacked, all.)
    let mut together = [0u32; 6];
    for _ in 0..600 {
        sim.step(&[]);
        for (k, &id) in fleet.iter().enumerate() {
            let u = sim.unit(id).unwrap();
            assert!(u.orders.is_empty() && !u.landed, "idle and in the air");
            assert!((u.pos - spot).length_xz() < Fx::from_int(450), "aircraft {k} round the spot it was sent to: {:?}", u.pos);
            moved[k] += (u.pos - last[k]).length_xz();
            last[k] = u.pos;
        }
        let mut pair = 0;
        for (a, &p) in circling.iter().enumerate() {
            for &q in &circling[a + 1..] {
                together[pair] += u32::from((sim.unit(p).unwrap().pos - sim.unit(q).unwrap().pos).length_xz() < Fx::from_int(24));
                pair += 1;
            }
        }
    }
    for (k, &id) in circling.iter().enumerate() {
        assert!(moved[k] > Fx::from_int(300), "aircraft {k} keeps flying: {:?} in twenty seconds", moved[k]);
        assert!(sim.unit(id).unwrap().orbit.is_some(), "aircraft {k} circling");
    }
    assert!(together.iter().all(|&t| t < 60), "none stays on top of another: ticks together {together:?}");
    // (The slow builders loiter: slower than the transports.)
    assert!(moved[2] < moved[0] && moved[3] < moved[1], "{moved:?}");
    assert!(moved[4] < Fx::from_int(20) && sim.unit(gunship).unwrap().orbit.is_none(), "the gunship hovers: {:?}", moved[4]);
}

/// Planes sent at one target together come in on it across a front, each from its place in the
/// group (0, 1, -1, 2, -2...: `Unit::fan`), not one behind the other; one sent alone, and a group
/// given another order, straight at it.
#[test]
fn a_group_of_planes_sent_at_one_target_fans_out() {
    let mut sim = airfield();
    let target = sim.spawn_unit(sim.types.id("TANK").unwrap(), 1, FxVec3::from_ints(950, 60, 500), 0, true).unwrap();
    let planes: Vec<UnitId> = (0..5).map(|i| {
        let id = sim.spawn_unit(sim.types.id("FIGHTER").unwrap(), 0, FxVec3::from_ints(60, 60, 480 + i * 8), 0, true).unwrap();
        let u = sim.unit_mut(id).unwrap();
        u.landed = false;
        u.pos.y = Fx::from_int(60 + 80);
        id
    }).collect();
    sim.step(&[Command::Order { player: 0, units: planes.clone(), order: Order::Attack(Target::Unit(target)), queue: false }]);
    let fans: Vec<i8> = planes.iter().map(|&p| sim.unit(p).unwrap().fan).collect();
    assert_eq!(fans, [0, 1, -1, 2, -2], "their places across the front");
    sim.step(&[]);
    // Far off, each heads for its own place beside the target, the spacing apart.
    let tp = sim.unit(target).unwrap().pos;

    let off = |p: UnitId| sim.unit(p).unwrap().path_goal.map(|g| (g - tp).length_xz());
    assert!(off(planes[0]).is_some_and(|d| d < Fx::from_int(8)), "the first straight at it: {:?}", off(planes[0]));
    for (k, &p) in planes.iter().enumerate().skip(1) {
        let want = crate::combat::FAN_SPACING * fans[k].unsigned_abs() as i64;
        assert!(off(p).is_some_and(|d| (d - want).abs() < Fx::from_int(16)), "plane {k} comes in {want:?} to its side: {:?}", off(p));
    }
    // One sent alone: straight at it.
    sim.step(&[Command::Order { player: 0, units: vec![planes[3]], order: Order::Attack(Target::Unit(target)), queue: false }]);
    assert_eq!(sim.unit(planes[3]).unwrap().fan, 0, "sent alone, no place in a group");
}

// ---- Repair trips ---------------------------------------------------------------------------

/// Pads and carriers that mend a fighter at a fifth of its health a second (three seconds from
/// 400 of its 1000), so that who waits for whom shows.
fn slow_repairs(sim: &mut Sim) {
    let work = sim.types.by_name("FIGHTER").unwrap().build_work;
    for name in ["PAD", "CARRIER"] {
        let kind = sim.types.id(name).unwrap();
        Arc::get_mut(&mut sim.types).unwrap().types[kind].work_rate = work / 150;
    }
}

/// A fighter of player 0 in flight over (`x`, 300), worn down to `health` (of its 1000), sent on
/// patrol from there to (`x`, 700) and back.
fn on_patrol(sim: &mut Sim, x: i64, health: i64) -> UnitId {
    let fighter = sim.spawn_airborne(sim.types.id("FIGHTER").unwrap(), 0, FxVec3::from_ints(x, 0, 300), 0, None).unwrap();
    sim.unit_mut(fighter).unwrap().health = Fx::from_int(health);
    sim.step(&[Command::Order { player: 0, units: vec![fighter], order: Order::Patrol(FxVec3::from_ints(x, 0, 700)), queue: false }]);
    fighter
}

/// Steps until `id` sets off on a repair trip (it looks once a second).
fn until_trip(sim: &mut Sim, id: UnitId) {
    for _ in 0..30 {
        if sim.unit(id).unwrap().repair_trip {
            return;
        }
        sim.step(&[]);
    }
}

/// Whether `id` has landed on `pad`'s deck.
fn on_deck(sim: &Sim, id: UnitId, pad: UnitId) -> bool {
    sim.unit(id).is_some_and(|u| u.landed && u.air_pad.is_some_and(|(p, _)| p == pad))
}

fn patrolling(u: &Unit) -> bool {
    matches!(u.orders.front(), Some(Order::Patrol(_)))
}

fn mended(u: &Unit) -> bool {
    u.health >= Fx::from_int(1000)
}

/// A fighter on patrol worn below two-thirds of its health breaks off for a repair pad 800 px
/// away, its orders held (as TA's aircraft do), lands on the pad, is mended there and takes up
/// its patrol again.
#[test]
fn a_worn_fighter_on_patrol_flies_to_a_far_pad_is_mended_and_patrols_on() {
    let mut sim = airfield();
    slow_repairs(&mut sim);
    let pad = put(&mut sim, "PAD", 0, 100);
    let fighter = on_patrol(&mut sim, 900, 400);
    let (mut held, mut landed, mut done) = (None, None, None);
    for tick in 0..900 {
        sim.step(&[]);
        let u = sim.unit(fighter).unwrap();
        if held.is_none() && u.repair_trip {
            assert!(tick < 30, "it breaks off within a second");
            assert_eq!(u.air_pad, Some((pad, None)), "its deck taken as it sets off");
            held = Some((tick, u.orders.clone()));
        }
        if landed.is_none() && on_deck(&sim, fighter, pad) {
            landed = Some(tick);
            assert!(u.repair_trip && held.as_ref().is_some_and(|(_, o)| *o == u.orders), "its patrol held on the way, not flown: {:?}", u.orders);
        }
        if mended(u) {
            done = Some(tick);
            break;
        }
    }
    println!("off for the pad at tick {:?}, on its deck at {landed:?}, mended at {done:?}", held.map(|h| h.0));
    assert!(landed.is_some() && done.is_some(), "it lands on the pad and is mended: {:?}", sim.unit(fighter).unwrap().pos);
    for _ in 0..150 {
        sim.step(&[]);
    }
    let (u, at) = (sim.unit(fighter).unwrap(), sim.unit(pad).unwrap().pos);
    assert!(!u.repair_trip && u.air_pad.is_none() && !u.landed && patrolling(u), "mended, it takes off on its patrol: {:?}", u.orders);
    assert!(!u.pos.within_xz(at, Fx::from_int(250)), "and flies off on it: {:?}", u.pos);
}

/// At two-thirds of its health or more a fighter keeps to its patrol; a point less, it goes.
#[test]
fn a_fighter_at_two_thirds_of_its_health_keeps_to_its_patrol() {
    for (health, goes) in [(667, false), (666, true)] {
        let mut sim = airfield();
        put(&mut sim, "PAD", 0, 100);
        let fighter = on_patrol(&mut sim, 900, health);
        let mut went = false;
        for _ in 0..300 {
            sim.step(&[]);
            let u = sim.unit(fighter).unwrap();
            went |= u.repair_trip;
            assert!(patrolling(u) && (u.repair_trip || u.air_pad.is_none() && !u.landed), "its patrol kept ({health}): {:?}", u.orders);
        }
        assert_eq!(went, goes, "at {health} of its 1000 it goes for the pad: {went}");
    }
}

/// Two worn fighters and one pad: one takes it, and the other keeps to its patrol until the pad
/// is free again, then is mended there too. Never are two on one deck.
#[test]
fn worn_fighters_take_turns_at_a_single_pad() {
    let mut sim = airfield();
    slow_repairs(&mut sim);
    let pad = put(&mut sim, "PAD", 0, 100);
    let fighters = [on_patrol(&mut sim, 900, 400), on_patrol(&mut sim, 700, 400)];
    let (mut waited, mut decked, mut done) = (false, [false; 2], [None; 2]);
    for tick in 0..1800 {
        sim.step(&[]);
        let [a, b] = fighters.map(|f| sim.unit(f).unwrap());
        assert!(a.air_pad.is_none() || a.air_pad != b.air_pad, "one deck, one aircraft: {:?}", a.air_pad);
        for (k, (x, y)) in [(a, b), (b, a)].into_iter().enumerate() {
            decked[k] |= on_deck(&sim, x.id, pad);
            // (One being mended on the deck while the other, worn too, goes on with its patrol.)
            waited |= on_deck(&sim, x.id, pad) && !mended(x) && !mended(y) && !y.repair_trip && patrolling(y);
            if done[k].is_none() && mended(x) {
                done[k] = Some(tick);
            }
        }
        if done.iter().all(Option::is_some) {
            break;
        }
    }
    println!("mended at ticks {done:?}");
    assert!(waited, "the second keeps to its patrol while the first is mended");
    assert!(decked == [true; 2] && done.iter().all(Option::is_some), "both are mended on the pad in turn: on its deck {decked:?}, mended {done:?}");
}

/// An idle fighter that needs no mending, parked on the only pad, gives it up to a worn one: it
/// takes off and lands on the ground near by, and the worn one lands on the pad and is mended.
#[test]
fn an_idle_fighter_gives_up_the_only_pad_to_a_worn_one() {
    let mut sim = airfield();
    slow_repairs(&mut sim);
    let pad = put(&mut sim, "PAD", 0, 100);
    let idle = put(&mut sim, "FIGHTER", 0, 100);
    sim.unit_mut(idle).unwrap().pos.y = Fx::from_int(140);
    for _ in 0..300 {
        sim.step(&[]);
        if on_deck(&sim, idle, pad) {
            break;
        }
    }
    assert!(on_deck(&sim, idle, pad), "the idle fighter parks on the pad: {:?}", sim.unit(idle).unwrap().pos);
    let worn = on_patrol(&mut sim, 900, 400);
    until_trip(&mut sim, worn);
    let (i, w) = (sim.unit(idle).unwrap(), sim.unit(worn).unwrap());
    assert!(w.repair_trip && w.air_pad == Some((pad, None)), "the worn fighter takes the pad: {:?}", w.air_pad);
    assert!(i.air_pad.is_none() && !i.landed, "the idle one gives it up and takes off: {:?}, landed {}", i.air_pad, i.landed);
    let mut decked = false;
    for _ in 0..900 {
        sim.step(&[]);
        decked |= on_deck(&sim, worn, pad);
        assert!(sim.unit(idle).unwrap().air_pad.is_none(), "the idle one does not take the pad back");
        if mended(sim.unit(worn).unwrap()) && sim.unit(idle).unwrap().landed {
            break;
        }
    }
    assert!(decked && mended(sim.unit(worn).unwrap()), "the worn one lands on the pad and is mended");
    settle(&mut sim, idle, 300);
    let at = sim.unit(pad).unwrap().pos;
    assert!(rests_level(&sim, idle) && sim.unit(idle).unwrap().pos.within_xz(at, Fx::from_int(16 * 16 + 24)), "the idle one lands on the ground near by: {:?}", sim.unit(idle).unwrap().pos);
}

/// A damaged fighter sent onto the only pad (a right-click on it) takes it from an idle one that
/// needs no mending, as one on a repair trip does: it lands on it and is mended, where it would
/// otherwise have set down beside the pad and, mended only on its deck, never been.
#[test]
fn a_damaged_fighter_sent_onto_an_occupied_pad_takes_it_from_an_idle_one() {
    let mut sim = airfield();
    let pad = put(&mut sim, "PAD", 0, 300);
    let idle = put(&mut sim, "FIGHTER", 0, 300);
    sim.unit_mut(idle).unwrap().pos.y = Fx::from_int(140);
    for _ in 0..300 {
        sim.step(&[]);
        if on_deck(&sim, idle, pad) {
            break;
        }
    }
    assert!(on_deck(&sim, idle, pad), "the idle fighter parks on the pad");
    // Above two-thirds of its health: no repair trip, only the move.
    let damaged = sim.spawn_airborne(sim.types.id("FIGHTER").unwrap(), 0, FxVec3::from_ints(700, 0, 300), 0, None).unwrap();
    sim.unit_mut(damaged).unwrap().health = Fx::from_int(800);
    let at = sim.unit(pad).unwrap().pos;
    sim.step(&[Command::Order { player: 0, units: vec![damaged], order: Order::Move(at), queue: false }]);
    let mut decked = false;
    for _ in 0..600 {
        sim.step(&[]);
        assert!(!sim.unit(damaged).unwrap().repair_trip, "moved, not on a mission: no trip");
        decked |= on_deck(&sim, damaged, pad);
        if mended(sim.unit(damaged).unwrap()) {
            break;
        }
    }
    assert!(decked && mended(sim.unit(damaged).unwrap()), "it takes the pad and is mended: {:?}", sim.unit(damaged).unwrap().pos);
    assert!(sim.unit(idle).unwrap().air_pad.is_none(), "the idle one gave it up");
}

/// A pad switched off takes no aircraft and mends none: a worn fighter on its way there when it
/// is switched off takes up its patrol again, and goes to no pad while it is off.
#[test]
fn a_pad_switched_off_is_not_used_and_mends_nothing() {
    let mut sim = airfield();
    let kind = sim.types.id("PAD").unwrap();
    {
        let t = &mut Arc::get_mut(&mut sim.types).unwrap().types[kind];
        t.def.on_offable = true;
        t.def.activate_when_built = true;
    }
    let pad = put(&mut sim, "PAD", 0, 100);
    assert!(sim.unit(pad).unwrap().activated, "switched on as it is finished");
    let fighter = on_patrol(&mut sim, 900, 400);
    until_trip(&mut sim, fighter);
    assert!(sim.unit(fighter).unwrap().repair_trip, "off to the pad while it is on");
    sim.step(&[Command::SetActive { player: 0, units: vec![pad], on: false }]);
    let u = sim.unit(fighter).unwrap();
    assert!(!u.repair_trip && u.air_pad.is_none() && patrolling(u), "the pad switched off, its patrol taken up again: {:?}, {:?}", u.air_pad, u.orders);
    for _ in 0..300 {
        sim.step(&[]);
        let u = sim.unit(fighter).unwrap();
        assert!(!u.repair_trip && u.air_pad.is_none() && !u.landed, "no trip to a pad switched off");
    }
    // Nor does it mend an aircraft on its deck; switched on again, it does.
    let parked = put(&mut sim, "FIGHTER", 0, 100);
    {
        let u = sim.unit_mut(parked).unwrap();
        u.health = Fx::from_int(500);
        u.landed = true;
        u.air_pad = Some((pad, None));
    }
    sim.grid.rebuild(sim.units.iter().flatten());
    let t = sim.types.get(kind).clone();
    for _ in 0..5 {
        sim.tick += 1;
        sim.update_air_base(pad, &t);
    }
    assert_eq!(sim.unit(parked).unwrap().health, Fx::from_int(500), "a pad switched off mends nothing");
    sim.unit_mut(pad).unwrap().activated = true;
    for _ in 0..5 {
        sim.tick += 1;
        sim.update_air_base(pad, &t);
    }
    assert!(sim.unit(parked).unwrap().health > Fx::from_int(500), "switched on, it mends");
}

/// A pad mends the aircraft on its deck, not one parked on the ground beside it (40 px from its
/// middle, where pads used to mend them too).
#[test]
fn a_pad_mends_the_aircraft_on_its_deck_not_those_parked_beside_it() {
    let mut sim = airfield();
    let pad = put(&mut sim, "PAD", 0, 300);
    let (beside, on) = (put(&mut sim, "FIGHTER", 0, 340), put(&mut sim, "FIGHTER", 0, 300));
    for id in [beside, on] {
        let u = sim.unit_mut(id).unwrap();
        u.health = Fx::from_int(500);
        u.landed = true;
    }
    sim.unit_mut(on).unwrap().air_pad = Some((pad, None));
    sim.grid.rebuild(sim.units.iter().flatten());
    let t = sim.types.by_name("PAD").unwrap().clone();
    for _ in 0..5 {
        sim.tick += 1;
        sim.update_air_base(pad, &t);
    }
    assert!(sim.unit(on).unwrap().health > Fx::from_int(500), "the one on its deck is mended");
    assert_eq!(sim.unit(beside).unwrap().health, Fx::from_int(500), "the one parked beside it is not");
}

/// A new order calls a repair trip off: the deck given up, the order carried out. An order
/// queued after those it holds does not.
#[test]
fn a_new_order_calls_a_repair_trip_off_and_a_queued_one_does_not() {
    let mut sim = airfield();
    let pad = put(&mut sim, "PAD", 0, 100);
    let fighter = on_patrol(&mut sim, 900, 400);
    until_trip(&mut sim, fighter);
    assert!(sim.unit(fighter).unwrap().repair_trip, "off to the pad");
    let later = FxVec3::from_ints(600, 0, 800);
    sim.step(&[Command::Order { player: 0, units: vec![fighter], order: Order::Move(later), queue: true }]);
    let u = sim.unit(fighter).unwrap();
    assert!(u.repair_trip && u.air_pad == Some((pad, None)) && u.orders.back() == Some(&Order::Move(later)), "queued, it goes on to the pad: {:?}", u.orders);
    let there = FxVec3::from_ints(800, 0, 900);
    sim.step(&[Command::Order { player: 0, units: vec![fighter], order: Order::Move(there), queue: false }]);
    let u = sim.unit(fighter).unwrap();
    assert!(!u.repair_trip && u.air_pad.is_none() && u.orders.front() == Some(&Order::Move(there)), "a new order calls the trip off: {:?}, {:?}", u.air_pad, u.orders);
    for _ in 0..240 {
        sim.step(&[]);
        assert!(sim.unit(fighter).unwrap().air_pad.is_none(), "and it does not go to the pad");
    }
    assert!(sim.unit(fighter).unwrap().pos.within_xz(there, Fx::from_int(64)), "it goes where it was sent: {:?}", sim.unit(fighter).unwrap().pos);
}

/// A carrier takes a worn fighter on a repair trip as a pad does, out at sea.
#[test]
fn a_worn_fighter_on_patrol_over_the_sea_is_mended_on_a_carrier() {
    let mut map = flat_map();
    map.sea_level = 150;
    let mut sim = airfield_on_map(&map);
    slow_repairs(&mut sim);
    let carrier = put(&mut sim, "CARRIER", 0, 200);
    let fighter = on_patrol(&mut sim, 900, 400);
    let mut decked = false;
    for _ in 0..900 {
        sim.step(&[]);
        decked |= on_deck(&sim, fighter, carrier);
        if mended(sim.unit(fighter).unwrap()) {
            break;
        }
    }
    assert!(decked && mended(sim.unit(fighter).unwrap()), "it lands on the carrier and is mended there: {:?}", sim.unit(fighter).unwrap().pos);
    for _ in 0..150 {
        sim.step(&[]);
    }
    let (u, at) = (sim.unit(fighter).unwrap(), sim.unit(carrier).unwrap().pos);
    assert!(!u.landed && u.air_pad.is_none() && patrolling(u) && !u.pos.within_xz(at, Fx::from_int(250)), "then patrols on: {:?}, {:?}", u.pos, u.orders);
}

/// A plane that brakes slowly (as TA's Thunder: top speed 9, braking 0.4), sent onto a pad's
/// middle, flies on past it as it stops, beyond the reach an idle aircraft looks for a pad in:
/// it comes back to land on the pad and is mended there, not set down on the ground beyond.
#[test]
fn a_slow_braking_plane_sent_onto_a_pad_lands_on_it() {
    let mut sim = airfield();
    let kind = sim.types.id("FIGHTER").unwrap();
    {
        let t = &mut Arc::get_mut(&mut sim.types).unwrap().types[kind];
        (t.max_speed, t.brake, t.acceleration) = (Fx::from_int(9), Fx::from_ratio(2, 5), Fx::from_ratio(2, 25));
    }
    let pad = put(&mut sim, "PAD", 0, 300);
    let plane = sim.spawn_airborne(kind, 0, FxVec3::from_ints(900, 0, 300), 0, None).unwrap();
    sim.unit_mut(plane).unwrap().health = Fx::from_int(500);
    let at = sim.unit(pad).unwrap().pos;
    sim.step(&[Command::Order { player: 0, units: vec![plane], order: Order::Move(at), queue: false }]);
    let (mut beyond, mut decked) = (Fx::ZERO, false);
    for _ in 0..900 {
        sim.step(&[]);
        let u = sim.unit(plane).unwrap();
        if u.orders.is_empty() && u.air_pad.is_none() {
            beyond = beyond.max((u.pos - at).length_xz());
        }
        decked |= on_deck(&sim, plane, pad);
        if mended(u) {
            break;
        }
    }
    println!("it came to a stop {:.0} px past the pad's middle", beyond.to_f32());
    assert!(beyond > Fx::from_int(32 + 24), "it stops beyond the reach of the pad an idle aircraft has: {beyond:?}");
    assert!(decked && mended(sim.unit(plane).unwrap()), "it lands on the pad and is mended: {:?}", sim.unit(plane).unwrap().pos);
}
