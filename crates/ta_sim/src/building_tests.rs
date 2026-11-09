//! Building lifecycle, yard maps, power and sensors, without original assets.

use super::*;

fn buildings() -> Sim {
    let mut sim = sim_of(data_of(&[
        unit("GEO", "FootprintX=2; FootprintZ=2; YardMap=GGGG; EnergyMake=250;"),
        unit("YARD", "FootprintX=3; FootprintZ=2; YardMap=ocyioc;"),
        unit("WALL", "FootprintX=1; FootprintZ=1;"),
        unit("RADAR", "SightDistance=32; RadarDistance=700; SonarDistance=700; RadarDistanceJam=700; SonarDistanceJam=700; EnergyUse=30; ActivateWhenBuilt=1; OnOffable=1;"),
        unit("MAKER", "EnergyMake=30; MakesMetal=1; EnergyUse=3; MetalStorage=100; EnergyStorage=200; ActivateWhenBuilt=1; OnOffable=1;"),
        unit("TARGET", "SightDistance=32; CanMove=1; MaxVelocity=1;"),
    ]), 2);
    for p in &mut sim.players { p.endless = true; }
    sim
}

fn put(sim: &mut Sim, name: &str, x: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), 0, FxVec3::from_ints(x, 60, 300), 0, true).unwrap()
}

fn scripted_yard(sim: &mut Sim) -> UnitId {
    let t = sim.types.id("YARD").unwrap();
    Arc::get_mut(&mut sim.types).unwrap().types[t].script = Some(Arc::new(crate::cob::CobProgram::new(Default::default(), &[])));
    put(sim, "YARD", 300)
}

fn yard_switch(sim: &mut Sim, id: UnitId, open: bool) {
    sim.with_script(id, |_, host| host.set_value(crate::host::value::YARD_OPEN, open as i32));
}

#[test]
fn geothermal_plants_are_solid_and_reserve_their_site() {
    let mut sim = buildings();
    let id = put(&mut sim, "GEO", 300);
    let u = sim.unit(id).unwrap();
    let origin = u.occupied_origin.unwrap();
    assert_eq!(sim.world.occupant(origin.0, origin.1), Occupant::Unit(id.slot));
    assert!(!sim.can_place(sim.types.id("WALL").unwrap(), u.pos), "a geothermal plant cannot be built through");
}

#[test]
fn script_yard_state_changes_movement_and_invalidates_routes() {
    let mut sim = buildings();
    let id = scripted_yard(&mut sim);
    let (x, z) = sim.unit(id).unwrap().occupied_origin.unwrap();
    assert_eq!(sim.world.occupant(x + 1, z), Occupant::Unit(id.slot), "c is blocked while closed");
    assert_eq!(sim.world.occupant(x, z + 1), Occupant::Free, "i is free while closed");
    let before = sim.world.occupancy_version;
    yard_switch(&mut sim, id, true);
    assert_eq!(sim.world.occupant(x + 1, z), Occupant::Free, "c opens for movement");
    assert_eq!(sim.world.occupant(x, z + 1), Occupant::Unit(id.slot), "i closes when the yard opens");
    assert!(sim.world.occupancy_version > before, "path caches must see the change");
    let opened = sim.world.occupancy_version;
    yard_switch(&mut sim, id, true);
    assert_eq!(sim.world.occupancy_version, opened, "repeating the state does not invalidate routes again");
    yard_switch(&mut sim, id, false);
    assert_eq!(sim.world.occupant(x + 1, z), Occupant::Unit(id.slot));
    assert_eq!(sim.world.occupant(x + 2, z), Occupant::Free, "y stays free");
}

#[test]
fn an_open_yard_allows_movement_but_keeps_its_construction_reservation() {
    let mut sim = buildings();
    let id = scripted_yard(&mut sim);
    let (x, z) = sim.unit(id).unwrap().occupied_origin.unwrap();
    yard_switch(&mut sim, id, true);
    let t = sim.types.get(sim.types.id("WALL").unwrap());
    assert_eq!(sim.world.placement_error(t, 10, (x + 1, z)), Some(crate::world::PlacementError::Occupied));
    assert_eq!(sim.world.placement_error(t, 10, (x + 2, z)), None, "authored y cells stay buildable");
    let mut copy = sim.clone();
    copy.remove_unit(id);
    copy.remove_dead();
    assert_eq!(copy.world.placement_error(t, 10, (x + 1, z)), None, "removal frees both maps");
    assert_eq!(sim.world.placement_error(t, 10, (x + 1, z)), Some(crate::world::PlacementError::Occupied), "clone has independent reservations");
}

#[test]
fn unpaid_or_paralyzed_sensors_stop_coverage_and_resume_after_recovery() {
    let mut sim = buildings();
    let radar = put(&mut sim, "RADAR", 200);
    let target = sim.spawn_unit(sim.types.id("TARGET").unwrap(), 1, FxVec3::from_ints(700, 60, 300), 0, true).unwrap();
    sim.players[0].economy.energy = Fx::ZERO;
    sim.economy_end();
    sim.rebuild_vision();
    assert_eq!(sim.seen_by(sim.unit(target).unwrap(), 0), Seen::Hidden, "unpaid radar cannot reveal a distant target");
    let advanced = crate::vision::RADAR | crate::vision::SONAR | crate::vision::RADAR_JAM | crate::vision::SONAR_JAM;
    let at = sim.unit(target).unwrap().pos;
    assert_eq!(sim.vision.flags_at(0, at) & advanced, 0);
    sim.players[0].economy.energy = Fx::from_int(100);
    sim.economy_end();
    sim.rebuild_vision();
    assert_eq!(sim.seen_by(sim.unit(target).unwrap(), 0), Seen::Radar);
    assert_eq!(sim.vision.flags_at(0, at) & advanced, advanced);
    sim.unit_mut(radar).unwrap().paralyzed_until = sim.tick + 10;
    sim.rebuild_vision();
    assert_eq!(sim.seen_by(sim.unit(target).unwrap(), 0), Seen::Hidden, "EMP disables sensors");
    assert_eq!(sim.vision.flags_at(0, at) & advanced, 0);
    sim.unit_mut(radar).unwrap().paralyzed_until = sim.tick;
    sim.rebuild_vision();
    assert_eq!(sim.seen_by(sim.unit(target).unwrap(), 0), Seen::Radar);
    sim.set_active(radar, false);
    sim.rebuild_vision();
    assert_eq!(sim.vision.flags_at(0, at) & advanced, 0);
    assert_ne!(sim.vision.flags_at(0, sim.unit(radar).unwrap().pos) & crate::vision::LOS, 0, "switching off sensors preserves optical sight");
}

#[test]
fn paralyzed_buildings_keep_storage_but_stop_income_and_upkeep() {
    let mut sim = buildings();
    let maker = put(&mut sim, "MAKER", 300);
    sim.unit_mut(maker).unwrap().paralyzed_until = 10;
    let before = (sim.players[0].economy.metal, sim.players[0].economy.energy);
    sim.economy_end();
    let e = &sim.players[0].economy;
    assert_eq!((e.metal, e.energy), before, "a disabled plant cannot produce or charge upkeep");
    assert_eq!((e.metal_capacity, e.energy_capacity), (Fx::from_int(BASE_STORAGE + 100), Fx::from_int(BASE_STORAGE + 200)));
    assert_eq!(sim.unit_flows(sim.unit(maker).unwrap()), crate::flows::UnitFlows::default());
    sim.tick = 10;
    sim.economy_end();
    assert!(sim.players[0].economy.metal > before.0 && sim.players[0].economy.energy > before.1);
}

#[test]
fn mixed_on_off_commands_do_not_activate_unswitchable_or_enemy_buildings() {
    let mut sim = buildings();
    let wall = put(&mut sim, "WALL", 200);
    let radar = put(&mut sim, "RADAR", 300);
    let enemy = sim.spawn_unit(sim.types.id("RADAR").unwrap(), 1, FxVec3::from_ints(700, 60, 300), 0, true).unwrap();
    sim.apply(&Command::SetActive { player: 0, units: vec![wall, radar, enemy], on: false });
    sim.apply(&Command::SetActive { player: 0, units: vec![wall, radar], on: true });
    assert!(!sim.unit(wall).unwrap().activated, "only OnOffable units accept the command");
    assert!(sim.unit(radar).unwrap().activated && sim.unit(enemy).unwrap().activated);
}

#[test]
fn checksums_cover_building_armor_upkeep_and_script_busy_state() {
    let mut sim = buildings();
    let id = put(&mut sim, "MAKER", 300);
    let expected = sim.checksum();
    for field in 0..3 {
        let mut changed = sim.clone();
        let u = changed.unit_mut(id).unwrap();
        match field { 0 => u.armored = !u.armored, 1 => u.upkeep_paid = !u.upkeep_paid, _ => u.busy = !u.busy }
        assert_ne!(changed.checksum(), expected, "building state {field} must affect lockstep");
    }
}

#[test]
fn script_activation_runs_each_callback_once_per_transition() {
    use ta_formats::cob::{CobFunction, CobScript, op};
    let mut sim = buildings();
    let t = sim.types.id("WALL").unwrap();
    let mut script = CobScript { static_var_count: 2, ..Default::default() };
    for (name, counter) in [("Activate", 0), ("Deactivate", 1)] {
        script.functions.push(CobFunction { name: name.into(), offset: script.code.len() as u32 });
        script.code.extend([op::PUSH_STATIC, counter, op::PUSH_CONSTANT, 1, op::ADD, op::POP_STATIC, counter, op::PUSH_CONSTANT, 0, op::RETURN]);
    }
    Arc::get_mut(&mut sim.types).unwrap().types[t].script = Some(Arc::new(crate::cob::CobProgram::new(script, &[])));
    let id = put(&mut sim, "WALL", 300);
    for on in [true, true, false, false, true] {
        sim.with_script(id, |_, host| host.set_value(crate::host::value::ACTIVATION, on as i32));
        sim.with_script(id, |cob, host| cob.run(host));
    }
    assert_eq!(sim.unit(id).unwrap().cob.as_ref().unwrap().statics, [2, 1]);
}

#[test]
fn storage_moves_to_the_new_owner_and_disappears_on_removal() {
    let mut sim = buildings();
    let id = put(&mut sim, "MAKER", 300);
    sim.economy_end();
    assert_eq!(sim.players[0].economy.energy_capacity, Fx::from_int(BASE_STORAGE + 200));
    sim.transfer_unit(id, 1);
    sim.economy_end();
    assert_eq!(sim.players[0].economy.energy_capacity, Fx::from_int(BASE_STORAGE));
    assert_eq!(sim.players[1].economy.energy_capacity, Fx::from_int(BASE_STORAGE + 200));
    sim.remove_unit(id);
    sim.step(&[]);
    assert_eq!(sim.players[1].economy.energy_capacity, Fx::from_int(BASE_STORAGE));
    assert!(sim.can_place(sim.types.id("WALL").unwrap(), FxVec3::from_ints(300, 60, 300)));
}

#[test]
fn building_scripts_and_reservations_continue_identically_from_a_clone() {
    let mut sim = buildings();
    let yard = scripted_yard(&mut sim);
    let maker = put(&mut sim, "MAKER", 600);
    yard_switch(&mut sim, yard, true);
    sim.unit_mut(maker).unwrap().paralyzed_until = 12;
    let mut copy = sim.clone();
    for _ in 0..40 {
        sim.step(&[]);
        copy.step(&[]);
        assert_eq!(sim.checksum(), copy.checksum());
    }
    let (x, z) = sim.unit(yard).unwrap().occupied_origin.unwrap();
    let t = sim.types.get(sim.types.id("WALL").unwrap());
    assert_eq!(sim.world.placement_error(t, 10, (x + 1, z)), copy.world.placement_error(t, 10, (x + 1, z)));
}

#[test]
fn building_script_position_queries_use_the_same_physical_height_in_both_runners() {
    use ta_formats::cob::{CobFunction, CobScript, op};
    let data = data_of(&[unit("READER", ""), unit("SUB", "CanMove=1; MaxVelocity=1; MinWaterDepth=20; WaterLine=20;")]);
    let mut map = flat_map();
    map.sea_level = 120;
    let mut sim = sim_on_map(data, 1, &map);
    let sub = sim.spawn_unit(sim.types.id("SUB").unwrap(), 0, FxVec3::from_ints(500, 120, 300), 0, true).unwrap();
    let script = CobScript { static_var_count: 1, functions: vec![CobFunction { name: "Read".into(), offset: 0 }], code: vec![
        op::PUSH_CONSTANT, crate::host::value::UNIT_Y as u32, op::PUSH_CONSTANT, sub.script_id() as u32,
        op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 0, op::GET, op::POP_STATIC, 0,
        op::PUSH_CONSTANT, 0, op::RETURN,
    ], ..Default::default() };
    let t = sim.types.id("READER").unwrap();
    Arc::get_mut(&mut sim.types).unwrap().types[t].script = Some(Arc::new(crate::cob::CobProgram::new(script, &[])));
    let reader = put(&mut sim, "READER", 300);
    let expected = (sim.world.sea_level - Fx::from_int(20)).to_cob();
    sim.call_script(reader, "Read", &[]);
    assert_eq!(sim.unit(reader).unwrap().cob.as_ref().unwrap().statics[0], expected);
    sim.unit_mut(reader).unwrap().cob.as_mut().unwrap().statics[0] = 0;
    sim.start_script(reader, "Read", &[]);
    sim.run_scripts(&sim.types.clone());
    assert_eq!(sim.unit(reader).unwrap().cob.as_ref().unwrap().statics[0], expected);
}
