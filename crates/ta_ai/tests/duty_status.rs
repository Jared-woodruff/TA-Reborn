//! What the officer tells of a constructor on duty, on real game data (skipped without a TA install): at
//! work, getting away from enemies or with nothing to do, and why; and the one notice that comes of having
//! nothing to do for a while. Each test pins down a state the player is shown (the card's line, the tag).

mod common;

use common::*;
use ta_ai::{Duty, DutyEvent, DutyStatus, IdleReason};
use ta_sim::{Command, Fx, FxVec3, UnitId};

/// What a unit on duty is about, if it is idle: why.
fn idle(scene: &Scene, id: UnitId) -> Option<IdleReason> {
    match scene.officer.status(id) {
        Some(DutyStatus::Idle(why)) => Some(why),
        _ => None,
    }
}

/// Plays on, a few seconds at a time, until the unit has nothing to do (at most `limit` seconds).
fn play_until_idle(scene: &mut Scene, id: UnitId, limit: u64) -> Option<IdleReason> {
    for _ in 0..limit / 2 {
        scene.play(2);
        if let Some(why) = idle(scene, id) {
            return Some(why);
        }
    }
    None
}

/// Stores filled to the brim.
fn fill_stores(scene: &mut Scene) {
    let p = &mut scene.sim.players[0].economy;
    p.metal = p.metal_capacity;
    p.energy = p.energy_capacity;
}

/// Everything the ground round home holds to take apart gone: trees, rocks, wrecks.
fn clear_the_ground(scene: &mut Scene) {
    for f in scene.sim.features.iter_mut() {
        f.alive = false;
    }
}

// ---- Nothing to do, and why ---------------------------------------------------------------------------

/// A mender with nothing hurt near it says so; it is at work when something is.
#[test]
fn a_mender_with_nothing_hurt_says_so_and_is_at_work_when_something_is() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let builder = serve(&mut scene, "ARMCK", Duty::Repair, 0);
    scene.play(6);
    assert_eq!(scene.officer.status(builder), Some(DutyStatus::Idle(IdleReason::NothingHurt)));
    // (A building beside it: one may stand where no constructor can walk up to it.)
    let at = scene.sim.unit(builder).unwrap().pos + FxVec3::from_ints(30, 0, -70);
    let solar = scene.spawn_at("ARMSOLAR", 0, at);
    let max = scene.sim.unit_type(scene.sim.unit(solar).unwrap()).max_health;
    scene.sim.set_health(solar, max / 3);
    scene.play(6);
    assert_eq!(scene.officer.status(builder), Some(DutyStatus::Working), "sent to mend it");
    assert!(scene.sim.unit(solar).unwrap().health > max / 3, "and mending");
}

/// A reclaimer says whether there is nothing to take or the stores are full (nothing is worth taking of a
/// kind the store is full of).
#[test]
fn a_reclaimer_says_whether_nothing_is_left_to_take_or_the_stores_are_full() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    clear_the_ground(&mut scene);
    let builder = serve(&mut scene, "ARMCK", Duty::Reclaim, 0);
    scene.play(6);
    assert_eq!(idle(&scene, builder), Some(IdleReason::NothingToReclaim));
    // A wreck beside it, and the stores full: nothing to gain by taking it.
    let at = scene.sim.unit(builder).unwrap().pos + FxVec3::from_ints(40, 0, 0);
    let wreck = scene.sim.place_feature("ARMFLASH_DEAD", at, 0).expect("a wreck");
    fill_stores(&mut scene);
    scene.play(6);
    assert_eq!(idle(&scene, builder), Some(IdleReason::StoresFull));
    // Room in the stores again: it goes for the wreck.
    scene.sim.players[0].economy.metal = Fx::ZERO;
    scene.play(6);
    assert_eq!(scene.officer.status(builder), Some(DutyStatus::Working));
    assert!(scene.sim.features[wreck].reclaimed > Fx::ZERO, "and takes it apart");
}

/// A constructor on the defence duty says whether the towers wanted stand or it is saving up for one.
#[test]
fn a_defender_says_whether_the_towers_stand_or_it_is_saving_up() {
    // Metal coming in, and the stores empty: the cheapest tower cannot be paid for within a minute.
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let builder = serve(&mut scene, "ARMCK", Duty::Defence, 0);
    for d in deposits_near(&scene, 1500).iter().take(4) {
        scene.spawn_at("ARMMEX", 0, *d);
    }
    scene.sim.players[0].economy.metal = Fx::ZERO;
    scene.sim.players[0].economy.energy = Fx::ZERO;
    let why = play_until_idle(&mut scene, builder, 60);
    println!("poor: {why:?}");
    assert_eq!(why, Some(IdleReason::SavingUp));
    // The two towers wanted at the start standing: done, whatever the stores hold.
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let builder = serve(&mut scene, "ARMCK", Duty::Defence, 0);
    for k in 0..2 {
        scene.spawn("ARMLLT", 300 + 60 * k, 60);
    }
    let why = play_until_idle(&mut scene, builder, 60);
    println!("towers standing: {why:?}");
    assert_eq!(why, Some(IdleReason::DefencesDone));
}

/// An economy builder says why it has nothing more to build: every free deposit taken, or an advanced
/// constructor waiting for the metal income to pay for its mines.
#[test]
fn an_economy_builder_says_why_it_has_nothing_more_to_build() {
    // Every deposit within its reach taken.
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    for d in deposits_near(&scene, 2600) {
        scene.spawn_at("ARMMEX", 0, d);
    }
    scene.sim.players[0].economy.metal = Fx::from_int(5000);
    scene.sim.players[0].economy.energy = Fx::from_int(20000);
    let builder = serve(&mut scene, "ARMCK", Duty::Economy, 0);
    let why = play_until_idle(&mut scene, builder, 240);
    println!("every deposit taken: {why:?}");
    assert_eq!(why, Some(IdleReason::NoDeposits));
    // Free deposits, an advanced constructor and a small income.
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    scene.sim.players[0].economy.metal = Fx::from_int(900);
    scene.sim.players[0].economy.energy = Fx::from_int(8000);
    for d in deposits_near(&scene, 1500).iter().take(2) {
        scene.spawn_at("ARMMEX", 0, *d);
    }
    for k in 0..8 {
        scene.spawn("ARMSOLAR", -140 - 70 * (k % 4), 250 + 90 * (k / 4));
    }
    let builder = serve(&mut scene, "ARMACK", Duty::Economy, 0);
    let why = play_until_idle(&mut scene, builder, 240);
    println!("advanced, a small income: {why:?}");
    assert_eq!(why, Some(IdleReason::WaitingForIncome(10)));
}

/// A constructor away from home with no way back says so.
#[test]
fn a_constructor_that_cannot_get_home_says_so() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let Some(at) = cut_off_spot(&scene, "ARMCK") else { return };
    let id = scene.spawn_at("ARMCK", 0, at);
    scene.officer.set(id, Some(Duty::Reclaim));
    let why = play_until_idle(&mut scene, id, 40);
    println!("cut off: {why:?}");
    assert_eq!(why, Some(IdleReason::Stranded));
}

/// A ship with nothing to do is not called stranded because the base stands on land: it never could get there.
#[test]
fn a_ship_with_nothing_to_do_is_not_called_stranded() {
    for duty in [Duty::Reclaim, Duty::Repair] {
        let Some(mut scene) = Scene::on("coast to coast", ["ARM", "CORE"], true) else { return };
        // On the water some way from home (the base is on the land).
        let Some(at) = [(1200, 0), (-1200, 0), (0, 1200), (0, -1200), (2000, 1000), (-2000, -1000)]
            .into_iter()
            .map(|(dx, dz)| spot_for(&scene, "ARMCS", scene.start + FxVec3::from_ints(dx, 0, dz), 0, 0))
            .find(|&p| px(p, scene.start) >= 500)
        else {
            return;
        };
        let ship = scene.spawn_at("ARMCS", 0, at);
        scene.officer.set(ship, Some(duty));
        let why = play_until_idle(&mut scene, ship, 60);
        println!("{}: {} px from home, {why:?}", duty.name(), px(at, scene.start));
        assert!(why.is_some(), "{}: it had nothing to do", duty.name());
        assert_ne!(why, Some(IdleReason::Stranded), "{}", duty.name());
    }
}

// ---- Getting away -------------------------------------------------------------------------------------

/// A builder with armed enemies close says it is getting away, and is at work again when they are gone.
#[test]
fn a_builder_says_it_is_getting_away_while_enemies_are_close() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let builder = serve(&mut scene, "ARMCK", Duty::Economy, 0);
    scene.play(30);
    assert_ne!(scene.officer.status(builder), Some(DutyStatus::GettingAway));
    let at = scene.sim.unit(builder).unwrap().pos;
    let squad: Vec<UnitId> = (0..6).map(|k| scene.spawn_at("CORAK", 1, at + FxVec3::from_ints(300 + 20 * k, 0, 0))).collect();
    scene.sim.step(&[Command::SetStates { player: 1, units: squad.clone(), fire: Some(0), movement: Some(0) }]);
    scene.play(3);
    assert_eq!(scene.officer.status(builder), Some(DutyStatus::GettingAway));
    for s in squad {
        scene.sim.remove_unit(s);
    }
    scene.play(20);
    assert_ne!(scene.officer.status(builder), Some(DutyStatus::GettingAway), "free to work again");
}

// ---- The notice -----------------------------------------------------------------------------------------

/// A constructor with nothing to do for twenty seconds is told of, once after its duty is given; a new duty
/// is told of again.
#[test]
fn nothing_to_do_for_a_while_is_told_once_after_a_duty_is_given() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let builder = serve(&mut scene, "ARMCK", Duty::Repair, 0);
    scene.play(10);
    assert!(scene.officer.take_events().is_empty(), "not yet");
    scene.play(25);
    assert_eq!(scene.officer.take_events(), vec![DutyEvent::Idle { unit: builder, duty: Duty::Repair, why: IdleReason::NothingHurt }]);
    scene.play(60);
    assert!(scene.officer.take_events().is_empty(), "once");
    // The same duty given again is no new duty; another one is.
    scene.officer.set(builder, Some(Duty::Repair));
    scene.play(40);
    assert!(scene.officer.take_events().is_empty());
    clear_the_ground(&mut scene);
    scene.sim.players[0].economy.metal = Fx::from_int(300);
    scene.officer.set(builder, Some(Duty::Reclaim));
    scene.play(40);
    assert_eq!(scene.officer.take_events(), vec![DutyEvent::Idle { unit: builder, duty: Duty::Reclaim, why: IdleReason::NothingToReclaim }]);
}

/// Several constructors on one duty with the same reason are told of in one notice.
#[test]
fn constructors_idle_for_the_same_reason_are_told_of_together() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let ids: Vec<UnitId> = (0..4).map(|k| serve(&mut scene, "ARMCK", Duty::Repair, k)).collect();
    scene.play(40);
    let events = scene.officer.take_events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(matches!(events[0], DutyEvent::Idle { duty: Duty::Repair, why: IdleReason::NothingHurt, unit } if ids.contains(&unit)));
}

/// Constructors running out of work one after another, for one reason, are one piece of news (four reclaimers
/// put on duty a little apart were four notices of the same words).
#[test]
fn constructors_running_out_of_work_one_after_another_are_one_notice() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    clear_the_ground(&mut scene);
    // (Stores that stay half empty: it is always the same reason.)
    scene.sim.players[0].economy.metal = Fx::from_int(300);
    scene.sim.players[0].economy.energy = Fx::from_int(300);
    let mut told = Vec::new();
    for k in 0..4 {
        serve(&mut scene, "ARMCK", Duty::Reclaim, k);
        scene.play(25);
        told.extend(scene.officer.take_events());
    }
    scene.play(30);
    told.extend(scene.officer.take_events());
    assert_eq!(told.len(), 1, "{told:?}");
    // And again once the two minutes are up, for a unit that has been given a duty since.
    serve(&mut scene, "ARMCK", Duty::Reclaim, 4);
    scene.play(120);
    told.extend(scene.officer.take_events());
    assert_eq!(told.len(), 2, "{told:?}");
}

/// The status is none for a unit not on duty, and goes with the duty.
#[test]
fn a_unit_off_duty_has_no_status() {
    let Some(mut scene) = Scene::on("sherwood", ["ARM", "CORE"], true) else { return };
    let builder = serve(&mut scene, "ARMCK", Duty::Repair, 0);
    assert_eq!(scene.officer.status(builder), None, "before its first beat");
    scene.play(6);
    assert!(scene.officer.status(builder).is_some());
    scene.officer.set(builder, None);
    assert_eq!(scene.officer.status(builder), None);
    let other = scene.spawn("ARMCK", 100, 100);
    scene.play(6);
    assert_eq!(scene.officer.status(other), None);
}
