//! Skirmish decision regressions using synthetic data rather than original assets.

use super::*;

fn put(sim: &mut Sim, name: &str, owner: u8, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), owner, FxVec3::from_ints(x, 60, z), 0, true).unwrap()
}

fn ocean(base: Sim, seam: bool) -> Sim {
    let heights = (0..4096).map(|i| if !seam || (32..48).contains(&(i % 64)) { 0 } else { 60 }).collect();
    let map = TntMap { width: 64, height: 64, sea_level: 50, heights, features: vec![FEATURE_NONE; 4096], tiles: vec![], tile_map: vec![], feature_names: vec![], minimap: Default::default() };
    let setup: Vec<_> = (0..2).map(|color| PlayerSetup { side: "ARM".into(), color, start: FxVec3::ZERO, metal: fx(1000), energy: fx(1000), ai: false }).collect();
    let mut sim = Sim::new(base.types.clone(), base.data.clone(), &map, &MapInfo::default(), 0, &setup, 7);
    sim.los_enabled = false;
    for p in &mut sim.players { p.endless = true; }
    sim
}

fn attacks(out: &[Command], shooter: UnitId, target: UnitId) -> bool {
    out.iter().any(|c| matches!(c, Command::Order { units, order: Order::Attack(Target::Unit(t)), .. } if units.contains(&shooter) && *t == target))
}

#[test]
fn a_scouts_new_tour_is_not_overwritten_by_the_same_thinks_attack_wave() {
    let mut sim = fixture();
    for k in 0..6 { put(&mut sim, "TANK", 0, 150 + k * 20, 200); }
    sim.tick = MINUTE;
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.enemy_bases = vec![FxVec3::from_ints(900, 60, 900)];
    ai.enemy_base_seen = vec![false];
    ai.enemy_base_owners = vec![u8::MAX];
    ai.p.wave_base = 1;
    ai.p.wave_growth = 0;
    ai.p.wave_cooldown = 0;
    let s = ai.survey(&sim);
    let mut out = Vec::new();
    ai.run_scouting(&sim, &s, &mut out);
    let scout = ai.ground_scout.expect("one unit should scout");
    ai.run_army(&sim, &s, &mut out);
    assert!(out.iter().any(|c| matches!(c, Command::Order { units, order: Order::Fight(_), .. } if units.iter().any(|&id| id != scout))), "the remaining army should still attack");
    assert!(!out.iter().any(|c| matches!(c, Command::Order { units, order: Order::Fight(_) | Order::Attack(_), .. } if units.contains(&scout))), "the scout keeps its tour");
}

#[test]
fn mission_scripted_planes_are_not_recruited_as_scouts() {
    let mut sim = fixture_edit(|d| { let t = d.unit_id("BOMBER").unwrap(); d.units[t].weapons = [None, None, None]; });
    let plane = put(&mut sim, "BOMBER", 0, 200, 200);
    sim.give_orders(plane, [Order::Wait(10000)]);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.enemy_bases = vec![FxVec3::from_ints(900, 60, 900)];
    ai.enemy_base_seen = vec![false];
    ai.enemy_base_owners = vec![u8::MAX];
    let mut out = Vec::new();
    ai.run_scouting(&sim, &ai.survey(&sim), &mut out);
    assert!(ai.ground_scout.is_none() && out.is_empty(), "mission orders must retain control of the plane");
}

#[test]
fn loaded_units_remain_owned_but_are_not_available_for_combat_orders() {
    let mut sim = fixture_edit(|d| {
        let b = d.unit_id("BUILDER").unwrap();
        d.units[b].transport_capacity = 2;
        d.units[b].can_load = true;
    });
    for p in &mut sim.players { p.endless = true; }
    let transport = put(&mut sim, "BUILDER", 0, 200, 300);
    let cargo = put(&mut sim, "TANK", 0, 220, 300);
    sim.give_orders(transport, [Order::Load(cargo)]);
    for _ in 0..180 {
        sim.step(&[]);
        if sim.unit(cargo).unwrap().carried_by.is_some() { break; }
    }
    assert_eq!(sim.unit(cargo).unwrap().carried_by, Some(transport));
    let ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let s = ai.survey(&sim);
    assert!(s.own.iter().any(|u| u.id == cargo));
    assert!(!s.army.iter().any(|u| u.id == cargo), "a passenger cannot be counted as a ready combat unit");
}

#[test]
fn torpedoes_do_not_receive_concentrated_fire_orders_against_dry_targets() {
    let mut sim = fixture_edit(|d| { let w = d.weapon_id("GUN").unwrap(); d.weapons[w].water_weapon = true; });
    sim.los_enabled = false;
    let shooter = put(&mut sim, "TANK", 0, 200, 300);
    let target = put(&mut sim, "TANK", 1, 300, 300);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let mut out = Vec::new();
    ai.focus_fire(&sim, &ai.survey(&sim), &mut out);
    assert!(!attacks(&out, shooter, target));
}

#[test]
fn surface_guns_do_not_receive_concentrated_fire_orders_against_submarines() {
    let base = fixture_edit(|d| {
        for name in ["TANK", "BOMBER"] {
            let t = d.unit_id(name).unwrap();
            d.units[t].can_fly = false;
            d.units[t].floater = true;
            d.units[t].min_water_depth = 10.0;
            d.units[t].weapons = [Some("GUN".into()), None, None];
            if name == "BOMBER" { d.units[t].water_line = 20.0; }
        }
    });
    let mut sim = ocean(base, false);
    let shooter = put(&mut sim, "TANK", 0, 200, 300);
    let target = put(&mut sim, "BOMBER", 1, 300, 300);
    assert!(sim.submerged(sim.unit(target).unwrap()));
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let mut out = Vec::new();
    ai.focus_fire(&sim, &ai.survey(&sim), &mut out);
    assert!(!attacks(&out, shooter, target));
}

#[test]
fn concentrated_fire_uses_the_range_of_a_compatible_weapon() {
    let mut sim = fixture_edit(|d| {
        let gun = d.weapon_id("GUN").unwrap();
        d.weapons[gun].water_weapon = true;
        d.weapons[gun].range = 600.0;
        let short = d.weapon_id("BOMB").unwrap();
        d.weapons[short].dropped = false;
        d.weapons[short].command_fire = false;
        d.weapons[short].range = 100.0;
        let tank = d.unit_id("TANK").unwrap();
        d.units[tank].weapons[1] = Some("BOMB".into());
        let target = d.unit_id("BUILDER").unwrap();
        d.units[target].weapons[0] = Some("BOMB".into());
    });
    sim.los_enabled = false;
    let shooter = put(&mut sim, "TANK", 0, 200, 300);
    let target = put(&mut sim, "BUILDER", 1, 450, 300);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let mut out = Vec::new();
    ai.focus_fire(&sim, &ai.survey(&sim), &mut out);
    assert!(!attacks(&out, shooter, target), "long torpedo range cannot stand in for the short surface gun");
}

#[test]
fn repair_assignments_choose_a_reachable_builder_across_a_water_barrier() {
    let mut sim = ocean(fixture(), true);
    let target = put(&mut sim, "TANK", 0, 850, 300);
    let blocked = put(&mut sim, "BUILDER", 0, 480, 300);
    let reachable = put(&mut sim, "BUILDER", 0, 850, 850);
    sim.set_health(target, fx(100));
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.rally = sim.unit(target).unwrap().pos;
    ai.repairing.push((target, 0));
    let mut out = Vec::new();
    ai.run_repairs(&sim, &ai.survey(&sim), &mut out);
    assert!(out.iter().any(|c| matches!(c, Command::Order { units, order: Order::Repair(t), .. } if *t == target && units.contains(&reachable))), "the reachable helper must get the job");
    assert!(!out.iter().any(|c| matches!(c, Command::Order { units, order: Order::Repair(_), .. } if units.contains(&blocked))));
}

#[test]
fn construction_aircraft_can_be_assigned_to_repair_a_ship() {
    let base = fixture_edit(|d| {
        let b = d.unit_id("BUILDER").unwrap();
        d.units[b].can_fly = true;
        let t = d.unit_id("TANK").unwrap();
        d.units[t].floater = true;
        d.units[t].min_water_depth = 10.0;
    });
    let mut sim = ocean(base, false);
    let target = put(&mut sim, "TANK", 0, 400, 300);
    let helper = put(&mut sim, "BUILDER", 0, 350, 300);
    sim.set_health(target, fx(100));
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let post = sim.unit(target).unwrap().pos;
    ai.naval = Some(crate::navy::Naval { yard: sim.types.id("FACTORY").unwrap(), site: post, post, reaches: true });
    ai.repairing.push((target, 0));
    let mut out = Vec::new();
    ai.run_repairs(&sim, &ai.survey(&sim), &mut out);
    assert!(out.iter().any(|c| matches!(c, Command::Order { units, order: Order::Repair(t), .. } if *t == target && units.contains(&helper))));
}

#[test]
fn outbound_air_strikes_keep_their_ledger_until_the_queued_attack_finishes() {
    let mut sim = fixture();
    let plane = put(&mut sim, "BOMBER", 0, 200, 200);
    let target = put(&mut sim, "EXTRACTOR", 1, 900, 900);
    let at = sim.unit(target).unwrap().pos;
    sim.give_orders(plane, [Order::Move(at), Order::Attack(Target::Unit(target))]);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.enemy_buildings.push((target, at, sim.unit(target).unwrap().type_id));
    ai.air_strike = Some(crate::army::AirStrike { targets: vec![(target, 100)], planes: vec![(plane, 100)], started: 0 });
    ai.run_air(&sim, &ai.survey(&sim), &mut Vec::new());
    assert!(ai.air_strike.is_some(), "an outbound approach near home is still part of the strike");
}

#[test]
fn units_now_known_to_be_friendly_are_removed_from_enemy_army_memory() {
    let mut sim = fixture();
    sim.los_enabled = false;
    let enemy = put(&mut sim, "TANK", 1, 700, 700);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.remember(&sim, &ai.survey(&sim));
    assert!(ai.enemy_army.iter().any(|a| a.0 == enemy));
    sim.transfer_unit(enemy, 0);
    ai.remember(&sim, &ai.survey(&sim));
    assert!(!ai.enemy_army.iter().any(|a| a.0 == enemy), "captured units are known friendly immediately");
}

#[test]
fn repeated_ally_requests_do_not_send_more_resources_than_the_recipient_can_store() {
    let mut sim = fixture_players(3, |_| {});
    sim.set_team(1, 0);
    sim.set_starting_resources(0, fx(3000), fx(1000));
    sim.players[1].economy.metal = fx(900);
    sim.tick = 15;
    for _ in 0..2 { sim.signals.push(ta_sim::Signal { player: 1, kind: SignalKind::NeedMetal, at: FxVec3::ZERO, tick: 0 }); }
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let mut out = Vec::new();
    ai.hear_allies(&sim, &ai.survey(&sim), &mut out);
    let given: Fx = out.iter().filter_map(|c| match c { Command::Share { to: 1, metal, .. } => Some(*metal), _ => None }).fold(Fx::ZERO, |a, b| a + b);
    assert_eq!(given, fx(100), "pending gifts reserve recipient capacity within the think");
}

#[test]
fn a_former_allys_requests_are_discarded_before_military_orders_are_planned() {
    let mut sim = fixture_players(3, |_| {});
    sim.set_team(1, 0);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let request = crate::ally::Request { at: FxVec3::from_ints(700, 60, 700), until: MINUTE, from: 1, answered: false };
    ai.requests.attack = Some(request);
    ai.requests.defend = Some(request);
    sim.players[1].defeated = true;
    ai.hear_allies(&sim, &ai.survey(&sim), &mut Vec::new());
    assert!(ai.requests.attack.is_none() && ai.requests.defend.is_none());
}

#[test]
fn scripted_metal_makers_are_left_alone_in_a_mixed_economy() {
    let mut sim = fixture_edit(|d| {
        let t = d.unit_id("EXTRACTOR").unwrap();
        d.units[t].extracts_metal = 0.0;
        d.units[t].makes_metal = 1.0;
        d.units[t].energy_use = 60.0;
        d.units[t].on_offable = true;
        d.units[t].activate_when_built = true;
    });
    let scripted = put(&mut sim, "EXTRACTOR", 0, 200, 300);
    let free = put(&mut sim, "EXTRACTOR", 0, 300, 300);
    sim.give_orders(scripted, [Order::Wait(10000)]);
    sim.players[0].economy.energy = Fx::ZERO;
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let commands = ai.think(&sim);
    assert!(commands.iter().any(|c| matches!(c, Command::SetActive { units, on: false, .. } if units.contains(&free))));
    assert!(!commands.iter().any(|c| matches!(c, Command::SetActive { units, .. } if units.contains(&scripted))), "the script retains control of its metal maker");
}

#[test]
fn an_air_scouts_new_tour_is_not_overwritten_by_the_air_wings_strike() {
    let mut sim = fixture();
    for k in 0..3 { put(&mut sim, "BOMBER", 0, 200 + 20 * k, 200); }
    let target = put(&mut sim, "EXTRACTOR", 1, 900, 900);
    sim.tick = MINUTE;
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.enemy_bases = vec![sim.unit(target).unwrap().pos];
    ai.enemy_base_seen = vec![false];
    ai.enemy_base_owners = vec![u8::MAX];
    ai.enemy_buildings.push((target, sim.unit(target).unwrap().pos, sim.unit(target).unwrap().type_id));
    let s = ai.survey(&sim);
    let mut out = Vec::new();
    ai.run_scouting(&sim, &s, &mut out);
    let scout = ai.ground_scout.expect("a spare bomber can scout");
    ai.run_air(&sim, &s, &mut out);
    assert!(!attacks(&out, scout, target), "the air wing cannot take back a scout's pending tour");
}

#[test]
fn compatible_torpedoes_still_receive_concentrated_fire_orders() {
    let base = fixture_edit(|d| {
        let t = d.unit_id("TANK").unwrap();
        d.units[t].floater = true;
        d.units[t].min_water_depth = 10.0;
        let w = d.weapon_id("GUN").unwrap();
        d.weapons[w].water_weapon = true;
    });
    let mut sim = ocean(base, false);
    let shooter = put(&mut sim, "TANK", 0, 200, 300);
    let target = put(&mut sim, "TANK", 1, 300, 300);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let mut out = Vec::new();
    ai.focus_fire(&sim, &ai.survey(&sim), &mut out);
    assert!(attacks(&out, shooter, target));
}

#[test]
fn cloned_ai_and_simulation_continue_with_identical_commands_on_every_difficulty() {
    for level in [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard] {
        let mut sim = fixture_with_starts(&[(150, 150), (850, 850)]);
        for p in &mut sim.players { p.endless = true; }
        put(&mut sim, "BUILDER", 0, 200, 250);
        for k in 0..6 { put(&mut sim, "TANK", 0, 150 + 20 * k, 200); }
        put(&mut sim, "TANK", 1, 850, 850);
        let mut ai = SkirmishAi::new(0, &sim, level);
        let mut issued = 0;
        for _ in 0..120 {
            let commands = ai.think(&sim);
            issued += commands.len();
            sim.step(&commands);
        }
        let (mut copy, mut copy_ai) = (sim.clone(), ai.clone());
        for _ in 0..360 {
            let (a, b) = (ai.think(&sim), copy_ai.think(&copy));
            issued += a.len();
            assert_eq!(a, b, "{level:?} command stream at tick {}", sim.tick);
            assert_eq!(ai.take_answers(), copy_ai.take_answers());
            sim.step(&a);
            copy.step(&b);
            assert_eq!(sim.checksum(), copy.checksum(), "{level:?} continued simulation at tick {}", sim.tick);
        }
        assert!(issued > 0, "{level:?} must exercise real decisions");
    }
}

#[test]
fn construction_planning_does_not_replace_a_commanders_pending_manual_shot() {
    let mut sim = fixture_edit(|d| {
        let t = d.unit_id("COMMANDER").unwrap();
        d.units[t].worker_time = 300.0;
        d.units[t].build_options = vec!["FACTORY".into()];
        d.units[t].weapons[0] = Some("BOMB".into());
        d.units[t].can_dgun = true;
        let builder = d.unit_id("BUILDER").unwrap();
        d.units[builder].build_options.push("EXTRACTOR".into());
        let factory = d.unit_id("FACTORY").unwrap();
        d.units[factory].build_options.push("BUILDER".into());
    });
    sim.los_enabled = false;
    let commander = put(&mut sim, "COMMANDER", 0, 300, 300);
    put(&mut sim, "TANK", 1, 380, 300);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let s = ai.survey(&sim);
    let mut out = Vec::new();
    ai.run_commander(&sim, &s, &mut out);
    assert!(out.iter().any(|c| matches!(c, Command::Order { units, order: Order::ManualFire(_), .. } if units.contains(&commander))));
    ai.run_builders(&sim, &s, &mut out);
    assert!(!out.iter().any(|c| matches!(c, Command::Order { units, order: Order::Build { .. }, .. } if units.contains(&commander))), "the manual shot must survive this think's construction phase");
}

#[test]
fn repair_duties_do_not_replace_a_builders_pending_construction_order() {
    let mut sim = fixture_edit(|d| {
        let builder = d.unit_id("BUILDER").unwrap();
        d.units[builder].build_options.push("EXTRACTOR".into());
        let factory = d.unit_id("FACTORY").unwrap();
        d.units[factory].build_options.push("BUILDER".into());
    });
    let builder = put(&mut sim, "BUILDER", 0, 300, 300);
    let target = put(&mut sim, "TANK", 0, 400, 300);
    sim.set_health(target, fx(100));
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.rally = sim.unit(target).unwrap().pos;
    ai.repairing.push((target, 0));
    let s = ai.survey(&sim);
    let mut out = Vec::new();
    ai.run_builders(&sim, &s, &mut out);
    assert!(out.iter().any(|c| matches!(c, Command::Order { units, order: Order::Build { .. }, .. } if units.contains(&builder))), "the builder should have a new construction job");
    ai.run_repairs(&sim, &s, &mut out);
    assert!(!out.iter().any(|c| matches!(c, Command::Order { units, order: Order::Repair(_), .. } if units.contains(&builder))), "repairs must not cancel a construction order issued earlier in this think");
}
