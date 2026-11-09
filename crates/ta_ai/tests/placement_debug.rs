use std::sync::Arc;
use ta_data::{GameData, MapInfo};
use ta_formats::{tdf, tnt::TntMap};
use ta_sim::{Fx, FxVec3, PlayerSetup, Sim, UnitTypes};

#[test]
#[ignore]
fn factory_sites() {
    let install = ta_vfs::install::discover(None).unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    let p = PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::ONE, energy: Fx::ONE, ai: true };
    let sim = Sim::new(types.clone(), data, &map, &info, 0, &[p], 1);
    let lab = types.id("ARMLAB").unwrap();
    for (n, x, z) in &info.schemas[0].start_positions[..2] {
        let at = FxVec3::new(Fx::from_f32(*x), Fx::ZERO, Fx::from_f32(*z));
        for r in [10, 30, 60] {
            println!("start {n} r={r}: {:?}", sim.find_build_site(lab, at, r));
        }
        let mut errs = std::collections::BTreeMap::new();
        for dz in -10..10 { for dx in -10..10 {
            let p = at + FxVec3::from_ints(dx * 32, 0, dz * 32);
            *errs.entry(format!("{:?}", sim.placement_error(lab, p))).or_insert(0) += 1;
        }}
        println!("  errors in 640px square: {errs:?}");
    }
}

#[test]
#[ignore]
fn shipyard_sites() {
    let install = ta_vfs::install::discover(None).unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let name = std::env::var("TA_TEST_MAP").unwrap_or_else(|_| "Coast To Coast".into());
    let map = TntMap::parse(&vfs.read(&format!("maps/{name}.tnt")).unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{name}.ota")).unwrap()));
    let p = PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::ONE, energy: Fx::ONE, ai: true };
    let sim = Sim::new(types.clone(), data, &map, &info, 0, &[p], 1);
    let yard = types.id("ARMSY").unwrap();
    println!("sea level {}, size {:?}", sim.world.sea_level.to_f32(), sim.world.size_px());
    let t = types.get(yard);
    println!("ARMSY footprint {:?} minwaterdepth {} maxwaterdepth {} yardmap {:?}", t.footprint, t.def.min_water_depth, t.def.max_water_depth, t.def.yard_map);
    let mut errs = std::collections::BTreeMap::new();
    for z in (100..1900).step_by(100) {
        for x in (100..3300).step_by(100) {
            let p = FxVec3::from_ints(x, 0, z);
            let depth = sim.world.sea_level - sim.world.height_at(p.x, p.z);
            let e = format!("{:?}", sim.placement_error(yard, p));
            if depth > Fx::from_int(20) {
                *errs.entry(e).or_insert(0) += 1;
            }
        }
    }
    println!("errors over deep water: {errs:?}");
    for (n, x, z) in &info.schemas[0].start_positions {
        let at = FxVec3::new(Fx::from_f32(*x), Fx::ZERO, Fx::from_f32(*z));
        for r in [30, 60, 120] {
            println!("start {n} ({x}, {z}) r={r}: {:?}", sim.find_build_site(yard, at, r).map(|p| (p.x.to_int(), p.z.to_int())));
        }
    }
}

#[test]
#[ignore]
fn water_depth_fields() {
    let install = ta_vfs::install::discover(None).unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = GameData::load(&vfs);
    for u in &data.units {
        let text = String::from_utf8_lossy(&vfs.read(&format!("units/{}.fbi", u.name.to_ascii_lowercase())).unwrap_or_default()).to_ascii_lowercase();
        let has_max = text.contains("maxwaterdepth");
        let floater = text.contains("floater");
        if u.min_water_depth > 0.0 || floater || u.max_velocity <= 0.0 && !has_max {
            println!("{:10} static={} min={} max={} (given: {has_max}) floater={floater} class={:?}", u.name, u.max_velocity <= 0.0, u.min_water_depth, u.max_water_depth, u.movement_class);
        }
    }
}

#[test]
#[ignore]
fn sea_deposits() {
    let install = ta_vfs::install::discover(None).unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let name = std::env::var("TA_TEST_MAP").unwrap_or_else(|_| "Coast To Coast".into());
    let map = TntMap::parse(&vfs.read(&format!("maps/{name}.tnt")).unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{name}.ota")).unwrap()));
    let p = PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::ONE, energy: Fx::ONE, ai: true };
    let sim = Sim::new(types.clone(), data, &map, &info, 0, &[p], 1);
    let (mut land, mut sea) = (0, 0);
    for f in &sim.features {
        let def = &sim.data.features[f.def];
        if f.alive && def.is_metal_deposit() {
            let depth = sim.world.sea_level - sim.world.height_at(f.pos.x, f.pos.z);
            if depth > Fx::ZERO { sea += 1 } else { land += 1 }
        }
    }
    println!("{name}: metal deposits on land {land}, under water {sea}; tidal strength {}", info.tidal_strength);
}

#[test]
#[ignore]
fn metal_layer() {
    let install = ta_vfs::install::discover(None).unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let name = std::env::var("TA_TEST_MAP").unwrap_or_else(|_| "Between Two Lakes".into());
    let map = TntMap::parse(&vfs.read(&format!("maps/{name}.tnt")).unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{name}.ota")).unwrap()));
    let p = PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::ONE, energy: Fx::ONE, ai: true };
    let sim = Sim::new(types.clone(), data, &map, &info, 0, &[p], 1);
    let (w, h) = (sim.world.width as i32, sim.world.height as i32);
    let (mut cells, mut total) = (0, 0i64);
    for z in 0..h { for x in 0..w {
        let m = sim.world.metal_under((x, z), (1, 1));
        if m > 0 { cells += 1; total += m; }
    }}
    let features = sim.features.iter().filter(|f| f.alive && sim.data.features[f.def].is_metal_deposit()).count();
    println!("{name}: {cells} metal cells of {} (total {total}), {features} deposit features", w * h);
}

#[test]
#[ignore]
fn feature_values() {
    let install = ta_vfs::install::discover(None).unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let name = std::env::var("TA_TEST_MAP").unwrap_or_else(|_| "Coast To Coast".into());
    let map = TntMap::parse(&vfs.read(&format!("maps/{name}.tnt")).unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{name}.ota")).unwrap()));
    let p = PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::ONE, energy: Fx::ONE, ai: true };
    let sim = Sim::new(types.clone(), data, &map, &info, 0, &[p], 1);
    let mut seen = std::collections::BTreeMap::new();
    for f in &sim.features {
        let d = &sim.data.features[f.def];
        *seen.entry((d.name.clone(), d.reclaimable, d.metal as i64, d.energy as i64, d.description.clone())).or_insert(0) += 1;
    }
    for (k, n) in seen { println!("{n:5} {k:?}"); }
}
