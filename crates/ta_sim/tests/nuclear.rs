//! Nuclear missiles against the real game data (skipped without a TA install): where they come
//! down, what they do to a Commander there, one after another, and under an anti-missile
//! defence's cover.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, SimEvent, Target, UnitId, UnitTypes};

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
    let mut sim = Sim::new(types, data, &map, &info, 0, &players, 99);
    // (The silo's side sees what it aims at: an attack order needs its target in sight.)
    sim.los_enabled = false;
    Some(sim)
}

fn rich(sim: &mut Sim) {
    for p in &mut sim.players {
        p.economy.metal_capacity = Fx::from_int(1_000_000);
        p.economy.energy_capacity = Fx::from_int(10_000_000);
        p.economy.metal = Fx::from_int(1_000_000);
        p.economy.energy = Fx::from_int(10_000_000);
    }
}

/// What became of one strike.
#[derive(Debug, Default)]
struct Strike {
    /// Where the warhead went off, if it did.
    blast: Option<FxVec3>,
    intercepted: bool,
    /// The missile's places, a tick at a time, from its launch.
    flight: Vec<FxVec3>,
    /// Where it was sent as it was launched, and where its target stood then.
    aimed: Option<FxVec3>,
    stood: Option<FxVec3>,
}

/// Orders `silo` to attack `target` and plays on until its missile goes off or is
/// intercepted (or `ticks` pass), every tick `each` first (a target walking off, say).
fn strike(sim: &mut Sim, silo: UnitId, target: Target, ticks: u32, mut each: impl FnMut(&mut Sim)) -> Strike {
    let weapon = sim.unit_type(sim.unit(silo).unwrap()).weapons[0].unwrap();
    let owner = sim.unit(silo).unwrap().owner;
    sim.step(&[Command::Order { player: owner, units: vec![silo], order: Order::Attack(target), queue: false }]);
    let mut out = Strike::default();
    for _ in 0..ticks {
        rich(sim);
        each(sim);
        sim.step(&[]);
        if let Some(p) = sim.projectiles.iter().find(|p| p.owner == silo && p.weapon == weapon) {
            out.flight.push(p.pos);
        }
        for e in sim.take_events() {
            match e {
                SimEvent::WeaponFired { unit, target: aimed, .. } if unit == silo => {
                    out.aimed = Some(aimed);
                    out.stood = match target {
                        Target::Unit(id) => sim.unit(id).map(|u| u.pos),
                        Target::Ground(p) => Some(p),
                        Target::Projectile(_) => None,
                    };
                }
                SimEvent::Explosion { weapon: w, pos, .. } if w == weapon => out.blast = Some(pos),
                SimEvent::Intercepted { attacker, .. } if attacker == owner => out.intercepted = true,
                _ => {}
            }
        }
        if out.blast.is_some() || out.intercepted {
            break;
        }
    }
    out
}

/// Lets the dust settle (wrecks fall, blasts set off by the dead go off) and the silo cycle.
fn settle(sim: &mut Sim) {
    for _ in 0..150 {
        rich(sim);
        sim.step(&[]);
    }
}

fn health(sim: &Sim, id: UnitId) -> Option<f32> {
    sim.unit(id).filter(|u| !u.dead).map(|u| u.health.to_f32())
}

/// A silo at a quarter of the way across the map and an enemy Commander at three quarters,
/// on the map's middle line; the silo holds `missiles`.
fn duel(silo: &str, commander: &str, missiles: u32) -> Option<(Sim, UnitId, UnitId)> {
    let mut sim = sim_on("Painted Desert")?;
    let (w, h) = sim.world.size_px();
    let silo_type = sim.types.id(silo)?;
    let silo_at = sim.find_build_site(silo_type, FxVec3::new(w / 8, Fx::ZERO, h / 2), 40)?;
    let silo = sim.spawn_unit(silo_type, 0, silo_at, 0, true)?;
    let victim_type = sim.types.id(commander)?;
    let at = sim.find_build_site(victim_type, FxVec3::new(w * 7 / 8, Fx::ZERO, h / 2), 40)?;
    let victim = sim.spawn_unit(victim_type, 1, at, 0, true)?;
    sim.grant_stockpile(silo, missiles);
    Some((sim, silo, victim))
}

/// TA's strategic missiles (its `cruise` missiles: the nukes, the EMP and tactical missiles)
/// climb out of their tubes, turn straight for their target (no hook round from the north
/// first), cross the map high above everything and come down on it from above: over a
/// quarter of its height out from the target it is still higher than it is far from it, and it
/// lands on the spot. Steered straight at the target from the top of their climb they came in
/// so low that a building in front, or a tall unit beside the target, took the blast.
#[test]
fn strategic_missiles_turn_for_their_target_and_dive_onto_it_from_above() {
    for silo_name in ["ARMSILO", "CORSILO", "ARMEMP", "CORTRON"] {
        let Some((mut sim, silo, victim)) = duel(silo_name, "ARMCOM", 1) else { eprintln!("TA install (or {silo_name}) not found; skipping"); return };
        let goal = sim.unit(victim).unwrap().pos;
        sim.remove_unit(victim);
        let s = strike(&mut sim, silo, Target::Ground(goal), 30 * 120, |_| {});
        let blast = s.blast.unwrap_or_else(|| panic!("{silo_name}: the missile should go off ({s:?})"));
        // (From where it left its tube.)
        let launch = s.flight[0];
        if std::env::var("TA_TRACE").is_ok() {
            for (tick, p) in s.flight.iter().enumerate().filter(|(t, _)| t % 15 == 0 || *t + 40 > s.flight.len()) {
                println!("{silo_name} {tick}: {:?}, {:.0} px out", p.to_f32(), p.distance_xz(goal).to_f32());
            }
        }
        assert!(blast.within_xz(goal, Fx::from_int(8)), "{silo_name}: lands on the spot: {:?} for {:?}", blast.to_f32(), goal.to_f32());
        // (Off the line from the silo to the target: the turn after the climb is towards it.)
        let line = (goal - launch).xz();
        let across = |p: FxVec3| {
            let rel = (p - launch).xz();
            ((rel.x * line.z - rel.z * line.x) / line.length_xz()).abs()
        };
        let wide = s.flight.iter().map(|&p| across(p)).max().unwrap();
        assert!(wide <= Fx::from_int(4), "{silo_name}: flies straight for its target, at most {} px off the line", wide.to_f32());
        // High over the ground until close, then down from above.
        let ground = sim.world.height_at(goal.x, goal.z);
        for &p in &s.flight {
            let out = p.distance_xz(goal);
            let up = p.y - ground;
            if out >= Fx::from_int(800) && out <= line.length_xz() - Fx::from_int(800) {
                assert!(up >= Fx::from_int(400), "{silo_name}: cruises high, {} px up {} px out", up.to_f32(), out.to_f32());
            }
            if out >= Fx::from_int(40) && out <= Fx::from_int(300) {
                assert!(up >= out, "{silo_name}: comes down from above: {} px up {} px out", up.to_f32(), out.to_f32());
            }
        }
    }
}

/// A Commander at the far end of the map, buildings in front of it on the missile's way in:
/// the first nuke comes down on the Commander, leaving it what TA's numbers leave (3,000
/// health, 2,900 damage to a Commander), and the second destroys it. They came in low, went off
/// against the first building in their way and left the Commander standing.
#[test]
fn nukes_come_down_on_a_commander_behind_buildings_and_the_second_destroys_it() {
    for (silo_name, commander, wall) in [("ARMSILO", "CORCOM", "CORLAB"), ("CORSILO", "ARMCOM", "ARMLAB")] {
        let Some((mut sim, silo, victim)) = duel(silo_name, commander, 2) else { eprintln!("TA install not found; skipping"); return };
        let at = sim.unit(victim).unwrap().pos;
        let wall_type = sim.types.id(wall).unwrap();
        // Factories along the way in, beyond the blast's reach of the Commander (their own blasts
        // would hurt it), and the Commander's side's.
        for back in [330, 430, 530, 650] {
            if let Some(p) = sim.find_build_site(wall_type, at - FxVec3::from_ints(back, 0, 0), 2) {
                sim.spawn_unit(wall_type, 1, p, 0, true);
            }
        }
        let max = sim.unit_type(sim.unit(victim).unwrap()).max_health.to_f32();
        let first = strike(&mut sim, silo, Target::Unit(victim), 30 * 120, |_| {});
        let blast = first.blast.unwrap_or_else(|| panic!("{silo_name}: the first nuke should go off"));
        assert!(blast.within_xz(at, Fx::from_int(32)), "{silo_name}: comes down on the Commander at {:?}, not on {:?}", at.to_f32(), blast.to_f32());
        let left = health(&sim, victim);
        assert!(left.is_some_and(|h| h <= max - 2899.0), "{silo_name}: a Commander at ground zero takes TA's 2,900: {left:?} of {max} left");
        settle(&mut sim);
        let second = strike(&mut sim, silo, Target::Unit(victim), 30 * 120, |_| {});
        assert!(second.blast.is_some(), "{silo_name}: the second nuke should go off");
        assert_eq!(health(&sim, victim), None, "{silo_name}: the second nuke destroys the Commander");
    }
}

/// A Commander walking off under strikes ordered at it, with the Retaliator and the Silencer:
/// each missile goes to where the Commander stood as it was launched and comes down there, as
/// TA's did, and the Commander, walked on out of its reach by then, is unharmed. (They followed
/// it wherever it walked and came down on it.)
#[test]
fn nukes_come_down_where_a_commander_on_the_move_stood_at_their_launch() {
    for (silo_name, commander) in [("ARMSILO", "CORCOM"), ("CORSILO", "ARMCOM")] {
        let Some((mut sim, silo, victim)) = duel(silo_name, commander, 1) else { eprintln!("TA install not found; skipping"); return };
        let (w, h) = sim.world.size_px();
        // Up and down the map, as a Commander about its base.
        let ends = [FxVec3::new(w * 7 / 8, Fx::ZERO, h / 5), FxVec3::new(w * 7 / 8, Fx::ZERO, h * 4 / 5)];
        let mut leg = 0;
        let mut walk = |sim: &mut Sim| {
            let Some(u) = sim.unit(victim).filter(|u| !u.dead) else { return };
            if u.orders.is_empty() {
                leg = (leg + 1) % 2;
                sim.give_orders(victim, [Order::Move(ends[leg])]);
            }
        };
        let max = sim.unit_type(sim.unit(victim).unwrap()).max_health.to_f32();
        let s = strike(&mut sim, silo, Target::Unit(victim), 30 * 120, &mut walk);
        let blast = s.blast.unwrap_or_else(|| panic!("{silo_name}: the nuke should go off ({s:?})"));
        let (aimed, stood) = (s.aimed.expect("launched"), s.stood.expect("the Commander there at the launch"));
        let now = sim.unit(victim).filter(|u| !u.dead).map(|u| u.pos);
        println!("{silo_name}: sent at {:?}, the Commander there at {:?}; the blast at {:?}, the Commander then at {:?}", aimed.to_f32(), stood.to_f32(), blast.to_f32(), now.map(|p| p.to_f32()));
        assert!(aimed.within_xz(stood, Fx::from_int(2)), "{silo_name}: sent at where the Commander stood");
        assert!(blast.within_xz(stood, Fx::from_int(16)), "{silo_name}: comes down there, not after the Commander");
        let now = now.unwrap_or_else(|| panic!("{silo_name}: the Commander walks out of it alive"));
        assert!(now.distance_xz(stood) > Fx::from_int(320), "{silo_name}: the Commander walked on, {} px", now.distance_xz(stood).to_f32());
        assert_eq!(health(&sim, victim), Some(max), "{silo_name}: and is unharmed");
    }
}

/// Under an anti-missile defence with one interceptor: the first strike is intercepted (the
/// Commander unharmed), the second, with the defence's stock spent, comes down on it, and a
/// third finishes it.
#[test]
fn an_anti_nuke_stops_one_missile_for_each_interceptor_it_holds() {
    for (silo_name, commander, anti) in [("ARMSILO", "CORCOM", "CORFMD"), ("CORSILO", "ARMCOM", "ARMAMD")] {
        let Some((mut sim, silo, victim)) = duel(silo_name, commander, 3) else { eprintln!("TA install not found; skipping"); return };
        let at = sim.unit(victim).unwrap().pos;
        let anti_type = sim.types.id(anti).unwrap();
        let p = sim.find_build_site(anti_type, at + FxVec3::from_ints(0, 0, 500), 40).unwrap();
        let defence = sim.spawn_unit(anti_type, 1, p, 0, true).unwrap();
        sim.grant_stockpile(defence, 1);
        // (Its next interceptor takes two minutes: none comes in time for the second strike.)
        let max = sim.unit_type(sim.unit(victim).unwrap()).max_health.to_f32();
        let first = strike(&mut sim, silo, Target::Unit(victim), 30 * 120, |_| {});
        assert!(first.intercepted && first.blast.is_none(), "{silo_name}: the first is intercepted: {first:?}");
        assert_eq!(health(&sim, victim), Some(max), "{silo_name}: an intercepted missile harms nobody");
        settle(&mut sim);
        let second = strike(&mut sim, silo, Target::Unit(victim), 30 * 120, |_| {});
        assert!(!second.intercepted && second.blast.is_some_and(|b| b.within_xz(at, Fx::from_int(32))), "{silo_name}: the second gets through: {second:?}");
        assert!(health(&sim, victim).is_some_and(|h| h <= max - 2899.0), "{silo_name}: {:?}", health(&sim, victim));
        settle(&mut sim);
        let third = strike(&mut sim, silo, Target::Unit(victim), 30 * 120, |_| {});
        assert!(third.blast.is_some(), "{silo_name}: the third goes off");
        assert_eq!(health(&sim, victim), None, "{silo_name}: and destroys the Commander");
    }
}

/// Strikes queued one after another (with Shift): the silo launches each in turn as it cycles,
/// each comes down on its own place, and the queue and the stock are spent.
#[test]
fn queued_strikes_launch_in_turn_each_at_its_own_place() {
    let Some((mut sim, silo, victim)) = duel("CORSILO", "ARMCOM", 3) else { eprintln!("TA install not found; skipping"); return };
    let at = sim.unit(victim).unwrap().pos;
    sim.remove_unit(victim);
    let place = |dz: i64| {
        let (x, z) = (at.x, at.z + Fx::from_int(dz));
        FxVec3::new(x, sim.world.height_at(x, z), z)
    };
    let places = [place(0), place(600), place(-600)];
    let weapon = sim.unit_type(sim.unit(silo).unwrap()).weapons[0].unwrap();
    let orders: Vec<Command> = places.iter().enumerate().map(|(i, &p)| Command::Order { player: 0, units: vec![silo], order: Order::Attack(Target::Ground(p)), queue: i > 0 }).collect();
    sim.step(&orders);
    let (mut launches, mut blasts) = (0, Vec::new());
    for _ in 0..30 * 180 {
        rich(&mut sim);
        sim.step(&[]);
        for e in sim.take_events() {
            match e {
                SimEvent::WeaponFired { unit, .. } if unit == silo => launches += 1,
                SimEvent::Explosion { weapon: w, pos, .. } if w == weapon => blasts.push(pos),
                _ => {}
            }
        }
        if blasts.len() == places.len() {
            break;
        }
    }
    assert_eq!((launches, blasts.len()), (3, 3), "three launches, three blasts");
    for (blast, place) in blasts.iter().zip(places) {
        assert!(blast.within_xz(place, Fx::from_int(8)), "each at its own place, in turn: {:?} for {:?}", blast.to_f32(), place.to_f32());
    }
    let u = sim.unit(silo).unwrap();
    assert!(u.orders.is_empty() && u.stockpile == 0, "the queue and the stock are spent: {:?}, {}", u.orders, u.stockpile);
}
