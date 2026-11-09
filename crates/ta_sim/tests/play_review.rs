//! Gameplay faults a play-test review found, each held to the fixed behaviour, against the
//! real game data (skipped without a TA install).

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, SimEvent, Target, UnitId, UnitTypes};

/// A game on `map` between ARM (player 0, its Commander at the first start) and CORE (player
/// 1, no units), or None without an install.
fn game(map: &str) -> Option<(Sim, UnitId)> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let tnt = TntMap::parse(&vfs.read(&format!("maps/{map}.tnt")).ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map}.ota")).ok()?));
    let (_, x, z) = info.schemas[0].start_positions[0];
    let start = FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z));
    let players: Vec<PlayerSetup> = ["ARM", "CORE"]
        .iter()
        .enumerate()
        .map(|(i, side)| PlayerSetup { side: (*side).into(), color: i as u8, start: if i == 0 { start } else { FxVec3::ZERO }, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let mut sim = Sim::new(types.clone(), data, &tnt, &info, 0, &players, 1234);
    let commander = sim.spawn_unit(types.id("ARMCOM")?, 0, start, 0, true)?;
    Some((sim, commander))
}

fn id(sim: &Sim, name: &str) -> usize {
    sim.types.id(name).unwrap_or_else(|| panic!("no {name}"))
}

/// Plays `ticks` ticks; the shots `unit` fires.
fn shots(sim: &mut Sim, unit: UnitId, ticks: u64) -> u32 {
    let mut n = 0;
    for _ in 0..ticks {
        sim.step(&[]);
        n += sim.take_events().iter().filter(|e| matches!(e, SimEvent::WeaponFired { unit: u, .. } if *u == unit)).count() as u32;
    }
    n
}

/// Whether a unit stands on a cell its kind can't (inside a building).
fn inside_a_building(sim: &Sim, id: UnitId) -> bool {
    let u = sim.unit(id).unwrap();
    let t = sim.unit_type(u);
    !sim.world.passable(&sim.move_spec(t.id), ta_sim::world::World::pos_to_anchor(u.pos, t.footprint), None)
}

/// Pieces slide along x the way TA's scripts move them: the Arm Advanced Vehicle Plant's roof
/// is two halves meeting in the middle (cylinder1 spans x -25..1 at rest, cylinder2 1..27),
/// and opening slides them out to its edges. With the sign of x-axis moves the wrong way round
/// they passed through each other and the plant never opened.
#[test]
fn an_advanced_vehicle_plant_opens_its_roof() {
    let Some((mut sim, commander)) = game("Painted Desert") else { return };
    let start = sim.unit(commander).unwrap().pos;
    sim.remove_unit(commander);
    let plant = sim.spawn_unit(id(&sim, "ARMAVP"), 0, start, 0, true).unwrap();
    let apart = |sim: &Sim| {
        let u = sim.unit(plant).unwrap();
        let t = sim.unit_type(u);
        let x = |name: &str| ta_sim::piece_world_position(u, t, u.cob.as_ref(), t.model.piece_index(name).unwrap()).x.to_f32();
        (x("cylinder2") - x("cylinder1")).abs()
    };
    sim.step(&[]);
    let closed = apart(&sim);
    sim.step(&[Command::Produce { player: 0, factory: plant, type_id: id(&sim, "ARMACV"), count: 1 }]);
    for _ in 0..90 {
        sim.step(&[]);
    }
    let open = apart(&sim);
    println!("the roof halves' pivots: {closed:.1} apart closed, {open:.1} open");
    assert!(open > closed + 40.0, "the roof halves slide apart: {closed:.1} apart closed, {open:.1} open");
}

/// Guns whose aim scripts guard their unfolding from signals (the Hammer, the Zeus, the
/// Jethro) used to stop for good once they had aimed at a moving target: the engine killed
/// the aim it was replacing inside that step, leaving the script busy for ever. (The Jethro,
/// anti-air, takes a tank on only when told to: TA's bad targets.)
#[test]
fn guns_keep_firing_after_a_moving_target() {
    for name in ["ARMHAM", "ARMZEUS", "ARMJETH", "ARMSTUMP"] {
        let Some((mut sim, commander)) = game("Painted Desert") else { return };
        let start = sim.unit(commander).unwrap().pos;
        sim.remove_unit(commander);
        sim.los_enabled = false;
        sim.players[1].cheats.invulnerable = true;
        let gun = sim.spawn_unit(id(&sim, name), 0, start, 0, true).unwrap();
        let goliath = id(&sim, "CORGOL");
        let target = sim.spawn_unit(goliath, 1, start + FxVec3::from_ints(-120, 0, 90), 0, true).unwrap();
        let mut orders = vec![
            Command::SetStates { player: 1, units: vec![target], fire: Some(0), movement: Some(0) },
            Command::Order { player: 1, units: vec![target], order: Order::Patrol(start + FxVec3::from_ints(120, 0, 90)), queue: false },
        ];
        if name == "ARMJETH" {
            orders.push(Command::Order { player: 0, units: vec![gun], order: Order::Attack(Target::Unit(target)), queue: false });
        }
        sim.step(&orders);
        let moving = shots(&mut sim, gun, 30 * 15);
        sim.step(&[Command::Order { player: 1, units: vec![target], order: Order::Stop, queue: false }]);
        let still = shots(&mut sim, gun, 30 * 15);
        println!("{name}: {moving} shots at the moving target, then {still} once it stopped");
        assert!(moving > 0 && still > 0, "{name} fires at a moving target and goes on firing");
    }
}

/// The Slinger opens its launcher from its aim (a state change its aim's own signal would cut
/// short): asked to aim again while it unfolded, it used to stick half open and never fire.
/// It and the Jethro shoot at a plane flying over.
#[test]
fn anti_aircraft_kbots_shoot_at_a_passing_plane() {
    for name in ["CORAH", "ARMJETH"] {
        let Some((mut sim, commander)) = game("Painted Desert") else { return };
        let start = sim.unit(commander).unwrap().pos;
        sim.remove_unit(commander);
        sim.los_enabled = false;
        sim.players[1].cheats.invulnerable = true;
        let t = id(&sim, name);
        let site = sim.find_build_site(t, start, 10).unwrap();
        let gun = sim.spawn_unit(t, 0, site, 0, true).unwrap();
        let avenger = id(&sim, "CORVENG");
        let plane = sim.spawn_unit(avenger, 1, site + FxVec3::from_ints(-300, 0, 150), 16384, true).unwrap();
        sim.step(&[
            Command::SetStates { player: 1, units: vec![plane], fire: Some(0), movement: Some(0) },
            Command::Order { player: 1, units: vec![plane], order: Order::Patrol(site + FxVec3::from_ints(300, 0, 150)), queue: false },
        ]);
        let n = shots(&mut sim, gun, 30 * 30);
        println!("{name}: {n} shots at a patrolling Avenger in 30 s");
        assert!(n >= 6, "{name} shoots at the plane");
    }
}

/// A factory sends each unit along its rally route and goes on building: with a patrol it used
/// to stop after the first (a unit let it go only with one order left, a patrol keeps two), and
/// long routes held each next unit up until the last leg. A unit given an order the moment it
/// is done that keeps it in the yard (to guard the factory, as a computer player does with
/// builders) held the factory up for good.
#[test]
fn factories_go_on_building_whatever_the_rally_route() {
    let mut finished = Vec::new();
    for kind in ["none", "patrol", "route", "guard"] {
        let Some((mut sim, commander)) = game("Coast To Coast") else { return };
        let start = sim.unit(commander).unwrap().pos;
        let (lab, peewee) = (id(&sim, "ARMLAB"), id(&sim, "ARMPW"));
        let site = sim.find_build_site(lab, start + FxVec3::from_ints(0, 0, 160), 20).expect("room for a lab");
        let factory = sim.spawn_unit(lab, 0, site, 0, true).unwrap();
        sim.set_starting_resources(0, Fx::from_int(20_000), Fx::from_int(20_000));
        let order = |o: Order, queue: bool| Command::Order { player: 0, units: vec![factory], order: o, queue };
        let mut cmds = vec![Command::Produce { player: 0, factory, type_id: peewee, count: 3 }];
        match kind {
            "patrol" => cmds.push(order(Order::Patrol(site + FxVec3::from_ints(250, 0, 200)), false)),
            "route" => {
                cmds.push(order(Order::Move(site + FxVec3::from_ints(150, 0, 120)), false));
                cmds.push(order(Order::Move(site + FxVec3::from_ints(-600, 0, 900)), true));
            }
            _ => {}
        }
        sim.step(&cmds);
        let mut done = Vec::new();
        // ("guard": each told to guard the factory once it has been sent on its way.)
        let mut guards: Vec<(UnitId, u64)> = Vec::new();
        for tick in 0..30 * 70 {
            let orders: Vec<Command> = guards.iter().filter(|&&(_, at)| at == tick).map(|&(id, _)| Command::Order { player: 0, units: vec![id], order: Order::Guard(factory), queue: false }).collect();
            sim.step(&orders);
            for e in sim.take_events() {
                if let SimEvent::UnitCompleted { id, .. } = e
                    && sim.unit(id).is_some_and(|u| u.type_id == peewee)
                {
                    done.push(tick);
                    if kind == "guard" {
                        guards.push((id, tick + 3));
                    }
                }
            }
        }
        println!("rally {kind}: Peewees finished at ticks {done:?}");
        assert_eq!(done.len(), 3, "{kind}: all three Peewees are built");
        finished.push(done[2]);
    }
    // A route out costs at most the walk out of the yard: a second a unit.
    assert!(finished[1..].iter().all(|&f| f <= finished[0] + 30 * 3), "rally routes and orders in the yard don't slow production: {finished:?}");
}

/// Taking the unit in production off the queue used to leave it building, and the next unit
/// queued was dropped when it was done.
#[test]
fn cancelling_the_unit_in_production_stops_it() {
    let Some((mut sim, commander)) = game("Coast To Coast") else { return };
    let start = sim.unit(commander).unwrap().pos;
    let (lab, peewee, rocko) = (id(&sim, "ARMLAB"), id(&sim, "ARMPW"), id(&sim, "ARMROCK"));
    let site = sim.find_build_site(lab, start + FxVec3::from_ints(0, 0, 160), 20).expect("room for a lab");
    let factory = sim.spawn_unit(lab, 0, site, 0, true).unwrap();
    sim.set_starting_resources(0, Fx::from_int(20_000), Fx::from_int(20_000));
    sim.step(&[Command::Produce { player: 0, factory, type_id: peewee, count: 1 }, Command::Produce { player: 0, factory, type_id: rocko, count: 1 }]);
    for _ in 0..30 * 6 {
        sim.step(&[]);
    }
    let frame = sim.unit(factory).unwrap().build.map(|b| b.target).expect("the Peewee in production");
    assert_eq!(sim.unit(frame).map(|u| u.type_id), Some(peewee));
    let metal = sim.players[0].economy.metal;
    sim.step(&[Command::Produce { player: 0, factory, type_id: peewee, count: -1 }]);
    assert!(sim.unit(frame).is_none_or(|u| u.dead), "the Peewee's frame is taken apart");
    assert!(sim.players[0].economy.metal > metal, "the metal spent on it comes back");
    sim.take_events();
    let mut done = Vec::new();
    for _ in 0..30 * 60 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::UnitCompleted { id, type_id } = e
                && sim.unit(id).is_some()
            {
                done.push(sim.types.get(type_id).name.clone());
            }
        }
    }
    assert_eq!(done, vec!["ARMROCK".to_string()], "the Rocko queued after it is built, and nothing else");
}

/// Units sent to one place (a rally point, a computer player's move) used to run on the spot
/// at full speed for good: the crowd pushed each back as it stepped in.
#[test]
fn units_sent_to_one_place_arrive_in_a_crowd() {
    for name in ["ARMSTUMP", "ARMPW", "ARMBULL"] {
        let Some((mut sim, commander)) = game("Painted Desert") else { return };
        let start = sim.unit(commander).unwrap().pos;
        sim.remove_unit(commander);
        sim.local_avoidance = true;
        let t = id(&sim, name);
        let ids: Vec<UnitId> = (0..16).map(|k| sim.spawn_unit(t, 0, start + FxVec3::from_ints((k % 4) * 40, 0, (k / 4) * 40), 0, true).unwrap()).collect();
        let spot = start + FxVec3::from_ints(700, 0, 300);
        sim.step(&[Command::Order { player: 0, units: ids.clone(), order: Order::Move(spot), queue: false }]);
        for _ in 0..30 * 60 {
            sim.step(&[]);
        }
        let walking: Vec<f32> = ids.iter().filter_map(|&u| sim.unit(u)).filter(|u| !u.orders.is_empty() || u.speed > Fx::ZERO).map(|u| (u.pos - spot).xz().length_xz().to_f32()).collect();
        let far = ids.iter().filter_map(|&u| sim.unit(u)).map(|u| (u.pos - spot).xz().length_xz().to_f32()).fold(0.0, f32::max);
        println!("{name}: still walking after a minute {walking:?}; the farthest stopped {far:.0} px from the spot");
        assert!(walking.is_empty(), "{name}: every unit arrives");
        assert!(far < 160.0, "{name}: they gather round the spot");
    }
}

/// A building laid where units stand (the builder's own spot, an idle Peewee's) used to go up
/// round them, leaving them inside it for good.
#[test]
fn units_step_off_a_building_site() {
    let Some((mut sim, commander)) = game("Painted Desert") else { return };
    sim.set_starting_resources(0, Fx::from_int(20_000), Fx::from_int(20_000));
    let start = sim.unit(commander).unwrap().pos;
    let (solar, pw) = (id(&sim, "ARMSOLAR"), id(&sim, "ARMPW"));
    let peewee = sim.spawn_unit(pw, 0, start + FxVec3::from_ints(120, 0, 0), 0, true).unwrap();
    let own = sim.snap_to_grid(solar, start);
    let other = sim.snap_to_grid(solar, start + FxVec3::from_ints(120, 0, 0));
    sim.step(&[
        Command::Order { player: 0, units: vec![commander], order: Order::Build { type_id: solar, pos: own }, queue: false },
        Command::Order { player: 0, units: vec![commander], order: Order::Build { type_id: solar, pos: other }, queue: true },
    ]);
    for _ in 0..30 * 90 {
        sim.step(&[]);
    }
    let built = sim.units().filter(|u| u.type_id == solar && u.is_complete()).count();
    assert_eq!(built, 2, "both solars go up");
    assert!(!inside_a_building(&sim, commander), "the Commander stepped off its own site");
    assert!(!inside_a_building(&sim, peewee), "the Peewee stepped off the second site");
}

/// The Commander came up to an attack order's target by its longest weapon, the D-gun (which
/// waits for manual fire): 216 px off, out of its laser's 200, it never fired at a mine.
#[test]
fn a_commander_closes_to_laser_range_to_attack() {
    for victim in ["CORMINE1", "CORAK"] {
        let Some((mut sim, commander)) = game("Painted Desert") else { return };
        sim.los_enabled = false;
        let start = sim.unit(commander).unwrap().pos;
        let vt = id(&sim, victim);
        let site = sim.find_build_site(vt, start + FxVec3::from_ints(450, 0, 0), 8).unwrap();
        let v = sim.spawn_unit(vt, 1, site, 0, true).unwrap();
        sim.step(&[
            Command::SetStates { player: 1, units: vec![v], fire: Some(0), movement: Some(0) },
            Command::SetStates { player: 0, units: vec![commander], fire: Some(0), movement: None },
        ]);
        sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Attack(Target::Unit(v)), queue: false }]);
        let n = shots(&mut sim, commander, 30 * 20);
        assert!(n > 0 && sim.unit(v).is_none_or(|u| u.dead), "the Commander fires on the {victim} and destroys it");
    }
}

/// A Guardian (range 1250; its shells carry about 1,094 at TA's gravity) locked on to the
/// first enemy inside its range and never fired, past it or at anything else.
#[test]
fn long_guns_engage_what_their_shells_reach() {
    for name in ["ARMGUARD", "CORPUN"] {
        let Some((mut sim, commander)) = game("Painted Desert") else { return };
        let start = sim.unit(commander).unwrap().pos;
        sim.remove_unit(commander);
        sim.los_enabled = false;
        let g = id(&sim, name);
        let site = sim.find_build_site(g, start, 20).unwrap();
        let gun = sim.spawn_unit(g, 0, site, 0, true).unwrap();
        let llt = id(&sim, "CORLLT");
        let far = [1192, -1192].iter().find_map(|&dx| sim.find_build_site(llt, site + FxVec3::from_ints(dx, 0, 0), 2)).expect("room for a far target");
        let far = sim.spawn_unit(llt, 1, far, 0, true).unwrap();
        let reach = sim.weapon_types.get(sim.unit_type(sim.unit(gun).unwrap()).weapons[0].unwrap()).range;
        let d = (sim.unit(far).unwrap().pos - site).xz().length_xz();
        assert!(d < reach, "the far target is inside the gun's range ({d} < {reach})");
        assert_eq!(shots(&mut sim, gun, 30 * 10), 0, "{name}: out of its shells' reach, nothing to fire at");
        let near = sim.find_build_site(llt, site + FxVec3::from_ints(0, 0, 600), 6).or_else(|| sim.find_build_site(llt, site + FxVec3::from_ints(0, 0, -600), 6)).unwrap();
        let near = sim.spawn_unit(llt, 1, near, 0, true).unwrap();
        let n = shots(&mut sim, gun, 30 * 20);
        println!("{name}: {n} shots once a target in reach turned up");
        assert!(n > 0 && sim.unit(near).is_none_or(|u| u.dead), "{name} shells the target its shells reach");
    }
}

/// Fighters from an air plant with a rally point all set down on the one spot.
#[test]
fn aircraft_spread_out_where_they_land() {
    let Some((mut sim, commander)) = game("Painted Desert") else { return };
    let start = sim.unit(commander).unwrap().pos;
    let plant = id(&sim, "ARMAP");
    let site = sim.find_build_site(plant, start + FxVec3::from_ints(0, 0, 200), 20).unwrap();
    let f = sim.spawn_unit(plant, 0, site, 0, true).unwrap();
    sim.set_starting_resources(0, Fx::from_int(50_000), Fx::from_int(50_000));
    sim.players[0].cheats.instant_build = true;
    let fighter = id(&sim, "ARMFIG");
    sim.step(&[
        Command::Produce { player: 0, factory: f, type_id: fighter, count: 6 },
        Command::Order { player: 0, units: vec![f], order: Order::Move(site + FxVec3::from_ints(400, 0, 300)), queue: false },
    ]);
    for _ in 0..30 * 90 {
        sim.step(&[]);
    }
    let landed: Vec<FxVec3> = sim.units().filter(|u| u.type_id == fighter && u.landed).map(|u| u.pos).collect();
    assert_eq!(landed.len(), 6, "all six have landed");
    for (i, a) in landed.iter().enumerate() {
        for b in &landed[i + 1..] {
            assert!(!a.within_xz(*b, Fx::from_int(16)), "two fighters landed on top of each other");
        }
    }
}

/// A submarine under fire: only torpedoes and depth charges reach it (a ship's guns and
/// missiles used to sink it), and radar doesn't show it.
#[test]
fn only_torpedoes_and_depth_charges_reach_a_submarine() {
    let Some((mut sim, commander)) = game("Coast To Coast") else { return };
    sim.remove_unit(commander);
    sim.los_enabled = false;
    let (w, h) = sim.world.size_px();
    let sea = sim.world.sea_level;
    let deep = |x: i64, z: i64| sim.world.height_at(Fx::from_int(x), Fx::from_int(z)) < sea - Fx::from_int(40);
    let spot = (200..h.to_int() - 200).step_by(32).flat_map(|z| (200..w.to_int() - 600).step_by(32).map(move |x| (x, z))).find(|&(x, z)| (0..=400).step_by(50).all(|dx| deep(x + dx, z) && deep(x + dx, z + 60) && deep(x + dx, z - 60))).expect("open sea");
    let sub = sim.spawn_unit(id(&sim, "ARMSUB"), 0, FxVec3::from_ints(spot.0, 0, spot.1), 16384, true).unwrap();
    let shooters: Vec<UnitId> = ["CORROY", "CORPT", "CORSH"].iter().enumerate().map(|(i, name)| sim.spawn_unit(id(&sim, name), 1, FxVec3::from_ints(spot.0 + 220 + 40 * i as i64, 0, spot.1 - 40 + 40 * i as i64), -16384, true).unwrap()).collect();
    sim.step(&[Command::SetStates { player: 0, units: vec![sub], fire: Some(0), movement: Some(0) }]);
    assert!(sim.submerged(sim.unit(sub).unwrap()), "a submarine is under the water");
    let mut fired = std::collections::BTreeSet::new();
    for _ in 0..30 * 20 {
        sim.step(&[]);
        for e in sim.take_events() {
            if let SimEvent::WeaponFired { unit, weapon, .. } = e
                && shooters.contains(&unit)
            {
                fired.insert(sim.weapon_types.get(weapon).name.clone());
            }
        }
    }
    println!("fired at the submarine: {fired:?}");
    assert!(!fired.is_empty(), "the destroyer's depth charges go after it");
    assert!(fired.iter().all(|name| sim.weapon_types.by_name(name).is_some_and(|w| w.water)), "only weapons that go under the water fire at it");
}

/// An unfinished tower under fire took its attacker as a target its weapons never look at
/// (they wake when it is finished), and held it for good.
#[test]
fn an_unfinished_tower_takes_no_target() {
    let Some((mut sim, commander)) = game("Painted Desert") else { return };
    sim.los_enabled = false;
    sim.set_starting_resources(0, Fx::from_int(5000), Fx::from_int(5000));
    let start = sim.unit(commander).unwrap().pos;
    let llt = id(&sim, "ARMLLT");
    let site = sim.find_build_site(llt, start + FxVec3::from_ints(0, 0, 96), 10).unwrap();
    // The Commander gets it half built, and leaves it.
    sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Build { type_id: llt, pos: site }, queue: false }]);
    let mut tower = None;
    for _ in 0..30 * 30 {
        sim.step(&[]);
        tower = sim.units().find(|u| u.type_id == llt).map(|u| u.id);
        if tower.and_then(|t| sim.unit(t)).is_some_and(|t| t.build_progress > Fx::from_ratio(1, 2)) {
            break;
        }
    }
    let tower = tower.expect("the tower's frame");
    sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Stop, queue: false }]);
    sim.remove_unit(commander);
    let raider = sim.spawn_unit(id(&sim, "CORAK"), 1, site + FxVec3::from_ints(120, 0, 0), 0, true).unwrap();
    sim.step(&[Command::Order { player: 1, units: vec![raider], order: Order::Attack(Target::Unit(tower)), queue: false }]);
    let mut hit = false;
    for _ in 0..30 * 5 {
        sim.step(&[]);
        let Some(t) = sim.unit(tower) else { break };
        hit |= t.last_attacker == Some(raider);
        assert!(!t.is_complete(), "the tower stays unfinished");
        assert!(t.weapons.iter().all(|w| w.target.is_none()), "an unfinished tower holds no target");
    }
    assert!(hit, "the tower was shot at");
}

/// Fighter groups shot themselves down: their missiles, launched among their wingmen, struck
/// them, and planes blew up in the air with their whole death blast, hitting the planes
/// round them.
#[test]
fn a_fighter_group_does_not_shoot_itself_down() {
    let Some((mut sim, commander)) = game("Painted Desert") else { return };
    let start = sim.unit(commander).unwrap().pos;
    sim.remove_unit(commander);
    sim.los_enabled = false;
    let (vamp, fig) = (id(&sim, "CORVAMP"), id(&sim, "ARMFIG"));
    // CORE's ten Vampires (player 1) close together, ARM's six fighters coming.
    let ours: Vec<UnitId> = (0..10).map(|k| sim.spawn_unit(vamp, 1, start + FxVec3::from_ints(-400 + (k % 5) * 30, 0, (k / 5) * 30), 16384, true).unwrap()).collect();
    let theirs: Vec<UnitId> = (0..6).map(|k| sim.spawn_unit(fig, 0, start + FxVec3::from_ints(400 + (k % 3) * 30, 0, (k / 3) * 30), -16384, true).unwrap()).collect();
    sim.step(&[
        Command::Order { player: 1, units: ours.clone(), order: Order::Fight(start + FxVec3::from_ints(500, 0, 0)), queue: false },
        Command::Order { player: 0, units: theirs.clone(), order: Order::Fight(start + FxVec3::from_ints(-500, 0, 0)), queue: false },
    ]);
    let mut friendly = 0;
    for _ in 0..30 * 40 {
        let health: Vec<(UnitId, Fx)> = ours.iter().filter_map(|&u| sim.unit(u).map(|v| (u, v.health))).collect();
        sim.step(&[]);
        sim.take_events();
        for (u, before) in health {
            if let Some(v) = sim.unit(u)
                && v.health < before
                && v.last_attacker.is_some_and(|a| ours.contains(&a))
            {
                friendly += 1;
            }
        }
    }
    assert_eq!(friendly, 0, "no Vampire is hit by another");
    // A plane blowing up in the middle of a formation in the air hurts none of the others.
    let Some((mut sim, commander)) = game("Painted Desert") else { return };
    let start = sim.unit(commander).unwrap().pos;
    sim.remove_unit(commander);
    let formation: Vec<UnitId> = (0..9).map(|k| sim.spawn_unit(vamp, 1, start + FxVec3::from_ints((k % 3) * 20, 0, (k / 3) * 20), 0, true).unwrap()).collect();
    sim.step(&[Command::Order { player: 1, units: formation.clone(), order: Order::Move(start + FxVec3::from_ints(2000, 0, 0)), queue: false }]);
    for _ in 0..90 {
        sim.step(&[]);
    }
    assert!(formation.iter().all(|&u| sim.unit(u).is_some_and(|v| !v.landed)), "the formation is in the air");
    sim.step(&[Command::SelfDestruct { player: 1, units: vec![formation[4]] }]);
    for _ in 0..30 * 6 {
        sim.step(&[]);
    }
    assert!(sim.unit(formation[4]).is_none(), "the middle plane is gone");
    let hurt = formation.iter().filter(|&&u| u != formation[4]).filter(|&&u| sim.unit(u).is_none_or(|v| v.health < sim.unit_type(v).max_health)).count();
    assert_eq!(hurt, 0, "its blast spares the planes flying beside it");
}

/// A Commander burnt by a Pyro kept shooting the solar collector it had picked first (a
/// campaign mission was lost that way, the Pyros never touched): a unit under attack turns on
/// its attacker from a target that can't fight back.
#[test]
fn a_unit_under_attack_turns_on_its_attacker() {
    let Some((mut sim, commander)) = game("Painted Desert") else { return };
    sim.los_enabled = false;
    let start = sim.unit(commander).unwrap().pos;
    let (solar, pyro) = (id(&sim, "CORSOLAR"), id(&sim, "CORPYRO"));
    let site = sim.find_build_site(solar, start + FxVec3::from_ints(150, 0, 0), 4).expect("room for a solar");
    let collector = sim.spawn_unit(solar, 1, site, 0, true).unwrap();
    for _ in 0..30 {
        sim.step(&[]);
    }
    assert_eq!(sim.unit(commander).unwrap().weapons[0].target, Some(Target::Unit(collector)), "the Commander shoots the solar first");
    let p = sim.spawn_unit(pyro, 1, start + FxVec3::from_ints(-160, 0, 0), 16384, true).unwrap();
    sim.step(&[Command::Order { player: 1, units: vec![p], order: Order::Attack(Target::Unit(commander)), queue: false }]);
    let (mut burnt, mut turned) = (None, None);
    for tick in 0..30 * 10 {
        sim.step(&[]);
        let c = sim.unit(commander).unwrap();
        if burnt.is_none() && c.last_attacker == Some(p) {
            burnt = Some(tick);
        }
        if c.weapons[0].target == Some(Target::Unit(p)) {
            turned = Some(tick);
            break;
        }
    }
    println!("burnt at tick {burnt:?}, turned on the Pyro at {turned:?}");
    let (Some(burnt), Some(turned)) = (burnt, turned) else { panic!("the Pyro burns the Commander and the Commander turns on it: {burnt:?} {turned:?}") };
    assert!(turned <= burnt + 2, "at once");
}

/// A unit splashed by a friend's shell turned its guns on the friend (in a big battle most of
/// the shots were soon at friends): only an enemy's hit is answered.
#[test]
fn a_friends_splash_is_not_answered() {
    let Some((mut sim, commander)) = game("Painted Desert") else { return };
    sim.los_enabled = false;
    let start = sim.unit(commander).unwrap().pos;
    let ham = sim.spawn_unit(id(&sim, "ARMHAM"), 0, start + FxVec3::from_ints(-160, 0, 0), 16384, true).unwrap();
    let full = sim.unit(ham).unwrap().health;
    let before = sim.unit(commander).unwrap().health;
    sim.step(&[Command::Order { player: 0, units: vec![ham], order: Order::Attack(Target::Ground(start + FxVec3::from_ints(12, 0, 12))), queue: false }]);
    for _ in 0..30 * 10 {
        sim.step(&[]);
        let c = sim.unit(commander).unwrap();
        assert!(c.weapons.iter().all(|w| w.target != Some(Target::Unit(ham))), "the Commander never aims at the Hammer");
    }
    assert!(sim.unit(commander).unwrap().health < before, "the Hammer's shells splashed the Commander");
    assert_eq!(sim.unit(ham).unwrap().health, full, "nobody shot at the Hammer");
}

/// A build or reclaim order given again to the same place failed at once: the unit's way there
/// from the first one said it had come as near as it could.
#[test]
fn an_order_given_again_starts_afresh() {
    let Some((mut sim, commander)) = game("Painted Desert") else { return };
    sim.set_starting_resources(0, Fx::from_int(20_000), Fx::from_int(20_000));
    let start = sim.unit(commander).unwrap().pos;
    let solar = id(&sim, "ARMSOLAR");
    let site = sim.find_build_site(solar, start + FxVec3::from_ints(600, 0, 0), 10).expect("room for a solar");
    // (A rock or tree a few hundred pixels off, where there is one.)
    let feature = sim
        .features
        .iter()
        .enumerate()
        .filter(|(_, f)| f.alive && sim.data.features[f.def].reclaimable && f.pos.within_xz(start, Fx::from_int(900)) && !f.pos.within_xz(start, Fx::from_int(300)))
        .min_by_key(|(_, f)| (f.pos - start).xz().length_sq_raw())
        .map(|(i, _)| i);
    let mut orders = vec![("build", Order::Build { type_id: solar, pos: site })];
    if let Some(f) = feature {
        orders.push(("reclaim", Order::Reclaim(f)));
    }
    for (name, order) in orders {
        let give = Command::Order { player: 0, units: vec![commander], order: order.clone(), queue: false };
        sim.step(std::slice::from_ref(&give));
        for _ in 0..60 {
            sim.step(&[]);
        }
        sim.take_events();
        sim.step(&[give]);
        for _ in 0..10 {
            sim.step(&[]);
        }
        let failed = sim.take_events().into_iter().any(|e| matches!(e, SimEvent::OrderFailed { .. }));
        let c = sim.unit(commander).unwrap();
        println!("{name} given again: failed {failed}, orders {:?}, on its way {}", c.orders.front(), c.path_goal.is_some());
        assert!(!failed && c.orders.front() == Some(&order), "{name}: given again, the order stands");
        assert!(c.path_goal.is_some(), "{name}: and the Commander is on its way");
        sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Stop, queue: false }]);
    }
}

/// Two units meeting head on pushed each other back for good (on a campaign map, hovercraft
/// and amphibious kbots crossing a gap one unit wide both ways never got through): held up a
/// moment by its own side's units coming the other way, a unit slips past them.
#[test]
fn units_meeting_head_on_get_past_each_other() {
    for name in ["ARMSTUMP", "ARMPW", "ARMBULL"] {
        let Some((mut sim, commander)) = game("Painted Desert") else { return };
        let start = sim.unit(commander).unwrap().pos;
        sim.remove_unit(commander);
        let t = id(&sim, name);
        // Open ground: room for a row of fusion plants all the way across.
        let fusion = id(&sim, "ARMFUS");
        let open = (0..40i64)
            .map(|k| start + FxVec3::from_ints((k % 8) * 200 - 800, 0, (k / 8) * 200 - 400))
            .find(|&p| (0..=8).all(|k| sim.find_build_site(fusion, p + FxVec3::from_ints(k * 80, 0, 0), 0).is_some()))
            .expect("open ground");
        let (a_at, b_at) = (sim.snap_to_grid(t, open), sim.snap_to_grid(t, open + FxVec3::from_ints(640, 0, 0)));
        let a = sim.spawn_unit(t, 0, a_at, 16384, true).unwrap();
        let b = sim.spawn_unit(t, 0, b_at, -16384, true).unwrap();
        sim.step(&[
            Command::Order { player: 0, units: vec![a], order: Order::Move(b_at), queue: false },
            Command::Order { player: 0, units: vec![b], order: Order::Move(a_at), queue: false },
        ]);
        for _ in 0..30 * 30 {
            sim.step(&[]);
        }
        let (ua, ub) = (sim.unit(a).unwrap(), sim.unit(b).unwrap());
        let (da, db) = ((ua.pos - b_at).xz().length_xz(), (ub.pos - a_at).xz().length_xz());
        println!("{name}: {da} and {db} px from where they were sent");
        assert!(da < Fx::from_int(40) && db < Fx::from_int(40), "{name}: both get through");
    }
}
