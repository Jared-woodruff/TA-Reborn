//! Units and towers don't fire into their own side: a shot with a friend in its way, or with
//! a big blast coming down beside one, is held back (skipped without a TA install).
//! `cargo test --release -p ta_sim --test friendly_fire -- --nocapture`

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Cheat, Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitTypes};

fn sim_on(map_name: &str) -> Option<Sim> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map_name}.ota")).ok()?));
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(10000), energy: Fx::from_int(10000), ai: false })
        .collect();
    let mut sim = Sim::new(types, data, &map, &info, 0, &players, 7);
    sim.cheats_allowed = true;
    Some(sim)
}

/// A level, dry strip `len` px long east-west.
fn flat_spot(sim: &Sim, len: i64) -> Option<FxVec3> {
    let (w, h) = sim.world.size_px();
    let sea = sim.world.sea_level;
    for z in (96..h.to_int() - 96).step_by(32) {
        for x in (96..w.to_int() - len - 96).step_by(32) {
            let hs: Vec<Fx> = (0..=len).step_by(16).map(|dx| sim.world.height_at(Fx::from_int(x + dx), Fx::from_int(z))).collect();
            let (lo, hi) = (hs.iter().min()?, hs.iter().max()?);
            if *lo > sea && *hi - *lo <= Fx::from_int(3) {
                return Some(FxVec3::from_ints(x + 20, 0, z));
            }
        }
    }
    None
}

/// Whether `shooter` hit anything this tick (its target took damage from it).
fn hit_by(sim: &Sim, shooter: ta_sim::UnitId, victim: ta_sim::UnitId) -> bool {
    sim.unit(victim).is_some_and(|v| v.last_hit_tick == sim.tick && v.last_attacker == Some(shooter))
}

/// A Light Laser Tower with a friendly Gaat Gun right in front of it holds its fire at the
/// Stumpy beyond, and fires once the way is clear.
#[test]
fn a_tower_holds_fire_through_a_friend() {
    let Some(mut sim) = sim_on("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let a = flat_spot(&sim, 280).expect("no flat land");
    let llt = sim.spawn_unit(sim.types.id("CORLLT").unwrap(), 1, a, 0, true).unwrap();
    let wall = sim.spawn_unit(sim.types.id("CORHLT").unwrap(), 1, a + FxVec3::from_ints(70, 0, 0), 0, true).unwrap();
    let tank = sim.spawn_unit(sim.types.id("ARMSTUMP").unwrap(), 0, a + FxVec3::from_ints(220, 0, 0), 0, true).unwrap();
    // The Stumpy holds still and holds its fire, and so does the Gaat Gun (it would kill the
    // Stumpy): only the laser tower may shoot.
    sim.step(&[
        Command::SetStates { player: 0, units: vec![tank], fire: Some(0), movement: Some(0) },
        Command::SetStates { player: 1, units: vec![wall], fire: Some(0), movement: None },
    ]);
    let mut through = 0;
    for _ in 0..30 * 6 {
        sim.step(&[]);
        through += hit_by(&sim, llt, tank) as u32;
    }
    assert_eq!(through, 0, "the laser tower shot through the Gaat Gun {through} times");
    assert!(sim.unit(tank).is_some(), "the Stumpy died");
    // The way clear: it fires.
    sim.step(&[Command::Cheat { player: 1, cheat: Cheat::Destroy(vec![wall]) }]);
    let mut clear = 0;
    for _ in 0..30 * 6 {
        sim.step(&[]);
        clear += hit_by(&sim, llt, tank) as u32;
        if sim.unit(tank).is_none() {
            clear += 1;
            break;
        }
    }
    assert!(clear > 0, "with the way clear the laser tower never fired");
}

/// A crowd of Peewees storming a Core line in depth (the siege demo, small): how often a
/// unit is hurt by its own side, and how often the line fires at all.
#[test]
fn a_siege_has_no_friendly_fire() {
    let Some(mut sim) = sim_on("Gasplant Plain") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let (w, h) = sim.world.size_px();
    let (cx, cz) = (w.to_int() / 2, h.to_int() / 2);
    let front = cx + 300;
    for (name, back, step, first) in [("CORLLT", 0, 72, 0), ("CORHLT", 80, 120, 40), ("CORTOAST", 170, 200, 100), ("CORDOOM", 270, 300, 150), ("CORPUN", 400, 210, 0)] {
        let t = sim.types.id(name).unwrap();
        let mut z = cz - 500 + first;
        while z <= cz + 500 {
            if let Some(p) = sim.find_build_site(t, FxVec3::from_ints(front + back, 0, z), 6) {
                sim.spawn_unit(t, 1, p, -16384, true);
            }
            z += step;
        }
    }
    let pw = sim.types.id("ARMPW").unwrap();
    let mut cmds = Vec::new();
    for col in 0..30 {
        let z = cz - 500 + col * 34;
        let mut ids = Vec::new();
        for row in 0..20 {
            if let Some(id) = sim.spawn_unit(pw, 0, FxVec3::from_ints(front - 700 - row * 34, 0, z), 16384, true) {
                ids.push(id);
            }
        }
        cmds.push(Command::Order { player: 0, units: ids, order: Order::Fight(FxVec3::from_ints(front + 900, 0, z)), queue: false });
    }
    sim.step(&cmds);
    let (mut friendly, mut hostile) = (0u32, 0u32);
    for _ in 0..30 * 40 {
        sim.step(&[]);
        for u in sim.units() {
            if u.last_hit_tick != sim.tick || u.dead {
                continue;
            }
            let Some(src) = u.last_attacker.and_then(|a| sim.unit(a)) else { continue };
            if src.id == u.id {
                continue;
            }
            if sim.allied(src.owner, u.owner) { friendly += 1 } else { hostile += 1 }
        }
    }
    println!("siege, 40 s: {hostile} hits on the enemy, {friendly} on a unit's own side");
    assert!(hostile > 200, "the battle never got going ({hostile} hits)");
    assert!(friendly * 100 <= hostile, "{friendly} friendly hits against {hostile} on the enemy");
}
