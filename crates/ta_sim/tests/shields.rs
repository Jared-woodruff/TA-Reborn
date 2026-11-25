//! TA Reborn's shields for mods (`ShieldHealth` and the other shield keys: `sim/shield.rs`), played
//! out with made-up units on a flat map (no TA install needed): a bubble stopping its enemies' shots
//! from outside and letting its own side's out, a shot fired from inside it not stopped, a collapse
//! and its recharge, the upkeep (switched off, short of energy), a personal shield taking the damage
//! first, overlapping bubbles each taking what reaches them, an enhancement granting a shield, an
//! aircraft's shots under a dome, a copy playing on as the original; and shots set in flight by hand:
//! a missile flying on through a dome another shot collapsed that tick, a piercing shot stopped at a
//! dome sparing what is just under it, what runs under the water passing under a dome, its blast
//! too.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Projectile, Sim, SimEvent, Target, UnitId, UnitTypes};

/// Game data of the files given (path, text), as a mod's would be read.
fn data_of(files: &[(String, String)]) -> GameData {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta-sim-shields-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_mod_dir(&dir, "test").unwrap();
    let data = GameData::load(&vfs);
    std::fs::remove_dir_all(&dir).ok();
    data
}

fn unit(name: &str, keys: &str) -> (String, String) {
    (format!("units/{}.fbi", name.to_ascii_lowercase()), format!("[UNITINFO]\n{{\nUnitName={name};\nObjectname={name};\nMaxDamage=1000;\nSightDistance=900;\n{keys}\n}}\n"))
}

fn weapon(name: &str, keys: &str, damage: i64) -> (String, String) {
    (format!("weapons/{}.tdf", name.to_ascii_lowercase()), format!("[{name}] {{ range=800; reloadtime=0.5; weaponvelocity=600; lineofsight=1; areaofeffect=16; {keys} [DAMAGE] {{ default={damage}; }} }}"))
}

/// A flat dry map of `cells` × `cells` 16-px cells.
fn sim_of(data: GameData, cells: usize) -> Sim {
    let data = Arc::new(data);
    let types = Arc::new(UnitTypes::compile(&data, &|_| None));
    let map = TntMap { width: cells as u32, height: cells as u32, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; cells * cells], features: vec![FEATURE_NONE; cells * cells], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    let setups: Vec<PlayerSetup> = (0..2).map(|i| PlayerSetup { side: "ARM".into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
    let mut sim = Sim::new(types, data, &map, &MapInfo::default(), 0, &setups, 7);
    for p in &mut sim.players {
        // (Nobody is defeated for want of units while a test sets its pieces out.)
        p.endless = true;
    }
    sim
}

fn put(sim: &mut Sim, name: &str, owner: u8, x: i64, z: i64) -> UnitId {
    sim.spawn_unit(sim.types.id(name).unwrap(), owner, FxVec3::from_ints(x, 60, z), 0, true).unwrap()
}

fn run(sim: &mut Sim, ticks: u32) -> Vec<SimEvent> {
    let mut events = Vec::new();
    for _ in 0..ticks {
        sim.step(&[]);
        events.extend(sim.take_events());
    }
    events
}

fn health(sim: &Sim, id: UnitId) -> Fx {
    sim.unit(id).map_or(Fx::ZERO, |u| u.health)
}

/// A shield's hit points now, and whether it is up.
fn shield(sim: &Sim, id: UnitId) -> Option<(Fx, bool)> {
    sim.unit(id).and_then(|u| u.shield).map(|s| (s.health, s.up))
}

fn attack(sim: &mut Sim, player: u8, units: Vec<UnitId>, target: UnitId) {
    sim.step(&[Command::Order { player, units, order: Order::Attack(Target::Unit(target)), queue: false }]);
}

const GEN: &str = "FootprintX=2; FootprintZ=2; OnOffable=1; ActivateWhenBuilt=1; ShieldHealth=1000; ShieldRegen=60; ShieldRegenDelay=1; ShieldRechargeTime=4; ShieldRadius=300; ShieldHeight=150;";
const DUMMY: &str = "FootprintX=1; FootprintZ=1;";

/// A bubble stops its enemies' shots coming into it from outside (they burst on its dome, sparing what
/// is under it, the dome taking their damage); its own side's shots fired from under it go out; an
/// enemy's fired from inside the dome is not stopped.
#[test]
fn a_bubble_stops_its_enemies_shots_from_outside_and_lets_its_own_out() {
    let mut sim = sim_of(data_of(&[unit("GEN", GEN), unit("DUMMY", DUMMY), unit("GUN", "Weapon1=CANNON; FootprintX=2; FootprintZ=2;"), weapon("CANNON", "", 100)]), 160);
    assert!(sim.types.shields, "the game has shields");
    let generator = put(&mut sim, "GEN", 0, 1200, 1250);
    let ours = put(&mut sim, "DUMMY", 0, 1200, 1100);
    let theirs = put(&mut sim, "DUMMY", 1, 1350, 600);
    let gun = put(&mut sim, "GUN", 1, 1200, 600);
    let own_gun = put(&mut sim, "GUN", 0, 1350, 1250);
    run(&mut sim, 2);
    assert_eq!(shield(&sim, generator), Some((Fx::from_int(1000), true)), "up and whole once built");
    assert!(sim.bubbles().iter().any(|b| b.unit == generator), "its dome listed");
    attack(&mut sim, 1, vec![gun], ours);
    attack(&mut sim, 0, vec![own_gun], theirs);
    let events = run(&mut sim, 30 * 4);
    assert_eq!(health(&sim, ours), Fx::from_int(1000), "what is under the dome is spared");
    let (left, up) = shield(&sim, generator).unwrap();
    assert!(up && left < Fx::from_int(1000), "the dome took the shots: {left:?}");
    assert!(events.iter().any(|e| matches!(e, SimEvent::ShieldHit { unit, .. } if *unit == generator)), "its hits told");
    assert!(health(&sim, theirs) < Fx::from_int(1000), "its own side's shots go out");
    // An enemy's gun under the dome fires at what is under it, unstopped.
    let inside = put(&mut sim, "GUN", 1, 1050, 1250);
    sim.step(&[Command::Order { player: 1, units: vec![gun], order: Order::Stop, queue: false }]);
    attack(&mut sim, 1, vec![inside], ours);
    run(&mut sim, 30 * 3);
    assert!(health(&sim, ours) < Fx::from_int(1000), "a shot fired from inside the dome is not stopped");
}

/// Run out of hit points, a shield collapses (what is under it then takes the shots) and comes back
/// whole once it has recharged; meanwhile, hit no more, it regains its hit points.
#[test]
fn a_shield_collapses_and_recharges() {
    let mut sim = sim_of(data_of(&[unit("GEN", GEN), unit("DUMMY", DUMMY), unit("GUN", "Weapon1=CANNON; FootprintX=2; FootprintZ=2;"), weapon("CANNON", "", 250)]), 160);
    let generator = put(&mut sim, "GEN", 0, 1200, 1250);
    let ours = put(&mut sim, "DUMMY", 0, 1200, 1100);
    let gun = put(&mut sim, "GUN", 1, 1200, 600);
    run(&mut sim, 2);
    attack(&mut sim, 1, vec![gun], ours);
    let mut collapsed_at = None;
    for t in 0..30 * 10 {
        sim.step(&[]);
        if sim.take_events().iter().any(|e| matches!(e, SimEvent::ShieldChanged { unit, up: false, collapsed: true } if *unit == generator)) {
            collapsed_at = Some(t);
            break;
        }
    }
    assert!(collapsed_at.is_some(), "it collapsed");
    assert_eq!(shield(&sim, generator), Some((Fx::ZERO, false)));
    assert!(!sim.bubbles().iter().any(|b| b.unit == generator), "its dome is gone");
    run(&mut sim, 30 * 2);
    assert!(health(&sim, ours) < Fx::from_int(1000), "collapsed, it stops nothing");
    // The gun taken away: after its recharge time (4 s) it is back, whole.
    sim.remove_unit(gun);
    let events = run(&mut sim, 30 * 2 + 5);
    assert!(events.iter().any(|e| matches!(e, SimEvent::ShieldChanged { unit, up: true, .. } if *unit == generator)), "back up");
    assert_eq!(shield(&sim, generator), Some((Fx::from_int(1000), true)), "whole");
    // Hit and left alone, it regains its hit points (60 a second, from a second after).
    let gun = put(&mut sim, "GUN", 1, 1200, 600);
    attack(&mut sim, 1, vec![gun], ours);
    while shield(&sim, generator).unwrap().0 == Fx::from_int(1000) {
        sim.step(&[]);
    }
    sim.remove_unit(gun);
    // (The shots on their way land first.)
    let events = run(&mut sim, 30);
    let last = events.iter().filter(|e| matches!(e, SimEvent::ShieldHit { unit, .. } if *unit == generator)).count();
    let hurt = shield(&sim, generator).unwrap().0;
    let events = run(&mut sim, 30 * 3);
    assert!(!events.iter().any(|e| matches!(e, SimEvent::ShieldHit { .. })), "no more hits ({last} after the gun went)");
    let mended = shield(&sim, generator).unwrap().0;
    assert!(mended > hurt + Fx::from_int(100) && mended <= Fx::from_int(1000), "regained: {hurt:?} to {mended:?}");
}

/// A shield takes its upkeep while switched on; switched off it is down and takes none, and switched
/// on again it comes back up five seconds later with the hit points it had; short of energy it drops
/// as well, and comes back once it has energy again.
#[test]
fn a_shield_takes_its_upkeep_and_drops_without_it() {
    let keys = format!("{GEN} ShieldEnergyUse=30;");
    let mut sim = sim_of(data_of(&[unit("GEN", &keys)]), 128);
    let generator = put(&mut sim, "GEN", 0, 600, 600);
    run(&mut sim, 2);
    assert_eq!(shield(&sim, generator).map(|s| s.1), Some(true));
    let energy = sim.players[0].economy.energy;
    run(&mut sim, 30);
    let spent = energy - sim.players[0].economy.energy;
    assert!(spent >= Fx::from_int(29) && spent <= Fx::from_int(31), "a second's upkeep: {spent:?}");
    let flows = sim.unit_flows(sim.unit(generator).unwrap());
    assert!(flows.energy_use >= Fx::from_int(29) && flows.energy_use <= Fx::from_int(31), "its figures say so: {:?}", flows.energy_use);
    // Switched off: down, for nothing.
    sim.step(&[Command::SetActive { player: 0, units: vec![generator], on: false }]);
    run(&mut sim, 2);
    assert_eq!(shield(&sim, generator).map(|s| s.1), Some(false), "down while switched off");
    let energy = sim.players[0].economy.energy;
    run(&mut sim, 30);
    assert_eq!(sim.players[0].economy.energy, energy, "and it takes nothing");
    // On again: back up five seconds later.
    sim.step(&[Command::SetActive { player: 0, units: vec![generator], on: true }]);
    run(&mut sim, 30 * 4);
    assert_eq!(shield(&sim, generator).map(|s| s.1), Some(false), "still charging");
    run(&mut sim, 30 + 5);
    assert_eq!(shield(&sim, generator).map(|s| s.1), Some(true), "back up");
    // Short of energy: down; with energy again, back.
    sim.players[0].economy.energy = Fx::ZERO;
    run(&mut sim, 3);
    assert_eq!(shield(&sim, generator).map(|s| s.1), Some(false), "down without its energy");
    sim.players[0].economy.energy = Fx::from_int(1000);
    run(&mut sim, 30 * 5 + 5);
    assert_eq!(shield(&sim, generator), Some((Fx::from_int(1000), true)), "back up with its hit points");
}

/// A personal shield takes the damage its unit takes first, as much as it has; the rest, and every
/// hit once it has collapsed, goes to the unit.
#[test]
fn a_personal_shield_takes_the_damage_first() {
    let mut sim = sim_of(data_of(&[unit("BOT", "FootprintX=2; FootprintZ=2; ShieldHealth=250; ShieldRechargeTime=60;"), unit("GUN", "Weapon1=CANNON; FootprintX=2; FootprintZ=2;"), weapon("CANNON", "", 100)]), 128);
    let bot = put(&mut sim, "BOT", 0, 1000, 1000);
    let gun = put(&mut sim, "GUN", 1, 1000, 500);
    run(&mut sim, 2);
    assert!(sim.bubbles().is_empty(), "no dome");
    assert_eq!(shield(&sim, bot), Some((Fx::from_int(250), true)));
    attack(&mut sim, 1, vec![gun], bot);
    let mut taken = Vec::new();
    let mut collapsed = false;
    for _ in 0..30 * 6 {
        sim.step(&[]);
        for e in sim.take_events() {
            match e {
                SimEvent::ShieldHit { unit, damage, .. } if unit == bot => taken.push(damage),
                SimEvent::ShieldChanged { unit, collapsed: true, .. } if unit == bot => collapsed = true,
                _ => {}
            }
        }
    }
    assert_eq!(taken, [Fx::from_int(100), Fx::from_int(100), Fx::from_int(50)], "the shield took what it had");
    assert!(collapsed, "and collapsed");
    let lost = Fx::from_int(1000) - health(&sim, bot);
    assert!(lost >= Fx::from_int(50) && (lost - Fx::from_int(50)).0 % Fx::from_int(100).0 == 0, "the unit took the rest: {lost:?}");
}

/// Overlapping bubbles each take what of a blast reaches them; what is under both is spared.
#[test]
fn overlapping_bubbles_each_take_what_reaches_them() {
    let mut sim = sim_of(data_of(&[unit("GEN", GEN), unit("DUMMY", DUMMY), unit("GUN", "Weapon1=BIG; FootprintX=2; FootprintZ=2;"), weapon("BIG", "areaofeffect=256; edgeeffectiveness=0.5;", 200)]), 160);
    let a = put(&mut sim, "GEN", 0, 1100, 1300);
    let b = put(&mut sim, "GEN", 0, 1300, 1300);
    let ours = put(&mut sim, "DUMMY", 0, 1200, 1250);
    let gun = put(&mut sim, "GUN", 1, 1200, 500);
    run(&mut sim, 2);
    attack(&mut sim, 1, vec![gun], ours);
    run(&mut sim, 30 * 3);
    let (ha, hb) = (shield(&sim, a).unwrap().0, shield(&sim, b).unwrap().0);
    assert!(ha < Fx::from_int(1000) && hb < Fx::from_int(1000), "both domes took it: {ha:?} {hb:?}");
    assert_eq!(health(&sim, ours), Fx::from_int(1000), "under both, spared");
}

/// An enhancement grants its unit a shield while it is installed (its figures' shield); taken out,
/// the shield goes with it. The checksum tells a shield's state.
#[test]
fn an_enhancement_grants_a_shield() {
    let acu = "Builder=1; WorkerTime=100; FootprintX=2; FootprintZ=2; [ENHANCEMENTS] { [SHIELD] { Slot=Back; BuildTime=1; ShieldHealth=400; ShieldEnergyUse=5; } }";
    let mut sim = sim_of(data_of(&[unit("ACU", acu), unit("GUN", "Weapon1=CANNON; FootprintX=2; FootprintZ=2;"), weapon("CANNON", "", 100)]), 128);
    let acu = put(&mut sim, "ACU", 0, 1000, 1000);
    run(&mut sim, 2);
    assert!(sim.shield(sim.unit(acu).unwrap()).is_none() && shield(&sim, acu).is_none(), "none before");
    sim.step(&[Command::Order { player: 0, units: vec![acu], order: Order::Enhance { index: 0, remove: false }, queue: false }]);
    run(&mut sim, 30);
    assert_eq!(sim.unit(acu).unwrap().enhancements, [0]);
    let (make, _) = sim.shield(sim.unit(acu).unwrap()).expect("a shield of its enhancement's");
    assert!(!make.bubble() && make.health == Fx::from_int(400));
    assert_eq!(shield(&sim, acu), Some((Fx::from_int(400), true)));
    let before = sim.checksum();
    let gun = put(&mut sim, "GUN", 1, 1000, 500);
    attack(&mut sim, 1, vec![gun], acu);
    run(&mut sim, 30 * 2);
    assert!(shield(&sim, acu).unwrap().0 < Fx::from_int(400) && health(&sim, acu) == Fx::from_int(1000), "it takes the hits first");
    assert_ne!(sim.checksum(), before);
    sim.remove_unit(gun);
    sim.step(&[Command::Order { player: 0, units: vec![acu], order: Order::Enhance { index: 0, remove: true }, queue: false }]);
    run(&mut sim, 2);
    assert!(sim.shield(sim.unit(acu).unwrap()).is_none() && shield(&sim, acu).is_none(), "taken out, gone");
}

/// A copy of a battle with shields plays on as the original, bit for bit.
#[test]
fn shields_play_the_same_in_a_copy() {
    let mut sim = sim_of(data_of(&[unit("GEN", GEN), unit("DUMMY", DUMMY), unit("GUN", "Weapon1=CANNON; FootprintX=2; FootprintZ=2;"), weapon("CANNON", "", 150)]), 160);
    put(&mut sim, "GEN", 0, 1200, 1250);
    let ours = put(&mut sim, "DUMMY", 0, 1200, 1100);
    let gun = put(&mut sim, "GUN", 1, 1200, 600);
    run(&mut sim, 2);
    attack(&mut sim, 1, vec![gun], ours);
    run(&mut sim, 30);
    let mut copy = sim.clone();
    run(&mut sim, 30 * 8);
    run(&mut copy, 30 * 8);
    assert_eq!(sim.checksum(), copy.checksum());
}

/// An aircraft counts as over every dome (TA's flying heights are low, under a big dome's top): a
/// gunship hovering under an enemy's dome has its shots burst on the dome at once, and what is
/// under it is spared (`Sim::bubble_entry`).
#[test]
fn an_aircraft_under_a_dome_fires_into_it_in_vain() {
    let dome = "FootprintX=2; FootprintZ=2; ShieldHealth=100000; ShieldRadius=700; ShieldHeight=260;";
    let gunship = "CanFly=1; HoverAttack=1; MaxVelocity=3; Acceleration=0.2; BrakeRate=0.4; TurnRate=900; CruiseAlt=90; FootprintX=2; FootprintZ=2; Weapon1=CANNON;";
    let mut sim = sim_of(data_of(&[unit("GEN", dome), unit("DUMMY", DUMMY), unit("SHIP", gunship), weapon("CANNON", "range=260;", 100)]), 160);
    let generator = put(&mut sim, "GEN", 0, 1200, 1250);
    let ours = put(&mut sim, "DUMMY", 0, 1200, 1100);
    let ship = sim.spawn_airborne(sim.types.id("SHIP").unwrap(), 1, FxVec3::from_ints(1150, 0, 1150), 0, None).unwrap();
    run(&mut sim, 2);
    attack(&mut sim, 1, vec![ship], ours);
    let mut fired_under = 0;
    for _ in 0..30 * 6 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::WeaponFired { unit, pos, .. } = e
                && unit == ship
                && sim.bubbles().iter().any(|b| b.unit == generator && b.contains(pos))
            {
                fired_under += 1;
            }
        }
    }
    assert!(fired_under > 0, "it fired from under the dome");
    assert_eq!(health(&sim, ours), Fx::from_int(1000), "what is under the dome is spared");
    assert!(shield(&sim, generator).unwrap().0 < Fx::from_int(100_000), "the dome took the shots");
}

/// A shot set in flight by hand: `weapon` of `owner`'s, at `pos` going `vel` a tick, `age` ticks old.
fn shot(sim: &Sim, id: u32, weapon: &str, owner: UnitId, pos: FxVec3, vel: FxVec3, age: u32) -> Projectile {
    let weapon = sim.weapon_types.by_name(weapon).unwrap().id;
    let owner_player = sim.unit(owner).unwrap().owner;
    Projectile { id, weapon, owner, owner_player, pos, prev_pos: pos, vel, target: None, target_pos: pos + vel.scale(Fx::from_int(100)), age, ttl: 1000, hits: Vec::new(), dead: false }
}

/// Two tactical missiles (a class domes stop, unlike the strategic ones) come into a weak dome in the
/// same tick, their flights planned together (an interceptor is in the air): the first collapses it,
/// and the second, the dome gone, flies on rather than bursting where the dome was.
#[test]
fn a_missile_flies_on_through_a_dome_collapsed_that_tick() {
    let weak = "FootprintX=2; FootprintZ=2; ShieldHealth=100; ShieldRadius=300; ShieldHeight=150;";
    let mut sim = sim_of(data_of(&[unit("GEN", weak), unit("GUN", "Weapon1=MISSILE; Weapon2=ANTI; FootprintX=2; FootprintZ=2;"), weapon("MISSILE", "targetable=2;", 300), weapon("ANTI", "interceptor=2;", 10)]), 160);
    let generator = put(&mut sim, "GEN", 0, 1200, 1250);
    let launcher = put(&mut sim, "GUN", 1, 200, 200);
    let defender = put(&mut sim, "GUN", 0, 2200, 2200);
    run(&mut sim, 2);
    assert!(shield(&sim, generator).is_some_and(|(_, up)| up), "the dome up");
    let first = shot(&sim, 900, "MISSILE", launcher, FxVec3::from_ints(1200, 100, 930), FxVec3::from_ints(0, 0, 40), 5);
    let second = shot(&sim, 901, "MISSILE", launcher, FxVec3::from_ints(1200, 100, 1570), FxVec3::from_ints(0, 0, -40), 5);
    let anti = shot(&sim, 902, "ANTI", defender, FxVec3::from_ints(2400, 300, 2400), FxVec3::from_ints(1, 0, 0), 5);
    sim.projectiles.extend([first, second, anti]);
    sim.take_events();
    let missile = sim.weapon_types.by_name("MISSILE").unwrap().id;
    // (Alike from either side, they reach the dome in the same tick.)
    for _ in 0..30 {
        sim.step(&[]);
        let events = sim.take_events();
        if !events.iter().any(|e| matches!(e, SimEvent::ShieldChanged { unit, collapsed: true, .. } if *unit == generator)) {
            assert!(!events.iter().any(|e| matches!(e, SimEvent::Explosion { weapon, .. } if *weapon == missile)), "nothing burst before the dome collapsed");
            continue;
        }
        assert!(sim.projectiles.iter().any(|p| p.id == 902), "the interceptor still in the air: the flights were planned together");
        let burst = events.iter().filter(|e| matches!(e, SimEvent::Explosion { weapon, .. } if *weapon == missile)).count();
        assert_eq!(burst, 1, "the first burst on the dome, the second flies on");
        assert_eq!(sim.projectiles.iter().filter(|p| p.id == 900 || p.id == 901).count(), 1, "one missile still in the air");
        return;
    }
    panic!("the dome never collapsed");
}

/// A piercing shot (`noexplode`, harming what its flight passes and its area round it) stopped at a
/// dome harms nothing under it, however near the dome's edge it stands.
#[test]
fn a_piercing_shot_stopped_at_a_dome_spares_what_is_under_it() {
    let mut sim = sim_of(data_of(&[unit("GEN", GEN), unit("DUMMY", DUMMY), unit("GUN", "Weapon1=PIERCE; FootprintX=2; FootprintZ=2;"), weapon("PIERCE", "noexplode=1; areaofeffect=128;", 200)]), 160);
    let generator = put(&mut sim, "GEN", 0, 1200, 1250);
    // (Just under the dome's edge, 950 along there.)
    let ours = put(&mut sim, "DUMMY", 0, 1200, 965);
    let gun = put(&mut sim, "GUN", 1, 1200, 400);
    run(&mut sim, 2);
    sim.projectiles.push(shot(&sim, 900, "PIERCE", gun, FxVec3::from_ints(1200, 68, 900), FxVec3::from_ints(0, 0, 40), 20));
    sim.take_events();
    let events = run(&mut sim, 3);
    assert!(events.iter().any(|e| matches!(e, SimEvent::ShieldHit { unit, .. } if *unit == generator)), "it burst on the dome");
    assert_eq!(health(&sim, ours), Fx::from_int(1000), "what is under the dome is spared");
}

/// What runs under the water passes under a dome, and so does its blast: a depth charge going off
/// beside a dome takes nothing off it, and what is under the dome within its reach takes its share.
#[test]
fn what_runs_under_the_water_passes_under_a_dome_its_blast_too() {
    let mut sim = sim_of(data_of(&[unit("GEN", GEN), unit("DUMMY", DUMMY), unit("GUN", "Weapon1=DEPTH; FootprintX=2; FootprintZ=2;"), weapon("DEPTH", "waterweapon=1; areaofeffect=400;", 300)]), 160);
    let generator = put(&mut sim, "GEN", 0, 1200, 1250);
    let ours = put(&mut sim, "DUMMY", 0, 1200, 1000);
    let gun = put(&mut sim, "GUN", 1, 1200, 400);
    run(&mut sim, 2);
    // (Going off on the ground 40 px outside the dome.)
    sim.projectiles.push(shot(&sim, 900, "DEPTH", gun, FxVec3::from_ints(1200, 64, 910), FxVec3::from_ints(0, -8, 0), 20));
    sim.take_events();
    let events = run(&mut sim, 3);
    assert!(events.iter().any(|e| matches!(e, SimEvent::Explosion { weapon, .. } if *weapon == sim.weapon_types.by_name("DEPTH").unwrap().id)), "it went off");
    assert!(!events.iter().any(|e| matches!(e, SimEvent::ShieldHit { .. })), "nothing off the dome");
    assert_eq!(shield(&sim, generator), Some((Fx::from_int(1000), true)), "the dome whole");
    assert!(health(&sim, ours) < Fx::from_int(1000), "what is under the dome in its reach takes its share");
}
