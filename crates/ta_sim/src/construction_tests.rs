//! Construction regressions with synthetic definitions; no installed assets required.

use super::*;

fn yard(worker: &str) -> Sim {
    let data = data_of(&[
        unit("BUILDER", &format!("Builder=1; WorkerTime={worker}; CanMove=1; MaxVelocity=1; BuildDistance=128; CanReclamate=1; CanResurrect=1; CanCapture=1; FootprintX=1; FootprintZ=1;")),
        unit("FRAME", "BuildTime=30; BuildCostMetal=100; BuildCostEnergy=3000; FootprintX=2; FootprintZ=2; Corpse=FRAME_DEAD;"),
        unit("TRANSPORT", "CanMove=1; MaxVelocity=1; TransportCapacity=2; TransportSize=4;"),
        ("features/all/frame_dead.tdf".into(), "[FRAME_DEAD] { footprintx=2; footprintz=2; blocking=1; reclaimable=1; metal=50; damage=1000; }".into()),
    ]);
    let mut sim = sim_of(with_menu(data, "BUILDER", &["FRAME"]), 2);
    for player in &mut sim.players { player.endless = true; }
    sim
}

fn put(sim: &mut Sim, name: &str, owner: u8, x: i64, complete: bool) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), owner, FxVec3::from_ints(x, 60, 300), 0, complete).unwrap()
}

#[test]
fn guarding_an_allied_nanoframe_finishes_it() {
    let mut sim = yard("300");
    sim.set_team(1, 0);
    sim.set_starting_resources(0, Fx::from_int(10000), Fx::from_int(10000));
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let frame = put(&mut sim, "FRAME", 1, 280, false);
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Guard(frame), queue: false }]);
    for _ in 0..120 { sim.step(&[]); }
    assert!(sim.unit(frame).unwrap().is_complete(), "guard must help the guarded frame itself");
    assert_eq!(sim.unit(builder).unwrap().orders.front(), Some(&Order::Guard(frame)), "keep guarding after completion");
}

#[test]
fn construction_never_advances_more_work_than_the_resources_paid_for() {
    let mut sim = yard("0.0000152587890625"); // one raw unit of work a tick
    let frame_type = sim.types.id("FRAME").unwrap();
    Arc::get_mut(&mut sim.types).unwrap().types[frame_type].cost_metal = Fx::from_int(1_000_000);
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let frame = put(&mut sim, "FRAME", 0, 280, false);
    let t = sim.types.get(sim.unit(builder).unwrap().type_id).clone();
    let tt = sim.types.get(sim.unit(frame).unwrap().type_id).clone();
    let price = tt.cost_metal.share(1, tt.build_work);
    assert!(price > Fx(1));
    sim.players[0].economy.metal = price - Fx(1);
    sim.apply_build_power(builder, &t, frame);
    assert_eq!(sim.unit(frame).unwrap().build_work, 0, "one work atom must not be discounted to the available metal");
    sim.players[0].economy.metal = price;
    sim.apply_build_power(builder, &t, frame);
    assert_eq!(sim.unit(frame).unwrap().build_work, 1, "resume when its exact price is available");
    assert_eq!(sim.players[0].economy.metal, Fx::ZERO);
}

#[test]
fn resurrection_spends_partial_energy_instead_of_stalling_below_a_full_tick() {
    let mut sim = yard("300");
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let wreck = sim.place_feature("FRAME_DEAD", FxVec3::from_ints(280, 60, 300), 0).unwrap();
    sim.players[0].economy.energy = Fx::ONE;
    let metal = sim.players[0].economy.metal;
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Resurrect(wreck), queue: false }]);
    assert!(sim.features[wreck].resurrect_work > 0, "one energy buys partial work even when a full tick costs more");
    let t = sim.types.by_name("FRAME").unwrap();
    let paid = t.cost_energy.share(sim.features[wreck].resurrect_work, t.build_work);
    assert_eq!(sim.players[0].economy.energy, Fx::ONE - paid);
    assert_eq!(sim.players[0].economy.metal, metal, "resurrection does not spend metal");
}

#[test]
fn metal_starved_construction_does_not_stall_energy_only_resurrection() {
    let mut sim = yard("30");
    sim.set_starting_resources(0, Fx::ZERO, Fx::from_int(10000));
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let resurrector = put(&mut sim, "BUILDER", 0, 360, true);
    let frame = put(&mut sim, "FRAME", 0, 280, false);
    let wreck = sim.place_feature("FRAME_DEAD", FxVec3::from_ints(440, 60, 300), 0).unwrap();
    sim.step(&[
        Command::Order { player: 0, units: vec![builder], order: Order::Repair(frame), queue: false },
        Command::Order { player: 0, units: vec![resurrector], order: Order::Resurrect(wreck), queue: false },
    ]);
    for _ in 0..11 { sim.step(&[]); }
    let economy = &sim.players[0].economy;
    assert!(economy.metal_demand > Fx::ZERO && economy.build_factor == Fx::ZERO, "the unrelated build remains metal-starved");
    assert_eq!(sim.unit(frame).unwrap().build_work, 0, "construction cannot spend unavailable metal");
    assert_eq!(sim.features[wreck].resurrect_work, 12 * sim.types.by_name("BUILDER").unwrap().work_rate, "resurrection keeps its full energy-funded pace");
    assert_eq!(economy.metal, Fx::ZERO);
    assert_eq!(economy.energy, Fx::from_int(8800), "only resurrection's exact energy price is charged");
}

#[test]
fn resurrection_draw_is_reported_and_suspended_while_paralyzed() {
    let mut sim = yard("300");
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let wreck = sim.place_feature("FRAME_DEAD", FxVec3::from_ints(280, 60, 300), 0).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Resurrect(wreck), queue: false }]);
    assert!(sim.unit_flows(sim.unit(builder).unwrap()).energy_use > Fx::ZERO, "show the energy draw of resurrection");
    sim.unit_mut(builder).unwrap().paralyzed_until = sim.tick + 100;
    assert_eq!(sim.unit_flows(sim.unit(builder).unwrap()).energy_use, Fx::ZERO, "a frozen constructor has no work draw");
}

#[test]
fn fully_paid_resurrection_waits_at_the_unit_cap_without_further_draw() {
    let mut sim = yard("3000");
    sim.set_starting_resources(0, Fx::from_int(10000), Fx::from_int(10000));
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let wreck = sim.place_feature("FRAME_DEAD", FxVec3::from_ints(280, 60, 300), 0).unwrap();
    sim.unit_limit = 1;
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Resurrect(wreck), queue: false }]);
    assert_eq!(sim.features[wreck].resurrected, Fx::ONE);
    assert!(sim.features[wreck].alive);
    assert_eq!(sim.unit_flows(sim.unit(builder).unwrap()).energy_use, Fx::ZERO);
    sim.players[0].economy.energy = Fx::ZERO;
    sim.unit_limit = 2;
    sim.step(&[]);
    assert!(!sim.features[wreck].alive, "paid work completes when room opens, even without more energy");
    assert_eq!(sim.players[0].unit_count, 2);
}

#[test]
fn resurrection_respects_the_allowed_unit_roster() {
    let mut sim = yard("3000");
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let wreck = sim.place_feature("FRAME_DEAD", FxVec3::from_ints(280, 60, 300), 0).unwrap();
    let mut allowed = vec![true; sim.types.types.len()];
    allowed[sim.types.id("FRAME").unwrap()] = false;
    sim.allowed = Some(allowed);
    sim.set_starting_resources(0, Fx::from_int(10000), Fx::from_int(10000));
    let energy = sim.players[0].economy.energy;
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Resurrect(wreck), queue: false }]);
    assert_eq!(sim.features[wreck].resurrect_work, 0, "a banned unit cannot be produced through its wreck");
    assert_eq!(sim.players[0].economy.energy, energy);
    assert!(sim.features[wreck].alive);
    assert!(sim.unit(builder).unwrap().orders.is_empty());
}

#[test]
fn factory_release_keeps_an_order_given_as_its_product_completes() {
    let mut sim = sim_of(construction_data(), 1);
    sim.players[0].endless = true;
    let factory = put(&mut sim, "FACTORY", 0, 300, true);
    let tank = sim.types.id("TANK").unwrap();
    sim.unit_mut(factory).unwrap().in_build_stance = true;
    sim.step(&[Command::Produce { player: 0, factory, type_id: tank, count: 1 }]);
    sim.step(&[]);
    let product = sim.unit(factory).unwrap().build.unwrap().target;
    sim.finish_frame(product);
    let destination = FxVec3::from_ints(800, 60, 300);
    sim.step(&[Command::Order { player: 0, units: vec![product], order: Order::Move(destination), queue: false }]);
    assert!(sim.unit(product).unwrap().orders.contains(&Order::Move(destination)), "factory exit/rally must not replace the player's order");
    assert!(sim.unit(factory).unwrap().factory_queue.is_empty());
}

#[test]
fn loaded_units_cannot_be_repaired_reclaimed_or_captured_from_outside() {
    for action in 0..3 {
        let mut sim = yard("300");
        let builder = put(&mut sim, "BUILDER", 0, 200, true);
        let owner = if action == 2 { 1 } else { 0 };
        let carrier = put(&mut sim, "TRANSPORT", owner, 280, true);
        let passenger = put(&mut sim, "BUILDER", owner, 320, true);
        sim.unit_mut(passenger).unwrap().health = Fx::from_int(500);
        sim.attach(carrier, passenger, None);
        let order = match action { 0 => Order::Repair(passenger), 1 => Order::ReclaimUnit(passenger), _ => Order::Capture(passenger) };
        sim.step(&[Command::Order { player: 0, units: vec![builder], order, queue: false }]);
        for _ in 0..8 { sim.step(&[]); }
        let u = sim.unit(passenger).expect("the passenger remains aboard");
        assert_eq!(u.health, Fx::from_int(500), "inaccessible cargo must not be worked on (action {action})");
        assert!(u.is_complete());
        assert_eq!(u.capture_work, 0);
        assert_eq!(u.carried_by, Some(carrier));
        assert!(sim.unit(builder).unwrap().orders.is_empty());
    }
}

#[test]
fn capture_stops_when_its_enemy_target_becomes_hidden() {
    let mut sim = yard("30");
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let target = put(&mut sim, "BUILDER", 1, 280, true);
    sim.los_enabled = true;
    sim.vision.set_all(0, vision::LOS);
    sim.give_orders(builder, [Order::Capture(target)]);
    let t = sim.types.by_name("BUILDER").unwrap().clone();
    sim.update_capture_order(builder, &t, target);
    sim.update_capture_order(builder, &t, target);
    let work = sim.unit(target).unwrap().capture_work;
    assert!(work > 0);
    sim.vision.rebuild(&[]);
    assert_eq!(sim.seen_by(sim.unit(target).unwrap(), 0), Seen::Hidden);
    sim.update_capture_order(builder, &t, target);
    assert_eq!(sim.unit(target).unwrap().capture_work, work, "capture cannot work on an unseen live position");
    assert!(sim.unit(builder).unwrap().orders.is_empty());
    assert!(!sim.unit(builder).unwrap().in_build_stance);
}

#[test]
fn capture_waits_for_room_at_the_unit_limit() {
    let mut sim = yard("3000");
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let target = put(&mut sim, "BUILDER", 1, 280, true);
    sim.unit_limit = 1;
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Capture(target), queue: false }]);
    for _ in 0..4 { sim.step(&[]); }
    assert_eq!(sim.unit(target).unwrap().owner, 1, "capture must not exceed the ownership limit");
    assert_eq!(sim.unit(target).unwrap().capture_progress, Fx::ONE, "keep completed work while waiting for room");
    sim.unit_limit = 2;
    sim.step(&[]);
    assert_eq!(sim.unit(target).unwrap().owner, 0);
    assert_eq!(sim.players[0].unit_count, 2);
}

#[test]
fn capturing_takes_four_times_the_cost_never_more_than_two_minutes_of_a_commanders_work() {
    let data = data_of(&[
        unit("BUILDER", "Builder=1; WorkerTime=300; CanMove=1; MaxVelocity=1; BuildDistance=128; CanCapture=1; FootprintX=1; FootprintZ=1;"),
        unit("SCOUT", "BuildCostMetal=100; BuildCostEnergy=600; FootprintX=1; FootprintZ=1;"),
        unit("GATE", "BuildCostMetal=200000; BuildCostEnergy=2000000; FootprintX=2; FootprintZ=2;"),
    ]);
    // The scout: four times (100 metal, 600 / 60 energy and 20), at 300 a second: 1.73 s. The
    // gate's four times its cost would be some 52 minutes: two minutes.
    for (name, ticks) in [("SCOUT", 52), ("GATE", 120 * 30)] {
        let mut sim = sim_of(data.clone(), 2);
        for player in &mut sim.players { player.endless = true; }
        let builder = put(&mut sim, "BUILDER", 0, 200, true);
        let target = put(&mut sim, name, 1, 280, true);
        sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Capture(target), queue: false }]);
        let mut worked = 0;
        for _ in 0..30 * 600 {
            sim.step(&[]);
            let u = sim.unit(target).unwrap();
            if u.owner == 0 {
                worked += 1;
                break;
            }
            if u.capture_progress > Fx::ZERO { worked += 1; }
        }
        assert_eq!(sim.unit(target).unwrap().owner, 0, "{name} captured");
        assert_eq!(worked, ticks, "{name}: ticks of work");
    }
}

#[test]
fn fractional_repairs_accumulate_instead_of_rounding_to_zero_forever() {
    let data = data_of(&[
        unit("BUILDER", "Builder=1; CanMove=1; MaxVelocity=1; WorkerTime=0.3; BuildDistance=128;"),
        unit("PATIENT", "BuildTime=1000; MaxDamage=1;"),
    ]);
    let mut sim = sim_of(data, 1);
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let patient = put(&mut sim, "PATIENT", 0, 280, true);
    sim.unit_mut(patient).unwrap().health = Fx::from_ratio(1, 2);
    let t = sim.types.get(sim.unit(builder).unwrap().type_id).clone();
    let tt = sim.types.get(sim.unit(patient).unwrap().type_id).clone();
    for _ in 0..100 { sim.apply_build_power(builder, &t, patient); }
    let expected = Fx::from_ratio(1, 2) + tt.max_health.share(100 * t.work_rate, tt.build_work);
    assert!(expected > Fx::from_ratio(1, 2));
    assert_eq!(sim.unit(patient).unwrap().health, expected, "sub-precision healing must carry into later ticks");
}

#[test]
fn repair_pads_accumulate_precise_healing_at_the_modified_work_rate() {
    let data = data_of(&[
        unit("PAD", "IsAirBase=1; WorkerTime=0.3; FootprintX=4; FootprintZ=4;"),
        unit("PLANE", "CanFly=1; CanMove=1; MaxVelocity=5; Weapon1=GUN; BuildTime=1000; MaxDamage=1;"),
        weapon_file("GUN", "range=100;"),
    ]);
    let mut sim = sim_of(data, 1);
    let pad = put(&mut sim, "PAD", 0, 300, true);
    let plane = put(&mut sim, "PLANE", 0, 300, true);
    sim.unit_mut(plane).unwrap().health = Fx::from_ratio(1, 2);
    sim.unit_mut(plane).unwrap().landed = true;
    sim.unit_mut(plane).unwrap().air_pad = Some((pad, None));
    sim.grid.rebuild(sim.units.iter().flatten());
    sim.mutators.build_speed = 2;
    let t = sim.types.by_name("PAD").unwrap().clone();
    let pt = sim.types.by_name("PLANE").unwrap().clone();
    for tick in 0..100 { sim.tick = tick * 5; sim.update_air_base(pad, &t); }
    let expected = Fx::from_ratio(1, 2) + pt.max_health.share(t.work_rate * 2 * 5 * 100, pt.build_work);
    assert_eq!(sim.unit(plane).unwrap().health, expected);
}

#[test]
fn work_beams_stop_while_approaching_after_a_target_leaves_range() {
    for action in 0..4 {
        let mut sim = yard("0.3");
        let builder = put(&mut sim, "BUILDER", 0, 200, true);
        let wreck = sim.place_feature("FRAME_DEAD", FxVec3::from_ints(280, 60, 300), 0).unwrap();
        // (Taking one of its own apart: a unit with metal to it, else it is gone at once.)
        let target = put(&mut sim, if action == 3 { "FRAME" } else { "BUILDER" }, if action == 2 { 1 } else { 0 }, 280, true);
        let order = match action { 0 => Order::Reclaim(wreck), 1 => Order::Resurrect(wreck), 2 => Order::Capture(target), _ => Order::ReclaimUnit(target) };
        sim.step(&[Command::Order { player: 0, units: vec![builder], order, queue: false }]);
        sim.step(&[]);
        assert!(sim.unit(builder).unwrap().in_build_stance, "job starts (action {action})");
        sim.unit_mut(builder).unwrap().pos.x = Fx::from_int(800);
        sim.step(&[]);
        let u = sim.unit(builder).unwrap();
        assert!(!u.in_build_stance && u.build.is_none(), "leaving range ends the old beam before approaching (action {action})");
    }
}

#[test]
fn cancelled_work_does_not_keep_walking_to_the_old_target() {
    for action in 0..5 {
        let mut sim = yard("30");
        sim.los_enabled = false;
        let builder = put(&mut sim, "BUILDER", 0, 100, true);
        let target = put(&mut sim, "BUILDER", if action == 4 { 1 } else { 0 }, 800, true);
        sim.unit_mut(target).unwrap().health = Fx::from_int(500);
        let wreck = sim.place_feature("FRAME_DEAD", FxVec3::from_ints(800, 60, 300), 0).unwrap();
        let order = match action { 0 => Order::Reclaim(wreck), 1 => Order::Resurrect(wreck), 2 => Order::ReclaimUnit(target), 3 => Order::Repair(target), _ => Order::Capture(target) };
        sim.step(&[Command::Order { player: 0, units: vec![builder], order, queue: false }]);
        assert!(sim.unit(builder).unwrap().path_goal.is_some(), "approach starts (action {action})");
        match action { 0 | 1 => sim.remove_feature(wreck, true), 3 => sim.unit_mut(target).unwrap().health = Fx::from_int(1000), _ => sim.remove_unit(target) }
        sim.step(&[]);
        let u = sim.unit(builder).unwrap();
        assert!(u.orders.is_empty() && u.path_goal.is_none(), "completed/invalid work must clear its old approach (action {action})");
    }
}

#[test]
fn construction_checksums_include_exact_work_and_feature_progress() {
    let mut sim = yard("300");
    let frame = put(&mut sim, "FRAME", 0, 280, false);
    let wreck = sim.place_feature("FRAME_DEAD", FxVec3::from_ints(400, 60, 300), 0).unwrap();
    let checksum = sim.checksum();
    for field in 0..5 {
        let mut changed = sim.clone();
        match field {
            0 => { let total = changed.types.by_name("FRAME").unwrap().build_work; changed.unit_mut(frame).unwrap().set_build_work(1, total); }
            1 => changed.features[wreck].reclaim_work = 1,
            2 => changed.features[wreck].resurrect_work = 1,
            3 => changed.unit_mut(frame).unwrap().finished = true,
            _ => changed.unit_mut(frame).unwrap().taken_apart_at = Some(1),
        }
        assert_ne!(changed.checksum(), checksum, "construction divergence must be visible in lockstep (field {field})");
    }
}

#[test]
fn construction_checksums_include_queue_contents_jobs_and_next_demand() {
    let mut sim = sim_of(construction_data(), 1);
    let factory = put(&mut sim, "FACTORY", 0, 300, true);
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let frame = put(&mut sim, "BUILDING", 0, 380, false);
    let other = put(&mut sim, "BUILDING", 0, 450, false);
    let tank = sim.types.id("TANK").unwrap();
    let alternate = sim.types.id("OTHER").unwrap();
    sim.unit_mut(factory).unwrap().factory_queue.push_back(tank);
    sim.unit_mut(factory).unwrap().rally.push(Order::Move(FxVec3::from_ints(600, 60, 300)));
    sim.give_orders(builder, [Order::Repair(frame)]);
    sim.unit_mut(builder).unwrap().build = Some(BuildJob { target: frame, started: true });
    let checksum = sim.checksum();
    for field in 0..7 {
        let mut changed = sim.clone();
        match field {
            0 => changed.unit_mut(factory).unwrap().factory_queue[0] = alternate,
            1 => changed.unit_mut(factory).unwrap().rally[0] = Order::Move(FxVec3::from_ints(700, 60, 300)),
            2 => changed.unit_mut(builder).unwrap().orders[0] = Order::Repair(other),
            3 => changed.unit_mut(builder).unwrap().build.as_mut().unwrap().target = other,
            4 => changed.unit_mut(builder).unwrap().build.as_mut().unwrap().started = false,
            5 => changed.unit_mut(frame).unwrap().repair_work = 1,
            _ => changed.players[0].economy.energy_demand_next += Fx::ONE,
        }
        assert_ne!(changed.checksum(), checksum, "same-length queues and future work state must be hashed (field {field})");
    }
}

#[test]
fn an_in_progress_construction_snapshot_continues_identically() {
    let mut sim = yard("30");
    sim.set_starting_resources(0, Fx::from_int(10000), Fx::from_int(10000));
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let helper = put(&mut sim, "BUILDER", 0, 360, true);
    let frame = put(&mut sim, "FRAME", 0, 280, false);
    let wreck = sim.place_feature("FRAME_DEAD", FxVec3::from_ints(440, 60, 300), 0).unwrap();
    sim.step(&[
        Command::Order { player: 0, units: vec![builder], order: Order::Repair(frame), queue: false },
        Command::Order { player: 0, units: vec![helper], order: Order::Resurrect(wreck), queue: false },
    ]);
    for _ in 0..12 { sim.step(&[]); }
    assert!(sim.unit(frame).unwrap().build_work > 0 && !sim.unit(frame).unwrap().is_complete());
    assert!(sim.features[wreck].resurrect_work > 0 && sim.features[wreck].alive);
    let mut restored = sim.clone();
    for _ in 0..60 {
        sim.step(&[]);
        restored.step(&[]);
        assert_eq!(sim.checksum(), restored.checksum());
    }
    assert!(sim.unit(frame).unwrap().is_complete());
    assert!(!sim.features[wreck].alive);
}

/// One of a side's constructors taking a building apart and another sent to repair it, as the
/// skirmish AI's extractor upgrades had it (TA: Enhanced's fast AI constructors on Coast To
/// Coast): `reclaimer` takes `target` apart, `helper` (far faster) is told to repair it once
/// the reclaim has begun.
fn taking_apart(sim: &mut Sim) -> (UnitId, UnitId, UnitId) {
    let target = put(sim, "TOWER", 0, 300, true);
    let reclaimer = put(sim, "RECLAIMER", 0, 200, true);
    let helper = put(sim, "HELPER", 0, 400, true);
    sim.take_events();
    (target, reclaimer, helper)
}

fn reclaim_data() -> GameData {
    data_of(&[
        unit("RECLAIMER", "Builder=1; WorkerTime=100; CanMove=1; MaxVelocity=1; BuildDistance=128; CanReclamate=1; FootprintX=1; FootprintZ=1;"),
        unit("HELPER", "Builder=1; WorkerTime=1000; CanMove=1; MaxVelocity=1; BuildDistance=128; FootprintX=1; FootprintZ=1;"),
        // (Switched on and set to hold fire as it is finished: TA's extractors, Big Bertha.)
        unit("TOWER", "BuildTime=600; BuildCostMetal=60; BuildCostEnergy=600; FootprintX=2; FootprintZ=2; OnOffable=1; ActivateWhenBuilt=1; Weapon1=SHELL; NoAutoFire=1;"),
        ("weapons/shell.tdf".into(), "[SHELL] { range=500; reloadtime=1; damage { default=10; } }".into()),
    ])
}

fn completions(sim: &mut Sim, of: UnitId) -> usize {
    sim.take_events().iter().filter(|e| matches!(e, SimEvent::UnitCompleted { id, .. } if *id == of)).count()
}

/// A unit is finished once: work on one its own side is taking apart mends it and no more, as
/// TA's repair does, and the reclaim goes on to the end, its metal back and nothing paid. (It
/// was built back up and finished again every tick, and taken apart again the next, for ever:
/// thousands of `UnitCompleted`, each counted as built and switched on again.)
#[test]
fn a_unit_taken_apart_while_another_builder_repairs_it_is_finished_once() {
    let mut sim = sim_of(reclaim_data(), 1);
    sim.set_starting_resources(0, Fx::from_int(500), Fx::from_int(5000));
    let (target, reclaimer, helper) = taking_apart(&mut sim);
    let (metal, energy) = (sim.players[0].economy.metal, sim.players[0].economy.energy);
    let tt = sim.types.get(sim.unit(target).unwrap().type_id).clone();
    let cost = tt.cost_metal.share(tt.build_work, tt.build_work);
    sim.step(&[Command::Order { player: 0, units: vec![reclaimer], order: Order::ReclaimUnit(target), queue: false }]);
    let mut finished = completions(&mut sim, target);
    let mut ticks = 0;
    while sim.unit(target).is_some_and(|u| u.is_complete()) && ticks < 120 {
        sim.step(&[]);
        finished += completions(&mut sim, target);
        ticks += 1;
    }
    assert!(sim.unit(target).is_some_and(|u| !u.is_complete() && sim.being_taken_apart(u)), "the reclaim has begun");
    // Work put into it (as a guard's or a patrol's help comes): mended, not built back up.
    let (work, ht) = (sim.unit(target).unwrap().build_work, sim.types.get(sim.unit(helper).unwrap().type_id).clone());
    sim.apply_build_power(helper, &ht, target);
    assert_eq!(sim.unit(target).unwrap().build_work, work, "not built back up");
    sim.step(&[Command::Order { player: 0, units: vec![helper], order: Order::Repair(target), queue: false }]);
    finished += completions(&mut sim, target);
    for _ in 0..600 {
        if sim.unit(target).is_none() {
            break;
        }
        sim.step(&[]);
        finished += completions(&mut sim, target);
    }
    assert_eq!(finished, 0, "finished once, as it was put down: never again");
    assert!(sim.unit(target).is_none(), "taken apart to the end, not undone by the repair");
    assert_eq!(sim.players[0].stats.built, 0, "nothing counted as built");
    let e = &sim.players[0].economy;
    assert_eq!((e.metal, e.energy), (metal + cost, energy), "its metal back, the repair free");
}

/// One left part taken apart (its reclaimer stopped) is built back up at a build's price, and
/// is whole again without being finished a second time: no `UnitCompleted`, and not switched
/// back on nor set to hold fire again (the player switched it off and set it to fire at will).
#[test]
fn a_unit_left_part_taken_apart_is_built_back_up_without_being_finished_again() {
    let mut sim = sim_of(reclaim_data(), 1);
    sim.set_starting_resources(0, Fx::from_int(500), Fx::from_int(5000));
    let (target, reclaimer, helper) = taking_apart(&mut sim);
    assert!(sim.unit(target).is_some_and(|u| u.finished && u.activated && u.fire_state == 0), "switched on and holding fire as finished");
    sim.step(&[
        Command::SetActive { player: 0, units: vec![target], on: false },
        Command::SetStates { player: 0, units: vec![target], fire: Some(2), movement: None },
        Command::Order { player: 0, units: vec![reclaimer], order: Order::ReclaimUnit(target), queue: false },
    ]);
    let total = sim.types.get(sim.unit(target).unwrap().type_id).build_work;
    for _ in 0..300 {
        if sim.unit(target).is_some_and(|u| u.build_work <= total / 2) {
            break;
        }
        sim.step(&[]);
    }
    assert!(sim.unit(target).is_some_and(|u| u.build_work <= total / 2), "half taken apart");
    sim.step(&[Command::Order { player: 0, units: vec![reclaimer], order: Order::Stop, queue: false }]);
    for _ in 0..TICKS_PER_SECOND {
        sim.step(&[]);
    }
    assert!(sim.unit(target).is_some_and(|u| !u.is_complete() && !sim.being_taken_apart(u)), "left part taken apart");
    assert_eq!(completions(&mut sim, target), 0);
    let (metal, energy) = (sim.players[0].economy.metal, sim.players[0].economy.energy);
    let left = sim.unit(target).unwrap().build_work;
    let tt = sim.types.get(sim.unit(target).unwrap().type_id).clone();
    sim.step(&[Command::Order { player: 0, units: vec![helper], order: Order::Repair(target), queue: false }]);
    let mut finished = completions(&mut sim, target);
    for _ in 0..600 {
        if sim.unit(target).is_some_and(|u| u.is_complete()) {
            break;
        }
        sim.step(&[]);
        finished += completions(&mut sim, target);
    }
    let u = sim.unit(target).unwrap();
    assert!(u.is_complete(), "built back up");
    assert_eq!(finished, 0, "whole again, not finished again");
    assert_eq!((u.activated, u.fire_state), (false, 2), "as the player left it");
    assert_eq!(sim.players[0].stats.built, 0);
    let e = &sim.players[0].economy;
    let price = |of: Fx| of.share(total, total) - of.share(left, total);
    assert_eq!(e.metal, metal - price(tt.cost_metal), "the metal its reclaim gave back paid again");
    assert_eq!(e.energy, energy - price(tt.cost_energy), "and the energy, as a build pays it");
}

/// A patrolling constructor passes over a building its side is taking apart (work on it would
/// only mend what is left), and helps with one left part taken apart.
#[test]
fn patrolling_builders_leave_a_building_being_taken_apart_alone() {
    let mut sim = sim_of(reclaim_data(), 1);
    sim.set_starting_resources(0, Fx::from_int(500), Fx::from_int(5000));
    let (target, reclaimer, helper) = taking_apart(&mut sim);
    sim.step(&[Command::Order { player: 0, units: vec![reclaimer], order: Order::ReclaimUnit(target), queue: false }]);
    for _ in 0..60 {
        sim.step(&[]);
    }
    assert!(sim.unit(target).is_some_and(|u| sim.being_taken_apart(u)));
    let t = sim.types.get(sim.unit(helper).unwrap().type_id).clone();
    assert_eq!(sim.patrol_work(helper, &t), None, "nothing to do for it");
    sim.step(&[Command::Order { player: 0, units: vec![reclaimer], order: Order::Stop, queue: false }]);
    for _ in 0..TICKS_PER_SECOND {
        sim.step(&[]);
    }
    assert_eq!(sim.patrol_work(helper, &t), Some(Order::Repair(target)), "built back up once left");
}

/// Construction left with nobody at it un-builds after a while (ten seconds), its metal coming
/// back as the work goes (its energy does not), and is gone when nothing of it is left: a whole
/// frame in a hundred seconds, one half built in fifty.
#[test]
fn abandoned_construction_un_builds_and_gives_its_metal_back() {
    let mut sim = yard("300");
    sim.set_starting_resources(0, Fx::from_int(10000), Fx::from_int(100000));
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let pos = FxVec3::from_ints(280, 60, 300);
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Build { type_id: sim.types.id("FRAME").unwrap(), pos }, queue: false }]);
    // Half built (BuildTime 30, WorkerTime 300: the whole of it in three ticks of work, so a few
    // ticks for the builder to get going and one of work), then left.
    let frame = (0..120).find_map(|_| {
        sim.step(&[]);
        sim.units.iter().flatten().find(|u| u.type_id == sim.types.id("FRAME").unwrap() && u.build_work > 0).map(|u| u.id)
    }).expect("the builder gets to work");
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Stop, queue: false }]);
    let (work, total) = { let u = sim.unit(frame).unwrap(); (u.build_work, sim.unit_type(u).build_work) };
    assert!(work > 0 && work < total, "part built: {work} of {total}");
    let metal_left = sim.players[0].economy.metal;
    let spent = sim.unit_type(sim.unit(frame).unwrap()).cost_metal.share(work, total);
    let energy = sim.players[0].economy.energy;
    for _ in 0..(super::FRAME_GRACE - 2) { sim.step(&[]); }
    assert_eq!(sim.unit(frame).unwrap().build_work, work, "it stands as it was for a while");
    let mut gone_at = None;
    for t in 0..(super::FRAME_DECAY + 60) {
        sim.step(&[]);
        if sim.unit(frame).is_none() {
            gone_at = Some(t);
            break;
        }
    }
    let gone_at = gone_at.expect("and then it un-builds and is gone");
    let expected = super::FRAME_DECAY * work / total;
    assert!(gone_at.abs_diff(expected) <= 8, "in the time its share of a whole frame's takes: {gone_at} ticks, {expected} expected");
    let back = sim.players[0].economy.metal - metal_left;
    assert!((back - spent).abs() <= Fx::from_ratio(1, 100), "the metal it took comes back: {back:?} of {spent:?}");
    assert!(sim.players[0].economy.energy >= energy, "its energy does not");
    assert!(sim.features.iter().all(|f| !f.alive), "no wreck");
}

/// A frame set down and left before any work went into it goes once the while is up; one a scene
/// or a mission placed unfinished, which nobody worked on, stays; one a builder comes back to
/// stops un-building.
#[test]
fn frames_set_down_and_left_go_while_placed_ones_and_tended_ones_stay() {
    let mut sim = yard("300");
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let placed = put(&mut sim, "FRAME", 0, 600, false);
    let pos = FxVec3::from_ints(280, 60, 300);
    let frame_type = sim.types.id("FRAME").unwrap();
    // No metal: the builder sets the frame down but cannot put work into it.
    sim.players[0].economy.metal = Fx::ZERO;
    sim.players[0].economy.metal_capacity = Fx::ZERO;
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Build { type_id: frame_type, pos }, queue: false }]);
    let frame = (0..120).find_map(|_| {
        sim.step(&[]);
        sim.units.iter().flatten().find(|u| u.type_id == frame_type && u.id != placed).map(|u| u.id)
    }).expect("the builder sets it down");
    for _ in 0..120 { sim.step(&[]); }
    assert!(sim.unit(frame).is_some() && sim.unit(frame).unwrap().build_work == 0, "tended while the builder waits for metal, at nothing");
    sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::Stop, queue: false }]);
    for _ in 0..(super::FRAME_GRACE + 2) { sim.step(&[]); }
    assert!(sim.unit(frame).is_none(), "left at nothing: gone once the while is up");
    for _ in 0..(super::FRAME_DECAY + 60) { sim.step(&[]); }
    assert!(sim.unit(placed).is_some(), "placed unfinished and never worked on: it stays");
}

/// Taking one of its own units apart goes at TA's pace, by the unit's metal as a wreck's: a metal
/// maker, which costs no metal, at once; a unit of 100 metal in twenty times that in worker time
/// (a 300-worker builder: about seven seconds).
#[test]
fn reclaiming_a_unit_takes_as_long_as_its_metal_says() {
    let data = data_of(&[
        unit("BUILDER", "Builder=1; WorkerTime=300; CanMove=1; MaxVelocity=1; BuildDistance=128; CanReclamate=1; FootprintX=1; FootprintZ=1;"),
        unit("MAKER", "BuildTime=2605; BuildCostMetal=0; BuildCostEnergy=687; FootprintX=2; FootprintZ=2;"),
        unit("SOLAR", "BuildTime=2495; BuildCostMetal=100; BuildCostEnergy=760; FootprintX=2; FootprintZ=2;"),
    ]);
    let mut sim = sim_of(with_menu(data, "BUILDER", &["MAKER", "SOLAR"]), 2);
    for player in &mut sim.players { player.endless = true; }
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let maker = put(&mut sim, "MAKER", 0, 280, true);
    let solar = sim.spawn_unit(sim.types.id("SOLAR").unwrap(), 0, FxVec3::from_ints(200, 60, 390), 0, true).unwrap();
    let ticks = |sim: &mut Sim, target: UnitId| {
        sim.step(&[Command::Order { player: 0, units: vec![builder], order: Order::ReclaimUnit(target), queue: false }]);
        let mut working = None;
        for t in 0..900u64 {
            sim.step(&[]);
            if working.is_none() && sim.unit(target).is_some_and(|u| !u.is_complete()) {
                working = Some(t);
            }
            if sim.unit(target).is_none() {
                return t - working.unwrap_or(t);
            }
        }
        panic!("never taken apart");
    };
    assert!(ticks(&mut sim, maker) <= 1, "a metal maker costs no metal: gone at once");
    let solar_ticks = ticks(&mut sim, solar);
    let expected = 100 * 20 * TICKS_PER_SECOND as u64 / 300;
    assert!(solar_ticks.abs_diff(expected) <= 4, "100 metal: twenty times that in worker time: {solar_ticks} ticks, {expected} expected");
}

/// A builder's script as Total Mayhem's advanced construction kbots have it (`ARMACK.bos`): the
/// nano arm goes out as a job starts, raising the build stance, unless it is out already; after a
/// job it stays out five seconds and comes in, dropping the stance, only if no new job came.
fn arm_stays_out_script() -> crate::cob::CobProgram {
    use crate::host::value::IN_BUILD_STANCE;
    use ta_formats::cob::{CobFunction, CobScript, op};
    // (Statics: 0 the arm out, 1 a job asked for.)
    let mut s = CobScript { static_var_count: 2, ..Default::default() };
    let stance = |code: &mut Vec<u32>, to: u32| code.extend([op::PUSH_CONSTANT, IN_BUILD_STANCE as u32, op::PUSH_CONSTANT, to, op::SET_VALUE]);
    let ret = |code: &mut Vec<u32>| code.extend([op::PUSH_CONSTANT, 0, op::RETURN]);
    s.functions.push(CobFunction { name: "StartBuilding".into(), offset: s.code.len() as u32 });
    s.code.extend([op::PUSH_CONSTANT, 1, op::POP_STATIC, 1, op::PUSH_STATIC, 0, op::JUMP_IF_FALSE, 0]);
    let out = s.code.len() - 1;
    ret(&mut s.code);
    s.code[out] = s.code.len() as u32;
    stance(&mut s.code, 1);
    s.code.extend([op::PUSH_CONSTANT, 1, op::POP_STATIC, 0]);
    ret(&mut s.code);
    s.functions.push(CobFunction { name: "StopBuilding".into(), offset: s.code.len() as u32 });
    s.code.extend([op::PUSH_CONSTANT, 0, op::POP_STATIC, 1, op::PUSH_CONSTANT, 5000, op::SLEEP, op::PUSH_STATIC, 1, op::JUMP_IF_FALSE, 0]);
    let fold = s.code.len() - 1;
    ret(&mut s.code);
    s.code[fold] = s.code.len() as u32;
    stance(&mut s.code, 0);
    s.code.extend([op::PUSH_CONSTANT, 0, op::POP_STATIC, 0]);
    ret(&mut s.code);
    crate::cob::CobProgram::new(s, &[])
}

/// The build stance is the builder's script's, as in TA: a building ordered while the arm of a
/// Total Mayhem construction kbot is still out from the last job is built at once, its stance as
/// the script left it. (The game dropped the stance as each job ended; the script, its arm still
/// out, did not raise it again, and the kbot stood at the next building until its arm came in.)
#[test]
fn a_builder_whose_arm_is_still_out_builds_the_next_job_at_once() {
    let mut sim = yard("300");
    sim.set_starting_resources(0, Fx::from_int(10000), Fx::from_int(100000));
    let kind = sim.types.id("BUILDER").unwrap();
    std::sync::Arc::get_mut(&mut sim.types).unwrap().types[kind].script = Some(std::sync::Arc::new(arm_stays_out_script()));
    let builder = put(&mut sim, "BUILDER", 0, 200, true);
    let frame = sim.types.id("FRAME").unwrap();
    let order = |pos: FxVec3| Command::Order { player: 0, units: vec![builder], order: Order::Build { type_id: frame, pos }, queue: false };
    let finished = |sim: &Sim| sim.units().filter(|u| u.type_id == frame && u.is_complete()).count();
    sim.step(&[order(FxVec3::from_ints(280, 60, 300))]);
    assert!((0..300).any(|_| {
        sim.step(&[]);
        finished(&sim) == 1
    }), "the first is built");
    // At once, the arm still out (it stays out five seconds).
    sim.step(&[order(FxVec3::from_ints(280, 60, 360))]);
    let at = (0..60).find(|_| {
        sim.step(&[]);
        sim.units().any(|u| u.type_id == frame && !u.is_complete() && u.build_work > 0)
    });
    assert!(at.is_some(), "the next is built at once, without waiting for the arm to come in and go out again");
    assert!((0..300).any(|_| {
        sim.step(&[]);
        finished(&sim) == 2
    }), "and finished");
    // Five seconds after its last job, with none since, the arm comes in and the stance drops.
    assert!(sim.unit(builder).unwrap().in_build_stance, "the stance stays raised while the arm is out");
    for _ in 0..(6 * TICKS_PER_SECOND) {
        sim.step(&[]);
    }
    assert!(!sim.unit(builder).unwrap().in_build_stance, "and drops as the script takes the arm in");
}
