//! Quality assurance of the duties, on real game data (skipped without a TA install): the trust a
//! player puts in a constructor on duty. It never wanders into the enemy's base or range, never
//! stalls the economy by spending everything, never fights other units over a job and never
//! loops. Each test pins down something found by hand with `duty_explore.rs`.

mod common;

use common::*;
use ta_ai::{Duty, DutyEvent};
use ta_sim::{Command, Fx, FxVec3, Order, Target, UnitId};

/// How often a unit was told to reclaim a feature.
fn reclaim_orders(a: &Audit, feature: usize) -> usize {
    a.ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Reclaim(f), .. } if *f == feature)).count()
}

/// The first place a unit was told to move to, if it was.
fn first_move(a: &Audit, who: UnitId) -> Option<FxVec3> {
    a.ordered.iter().find_map(|(_, c)| match c {
        Command::Order { units, order: Order::Move(p), .. } if units.contains(&who) => Some(*p),
        _ => None,
    })
}

// ---- Looping --------------------------------------------------------------------------------------

/// A constructor cut off from home (another island, a walled-in plateau) with nothing to do was
/// told to go home every half second, for ever: a path search each time.
#[test]
fn a_constructor_that_cannot_get_home_is_not_sent_every_beat() {
    for duty in Duty::ALL {
        let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
        let Some(at) = cut_off_spot(&scene, "ARMCK") else { return };
        let id = scene.spawn_at("ARMCK", 0, at);
        scene.officer.set(id, Some(duty));
        let a = audit(&mut scene, 3 * 60, &[]);
        assert!(a.ordered.len() <= 3, "{}: {} orders in three minutes, {:?}", duty.name(), a.ordered.len(), a.spam.first());
        assert!(a.spam.is_empty(), "{}: {:?}", duty.name(), a.spam);
    }
}

/// A wreck the simulation drops the order for when the builder has walked a long way and stopped
/// short of it: remembered as out of reach, so the trip is not made again and again.
#[test]
fn a_wreck_a_builder_cannot_get_at_is_not_made_for_again_and_again() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    scene.sim.clear_features_where(|_, _| true);
    scene.sim.players[0].economy.metal = Fx::from_int(50);
    let toward = (scene.foe - scene.start).xz().normalize();
    let wrecks: Vec<usize> = [500, 900].iter().map(|&d| scene.sim.place_feature("ARMFLASH_DEAD", scene.start + toward.scale(Fx::from_int(d)), 0).expect("the wreck")).collect();
    serve(&mut scene, "ARMCK", Duty::Reclaim, 0);
    let a = audit(&mut scene, 10 * 60, &[]);
    for &f in &wrecks {
        let n = reclaim_orders(&a, f);
        let reclaimed = !scene.sim.features[f].alive;
        println!("wreck {f}: {n} orders, reclaimed {reclaimed}");
        assert!(n <= 2, "wreck {f} was made for {n} times in ten minutes (reclaimed: {reclaimed})");
    }
    assert!(a.ordered.len() <= 8, "{} orders in ten minutes", a.ordered.len());
}

// ---- Keeping near home and out of harm's way -------------------------------------------------------

/// The Commander on the reclaim duty stays within its reach of home, wrecks further out or not
/// (it walked 1,300 px out to one).
#[test]
fn the_commander_on_reclaim_duty_stays_within_its_reach_of_home() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    scene.sim.clear_features_where(|_, _| true);
    scene.sim.players[0].economy.metal = Fx::from_int(50);
    let toward = (scene.foe - scene.start).xz().normalize();
    let near = scene.sim.place_feature("ARMFLASH_DEAD", scene.start + FxVec3::from_ints(-120, 0, 90), 0).expect("the wreck");
    // (A Flash's wreck is one of those the duty makes for when metal is short; a lighter wreck is only scrap.)
    for (name, d) in [("ARMFLASH_DEAD", 900), ("ARMFLASH_DEAD", 1300), ("CORAK_DEAD", 1000), ("CORAK_DEAD", 1500)] {
        scene.sim.place_feature(name, scene.start + toward.scale(Fx::from_int(d)), 0).expect("the wreck");
    }
    // (The whole map in sight: the empty starts are known to be empty, so only the real enemy's ground is out of bounds.)
    scene.sim.los_enabled = false;
    let com = scene.commander();
    scene.officer.set(com, Some(Duty::Reclaim));
    let (start, mut farthest) = (scene.start, 0);
    scene.play_watched(6 * 60, &[], |sim, _| farthest = farthest.max(px(sim.unit(com).unwrap().pos, start)));
    println!("the Commander went {farthest} px from home at most; the near wreck is gone: {}", !scene.sim.features[near].alive);
    assert!(farthest <= 700 + 130, "went {farthest} px from home");
    assert!(!scene.sim.features[near].alive, "the wreck beside the base was reclaimed");
}

/// Armed buildings seen are kept clear of when they are out of sight again: a builder that was
/// driven off by a tower went back to the deposit by it every few minutes and was shot again.
#[test]
fn a_tower_seen_and_then_out_of_sight_is_kept_clear_of() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let target = deposits_near(&scene, 1500)[3];
    let tower_at = target + FxVec3::from_ints(40, 0, 40);
    scene.spawn_at("CORLLT", 1, tower_at);
    // A scout of ours has a look at it, and goes.
    let scout = scene.spawn_at("ARMFLEA", 0, tower_at + FxVec3::from_ints(200, 0, 0));
    serve(&mut scene, "ARMCK", Duty::Economy, 0);
    scene.play(4);
    scene.sim.remove_unit(scout);
    let a = audit(&mut scene, 8 * 60, &[]);
    let at_the_tower = a.ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Build { pos, .. }, .. } if px(*pos, tower_at) < 250)).count();
    println!("{} orders; {at_the_tower} to build by the tower; under guns {}", a.ordered.len(), a.under_guns.len());
    assert_eq!(at_the_tower, 0, "sent to build by a tower it had seen");
    assert!(a.under_guns.is_empty(), "{:?}", a.under_guns);
}

/// With a tower known, no job takes the builder across its range (it crossed on the way from one
/// site to another, two to seven seconds under fire each time).
#[test]
fn a_builder_does_not_cross_the_range_of_a_tower_it_knows() {
    // One wreck lies 1,000 px east of the start, the only thing to reclaim; with a tower off the
    // straight way to it, 150 px from the way and 600 from the wreck itself, it is only worth the trip
    // by a way round, and there is none to the unit but the straight one.
    let reclaim = |tower: bool| {
        let mut scene = Scene::on("sherwood", ["ARM", "CORE"], true)?;
        scene.sim.clear_features_where(|_, _| true);
        scene.sim.players[0].economy.metal = Fx::from_int(50);
        let wreck_at = scene.start + FxVec3::from_ints(1000, 0, 0);
        let wreck = scene.sim.place_feature("ARMFLASH_DEAD", wreck_at, 0)?;
        let tower_at = scene.start + FxVec3::from_ints(400, 0, 150);
        if tower {
            scene.spawn_at("CORLLT", 1, tower_at);
        }
        // The whole map in sight: the tower, and that the starts but ours are empty, are known.
        scene.sim.los_enabled = false;
        let builder = serve(&mut scene, "ARMCK", Duty::Reclaim, 0);
        let a = audit(&mut scene, 4 * 60, &[]);
        let ordered = a.ordered.iter().any(|(_, c)| matches!(c, Command::Order { order: Order::Reclaim(f), .. } if *f == wreck));
        println!("tower {tower}: reclaim ordered {ordered}, wreck gone {}, under guns {}, {} orders", !scene.sim.features[wreck].alive, a.under_guns.len(), a.ordered.len());
        let _ = builder;
        Some((ordered, a))
    };
    let Some((ordered, _)) = reclaim(false) else { return };
    assert!(ordered, "without the tower the wreck is worth the trip");
    let (ordered, a) = reclaim(true).unwrap();
    assert!(a.under_guns.is_empty(), "{:?}", a.under_guns);
    assert!(!ordered, "set out across the tower's reach");
}

/// Armed enemies walking up to a builder make it get away, in the base too (it went on building
/// until it died).
#[test]
fn a_builder_gets_away_from_armed_enemies_close_by() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let builder = serve(&mut scene, "ARMCK", Duty::Economy, 0);
    scene.play(40);
    let at = scene.sim.unit(builder).unwrap().pos;
    let squad: Vec<UnitId> = (0..6).map(|k| scene.spawn_at("CORAK", 1, at + FxVec3::from_ints(300 + 20 * k, 0, 0))).collect();
    let attack = Command::Order { player: 1, units: squad.clone(), order: Order::Attack(Target::Unit(builder)), queue: false };
    let a = audit(&mut scene, 3, &[attack]);
    let away = first_move(&a, builder).expect("told to get away");
    let squad_at = scene.sim.unit(squad[0]).unwrap().pos;
    println!("builder at ({}, {}), told to move to ({}, {}), the squad at ({}, {})", at.x.to_int(), at.z.to_int(), away.x.to_int(), away.z.to_int(), squad_at.x.to_int(), squad_at.z.to_int());
    assert!(px(away, squad_at) > px(at, squad_at), "away from them, not towards");
    // It holds back while they are about, and does not take up its building again at once.
    let b = audit(&mut scene, 3, &[]);
    assert!(!b.ordered.iter().any(|(_, c)| matches!(c, Command::Order { order: Order::Build { .. }, .. })), "building under their guns");
}

/// A builder driven off a job by enemies that stay put does not take it up again as soon as it has held
/// back (it would be driven off again, and so for ever, at the edge of their guns).
#[test]
fn a_builder_driven_off_a_job_does_not_go_back_to_it_at_once() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let builder = serve(&mut scene, "ARMCK", Duty::Economy, 0);
    let a = audit(&mut scene, 3, &[]);
    let site = a.ordered.iter().find_map(|(_, c)| match c {
        Command::Order { order: Order::Build { pos, .. }, .. } => Some(*pos),
        _ => None,
    }).expect("it set out to build");
    // Six kbots of the enemy's stand 300 px from the site, holding their position and their fire.
    let squad: Vec<UnitId> = (0..6).map(|k| scene.spawn_at("CORAK", 1, site + FxVec3::from_ints(300, 0, 20 * k - 50))).collect();
    scene.sim.step(&[Command::SetStates { player: 1, units: squad, fire: Some(0), movement: Some(0) }]);
    let b = audit(&mut scene, 50, &[]);
    let again = b.ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Build { pos, .. }, units, .. } if px(*pos, site) < 40 && units.contains(&builder))).count();
    let moves = b.ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Move(_), .. })).count();
    println!("after the squad came: {again} orders for the site again, {moves} moves");
    assert_eq!(again, 0, "back at the job within fifty seconds");
    assert!(moves >= 1 && moves <= 4, "it got away ({moves} moves)");
}

/// The Commander is handed back to the player when armed enemies come close, before it is hit.
#[test]
fn the_commander_is_handed_back_when_armed_enemies_come_close() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let com = scene.commander();
    scene.officer.set(com, Some(Duty::Economy));
    scene.play(20);
    assert!(scene.officer.take_events().is_empty());
    let at = scene.sim.unit(com).unwrap().pos;
    let squad: Vec<UnitId> = (0..4).map(|k| scene.spawn_at("CORAK", 1, at + FxVec3::from_ints(220 + 15 * k, 0, 0))).collect();
    let attack = scene.attack(&squad, com);
    let a = audit(&mut scene, 2, &[attack]);
    assert_eq!(scene.officer.duty(com), None, "back under the player's command");
    assert_eq!(scene.officer.take_events(), vec![DutyEvent::Danger(com)]);
    let b = audit(&mut scene, 10, &[]);
    assert!(a.ordered.iter().chain(b.ordered.iter()).all(|(_, c)| !matches!(c, Command::Order { units, .. } if units.contains(&com))) || a.ordered.len() <= 1, "no more orders for it: {:?}", b.ordered);
    assert!(b.ordered.is_empty(), "{:?}", b.ordered);
}

// ---- Working together ------------------------------------------------------------------------------

/// Ten constructors on four duties, with hurt buildings and wrecks about: no two sent at one job
/// (two menders went to the same solar collector, two reclaimers to the same wreck).
#[test]
fn constructors_on_duty_are_not_sent_at_one_job() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    scene.sim.players[0].economy.metal = Fx::from_int(200);
    let lab = scene.spawn("ARMLAB", 0, 140);
    let kbot = scene.type_id("ARMPW");
    scene.sim.step(&[Command::Produce { player: 0, factory: lab, type_id: kbot, count: 100 }]);
    for k in 0..4 {
        let s = scene.spawn("ARMSOLAR", -140 - 70 * k, 150);
        let max = scene.sim.unit_type(scene.sim.unit(s).unwrap()).max_health;
        scene.sim.set_health(s, max / 3);
    }
    for k in 0..6 {
        let at = scene.start + FxVec3::from_ints(150 + 60 * k, 0, -120);
        scene.sim.place_feature(if k % 2 == 0 { "ARMFLASH_DEAD" } else { "CORAK_DEAD" }, at, 0);
    }
    scene.spawn_frame("ARMSOLAR", scene.start + FxVec3::from_ints(-400, 0, 300));
    scene.spawn_frame("ARMMEX", scene.start + FxVec3::from_ints(-300, 0, 400));
    for (k, duty) in [Duty::Economy, Duty::Economy, Duty::Economy, Duty::Economy, Duty::Repair, Duty::Repair, Duty::Reclaim, Duty::Reclaim, Duty::Reclaim, Duty::Defence].into_iter().enumerate() {
        serve(&mut scene, "ARMCK", duty, k as i64);
    }
    let a = audit(&mut scene, 6 * 60, &[]);
    a.report("ten constructors on four duties");
    assert!(a.conflicts.is_empty(), "{} samples of two at one job: {:?}", a.conflicts.len(), a.conflicts.iter().take(3).collect::<Vec<_>>());
    assert!(a.ordered.len() > 20, "and they worked");
}

/// The repair duty does not walk up to a hurt building with armed enemies at it, and mends it once
/// they are gone.
#[test]
fn the_repair_duty_waits_while_the_enemy_is_at_the_hurt_building() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let solar = scene.spawn("ARMSOLAR", 200, 60);
    let max = scene.sim.unit_type(scene.sim.unit(solar).unwrap()).max_health;
    scene.sim.set_health(solar, max / 3);
    let at = scene.sim.unit(solar).unwrap().pos;
    // Three kbots of the enemy's stand by it, their fire held (so that they do not finish it).
    let squad: Vec<UnitId> = (0..3).map(|k| scene.spawn_at("CORAK", 1, at + FxVec3::from_ints(120, 0, 30 * k))).collect();
    scene.sim.step(&[Command::SetStates { player: 1, units: squad.clone(), fire: Some(0), movement: Some(0) }]);
    scene.sim.los_enabled = false;
    // (Beside it: a building may stand where no constructor can walk up to it.)
    let builder = scene.spawn_at("ARMCK", 0, at + FxVec3::from_ints(-30, 0, 70));
    scene.officer.set(builder, Some(Duty::Repair));
    let a = audit(&mut scene, 30, &[]);
    let repairs = |a: &Audit| a.ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Repair(id), .. } if *id == solar)).count();
    println!("with the enemy there: {} repair orders; health {}", repairs(&a), scene.sim.unit(solar).unwrap().health.to_int());
    assert_eq!(repairs(&a), 0, "sent to mend with armed enemies at the building");
    for s in squad {
        scene.sim.remove_unit(s);
    }
    let hurt = scene.sim.unit(solar).unwrap().health;
    let b = audit(&mut scene, 90, &[]);
    println!("with the enemy gone: {} repair orders; health {}", repairs(&b), scene.sim.unit(solar).unwrap().health.to_int());
    assert!(repairs(&b) > 0 && scene.sim.unit(solar).unwrap().health > hurt, "and mended it when they had gone");
}

// ---- The player's own orders and changes of duty ----------------------------------------------------

/// A unit with orders of the player's (queued, say) finishes them before its duty starts: a duty
/// begins when the unit is free.
#[test]
fn a_unit_with_orders_finishes_them_before_its_duty_starts() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let builder = scene.spawn("ARMCK", 60, 60);
    let hurt = scene.spawn("ARMSOLAR", 180, 40);
    let max = scene.sim.unit_type(scene.sim.unit(hurt).unwrap()).max_health;
    scene.sim.set_health(hurt, max / 3);
    // The player sent it on a walk of two legs, then put it on the repair duty.
    let (a, b) = (scene.start + FxVec3::from_ints(300, 0, 250), scene.start + FxVec3::from_ints(-100, 0, 300));
    scene.sim.step(&[
        Command::Order { player: 0, units: vec![builder], order: Order::Move(a), queue: false },
        Command::Order { player: 0, units: vec![builder], order: Order::Move(b), queue: true },
    ]);
    scene.officer.set(builder, Some(Duty::Repair));
    let mut first_order = None;
    let mut front_during = Vec::new();
    scene.play_watched(40, &[], |sim, commands| {
        if first_order.is_none() && !commands.is_empty() {
            first_order = Some(sim.tick);
            front_during.push(sim.unit(builder).unwrap().orders.len());
        }
    });
    let at = first_order.expect("it was sent to mend in the end");
    println!("the officer's first order came at tick {at}, the unit had {:?} orders left", front_during);
    assert!(at > 60, "the walk was not cut short: the officer waited ({at})");
    assert_eq!(front_during, vec![0], "it spoke only when the unit was free");
}

/// Changing a unit's duty does not drag it off the job it is at: the new duty takes it when it is
/// done, and the old duty's job is not given again.
#[test]
fn changing_a_duty_lets_the_unit_finish_what_it_is_at() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let builder = scene.spawn("ARMCK", 60, 60);
    scene.officer.set(builder, Some(Duty::Economy));
    scene.play(3);
    let building = scene.sim.unit(builder).unwrap().orders.front().cloned();
    assert!(matches!(building, Some(Order::Build { .. })), "{building:?}");
    // Wrecks near by for the reclaim duty; the player changes the duty at once.
    scene.sim.place_feature("ARMFLASH_DEAD", scene.start + FxVec3::from_ints(120, 0, 100), 0);
    scene.officer.set(builder, Some(Duty::Reclaim));
    let ordered = scene.play(60);
    let builds_then = ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Build { .. }, .. })).count();
    let reclaims = ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Reclaim(_), .. })).count();
    println!("{builds_then} build orders and {reclaims} reclaim orders after the change");
    assert_eq!(builds_then, 0, "the old duty gave no more jobs");
    assert!(reclaims > 0, "the new duty took it when it was free");
    assert!(ordered.iter().all(|(t, _)| *t > 0), "nothing at once: it was at its job");
    assert_eq!(scene.officer.duty(builder), Some(Duty::Reclaim));
}

// ---- Not stalling the economy ----------------------------------------------------------------------

/// A lab turning out kbots on small stores makes at least as many with constructors on any duty as
/// without (a constructor taking its share of the metal and power must pay for it, not stall the
/// factory); the economy duty makes more.
#[test]
fn constructors_on_duty_do_not_slow_a_busy_lab() {
    let made = |duties: &[Duty], metal: i64| {
        let mut scene = Scene::on("sherwood", ["ARM", "CORE"], true)?;
        scene.sim.players[0].economy.metal = Fx::from_int(metal);
        scene.sim.players[0].economy.energy = Fx::from_int(metal);
        let lab = scene.spawn("ARMLAB", 0, 140);
        let kbot = scene.type_id("ARMPW");
        scene.sim.step(&[Command::Produce { player: 0, factory: lab, type_id: kbot, count: 200 }]);
        for (k, &duty) in duties.iter().enumerate() {
            serve(&mut scene, "ARMCK", duty, k as i64);
        }
        scene.play(8 * 60);
        Some(scene.count("ARMPW"))
    };
    for metal in [300, 100] {
        let Some(alone) = made(&[], metal) else { return };
        for duties in [vec![Duty::Economy], vec![Duty::Repair, Duty::Reclaim], vec![Duty::Defence, Duty::Defence], vec![Duty::Economy; 3]] {
            let with = made(&duties, metal).unwrap();
            println!("stores {metal}: {alone} kbots alone, {with} with {:?}", duties.iter().map(|d| d.name()).collect::<Vec<_>>());
            assert!(with * 10 >= alone * 9, "{with} kbots against {alone} alone, with {:?}", duties.iter().map(|d| d.name()).collect::<Vec<_>>());
        }
        assert!(made(&[Duty::Economy; 3], metal).unwrap() > alone, "three on the economy duty make the lab's output grow");
    }
}

// ---- The matrix ------------------------------------------------------------------------------------

/// Each duty on each side, for a constructor and for the Commander: no repeated orders, no two at a
/// job, nowhere near the enemy.
#[test]
fn every_duty_keeps_order_on_both_sides() {
    for sides in [["ARM", "CORE"], ["CORE", "ARM"]] {
        for kind in if sides[0] == "ARM" { ["ARMCK", "ARMCV", "ARMCOM"] } else { ["CORCK", "CORCV", "CORCOM"] } {
            for duty in Duty::ALL {
                let Some(mut scene) = Scene::on("sherwood", sides, true) else { return };
                if kind.ends_with("COM") {
                    let com = scene.commander();
                    scene.officer.set(com, Some(duty));
                } else {
                    serve(&mut scene, kind, duty, 0);
                    serve(&mut scene, kind, duty, 1);
                }
                let a = audit(&mut scene, 2 * 60, &[]);
                assert!(a.spam.is_empty(), "{} {kind} {}: {:?}", sides[0], duty.name(), a.spam);
                assert!(a.conflicts.is_empty(), "{} {kind} {}: {:?}", sides[0], duty.name(), a.conflicts.first());
                assert!(a.under_guns.is_empty() && a.nearest_foe.is_none_or(|d| d > 1500), "{} {kind} {}: {:?} {:?}", sides[0], duty.name(), a.nearest_foe, a.under_guns.first());
            }
        }
    }
}

/// On islands and at sea: ships, hovercraft, aircraft and kbots each on their duties, with no
/// repeated orders (a constructor that can build nothing where it is stands still).
#[test]
fn every_duty_keeps_order_on_islands_and_at_sea() {
    for map_name in ["coast to coast", "hundred isles", "brain coral"] {
        for kind in ["ARMCK", "ARMCH", "ARMCA", "ARMCS"] {
            for duty in Duty::ALL {
                let Some(mut scene) = Scene::on(map_name, ["ARM", "CORE"], true) else { return };
                serve(&mut scene, kind, duty, 0);
                let a = audit(&mut scene, 2 * 60, &[]);
                assert!(a.spam.is_empty(), "{map_name} {kind} {}: {:?}", duty.name(), a.spam);
                assert!(a.ordered.len() < 40, "{map_name} {kind} {}: {} orders", duty.name(), a.ordered.len());
            }
        }
    }
}

/// An advanced constructor on the economy duty upgrades extractors to moho mines once the metal
/// income can pay for them, and stays its hand before (a moho costs 1,500 metal).
#[test]
fn an_advanced_constructor_upgrades_extractors_when_the_income_can_pay() {
    for (mines, expect) in [(4, false), (12, true)] {
        let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
        scene.sim.players[0].economy.metal = Fx::from_int(900);
        scene.sim.players[0].economy.energy = Fx::from_int(8000);
        for d in deposits_near(&scene, 1500).iter().take(mines) {
            scene.spawn_at("ARMMEX", 0, *d);
        }
        for k in 0..8 {
            scene.spawn("ARMSOLAR", -140 - 70 * (k % 4), 250 + 90 * (k / 4));
        }
        serve(&mut scene, "ARMACK", Duty::Economy, 0);
        let a = audit(&mut scene, 4 * 60, &[]);
        let mohos = a.ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Build { type_id, .. }, .. } if scene.sim.types.get(*type_id).name == "ARMMOHO")).count();
        println!("{mines} extractors: {mohos} moho orders");
        assert_eq!(mohos > 0, expect, "{mines} extractors");
    }
}

/// With the stores empty and next to nothing coming in, the economy duty starts with what makes
/// metal or power, not with storage or a metal maker.
#[test]
fn with_the_stores_empty_the_economy_duty_starts_with_income() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    scene.sim.players[0].economy.metal = Fx::ZERO;
    scene.sim.players[0].economy.energy = Fx::ZERO;
    serve(&mut scene, "ARMCK", Duty::Economy, 0);
    let a = audit(&mut scene, 2 * 60, &[]);
    let first = a.ordered.iter().find_map(|(_, c)| match c {
        Command::Order { order: Order::Build { type_id, .. }, .. } => Some(scene.sim.types.get(*type_id).name.clone()),
        _ => None,
    });
    println!("first build: {first:?}");
    assert!(matches!(first.as_deref(), Some("ARMMEX" | "ARMSOLAR" | "ARMWIN")), "{first:?}");
    assert!(a.spam.is_empty());
}
