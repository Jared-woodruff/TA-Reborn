//! Original naval motion and underwater combat, using the installed models and scripts.

use super::*;
use super::air_original::{field, original};

#[test]
fn original_naval_roster_moves_stops_and_reverses() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let mut checked = 0;
    for t in types.types.iter().filter(|t| t.locomotion == Locomotion::Naval) {
        let mut sim = field(data, types, true);
        let ship = sim.spawn_unit(t.id, 0, FxVec3::from_ints(600, 150, 1000), 16384, true).unwrap();
        sim.unit_mut(ship).unwrap().fire_state = 0;
        for x in [1200, 600] {
            let goal = FxVec3::from_ints(x, 150, 1000);
            sim.give_orders(ship, [Order::Move(goal)]);
            for _ in 0..3600 {
                sim.step(&[]);
                if sim.unit(ship).unwrap().orders.is_empty() { break; }
            }
            for _ in 0..120 { sim.step(&[]); }
            let u = sim.unit(ship).unwrap();
            assert!(u.orders.is_empty() && u.speed.is_zero() && !u.moving && u.pos.within_xz(goal, t.radius * 3 + Fx::from_int(24)), "{} failed its sortie: {:?}, {:?}, {:?}", t.name, u.pos, u.path_goal, u.orders);
            assert!(sim.world.passable(&sim.move_spec(t.id), World::pos_to_anchor(u.pos, t.footprint), None));
            assert_eq!(u.model_pos(t).y, sim.world.sea_level - t.waterline);
        }
        checked += 1;
    }
    assert!(checked >= 20, "expected the original naval roster, checked {checked}");
    println!("{checked} original naval units moved, stopped and reversed");
}

#[test]
fn original_submarines_depth_charges_and_seaplanes_hit_wet_targets() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let mut pairs = vec![("ARMSUB", "CORROY"), ("CORSUB", "ARMROY"), ("ARMROY", "CORSUB"), ("ARMTL", "CORSUB")];
    for t in types.types.iter().filter(|t| t.locomotion == Locomotion::Air && t.weapons.iter().flatten().any(|&w| data.weapons[w].water_weapon)) {
        pairs.push((t.name.as_str(), if t.def.side == "ARM" { "CORSUB" } else { "ARMSUB" }));
    }
    for (source, target) in pairs {
        // The shared fixture has one player; add the target's opponent normally.
        let setup = [PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::from_int(10000), energy: Fx::from_int(10000), ai: false }, PlayerSetup { side: "CORE".into(), color: 1, start: FxVec3::ZERO, metal: Fx::from_int(10000), energy: Fx::from_int(10000), ai: false }];
        let map = TntMap { width: 128, height: 128, sea_level: 150, heights: vec![60; 128 * 128], features: vec![FEATURE_NONE; 128 * 128], tiles: vec![], tile_map: vec![], feature_names: vec![], minimap: Default::default() };
        let mut sim = Sim::new(types.clone(), data.clone(), &map, &MapInfo::default(), 0, &setup, 73);
        sim.los_enabled = false;
        let shooter = sim.spawn_unit(types.id(source).unwrap(), 0, FxVec3::from_ints(600, 150, 1000), 16384, true).unwrap();
        let victim = sim.spawn_unit(types.id(target).unwrap(), 1, FxVec3::from_ints(850, 150, 1000), 0, true).unwrap();
        let start = sim.unit(victim).unwrap().health;
        sim.unit_mut(victim).unwrap().fire_state = 0;
        sim.unit_mut(victim).unwrap().move_state = 0;
        if sim.unit_type(sim.unit(shooter).unwrap()).locomotion == Locomotion::Air {
            let alt = sim.unit_type(sim.unit(shooter).unwrap()).cruise_alt;
            sim.unit_mut(shooter).unwrap().pos.y += alt;
        }
        sim.give_orders(shooter, [Order::Attack(Target::Unit(victim))]);
        let mut fired = false;
        let mut hit = false;
        for _ in 0..900 {
            sim.step(&[]);
            fired |= sim.take_events().iter().any(|e| matches!(e, SimEvent::WeaponFired { unit, weapon, .. } if *unit == shooter && sim.weapon_types.get(*weapon).water));
            hit |= sim.unit(victim).is_none_or(|u| u.health < start);
            if fired && hit { break; }
        }
        assert!(fired && hit, "{source} must fire and damage {target}: fired {fired}, hit {hit}; shooter {:?}; projectiles {:?}", sim.unit(shooter).map(|u| (u.pos, &u.orders)), sim.projectiles.iter().map(|p| (p.pos, p.vel, p.age)).collect::<Vec<_>>());
        println!("{source} hit {target} with its original weapon/script");
    }
}
