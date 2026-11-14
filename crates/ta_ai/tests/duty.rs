//! A person's constructors on duty, on real game data (skipped without a TA install): the
//! player's own `DutyOfficer` sends them to work with ordinary orders, and no one else's.
//!
//! `TA_TEST_MAP` picks the map (default Sherwood), `TA_TEST_LOG=1` prints the officer's debug
//! log (what each constructor was sent to do).

mod common;

use std::collections::BTreeSet;

use common::*;
use ta_ai::Duty;
use ta_sim::{Cheat, Command, Fx, FxVec3, Order, Target, UnitId};

#[test]
fn a_constructor_on_the_economy_builds_extractors_and_power() {
    let Some(mut scene) = Scene::new() else { return };
    let builder = scene.spawn("ARMCK", 60, 60);
    let (extractors, generators) = (scene.extractors(), scene.generators());
    scene.officer.set(builder, Some(Duty::Economy));
    // The store is empty: power is what is wanted first.
    scene.sim.players[0].economy.energy = Fx::ZERO;
    let ordered = scene.play(4 * 60);
    let built = ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Build { .. }, .. })).count();
    println!("economy duty: {} extractors and {} power plants new in four minutes, {built} build orders", scene.extractors() - extractors, scene.generators() - generators);
    assert!(scene.extractors() >= extractors + 2, "extractors on free metal");
    assert!(scene.generators() > generators, "power when it runs short");
    assert!(scene.officer.duty(builder) == Some(Duty::Economy), "the officer never takes a unit off duty");
}

#[test]
fn a_constructor_on_repair_duty_mends_a_damaged_building() {
    let Some(mut scene) = Scene::new() else { return };
    let solar = scene.spawn("ARMSOLAR", 180, 40);
    let lab = scene.spawn("ARMLAB", -200, 40);
    // A Peewee of the enemy's shoots at the solar collector for a while, then goes.
    let raider = scene.spawn_at("ARMPW", 1, scene.sim.unit(solar).unwrap().pos + FxVec3::from_ints(60, 0, 0));
    let attack = Command::Order { player: 1, units: vec![raider], order: Order::Attack(Target::Unit(solar)), queue: false };
    let max = scene.sim.unit_type(scene.sim.unit(solar).unwrap()).max_health;
    let mut extra = vec![attack];
    for _ in 0..30 * 40 {
        scene.sim.step(&std::mem::take(&mut extra));
        scene.sim.take_events();
        if scene.sim.unit(solar).unwrap().health * 2 < max {
            break;
        }
    }
    scene.sim.remove_unit(raider);
    let hurt = scene.sim.unit(solar).unwrap().health;
    assert!(hurt < max, "the solar collector was hurt");
    // (Beside it: a building may stand where no constructor can walk up to it.)
    let builder = scene.spawn_at("ARMCK", 0, scene.sim.unit(solar).unwrap().pos + FxVec3::from_ints(0, 0, 60));
    scene.officer.set(builder, Some(Duty::Repair));
    scene.play(60);
    let after = scene.sim.unit(solar).unwrap().health;
    println!("repair duty: health {:.0} of {:.0}, mended to {:.0}", hurt.to_f32(), max.to_f32(), after.to_f32());
    assert!(after >= max, "mended whole");
    assert!(scene.sim.unit(lab).unwrap().health >= scene.sim.unit_type(scene.sim.unit(lab).unwrap()).max_health);
}

#[test]
fn a_constructor_on_reclaim_duty_takes_wrecks() {
    let Some(mut scene) = Scene::new() else { return };
    // Some of our tanks shot to pieces by raiders near the base leave wrecks (a unit blown up in
    // one blast leaves none).
    let tanks: Vec<UnitId> = (0..3).map(|k| scene.spawn("ARMSTUMP", 70 + 60 * k, -80)).collect();
    let at = scene.sim.unit(tanks[0]).unwrap().pos;
    let raiders: Vec<UnitId> = (0..8).map(|k| scene.spawn_at("ARMPW", 1, at + FxVec3::from_ints(0, 0, -120 - 20 * k))).collect();
    let orders: Vec<Command> = raiders.iter().enumerate().map(|(k, &r)| Command::Order { player: 1, units: vec![r], order: Order::Attack(Target::Unit(tanks[k % 3])), queue: false }).chain([Command::Cheat { player: 1, cheat: Cheat::Damage(1) }]).collect();
    let mut extra = orders;
    for _ in 0..30 * 120 {
        scene.sim.step(&std::mem::take(&mut extra));
        scene.sim.take_events();
        if tanks.iter().all(|&t| scene.sim.unit(t).is_none_or(|u| u.dead)) {
            break;
        }
    }
    for r in raiders {
        scene.sim.remove_unit(r);
    }
    scene.sim.step(&[]);
    let wrecks = |scene: &Scene| -> Vec<usize> {
        scene
            .sim
            .features
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                let d = &scene.sim.data.features[f.def];
                f.alive && d.reclaimable && d.metal > 0.0 && f.pos.within_xz(at, Fx::from_int(200))
            })
            .map(|(i, _)| i)
            .collect()
    };
    let before = wrecks(&scene);
    assert!(!before.is_empty(), "the tanks left wrecks");
    let builder = scene.spawn("ARMCK", 60, 60);
    scene.officer.set(builder, Some(Duty::Reclaim));
    scene.sim.players[0].economy.metal = Fx::ZERO;
    let produced = scene.sim.players[0].stats.metal_produced;
    let ordered = scene.play(120);
    let left = wrecks(&scene);
    let gained = (scene.sim.players[0].stats.metal_produced - produced).to_int();
    println!("reclaim duty: {} wrecks, {} left, {gained} metal taken", before.len(), left.len());
    assert!(ordered.iter().any(|(_, c)| matches!(c, Command::Order { order: Order::Reclaim(f), .. } if before.contains(f))), "sent to reclaim a wreck");
    assert!(left.len() < before.len(), "wrecks reclaimed");
    assert!(gained > 0, "metal from the wrecks");
}

/// A wreck whose reclaim was left nearly done still pays all of its metal when it is finished (it
/// pays when done), so a constructor on reclaim duty goes to finish it: valued by what was left of
/// it, as when reclaiming paid as it went, it was passed over.
#[test]
fn a_constructor_on_reclaim_duty_finishes_a_wreck_left_nearly_reclaimed() {
    let Some(mut scene) = Scene::new() else { return };
    let wreck = ["ARMFUS_DEAD", "CORFUS_DEAD", "ARMMAKR_DEAD", "CORTHUD_DEAD"]
        .into_iter()
        .flat_map(|name| [(200, 120), (-200, 120), (120, -200), (-120, -200)].map(|(dx, dz)| (name, dx, dz)))
        .find_map(|(name, dx, dz)| {
            let i = scene.sim.place_feature(name, scene.start + FxVec3::from_ints(dx, 0, dz), 0)?;
            (scene.sim.data.features[scene.sim.features[i].def].metal >= 60.0).then_some(i)
        })
        .expect("a rich wreck put down near the base");
    scene.sim.features[wreck].reclaimed = Fx::from_ratio(99, 100);
    let builder = scene.spawn("ARMCK", 60, 60);
    scene.officer.set(builder, Some(Duty::Reclaim));
    let ordered = scene.play(20);
    assert!(
        ordered.iter().any(|(_, c)| matches!(c, Command::Order { order: Order::Reclaim(f), .. } if *f == wreck)),
        "sent to finish the wreck left at 99%: {:?}",
        ordered.iter().map(|(_, c)| c).take(4).collect::<Vec<_>>()
    );
}

#[test]
fn a_constructor_on_defence_duty_puts_up_towers_on_the_side_of_the_enemy() {
    let Some(mut scene) = Scene::new() else { return };
    // Metal and power coming in (the defence duty waits for an economy that can pay for a tower within
    // a minute: a tower costs thousands of energy).
    for (x, z) in [(200, 1576), (248, 1784), (584, 1336), (88, 2248)] {
        scene.spawn_at("ARMMEX", 0, FxVec3::from_ints(x, 0, z));
    }
    for k in 0..3 {
        scene.spawn("ARMSOLAR", -140 - 70 * k, 150);
    }
    let builder = scene.spawn("ARMCK", 60, 60);
    scene.officer.set(builder, Some(Duty::Defence));
    let ordered = scene.play(3 * 60);
    let enemy = scene.sim.units().find(|u| u.owner == 1).unwrap().pos;
    let towers: Vec<FxVec3> = ordered
        .iter()
        .filter_map(|(_, c)| match c {
            Command::Order { order: Order::Build { type_id, pos }, .. } if scene.sim.types.get(*type_id).weapons.iter().any(|w| w.is_some()) => Some(*pos),
            _ => None,
        })
        .collect();
    println!("defence duty: {} tower orders, the first {:?}", towers.len(), towers.first().map(|p| (p.x.to_int(), p.z.to_int())));
    assert!(!towers.is_empty(), "towers up");
    let from = |p: FxVec3| (p - enemy).xz().length();
    assert!(towers.iter().all(|&p| from(p) < from(scene.start)), "every tower is nearer the enemy than the Commander's start");
    assert!(ordered.iter().all(|(_, c)| matches!(c, Command::Order { order: Order::Build { .. } | Order::Repair(_) | Order::Guard(_) | Order::Move(_), .. })), "nothing but building, helping and going home");
}

#[test]
fn the_officer_goes_only_where_the_player_has_been() {
    // Unmapped: the ground is dark until explored, and the officer sends a constructor only to
    // sites it can see: extractors and power, and the towers of a defence, which face the enemy
    // (the side nobody has been to).
    let Some(mut scene) = Scene::with(false) else { return };
    let builder = scene.spawn("ARMCK", 60, 60);
    scene.officer.set(builder, Some(Duty::Economy));
    let guard = scene.spawn("ARMCK", -60, 60);
    scene.officer.set(guard, Some(Duty::Defence));
    let mut sites = 0;
    for _ in 0..30 * 180 {
        let commands = scene.officer.think(&scene.sim);
        for c in &commands {
            if let Command::Order { order: Order::Build { pos, .. }, .. } = c {
                assert!(scene.sim.explored(0, *pos), "a site nobody has seen: ({}, {})", pos.x.to_int(), pos.z.to_int());
                sites += 1;
            }
        }
        scene.sim.step(&commands);
        scene.sim.take_events();
    }
    println!("unmapped: {sites} build orders, all on explored ground");
    assert!(sites > 0);
}

#[test]
fn the_officer_orders_only_the_units_on_duty() {
    let Some(mut scene) = Scene::new() else { return };
    let duty = scene.spawn("ARMCK", 60, 60);
    let free = scene.spawn("ARMCK", -60, 60);
    // A factory with work to help, a damaged unit nearby is none of the officer's concern.
    let lab = scene.spawn("ARMLAB", -200, 120);
    let tank = scene.types_id("ARMSTUMP");
    scene.sim.step(&[Command::Produce { player: 0, factory: lab, type_id: tank, count: 3 }]);
    scene.officer.set(duty, Some(Duty::Economy));
    let ordered = scene.play(3 * 60);
    assert!(!ordered.is_empty());
    for (tick, c) in &ordered {
        match c {
            Command::Order { player, units, .. } => {
                assert_eq!(*player, 0);
                assert_eq!(units, &vec![duty], "tick {tick}: {c:?}");
            }
            other => panic!("tick {tick}: the officer gave a command that is no order to a unit on duty: {other:?}"),
        }
    }
    assert!(scene.sim.unit(free).unwrap().orders.is_empty(), "the constructor not on duty was left alone");
    assert_eq!(scene.officer.units().collect::<Vec<_>>(), vec![(duty, Duty::Economy)]);
}

/// A solar collector of ours, hurt to a third of its health: it, and its health now and at best.
fn hurt_solar(scene: &mut Scene, at: FxVec3) -> (UnitId, Fx, Fx) {
    let solar = scene.spawn_at("ARMSOLAR", 0, at);
    let max = scene.sim.unit_type(scene.sim.unit(solar).unwrap()).max_health;
    scene.sim.set_health(solar, max / 3);
    (solar, scene.sim.unit(solar).unwrap().health, max)
}

#[test]
fn a_constructor_helping_at_a_busy_factory_still_goes_to_what_needs_mending() {
    let Some(mut scene) = Scene::new() else { return };
    let builder = scene.spawn("ARMCK", 60, 60);
    let lab = scene.spawn("ARMLAB", -200, 120);
    let kbot = scene.types_id("ARMPW");
    scene.sim.step(&[Command::Produce { player: 0, factory: lab, type_id: kbot, count: 60 }]);
    // Nothing to mend: it lends a hand at the busy lab.
    scene.officer.set(builder, Some(Duty::Repair));
    scene.play(40);
    let helping: Vec<Order> = scene.sim.unit(builder).unwrap().orders.iter().cloned().collect();
    println!("repair duty with nothing to mend: {helping:?}");
    assert!(!helping.is_empty(), "it helps at the lab");
    // Then a solar collector near the base is hurt: it goes to mend it, within the half minute.
    let at = scene.sim.unit(lab).unwrap().pos + FxVec3::from_ints(0, 0, -130);
    let (solar, hurt, max) = hurt_solar(&mut scene, at);
    assert!(hurt < max);
    scene.play(30);
    let after = scene.sim.unit(solar).unwrap().health;
    println!("a helper at the lab: the solar collector hurt to {:.0} of {:.0}, mended to {:.0} in half a minute", hurt.to_f32(), max.to_f32(), after.to_f32());
    assert!(after > hurt, "it left the lab to mend");
}

#[test]
fn a_constructor_helping_at_a_factory_takes_a_new_duty_at_once() {
    let Some(mut scene) = Scene::new() else { return };
    let builder = scene.spawn("ARMCK", 60, 60);
    let lab = scene.spawn("ARMLAB", -200, 120);
    let kbot = scene.types_id("ARMPW");
    // The player has it lending a hand at a busy lab.
    scene.sim.step(&[Command::Produce { player: 0, factory: lab, type_id: kbot, count: 60 }]);
    assert!(scene.play(2).is_empty(), "nobody on duty");
    scene.play_with(3, &[Command::Order { player: 0, units: vec![builder], order: Order::Guard(lab), queue: false }]);
    let helping: Vec<Order> = scene.sim.unit(builder).unwrap().orders.iter().cloned().collect();
    println!("the player's helper: {helping:?}");
    assert!(helping.iter().any(|o| matches!(o, Order::Guard(_))), "it helps the lab");
    // Put on the economy, which has work for it (free deposits), it leaves the lab within a think, not
    // at the next of the helpers' now-and-then looks.
    scene.officer.set(builder, Some(Duty::Economy));
    let ordered = scene.play(2);
    let first = ordered.iter().find(|(_, c)| matches!(c, Command::Order { order: Order::Build { .. }, .. }));
    println!("economy duty for a helper at the lab: {:?}", first.map(|(tick, c)| (*tick, c)));
    let first_tick = first.expect("a new job at once").0;
    assert!(first_tick < scene.sim.tick - 30, "within the first second");
}

#[test]
fn the_officer_is_deterministic() {
    if data().is_none() {
        return;
    }
    let run = || {
        let mut scene = Scene::new().unwrap();
        let a = scene.spawn("ARMCK", 60, 60);
        let b = scene.spawn("ARMCK", -60, 60);
        scene.spawn("ARMLAB", -200, 120);
        scene.officer.set(a, Some(Duty::Economy));
        scene.officer.set(b, Some(Duty::Defence));
        let ordered = scene.play(2 * 60);
        (ordered, scene.sim.checksum())
    };
    let (first, sum) = run();
    let (second, again) = run();
    assert!(!first.is_empty());
    assert_eq!(first, second, "the same commands for the same game");
    assert_eq!(sum, again);
}

#[test]
fn units_lost_leave_their_duty_and_an_officer_with_no_one_on_duty_is_silent() {
    let Some(mut scene) = Scene::new() else { return };
    assert!(scene.play(30).is_empty(), "nobody on duty");
    let builder = scene.spawn("ARMCK", 60, 60);
    let other = scene.spawn("ARMCK", -60, 60);
    scene.officer.set(builder, Some(Duty::Reclaim));
    scene.officer.set(other, Some(Duty::Repair));
    assert_eq!(scene.officer.len(), 2);
    scene.play_with(2, &[Command::Cheat { player: 0, cheat: Cheat::Destroy(vec![builder]) }]);
    assert_eq!(scene.officer.units().map(|(id, _)| id).collect::<BTreeSet<_>>(), BTreeSet::from([other]));
    scene.officer.set(other, None);
    assert!(scene.officer.is_empty());
    assert!(scene.play(30).is_empty());
}

impl Scene {
    fn types_id(&self, name: &str) -> usize {
        self.type_id(name)
    }
}
