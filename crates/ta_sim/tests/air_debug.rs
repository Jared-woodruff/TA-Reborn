//! Aircraft behaviour diagnostics (ignored by default).

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, SimEvent, Target, UnitTypes};

fn sim_on(map_name: &str) -> Option<Sim> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map_name}.ota")).ok()?));
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: if i == 0 { "ARM".into() } else { "CORE".into() }, color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    Some(Sim::new(types, data, &map, &info, 0, &players, 99))
}

#[test]
#[ignore]
fn air_debug() {
    let Some(mut sim) = sim_on("Coast To Coast") else { return };
    let a = FxVec3::from_ints(1600, 0, 1200);
    let id = |sim: &Sim, n: &str| sim.types.id(n).unwrap();
    let t = sim.types.get(id(&sim, "ARMTHUND"));
    println!("ARMTHUND speed {:.2} px/tick accel {:.3} brake {:.3} turn {} cruise {:.0}", t.max_speed.to_f32(), t.acceleration.to_f32(), t.brake.to_f32(), t.turn_rate, t.cruise_alt.to_f32());
    let w = sim.weapon_types.get(t.weapons[0].unwrap());
    println!("bomb: range {:.0} reload {} aoe {:.0} traj {:?} burst {} cmd {}", w.range.to_f32(), w.reload_ticks, w.aoe_radius.to_f32(), w.trajectory, w.burst, w.command_fire);
    println!("gravity {:.4} px/tick^2; sea {:.0}; ground at a {:.0}", sim.gravity.to_f32(), sim.world.sea_level.to_f32(), sim.world.height_at(a.x, a.z).to_f32());
    let bomber = sim.spawn_unit(id(&sim, "ARMTHUND"), 0, a + FxVec3::from_ints(-600, 0, 0), 16384, true).unwrap();
    let thud = sim.spawn_unit(id(&sim, "CORTHUD"), 1, a + FxVec3::from_ints(100, 0, 0), -16384, true).unwrap();
    sim.step(&[Command::Order { player: 0, units: vec![bomber], order: Order::Attack(Target::Unit(thud)), queue: false }]);
    for tick in 0..30 * 30 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::WeaponFired { unit, pos, .. } = e && unit == bomber {
                println!("  tick {tick}: bomb released at ({:.0},{:.0},{:.0})", pos.x.to_f32(), pos.y.to_f32(), pos.z.to_f32());
            }
            if let SimEvent::Explosion { pos, .. } = e {
                println!("  tick {tick}: explosion at ({:.0},{:.0},{:.0})", pos.x.to_f32(), pos.y.to_f32(), pos.z.to_f32());
            }
        }
        if tick % 15 == 0 {
            let b = sim.unit(bomber).unwrap();
            let th = sim.unit(thud).map(|u| u.pos).unwrap_or(FxVec3::ZERO);
            println!(
                "tick {tick:4}: pos ({:.0},{:.0},{:.0}) hdg {} spd {:.2} goal {:?} pass {} target {:?} thud ({:.0},{:.0}) hp {:?}",
                b.pos.x.to_f32(), b.pos.y.to_f32(), b.pos.z.to_f32(), b.heading, b.speed.to_f32(),
                b.path_goal.map(|g| (g.x.to_int(), g.z.to_int())), b.air_pass, b.weapons[0].target.is_some(),
                th.x.to_f32(), th.z.to_f32(), sim.unit(thud).map(|u| u.health.to_f32())
            );
        }
    }
}
