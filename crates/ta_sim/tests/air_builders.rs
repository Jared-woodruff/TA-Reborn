//! Construction aircraft at work (skipped without a TA install): TA's drift round what they
//! build rather than hang still over it, always within reach of it, and the building goes up as
//! fast as it would with them still.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitTypes};

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
    let mut sim = Sim::new(types, data, &map, &info, 0, &players, 7);
    sim.los_enabled = false;
    Some(sim)
}

#[test]
fn construction_aircraft_drift_round_what_they_build() {
    for (plane, building) in [("ARMCA", "ARMSOLAR"), ("CORCA", "CORSOLAR")] {
        let Some(mut sim) = sim_on("Painted Desert") else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let (p, b) = (sim.types.id(plane).unwrap(), sim.types.id(building).unwrap());
        let site = sim.snap_to_grid(b, FxVec3::from_ints(1700, 0, 1500));
        let ca = sim.spawn_unit(p, 0, FxVec3::from_ints(1500, 0, 1500), 0, true).unwrap();
        sim.step(&[Command::Order { player: 0, units: vec![ca], order: Order::Build { type_id: b, pos: site }, queue: false }]);
        let (mut path, mut last, mut working, mut built_at) = (Fx::ZERO, None::<FxVec3>, 0, None);
        for tick in 0..30 * 120 {
            // (Stores kept full: the flying, not the economy, is under test.)
            let e = &mut sim.players[0].economy;
            (e.metal, e.energy) = (e.metal_capacity, e.energy_capacity);
            sim.step(&[]);
            sim.take_events();
            let u = sim.unit(ca).unwrap();
            if let Some(job) = u.build {
                let fp = sim.types.get(b).footprint;
                assert!(sim.in_build_range(u, site, fp), "{plane} drifted out of reach at tick {tick}");
                working += 1;
                if let Some(l) = last {
                    path += (u.pos - l).xz().length();
                }
                last = Some(u.pos);
                let _ = job;
            }
            if sim.units().any(|o| o.type_id == b && o.is_complete()) {
                built_at = Some(tick);
                break;
            }
        }
        let built_at = built_at.unwrap_or_else(|| panic!("{plane} never finished the {building}"));
        println!("{plane}: {building} built in {:.1} s, {:.0} px flown round it over {:.1} s at work", built_at as f32 / 30.0, path.to_f32(), working as f32 / 30.0);
        assert!(working > 30, "{plane} was at work");
        assert!(path > Fx::from_int(working / 30 * 8), "{plane} drifts while it builds ({:.0} px in {} ticks)", path.to_f32(), working);
    }
}
