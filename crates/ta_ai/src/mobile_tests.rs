//! The computer players and factories that move (`MobileFactory`), with made-up units: one with
//! nothing to make is given the best fighting unit of its menu that it can pay for, none while it
//! is making one, and it fights with the army.

use super::*;

/// The fixture with a factory that moves (`MOVER`), its menu a tank and a costly heavy tank.
fn sim() -> Sim {
    let mut sim = fixture_edit(|data| {
        let factory = data.unit_id("FACTORY").unwrap();
        let mut mover = data.units[factory].clone();
        (mover.name, mover.object, mover.mobile_factory, mover.can_move, mover.max_velocity, mover.worker_time) = ("MOVER".into(), "MOVER".into(), true, true, 1.0, 300.0);
        mover.build_options = vec!["TANK".into(), "HEAVY".into()];
        let tank = data.unit_id("TANK").unwrap();
        // (A hundred seconds' work at its build power: still being made a few ticks on.)
        data.units[tank].build_time = 30_000.0;
        let mut heavy = data.units[tank].clone();
        (heavy.name, heavy.object, heavy.build_cost_metal, heavy.max_damage) = ("HEAVY".into(), "HEAVY".into(), 100_000.0, 2000.0);
        data.units.push(mover);
        data.units.push(heavy);
    });
    sim.set_starting_resources(0, fx(1_000), fx(10_000));
    sim
}

#[test]
fn a_factory_that_moves_makes_the_best_unit_it_can_pay_for() {
    let mut sim = sim();
    let mover = sim.spawn_unit(sim.types.id("MOVER").unwrap(), 0, FxVec3::from_ints(400, 60, 400), 0, true).unwrap();
    assert!(sim.unit_type(sim.unit(mover).unwrap()).is_factory, "a factory");
    sim.step(&[]);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    assert!(ai.info[sim.unit(mover).unwrap().type_id].mobile_factory);
    let s = ai.survey(&sim);
    assert!(s.army.iter().any(|u| u.id == mover), "it fights with the army");
    let mut out = Vec::new();
    ai.run_mobile_factories(&sim, &s, &mut out);
    let tank = sim.types.id("TANK").unwrap();
    assert!(matches!(out.as_slice(), [Command::Produce { factory, type_id, count: 1, .. }] if *factory == mover && *type_id == tank), "the tank it can pay for, not the heavy one: {out:?}");
    // Making it on itself, it is given nothing more.
    sim.step(&out);
    sim.step(&[]);
    assert!(sim.unit(mover).unwrap().build.is_some(), "making it on itself");
    let mut more = Vec::new();
    ai.run_mobile_factories(&sim, &ai.survey(&sim), &mut more);
    assert!(more.is_empty(), "nothing more while it makes one: {more:?}");
}
