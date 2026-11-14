use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitTypes};

#[test]
fn original_commander_scripts_capture_gates_and_tanks_in_the_original_work_time() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|path| vfs.read_opt(path)));
    let map = TntMap { width: 64, height: 64, sea_level: 0, heights: vec![60; 64 * 64], features: vec![FEATURE_NONE; 64 * 64], tiles: Vec::new(), tile_map: Vec::new(), feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    let players: Vec<_> = (0..2).map(|color| PlayerSetup { side: "ARM".into(), color, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
    for (name, health, expected) in [("CORGATE", None, 1800), ("CORGATE", Some(Fx::ONE), 900), ("ARMFLASH", None, 186)] {
        let mut sim = Sim::new(types.clone(), data.clone(), &map, &MapInfo::default(), 0, &players, 7);
        sim.los_enabled = false;
        let commander = sim.spawn_unit(types.id("ARMCOM").unwrap(), 0, FxVec3::from_ints(400, 60, 500), 0, true).unwrap();
        let target = sim.spawn_unit(types.id(name).unwrap(), 1, FxVec3::from_ints(500, 60, 500), 0, true).unwrap();
        if let Some(health) = health { sim.set_health(target, health); }
        sim.step(&[
            Command::Order { player: 0, units: vec![commander], order: Order::SetStates { fire: 0, movement: 0 }, queue: false },
            Command::Order { player: 1, units: vec![target], order: Order::SetStates { fire: 0, movement: 0 }, queue: false },
        ]);
        sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Capture(target), queue: false }]);
        let mut working = 0;
        let mut elapsed = 0;
        while elapsed < expected + 300 && sim.unit(target).unwrap().owner == 1 {
            let before = sim.unit(target).unwrap().capture_work;
            sim.step(&[]);
            let after = sim.unit(target).unwrap();
            working += u64::from(after.capture_work > before || after.owner == 0);
            elapsed += 1;
        }
        assert_eq!(sim.unit(target).unwrap().owner, 0, "{name} must transfer after its capture work");
        assert_eq!(working, expected, "{name} with health {health:?}");
        println!("{name}, health {health:?}: {working} work ticks, {elapsed} elapsed ticks");
    }
}
