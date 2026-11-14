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
