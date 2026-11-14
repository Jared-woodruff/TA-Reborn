use super::*;
use std::sync::{Arc, atomic::{AtomicU32, Ordering}};

const SOURCE: &str = r#"
id = "review"
title = "Review"
tagline = "Review"
briefing = "Review"
map = "test"
kind = "battle"
time = 12.0
victory = [{survive = 600}]
[player]
side = "ARM"
start = "200 200"
commander = true
units = [{unit = "TANK", at = "200 300", from = "easy", until = "hard"}]
[[enemy]]
side = "CORE"
start = "800 800"
commander = true
ai = "chosen"
"#;

fn assets() -> (Assets, ta_formats::tnt::TntMap) {
    static SERIAL: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta_scenario_review_{}_{}", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(dir.join("units")).unwrap();
    std::fs::create_dir_all(dir.join("gamedata")).unwrap();
    for name in ["ARMCOM", "CORCOM", "ARMCOM2", "TANK"] {
        std::fs::write(dir.join(format!("units/{name}.fbi")), format!("[UNITINFO]{{UnitName={name};ObjectName={name};Commander={};CanMove=1;MaxVelocity=1;MaxDamage=1000;}}", u8::from(name != "TANK"))).unwrap();
    }
    std::fs::write(dir.join("units/TOWER.fbi"), "[UNITINFO]{UnitName=TOWER;FootprintX=2;FootprintZ=2;MaxDamage=1000;}").unwrap();
    std::fs::write(dir.join("units/MEX.fbi"), "[UNITINFO]{UnitName=MEX;FootprintX=2;FootprintZ=2;MaxDamage=1000;ExtractsMetal=1;}").unwrap();
    std::fs::write(dir.join("gamedata/sidedata.tdf"), "[SIDE0]{name=ARM;commander=ARMCOM;} [SIDE1]{name=CORE;commander=CORCOM;} [SIDE2]{name=ARM;commander=ARMCOM2;}").unwrap();
    let mut vfs = ta_vfs::Vfs::new(); vfs.mount_mod_dir(&dir, "test").unwrap();
    let data = Arc::new(ta_data::GameData::load(&vfs));
    let types = Arc::new(ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let info = ta_data::MapInfo::default();
    let maps = vec![crate::setup::MapEntry { name: "test".into(), title: "Test".into(), description: String::new(), planet: String::new(), size: String::new(), start_positions: 0, origin: Default::default(), info }];
    let map = ta_formats::tnt::TntMap { width: 64, height: 64, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; 64 * 64], features: vec![ta_formats::tnt::FEATURE_NONE; 64 * 64], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    let assets = Assets { vfs: Arc::new(vfs), data, types, maps: maps.clone(), selectable_maps: maps, campaigns: Vec::new(), data_hash: 0, compatibility: Default::default(), mods: Default::default() };
    std::fs::remove_dir_all(dir).unwrap();
    (assets, map)
}

fn sim(assets: &Assets, map: &ta_formats::tnt::TntMap) -> Sim {
    let players: Vec<_> = ["ARM", "CORE"].into_iter().enumerate().map(|(i, side)| ta_sim::PlayerSetup { side: side.into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::ZERO, energy: Fx::ZERO, ai: i > 0 }).collect();
    Sim::new(assets.types.clone(), assets.data.clone(), map, &ta_data::MapInfo::default(), 0, &players, 7)
}

fn ground() -> Ground { Ground { size: (1024.0, 1024.0), starts: Vec::new() } }

#[test]
fn authored_defense_lines_leave_gaps_instead_of_packing_towers() {
    let (assets, map) = assets();
    let mut sim = sim(&assets, &map);
    // A coincident facing point must still produce a proper row.
    let source = SOURCE.replace("TANK\", at = \"200 300\"", "TOWER\", count = 3, at = \"400 400\", formation = \"line\", spacing = 160, facing = \"400 400\"");
    Scenario::parse(&source).unwrap().apply(&mut sim, Difficulty::Normal, &ground()).unwrap();
    let towers: Vec<_> = sim.units().filter(|u| sim.unit_type(u).name == "TOWER").collect();
    assert_eq!(towers.len(), 3);
    assert_eq!(towers.iter().map(|u| u.pos.z).min(), towers.iter().map(|u| u.pos.z).max());
    let mut xs: Vec<_> = towers.iter().map(|u| u.pos.x.to_int()).collect();
    xs.sort_unstable();
    assert!(xs.windows(2).all(|pair| pair[1] - pair[0] == 160));
}

#[test]
fn extractors_claim_separate_productive_deposits_before_other_buildings() {
    let (assets, map) = assets();
    let mut sim = sim(&assets, &map);
    for (x,z) in [(20,20), (40,20)] {
        let index = (z * sim.world.width as i32 + x) as usize;
        sim.world.metal[index] = 100;
    }
    let source = SOURCE.replace("{unit = \"TANK\", at = \"200 300\", from = \"easy\", until = \"hard\"}",
        "{unit = 'TOWER', at = '320 320', formation = 'grid'}, {unit = 'MEX', count = 2, at = '400 320', formation = 'deposits'}");
    Scenario::parse(&source).unwrap().apply(&mut sim, Difficulty::Normal, &ground()).unwrap();
    let mines: Vec<_> = sim.units().filter(|u| sim.unit_type(u).name == "MEX").collect();
    assert_eq!(mines.len(), 2);
    for mine in &mines { assert!(sim.extraction_at(mine.type_id, mine.pos) > Fx::ZERO); }
    assert!(!mines[0].pos.within_xz(mines[1].pos, Fx::from_int(128)));
}

#[test]
fn invalid_formations_are_rejected_before_setup() {
    let (assets, map) = assets();
    for extra in ["formation = 'blob'", "formation = 'grid', spacing = nan", "formation = 'line', spacing = -10", "spacing = 100", "facing = 'north'"] {
        assert!(Scenario::parse(&SOURCE.replace("at = \"200 300\"", &format!("at = '200 300', {extra}"))).is_err());
    }
    for extra in ["formation = 'deposits'", "formation = 'line', facing = 'missing'"] {
        let s = Scenario::parse(&SOURCE.replace("at = \"200 300\"", &format!("at = '200 300', {extra}"))).unwrap();
        let mut sim = sim(&assets, &map);
        let before = sim.checksum();
        assert!(s.apply(&mut sim, Difficulty::Normal, &ground()).is_err());
        assert_eq!(sim.checksum(), before);
    }
}

#[test]
fn starting_bases_preserve_each_missions_force_roles() {
    for source in BUILT_IN {
        let s = Scenario::parse(source).unwrap();
        if matches!(s.id.as_str(), "last-stand" | "endless-siege" | "until-dawn" | "titans" | "armageddon") {
            assert!(s.enemies.iter().all(|f| !f.commander && f.units.is_empty()), "{}: wave-only enemies must arrive on schedule", s.id);
        }
        if matches!(s.id.as_str(), "extraction" | "armageddon") {
            assert!(s.player.units.iter().all(|p| !p.unit.ends_with("MEX") && !p.unit.ends_with("SOLAR") && !p.unit.ends_with("LAB")), "{}: mobile-only mission acquired a base", s.id);
        }
    }
}

#[test]
fn ambiguous_goals_and_unknown_difficulty_words_are_rejected() {
    for source in [
        SOURCE.replace("survive = 600", "survive = 600, lose = 'commander'"),
        SOURCE.replace("survive = 600", "survive = 600, radius = 100"),
        SOURCE.replace("ai = \"chosen\"", "ai = 'expert'"),
        SOURCE.replace("from = \"easy\"", "from = 'hrd'"),
        SOURCE.replace("until = \"hard\"", "until = 'chosen'"),
        SOURCE.replace("from = \"easy\"", "from = 'hard'").replace("until = \"hard\"", "until = 'easy'"),
    ] { assert!(Scenario::parse(&source).is_err(), "accepted invalid scenario: {source}"); }
}

#[test]
fn scenario_lighting_rejects_nonfinite_or_empty_time_keys() {
    for value in ["nan", "inf", "[]", "[[-1.0, 12.0]]", "[[0.0, nan]]"] {
        assert!(Scenario::parse(&SOURCE.replace("time = 12.0", &format!("time = {value}"))).is_err(), "accepted {value}");
    }
}

#[test]
fn scenario_catalogue_checks_destinations_goals_sides_and_commanders() {
    let (mut assets, _) = assets();
    assert!(check(&Scenario::parse(SOURCE).unwrap(), &assets).is_ok());
    for source in [
        SOURCE.replace("survive = 600", "reach = 'missing', radius = 100"),
        SOURCE.replace("survive = 600", "destroy = 'MISSING'"),
        SOURCE.replace("side = \"ARM\"", "side = 'MISSING'"),
        SOURCE.replace("at = \"200 300\"", "at = '200 300', patrol = 'missing'"),
        format!("{SOURCE}\n[[wave]]\nat = 30\nfrom = 'missing'\nunits = [{{unit = 'TANK'}}]"),
        format!("{SOURCE}\n[[wave]]\nat = 30\nfrom = 'north'\nto = 'missing'\nunits = [{{unit = 'TANK'}}]"),
    ] {
        let scenario = Scenario::parse(&source).unwrap();
        assert!(check(&scenario, &assets).is_err(), "catalogue offered an invalid scenario: {source}");
    }
    Arc::get_mut(&mut assets.data).unwrap().sides[0].commander = "MISSING".into();
    assert!(check(&Scenario::parse(SOURCE).unwrap(), &assets).is_err());
}

#[test]
fn failed_scenario_preflight_leaves_the_simulation_untouched() {
    let (assets, map) = assets();
    let mut sim = sim(&assets, &map);
    let before = sim.checksum();
    let scenario = Scenario::parse(&SOURCE.replace("survive = 600", "reach = 'missing', radius = 100")).unwrap();
    assert!(scenario.apply(&mut sim, Difficulty::Normal, &ground()).is_err());
    assert_eq!(sim.unit_count(), 0, "validation precedes spawning");
    assert_eq!(sim.checksum(), before);
}

#[test]
fn scenario_commanders_use_the_force_side_key_including_duplicate_races() {
    let (assets, map) = assets();
    let mut sim = sim(&assets, &map);
    let scenario = Scenario::parse(&SOURCE.replace("side = \"ARM\"", "side = 'ARM#2'")).unwrap();
    scenario.apply(&mut sim, Difficulty::Normal, &ground()).unwrap();
    let commander = sim.units().find(|u| u.owner == 0 && sim.unit_type(u).def.is_commander).unwrap();
    assert_eq!(sim.unit_type(commander).name, "ARMCOM2");
}

#[test]
fn zero_count_or_zero_growth_waves_do_not_create_extra_units() {
    let (assets, map) = assets();
    for (count, growth, expected) in [(0, 1, 0), (2, 0, 1)] {
        let mut sim = sim(&assets, &map);
        let source = format!("{SOURCE}\n[[wave]]\nat = 30\nevery = 30\ntimes = 2\ngrow = {growth}.0\nfrom = 'north'\nunits = [{{unit = 'TANK', count = {count}}}]");
        Scenario::parse(&source).unwrap().apply(&mut sim, Difficulty::Normal, &ground()).unwrap();
        assert_eq!(sim.mission.as_ref().unwrap().waves.len(), expected);
    }
}

#[test]
fn scenario_cannot_start_without_its_requested_commander() {
    let (assets, mut map) = assets();
    map.sea_level = 100; map.heights.fill(0);
    let mut sim = sim(&assets, &map);
    assert!(Scenario::parse(SOURCE).unwrap().apply(&mut sim, Difficulty::Normal, &ground()).is_err());
    assert!(sim.mission.is_none());
}

#[test]
fn catalogue_preflight_allows_starts_generated_for_extra_forces() {
    for authored in [2, 0, 4] {
        let (mut assets, _) = assets();
        assets.maps[0].info.schemas = vec![ta_data::map::MapSchema {
            start_positions: (1..=authored).map(|i| (i, 100.0 * i as f32, 200.0)).collect(),
            ..Default::default()
        }];
        assets.maps[0].start_positions = authored as usize;
        let source = format!("{SOURCE}\n[[ally]]\nside = 'ARM'\nstart = '200 800'\n[[wave]]\nat = 30\nplayer = true\nfrom = 'start 3'\nto = 'start 1'\nunits = [{{unit = 'TANK'}}]");
        let scenario = Scenario::parse(&source).unwrap();
        assert!(check(&scenario, &assets).is_ok(), "{authored} authored starts must allow all three forces' indexes");
        assert!(scenario.setup(&source, Difficulty::Normal, &assets, 7).is_ok());
        let other_references = source.replace("start = '200 800'", "start = 'start 3'")
            .replace("at = \"200 300\"", "at = 'start 3', patrol = 'start 3'")
            .replace("survive = 600", "reach = 'start 3', radius = 100, unit = 'commander'");
        assert!(check(&Scenario::parse(&other_references).unwrap(), &assets).is_ok(), "generated positions also work in forces, placements, patrols and goals");
        if authored == 4 {
            let fourth = Scenario::parse(&source.replace("start 3", "start 4")).unwrap();
            assert!(check(&fourth, &assets).is_ok(), "authored positions beyond the force count remain valid");
        }
        let invalid = Scenario::parse(&source.replace("start 3", &format!("start {}", authored.max(3) + 1))).unwrap();
        assert!(check(&invalid, &assets).is_err(), "references beyond both authored and generated starts remain invalid");
    }
}

/// A goal to destroy a type of unit is every one of them (Titans' three Krogoths, Iron Rain's two
/// Intimidators): the battle's goals say "Destroy every Krogoth", as the scenarios page does,
/// not "Destroy Krogoth".
#[test]
fn a_goal_to_destroy_a_type_of_unit_names_every_one_of_them() {
    let (assets, map) = assets();
    let mut sim = sim(&assets, &map);
    let source = SOURCE.replace("victory = [{survive = 600}]", "victory = [{destroy = 'TANK'}, {destroy = 'commander'}]");
    let run = Scenario::parse(&source).unwrap().apply(&mut sim, Difficulty::Normal, &ground()).unwrap();
    assert!(matches!(&run.victory[0], GoalView::DestroyEvery { unit } if !unit.is_empty()), "{:?}", run.victory[0]);
    assert!(matches!(&run.victory[1], GoalView::Destroy { .. }), "{:?}", run.victory[1]);
}
