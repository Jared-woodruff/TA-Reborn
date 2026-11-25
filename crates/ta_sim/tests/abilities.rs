//! TA Reborn's abilities for mods, played out with made-up units on a flat map (no TA install
//! needed): a weapon's minimum range (`minrange`), weapon toggles (`[TOGGLE]`,
//! `Command::SetToggle`), stealth fields (`RadarStealthField`, `SonarStealthField`) and jamming
//! granted by an enhancement, remote viewing (`RemoteViewRadius`, `Order::RemoteView`), stuns
//! (`paralyzetime`), regeneration auras (`RegenAuraRadius`) and an enhancement's own production and
//! upkeep (resource allocation).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::{FEATURE_NONE, TntMap};
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Seen, Sim, SimEvent, Target, UnitId, UnitTypes};

/// Game data of the files given (path, text), as a mod's would be read.
fn data_of(files: &[(String, String)]) -> GameData {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("ta-sim-abilities-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
    (format!("units/{}.fbi", name.to_ascii_lowercase()), format!("[UNITINFO]\n{{\nUnitName={name};\nObjectname={name};\nMaxDamage=1000;\nSightDistance=600;\n{keys}\n}}\n"))
}

fn weapon(name: &str, keys: &str) -> (String, String) {
    (format!("weapons/{}.tdf", name.to_ascii_lowercase()), format!("[{name}] {{ range=400; reloadtime=0.5; weaponvelocity=600; lineofsight=1; {keys} [DAMAGE] {{ default=10; }} }}"))
}

/// A flat map of `cells` × `cells` 16-px cells, dry; with `sea` the western half under the sea.
fn sim_of(data: GameData, cells: usize, sea: bool) -> Sim {
    let data = Arc::new(data);
    let types = Arc::new(UnitTypes::compile(&data, &|_| None));
    let heights = (0..cells * cells).map(|i| if sea && i % cells < cells / 2 { 10 } else { 60 }).collect();
    let map = TntMap { width: cells as u32, height: cells as u32, sea_level: if sea { 40 } else { 0 }, tiles: Vec::new(), tile_map: Vec::new(), heights, features: vec![FEATURE_NONE; cells * cells], feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
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

const TANK: &str = "CanMove=1; BMcode=1; MaxVelocity=2; Acceleration=0.5; BrakeRate=1; TurnRate=2000; FootprintX=2; FootprintZ=2; MovementClass=TANKSH2;";

/// A gun with a minimum range fires past it, never within it: of two enemies it takes the farther,
/// and once that one is gone it holds its fire on the nearer.
#[test]
fn a_weapon_fires_nothing_nearer_than_its_minimum_range() {
    let mut sim = sim_of(data_of(&[unit("ART", "Weapon1=SHELL;"), unit("DUMMY", "FootprintX=1; FootprintZ=1;"), weapon("SHELL", "range=600; minrange=200;")]), 128, false);
    let art = put(&mut sim, "ART", 0, 600, 600);
    let near = put(&mut sim, "DUMMY", 1, 720, 600);
    let far = put(&mut sim, "DUMMY", 1, 600, 1000);
    run(&mut sim, 30 * 3);
    assert!(health(&sim, far) < Fx::from_int(1000), "the one past its minimum range is fired on");
    assert_eq!(health(&sim, near), Fx::from_int(1000), "the one within it is not");
    sim.remove_unit(far);
    run(&mut sim, 30 * 3);
    assert_eq!(health(&sim, near), Fx::from_int(1000), "alone within it, it is still not");
    assert!(sim.unit(art).unwrap().weapons[0].target.is_none_or(|t| t != Target::Unit(near)), "nor kept as a target");
    // TA's weapons, without one, fire as near as they like.
    let w = sim.weapon_types.get(sim.unit(art).unwrap().weapon_slots(sim.unit_type(sim.unit(art).unwrap()))[0].unwrap());
    assert_eq!(w.min_range, Fx::from_int(200));
}

/// Told to attack a unit within its minimum range, a mobile unit backs off between its minimum and
/// its maximum range, and fires from there.
#[test]
fn an_attack_order_keeps_its_unit_between_its_minimum_and_maximum_range() {
    let mut sim = sim_of(data_of(&[unit("MOBILEART", &format!("{TANK} Weapon1=SHELL;")), unit("DUMMY", "FootprintX=1; FootprintZ=1;"), weapon("SHELL", "range=600; minrange=250;")]), 128, false);
    let art = put(&mut sim, "MOBILEART", 0, 900, 900);
    let near = put(&mut sim, "DUMMY", 1, 1000, 900);
    sim.step(&[Command::Order { player: 0, units: vec![art], order: Order::Attack(Target::Unit(near)), queue: false }]);
    let mut farthest = Fx::ZERO;
    for _ in 0..30 * 15 {
        sim.step(&[]);
        let (a, n) = (sim.unit(art).unwrap().pos, sim.unit(near).unwrap().pos);
        farthest = farthest.max(a.distance_xz(n));
        if health(&sim, near) < Fx::from_int(1000) {
            break;
        }
    }
    assert!(farthest >= Fx::from_int(250), "it backed off past its minimum range: {farthest:?}");
    assert!(health(&sim, near) < Fx::from_int(1000), "and fired from there");
    let d = sim.unit(art).unwrap().pos.distance_xz(sim.unit(near).unwrap().pos);
    assert!(d <= Fx::from_int(600), "within its reach: {d:?}");
}

/// A weapon toggle swaps the slots it names (one emptied, another filled) on and back off, its
/// script told and an event following; a unit without one is not switched; the checksum tells it.
#[test]
fn a_toggle_swaps_its_units_weapons() {
    let sniper = "Weapon1=GUN; [TOGGLE] { Name=Long Gun; Weapon1=; Weapon2=LONG; }";
    let mut sim = sim_of(data_of(&[unit("SNIPER", sniper), unit("DUMMY", "FootprintX=1; FootprintZ=1;"), weapon("GUN", "range=200;"), weapon("LONG", "range=600;")]), 128, false);
    let (gun, long) = (sim.data.weapon_id("GUN").unwrap(), sim.data.weapon_id("LONG").unwrap());
    let s = put(&mut sim, "SNIPER", 0, 600, 600);
    let plain = put(&mut sim, "DUMMY", 0, 300, 300);
    let target = put(&mut sim, "DUMMY", 1, 600, 1000);
    let slots = |sim: &Sim| *sim.unit(s).unwrap().weapon_slots(sim.unit_type(sim.unit(s).unwrap()));
    assert_eq!(slots(&sim)[..2], [Some(gun), None]);
    assert!(sim.can_toggle(sim.unit(s).unwrap()) && !sim.can_toggle(sim.unit(plain).unwrap()));
    run(&mut sim, 30 * 2);
    assert_eq!(health(&sim, target), Fx::from_int(1000), "out of its own gun's reach");
    let before = sim.checksum();
    sim.step(&[Command::SetToggle { player: 0, units: vec![s, plain], on: true }]);
    let events = sim.take_events();
    assert!(events.iter().any(|e| matches!(e, SimEvent::UnitToggled { id, on: true } if *id == s)));
    assert!(sim.unit(s).unwrap().toggled && !sim.unit(plain).unwrap().toggled, "only what has a toggle");
    assert_eq!(slots(&sim)[..2], [None, Some(long)], "its own emptied, the long gun in its slot");
    assert_ne!(sim.checksum(), before);
    run(&mut sim, 30 * 3);
    assert!(health(&sim, target) < Fx::from_int(1000), "the long gun reaches it");
    // Another player's command switches nothing of ours; off, its own again.
    sim.step(&[Command::SetToggle { player: 1, units: vec![s], on: false }]);
    assert!(sim.unit(s).unwrap().toggled);
    sim.step(&[Command::SetToggle { player: 0, units: vec![s], on: false }]);
    assert!(!sim.unit(s).unwrap().toggled);
    assert_eq!(slots(&sim)[..2], [Some(gun), None]);
}

/// A toggle flipped and flipped back readies no gun sooner: a slot's reload runs on across the
/// switch (and while the slot is empty), so the long gun fired waits out its reload whatever the
/// toggling; waited out, it fires again.
#[test]
fn toggling_readies_no_gun_sooner() {
    let sniper = "Weapon1=GUN; [TOGGLE] { Name=Long Gun; Weapon1=; Weapon2=LONG; }";
    let mut sim = sim_of(data_of(&[unit("SNIPER", sniper), unit("DUMMY", "FootprintX=1; FootprintZ=1;"), weapon("GUN", "range=200;"), weapon("LONG", "range=600; reloadtime=10;")]), 128, false);
    let s = put(&mut sim, "SNIPER", 0, 600, 600);
    let _target = put(&mut sim, "DUMMY", 1, 600, 1000);
    let reload = |sim: &Sim| sim.unit(s).unwrap().weapons[1].reload;
    sim.step(&[Command::SetToggle { player: 0, units: vec![s], on: true }]);
    // (Until the long gun has fired: its reload set, ten seconds.)
    let mut ticks = 0;
    while reload(&sim) == 0 && ticks < 30 * 5 {
        run(&mut sim, 1);
        ticks += 1;
    }
    let fired = reload(&sim);
    assert!(fired > 30 * 5, "the long gun fired, its reload running: {fired}");
    // Flipped and flipped back at once: not ready.
    sim.step(&[Command::SetToggle { player: 0, units: vec![s], on: false }]);
    sim.step(&[Command::SetToggle { player: 0, units: vec![s], on: true }]);
    assert!(reload(&sim) > 0 && reload(&sim) <= fired, "its reload ran on through the switch: {}", reload(&sim));
    // Off for a while: the reload runs down meanwhile, and on again it is that much nearer.
    sim.step(&[Command::SetToggle { player: 0, units: vec![s], on: false }]);
    let before = reload(&sim);
    run(&mut sim, 30 * 4);
    assert!(reload(&sim) + 30 * 4 - 2 <= before, "it ran down in the empty slot: {} from {before}", reload(&sim));
    run(&mut sim, 30 * 7);
    sim.step(&[Command::SetToggle { player: 0, units: vec![s], on: true }]);
    assert_eq!(reload(&sim), 0, "waited out: ready on its return");
}

/// A launcher toggled away loses the missiles it built, as a swapped enhancement's does.
#[test]
fn a_toggled_launcher_loses_its_missiles() {
    let silo = "Weapon1=NUKE; [TOGGLE] { Weapon1=GUN; }";
    let mut sim = sim_of(data_of(&[unit("SILO", silo), weapon("NUKE", "stockpile=1; commandfire=1; reloadtime=1; metalpershot=1; energypershot=1;"), weapon("GUN", "range=200;")]), 64, false);
    let s = put(&mut sim, "SILO", 0, 400, 400);
    run(&mut sim, 30 * 3);
    assert!(sim.unit(s).unwrap().stockpile >= 1, "missiles built");
    sim.step(&[Command::SetToggle { player: 0, units: vec![s], on: true }]);
    assert_eq!(sim.unit(s).unwrap().stockpile, 0, "lost with the launcher swapped");
}

/// A stealth field hides its side's units round it from enemy radar (and a sonar field from sonar),
/// as `Stealth=1` hides one; an enemy's field hides nothing of ours; an enhancement grants jamming.
#[test]
fn stealth_fields_and_jamming_hide_units_from_radar_and_sonar() {
    let mut sim = sim_of(
        data_of(&[
            unit("TANKY", "FootprintX=2; FootprintZ=2;"),
            unit("RADAR", "RadarDistance=3000; SonarDistance=3000; SightDistance=40; FootprintX=1; FootprintZ=1;"),
            unit("FIELD", "RadarStealthField=300; SightDistance=40; FootprintX=2; FootprintZ=2;"),
            unit("SONARFIELD", "SonarStealthField=300; SightDistance=40; FootprintX=1; FootprintZ=1; MinWaterDepth=5; Floater=1;"),
            unit("SUB", "CanMove=1; BMcode=1; MaxVelocity=2; MinWaterDepth=20; WaterLine=24; FootprintX=2; FootprintZ=2; SightDistance=40;"),
            unit("ACU", "Builder=1; WorkerTime=100; SightDistance=40; FootprintX=2; FootprintZ=2; [ENHANCEMENTS] { [JAM] { Slot=Back; BuildTime=1; RadarDistanceJam=300; EnergyUse=5; } }"),
        ]),
        128,
        true,
    );
    put(&mut sim, "RADAR", 1, 1900, 100);
    let tank = put(&mut sim, "TANKY", 0, 1500, 1500);
    // (A submarine, under the water: radar never sees it, sonar does.)
    let ship = put(&mut sim, "SUB", 0, 400, 1500);
    let acu = put(&mut sim, "ACU", 0, 1700, 600);
    run(&mut sim, 12);
    let seen = |sim: &Sim, id: UnitId| sim.seen_by(sim.unit(id).unwrap(), 1);
    assert_eq!((seen(&sim, tank), seen(&sim, ship), seen(&sim, acu)), (Seen::Radar, Seen::Radar, Seen::Radar), "on radar and sonar");
    // An enemy's field over ours hides nothing; its own side's does.
    put(&mut sim, "FIELD", 1, 1550, 1500);
    run(&mut sim, 12);
    assert_eq!(seen(&sim, tank), Seen::Radar, "the enemy's field hides nothing of ours");
    let field = put(&mut sim, "FIELD", 0, 1450, 1500);
    put(&mut sim, "SONARFIELD", 0, 450, 1550);
    run(&mut sim, 12);
    assert_eq!(seen(&sim, tank), Seen::Hidden, "hidden from radar under its side's field");
    assert_eq!(seen(&sim, ship), Seen::Hidden, "hidden from sonar under its side's sonar field");
    assert_eq!(sim.seen_by(sim.unit(field).unwrap(), 1), Seen::Hidden, "the generator too");
    // Jamming an enhancement grants: hidden once installed.
    assert_eq!(seen(&sim, acu), Seen::Radar);
    sim.step(&[Command::Order { player: 0, units: vec![acu], order: Order::Enhance { index: 0, remove: false }, queue: false }]);
    run(&mut sim, 30);
    assert_eq!(sim.unit(acu).unwrap().enhancements, [0]);
    assert_eq!(seen(&sim, acu), Seen::Hidden, "its jamming hides it");
}

/// Remote viewing sees round a far place while the order stands and it pays; Stop ends it, and
/// short of energy it sees nothing; a unit without it is not given the order.
#[test]
fn remote_viewing_shows_a_far_place_while_it_is_paid_for() {
    let mut sim = sim_of(data_of(&[unit("EYE", "RemoteViewRadius=200; RemoteViewEnergy=30; SightDistance=40; FootprintX=2; FootprintZ=2;"), unit("TANKY", "FootprintX=2; FootprintZ=2; SightDistance=40;")]), 128, false);
    let eye = put(&mut sim, "EYE", 0, 100, 100);
    let plain = put(&mut sim, "TANKY", 0, 200, 100);
    let enemy = put(&mut sim, "TANKY", 1, 1600, 1600);
    let place = FxVec3::from_ints(1550, 0, 1550);
    run(&mut sim, 12);
    assert!(!sim.in_sight(0, place) && sim.seen_by(sim.unit(enemy).unwrap(), 0) == Seen::Hidden);
    let energy = sim.players[0].economy.energy;
    sim.step(&[Command::Order { player: 0, units: vec![eye, plain], order: Order::RemoteView(place), queue: false }]);
    assert!(sim.unit(plain).unwrap().orders.is_empty(), "a unit that cannot view from afar is not given the order");
    run(&mut sim, 30);
    assert!(sim.in_sight(0, place), "the place in sight");
    assert_eq!(sim.seen_by(sim.unit(enemy).unwrap(), 0), Seen::Visible, "and the enemy there");
    assert!(sim.viewing(sim.unit(eye).unwrap()).is_some());
    let spent = energy - sim.players[0].economy.energy;
    assert!(spent >= Fx::from_int(28) && spent <= Fx::from_int(32), "its energy, a second's: {spent:?}");
    sim.step(&[Command::Order { player: 0, units: vec![eye], order: Order::Stop, queue: false }]);
    run(&mut sim, 12);
    assert!(!sim.in_sight(0, place) && sim.unit(eye).unwrap().remote_view.is_none(), "Stop ends it");
    // Short of energy: nothing seen.
    sim.players[0].economy.energy = Fx::ZERO;
    sim.step(&[Command::Order { player: 0, units: vec![eye], order: Order::RemoteView(place), queue: false }]);
    run(&mut sim, 12);
    assert!(!sim.in_sight(0, place), "nothing seen without its energy");
    assert_eq!(sim.unit(eye).unwrap().orders.front(), Some(&Order::RemoteView(place)), "the order stands, waiting for energy");
}

/// A unit viewing from afar that its owner drives sees nothing there meanwhile: its orders, and so
/// its view's payment, wait; let go, it views (and pays) again.
#[test]
fn remote_viewing_waits_while_its_unit_is_driven() {
    let mut sim = sim_of(data_of(&[unit("SEER", &format!("{TANK} RemoteViewRadius=200; RemoteViewEnergy=30;")), unit("TANKY", "FootprintX=2; FootprintZ=2; SightDistance=40;")]), 128, false);
    let seer = put(&mut sim, "SEER", 0, 100, 100);
    let _enemy = put(&mut sim, "TANKY", 1, 1600, 1600);
    let place = FxVec3::from_ints(1550, 0, 1550);
    sim.step(&[Command::Order { player: 0, units: vec![seer], order: Order::RemoteView(place), queue: false }]);
    run(&mut sim, 30);
    assert!(sim.in_sight(0, place), "viewing");
    let input = ta_sim::DirectInput { throttle: 0, turn: 0, course: None, heading: 0, pitch: 0, fire: false };
    sim.step(&[Command::DirectControl { player: 0, unit: seer, input: Some(input) }]);
    assert!(sim.unit(seer).unwrap().direct_control.is_some(), "driven");
    let energy = sim.players[0].economy.energy;
    for _ in 0..30 {
        sim.step(&[Command::DirectControl { player: 0, unit: seer, input: Some(input) }]);
    }
    let _ = sim.take_events();
    assert!(sim.viewing(sim.unit(seer).unwrap()).is_none() && !sim.in_sight(0, place), "nothing seen there while driven");
    assert!(sim.players[0].economy.energy >= energy, "nor paid for");
    sim.step(&[Command::DirectControl { player: 0, unit: seer, input: None }]);
    run(&mut sim, 30);
    assert!(sim.in_sight(0, place), "let go, viewing again");
}

/// A stunning shot leaves the enemy it hits unable to move for its time, beside its damage; its side
/// and the categories it spares are not stunned; one that goes off as it fires stuns round its unit.
#[test]
fn a_stun_stops_the_units_it_hits() {
    let mut sim = sim_of(
        data_of(&[
            unit("STUNNER", "Weapon1=EMP;"),
            unit("PULSER", "Weapon1=PULSE;"),
            unit("VICTIM", &format!("{TANK} Category=TANK;")),
            unit("BOSS", &format!("{TANK} Category=COMMANDER;")),
            weapon("EMP", "range=400; reloadtime=10; paralyzetime=3; paralyzeradius=80; paralyzeexempt=COMMANDER;"),
            weapon("PULSE", "range=400; reloadtime=10; paralyzetime=2; paralyzeradius=300; paralyzeonfire=1;"),
        ]),
        128,
        false,
    );
    let stunner = put(&mut sim, "STUNNER", 0, 400, 400);
    let victim = put(&mut sim, "VICTIM", 1, 400, 700);
    let boss = put(&mut sim, "BOSS", 1, 440, 720);
    let friend = put(&mut sim, "VICTIM", 0, 360, 700);
    let mut stunned = false;
    for _ in 0..30 * 4 {
        sim.step(&[]);
        if sim.unit(victim).unwrap().paralyzed_until > sim.tick {
            stunned = true;
            break;
        }
    }
    assert!(stunned, "the victim stunned");
    // Sent away while stunned, it goes nowhere until it wears off.
    sim.step(&[Command::Order { player: 1, units: vec![victim], order: Order::Move(FxVec3::from_ints(1800, 60, 700)), queue: false }]);
    assert!(health(&sim, victim) < Fx::from_int(1000), "and hurt as well");
    assert!(sim.unit(boss).unwrap().paralyzed_until <= sim.tick, "the category it spares is not stunned");
    assert!(sim.unit(friend).unwrap().paralyzed_until <= sim.tick, "nor its own side");
    let (held, until) = (sim.unit(victim).unwrap().pos, sim.unit(victim).unwrap().paralyzed_until);
    assert!(until >= sim.tick + 80, "three seconds of it: {} ticks", until - sim.tick);
    sim.remove_unit(stunner);
    run(&mut sim, 60);
    assert_eq!(sim.unit(victim).unwrap().pos, held, "it does not move while stunned");
    run(&mut sim, 60);
    assert_ne!(sim.unit(victim).unwrap().pos, held, "and moves on once it wears off");
    // A pulse as it fires: an enemy beside the shooter, not the one shot at.
    let pulser = put(&mut sim, "PULSER", 0, 1000, 1500);
    let beside = put(&mut sim, "VICTIM", 1, 1000, 1250);
    let shot = put(&mut sim, "VICTIM", 1, 1000, 1880);
    let mut pulsed = false;
    for _ in 0..30 * 3 {
        sim.step(&[]);
        pulsed |= sim.unit(beside).unwrap().paralyzed_until > sim.tick;
    }
    assert!(pulsed, "the enemy within its pulse stunned");
    assert!(sim.unit(shot).unwrap().paralyzed_until <= sim.tick, "the one shot at, beyond the pulse, not");
    assert!(sim.unit(pulser).is_some());
}

/// A regeneration aura mends its side's other damaged units round it once a second, by its rate or
/// its share (no more than its rate); the strongest of two alone; not enemies, not those out of its
/// reach, not itself.
#[test]
fn a_regeneration_aura_mends_its_sides_units_round_it() {
    let mut sim = sim_of(
        data_of(&[
            unit("HEALER", "RegenAuraRadius=200; RegenAuraRate=10; FootprintX=2; FootprintZ=2;"),
            unit("SHARER", "RegenAuraRadius=200; RegenAuraShare=0.05; RegenAuraRate=20; FootprintX=2; FootprintZ=2;"),
            unit("TANKY", "FootprintX=2; FootprintZ=2;"),
        ]),
        128,
        false,
    );
    let healer = put(&mut sim, "HEALER", 0, 400, 400);
    let near = put(&mut sim, "TANKY", 0, 500, 400);
    let far = put(&mut sim, "TANKY", 0, 800, 400);
    let enemy = put(&mut sim, "TANKY", 1, 400, 500);
    for id in [healer, near, far, enemy] {
        sim.set_health(id, Fx::from_int(500));
    }
    run(&mut sim, 30 * 2);
    assert_eq!(health(&sim, near), Fx::from_int(520), "ten a second, two seconds");
    assert_eq!((health(&sim, far), health(&sim, enemy), health(&sim, healer)), (Fx::from_int(500), Fx::from_int(500), Fx::from_int(500)), "not out of reach, an enemy or itself");
    // A share of the most health (5% of 1,000) held to its rate (20); the stronger aura alone.
    put(&mut sim, "SHARER", 0, 450, 450);
    run(&mut sim, 30);
    assert_eq!(health(&sim, near), Fx::from_int(540), "the stronger alone, its share held to its rate");
}

/// An enhancement's own production and upkeep (resource allocation, a jammer's energy) and an aura it
/// grants: income up, the upkeep paid while it is installed, its side mended.
#[test]
fn enhancements_grant_production_upkeep_and_auras() {
    let acu = "Builder=1; WorkerTime=100; FootprintX=2; FootprintZ=2; \
        [ENHANCEMENTS] { [RAS] { Slot=Back; BuildTime=1; MetalMake=10; EnergyMake=100; } [JAM] { Slot=LCH; BuildTime=1; RadarDistanceJam=300; EnergyUse=40; } \
        [AURA] { Slot=RCH; BuildTime=1; RegenAuraRadius=200; RegenAuraRate=10; } }";
    let mut sim = sim_of(data_of(&[unit("ACU", acu), unit("TANKY", "FootprintX=2; FootprintZ=2;")]), 64, false);
    let a = put(&mut sim, "ACU", 0, 400, 400);
    let tank = put(&mut sim, "TANKY", 0, 500, 400);
    sim.set_health(tank, Fx::from_int(500));
    for index in 0..3u16 {
        sim.step(&[Command::Order { player: 0, units: vec![a], order: Order::Enhance { index, remove: false }, queue: true }]);
    }
    run(&mut sim, 30);
    assert_eq!(sim.unit(a).unwrap().enhancements, [0, 1, 2]);
    let figures = sim.unit_flows(sim.unit(a).unwrap());
    let near = |v: Fx, n: i64| (v - Fx::from_int(n)).abs() < Fx::from_ratio(1, 100);
    assert!(near(figures.metal_make, 10) && near(figures.energy_make, 100), "resource allocation's: {figures:?}");
    assert!(near(figures.energy_use, 40), "the jammer's upkeep: {figures:?}");
    // (Room in the stores for what comes in.)
    sim.players[0].economy.metal = Fx::ZERO;
    sim.players[0].economy.energy = Fx::from_int(500);
    let (metal, energy) = (sim.players[0].economy.metal, sim.players[0].economy.energy);
    let healed = health(&sim, tank);
    run(&mut sim, 30);
    let e = &sim.players[0].economy;
    assert!((e.metal - metal - Fx::from_int(10)).abs() < Fx::from_ratio(1, 10), "ten metal a second: {:?}", e.metal - metal);
    assert!((e.energy - energy - Fx::from_int(60)).abs() < Fx::from_ratio(1, 10), "a hundred energy less forty: {:?}", e.energy - energy);
    assert_eq!(health(&sim, tank) - healed, Fx::from_int(10), "its aura mends");
}

/// The same orders give the same game: two copies with toggles, views, stuns and auras agree tick by
/// tick.
#[test]
fn the_abilities_play_the_same_on_every_computer() {
    let files = [
        unit("SNIPER", &format!("{TANK} Weapon1=GUN; [TOGGLE] {{ Weapon1=; Weapon2=LONG; }}")),
        unit("EYE", "RemoteViewRadius=200; RemoteViewEnergy=5; FootprintX=2; FootprintZ=2; RegenAuraRadius=300; RegenAuraRate=5;"),
        unit("STUNNER", &format!("{TANK} Weapon1=EMP;")),
        weapon("GUN", "range=200; minrange=40;"),
        weapon("LONG", "range=600; minrange=150;"),
        weapon("EMP", "range=300; reloadtime=3; paralyzetime=1;"),
    ];
    let play = || {
        let mut sim = sim_of(data_of(&files), 128, false);
        let s = put(&mut sim, "SNIPER", 0, 400, 400);
        let eye = put(&mut sim, "EYE", 0, 300, 300);
        let st = put(&mut sim, "STUNNER", 1, 700, 700);
        sim.step(&[
            Command::SetToggle { player: 0, units: vec![s], on: true },
            Command::Order { player: 0, units: vec![eye], order: Order::RemoteView(FxVec3::from_ints(1500, 0, 1500)), queue: false },
            Command::Order { player: 1, units: vec![st], order: Order::Fight(FxVec3::from_ints(400, 60, 400)), queue: false },
        ]);
        (0..30 * 20).map(|_| {
            sim.step(&[]);
            sim.checksum()
        }).collect::<Vec<u64>>()
    };
    assert_eq!(play(), play());
}
