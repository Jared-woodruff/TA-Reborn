//! Defence against the air (`air.rs`) and the Commander getting away from guns it cannot fight, on
//! made-up data (no install needed): a constructor that puts up extractors, power, an
//! anti-aircraft tower and plants, a factory of tanks and anti-aircraft vehicles, an air plant of
//! fighters, the enemy's bombers and towers.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::{PlayerSetup, UnitTypes};

use super::*;

/// The units on a flat map 2,560 px across, two players, starts at (400, 400) and (2200, 2200),
/// ten minutes in, everything in sight, our income twenty metal a second.
fn fixture() -> Sim {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta-ai-air-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(dir.join("units")).unwrap();
    std::fs::create_dir_all(dir.join("weapons")).unwrap();
    for (name, keys) in [
        ("COMMANDER", "Commander=1; Builder=1; WorkerTime=300; BuildDistance=200; CanMove=1; MaxVelocity=2; MaxDamage=3000; Weapon1=GUN;"),
        ("BUILDER", "Builder=1; WorkerTime=300; BuildDistance=200; CanMove=1; MaxVelocity=2;"),
        ("FACTORY", "Builder=1; WorkerTime=100; FootprintX=4; FootprintZ=4; BuildTime=100; BuildCostMetal=300;"),
        ("AIRPLANT", "Builder=1; WorkerTime=100; FootprintX=4; FootprintZ=4; BuildTime=100; BuildCostMetal=300;"),
        ("EXTRACTOR", "ExtractsMetal=1; FootprintX=2; FootprintZ=2; BuildCostMetal=50;"),
        ("POWER", "EnergyMake=20; FootprintX=2; FootprintZ=2; BuildCostMetal=40;"),
        ("AATOWER", "FootprintX=2; FootprintZ=2; Weapon1=SAM; BuildCostMetal=90;"),
        ("TOWER", "FootprintX=2; FootprintZ=2; Weapon1=GUN; BuildCostMetal=200;"),
        ("TANK", "CanMove=1; MaxVelocity=2; Weapon1=GUN; BuildCostMetal=60;"),
        ("AAVEH", "CanMove=1; MaxVelocity=2; Weapon1=SAM; BuildCostMetal=100;"),
        ("FIGHTER", "CanFly=1; CanMove=1; MaxVelocity=8; CruiseAlt=100; Weapon1=SAM; BuildCostMetal=100;"),
        ("BOMBER", "CanFly=1; CanMove=1; MaxVelocity=8; CruiseAlt=100; Weapon1=BOMB; BuildCostMetal=100;"),
    ] {
        let health = if keys.contains("MaxDamage") { "" } else { "MaxDamage=1000;" };
        std::fs::write(dir.join(format!("units/{name}.fbi")), format!("[UNITINFO] {{ UnitName={name}; Objectname={name}; SightDistance=300; {health} {keys} }}")).unwrap();
    }
    std::fs::write(
        dir.join("weapons/test.tdf"),
        "[GUN] { range=200; reloadtime=1; weaponvelocity=400; [DAMAGE] { default=10; } } \
         [SAM] { range=600; reloadtime=1; weaponvelocity=900; toairweapon=1; [DAMAGE] { default=50; } } \
         [BOMB] { dropped=1; commandfire=1; range=150; reloadtime=1; weaponvelocity=100; [DAMAGE] { default=200; } }",
    )
    .unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_mod_dir(&dir, "test").unwrap();
    let mut data = GameData::load(&vfs);
    std::fs::remove_dir_all(dir).unwrap();
    let base = ["EXTRACTOR", "POWER", "FACTORY", "AIRPLANT", "AATOWER", "TOWER"];
    for (name, list) in [("COMMANDER", base.to_vec()), ("BUILDER", base.to_vec()), ("FACTORY", vec!["BUILDER", "TANK", "AAVEH"]), ("AIRPLANT", vec!["FIGHTER", "BOMBER"])] {
        let unit = data.unit_id(name).unwrap();
        data.units[unit].build_options = list.iter().map(|s| s.to_string()).collect();
    }
    let data = Arc::new(data);
    let types = Arc::new(UnitTypes::compile(&data, &|_| None));
    let cells = 160;
    let n = (cells * cells) as usize;
    let map = TntMap { width: cells, height: cells, sea_level: 0, heights: vec![60; n], features: vec![FEATURE_NONE; n], tiles: vec![], tile_map: vec![], feature_names: vec![], minimap: Default::default(), flags: Vec::new() };
    let players: Vec<_> = (0..2).map(|color| PlayerSetup { side: "ARM".into(), color, start: FxVec3::ZERO, metal: fx(1000), energy: fx(1000), ai: false }).collect();
    let mut sim = Sim::new(types, data, &map, &MapInfo::default(), 0, &players, 7);
    sim.los_enabled = false;
    sim.start_spots = vec![FxVec3::from_ints(400, 0, 400), FxVec3::from_ints(2200, 0, 2200)];
    sim.players[0].start = sim.start_spots[0];
    sim.players[1].start = sim.start_spots[1];
    sim.tick = 10 * MINUTE;
    rich(&mut sim);
    sim
}

/// Our income: twenty metal and five hundred energy a second, the stores half full.
fn rich(sim: &mut Sim) {
    let e = &mut sim.players[0].economy;
    e.metal_capacity = fx(4000);
    e.energy_capacity = fx(8000);
    e.metal = fx(2000);
    e.energy = fx(4000);
    e.metal_income = fx(20);
    e.energy_income = fx(500);
}

fn id(sim: &Sim, name: &str) -> usize {
    sim.types.id(name).unwrap()
}

fn put(sim: &mut Sim, name: &str, owner: u8, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(id(sim, name), owner, FxVec3::from_ints(x, 60, z), 0, true).unwrap()
}

/// Our Commander at home, (400, 400), and a computer player for us.
fn at_home(sim: &mut Sim) -> SkirmishAi {
    put(sim, "COMMANDER", 0, 400, 400);
    SkirmishAi::new(0, sim, Difficulty::Normal)
}

/// A think's look round: what is seen remembered, the cover from the air planned.
fn look(ai: &mut SkirmishAi, sim: &Sim) {
    let s = ai.survey(sim);
    ai.remember(sim, &s);
    ai.plan_air_cover(sim, &s);
}

/// The anti-aircraft wanted grows with the armed aircraft the enemy shows (a tower and two vehicles
/// for two bombers; many more, before more of the economy, for eight), and the threat fades a
/// bomber every half minute once they are out of sight; an air plant of theirs seen is a threat
/// before any of its aircraft is.
#[test]
fn anti_aircraft_grows_with_the_bombers_seen_and_fades() {
    let mut sim = fixture();
    let mut ai = at_home(&mut sim);
    look(&mut ai, &sim);
    let s = ai.survey(&sim);
    assert!(!ai.air_threat(s.now));
    assert_eq!((ai.aa_towers_wanted(&s), ai.aa_units_wanted(&s)), (0, 0), "nothing seen, nothing wanted");
    let bombers: Vec<UnitId> = (0..2).map(|k| put(&mut sim, "BOMBER", 1, 1500 + 40 * k, 1500)).collect();
    look(&mut ai, &sim);
    let s = ai.survey(&sim);
    assert_eq!(ai.air_pressure(s.now), 2);
    assert!(ai.air_threat(s.now) && !ai.air_heavy(s.now), "two bombers: a light threat");
    let light = (ai.aa_towers_wanted(&s), ai.aa_units_wanted(&s));
    assert_eq!(light, (1, 2));
    let more: Vec<UnitId> = (2..8).map(|k| put(&mut sim, "BOMBER", 1, 1500 + 40 * k, 1500)).collect();
    look(&mut ai, &sim);
    let s = ai.survey(&sim);
    assert_eq!(ai.air_pressure(s.now), 8);
    assert!(ai.air_heavy(s.now), "eight bombers: a heavy threat");
    let heavy = (ai.aa_towers_wanted(&s), ai.aa_units_wanted(&s));
    assert!(heavy.0 >= 6 && heavy.1 >= 8, "many more of both: {heavy:?} against {light:?}");
    // Out of sight, the threat fades: one bomber every half minute.
    for b in bombers.into_iter().chain(more) {
        sim.remove_unit(b);
    }
    sim.tick += 2 * AIR_FADE;
    look(&mut ai, &sim);
    assert_eq!(ai.air_pressure(sim.tick), 6);
    sim.tick += 10 * AIR_FADE;
    look(&mut ai, &sim);
    assert_eq!(ai.air_pressure(sim.tick), 0);
    // An air plant of the enemy's seen: a threat (a tower and a vehicle) before its aircraft come.
    let mut sim = fixture();
    let mut ai = at_home(&mut sim);
    put(&mut sim, "AIRPLANT", 1, 2100, 2100);
    look(&mut ai, &sim);
    let s = ai.survey(&sim);
    assert!(ai.air_threat(s.now) && !ai.air_heavy(s.now));
    assert_eq!((ai.aa_towers_wanted(&s), ai.aa_units_wanted(&s)), (1, 1));
}

/// Hurt with armed enemy aircraft over it: a raid from the air, and the threat heavy, however few
/// they are; hurt with none about (a tank's shells), no raid.
#[test]
fn hurt_under_bombers_is_a_raid_from_the_air() {
    let mut sim = fixture();
    let mut ai = at_home(&mut sim);
    let mex = put(&mut sim, "EXTRACTOR", 0, 1200, 400);
    let tank = put(&mut sim, "TANK", 1, 1300, 400);
    look(&mut ai, &sim);
    // (Hurt by a tank: no raid.)
    let shot = |sim: &mut Sim, at: UnitId, by: UnitId| {
        sim.step(&[Command::Order { player: 1, units: vec![by], order: Order::Attack(Target::Unit(at)), queue: false }]);
        for _ in 0..60 {
            sim.step(&[]);
        }
    };
    shot(&mut sim, mex, tank);
    look(&mut ai, &sim);
    assert!(ai.air_raids.is_empty(), "no aircraft about: no raid");
    // Hurt with a bomber over it.
    let bomber = put(&mut sim, "BOMBER", 1, 1250, 420);
    shot(&mut sim, mex, tank);
    look(&mut ai, &sim);
    assert!(ai.air_raids.iter().any(|(p, _)| p.within_xz(FxVec3::from_ints(1200, 0, 400), fx(RAID_NEAR))), "{:?}", ai.air_raids);
    assert_eq!(ai.air_pressure(sim.tick), 1);
    assert!(ai.air_heavy(sim.tick), "one bomber that hurt us is a heavy threat");
    let _ = bomber;
}

/// Under bombers anti-aircraft goes up by the part of our economy most exposed (an extractor field
/// out from the base with no cover), not round the base; once a tower covers it, the next goes by
/// the next part (the base's power), and two builders in one think go to two parts.
#[test]
fn anti_aircraft_goes_where_the_economy_is_exposed() {
    let mut sim = fixture();
    let mut ai = at_home(&mut sim);
    for (x, z) in [(1200, 400), (1260, 400), (1230, 470)] {
        put(&mut sim, "EXTRACTOR", 0, x, z);
    }
    // (The field worth 450 to a bomber, the base's power 360.)
    for (x, z) in [(300, 300), (340, 300), (300, 340)] {
        put(&mut sim, "POWER", 0, x, z);
    }
    let builder = put(&mut sim, "BUILDER", 0, 800, 400);
    let other = put(&mut sim, "BUILDER", 0, 820, 420);
    // (Six bombers: one tower's cover is enough for a part.)
    for k in 0..6 {
        put(&mut sim, "BOMBER", 1, 2000 + 30 * k, 2000);
    }
    look(&mut ai, &sim);
    let field = FxVec3::from_ints(1230, 0, 430);
    assert!(ai.air_exposed.first().is_some_and(|&(p, _)| p.within_xz(field, fx(RAID_NEAR))), "the field first: {:?}", ai.air_exposed);
    let s = ai.survey(&sim);
    let (mut plan, _) = ai.start_builders(&sim, &s, &mut Vec::new());
    let b = sim.unit(builder).unwrap();
    let opts = sim.unit_type(b).ai_build_options.clone();
    let Some(Task::Build(t, site)) = ai.aa_task(&sim, &s, b, &opts, &plan) else { panic!("anti-aircraft wanted") };
    assert_eq!(t, id(&sim, "AATOWER"));
    assert!(site.within_xz(field, fx(250)), "by the field: {site:?}");
    // The other builder, the same think: by the next part, the base's power.
    ai.give_task(&sim, &s, b, Task::Build(t, site), true, &mut plan, &mut Vec::new());
    let o = sim.unit(other).unwrap();
    let Some(Task::Build(_, second)) = ai.aa_task(&sim, &s, o, &opts, &plan) else { panic!("more anti-aircraft wanted") };
    assert!(!second.within_xz(field, fx(RAID_NEAR)) && second.within_xz(FxVec3::from_ints(320, 0, 320), fx(300)), "by the base's power: {second:?}");
    // Built, the field is covered: no longer exposed.
    sim.spawn_unit(t, 0, site, 0, true).unwrap();
    look(&mut ai, &sim);
    assert!(!ai.air_exposed.iter().any(|&(p, _)| p.within_xz(field, fx(RAID_NEAR))), "{:?}", ai.air_exposed);
}

/// Under bombers a builder puts up anti-aircraft before another extractor; with no aircraft about,
/// the extractor; and an extractor ordered where no cover reaches gets a tower beside it.
#[test]
fn under_bombers_anti_aircraft_comes_before_more_extractors() {
    let mut sim = fixture();
    // (Metal everywhere: an extractor may stand anywhere.)
    sim.world.metal.fill(200);
    let mut ai = at_home(&mut sim);
    let builder = put(&mut sim, "BUILDER", 0, 500, 500);
    put(&mut sim, "FACTORY", 0, 600, 300);
    look(&mut ai, &sim);
    let task = |ai: &mut SkirmishAi, sim: &Sim| {
        let s = ai.survey(sim);
        let (mut plan, _) = ai.start_builders(sim, &s, &mut Vec::new());
        match ai.builder_task(sim, &s, sim.unit(builder).unwrap(), &mut plan) {
            Some(Task::Build(t, _) | Task::ReclaimBuild(_, t, _)) => Some(t),
            _ => None,
        }
    };
    assert_eq!(task(&mut ai, &sim), Some(id(&sim, "EXTRACTOR")), "no aircraft about: more metal");
    for k in 0..6 {
        put(&mut sim, "BOMBER", 1, 2000 + 30 * k, 2000);
    }
    look(&mut ai, &sim);
    assert_eq!(task(&mut ai, &sim), Some(id(&sim, "AATOWER")), "bombers about: anti-aircraft first");
    // An extractor put up where no cover reaches: a tower beside it, after it.
    let s = ai.survey(&sim);
    let (mut plan, _) = ai.start_builders(&sim, &s, &mut Vec::new());
    let b = sim.unit(builder).unwrap();
    let mex = sim.snap_to_grid(id(&sim, "EXTRACTOR"), FxVec3::from_ints(1400, 0, 900));
    let mut out = Vec::new();
    ai.give_task(&sim, &s, b, Task::Build(id(&sim, "EXTRACTOR"), mex), true, &mut plan, &mut out);
    let aa = id(&sim, "AATOWER");
    assert!(out.iter().any(|c| matches!(c, Command::Order { order: Order::Build { type_id, pos }, queue: true, .. } if *type_id == aa && pos.within_xz(mex, fx(160)))), "{out:?}");
}

/// Idle mobile anti-aircraft waits at the posts over the parts of our economy most exposed (the
/// rally point among them); idle fighters go on patrol over home and those parts.
#[test]
fn mobile_anti_aircraft_is_posted_and_fighters_patrol() {
    let mut sim = fixture();
    let mut ai = at_home(&mut sim);
    for (x, z) in [(1600, 400), (1660, 400), (1630, 470)] {
        put(&mut sim, "EXTRACTOR", 0, x, z);
    }
    let vehicles: Vec<UnitId> = (0..6).map(|k| put(&mut sim, "AAVEH", 0, 420 + 30 * k, 600)).collect();
    let fighter = put(&mut sim, "FIGHTER", 0, 420, 420);
    for k in 0..6 {
        put(&mut sim, "BOMBER", 1, 2000 + 30 * k, 2000);
    }
    look(&mut ai, &sim);
    let field = FxVec3::from_ints(1630, 0, 430);
    assert!(ai.aa_posts.len() >= 2 && ai.aa_posts[0] == ai.rally && ai.aa_posts.iter().any(|p| p.within_xz(field, fx(RAID_NEAR))), "{:?}", ai.aa_posts);
    let mut out = Vec::new();
    ai.post_anti_air(&sim, &ai.survey(&sim), &mut out);
    let sent_to_field: usize = out.iter().filter_map(|c| match c {
        Command::Order { units, order: Order::Move(p), .. } if p.within_xz(field, fx(RAID_NEAR)) => Some(units.iter().filter(|u| vehicles.contains(u)).count()),
        _ => None,
    }).sum();
    assert!(sent_to_field >= 1, "vehicles to the field: {out:?}");
    assert!(out.iter().any(|c| matches!(c, Command::Order { units, order: Order::Patrol(_), .. } if units == &vec![fighter])), "the fighter patrols: {out:?}");
    assert!(!out.iter().any(|c| matches!(c, Command::Order { units, order: Order::Move(_), .. } if units.contains(&fighter))));
}

/// The Commander out of the base under a tower of the enemy's it sees, or hit with no armed unit of
/// theirs in sight about it, goes home, and the wreck it was after is left a while; hit in a fight
/// with units it sees, it stays (the D-gun is for them).
#[test]
fn the_commander_gets_away_from_guns_it_cannot_fight() {
    let mut sim = fixture();
    // (Home is the middle of our buildings: our Commander comes afterwards, out on the field.)
    put(&mut sim, "FACTORY", 0, 400, 400);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    let commander = put(&mut sim, "COMMANDER", 0, 1400, 400);
    let home = |out: &[Command]| out.iter().any(|c| matches!(c, Command::Order { units, order: Order::Move(p), .. } if units == &vec![commander] && p.within_xz(FxVec3::from_ints(400, 0, 400), fx(32))));
    look(&mut ai, &sim);
    let mut out = Vec::new();
    ai.run_commander(&sim, &ai.survey(&sim), &mut out);
    assert!(!home(&out), "nothing about: it stays at its work");
    // A tower of theirs that reaches it.
    let tower = put(&mut sim, "TOWER", 1, 1550, 400);
    look(&mut ai, &sim);
    let mut out = Vec::new();
    ai.run_commander(&sim, &ai.survey(&sim), &mut out);
    assert!(home(&out), "under a tower: home ({out:?})");
    assert!(sim.tick < ai.retreat_until);
    // Hit with nothing armed of theirs in sight about it (a gun out of sight, bombers).
    sim.remove_unit(tower);
    ai.retreat_until = 0;
    look(&mut ai, &sim);
    ai.commander_hit = Some(sim.tick);
    let mut out = Vec::new();
    ai.run_commander(&sim, &ai.survey(&sim), &mut out);
    assert!(home(&out), "hit by what it cannot see: home ({out:?})");
    // Hit in a fight with tanks it sees: it stays.
    ai.retreat_until = 0;
    put(&mut sim, "TANK", 1, 1500, 420);
    look(&mut ai, &sim);
    ai.commander_hit = Some(sim.tick);
    let mut out = Vec::new();
    ai.run_commander(&sim, &ai.survey(&sim), &mut out);
    assert!(!home(&out), "in a fight it sees: it stays ({out:?})");
}
