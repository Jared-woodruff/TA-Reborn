//! Integration tests against the real game data. Skipped when no TA install is found.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, UnitTypes};

struct Fixture {
    sim: Sim,
    commander: ta_sim::UnitId,
}

fn setup(map_name: &str) -> Option<Fixture> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").ok()?;
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let map = TntMap::parse(&vfs.read(&format!("maps/{map_name}.tnt")).ok()?).ok()?;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{map_name}.ota")).ok()?));
    let (_, x, z) = info.schemas[0].start_positions[0];
    let start = FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z));
    let players = vec![PlayerSetup {
        side: "ARM".into(),
        color: 0,
        start,
        metal: Fx::from_int(1000),
        energy: Fx::from_int(1000),
        ai: false,
    }];
    let mut sim = Sim::new(types.clone(), data, &map, &info, 0, &players, 1234);
    let commander = sim.spawn_unit(types.id("ARMCOM")?, 0, start, 0, true)?;
    Some(Fixture { sim, commander })
}

#[test]
fn water_buildings_go_on_water_and_land_buildings_on_land() {
    let Some(Fixture { sim, commander }) = setup("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let id = |n: &str| sim.types.id(n).unwrap();
    // Somewhere deep, and the commander's dry start.
    let (w, h) = sim.world.size_px();
    let mut deep = None;
    'search: for z in (200..h.to_int() - 200).step_by(64) {
        for x in (200..w.to_int() - 200).step_by(64) {
            let p = FxVec3::from_ints(x, 0, z);
            if sim.can_place(id("ARMSY"), p) {
                deep = Some(p);
                break 'search;
            }
        }
    }
    let deep = deep.expect("a shipyard fits somewhere at sea");
    let land = sim.unit(commander).unwrap().pos;
    for water in ["ARMSY", "ARMTIDE", "ARMPLAT"] {
        assert!(sim.can_place(id(water), deep), "{water} goes on deep water");
        assert!(!sim.can_place(id(water), land), "{water} needs water");
    }
    assert!(!sim.can_place(id("ARMSOLAR"), deep), "solar collectors stay on land");
}

/// A route's straight shortcuts must hold for a unit steering along them from wherever it
/// stands: on Slated Fate the Commander used to stand at the crater's rim for good, sliding
/// along it, with its extractor site inside.
#[test]
fn a_commander_climbs_into_the_crater_to_build() {
    let Some(Fixture { mut sim, commander }) = setup("Slated Fate") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    // The first start position is on the plain east of the crater.
    assert!(sim.unit(commander).unwrap().pos.within_xz(FxVec3::from_ints(1120, 0, 384), Fx::from_int(32)));
    let mex = sim.types.id("ARMMEX").unwrap();
    let site = sim.snap_to_grid(mex, FxVec3::from_ints(680, 0, 616));
    sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Build { type_id: mex, pos: site }, queue: false }]);
    let mut built = None;
    for tick in 0..30 * 60 {
        sim.step(&[]);
        if sim.units().any(|u| u.type_id == mex && u.is_complete() && u.pos.within_xz(site, Fx::from_int(24))) {
            built = Some(tick);
            break;
        }
    }
    println!("extractor in the crater finished after {built:?} ticks");
    assert!(built.is_some_and(|t| t < 30 * 40), "the Commander reaches the crater and builds within 40 s");
}

#[test]
fn commander_walks_and_builds() {
    let Some(Fixture { mut sim, commander }) = setup("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let start = sim.unit(commander).unwrap().pos;
    assert!(sim.unit(commander).unwrap().cob.is_some(), "commander should have a script");

    // Walk 200 px east.
    let dest = start + FxVec3::new(Fx::from_int(200), Fx::ZERO, Fx::ZERO);
    let cmd = Command::Order { player: 0, units: vec![commander], order: Order::Move(dest), queue: false };
    sim.step(&[cmd]);
    let mut arrived_at = None;
    for tick in 0..30 * 30 {
        sim.step(&[]);
        let u = sim.unit(commander).unwrap();
        if u.orders.is_empty() {
            arrived_at = Some(tick);
            break;
        }
    }
    let u = sim.unit(commander).unwrap();
    let err = u.pos.distance_xz(dest);
    println!("arrived after {arrived_at:?} ticks, {err:?} px from target, pos {:?}", u.pos);
    assert!(arrived_at.is_some(), "commander never arrived");
    assert!(err < Fx::from_int(30));

    // Build a solar collector next to it.
    let solar = sim.types.id("ARMSOLAR").unwrap();
    let site = sim.find_build_site(solar, u.pos + FxVec3::new(Fx::from_int(96), Fx::ZERO, Fx::ZERO), 30).expect("no site");
    println!("building at {site:?}, {} px away", site.distance_xz(u.pos));
    let cmd = Command::Order { player: 0, units: vec![commander], order: Order::Build { type_id: solar, pos: site }, queue: false };
    sim.step(&[cmd]);
    let mut built = None;
    for tick in 0..30 * 120 {
        sim.step(&[]);
        if sim.units().any(|u| u.type_id == solar && u.is_complete()) {
            built = Some(tick);
            break;
        }
    }
    let eco = &sim.players[0].economy;
    println!("solar built after {built:?} ticks; metal {} energy {} (+{}/s -{}/s)", eco.metal, eco.energy, eco.energy_income, eco.energy_expense);
    assert!(built.is_some(), "solar never finished");
    // ARMSOLAR: 2495 build time / 300 worker time ≈ 8.3 s ≈ 250 ticks (+ approach and arm animation).
    assert!(built.unwrap() < 30 * 20);
    // The finished collector opens and produces energy (commander 25 + solar 20).
    for _ in 0..60 {
        sim.step(&[]);
    }
    let eco = &sim.players[0].economy;
    assert!(eco.energy_income >= Fx::from_int(40), "energy income {}", eco.energy_income);
}

#[test]
fn simulation_is_deterministic() {
    let run = || {
        let Fixture { mut sim, commander } = setup("Coast To Coast")?;
        let dest = sim.unit(commander).unwrap().pos + FxVec3::new(Fx::from_int(-150), Fx::ZERO, Fx::from_int(120));
        sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Move(dest), queue: false }]);
        for _ in 0..300 {
            sim.step(&[]);
        }
        let u = sim.unit(commander).unwrap();
        Some((u.pos, u.heading, sim.players[0].economy.energy))
    };
    let (a, b) = (run(), run());
    assert_eq!(a, b);
}

#[test]
fn commander_reclaims_a_rock() {
    let Some(Fixture { mut sim, commander }) = setup("Sherwood") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let pos = sim.unit(commander).unwrap().pos;
    // Nearest reclaimable feature with metal.
    let (index, _) = sim
        .features
        .iter()
        .enumerate()
        .filter(|(_, f)| f.alive && sim.data.features[f.def].reclaimable && sim.data.features[f.def].metal > 0.0)
        .min_by_key(|(_, f)| (f.pos - pos).xz().length_sq_raw())
        .expect("a reclaimable rock");
    let def = sim.data.features[sim.features[index].def].clone();
    // Spend some metal first so the gain is visible below the storage cap.
    sim.players[0].economy.metal = Fx::from_int(100);
    let before = sim.players[0].economy.metal;
    sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Reclaim(index), queue: false }]);
    let mut done_at = None;
    for tick in 0..30 * 90 {
        sim.step(&[]);
        if !sim.features[index].alive {
            done_at = Some(tick);
            break;
        }
    }
    let after = sim.players[0].economy.metal;
    println!("reclaimed {} (metal {}) in {:?} ticks: metal {} -> {}", def.name, def.metal, done_at, before, after);
    assert!(done_at.is_some(), "feature never reclaimed");
    assert!(after - before >= Fx::from_f64(def.metal * 0.9), "metal should increase by the feature's value");
}

#[test]
fn commander_reclaims_the_tutorial_rocks_without_a_separate_move() {
    let Some(Fixture { sim: original, commander }) = setup("Painted Desert") else { return };
    let start = original.unit(commander).unwrap().pos;
    let mut rocks: Vec<_> = original.features.iter().enumerate()
        .filter(|(_, f)| f.alive && original.data.features[f.def].reclaimable && original.data.features[f.def].metal > 0.0)
        .map(|(i, f)| (f.pos.distance_xz(start), i)).collect();
    rocks.sort_unstable();
    assert!(!rocks.is_empty());
    for &(_, index) in rocks.iter().take(12) {
        let mut sim = original.clone();
        let rock = sim.features[index].clone();
        sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Reclaim(index), queue: false }]);
        for _ in 0..30 * 90 {
            sim.step(&[]);
            if !sim.features[index].alive { break; }
        }
        let u = sim.unit(commander).unwrap();
        assert!(!sim.features[index].alive, "{}: at {:?}, orders {:?}", sim.data.features[rock.def].name, u.pos, u.orders);
        let def = &sim.data.features[rock.def];
        for z in rock.origin.1..rock.origin.1 + def.footprint_z as i32 {
            for x in rock.origin.0..rock.origin.0 + def.footprint_x as i32 {
                assert_ne!(sim.world.occupant(x, z), ta_sim::world::Occupant::Feature(index as u32));
            }
        }
    }
}

/// A patrol shorter than the distance a point counts as reached from (three unit radii and
/// a bit) still goes back and forth: the unit used to stop halfway, both ends "reached".
#[test]
fn a_short_patrol_keeps_walking() {
    let Some(Fixture { mut sim, commander }) = setup("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let start = sim.unit(commander).unwrap().pos + FxVec3::from_ints(0, 0, 160);
    let pw = sim.spawn_unit(sim.types.id("ARMPW").unwrap(), 0, start, 0, true).unwrap();
    let there = start + FxVec3::from_ints(120, 0, 0);
    sim.step(&[Command::Order { player: 0, units: vec![pw], order: Order::Patrol(there), queue: false }]);
    let mut walked = Fx::ZERO;
    let mut last = sim.unit(pw).unwrap().pos;
    for _ in 0..30 * 12 {
        sim.step(&[]);
        let p = sim.unit(pw).unwrap().pos;
        walked += (p - last).xz().length();
        last = p;
    }
    assert!(walked > Fx::from_int(300), "walked {walked:?} in 12 s on a 120-pixel patrol");
}

#[test]
fn patrolling_commander_assists_and_loops() {
    let Some(Fixture { mut sim, commander }) = setup("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let start = sim.unit(commander).unwrap().pos;
    // A friendly nanoframe just off the patrol route.
    let solar = sim.types.id("ARMSOLAR").unwrap();
    let site = sim.find_build_site(solar, start + FxVec3::from_ints(150, 0, 90), 12).expect("site");
    let frame = sim.spawn_unit(solar, 0, site, 0, false).unwrap();
    let dest = start + FxVec3::from_ints(300, 0, 0);
    sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::Patrol(dest), queue: false }]);
    let mut finished = None;
    let mut max_x = Fx::ZERO;
    let mut returned = false;
    for tick in 0..30 * 150 {
        sim.step(&[]);
        let u = sim.unit(commander).unwrap();
        let dx = u.pos.x - start.x;
        max_x = max_x.max(dx);
        if finished.is_none() && sim.unit(frame).is_some_and(|f| f.is_complete()) {
            finished = Some(tick);
        }
        if finished.is_some() && max_x > Fx::from_int(200) && dx < Fx::from_int(80) {
            returned = true;
            break;
        }
    }
    let u = sim.unit(commander).unwrap();
    let orders: Vec<String> = u.orders.iter().map(|o| format!("{o:?}")).collect();
    println!("nanoframe finished at {finished:?}; furthest {:.0} px; returned {returned}; orders {orders:?}", max_x.to_f32());
    assert!(finished.is_some(), "a patrolling builder should finish nearby nanoframes");
    assert!(max_x > Fx::from_int(200), "the patrol should reach its far point");
    assert!(returned, "the patrol should loop back to its start");
    assert!(u.orders.iter().all(|o| matches!(o, Order::Patrol(_))) && u.orders.len() == 2, "patrol points keep cycling");
}

#[test]
fn guard_follows_its_charge() {
    let Some(Fixture { mut sim, commander }) = setup("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let start = sim.unit(commander).unwrap().pos;
    let flash = sim.types.id("ARMFLASH").unwrap();
    let p = sim.find_build_site(flash, start + FxVec3::from_ints(-80, 0, 0), 12).expect("site");
    let guard = sim.spawn_unit(flash, 0, p, 0, true).unwrap();
    let dest = start + FxVec3::from_ints(260, 0, 60);
    sim.step(&[
        Command::Order { player: 0, units: vec![guard], order: Order::Guard(commander), queue: false },
        Command::Order { player: 0, units: vec![commander], order: Order::Move(dest), queue: false },
    ]);
    for _ in 0..30 * 25 {
        sim.step(&[]);
    }
    let (c, g) = (sim.unit(commander).unwrap(), sim.unit(guard).unwrap());
    let gap = (c.pos - g.pos).length_xz().to_f32();
    println!("commander moved {:.0} px; guard gap {gap:.0} px; guard orders {:?}", (c.pos - start).length_xz().to_f32(), g.orders);
    assert!((c.pos - start).length_xz() > Fx::from_int(200));
    assert!(gap < 110.0, "the guard should stay close to the commander");
    assert!(matches!(g.orders.front(), Some(Order::Guard(_))), "guarding continues");
}

#[test]
fn factories_wait_at_the_unit_limit() {
    let Some(Fixture { mut sim, commander }) = setup("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let start = sim.unit(commander).unwrap().pos;
    let lab = sim.types.id("ARMLAB").unwrap();
    let peewee = sim.types.id("ARMPW").unwrap();
    let site = sim.find_build_site(lab, start + FxVec3::from_ints(0, 0, 160), 20).expect("site");
    let factory = sim.spawn_unit(lab, 0, site, 0, true).unwrap();
    sim.set_starting_resources(0, Fx::from_int(10_000), Fx::from_int(10_000));
    // Commander + lab + two more.
    sim.unit_limit = 4;
    sim.step(&[Command::Produce { player: 0, factory, type_id: peewee, count: 5 }]);
    for _ in 0..30 * 90 {
        sim.step(&[]);
    }
    let peewees = sim.units().filter(|u| u.type_id == peewee).count();
    println!("units {} (limit 4), peewees {peewees}, queue left {}", sim.players[0].unit_count, sim.unit(factory).unwrap().factory_queue.len());
    assert_eq!(sim.players[0].unit_count, 4);
    assert_eq!(peewees, 2, "production stops at the limit");
    assert!(!sim.unit(factory).unwrap().factory_queue.is_empty(), "the rest stays queued");
    assert!(sim.players[0].economy.metal_capacity >= Fx::from_int(10_000), "storage holds the starting metal");
}

#[test]
fn factories_keep_building_through_rally_orders_and_send_units_along_the_route() {
    let Some(Fixture { mut sim, commander }) = setup("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let start = sim.unit(commander).unwrap().pos;
    let lab = sim.types.id("ARMLAB").unwrap();
    let peewee = sim.types.id("ARMPW").unwrap();
    let site = sim.find_build_site(lab, start + FxVec3::from_ints(0, 0, 160), 20).expect("site");
    let factory = sim.spawn_unit(lab, 0, site, 0, true).unwrap();
    sim.set_starting_resources(0, Fx::from_int(10_000), Fx::from_int(10_000));
    sim.step(&[Command::Produce { player: 0, factory, type_id: peewee, count: 1 }]);
    let mut frame = None;
    for _ in 0..30 * 10 {
        sim.step(&[]);
        frame = sim.unit(factory).unwrap().build.map(|b| b.target);
        if frame.is_some_and(|f| sim.unit(f).is_some_and(|u| u.build_progress > Fx::from_ratio(1, 10))) {
            break;
        }
    }
    let frame = frame.expect("the lab starts a Peewee");
    // A rally point, then two queued waypoints: a move and a patrol.
    let (a, b, c) = (site + FxVec3::from_ints(200, 0, 150), site + FxVec3::from_ints(-150, 0, 300), site + FxVec3::from_ints(150, 0, 350));
    let order = |order: Order, queue: bool| Command::Order { player: 0, units: vec![factory], order, queue };
    sim.step(&[order(Order::Move(site), false), order(Order::Move(a), false), order(Order::Move(b), true), order(Order::Patrol(c), true)]);
    assert_eq!(sim.unit(factory).unwrap().build.map(|j| j.target), Some(frame), "rally orders don't interrupt production");
    assert_eq!(sim.unit(factory).unwrap().rally, vec![Order::Move(a), Order::Move(b), Order::Patrol(c)]);
    let mut released = None;
    for _ in 0..30 * 60 {
        sim.step(&[]);
        // Released: the factory has handed it its orders.
        if sim.unit(frame).is_some_and(|u| u.is_complete() && !u.orders.is_empty()) {
            released = sim.unit(frame).map(|u| u.orders.iter().cloned().collect::<Vec<_>>());
            break;
        }
    }
    assert_eq!(sim.units().filter(|u| u.type_id == peewee).count(), 1, "one Peewee, not a second frame");
    let orders = released.expect("the Peewee is finished");
    println!("released with {orders:?}");
    // Out of the yard, then the route; the patrol loops between b and c.
    assert!(matches!(orders[0], Order::Move(_)));
    assert_eq!(&orders[1..], &[Order::Move(a), Order::Move(b), Order::Patrol(c), Order::Patrol(b)]);
    // Stop clears the route.
    sim.step(&[order(Order::Stop, false)]);
    assert!(sim.unit(factory).unwrap().rally.is_empty());
}

#[test]
fn geothermal_plants_need_a_vent() {
    let Some(install) = ta_vfs::install::discover(None).ok() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    let geo = types.id("ARMGEO").unwrap();
    assert!(types.get(geo).needs_geo);
    // The first map with vents.
    for path in vfs.list("maps", Some("tnt"), false) {
        let Ok(map) = TntMap::parse(&vfs.read(&path).unwrap()) else { continue };
        let has_vent = map.feature_names.iter().any(|n| data.feature_id(n).is_some_and(|id| data.features[id].geothermal));
        if !has_vent {
            continue;
        }
        let ota = path.replace(".tnt", ".ota");
        let info = MapInfo::from_ota(&tdf::parse_bytes(&vfs.read_opt(&ota).unwrap_or_default()));
        let players = vec![PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }];
        let sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 1);
        let vents: Vec<FxVec3> = sim.features.iter().filter(|f| sim.data.features[f.def].geothermal).map(|f| f.pos).collect();
        println!("{path}: {} vents", vents.len());
        // Most vents take a plant (some sit on cliffs or under rocks), always right over the vent.
        let mut usable = 0;
        for v in &vents {
            if let Some(site) = sim.find_build_site(geo, *v, 2) {
                assert!(site.within_xz(*v, Fx::from_int(48)), "the plant sits on the vent");
                usable += 1;
            }
        }
        println!("{usable} usable");
        assert!(usable * 2 >= vents.len(), "most vents should be usable");
        // Away from every vent there is nowhere to put one.
        let (w, h) = sim.world.size_px();
        let far = [FxVec3::new(w / 4, Fx::ZERO, h / 4), FxVec3::new(w * 3 / 4, Fx::ZERO, h * 3 / 4)]
            .into_iter()
            .find(|p| !vents.iter().any(|v| v.within_xz(*p, Fx::from_int(300))))
            .expect("a spot away from vents");
        assert!(sim.find_build_site(geo, far, 6).is_none(), "no vent, no geothermal plant");
        return;
    }
    panic!("no map with geothermal vents found");
}

#[test]
fn commander_reclaims_its_own_building_for_the_metal() {
    let Some(Fixture { mut sim, commander }) = setup("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let start = sim.unit(commander).unwrap().pos;
    let solar = sim.types.id("ARMSOLAR").unwrap();
    let site = sim.find_build_site(solar, start + FxVec3::from_ints(0, 0, 90), 12).unwrap();
    let building = sim.spawn_unit(solar, 0, site, 0, true).unwrap();
    let cost = sim.types.get(solar).cost_metal.to_f32();
    // Room in storage for the refund.
    sim.set_starting_resources(0, Fx::from_int(100), Fx::from_int(1000));
    let features_before = sim.features.iter().filter(|f| f.alive).count();
    let metal_before = sim.players[0].economy.metal.to_f32();
    sim.step(&[Command::Order { player: 0, units: vec![commander], order: Order::ReclaimUnit(building), queue: false }]);
    let mut gone = None;
    for tick in 0..30 * 60 {
        sim.step(&[]);
        if sim.unit(building).is_none() {
            gone = Some(tick);
            break;
        }
    }
    let gained = sim.players[0].economy.metal.to_f32() - metal_before;
    println!("reclaimed after {gone:?} ticks; metal +{gained:.0} (cost {cost:.0}); commander orders {:?}", sim.unit(commander).map(|u| u.orders.len()));
    assert!(gone.is_some(), "the building is taken apart");
    assert!(gained > cost * 0.9, "most of its metal comes back (income adds a little)");
    assert_eq!(sim.features.iter().filter(|f| f.alive).count(), features_before, "no wreck is left");
    assert!(sim.unit(commander).is_some_and(|u| u.orders.is_empty()));
}

#[test]
fn forests_burn_spread_and_leave_charred_trees() {
    let Some(Fixture { mut sim, .. }) = setup("Sherwood") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    // A flammable tree with flammable neighbours.
    let flammable = |sim: &Sim, i: usize| sim.features[i].alive && sim.data.features[sim.features[i].def].flamable;
    let near = |sim: &Sim, i: usize| (0..sim.features.len()).filter(|&j| j != i && flammable(sim, j) && sim.features[j].pos.within_xz(sim.features[i].pos, Fx::from_int(40))).count();
    let has_charred = |sim: &Sim, i: usize| sim.data.feature_id(&sim.data.features[sim.features[i].def].feature_burnt).is_some();
    let tree = (0..sim.features.len()).filter(|&i| flammable(&sim, i) && has_charred(&sim, i)).max_by_key(|&i| near(&sim, i)).expect("a forest");
    println!("tree {tree} ({}) with {} flammable neighbours", sim.data.features[sim.features[tree].def].name, near(&sim, tree));
    // A tank parked beside it.
    let p = sim.features[tree].pos + FxVec3::from_ints(24, 0, 0);
    let tank = sim.spawn_unit(sim.types.id("ARMSTUMP").unwrap(), 0, p, 0, true).unwrap();
    let hp0 = sim.unit(tank).unwrap().health;
    let burnt_name = sim.data.features[sim.features[tree].def].feature_burnt.to_ascii_lowercase();
    sim.ignite(tree);
    let mut ignited = 1;
    for _ in 0..30 * 60 {
        sim.step(&[]);
        ignited += sim.take_events().iter().filter(|e| matches!(e, ta_sim::SimEvent::FeatureIgnited { .. })).count();
    }
    let hp = sim.unit(tank).map_or(Fx::ZERO, |u| u.health);
    let charred = sim.features.iter().filter(|f| f.alive && sim.data.features[f.def].name.to_ascii_lowercase() == burnt_name).count();
    println!("{ignited} features caught fire; tank {:.0} -> {:.0}; {charred} {burnt_name} standing", hp0.to_f32(), hp.to_f32());
    assert!(!sim.features[tree].alive, "the tree burnt down");
    assert!(charred >= 1, "it left a charred tree");
    assert!(ignited > 1, "the fire spread");
    assert!(hp < hp0, "the fire scorched the tank beside it");
}

/// Seconds a unit of `kind` spawned at `from` (facing `heading`) takes to arrive at `to`, at
/// most `limit`; None if it doesn't.
fn trip(sim: &mut Sim, kind: &str, from: (i64, i64), heading: i32, to: (i64, i64), limit: u64) -> Option<u64> {
    let id = sim.spawn_unit(sim.types.id(kind).unwrap(), 0, FxVec3::from_ints(from.0, 0, from.1), heading, true)?;
    sim.step(&[Command::Order { player: 0, units: vec![id], order: Order::Move(FxVec3::from_ints(to.0, 0, to.1)), queue: false }]);
    let arrived = (0..30 * limit).find(|_| {
        sim.step(&[]);
        sim.unit(id).is_some_and(|u| u.orders.is_empty())
    });
    sim.remove_unit(id);
    arrived.map(|t| t / 30)
}

/// Tanks through Shore To Shore's narrow pass, which runs diagonally across the grid: its route
/// turns at every cell, and a Flash or a Stumpy, too fast for its turning circle, ran past each
/// turn and came round again (44 and 47 s for trips of 12 and 13 at full speed). They steer
/// down its middle (`Sim::carrot`), and a waypoint they run past close by is passed.
#[test]
fn tanks_drive_through_a_narrow_diagonal_pass() {
    let Some(Fixture { mut sim, commander }) = setup("Shore To Shore") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    sim.remove_unit(commander);
    for kind in ["ARMFLASH", "ARMSTUMP", "ARMPW", "ARMHAM"] {
        let took = trip(&mut sim, kind, (1190, 1020), 0, (1578, 1550), 60);
        println!("{kind}: through the pass in {took:?} s");
        assert!(took.is_some_and(|s| s <= 24), "{kind} through the pass in {took:?} s");
    }
}

/// Hovercraft set off on a long trip without dithering (TA's `SteeringMode` 1: they come round
/// in an arc, never turning on the spot): turning round on the spot, a slow-turning one counted
/// as stuck before it faced its way, planned again from its cell's middle, and turned again
/// (the Anaconda took 41 s for a trip of 30 at full speed, a construction hovercraft 61 for 36).
#[test]
fn hovercraft_set_off_without_dithering() {
    let Some(Fixture { mut sim, commander }) = setup("Coast To Coast") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    sim.remove_unit(commander);
    for (kind, most) in [("ARMANAC", 34), ("CORSNAP", 35), ("ARMCH", 47)] {
        let took = trip(&mut sim, kind, (2700, 400), 0, (500, 1200), 90);
        println!("{kind}: 2341 px in {took:?} s");
        assert!(took.is_some_and(|s| s <= most), "{kind} across in {took:?} s");
    }
}

/// Aircraft never fly through the ground: a transport sent from a beach to the cliffs above it
/// rises with the ground (it climbs 2 px a tick, and flew into the cliff until it had climbed
/// out), and one whose pick-up was called off flies on at its cruising height, not at the
/// height it was going down to (it flew on at that, through the hills).
#[test]
fn aircraft_keep_above_the_ground() {
    let Some(Fixture { mut sim, commander }) = setup("Shore To Shore") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    sim.remove_unit(commander);
    let ground = |sim: &Sim, p: FxVec3| sim.world.height_at(p.x, p.z);
    let (beach, cliffs) = (FxVec3::from_ints(1300, 0, 1110), FxVec3::from_ints(900, 0, 700));
    assert!(ground(&sim, cliffs) > ground(&sim, beach) + Fx::from_int(80), "the cliffs stand over the beach");
    let atlas = sim.spawn_unit(sim.types.id("ARMATLAS").unwrap(), 0, beach, 0, true).unwrap();
    let tank = sim.spawn_unit(sim.types.id("ARMSTUMP").unwrap(), 0, beach + FxVec3::from_ints(120, 0, 40), 0, true).unwrap();
    // Off to pick the tank up; called off as it goes down to it, and sent up to the cliffs.
    sim.step(&[Command::Order { player: 0, units: vec![atlas], order: Order::Load(tank), queue: false }]);
    let (mut lowest, mut over_cliffs, mut called_off) = (Fx::MAX, Fx::ZERO, false);
    for _ in 0..30 * 40 {
        let u = sim.unit(atlas).unwrap();
        if !called_off && u.hover_alt.is_some() {
            called_off = true;
            sim.step(&[Command::Order { player: 0, units: vec![atlas], order: Order::Move(cliffs), queue: false }]);
        } else {
            sim.step(&[]);
        }
        let u = sim.unit(atlas).unwrap();
        lowest = lowest.min(u.pos.y - ground(&sim, u.pos));
        if called_off && u.pos.within_xz(cliffs, Fx::from_int(150)) && !u.landed {
            over_cliffs = over_cliffs.max(u.pos.y - ground(&sim, u.pos));
        }
    }
    let cruise = sim.unit_type(sim.unit(atlas).unwrap()).cruise_alt;
    println!("pick-up called off: {called_off}; lowest over the ground {lowest}; over the cliffs up to {over_cliffs} (cruising height {cruise})");
    assert!(called_off, "the transport went down to the tank");
    assert!(lowest >= Fx::ZERO, "never under the ground ({lowest})");
    assert!(over_cliffs >= cruise - Fx::from_int(4), "at its cruising height over the cliffs ({over_cliffs})");
}

/// An army gathered in a pass one unit wide (the computer player's rally point at the top of
/// Shore To Shore's) lets its side's units through: held up by one standing in its way, a unit
/// slips past it, as past one coming the other way. It used to wait behind them for good.
#[test]
fn units_slip_past_their_side_gathered_in_a_pass() {
    let Some(Fixture { mut sim, commander }) = setup("Shore To Shore") else {
        eprintln!("TA install not found; skipping");
        return;
    };
    sim.remove_unit(commander);
    let rally = FxVec3::from_ints(1184, 0, 1008);
    let kind = sim.types.id("ARMPW").unwrap();
    let gathered: Vec<_> = (0..4).map(|i| sim.spawn_unit(kind, 0, rally + FxVec3::from_ints(-40 + i * 20, 0, -20 + i * 10), 0, true).unwrap()).collect();
    sim.step(&[Command::Order { player: 0, units: gathered.clone(), order: Order::Move(rally), queue: false }]);
    for _ in 0..30 * 10 {
        sim.step(&[]);
    }
    assert!(gathered.iter().all(|&g| sim.unit(g).is_some_and(|u| u.orders.is_empty() && u.pos.within_xz(rally, Fx::from_int(64)))), "they have gathered");
    let took = trip(&mut sim, "ARMFLASH", (1000, 900), 16384, (1578, 1550), 60);
    println!("a Flash through the gathering in {took:?} s");
    assert!(took.is_some_and(|s| s <= 30), "through in {took:?} s");
}
