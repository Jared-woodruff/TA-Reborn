//! Surface water buildings have an explicit WaterLine, unlike seabed structures.
//! Synthetic placement checks run without an install; production also checks real COBs.

use std::sync::Arc;
use ta_data::{GameData, MapInfo, UnitDef};
use ta_formats::{image::IndexedImage, tdf, tnt::{TntMap, FEATURE_NONE}};
use ta_sim::{Command, Fx, FxVec3, Locomotion, PlayerSetup, Sim, UnitTypes};
use ta_sim::world::PlacementError;

fn players() -> Vec<PlayerSetup> {
    vec![PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO,
        metal: Fx::from_int(500_000), energy: Fx::from_int(500_000), ai: false }]
}

fn synthetic() -> Sim {
    let mut data = GameData::load(&ta_vfs::Vfs::new());
    for (name, fields) in [
        ("SURFACE", "MinWaterDepth=30; WaterLine=22;"),
        ("FRACTION", "MinWaterDepth=2; WaterLine=0.3;"),
        ("UNDERWATER", "MinWaterDepth=15;"),
        ("LAND", "MaxWaterDepth=0;"),
        ("LIMITED", "MinWaterDepth=10; MaxWaterDepth=40; WaterLine=4;"),
        ("FLOATER", "MinWaterDepth=1; WaterLine=10; Floater=1;"),
        ("SHIP", "BMcode=1; MaxVelocity=2; MinWaterDepth=10; Floater=1;"),
        ("SUBMARINE", "BMcode=1; MaxVelocity=2; MinWaterDepth=20; WaterLine=24;"),
    ] {
        let doc = tdf::parse(&format!("[UNITINFO] {{ UnitName={name}; FootprintX=2; FootprintZ=2; MaxDamage=100; {fields} }}"));
        data.units.push(UnitDef::from_tdf(doc.section("UNITINFO").unwrap()).unwrap());
    }
    let data = Arc::new(data);
    let types = Arc::new(UnitTypes::compile(&data, &|_| None));
    // Three level strips: depth 80, depth 20, and dry ground 20 above the sea.
    let map = TntMap {
        width: 96, height: 32, sea_level: 100,
        heights: (0..32).flat_map(|_| (0..96).map(|x| if x < 32 { 20 } else if x < 64 { 80 } else { 120 })).collect(),
        features: vec![FEATURE_NONE; 96 * 32], feature_names: vec![],
        tiles: vec![], tile_map: vec![], minimap: IndexedImage::new(1, 1, 0),
        flags: Vec::new(),
    };
    Sim::new(types, data, &map, &MapInfo::default(), 0, &players(), 42)
}

#[test]
fn static_waterline_places_surface_structures_without_lifting_the_seabed_or_land() {
    let mut sim = synthetic();
    for (index, (name, expected)) in [
        ("SURFACE", Fx::from_int(78)),
        ("FRACTION", Fx::from_int(100) - Fx::from_f32(0.3)),
        ("UNDERWATER", Fx::from_int(20)),
        ("FLOATER", Fx::from_int(100)),
        ("SHIP", Fx::from_int(100)),
        ("SUBMARINE", Fx::from_int(100)),
    ].into_iter().enumerate() {
        let type_id = sim.types.id(name).unwrap();
        let at = FxVec3::from_ints(96 + index as i64 * 48, 999, 128);
        // (A placement's ghost is drawn where the unit will stand.)
        let ghost = sim.resting_pos(type_id, at);
        let id = sim.spawn_unit(type_id, 0, at, 0, true).unwrap();
        assert_eq!(sim.unit(id).unwrap().pos.y, expected, "{name}: waterline and original mobile rules");
        assert_eq!(ghost, sim.unit(id).unwrap().pos, "{name}: the ghost stands where the unit does");
    }
    for (name, x) in [("LAND", 1120), ("SURFACE", 1216)] {
        let type_id = sim.types.id(name).unwrap();
        // Direct spawn is also used by scenarios; even forced water buildings cannot
        // be sunk below dry ground by their WaterLine.
        let id = sim.spawn_unit(type_id, 0, FxVec3::from_ints(x, 0, 128), 0, true).unwrap();
        assert_eq!(sim.unit(id).unwrap().pos.y, Fx::from_int(120), "{name}: dry footprint height");
    }
}

#[test]
fn static_waterline_does_not_relax_minimum_or_maximum_placement_depth() {
    let sim = synthetic();
    let deep = FxVec3::from_ints(128, 0, 128);
    let shallow = FxVec3::from_ints(640, 0, 128);
    let land = FxVec3::from_ints(1152, 0, 128);
    let surface = sim.types.id("SURFACE").unwrap();
    let limited = sim.types.id("LIMITED").unwrap();
    let dry = sim.types.id("LAND").unwrap();
    assert!(sim.can_place(surface, deep));
    assert_eq!(sim.placement_error(surface, shallow), Some(PlacementError::NeedsWater));
    assert_eq!(sim.placement_error(surface, land), Some(PlacementError::NeedsWater));
    assert!(sim.can_place(limited, shallow));
    assert_eq!(sim.placement_error(limited, deep), Some(PlacementError::TooDeep));
    assert!(sim.can_place(dry, land));
    assert_eq!(sim.placement_error(dry, deep), Some(PlacementError::TooDeep));
}

#[test]
fn installed_shipyards_produce_at_their_waterline_and_ships_leave_at_the_surface() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping original shipyard scripts");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|path| vfs.read_opt(path)));
    let map = TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    for name in ["ARMSY", "ARMASY", "CORSY", "CORASY"] {
        let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players(), 42);
        let type_id = types.id(name).unwrap();
        let ty = types.get(type_id);
        assert_eq!(ty.locomotion, Locomotion::Static);
        assert!(ty.def.water_line > 0.0 && !ty.def.floater);
        let mut site = None;
        'find: for z in (256..map.height_px() as i64 - 256).step_by(64) {
            for x in (256..map.width_px() as i64 - 256).step_by(64) {
                let candidate = sim.snap_to_grid(type_id, FxVec3::from_ints(x, 0, z));
                if candidate.y < sim.world.sea_level - Fx::from_int(60) && sim.can_place(type_id, candidate) {
                    site = Some(candidate);
                    break 'find;
                }
            }
        }
        let site = site.expect("a shipyard fits in deep water");
        let expected = sim.world.sea_level - Fx::from_f64(ty.def.water_line);
        // (Where a placement's ghost shows it: where it will stand, not on the sea floor.)
        assert_eq!(sim.resting_pos(type_id, site).y, expected, "{name}: the ghost's height");
        let factory = sim.spawn_unit(type_id, 0, site, 0, true).unwrap();
        assert_eq!(sim.unit(factory).unwrap().pos.y, expected, "{name}: spawn/anchor retains waterline");
        assert_eq!(sim.resting_pos(type_id, site), sim.unit(factory).unwrap().pos, "{name}: the ghost stands where the building does");
        let product = ty.build_options.iter().copied().find(|&id| {
            let child = types.get(id);
            child.locomotion == Locomotion::Naval && child.def.water_line <= 0.0
        }).expect("a surface ship in the build menu");
        sim.set_starting_resources(0, Fx::from_int(500_000), Fx::from_int(500_000));
        sim.step(&[Command::Produce { player: 0, factory, type_id: product, count: 1 }]);
        let mut built = None;
        let mut saw_construction = false;
        for _ in 0..30 * 180 {
            sim.step(&[]);
            assert_eq!(sim.unit(factory).unwrap().pos.y, expected, "{name}: Create/Activate never sink the root");
            if let Some(child) = sim.units().find(|u| u.type_id == product) {
                if !child.is_complete() {
                    saw_construction = true;
                    assert_eq!(child.pos.y, sim.world.sea_level, "{name}: a ship is built at the surface, where it floats once done (the yard's pad is under the water)");
                } else if !child.orders.is_empty() {
                    built = Some(child.id);
                    break;
                }
            }
        }
        let ship = built.unwrap_or_else(|| panic!("{name} did not finish and release its ship"));
        assert!(saw_construction, "{name}: observed the real construction sequence");
        for _ in 0..90 { sim.step(&[]); }
        let child = sim.unit(ship).unwrap();
        assert_eq!(child.pos.y, sim.world.sea_level, "{name}: finished surface ship follows normal naval movement");
        assert!(child.pos.distance_xz(sim.unit(factory).unwrap().pos) > Fx::from_int(40), "{name}: ship leaves the yard");
    }
}
