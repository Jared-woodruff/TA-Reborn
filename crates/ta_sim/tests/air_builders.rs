//! Construction aircraft at work (skipped without a TA install): as TA's do, they face what they
//! build and sway round it sideways along an arc rather than hang still over it, smoothly, never
//! flying backwards, always within reach of it; and the building goes up as fast as it would with
//! them still.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::fixed::{heading_of, heading_vector, wrap_angle};
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

fn f(v: Fx) -> f64 {
    v.to_f32() as f64
}

#[test]
fn construction_aircraft_face_what_they_build_and_sway_round_it() {
    // (From the west, and from the north-east: arriving square to a side and across a corner; and
    // a minute's work on a fusion plant, 5 by 4, from the south-west: one aircraft takes a
    // quarter of an hour over it.)
    for (plane, building, from, whole) in [
        ("ARMCA", "ARMSOLAR", (1500, 1500), true),
        ("CORCA", "CORSOLAR", (1900, 1300), true),
        ("ARMACA", "ARMFUS", (1450, 1650), false),
    ] {
        let Some(mut sim) = sim_on("Painted Desert") else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let (p, b) = (sim.types.id(plane).unwrap(), sim.types.id(building).unwrap());
        let site = sim.snap_to_grid(b, FxVec3::from_ints(1700, 0, 1500));
        let fp = sim.types.get(b).footprint;
        let ca = sim.spawn_unit(p, 0, FxVec3::from_ints(from.0, 0, from.1), 0, true).unwrap();
        sim.step(&[Command::Order { player: 0, units: vec![ca], order: Order::Build { type_id: b, pos: site }, queue: false }]);
        let (mut path, mut sideways, mut working, mut swaying, mut built_at) = (0.0, 0.0, 0, 0, None);
        let (mut worst_facing, mut worst_back, mut worst_jolt, mut worst_step) = (0i32, 0.0f64, 0.0f64, 0.0f64);
        let (mut last, mut last_step) = (None::<FxVec3>, None::<(f64, f64)>);
        // (TA_TRACE=1 follows the aircraft tick by tick.)
        let trace = std::env::var_os("TA_TRACE").is_some();
        for tick in 0..30 * 300 {
            // (Stores kept full: the flying, not the economy, is under test.)
            let e = &mut sim.players[0].economy;
            (e.metal, e.energy) = (e.metal_capacity, e.energy_capacity);
            sim.step(&[]);
            sim.take_events();
            let u = sim.unit(ca).unwrap();
            if let Some(job) = u.build {
                let center = sim.unit(job.target).map_or(site, |t| t.pos);
                assert!(sim.in_build_range(u, center, fp), "{plane} drifted out of reach at tick {tick}");
                working += 1;
                if let Some(l) = last {
                    let v = (u.pos - l).xz();
                    let (vx, vz) = (f(v.x), f(v.z));
                    path += vx.hypot(vz);
                    // Along its heading and across it: never backwards, mostly sideways.
                    let ahead = heading_vector(u.heading);
                    let (ax, az) = (f(ahead.x), f(ahead.z));
                    let along = vx * ax + vz * az;
                    sideways += (vx * az - vz * ax).abs();
                    worst_back = worst_back.max(-along);
                    if u.drift.is_some() {
                        swaying += 1;
                        worst_step = worst_step.max(vx.hypot(vz));
                        if let Some((px, pz)) = last_step {
                            worst_jolt = worst_jolt.max((vx - px).hypot(vz - pz));
                        }
                        last_step = Some((vx, vz));
                    }
                }
                last = Some(u.pos);
                if trace {
                    let off = wrap_angle(heading_of((center - u.pos).xz()) - u.heading) as f32 * 360.0 / 65536.0;
                    let (dx, dz) = ((f(u.pos.x - center.x).abs() - fp.0 as f64 * 8.0).max(0.0), (f(u.pos.z - center.z).abs() - fp.1 as f64 * 8.0).max(0.0));
                    let out = dx.hypot(dz);
                    println!("  {tick:5} at ({:.1}, {:.1}) speed {:.2} heading {} facing off {off:.1} deg, {out:.1} px from it, {:?}", f(u.pos.x), f(u.pos.z), f(u.speed), u.heading, u.drift.map(|d| (d.since, d.side)));
                }
                // Facing what it builds while it sways (it turns to face it first).
                if u.drift.is_some() {
                    worst_facing = worst_facing.max(wrap_angle(heading_of((center - u.pos).xz()) - u.heading).abs());
                }
            }
            if sim.units().any(|o| o.type_id == b && o.is_complete()) || !whole && working >= 30 * 60 {
                built_at = Some(tick);
                break;
            }
        }
        let built_at = built_at.unwrap_or_else(|| panic!("{plane} never finished the {building}"));
        println!(
            "{plane}: {building} {} {:.1} s; at work {:.1} s, swaying {:.1} s; flew {path:.0} px, {sideways:.0} px sideways; \
             facing off by {:.1} deg at most, backwards {worst_back:.3} px a tick, fastest {worst_step:.2} px a tick, jolt {worst_jolt:.3} px a tick",
            if whole { "built in" } else { "at work on it after" },
            built_at as f32 / 30.0,
            working as f32 / 30.0,
            swaying as f32 / 30.0,
            worst_facing as f32 * 360.0 / 65536.0,
        );
        assert!(working > 30 && swaying > working / 2, "{plane} was at work, swaying");
        assert!(path > working as f64 / 30.0 * 6.0, "{plane} sways while it builds ({path:.0} px in {working} ticks)");
        assert!(sideways > path * 0.8, "{plane} sways sideways ({sideways:.0} of {path:.0} px)");
        assert!(worst_facing <= 1024, "{plane} faces what it builds (off by {} units)", worst_facing);
        assert!(worst_back <= 0.05, "{plane} never flies backwards ({worst_back:.3} px a tick)");
        assert!(worst_step <= 2.0 && worst_jolt <= 0.1, "{plane} sways smoothly ({worst_step:.2} px a tick, jolt {worst_jolt:.3})");
    }
}
