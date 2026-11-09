//! Every unit's script through its life, against the real game data (skipped without a TA
//! install; run with `cargo test --release -p ta_sim --test unit_scripts -- --ignored
//! --nocapture`, `TA_UNITS=armsolar,corsolar` for a few). Each unit is built, switched off
//! and on, moved, set on a target, set building or producing, and blown up; the test
//! reports the script engine's warnings for it (unknown opcodes, stack overflows, runaway
//! loops) and pieces its animations leave implausibly far from the unit.

use std::sync::{Arc, Mutex};

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Locomotion, Order, PlayerSetup, Sim, Target, UnitTypes};

/// Script warnings logged while the current unit was being run.
static WARNINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            WARNINGS.lock().unwrap().push(format!("{}", r.args()));
        }
    }
    fn flush(&self) {}
}

fn run(sim: &mut Sim, commands: &[Command], ticks: u32) {
    sim.step(commands);
    for _ in 1..ticks {
        sim.step(&[]);
    }
}

/// A spot on dry, level land and one in open water deep enough for ships.
fn spots(sim: &Sim) -> (FxVec3, Option<FxVec3>) {
    let (w, h) = sim.world.size_px();
    let (w, h) = (w.to_int(), h.to_int());
    let sea = sim.world.sea_level;
    let mut land = None;
    let mut water = None;
    let at = |x: i64, z: i64| sim.world.height_at(Fx::from_int(x), Fx::from_int(z));
    let mut z = 256;
    while z < h - 256 && (land.is_none() || water.is_none()) {
        let mut x = 256;
        while x < w - 256 {
            let here = at(x, z);
            // Level over a unit's width, and well clear of the shore.
            let around = [at(x - 96, z), at(x + 96, z), at(x, z - 96), at(x, z + 96), at(x + 160, z + 160), at(x - 160, z - 160)];
            if land.is_none() && here > sea && around.iter().all(|a| (*a - here).abs() < Fx::from_int(6) && *a > sea) {
                land = Some(FxVec3::new(Fx::from_int(x), here, Fx::from_int(z)));
            }
            if water.is_none() && here < sea - Fx::from_int(30) && around.iter().all(|a| *a < sea - Fx::from_int(20)) {
                water = Some(FxVec3::new(Fx::from_int(x), sea, Fx::from_int(z)));
            }
            x += 64;
        }
        z += 64;
    }
    (land.expect("no level land on the test map"), water)
}

#[test]
#[ignore]
fn every_unit_script() {
    let Ok(install) = ta_vfs::install::discover(None) else {
        eprintln!("TA install not found; skipping");
        return;
    };
    if log::set_logger(&Capture).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read("maps/Coast To Coast.tnt").unwrap()).unwrap();
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read("maps/Coast To Coast.ota").unwrap()));
    let only: Option<Vec<String>> = std::env::var("TA_UNITS").ok().map(|v| v.split(',').map(|s| s.trim().to_ascii_uppercase()).collect());
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(50_000), energy: Fx::from_int(500_000), ai: false })
        .collect();
    let target_type = types.id("CORGOL").or_else(|| types.id("ARMBULL")).unwrap();
    let (mut tested, mut problems) = (0, Vec::new());
    for t in &types.types {
        if only.as_ref().is_some_and(|o| !o.contains(&t.name.to_ascii_uppercase())) || t.script.is_none() {
            continue;
        }
        tested += 1;
        WARNINGS.lock().unwrap().clear();
        let mut sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 7);
        let (land, water) = spots(&sim);
        let wet = t.def.min_water_depth > 0.0 || t.locomotion == Locomotion::Naval;
        let Some(at) = (if wet { water } else { Some(land) }) else { continue };
        let Some(id) = sim.spawn_unit(t.id, 0, at, 0, true) else {
            problems.push(format!("{}: could not be placed", t.name));
            continue;
        };
        run(&mut sim, &[], 90);
        let mut issues = Vec::new();
        // Animation poses left where the unit is: nothing should drift far off.
        let far = |sim: &Sim, stage: &str, issues: &mut Vec<String>| {
            let Some(u) = sim.unit(id) else { return };
            let reach = (t.radius * 4).max(Fx::from_int(160));
            for (i, piece) in t.model.pieces.iter().enumerate() {
                let p = ta_sim::unit::piece_world_position(u, t, u.cob.as_ref(), i);
                if !p.within_xz(u.pos, reach) || (p.y - u.pos.y).abs() > reach {
                    issues.push(format!("{stage}: piece {} at {} px from the unit", piece.name, (p - u.pos).length().to_int()));
                }
            }
        };
        far(&sim, "built", &mut issues);
        if t.def.on_offable {
            run(&mut sim, &[Command::SetActive { player: 0, units: vec![id], on: false }], 60);
            run(&mut sim, &[Command::SetActive { player: 0, units: vec![id], on: true }], 90);
            far(&sim, "switched on", &mut issues);
        }
        if t.max_speed > Fx::ZERO {
            let to = at + FxVec3::new(Fx::from_int(180), Fx::ZERO, Fx::from_int(60));
            run(&mut sim, &[Command::Order { player: 0, units: vec![id], order: Order::Move(to), queue: false }], 150);
            far(&sim, "moving", &mut issues);
        }
        if t.weapons.iter().any(|w| w.is_some()) {
            let here = sim.unit(id).map_or(at, |u| u.pos);
            if let Some(target) = sim.spawn_unit(target_type, 1, here + FxVec3::new(Fx::from_int(150), Fx::ZERO, Fx::from_int(150)), 0, true) {
                run(&mut sim, &[Command::Order { player: 0, units: vec![id], order: Order::Attack(Target::Unit(target)), queue: false }], 150);
                far(&sim, "attacking", &mut issues);
            }
        }
        if t.is_builder && !t.build_options.is_empty() {
            let here = sim.unit(id).map_or(at, |u| u.pos);
            let cmd = if t.is_factory {
                t.build_options.iter().copied().find(|&o| sim.types.get(o).locomotion != Locomotion::Static).map(|o| Command::Produce { player: 0, factory: id, type_id: o, count: 1 })
            } else {
                let site = here + FxVec3::new(Fx::from_int(-120), Fx::ZERO, Fx::from_int(120));
                t.build_options.first().map(|&o| Command::Order { player: 0, units: vec![id], order: Order::Build { type_id: o, pos: site }, queue: false })
            };
            if let Some(cmd) = cmd {
                run(&mut sim, &[cmd], 240);
                far(&sim, "building", &mut issues);
            }
        }
        run(&mut sim, &[Command::SelfDestruct { player: 0, units: vec![id] }], 240);
        let warnings = std::mem::take(&mut *WARNINGS.lock().unwrap());
        issues.extend(warnings);
        issues.sort();
        issues.dedup();
        for i in issues {
            problems.push(format!("{}: {i}", t.name));
        }
    }
    println!("{tested} unit scripts run, {} problems", problems.len());
    for p in &problems {
        println!("  {p}");
    }
    assert!(problems.iter().all(|p| !p.contains("unknown COB opcode")), "every opcode is implemented");
}
