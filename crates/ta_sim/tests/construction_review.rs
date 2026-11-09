//! Exercise installed TA constructors and factories with their original COB scripts.
//! Synthetic regressions live in src/construction_tests.rs; these skip without a TA install.

use std::sync::{Arc, OnceLock};
use ta_sim::{Command, Fx, FxVec3, Locomotion, Order, PlayerSetup, Sim, UnitTypes};

fn original() -> Option<&'static (Arc<ta_data::GameData>, Arc<UnitTypes>)> {
    static DATA: OnceLock<Option<(Arc<ta_data::GameData>, Arc<UnitTypes>)>> = OnceLock::new();
    DATA.get_or_init(|| {
        let install = ta_vfs::install::discover(None).ok()?;
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").ok()?;
        let data = Arc::new(ta_data::GameData::load(&vfs));
        let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
        Some((data, types))
    }).as_ref()
}

fn yard(data: &Arc<ta_data::GameData>, types: &Arc<UnitTypes>, water: bool) -> Sim {
    let map = ta_formats::tnt::TntMap { width: 64, height: 64, sea_level: if water { 128 } else { 0 }, heights: vec![if water { 0 } else { 60 }; 64 * 64], features: vec![ta_formats::tnt::FEATURE_NONE; 64 * 64], tiles: Vec::new(), tile_map: Vec::new(), feature_names: Vec::new(), minimap: Default::default() };
    let players = [PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::from_int(1_000_000), energy: Fx::from_int(1_000_000), ai: false }];
    let mut sim = Sim::new(types.clone(), data.clone(), &map, &ta_data::MapInfo::default(), 0, &players, 17);
    sim.set_starting_resources(0, Fx::from_int(1_000_000), Fx::from_int(1_000_000));
    sim.players[0].endless = true;
    sim
}

#[test]
fn original_constructor_roster_finishes_builds_or_assists_with_its_script() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let mut checked = 0;
    for t in types.types.iter().filter(|t| t.is_builder && !t.is_factory && !t.def.is_airbase) {
        let water = t.locomotion == Locomotion::Naval;
        let mut sim = yard(data, types, water);
        let at = FxVec3::from_ints(500, 60, 500);
        let site = FxVec3::from_ints(600, 60, 500);
        let product = t.build_options.iter().copied()
            .filter(|&o| types.get(o).locomotion == Locomotion::Static && sim.can_place(o, site))
            .min_by_key(|&o| (types.get(o).build_work, o));
        let builder = sim.spawn_unit(t.id, 0, at, 0, true).unwrap();
        let (type_id, order) = if let Some(type_id) = product {
            (type_id, Order::Build { type_id, pos: site })
        } else {
            assert!(t.build_options.is_empty(), "{} has no usable construction option in its own terrain", t.name);
            let type_id = types.id(if water { "ARMTIDE" } else { "ARMSOLAR" }).unwrap();
            let frame = sim.spawn_unit(type_id, 0, site, 0, false).unwrap();
            (type_id, Order::Repair(frame))
        };
        sim.step(&[Command::Order { player: 0, units: vec![builder], order, queue: false }]);
        let deadline = types.get(type_id).build_work.div_ceil(t.work_rate) + 900;
        let mut completed = false;
        for _ in 0..deadline {
            sim.step(&[]);
            if sim.units().any(|u| u.type_id == type_id && u.id != builder && u.is_complete()) { completed = true; break; }
        }
        let b = sim.unit(builder).unwrap();
        assert!(completed, "{} never finished {}: position {:?}, orders {:?}, stance {}, job {:?}", t.name, types.get(type_id).name, b.pos, b.orders, b.in_build_stance, b.build);
        checked += 1;
    }
    assert!(checked >= 20, "checked {checked} original constructors");
    println!("{checked} original constructors completed their work");
}

#[test]
fn original_factory_roster_opens_builds_and_releases_a_product() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let mut checked = 0;
    for t in types.types.iter().filter(|t| t.is_factory) {
        let mut sim = yard(data, types, t.def.min_water_depth > 0.0);
        let factory = sim.spawn_unit(t.id, 0, FxVec3::from_ints(500, 60, 500), 0, true).unwrap();
        let product = t.build_options.iter().copied().min_by_key(|&o| (types.get(o).build_work, o)).unwrap();
        sim.step(&[Command::Produce { player: 0, factory, type_id: product, count: 1 }]);
        let deadline = types.get(product).build_work.div_ceil(t.work_rate) + 900;
        let mut released = false;
        for _ in 0..deadline {
            sim.step(&[]);
            if sim.unit(factory).unwrap().factory_queue.is_empty() {
                released = sim.units().any(|u| u.type_id == product && u.id != factory && u.is_complete());
                break;
            }
        }
        let f = sim.unit(factory).unwrap();
        assert!(released, "{} never released {}: stance {}, active {}, job {:?}", t.name, types.get(product).name, f.in_build_stance, f.activated, f.build);
        checked += 1;
    }
    assert!(checked >= 10, "checked {checked} original factories");
    println!("{checked} original factories completed and released their product");
}
