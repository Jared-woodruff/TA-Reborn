//! Shields for the computer players (`shields.rs`), on made-up data (no install needed): shield
//! generators put up by our most valuable factories and defences that no dome of ours covers, once
//! the economy carries their upkeep; an enemy's dome counted in what guards a place.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::{PlayerSetup, UnitTypes};

use super::*;

/// A constructor (BUILD) whose options hold a shield generator (GEN, a dome of 400 px at 200 energy
/// a second) and a tower; a factory (FACT), a tower (TOWER); a flat map, two players, `minutes` in.
fn fixture(minutes: u64) -> Sim {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta-ai-shields-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(dir.join("units")).unwrap();
    std::fs::create_dir_all(dir.join("weapons")).unwrap();
    let units = [
        ("BUILD", "Builder=1; WorkerTime=300; BuildDistance=200; CanMove=1; MaxVelocity=2; Category=LEVEL2;"),
        ("FACT", "Builder=1; WorkerTime=60; FootprintX=4; FootprintZ=4; BuildTime=100; BuildCostMetal=1000;"),
        ("TOWER", "FootprintX=2; FootprintZ=2; Weapon1=GUN; BuildCostMetal=300;"),
        ("GEN", "FootprintX=2; FootprintZ=2; BuildCostMetal=600; OnOffable=1; ActivateWhenBuilt=1; ShieldHealth=9000; ShieldRegen=120; ShieldRadius=400; ShieldHeight=200; ShieldEnergyUse=200;"),
        ("TANK", "CanMove=1; MaxVelocity=2; Weapon1=GUN; BuildCostMetal=50;"),
    ];
    for (name, keys) in units {
        std::fs::write(dir.join(format!("units/{name}.fbi")), format!("[UNITINFO] {{ UnitName={name}; Objectname={name}; SightDistance=100; MaxDamage=1000; {keys} }}")).unwrap();
    }
    std::fs::write(dir.join("weapons/test.tdf"), "[GUN] { range=200; reloadtime=1; weaponvelocity=400; [DAMAGE] { default=10; } }").unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_mod_dir(&dir, "test").unwrap();
    let mut data = GameData::load(&vfs);
    std::fs::remove_dir_all(dir).unwrap();
    for (name, list) in [("BUILD", vec!["GEN", "TOWER", "FACT"]), ("FACT", vec!["BUILD", "TANK"])] {
        let unit = data.unit_id(name).unwrap();
        data.units[unit].build_options = list.iter().map(|s| s.to_string()).collect();
    }
    let data = Arc::new(data);
    let types = Arc::new(UnitTypes::compile(&data, &|_| None));
    let cells = 160u32;
    let n = (cells * cells) as usize;
    let map = TntMap { width: cells, height: cells, sea_level: 0, heights: vec![60; n], features: vec![FEATURE_NONE; n], tiles: vec![], tile_map: vec![], feature_names: vec![], minimap: Default::default(), flags: Vec::new() };
    let players: Vec<_> = (0..2).map(|color| PlayerSetup { side: "ARM".into(), color, start: FxVec3::ZERO, metal: fx(1000), energy: fx(1000), ai: false }).collect();
    let mut sim = Sim::new(types, data, &map, &MapInfo::default(), 0, &players, 7);
    sim.los_enabled = false;
    sim.start_spots = vec![FxVec3::from_ints(400, 0, 400), FxVec3::from_ints(2200, 0, 2200)];
    sim.players[0].start = sim.start_spots[0];
    sim.players[1].start = sim.start_spots[1];
    sim.tick = minutes * MINUTE;
    grown(&mut sim, 2000);
    sim
}

/// Our economy grown: forty metal a second, `energy` a second, the stores half full.
fn grown(sim: &mut Sim, energy: i64) {
    let e = &mut sim.players[0].economy;
    e.metal_capacity = fx(10_000);
    e.energy_capacity = fx(20_000);
    e.metal = fx(5000);
    e.energy = fx(10_000);
    e.metal_income = fx(40);
    e.energy_income = fx(energy);
    e.energy_expense = Fx::ZERO;
}

fn put(sim: &mut Sim, name: &str, owner: u8, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), owner, FxVec3::from_ints(x, 60, z), 0, true).unwrap()
}

/// The shield generator a constructor of ours would put up now, and where, if any.
fn shield(ai: &mut SkirmishAi, sim: &Sim, builder: UnitId) -> Option<(usize, FxVec3)> {
    let s = ai.survey(sim);
    let (plan, _) = ai.start_builders(sim, &s, &mut Vec::new());
    let b = sim.unit(builder).unwrap();
    let opts = sim.unit_type(b).ai_build_options.clone();
    match ai.shield_task(sim, &s, b, &opts, &plan) {
        Some(Task::Build(t, at)) => Some((t, at)),
        _ => None,
    }
}

/// A generator goes up by the most valuable of what no dome of ours covers (the factory, then the
/// tower), near enough for its dome to take it in; none once both are covered, none early in the
/// battle, none on power that cannot carry it.
#[test]
fn shield_generators_go_up_by_what_they_cover() {
    let mut sim = fixture(12);
    let factory = put(&mut sim, "FACT", 0, 500, 500);
    let tower = put(&mut sim, "TOWER", 0, 1300, 1300);
    let builder = put(&mut sim, "BUILD", 0, 700, 700);
    let generator = sim.types.id("GEN").unwrap();
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    let at = |sim: &Sim, u: UnitId| sim.unit(u).unwrap().pos;
    let (t, site) = shield(&mut ai, &sim, builder).expect("a generator by the factory");
    assert_eq!(t, generator);
    // (Its middle within seven eighths of the dome, a little nearer.)
    assert!(site.within_xz(at(&sim, factory), fx(400 * 7 / 8 - 16)), "within its dome's reach of the factory: {site:?}");
    sim.spawn_unit(generator, 0, site, 0, true).unwrap();
    let (_, site) = shield(&mut ai, &sim, builder).expect("then one by the tower");
    assert!(site.within_xz(at(&sim, tower), fx(400 * 7 / 8 - 16)), "{site:?}");
    sim.spawn_unit(generator, 0, site, 0, true).unwrap();
    assert!(shield(&mut ai, &sim, builder).is_none(), "both covered: none");
    // Early in the battle, or short of power for its upkeep: none.
    let mut early = fixture(5);
    put(&mut early, "FACT", 0, 500, 500);
    let b = put(&mut early, "BUILD", 0, 700, 700);
    let mut ai = SkirmishAi::new(0, &early, Difficulty::Normal);
    assert!(shield(&mut ai, &early, b).is_none(), "not early");
    let mut poor = fixture(12);
    put(&mut poor, "FACT", 0, 500, 500);
    let b = put(&mut poor, "BUILD", 0, 700, 700);
    grown(&mut poor, 300);
    let mut ai = SkirmishAi::new(0, &poor, Difficulty::Normal);
    assert!(shield(&mut ai, &poor, b).is_none(), "not on power that cannot carry its upkeep");
}

/// An enemy's dome we know of guards what is under it (its generator's cost twice), and is a place
/// our tactical missiles and long guns leave alone.
#[test]
fn an_enemy_dome_guards_what_is_under_it() {
    let mut sim = fixture(20);
    put(&mut sim, "GEN", 1, 2000, 2000);
    let under = put(&mut sim, "FACT", 1, 2100, 2000);
    let away = put(&mut sim, "FACT", 1, 2000, 2800);
    let mut ai = SkirmishAi::new(0, &sim, Difficulty::Normal);
    let s = ai.survey(&sim);
    ai.remember(&sim, &s);
    let at = |u: UnitId| sim.unit(u).unwrap().pos;
    let generator = sim.types.id("GEN").unwrap();
    assert_eq!(ai.dome_guard(at(under)), ai.info[generator].cost * 2);
    assert_eq!(ai.dome_guard(at(away)), 0);
    assert!(ai.under_enemy_dome(at(under)) && !ai.under_enemy_dome(at(away)));
    assert!(ai.defence_at(at(under)) > ai.defence_at(at(away)));
}
