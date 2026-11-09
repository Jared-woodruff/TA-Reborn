use std::sync::Arc;
use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Fx, FxVec3, PlayerSetup, Sim, UnitTypes};

#[test]
#[ignore]
fn debug_first_shot() {
    let install = ta_vfs::install::discover(None).unwrap();
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    let players: Vec<PlayerSetup> = (0..2).map(|i| PlayerSetup { side: "ARM".into(), color: i, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
    let mut sim = Sim::new(types.clone(), data, &map, &info, 0, &players, 99);
    for name in ["ARMSTUMP", "CORAK"] {
        let t = types.by_name(name).unwrap();
        for w in t.weapons.iter().flatten() {
            let w = sim.weapon_types.get(*w);
            println!("{name}: {} range {} vel {}/tick traj {:?} aoe {} dmg {} reload {} fire_state {}", w.name, w.range, w.velocity, w.trajectory, w.aoe_radius, w.default_damage, w.reload_ticks, t.def.standing_fire_order);
        }
    }
    println!("gravity {}", sim.gravity);
    let a = FxVec3::from_ints(2500, 0, 1200);
    let b = a + FxVec3::from_ints(180, 0, 0);
    let s = sim.spawn_unit(types.id("ARMSTUMP").unwrap(), 0, a, 16384, true).unwrap();
    let k = sim.spawn_unit(types.id("CORAK").unwrap(), 1, b, -16384, true).unwrap();
    println!("stumpy at {:?} ak at {:?}", sim.unit(s).unwrap().pos, sim.unit(k).unwrap().pos);
    for tick in 0..60 {
        sim.step(&[]);
        for p in &sim.projectiles {
            if p.age <= 12 { println!("tick {tick} proj {} age {} pos {:?} vel {:?}", p.id, p.age, p.pos, p.vel); }
        }
        for e in sim.take_events() { println!("tick {tick} {e:?}"); }
        let ku = sim.unit(k).unwrap();
        if tick % 10 == 0 { println!("tick {tick} ak weapons {:?} fire_state {}", ku.weapons[0], ku.fire_state); }
    }
}
