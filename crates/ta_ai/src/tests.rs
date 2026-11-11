//! Small asset-free matches for AI decisions that depend on building classification.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::{PlayerSetup, UnitTypes};

use super::*;
use crate::duty::SAVED_FORMAT;

#[path = "naval_tests.rs"]
mod naval;

#[path = "review_tests.rs"]
mod review;

fn fixture() -> Sim {
    fixture_edit(|_| {})
}

#[test]
fn captured_aircraft_count_as_losses_when_a_strike_is_accounted() {
    let mut sim = fixture();
    let bomber = sim.spawn_unit(sim.types.id("BOMBER").unwrap(), 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.air_strike = Some(crate::army::AirStrike { targets: Vec::new(), planes: vec![(bomber, 700)], started: 0 });
    sim.transfer_unit(bomber, 1);
    ai.run_air(&sim, &ai.survey(&sim), &mut Vec::new());
    assert!(ai.air_strike.is_none());
    assert_eq!(ai.air_ledger.1, 700, "a plane captured away is no longer an asset of the strike");
}

fn fixture_edit(edit: impl FnOnce(&mut GameData)) -> Sim {
    fixture_players(2, edit)
}

fn fixture_players(count: u8, edit: impl FnOnce(&mut GameData)) -> Sim {
    fixture_map(count, 64, edit)
}

/// The fixture with a flat map `cells` 16 px cells square.
fn fixture_map(count: u8, cells: u32, edit: impl FnOnce(&mut GameData)) -> Sim {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta-ai-buildings-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(dir.join("units")).unwrap();
    std::fs::create_dir_all(dir.join("weapons")).unwrap();
    for (name, keys) in [
        ("BUILDER", "Builder=1; WorkerTime=300; BuildDistance=200; CanMove=1; MaxVelocity=2;"),
        ("FACTORY", "Builder=1; WorkerTime=30; FootprintX=2; FootprintZ=2; BuildTime=100;"),
        ("TANK", "CanMove=1; MaxVelocity=2; Weapon1=GUN;"),
        ("BOMBER", "CanFly=1; CanMove=1; MaxVelocity=4; CruiseAlt=100; Weapon1=BOMB;"),
        ("COMMANDER", "Commander=1; Builder=1; CanMove=1; MaxVelocity=2;"),
        ("EXTRACTOR", "ExtractsMetal=1; FootprintX=2; FootprintZ=2;"),
    ] {
        std::fs::write(dir.join(format!("units/{name}.fbi")), format!("[UNITINFO] {{ UnitName={name}; Objectname={name}; MaxDamage=1000; SightDistance=100; BuildCostMetal=100; {keys} }}")).unwrap();
    }
    std::fs::write(dir.join("weapons/test.tdf"), "[GUN] { range=200; reloadtime=1; weaponvelocity=400; [DAMAGE] { default=10; } } [BOMB] { dropped=1; commandfire=1; range=150; reloadtime=1; weaponvelocity=100; [DAMAGE] { default=10; } }").unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_mod_dir(&dir, "test").unwrap();
    let mut data = GameData::load(&vfs);
    std::fs::remove_dir_all(dir).unwrap();
    for (name, product) in [("BUILDER", "FACTORY"), ("FACTORY", "TANK")] {
        let id = data.unit_id(name).unwrap();
        data.units[id].build_options.push(product.into());
    }
    edit(&mut data);
    let data = Arc::new(data);
    let types = Arc::new(UnitTypes::compile(&data, &|_| None));
    let n = (cells * cells) as usize;
    let map = TntMap { width: cells, height: cells, sea_level: 0, heights: vec![60; n], features: vec![FEATURE_NONE; n], tiles: vec![], tile_map: vec![], feature_names: vec![], minimap: Default::default() };
    let players: Vec<_> = (0..count).map(|color| PlayerSetup { side: "ARM".into(), color, start: FxVec3::ZERO, metal: fx(1000), energy: fx(1000), ai: false }).collect();
    Sim::new(types, data, &map, &MapInfo::default(), 0, &players, 7)
}

/// The fixture's map (`cells` 16 px cells square) with starts: ours at the first, the enemy at
/// the last, the others empty.
fn fixture_with_starts(spots: &[(i64, i64)]) -> Sim {
    fixture_sized(64, spots)
}

#[test]
fn silos_choose_targets_their_missiles_can_reach() {
    let mut sim = fixture_edit(|data| {
        let factory = data.unit_id("FACTORY").unwrap();
        data.units[factory].build_options.clear();
        data.units[factory].weapons[0] = Some("GUN".into());
        let gun = data.weapon_id("GUN").unwrap();
        let w = &mut data.weapons[gun];
        w.stockpile = true;
        w.command_fire = true;
        w.targetable = true;
        w.range = 400.0;
        w.area_of_effect = 128.0;
    });
    sim.los_enabled = false;
    let silo = sim.spawn_unit(sim.types.id("FACTORY").unwrap(), 0, FxVec3::from_ints(100, 60, 500), 0, true).unwrap();
    sim.grant_stockpile(silo, 1);
    let extractor = sim.types.id("EXTRACTOR").unwrap();
    let close = sim.spawn_unit(extractor, 1, FxVec3::from_ints(350, 60, 500), 0, true).unwrap();
    for x in [800, 830, 860] {
        sim.spawn_unit(extractor, 1, FxVec3::from_ints(x, 60, 500), 0, true).unwrap();
    }
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    assert_eq!(ai.info[sim.unit(silo).unwrap().type_id].role, Role::Silo);
    ai.remember(&sim, &ai.survey(&sim));
    let mut out = Vec::new();
    ai.run_silos(&sim, &ai.survey(&sim), &mut out);
    assert!(matches!(out.as_slice(), [Command::Order { order: Order::ManualFire(Target::Ground(p)), .. }]
        if p.within_xz(sim.unit(close).unwrap().pos, fx(32))), "prefer a reachable target over a richer distant cluster: {out:?}");
    sim.remove_unit(close);
    sim.step(&[]);
    ai.remember(&sim, &ai.survey(&sim));
    out.clear();
    ai.run_silos(&sim, &ai.survey(&sim), &mut out);
    assert!(out.is_empty(), "keep missiles when every target is out of range: {out:?}");
}

#[test]
fn nuclear_targeting_accounts_for_visible_mobile_missile_defence() {
    let mut sim = fixture_edit(|data| {
        let factory = data.unit_id("FACTORY").unwrap();
        data.units[factory].build_options.clear();
        data.units[factory].weapons[0] = Some("GUN".into());
        let gun = data.weapon_id("GUN").unwrap();
        let w = &mut data.weapons[gun];
        w.stockpile = true; w.command_fire = true; w.targetable = true;
        w.range = 400.0; w.area_of_effect = 128.0;
        let tank = data.unit_id("TANK").unwrap();
        data.units[tank].weapons[0] = Some("BOMB".into());
        let anti = data.weapon_id("BOMB").unwrap();
        let w = &mut data.weapons[anti];
        w.dropped = false; w.command_fire = false; w.interceptor = true; w.stockpile = true;
        w.coverage = 200.0; w.range = 32000.0;
    });
    sim.los_enabled = false;
    let silo = sim.spawn_unit(sim.types.id("FACTORY").unwrap(), 0, FxVec3::from_ints(100, 60, 500), 0, true).unwrap();
    sim.spawn_unit(sim.types.id("EXTRACTOR").unwrap(), 1, FxVec3::from_ints(350, 60, 500), 0, true).unwrap();
    sim.spawn_unit(sim.types.id("TANK").unwrap(), 1, FxVec3::from_ints(360, 60, 500), 0, true).unwrap();
    sim.grant_stockpile(silo, 1);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.remember(&sim, &ai.survey(&sim));
    let mut out = Vec::new();
    ai.run_silos(&sim, &ai.survey(&sim), &mut out);
    assert!(out.is_empty(), "one missile must not be wasted on known mobile anti-nuke cover: {out:?}");
    // A moving defence cannot be tracked through the fog using its live position.
    sim.los_enabled = true;
    sim.vision.rebuild(&[]);
    ai.remember(&sim, &ai.survey(&sim));
    ai.run_silos(&sim, &ai.survey(&sim), &mut out);
    assert_eq!(out.len(), 1, "unseen mobile defences must not become omniscient AI intelligence");
}

#[test]
fn defeating_a_land_enemy_replans_for_the_remaining_island_enemy() {
    let base = fixture_players(3, |_| {});
    let mut heights = vec![60; 128 * 128];
    for z in 0..128 {
        for x in 62..67 { heights[z * 128 + x] = 0; }
    }
    let map = TntMap { width: 128, height: 128, sea_level: 50, heights, features: vec![FEATURE_NONE; 16384], tiles: vec![], tile_map: vec![], feature_names: vec![], minimap: Default::default() };
    let players: Vec<_> = [(150, 150), (800, 150), (1800, 1800)].iter().enumerate().map(|(i, &(x, z))| PlayerSetup {
        side: "ARM".into(), color: i as u8, start: FxVec3::from_ints(x, 60, z), metal: fx(1000), energy: fx(1000), ai: true,
    }).collect();
    let mut sim = Sim::new(base.types.clone(), base.data.clone(), &map, &MapInfo::default(), 0, &players, 7);
    sim.los_enabled = false;
    let factory = sim.types.id("FACTORY").unwrap();
    let tank = sim.types.id("TANK").unwrap();
    for (i, p) in players.iter().enumerate() { sim.spawn_unit(factory, i as u8, p.start, 0, true).unwrap(); }
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.remember(&sim, &ai.survey(&sim));
    assert!(!ai.island && ai.crosses[tank]);
    assert!(ai.factory_reaches(&sim, factory));
    sim.players[1].defeated = true;
    ai.forget_defeated(&sim);
    assert_eq!(ai.enemy_bases, [players[2].start]);
    assert!(ai.island && !ai.crosses[tank], "the remaining enemy needs a sea or air crossing");
    assert!(!ai.factory_reaches(&sim, factory), "the old land-factory reach cache must not survive");
}

#[test]
fn naval_plans_use_long_range_ships_and_follow_enemy_base_changes() {
    let base = fixture_edit(|data| {
        let builder = data.unit_id("BUILDER").unwrap();
        data.units[builder].is_commander = true;
        let factory = data.unit_id("FACTORY").unwrap();
        data.units[factory].min_water_depth = 10.0;
        data.units[factory].floater = true;
        data.units[factory].build_options = vec!["TANK".into(), "BOMBER".into()];
        for name in ["TANK", "BOMBER"] {
            let id = data.unit_id(name).unwrap();
            data.units[id].can_fly = false;
            data.units[id].min_water_depth = 10.0;
            data.units[id].max_water_depth = 255.0;
            data.units[id].floater = true;
        }
        let long = data.unit_id("BOMBER").unwrap();
        data.units[long].build_cost_metal = 10000.0;
        let bomb = data.weapon_id("BOMB").unwrap();
        data.weapons[bomb].command_fire = false;
        data.weapons[bomb].dropped = false;
        data.weapons[bomb].range = 1000.0;
    });
    let heights = (0..4096).map(|i| if i % 64 < 30 { 0 } else { 60 }).collect();
    let map = TntMap { width: 64, height: 64, sea_level: 50, heights, features: vec![FEATURE_NONE; 4096], tiles: vec![], tile_map: vec![], feature_names: vec![], minimap: Default::default() };
    let players: Vec<_> = [(800, 200), (800, 900)].iter().enumerate().map(|(i, &(x, z))| PlayerSetup {
        side: "ARM".into(), color: i as u8, start: FxVec3::from_ints(x, 60, z), metal: fx(1000), energy: fx(1000), ai: true,
    }).collect();
    let mut sim = Sim::new(base.types.clone(), base.data.clone(), &map, &MapInfo::default(), 0, &players, 7);
    sim.spawn_unit(sim.types.id("BUILDER").unwrap(), 0, players[0].start, 0, true).unwrap();
    sim.spawn_unit(sim.types.id("EXTRACTOR").unwrap(), 1, players[1].start, 0, true).unwrap();
    sim.tick = MINUTE;
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let survey = ai.survey(&sim);
    assert!(survey.commander.is_some(), "the commander can build the yard");
    assert_eq!(survey.minutes, 1);
    let yard = sim.types.id("FACTORY").unwrap();
    assert_eq!(ai.info[yard].role, Role::Factory);
    assert_eq!(ai.info[yard].kind, Locomotion::Naval);
    ai.plan_navy(&sim, &survey);
    let naval = ai.naval.as_ref().expect("a shipyard fits along the coast");
    let short = sim.types.id("TANK").unwrap();
    let long = sim.types.id("BOMBER").unwrap();
    assert_eq!(ai.info[short].spec, ai.info[long].spec, "both ships use the same sea");
    let spec = ai.info[long].spec.unwrap();
    assert!(!map::can_reach(&sim.world, &spec, naval.site, players[1].start, (ai.info[short].range / 20) as i32));
    assert!(map::can_reach(&sim.world, &spec, naval.site, players[1].start, 30));
    assert!(naval.reaches, "the long-range ship can bombard the inland base");

    sim.set_mutators(ta_sim::Mutators { no_experimentals: true, ..Default::default() });
    assert!(sim.is_allowed(yard) && sim.is_allowed(short) && !sim.is_allowed(long));
    ai.replan(&sim);
    assert!(!ai.naval.as_ref().unwrap().reaches, "a restricted ship cannot justify a coastal attack");
    sim.allowed = None;
    sim.set_mutators(Default::default());
    ai.replan(&sim);
    assert!(ai.naval.as_ref().unwrap().reaches, "the same coast is reachable with the long-range ship allowed");

    ai.enemy_bases = vec![FxVec3::from_ints(1000, 60, 900)];
    ai.replan(&sim);
    assert!(!ai.naval.as_ref().unwrap().reaches, "a base beyond every ship's range must invalidate the naval plan");
}

fn fixture_sized(cells: u32, spots: &[(i64, i64)]) -> Sim {
    let mut sim = fixture_map(2, cells, |_| {});
    sim.start_spots = spots.iter().map(|&(x, z)| FxVec3::from_ints(x, 0, z)).collect();
    sim.players[0].start = sim.start_spots[0];
    sim.players[1].start = *sim.start_spots.last().unwrap();
    sim
}

#[test]
fn resource_producing_factories_still_build_armies() {
    let mut sim = fixture_edit(|data| {
        let factory = data.unit_id("FACTORY").unwrap();
        data.units[factory].energy_make = 10.0;
        data.units[factory].metal_make = 0.5;
    });
    let factory_type = sim.types.id("FACTORY").unwrap();
    let factory = sim.spawn_unit(factory_type, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    assert_eq!(ai.info[factory_type].role, Role::Factory);
    assert!(ai.info[sim.types.id("BUILDER").unwrap()].builds_factory, "constructors must discover the production chain");
    let mut commands = Vec::new();
    ai.run_factories(&sim, &ai.survey(&sim), &mut commands);
    assert!(commands.iter().any(|command| matches!(command, Command::Produce { factory: id, type_id, count, .. }
        if *id == factory && *type_id == sim.types.id("TANK").unwrap() && *count > 0)), "income must not disable factory production");
}

#[test]
fn tech_categories_accept_mod_tiers_without_substring_matches() {
    for (category, expected) in [("ARM LEVEL2 KBOT", 2), ("CORE lev2", 2), ("LEV3", 3), ("LEV4", 3), ("LEV5", 3),
        ("LEVEL2 LEV3", 3), ("NOTLEVEL2 LEVEL20 LEV255", 1), ("ARM", 1)] {
        let sim = fixture_edit(|data| {
            let tank = data.unit_id("TANK").unwrap();
            data.units[tank].category = category.into();
        });
        assert_eq!(info::tech_level(sim.types.get(sim.types.id("TANK").unwrap())), expected, "{category}");
    }
}

#[test]
fn remembered_factory_remains_an_air_target_after_los_is_lost() {
    let mut sim = fixture();
    let factory = sim.spawn_unit(sim.types.id("FACTORY").unwrap(), 1, FxVec3::from_ints(850, 0, 850), 0, true).unwrap();
    let planes: Vec<_> = (0..3).map(|i| sim.spawn_unit(sim.types.id("BOMBER").unwrap(), 0, FxVec3::from_ints(150 + 40 * i, 0, 150), 0, true).unwrap()).collect();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    assert_eq!(ai.info[sim.unit(factory).unwrap().type_id].kind, Locomotion::Ground, "factory products remain ground units");
    sim.vision.set_all(0, ta_sim::vision::LOS);
    ai.remember(&sim, &ai.survey(&sim));
    assert_eq!(ai.pick_air_target(&sim), Some(factory));
    sim.vision.rebuild(&[]);
    sim.tick = 20 * SECOND;
    assert_eq!(sim.seen_by(sim.unit(factory).unwrap(), 0), Seen::Hidden);
    let survey = ai.survey(&sim);
    ai.remember(&sim, &survey);
    assert_eq!(ai.pick_air_target(&sim), Some(factory));
    let mut commands = Vec::new();
    ai.run_air(&sim, &survey, &mut commands);
    let at = sim.unit(factory).unwrap().pos;
    assert!(commands.iter().any(|c| matches!(c, Command::Order { order: Order::Move(p), queue: false, .. } if *p == at)), "planes first approach the remembered position");
    assert!(commands.iter().any(|c| matches!(c, Command::Order { order: Order::Attack(Target::Unit(id)), queue: true, .. } if *id == factory)));
    sim.step(&commands);
    for id in planes {
        assert!(matches!(sim.unit(id).unwrap().orders.front(), Some(Order::Move(p)) if *p == at));
    }
}

#[test]
fn replacement_builder_finishes_factory_after_original_builder_dies() {
    let mut sim = fixture();
    let builder = sim.types.id("BUILDER").unwrap();
    let factory = sim.types.id("FACTORY").unwrap();
    let original = sim.spawn_unit(builder, 0, FxVec3::from_ints(280, 0, 300), 0, true).unwrap();
    let replacement = sim.spawn_unit(builder, 0, FxVec3::from_ints(260, 0, 350), 0, true).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![original], order: Order::Build { type_id: factory, pos: FxVec3::from_ints(352, 0, 304) }, queue: false }]);
    let frame = sim.unit(original).unwrap().build.unwrap().target;
    for _ in 0..3 { sim.step(&[]); }
    assert!(!sim.unit(frame).unwrap().is_complete());
    // The original constructor is gone; another must discover and finish its frame.
    sim.remove_unit(original);
    sim.step(&[]);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    let survey = ai.survey(&sim);
    assert!(matches!(ai.finish_task(&sim, &survey, sim.unit(replacement).unwrap(), 1000), Some(Task::Repair(id)) if id == frame));
    let mut commands = Vec::new();
    ai.run_builders(&sim, &survey, &mut commands);
    assert!(commands.iter().any(|c| matches!(c, Command::Order { units, order: Order::Repair(id), .. } if units.contains(&replacement) && *id == frame)));
    sim.step(&commands);
    for _ in 0..30 { sim.step(&[]); }
    assert!(sim.unit(frame).unwrap().is_complete());
    assert_eq!(sim.players[0].stats.built, 1);
}

#[test]
fn captured_units_leave_the_ais_army_and_repair_assignments() {
    let mut sim = fixture();
    let tank = sim.types.id("TANK").unwrap();
    let builder = sim.spawn_unit(sim.types.id("BUILDER").unwrap(), 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let captured = sim.spawn_unit(tank, 0, FxVec3::from_ints(400, 0, 300), 0, true).unwrap();
    let enemy = sim.spawn_unit(tank, 1, FxVec3::from_ints(500, 0, 300), 0, true).unwrap();
    sim.step(&[
        Command::SetStates { player: 0, units: vec![captured], fire: Some(0), movement: None },
        Command::Order { player: 1, units: vec![enemy], order: Order::Attack(Target::Unit(captured)), queue: false },
    ]);
    for _ in 0..600 {
        if sim.unit(captured).unwrap().health < fx(850) { break; }
        sim.step(&[]);
    }
    assert!(sim.unit(captured).unwrap().health < fx(850), "the returning tank needs repairs");
    sim.remove_unit(enemy);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    ai.repairing.push((captured, sim.tick));
    let mut commands = Vec::new();
    ai.run_repairs(&sim, &ai.survey(&sim), &mut commands);
    assert!(commands.iter().any(|c| matches!(c, Command::Order { units, order: Order::Repair(id), .. } if units == &[builder] && *id == captured)));

    sim.transfer_unit(captured, 1);
    commands.clear();
    ai.run_repairs(&sim, &ai.survey(&sim), &mut commands);
    assert!(ai.repairing.is_empty(), "capture removes the old owner's repair commitment");
    assert!(commands.is_empty(), "constructors must not be sent to repair the enemy");

    ai.attacking.push(captured);
    ai.defending.push((captured, sim.tick + MINUTE, sim.unit(builder).unwrap().pos));
    ai.muster = Some(army::Muster { units: vec![captured], at: ai.base, since: sim.tick });
    ai.fleet.push(captured);
    let survey = ai.survey(&sim);
    ai.run_army(&sim, &survey, &mut commands);
    ai.run_navy(&sim, &survey, &mut commands);
    assert!(ai.attacking.is_empty());
    assert!(ai.defending.is_empty());
    assert!(ai.muster.as_ref().is_none_or(|m| m.units.is_empty()));
    assert!(ai.fleet.is_empty());
}

#[test]
fn captured_commander_is_not_remembered_as_an_enemy_air_target() {
    let mut sim = fixture();
    let commander = sim.spawn_unit(sim.types.id("COMMANDER").unwrap(), 1, FxVec3::from_ints(700, 0, 700), 0, true).unwrap();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    sim.vision.set_all(0, ta_sim::vision::LOS);
    ai.remember(&sim, &ai.survey(&sim));
    assert_eq!(ai.enemy_commander.map(|c| c.0), Some(commander));
    sim.transfer_unit(commander, 0);
    ai.remember(&sim, &ai.survey(&sim));
    assert!(ai.enemy_commander.is_none(), "bombers must not keep targeting a captured Commander");
}

/// The player's helper sites a building only on ground the player has seen, whatever building
/// it is and however far the search goes; a computer player may site one anywhere it can build.
#[test]
fn the_helper_never_sites_a_building_in_the_dark() {
    let mut sim = fixture();
    // (Metal everywhere: an extractor may stand anywhere.)
    sim.world.metal.fill(200);
    sim.spawn_unit(sim.types.id("BUILDER").unwrap(), 0, FxVec3::from_ints(200, 0, 200), 0, true).unwrap();
    sim.step(&[]);
    let dark = FxVec3::from_ints(820, 0, 820);
    assert!(!sim.explored(0, dark), "the far corner is unexplored");
    let extractor = sim.types.id("EXTRACTOR").unwrap();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    let anywhere = ai.find_site(&sim, extractor, dark, 4, &|_| true);
    assert!(anywhere.is_some_and(|p| !sim.explored(0, p)), "a computer player sites it there, in the dark: {anywhere:?}");
    ai.helper = true;
    let site = ai.find_site(&sim, extractor, dark, 48, &|_| true);
    assert!(site.is_some_and(|p| sim.explored(0, p)), "the helper looks further and finds explored ground: {site:?}");
}

/// A constructor the player takes back off its economy duty, in the middle of upgrading an
/// extractor, gets no more orders for it: not while others stay on duty either.
#[test]
fn a_unit_taken_off_duty_mid_upgrade_is_left_alone() {
    let mut sim = fixture();
    sim.world.metal.fill(200);
    let builder_type = sim.types.id("BUILDER").unwrap();
    let builder = sim.spawn_unit(builder_type, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let other = sim.spawn_unit(builder_type, 0, FxVec3::from_ints(380, 0, 300), 0, true).unwrap();
    let mut officer = DutyOfficer::new(0, &sim);
    let deposit = officer.ai.deposits.iter().enumerate().min_by_key(|(_, d)| dist(d.pos, officer.ai.base)).unwrap().0;
    let extractor_type = sim.types.id("EXTRACTOR").unwrap();
    let old = sim.spawn_unit(extractor_type, 0, officer.ai.deposits[deposit].pos, 0, true).unwrap();
    officer.set(builder, Some(Duty::Economy));
    officer.set(other, Some(Duty::Economy));
    officer.ai.upgrades.push(Upgrade { deposit, builder, old, new_type: extractor_type, since: 0, reclaims: 0 });
    // Taken back (a player's order), the other still on duty: its upgrade goes with it.
    officer.set(builder, None);
    assert!(officer.ai.upgrades.is_empty(), "the upgrade is given up with the duty");
    for _ in 0..4 * THINK_TICKS {
        let commands = officer.think(&sim);
        assert!(
            !commands.iter().any(|c| matches!(c, Command::Order { units, .. } if units.contains(&builder))),
            "no orders for a unit back under the player's command: {commands:?}"
        );
        sim.step(&commands);
    }
}

/// An extractor of ours that a constructor is taking apart, to put a better one in its place, is
/// no unfinished building to finish: nobody is sent to it (a builder would only mend it as it
/// went). They were, and built it back up and finished it again every tick, for ever: TA:
/// Enhanced's CAIACK upgrading a CORMEX on Coast To Coast, with the Commander and four CAICKs
/// sent to finish it. Left part taken apart (its constructor gone), it is built back up.
#[test]
fn an_extractor_being_taken_apart_for_an_upgrade_is_not_given_helpers() {
    let mut sim = fixture_edit(|data| {
        let builder = data.unit_id("BUILDER").unwrap();
        data.units[builder].can_reclaim = true;
        let extractor = data.unit_id("EXTRACTOR").unwrap();
        data.units[extractor].build_time = 3000.0;
    });
    let builder_type = sim.types.id("BUILDER").unwrap();
    let reclaimer = sim.spawn_unit(builder_type, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let helper = sim.spawn_unit(builder_type, 0, FxVec3::from_ints(420, 0, 300), 0, true).unwrap();
    let old = sim.spawn_unit(sim.types.id("EXTRACTOR").unwrap(), 0, FxVec3::from_ints(360, 0, 300), 0, true).unwrap();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    sim.step(&[Command::Order { player: 0, units: vec![reclaimer], order: Order::ReclaimUnit(old), queue: false }]);
    for _ in 0..SECOND {
        sim.step(&[]);
    }
    assert!(sim.unit(old).is_some_and(|u| !u.is_complete() && sim.being_taken_apart(u)), "being taken apart");
    let survey = ai.survey(&sim);
    assert!(ai.finish_task(&sim, &survey, sim.unit(helper).unwrap(), 1000).is_none(), "nothing to finish");
    let mut commands = Vec::new();
    ai.run_builders(&sim, &survey, &mut commands);
    assert!(!commands.iter().any(|c| matches!(c, Command::Order { order: Order::Repair(id), .. } if *id == old)), "nobody sent to it: {commands:?}");
    sim.remove_unit(reclaimer);
    for _ in 0..SECOND {
        sim.step(&[]);
    }
    let survey = ai.survey(&sim);
    assert!(matches!(ai.finish_task(&sim, &survey, sim.unit(helper).unwrap(), 1000), Some(Task::Repair(id)) if id == old), "left part taken apart: finished");
}

#[test]
fn capturing_an_upgrade_builder_does_not_destroy_the_old_extractor() {
    let mut sim = fixture();
    sim.world.metal.fill(200);
    let builder = sim.spawn_unit(sim.types.id("BUILDER").unwrap(), 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    let deposit = ai.deposits.iter().enumerate().min_by_key(|(_, d)| dist(d.pos, ai.base)).unwrap().0;
    let extractor_type = sim.types.id("EXTRACTOR").unwrap();
    let old = sim.spawn_unit(extractor_type, 0, ai.deposits[deposit].pos, 0, true).unwrap();
    let upgrade = Upgrade { deposit, builder, old, new_type: extractor_type, since: 0, reclaims: 2 };
    ai.upgrades.push(upgrade);
    let mut commands = Vec::new();
    ai.run_upgrades(&sim, &ai.survey(&sim), &mut commands);
    assert!(commands.iter().any(|c| matches!(c, Command::SelfDestruct { units, .. } if units == &[old])), "an owned builder can replace the old extractor");

    commands.clear();
    sim.transfer_unit(builder, 1);
    ai.run_upgrades(&sim, &ai.survey(&sim), &mut commands);
    assert!(ai.upgrades.is_empty());
    assert!(commands.is_empty(), "the captured builder cannot replace the extractor, so leave it intact");

    sim.transfer_unit(builder, 0);
    sim.transfer_unit(old, 1);
    ai.upgrades.push(upgrade);
    ai.run_upgrades(&sim, &ai.survey(&sim), &mut commands);
    assert!(ai.upgrades.is_empty(), "capturing the extractor also ends the old owner's upgrade job");
    assert!(commands.is_empty());
}


#[test]
fn a_captured_builders_cancelled_work_is_not_blacklisted_as_unreachable() {
    let mut sim = fixture();
    let builder = sim.types.id("BUILDER").unwrap();
    let factory = sim.types.id("FACTORY").unwrap();
    let b = sim.spawn_unit(builder, 0, FxVec3::from_ints(280, 60, 300), 0, true).unwrap();
    let site = FxVec3::from_ints(352, 60, 304);
    let frame = sim.spawn_unit(factory, 0, site, 0, false).unwrap();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    ai.issued.push((b, factory, site, sim.tick));
    ai.issued_repairs.push((b, frame, sim.tick));
    sim.transfer_unit(b, 1);
    sim.tick += THINK_TICKS;
    ai.run_builders(&sim, &ai.survey(&sim), &mut Vec::new());
    assert!(ai.issued.is_empty() && ai.issued_repairs.is_empty());
    assert!(ai.failed.is_empty(), "capture says nothing about whether a build site can be reached");
    assert!(ai.unreachable_units.is_empty(), "other constructors may still finish the frame");
    assert!(ai.refusals.is_empty(), "the captured builder did not refuse its work");
}

#[test]
fn an_ally_who_needs_metal_gets_what_we_can_spare_and_an_enemy_nothing() {
    let mut sim = fixture();
    sim.set_team(1, 0);
    // (Each side in the game: a player without units is out.)
    let tank = sim.types.id("TANK").unwrap();
    sim.spawn_unit(tank, 0, FxVec3::from_ints(700, 0, 700), 0, true).unwrap();
    sim.spawn_unit(tank, 1, FxVec3::from_ints(150, 0, 150), 0, true).unwrap();
    let mut ai = SkirmishAi::new(1, &sim, Difficulty::Normal);
    // (The one asking is short of metal: a full store takes nothing more.)
    let room = Fx::from_int(120);
    sim.players[0].economy.metal = sim.players[0].economy.metal_capacity - room;
    sim.step(&[Command::Signal { player: 0, kind: SignalKind::NeedMetal, at: FxVec3::from_ints(100, 0, 100) }]);
    let mut out = Vec::new();
    ai.hear_allies(&sim, &ai.survey(&sim), &mut out);
    assert!(matches!(out[..], [Command::Share { player: 1, to: 0, metal, energy }] if metal > Fx::ZERO && energy == Fx::ZERO), "{out:?}");
    assert!(matches!(out[..], [Command::Share { metal, .. }] if metal <= room + Fx::from_int(2)), "no more than their store takes: {out:?}");
    assert!(matches!(ai.take_answers()[..], [Answer::Sent { to: 0, metal, energy: 0 }] if metal > 0));
    // The same signal is heard once.
    let mut out = Vec::new();
    ai.hear_allies(&sim, &ai.survey(&sim), &mut out);
    assert!(out.is_empty() && ai.take_answers().is_empty());
    // An enemy's is not heard at all.
    sim.set_team(1, 1);
    sim.step(&[Command::Signal { player: 0, kind: SignalKind::NeedMetal, at: FxVec3::from_ints(100, 0, 100) }]);
    let mut out = Vec::new();
    ai.hear_allies(&sim, &ai.survey(&sim), &mut out);
    assert!(out.is_empty() && ai.take_answers().is_empty());
}

#[test]
fn an_ally_calling_for_defence_gets_units_sent_to_the_spot() {
    let mut sim = fixture();
    sim.set_team(1, 0);
    let tank = sim.types.id("TANK").unwrap();
    sim.spawn_unit(tank, 0, FxVec3::from_ints(700, 0, 700), 0, true).unwrap();
    for i in 0..12 {
        sim.spawn_unit(tank, 1, FxVec3::from_ints(150 + 30 * i, 0, 150), 0, true).unwrap();
    }
    let mut ai = SkirmishAi::new(1, &sim, Difficulty::Normal);
    let spot = FxVec3::from_ints(800, 0, 800);
    sim.step(&[Command::Signal { player: 0, kind: SignalKind::Defend, at: spot }]);
    assert!(sim.signals.iter().any(|g| g.kind == SignalKind::Defend && g.player == 0), "the battle keeps the signal");
    let s = ai.survey(&sim);
    ai.remember(&sim, &s);
    let mut out = Vec::new();
    ai.hear_allies(&sim, &s, &mut out);
    ai.run_army(&sim, &s, &mut out);
    assert!(out.iter().any(|c| matches!(c, Command::Order { player: 1, order: Order::Fight(p), .. } if p.within_xz(spot, fx(16)))), "{out:?}");
    assert!(ai.take_answers().contains(&Answer::Defending { to: 0 }));
}

#[test]
fn raided_with_nothing_at_home_we_call_our_allies_to_defend() {
    // Player 1 (us) and player 0 allies; player 2 raids our base.
    let mut sim = fixture_players(3, |_| {});
    sim.set_team(1, 0);
    let factory = sim.types.id("FACTORY").unwrap();
    let tank = sim.types.id("TANK").unwrap();
    sim.spawn_unit(tank, 0, FxVec3::from_ints(900, 0, 900), 0, true).unwrap();
    sim.spawn_unit(factory, 1, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    for i in 0..8 {
        sim.spawn_unit(tank, 2, FxVec3::from_ints(380 + 20 * i, 0, 380), 0, true).unwrap();
    }
    // (Allies see through their team's first player's eyes.)
    sim.vision.set_all(0, ta_sim::vision::LOS);
    sim.vision.set_all(1, ta_sim::vision::LOS);
    let mut ai = SkirmishAi::new(1, &sim, Difficulty::Normal);
    let s = ai.survey(&sim);
    ai.remember(&sim, &s);
    assert!(!ai.threats(&sim, &s).is_empty(), "the raid is seen");
    let mut out = Vec::new();
    ai.run_army(&sim, &s, &mut out);
    assert!(out.iter().any(|c| matches!(c, Command::Signal { player: 1, kind: SignalKind::Defend, .. })), "{out:?}");
    // Once a minute at most.
    let mut again = Vec::new();
    ai.run_army(&sim, &s, &mut again);
    assert!(!again.iter().any(|c| matches!(c, Command::Signal { .. })));
}

/// What it has not seen it does not know: an enemy factory out of sight is no target, and no
/// enemy base is found where none was seen.
#[test]
fn unseen_enemy_buildings_are_unknown() {
    let mut sim = fixture_with_starts(&[(150, 150), (850, 850)]);
    let factory = sim.spawn_unit(sim.types.id("FACTORY").unwrap(), 1, FxVec3::from_ints(850, 0, 850), 0, true).unwrap();
    sim.spawn_unit(sim.types.id("BOMBER").unwrap(), 0, FxVec3::from_ints(150, 0, 150), 0, true).unwrap();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    assert_eq!(ai.enemy_bases, [FxVec3::from_ints(850, 0, 850)], "the map's other start");
    ai.remember(&sim, &ai.survey(&sim));
    assert!(ai.enemy_buildings.is_empty(), "nothing seen");
    assert_eq!(ai.pick_air_target(&sim), None);
    assert_eq!(ai.enemy_base_seen, [false], "a start not looked at");
    sim.vision.set_all(0, ta_sim::vision::LOS);
    ai.remember(&sim, &ai.survey(&sim));
    assert_eq!(ai.pick_air_target(&sim), Some(factory));
    assert_eq!((ai.enemy_base_seen.clone(), ai.enemy_base_owners.clone()), (vec![true], vec![1]), "seen: their base, and whose");
}

/// A building destroyed out of our sight is remembered until we look at its place; then it is
/// forgotten (no unit is looked up we cannot see).
#[test]
fn buildings_lost_unseen_are_remembered_until_their_place_is_seen() {
    let mut sim = fixture_with_starts(&[(150, 150), (850, 850)]);
    let factory = sim.spawn_unit(sim.types.id("FACTORY").unwrap(), 1, FxVec3::from_ints(850, 0, 850), 0, true).unwrap();
    sim.spawn_unit(sim.types.id("BUILDER").unwrap(), 0, FxVec3::from_ints(150, 0, 150), 0, true).unwrap();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    sim.vision.set_all(0, ta_sim::vision::LOS);
    ai.remember(&sim, &ai.survey(&sim));
    assert!(ai.enemy_buildings.iter().any(|b| b.0 == factory));
    sim.vision.rebuild(&[]);
    sim.remove_unit(factory);
    ai.remember(&sim, &ai.survey(&sim));
    assert!(ai.enemy_buildings.iter().any(|b| b.0 == factory), "destroyed where we could not see: still known");
    sim.vision.set_all(0, ta_sim::vision::LOS);
    ai.remember(&sim, &ai.survey(&sim));
    assert!(!ai.enemy_buildings.iter().any(|b| b.0 == factory), "its place seen without it");
}

/// Radar tells where, not what: a contact is a threat, but no building or unit known.
#[test]
fn radar_contacts_are_not_identified() {
    let mut sim = fixture_with_starts(&[(150, 150), (850, 850)]);
    let factory = sim.spawn_unit(sim.types.id("FACTORY").unwrap(), 1, FxVec3::from_ints(850, 0, 850), 0, true).unwrap();
    let tank = sim.spawn_unit(sim.types.id("TANK").unwrap(), 1, FxVec3::from_ints(600, 0, 600), 0, true).unwrap();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    sim.vision.set_all(0, ta_sim::vision::RADAR);
    assert_eq!(sim.seen_by(sim.unit(tank).unwrap(), 0), Seen::Radar);
    let survey = ai.survey(&sim);
    assert!(survey.enemies.is_empty() && survey.contacts.len() == 2, "two contacts, nothing seen");
    ai.remember(&sim, &survey);
    assert!(ai.enemy_buildings.is_empty() && ai.enemy_army.is_empty());
    assert!(ai.threat.at(FxVec3::from_ints(600, 0, 600)) > 0, "a contact is a threat");
    assert!(!ai.enemy_buildings.iter().any(|b| b.0 == factory));
    assert_eq!(ai.enemy_base_seen, [false]);
}

/// The enemy's starts are looked at: one found empty is dropped, one with their buildings is
/// their base; a scout is sent to those not looked at yet.
#[test]
fn starts_are_looked_at_and_the_enemy_found() {
    let mut sim = fixture_sized(256, &[(300, 300), (3800, 300), (300, 3800), (3800, 3800)]);
    sim.spawn_unit(sim.types.id("FACTORY").unwrap(), 1, FxVec3::from_ints(3810, 0, 3790), 0, true).unwrap();
    let scout = sim.spawn_unit(sim.types.id("TANK").unwrap(), 0, FxVec3::from_ints(310, 0, 350), 0, true).unwrap();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    assert_eq!(ai.enemy_bases.len(), 3, "the three other starts, which is the enemy's not known");
    let survey = ai.survey(&sim);
    ai.remember(&sim, &survey);
    let mut commands = Vec::new();
    ai.run_scouting(&sim, &survey, &mut commands);
    assert_eq!(ai.ground_scout, Some(scout), "the army's fastest goes");
    let firsts: Vec<FxVec3> = commands.iter().filter_map(|c| match c { Command::Order { units, order: Order::Move(p), .. } if units == &[scout] => Some(*p), _ => None }).collect();
    assert_eq!(firsts.len(), 3, "every start to look at, nearest first: {firsts:?}");
    // Every start looked at: the two empty ones dropped, the enemy's found.
    sim.vision.set_all(0, ta_sim::vision::LOS);
    ai.remember(&sim, &ai.survey(&sim));
    assert_eq!(ai.enemy_bases, [FxVec3::from_ints(3800, 0, 3800)], "the empty starts dropped");
    assert_eq!(ai.enemy_base_seen, [true]);
    let mut commands = Vec::new();
    ai.run_scouting(&sim, &ai.survey(&sim), &mut commands);
    assert_eq!(ai.ground_scout, None, "nothing left to look for");
}

// ---- Duties ----------------------------------------------------------------------------------

#[test]
fn duties_are_cycled_through_one_by_one_and_then_none() {
    let mut seen = Vec::new();
    let mut duty = None;
    for _ in 0..5 {
        duty = Duty::next(duty);
        seen.push(duty);
    }
    assert_eq!(seen, [Some(Duty::Economy), Some(Duty::Repair), Some(Duty::Reclaim), Some(Duty::Defence), None]);
    assert_eq!(Duty::next(duty), Some(Duty::Economy), "and round again");
    assert_eq!(Duty::ALL.iter().map(|d| d.name()).collect::<Vec<_>>(), ["economy", "repair", "reclaim", "defence"]);
}

#[test]
fn the_officer_orders_only_the_constructors_on_duty_and_runs_no_factory() {
    let mut sim = fixture();
    let (builder, factory, tank) = (sim.types.id("BUILDER").unwrap(), sim.types.id("FACTORY").unwrap(), sim.types.id("TANK").unwrap());
    let on_duty = sim.spawn_unit(builder, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let free = sim.spawn_unit(builder, 0, FxVec3::from_ints(300, 0, 360), 0, true).unwrap();
    let plant = sim.spawn_unit(factory, 0, FxVec3::from_ints(420, 0, 300), 0, true).unwrap();
    sim.spawn_unit(tank, 0, FxVec3::from_ints(500, 0, 300), 0, true).unwrap();
    // The factory has work (a computer player's builders would help it, and its factory would make
    // more).
    sim.step(&[Command::Produce { player: 0, factory: plant, type_id: tank, count: 2 }]);
    // (Work of the duty's own: an extractor left unfinished near by. The economy duty does not help
    // at the factories.)
    let frame = sim.spawn_unit(sim.types.id("EXTRACTOR").unwrap(), 0, FxVec3::from_ints(360, 0, 240), 0, false).unwrap();
    let mut officer = DutyOfficer::new(0, &sim);
    officer.set(on_duty, Some(Duty::Economy));
    let mut ordered = Vec::new();
    for _ in 0..30 * 90 {
        let commands = officer.think(&sim);
        ordered.extend(commands.iter().cloned());
        sim.step(&commands);
    }
    assert!(ordered.iter().any(|c| matches!(c, Command::Order { order: Order::Repair(id), .. } if *id == frame)), "the constructor on duty was given work: {ordered:?}");
    assert!(ordered.iter().all(|c| matches!(c, Command::Order { player: 0, units, .. } if units == &[on_duty])), "orders for the constructor on duty only: {ordered:?}");
    assert!(sim.unit(free).unwrap().orders.is_empty());
    assert!(sim.unit(frame).is_some_and(|u| u.is_complete()), "and it finished the extractor");
    assert_eq!(officer.units().collect::<Vec<_>>(), [(on_duty, Duty::Economy)], "and it stays on duty");
}

#[test]
fn units_that_are_lost_given_away_or_cannot_serve_leave_their_duty() {
    let mut sim = fixture();
    let (builder, factory, tank) = (sim.types.id("BUILDER").unwrap(), sim.types.id("FACTORY").unwrap(), sim.types.id("TANK").unwrap());
    let kept = sim.spawn_unit(builder, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let lost = sim.spawn_unit(builder, 0, FxVec3::from_ints(300, 0, 360), 0, true).unwrap();
    let given = sim.spawn_unit(builder, 0, FxVec3::from_ints(300, 0, 420), 0, true).unwrap();
    let plant = sim.spawn_unit(factory, 0, FxVec3::from_ints(420, 0, 300), 0, true).unwrap();
    let not_a_builder = sim.spawn_unit(tank, 0, FxVec3::from_ints(500, 0, 300), 0, true).unwrap();
    let frame = sim.spawn_unit(builder, 0, FxVec3::from_ints(300, 0, 480), 0, false).unwrap();
    let mut officer = DutyOfficer::new(0, &sim);
    assert!(DutyOfficer::can_serve(&sim, sim.unit(kept).unwrap()));
    assert!(!DutyOfficer::can_serve(&sim, sim.unit(plant).unwrap()), "a factory takes no duty");
    assert!(!DutyOfficer::can_serve(&sim, sim.unit(not_a_builder).unwrap()));
    for id in [kept, lost, given, plant, not_a_builder, frame] {
        officer.set(id, Some(Duty::Repair));
    }
    sim.remove_unit(lost);
    sim.transfer_unit(given, 1);
    officer.think(&sim);
    assert_eq!(officer.units().collect::<Vec<_>>(), [(kept, Duty::Repair)]);
    assert_eq!(officer.duty(kept), Some(Duty::Repair));
    assert_eq!(officer.duty(lost), None);
    officer.set(kept, None);
    assert!(officer.is_empty());
}

#[test]
fn the_officer_thinks_on_the_beat_and_is_quiet_with_no_one_on_duty() {
    let mut sim = fixture();
    let builder = sim.spawn_unit(sim.types.id("BUILDER").unwrap(), 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    sim.spawn_unit(sim.types.id("FACTORY").unwrap(), 0, FxVec3::from_ints(420, 0, 300), 0, true).unwrap();
    let mut officer = DutyOfficer::new(0, &sim);
    for tick in 0..60 {
        sim.tick = tick;
        assert!(officer.think(&sim).is_empty(), "nobody is on duty");
    }
    officer.set(builder, Some(Duty::Economy));
    let mut beats = Vec::new();
    for tick in 0..60 {
        sim.tick = tick;
        // (Nothing steps the sim here: the same job is given at every beat.)
        if !officer.think(&sim).is_empty() {
            beats.push(tick);
        }
    }
    assert!(beats.iter().all(|t| t % THINK_TICKS == 0), "{beats:?}");
}

#[test]
fn a_constructor_on_repair_duty_mends_a_hurt_unit_that_stands_and_leaves_one_on_the_move() {
    let mut sim = fixture();
    let (builder, tank) = (sim.types.id("BUILDER").unwrap(), sim.types.id("TANK").unwrap());
    let mender = sim.spawn_unit(builder, 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let hurt = sim.spawn_unit(tank, 0, FxVec3::from_ints(380, 0, 300), 0, true).unwrap();
    let marching = sim.spawn_unit(tank, 0, FxVec3::from_ints(380, 0, 380), 0, true).unwrap();
    let raiders = [sim.spawn_unit(tank, 1, FxVec3::from_ints(470, 0, 300), 0, true).unwrap(), sim.spawn_unit(tank, 1, FxVec3::from_ints(470, 0, 380), 0, true).unwrap()];
    // The raiders' guns (the units hold their fire, so only they are hurt) wear both down.
    sim.step(&[
        Command::SetStates { player: 0, units: vec![hurt, marching], fire: Some(0), movement: None },
        Command::Order { player: 1, units: vec![raiders[0]], order: Order::Attack(Target::Unit(hurt)), queue: false },
        Command::Order { player: 1, units: vec![raiders[1]], order: Order::Attack(Target::Unit(marching)), queue: false },
    ]);
    for _ in 0..1200 {
        if sim.unit(hurt).unwrap().health < fx(850) && sim.unit(marching).unwrap().health < fx(850) {
            break;
        }
        sim.step(&[]);
    }
    for raider in raiders {
        sim.remove_unit(raider);
    }
    let max = sim.unit_type(sim.unit(hurt).unwrap()).max_health;
    assert!(sim.unit(hurt).unwrap().health < max && sim.unit(marching).unwrap().health < max);
    // One of them is on its way somewhere.
    sim.step(&[Command::Order { player: 0, units: vec![marching], order: Order::Move(FxVec3::from_ints(900, 0, 900)), queue: false }]);
    let mut officer = DutyOfficer::new(0, &sim);
    officer.set(mender, Some(Duty::Repair));
    sim.tick = 0;
    let commands = officer.think(&sim);
    assert!(matches!(&commands[..], [Command::Order { units, order: Order::Repair(id), .. }] if units == &[mender] && *id == hurt), "the one that stands: {commands:?}");
    sim.step(&commands);
    for _ in 0..30 * 60 {
        let commands = officer.think(&sim);
        sim.step(&commands);
    }
    assert!(sim.unit(hurt).unwrap().health >= max, "mended whole");
}

/// A saved game keeps the duties as a few bytes and puts them back on the constructors still there:
/// not on those lost, given away or unfinished since, nor on a unit that cannot serve, and nonsense
/// puts back nothing.
#[test]
fn the_duties_are_kept_for_a_saved_game_and_put_back_on_what_is_still_there() {
    let mut sim = fixture();
    let (builder, factory, tank) = (sim.types.id("BUILDER").unwrap(), sim.types.id("FACTORY").unwrap(), sim.types.id("TANK").unwrap());
    let spawn = |sim: &mut Sim, t: usize, owner: u8, k: i64| sim.spawn_unit(t, owner, FxVec3::from_ints(300, 0, 300 + 60 * k), 0, true).unwrap();
    let (a, b, lost, given) = (spawn(&mut sim, builder, 0, 0), spawn(&mut sim, builder, 0, 1), spawn(&mut sim, builder, 0, 2), spawn(&mut sim, builder, 0, 3));
    let (plant, not_a_builder) = (spawn(&mut sim, factory, 0, 4), spawn(&mut sim, tank, 0, 5));
    let mut officer = DutyOfficer::new(0, &sim);
    assert!(officer.export().is_empty(), "nobody on duty: nothing to keep");
    officer.set(a, Some(Duty::Economy));
    officer.set(b, Some(Duty::Defence));
    for (id, duty) in [(lost, Duty::Reclaim), (given, Duty::Repair), (plant, Duty::Repair), (not_a_builder, Duty::Economy)] {
        officer.set(id, Some(duty));
    }
    let bytes = officer.export();
    assert_eq!(bytes.len(), 5 + 9 * 6);
    assert_eq!(bytes, officer.clone().export(), "the same bytes every time");
    // The battle goes on: one constructor is lost, another given to an ally; the plant and the tank
    // never could serve (the bytes are the officer's own, but a hand-made file may name them).
    sim.remove_unit(lost);
    sim.transfer_unit(given, 1);
    let mut loaded = DutyOfficer::new(0, &sim);
    assert_eq!(loaded.import(&sim, &bytes), 2);
    assert_eq!(loaded.units().collect::<Vec<_>>(), [(a, Duty::Economy), (b, Duty::Defence)]);
    // (Put back, a duty is a fresh one: the unit is looked at at the next beat.)
    assert!(loaded.fresh.contains(&a) && loaded.fresh.contains(&b));
    // What makes no sense puts back nothing, and never fails.
    let unknown = [bytes[..5].to_vec(), vec![0; 9]].concat();
    let mut other = DutyOfficer::new(0, &sim);
    for junk in [&[][..], &[SAVED_FORMAT][..], &[7, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0][..], &[SAVED_FORMAT, 255, 255, 255, 255][..], &[SAVED_FORMAT, 255, 255, 255, 255, 1, 2, 3][..], &unknown[..]] {
        assert_eq!(other.import(&sim, junk), 0, "{junk:?}");
    }
    assert!(other.is_empty());
    // A duty code of a later game is skipped, the rest read.
    let mut later = bytes.clone();
    later[5 + 8] = 9;
    assert_eq!(other.import(&sim, &later), 1, "the unit with the unknown duty is left under command");
    assert_eq!(other.duty(b), Some(Duty::Defence));
}

/// Plays the officer's orders into the game for `seconds`, collecting what it tells the player.
fn play_officer(officer: &mut DutyOfficer, sim: &mut Sim, told: &mut Vec<DutyEvent>, seconds: u64) {
    for _ in 0..30 * seconds {
        let commands = officer.think(sim);
        sim.step(&commands);
        told.extend(officer.take_events());
    }
}

#[test]
fn a_constructor_with_nothing_to_do_says_why_and_is_told_of_once_after_a_duty_is_given() {
    let mut sim = fixture();
    let builder = sim.spawn_unit(sim.types.id("BUILDER").unwrap(), 0, FxVec3::from_ints(300, 0, 300), 0, true).unwrap();
    let mut officer = DutyOfficer::new(0, &sim);
    officer.set(builder, Some(Duty::Repair));
    assert_eq!(officer.status(builder), None, "before its first beat");
    let mut told = Vec::new();
    play_officer(&mut officer, &mut sim, &mut told, 5);
    assert_eq!(officer.status(builder), Some(DutyStatus::Idle(IdleReason::NothingHurt)));
    assert!(told.is_empty(), "not yet: {told:?}");
    play_officer(&mut officer, &mut sim, &mut told, 25);
    assert_eq!(told, [DutyEvent::Idle { unit: builder, duty: Duty::Repair, why: IdleReason::NothingHurt }]);
    play_officer(&mut officer, &mut sim, &mut told, 60);
    assert_eq!(told.len(), 1, "once");
    // A new duty is a new start (its status is not the old duty's), and told of again in its turn.
    officer.set(builder, Some(Duty::Reclaim));
    assert_eq!(officer.status(builder), None);
    play_officer(&mut officer, &mut sim, &mut told, 40);
    assert_eq!(told.len(), 2, "{told:?}");
    assert!(matches!(told[1], DutyEvent::Idle { duty: Duty::Reclaim, .. }));
    // Off duty: nothing is kept of it.
    officer.set(builder, None);
    assert_eq!(officer.status(builder), None);
}

/// TA builds its long-range guns and the Doomsday Machine on hold fire (`NoAutoFire=1`) for the
/// player to set going: the computer player sets its own going, not its bombers.
#[test]
fn the_ai_sets_its_fixed_guns_built_on_hold_fire_going() {
    let mut sim = fixture_edit(|data| {
        let mut tower = data.units[data.unit_id("TANK").unwrap()].clone();
        tower.name = "TOWER".into();
        (tower.can_move, tower.max_velocity, tower.no_auto_fire) = (false, 0.0, true);
        data.units.push(tower);
    });
    let tower = sim.spawn_unit(sim.types.id("TOWER").unwrap(), 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
    assert_eq!(sim.unit(tower).unwrap().fire_state, 0, "built on hold fire");
    let bomber = sim.spawn_unit(sim.types.id("BOMBER").unwrap(), 0, FxVec3::from_ints(400, 60, 300), 0, true).unwrap();
    sim.step(&[Command::SetStates { player: 0, units: vec![bomber], fire: Some(0), movement: None }]);
    let ai = SkirmishAi::new(0, &sim, Difficulty::Hard);
    let mut out = Vec::new();
    ai.arm_defenses(&sim, &ai.survey(&sim), &mut out);
    assert!(matches!(&out[..], [Command::SetStates { units, fire: Some(2), movement: None, .. }] if units == &[tower]), "the tower set going, not the bomber");
}
