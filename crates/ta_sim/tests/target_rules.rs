//! TA's targeting rules on its own units, played out: an anti-air K-bot beside a tank never
//! shoots it, even shot at, until it is told to; an aircraft overhead it takes on at once.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Cheat, Command, Fx, FxVec3, Order, PlayerSetup, Sim, Target, UnitId, UnitTypes};

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
    Some(Sim::new(types, data, &map, &info, 0, &players, 5))
}

fn put(sim: &mut Sim, name: &str, owner: u8, x: i64, z: i64) -> UnitId {
    let at = FxVec3::new(Fx::from_int(x), sim.world.height_at(Fx::from_int(x), Fx::from_int(z)), Fx::from_int(z));
    sim.spawn_unit(sim.types.id(name).unwrap(), owner, at, 0, true).unwrap()
}

#[test]
fn anti_air_kbots_leave_tanks_alone_until_told() {
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    // A Jethro (anti-air: `BadTargetCategory=NOTAIR`) and a Raider (`VTOL`) 150 px apart.
    let (x, z) = (1500, 1500);
    let jethro = put(&mut sim, "ARMJETH", 0, x, z);
    let raider = put(&mut sim, "CORRAID", 1, x + 150, z);
    let full = sim.unit(jethro).unwrap().health;
    for _ in 0..30 * 6 {
        sim.step(&[]);
    }
    let j = sim.unit(jethro).unwrap();
    assert_eq!(sim.unit(raider).unwrap().weapons[0].target, Some(Target::Unit(jethro)), "the tank takes the K-bot on");
    assert!(j.health < full, "and hits it");
    assert_eq!(j.weapons[0].target, None, "the anti-air K-bot neither picks the tank nor answers it");
    assert!(sim.unit(raider).unwrap().health == sim.types.get(sim.unit(raider).unwrap().type_id).max_health, "nor hurts it");
    // Told to, it fires at the tank.
    sim.step(&[Command::Order { player: 0, units: vec![jethro], order: Order::Attack(Target::Unit(raider)), queue: false }]);
    for _ in 0..30 * 4 {
        sim.step(&[]);
    }
    let r = sim.unit(raider).unwrap();
    assert!(r.health < sim.types.get(r.type_id).max_health, "ordered, it hits the tank");
}

#[test]
fn anti_air_kbots_take_on_aircraft_and_ground_units_do_not() {
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    // (Everyone seeing everything, ours unhurt: the gunship fights from beyond a K-bot's sight.)
    sim.los_enabled = false;
    // A Jethro and a Peewee (`VTOL`: its gun could hit an aircraft, its rules say not to), and a
    // Core gunship sent to fight its way to them.
    let (x, z) = (1500, 1500);
    let jethro = put(&mut sim, "ARMJETH", 0, x, z);
    let peewee = put(&mut sim, "ARMPW", 0, x, z + 60);
    let rapier = put(&mut sim, "CORAPE", 1, x + 500, z);
    let there = sim.unit(peewee).unwrap().pos;
    sim.step(&[
        Command::Cheat { player: 0, cheat: Cheat::Invulnerable(true) },
        Command::Order { player: 1, units: vec![rapier], order: Order::Fight(there), queue: false },
    ]);
    let (mut jethro_on_it, mut peewee_on_it) = (false, false);
    for _ in 0..30 * 8 {
        sim.step(&[]);
        let alive = |id| sim.unit(id).filter(|u| !u.dead);
        if let Some(u) = alive(peewee) {
            peewee_on_it |= u.weapons[0].target.is_some();
        }
        if alive(jethro).is_some_and(|u| u.weapons[0].target == Some(Target::Unit(rapier))) {
            jethro_on_it = true;
            break;
        }
        if alive(jethro).is_none() || alive(rapier).is_none() {
            println!("tick {}: jethro {} rapier {}", sim.tick, alive(jethro).is_some(), alive(rapier).is_some());
            break;
        }
    }
    assert!(jethro_on_it, "the anti-air K-bot takes the gunship on");
    assert!(!peewee_on_it, "the Peewee neither picks the gunship nor answers it");
}

/// TA set its `NoAutoFire=1` units to hold fire on construction (the Big Bertha, the
/// Intimidator, the Vulcan, the Buzzsaw, the Doomsday Machine and the Annihilator, the last two
/// `StandingFireOrder=2` in their files), as it did the bombers, crawling bombs, silos, EMP
/// launcher and Sniper (`StandingFireOrder=0`); its other armed units fire at will, the
/// anti-nukes (`NoAutoFire=1` too) at nukes.
#[test]
fn ta_units_start_with_tas_fire_orders() {
    let Some(mut sim) = sim_on("Painted Desert") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut wrong = Vec::new();
    let mut x = 300;
    for (names, want) in [
        (&["ARMBRTHA", "CORINT", "ARMVULC", "CORBUZZ", "CORDOOM", "ARMANNI"][..], 0u8),
        (&["ARMTHUND", "CORSHAD", "ARMVADER", "CORROACH", "ARMSILO", "CORSILO", "ARMEMP", "CORTRON", "ARMSNIPE"][..], 0),
        (&["ARMAMD", "CORFMD", "ARMSCAB", "CORMABM"][..], 2),
        (&["ARMLLT", "CORHLT", "ARMGUARD", "CORPUN", "ARMRL", "ARMPW", "CORRAID"][..], 2),
    ] {
        for &name in names {
            let id = put(&mut sim, name, 0, x, 1200);
            x += 160;
            let got = sim.unit(id).unwrap().fire_state;
            if got != want {
                wrong.push(format!("{name}: {got}, TA {want}"));
            }
        }
    }
    assert!(wrong.is_empty(), "fire orders on completion (2 fire at will, 0 hold fire):
{}", wrong.join("
"));
}
