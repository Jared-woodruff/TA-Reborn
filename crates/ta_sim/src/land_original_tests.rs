//! Original vehicles, kbots, amphibians and hovercraft use their authored motion/scripts.

use super::*;
use super::air_original::{field, original};

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
