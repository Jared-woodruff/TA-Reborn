//! Original vehicles, kbots, amphibians and hovercraft use their authored motion/scripts.

use super::*;
use super::air_original::{field, original};

#[test]
fn original_dguns_hit_a_line_of_units_when_aimed_at_ground() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    for name in ["ARMCOM", "CORCOM"] {
        for ground in [false, true] {
            let mut sim = field(data, types, false);
            let commander = sim.spawn_unit(types.id(name).unwrap(), 0, FxVec3::from_ints(400, 60, 600), 16384, true).unwrap();
            let targets: Vec<_> = [90, 140, 190].into_iter().map(|dx| {
                sim.spawn_unit(types.id("CORTHUD").unwrap(), 0, FxVec3::from_ints(400 + dx, 60, 600), 0, true).unwrap()
            }).collect();
            for id in std::iter::once(commander).chain(targets.iter().copied()) { sim.unit_mut(id).unwrap().fire_state = 0; }
            let weapon = types.get(types.id(name).unwrap()).weapons.iter().flatten().copied().find(|&w| sim.weapon_types.get(w).command_fire).unwrap();
            let target = if ground { Target::Ground(sim.unit(*targets.last().unwrap()).unwrap().pos) } else { Target::Unit(targets[0]) };
            sim.give_orders(commander, [Order::ManualFire(target)]);
            let mut shots = 0;
            for _ in 0..180 {
                sim.step(&[]);
                for event in sim.take_events() {
                    if matches!(event, SimEvent::WeaponFired { weapon: w, .. } if w == weapon) { shots += 1; }
                }
            }
            let alive: Vec<_> = targets.iter().map(|&id| sim.unit(id).is_some_and(|u| !u.dead)).collect();
            println!("{name} ground={ground}: shots={shots}, alive={alive:?}");
            assert_eq!(shots, 1);
            assert!(alive.iter().all(|&a| !a), "{name} ground={ground}: a D-gun must pierce the line, alive={alive:?}");
        }
    }
}

#[test]
fn original_dguns_run_along_the_ground_past_their_aim_and_spare_the_commanders_side() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let thud = types.id("CORTHUD").unwrap();
    for name in ["ARMCOM", "CORCOM"] {
        let mut sim = field(data, types, false);
        let commander = sim.spawn_unit(types.id(name).unwrap(), 0, FxVec3::from_ints(400, 60, 600), 16384, true).unwrap();
        // Aimed at the ground 240 px east, a shot that kept its way down went under everything
        // past there; a builder beside the Commander, on the side of its gun, is out of the way.
        let line: Vec<_> = [150, 300, 450, 600, 700].into_iter().map(|dx| sim.spawn_unit(thud, 0, FxVec3::from_ints(400 + dx, 60, 600), 0, true).unwrap()).collect();
        let builder = sim.spawn_unit(thud, 0, FxVec3::from_ints(400, 60, 636), 0, true).unwrap();
        for id in [commander, builder].into_iter().chain(line.iter().copied()) { sim.unit_mut(id).unwrap().fire_state = 0; }
        let weapon = types.get(types.id(name).unwrap()).weapons.iter().flatten().copied().find(|&w| sim.weapon_types.get(w).command_fire).unwrap();
        sim.give_orders(commander, [Order::ManualFire(Target::Ground(FxVec3::from_ints(640, 60, 600)))]);
        let mut shots = 0;
        for _ in 0..200 {
            sim.step(&[]);
            for event in sim.take_events() {
                if matches!(event, SimEvent::WeaponFired { weapon: w, .. } if w == weapon) { shots += 1; }
            }
        }
        let alive: Vec<_> = line.iter().map(|&id| sim.unit(id).is_some_and(|u| !u.dead)).collect();
        assert_eq!(shots, 1);
        assert!(alive.iter().all(|&a| !a), "{name}: the shot runs on along the ground, alive={alive:?}");
        assert!(sim.unit(builder).is_some_and(|u| !u.dead), "{name}: the builder beside the Commander");
    }
}

#[test]
fn original_commanders_capture_the_galactic_gate_in_two_minutes() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let (Some(commander), Some(gate)) = (types.id("ARMCOM"), types.id("CORGATE")) else { return };
    let map = ta_formats::tnt::TntMap { width: 128, height: 128, sea_level: 0, heights: vec![60; 128 * 128], features: vec![ta_formats::tnt::FEATURE_NONE; 128 * 128], tiles: Vec::new(), tile_map: Vec::new(), feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    let players = [
        PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::ZERO, energy: Fx::ZERO, ai: false },
        PlayerSetup { side: "CORE".into(), color: 1, start: FxVec3::from_ints(1500, 60, 1500), metal: Fx::ZERO, energy: Fx::ZERO, ai: false },
    ];
    let mut sim = Sim::new(types.clone(), data.clone(), &map, &ta_data::MapInfo::default(), 0, &players, 73);
    let com = sim.spawn_unit(commander, 0, FxVec3::from_ints(400, 60, 600), 16384, true).unwrap();
    let target = sim.spawn_unit(gate, 1, FxVec3::from_ints(520, 60, 600), 0, true).unwrap();
    sim.unit_mut(com).unwrap().fire_state = 0;
    // Its file gives it 197,485 metal and 1,982,431 energy: four times that took 51 minutes.
    sim.give_orders(com, [Order::Capture(target)]);
    let mut ticks = 0;
    while ticks < 30 * 300 && sim.unit(target).is_some_and(|u| u.owner == 1) {
        sim.step(&[]);
        ticks += 1;
    }
    assert_eq!(sim.unit(target).map(|u| u.owner), Some(0), "captured within five minutes");
    let seconds = ticks as f64 / 30.0;
    assert!((120.0..125.0).contains(&seconds), "two minutes of work and the walk up, not {seconds:.1} s");
}

#[test]
fn original_land_units_move_stop_and_reverse_direction() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let mut checked = 0;
    for t in types.types.iter().filter(|t| matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover)) {
        let mut sim = field(data, types, false);
        let unit = sim.spawn_unit(t.id, 0, FxVec3::from_ints(400, 60, 600), 16384, true).unwrap();
        sim.unit_mut(unit).unwrap().fire_state = 0;
        for x in [1000, 400] {
            let goal = FxVec3::from_ints(x, 60, 600);
            sim.give_orders(unit, [Order::Move(goal)]);
            for _ in 0..2400 {
                sim.step(&[]);
                if sim.unit(unit).unwrap().orders.is_empty() { break; }
            }
            for _ in 0..120 { sim.step(&[]); }
            let u = sim.unit(unit).unwrap();
            assert!(u.orders.is_empty() && u.path_goal.is_none() && u.pos.within_xz(goal, t.radius * 3 + Fx::from_int(24)), "{} failed its Move to {x}: {:?}, {:?}, {:?}", t.name, u.pos, u.path_goal, u.orders);
            assert!(u.speed.is_zero() && !u.moving, "{} must stop its motion/script after arrival", t.name);
            assert!(sim.world.terrain_passable(&sim.move_spec(t.id), World::pos_to_anchor(u.pos, t.footprint)));
        }
        checked += 1;
    }
    assert!(checked >= 40, "expected the original land roster, checked {checked}");
    println!("{checked} original land units moved, stopped and reversed direction");
}
