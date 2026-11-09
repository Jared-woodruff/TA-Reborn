//! Installed building definitions and COB programs; skip without an original install.

use super::*;
use super::air_original::{field, original};

#[test]
fn original_buildings_finish_activate_and_keep_their_authored_yards() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let mut checked = 0;
    for t in types.types.iter().filter(|t| t.locomotion == Locomotion::Static) {
        let mut sim = field(data, types, t.def.min_water_depth > 0.0);
        let id = sim.spawn_unit(t.id, 0, FxVec3::from_ints(800, 60, 800), 0, false).unwrap();
        for _ in 0..15 { sim.step(&[]); }
        assert_eq!(sim.unit_flows(sim.unit(id).unwrap()), crate::flows::UnitFlows::default(), "{} cannot work before completion", t.name);
        sim.finish_frame(id);
        for _ in 0..180 { sim.step(&[]); }
        let u = sim.unit(id).unwrap();
        assert!(u.is_complete() && !u.dead && u.speed.is_zero(), "{} must finish and stay stationary", t.name);
        let (x, z) = u.occupied_origin.unwrap();
        let cells = if u.yard_open { &t.open_yard_cells } else { &t.blocking_cells };
        for dz in 0..t.footprint.1 { for dx in 0..t.footprint.0 {
            let expected = if cells[(dz * t.footprint.0 + dx) as usize] { Occupant::Unit(id.slot) } else { Occupant::Free };
            assert_eq!(sim.world.occupant(x + dx as i32, z + dz as i32), expected, "{} yard cell ({dx}, {dz})", t.name);
        } }
        if t.def.on_offable {
            sim.step(&[Command::SetActive { player: 0, units: vec![id], on: false }]);
            for _ in 0..90 { sim.step(&[]); }
            assert!(!sim.unit(id).unwrap().activated, "{} must switch off", t.name);
            sim.step(&[Command::SetActive { player: 0, units: vec![id], on: true }]);
            for _ in 0..90 { sim.step(&[]); }
            assert!(sim.unit(id).unwrap().activated, "{} must switch on", t.name);
        }
        checked += 1;
    }
    assert!(checked >= 100, "expected the original building roster, checked {checked}");
    println!("{checked} original buildings completed, ran their scripts and retained their yards");
}

#[test]
fn original_solar_collectors_close_when_hit_then_resume_production() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    for name in ["ARMSOLAR", "CORSOLAR"] {
        let mut sim = field(data, types, false);
        let id = sim.spawn_unit(types.id(name).unwrap(), 0, FxVec3::from_ints(800, 60, 800), 0, true).unwrap();
        for _ in 0..180 { sim.step(&[]); }
        assert!(sim.unit_flows(sim.unit(id).unwrap()).energy_make > Fx::ZERO, "{name} starts producing");
        sim.damage_unit(id, Fx::from_int(10), id, FxVec3::ZERO);
        let mut closed = false;
        let mut recovered = false;
        for _ in 0..900 {
            sim.step(&[]);
            let u = sim.unit(id).unwrap();
            closed |= u.armored && sim.unit_flows(u).energy_make.is_zero();
            recovered |= closed && !u.armored && sim.unit_flows(u).energy_make > Fx::ZERO;
            if recovered { break; }
        }
        assert!(closed && recovered, "{name} must stop output while armored, then reopen and resume");
    }
}
