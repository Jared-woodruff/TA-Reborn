//! Fleet duties honor repairs and actual weapon/target capabilities.

use super::*;

fn sea(torpedoes: bool) -> Sim {
    let base = fixture_edit(|data| {
        let tank = data.unit_id("TANK").unwrap();
        data.units[tank].floater = true;
        data.units[tank].min_water_depth = 10.0;
        data.units[tank].max_water_depth = 255.0;
        data.units[tank].build_cost_metal = 5000.0;
        let gun = data.weapon_id("GUN").unwrap();
        data.weapons[gun].water_weapon = torpedoes;
    });
    let mut heights = vec![20; 4096];
    for z in 0..64 { for x in 48..64 { heights[z * 64 + x] = 150; } }
    let map = TntMap { width: 64, height: 64, sea_level: 120, heights, features: vec![FEATURE_NONE; 4096], tiles: vec![], tile_map: vec![], feature_names: vec![], minimap: Default::default(), flags: Vec::new() };
    let players: Vec<_> = [(200, 300), (820, 300)].iter().enumerate().map(|(i, &(x, z))| PlayerSetup { side: "ARM".into(), color: i as u8, start: FxVec3::from_ints(x, 120, z), metal: fx(1000), energy: fx(1000), ai: true }).collect();
    let mut sim = Sim::new(base.types.clone(), base.data.clone(), &map, &MapInfo::default(), 0, &players, 7);
    sim.los_enabled = false;
    sim
}

fn ship(sim: &mut Sim, owner: u8, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id("TANK").unwrap(), owner, FxVec3::from_ints(x, 120, z), 0, true).unwrap()
}

#[test]
fn a_ship_reserved_for_repairs_is_not_reassigned_to_defence() {
    let mut sim = sea(false);
    let reserved = ship(&mut sim, 0, 300, 300);
    let other = ship(&mut sim, 0, 350, 400);
    ship(&mut sim, 1, 600, 300);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.repairing.push((reserved, 0));
    let mut out = Vec::new();
    ai.run_navy(&sim, &ai.survey(&sim), &mut out);
    assert!(out.iter().any(|c| matches!(c, Command::Order { units, .. } if units.contains(&other))), "the available ship should still defend");
    assert!(!out.iter().any(|c| matches!(c, Command::Order { units, .. } if units.contains(&reserved))), "fleet orders must not overwrite a repair retreat");
}

#[test]
fn a_torpedo_fleet_does_not_commit_to_bombarding_dry_buildings() {
    let mut sim = sea(true);
    for (x, z) in [(250, 250), (300, 300), (350, 350)] { ship(&mut sim, 0, x, z); }
    let at = FxVec3::from_ints(820, 150, 300);
    let kind = sim.types.id("EXTRACTOR").unwrap();
    let target = sim.spawn_unit(kind, 1, at, 0, true).unwrap();
    sim.tick = 5 * MINUTE;
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.enemy_buildings.push((target, at, kind));
    let post = FxVec3::from_ints(200, 120, 300);
    ai.naval = Some(crate::navy::Naval { yard: sim.types.id("FACTORY").unwrap(), site: post, post, reaches: true });
    let mut out = Vec::new();
    ai.run_navy(&sim, &ai.survey(&sim), &mut out);
    assert!(!out.is_empty(), "the fleet should still scout or gather");
    assert!(!out.iter().any(|c| matches!(c, Command::Order { order: Order::Fight(p), .. } if *p == at)), "torpedoes cannot justify a dry-land bombardment");
}

#[test]
fn a_torpedo_only_shipyard_cannot_justify_attacking_a_dry_base() {
    let sim = sea(true);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let yard = sim.types.id("FACTORY").unwrap();
    let site = FxVec3::from_ints(300, 120, 300);
    ai.enemy_bases = vec![FxVec3::from_ints(820, 150, 300)];
    assert!(!ai.yard_reaches(&sim, yard, site), "torpedo range cannot justify a shipyard for inland bombardment");
    ai.enemy_bases = vec![FxVec3::from_ints(600, 120, 300)];
    assert!(ai.yard_reaches(&sim, yard, site), "the same yard can still fight at a water base");
}

#[test]
fn the_naval_waiting_post_is_reachable_open_water_off_the_shipyard() {
    let base = fixture_edit(|data| {
        let builder = data.unit_id("BUILDER").unwrap();
        data.units[builder].is_commander = true;
        let yard = data.unit_id("FACTORY").unwrap();
        data.units[yard].floater = true;
        data.units[yard].min_water_depth = 10.0;
        data.units[yard].build_options.push("BOMBER".into());
        let ship = data.unit_id("TANK").unwrap();
        data.units[ship].floater = true;
        data.units[ship].min_water_depth = 10.0;
        data.units[ship].max_water_depth = 255.0;
        data.units[ship].footprint_x = 4;
        data.units[ship].footprint_z = 4;
        let sub = data.unit_id("BOMBER").unwrap();
        data.units[sub].can_fly = false;
        data.units[sub].floater = true;
        data.units[sub].min_water_depth = 10.0;
        data.units[sub].max_water_depth = 255.0;
        data.units[sub].water_line = 40.0;
    });
    let heights = (0..4096).map(|i| if i / 64 < 24 { 20 } else if i / 64 < 30 { 100 } else { 150 }).collect();
    let map = TntMap { width: 64, height: 64, sea_level: 120, heights, features: vec![FEATURE_NONE; 4096], tiles: vec![], tile_map: vec![], feature_names: vec![], minimap: Default::default(), flags: Vec::new() };
    let setups: Vec<_> = [(200, 650), (800, 650)].iter().enumerate().map(|(i, &(x, z))| PlayerSetup { side: "ARM".into(), color: i as u8, start: FxVec3::from_ints(x, 150, z), metal: fx(1000), energy: fx(1000), ai: true }).collect();
    let mut sim = Sim::new(base.types.clone(), base.data.clone(), &map, &MapInfo::default(), 0, &setups, 7);
    sim.spawn_unit(sim.types.id("BUILDER").unwrap(), 0, setups[0].start, 0, true).unwrap();
    sim.tick = MINUTE;
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.plan_navy(&sim, &ai.survey(&sim));
    let plan = ai.naval.as_ref().expect("a builder can build the shipyard along this coast");
    for name in ["TANK", "BOMBER"] {
        let spec = sim.move_spec(sim.types.id(name).unwrap());
        assert!(sim.world.passable(&spec, World::pos_to_anchor(plan.post, spec.footprint), None), "the waiting post must fit {name}'s footprint and draft: site {:?}, post {:?}", plan.site, plan.post);
    }
}
