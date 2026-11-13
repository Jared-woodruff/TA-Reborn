//! Original aircraft motion and repair-deck checks; skip without an installed game.

use std::sync::{Arc, OnceLock};
use crate::{Command, Fx, FxVec3, Locomotion, Order, PlayerSetup, Sim, UnitTypes};

type Original = (Arc<ta_data::GameData>, Arc<UnitTypes>);
pub(super) fn original() -> Option<&'static Original> {
    static DATA: OnceLock<Option<Original>> = OnceLock::new();
    DATA.get_or_init(|| {
        let install = ta_vfs::install::discover(None).ok()?;
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").ok()?;
        let data = Arc::new(ta_data::GameData::load(&vfs));
        let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
        Some((data, types))
    }).as_ref()
}

pub(super) fn field(data: &Arc<ta_data::GameData>, types: &Arc<UnitTypes>, water: bool) -> Sim {
    let map = ta_formats::tnt::TntMap { width: 128, height: 128, sea_level: if water { 150 } else { 0 }, heights: vec![60; 128 * 128], features: vec![ta_formats::tnt::FEATURE_NONE; 128 * 128], tiles: Vec::new(), tile_map: Vec::new(), feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    let players = [PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::ZERO, energy: Fx::ZERO, ai: false }];
    let mut sim = Sim::new(types.clone(), data.clone(), &map, &ta_data::MapInfo::default(), 0, &players, 73);
    sim.players[0].endless = true;
    sim.set_starting_resources(0, Fx::from_int(100000), Fx::from_int(100000));
    sim
}

#[test]
fn the_original_aircraft_roster_takes_off_moves_and_lands() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let mut checked = 0;
    for t in types.types.iter().filter(|t| t.locomotion == Locomotion::Air) {
        let mut sim = field(data, types, false);
        let aircraft = sim.spawn_unit(t.id, 0, FxVec3::from_ints(300, 60, 500), 16384, true).unwrap();
        sim.unit_mut(aircraft).unwrap().fire_state = 0;
        let goal = FxVec3::from_ints(700, 60, 500);
        sim.step(&[Command::Order { player: 0, units: vec![aircraft], order: Order::Move(goal), queue: false }]);
        let mut flew = false;
        for _ in 0..1200 {
            sim.step(&[]);
            let u = sim.unit(aircraft).unwrap();
            flew |= u.pos.y > Fx::from_int(80);
            if flew && u.orders.is_empty() && u.landed { break; }
        }
        let u = sim.unit(aircraft).unwrap();
        assert!(flew && u.landed && u.orders.is_empty() && u.pos.within_xz(goal, t.radius * 3 + Fx::from_int(24)), "{} failed its sortie: pos {:?}, landed {}, orders {:?}", t.name, u.pos, u.landed, u.orders);
        checked += 1;
    }
    assert!(checked >= 20, "checked {checked} original aircraft");
    println!("{checked} original aircraft took off, moved and landed");
}

/// Report only: for a 2 × 2 fighter (the Freedom Fighter) and a 3 × 3 bomber (the Thunder), how
/// much of each installed map's dry, open ground they may land on (of what they could before the
/// slope rule), and how much of the dry ground is further than the search reaches from any of it
/// (an idle aircraft there stays in the air). `TA_SURVEY_MAPS=a,b` for some maps.
#[test]
#[ignore]
fn landing_ground_survey() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let Ok(install) = ta_vfs::install::discover(None) else { return };
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    vfs.mount_builtin();
    let only: Option<Vec<String>> = std::env::var("TA_SURVEY_MAPS").ok().map(|s| s.split(',').map(|m| m.trim().to_ascii_lowercase()).collect());
    let kinds: Vec<_> = ["ARMFIG", "ARMTHUND"].iter().map(|n| types.by_name(n).unwrap()).collect();
    for t in &kinds {
        println!("{}: {:?}, rise {}", t.name, t.landing(), t.landing().rise());
    }
    let (mut stranded_maps, mut total) = (Vec::new(), 0);
    for path in vfs.list("maps", Some("tnt"), false) {
        let name = path.trim_start_matches("maps/").trim_end_matches(".tnt").to_string();
        if only.as_ref().is_some_and(|o| !o.contains(&name.to_ascii_lowercase())) { continue; }
        let Some(map) = vfs.read_opt(&path).and_then(|b| ta_formats::tnt::TntMap::parse(&b).ok()) else { println!("{name}: unreadable"); continue };
        let info = vfs.read_opt(&path.replace(".tnt", ".ota")).map(|b| ta_data::MapInfo::from_ota(&ta_formats::tdf::parse_bytes(&b))).unwrap_or_default();
        let players = [PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::ZERO, energy: Fx::ZERO, ai: false }];
        let sim = Sim::new(types.clone(), data.clone(), &map, &info, 0, &players, 73);
        let w = &sim.world;
        let (cw, ch) = (w.width as usize, w.height as usize);
        total += 1;
        let mut line = format!("{name:>28}");
        for t in &kinds {
            let fp = t.footprint;
            let landing = t.landing();
            let flat = crate::world::Landing { reach: 0, ..landing };
            let (mut old, mut level, mut ok, mut was) = (0u64, 0u64, vec![false; cw * ch], vec![false; cw * ch]);
            for z in 0..ch {
                for x in 0..cw {
                    let (x0, z0) = (x as i32 - fp.0 as i32 / 2, z as i32 - fp.1 as i32 / 2);
                    // Before: on the map, free and dry.
                    let free = (z0..z0 + fp.1 as i32).all(|cz| (x0..x0 + fp.0 as i32).all(|cx| w.in_bounds(cx, cz) && w.occupant(cx, cz) == crate::world::Occupant::Free));
                    if !free || (z0..=z0 + fp.1 as i32).any(|vz| (x0..=x0 + fp.0 as i32).any(|vx| w.vertex_height(vx, vz) < w.sea_level)) { continue; }
                    old += 1;
                    was[z * cw + x] = true;
                    let point = crate::sim::anchor_middle((x as i32, z as i32), fp);
                    level += w.air_landing_height(point, &flat).is_some() as u64;
                    ok[z * cw + x] = w.air_landing_height(point, &landing).is_some();
                }
            }
            // Chessboard distance (anchor cells) to the nearest place it may land, two passes.
            let distances = |ok: &[bool]| {
                let far = u32::MAX / 2;
                let mut dist: Vec<u32> = ok.iter().map(|&o| if o { 0 } else { far }).collect();
                for z in 0..ch {
                    for x in 0..cw {
                        for (dx, dz) in [(-1i64, 0i64), (-1, -1), (0, -1), (1, -1)] {
                            let (nx, nz) = (x as i64 + dx, z as i64 + dz);
                            if nx >= 0 && nz >= 0 && (nx as usize) < cw { dist[z * cw + x] = dist[z * cw + x].min(dist[nz as usize * cw + nx as usize] + 1); }
                        }
                    }
                }
                for z in (0..ch).rev() {
                    for x in (0..cw).rev() {
                        for (dx, dz) in [(1i64, 0i64), (1, 1), (0, 1), (-1, 1)] {
                            let (nx, nz) = (x as i64 + dx, z as i64 + dz);
                            if nx >= 0 && (nx as usize) < cw && (nz as usize) < ch { dist[z * cw + x] = dist[z * cw + x].min(dist[nz as usize * cw + nx as usize] + 1); }
                        }
                    }
                }
                dist
            };
            let (now, then) = (distances(&ok), distances(&was));
            // Over dry ground (every other cell; not a space map's void), how often the nearest
            // place is beyond the search.
            let (mut land, mut stranded, mut stranded_before) = (0u64, 0u64, 0u64);
            for z in (0..ch).step_by(2) {
                for x in (0..cw).step_by(2) {
                    if w.vertex_height(x as i32, z as i32) < w.sea_level || w.is_void(x as i32, z as i32) { continue; }
                    land += 1;
                    stranded += (now[z * cw + x] > 16) as u64;
                    stranded_before += (then[z * cw + x] > 16) as u64;
                }
            }
            let pct = |n: u64, of: u64| 100.0 * n as f64 / of.max(1) as f64;
            let may = ok.iter().filter(|&&o| o).count() as u64;
            line += &format!(" | {}: of the open ground level {:.0}%, may {:.0}%; land with none within 16 cells {:.1}% (before {:.1}%)",
                t.name, pct(level, old), pct(may, old), pct(stranded, land), pct(stranded_before, land));
            if pct(stranded, land) > pct(stranded_before, land) + 2.0 { stranded_maps.push(format!("{name} ({}: {:.1}% from {:.1}%)", t.name, pct(stranded, land), pct(stranded_before, land))); }
        }
        println!("{line}");
    }
    println!("{total} maps; more land with nowhere to land within 16 cells (2 points or more): {}", stranded_maps.join(", "));
}

/// A dry 128 × 128-cell field whose heights by column `height(x)` gives.
fn shaped_field(data: &Arc<ta_data::GameData>, types: &Arc<UnitTypes>, height: impl Fn(usize) -> u8) -> Sim {
    let mut map = ta_formats::tnt::TntMap { width: 128, height: 128, sea_level: 0, heights: vec![0; 128 * 128], features: vec![ta_formats::tnt::FEATURE_NONE; 128 * 128], tiles: Vec::new(), tile_map: Vec::new(), feature_names: Vec::new(), minimap: Default::default(), flags: Vec::new() };
    for (i, h) in map.heights.iter_mut().enumerate() {
        *h = height(i % 128);
    }
    let players = [PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::ZERO, energy: Fx::ZERO, ai: false }];
    let mut sim = Sim::new(types.clone(), data.clone(), &map, &ta_data::MapInfo::default(), 0, &players, 73);
    sim.players[0].endless = true;
    sim
}

/// Whether a landed aircraft rests where it may set down, at its height.
fn rests_level(sim: &Sim, id: crate::UnitId) -> bool {
    let u = sim.unit(id).unwrap();
    u.landed && u.air_pad.is_none() && sim.world.air_landing_height(u.pos, &sim.unit_type(u).landing()) == Some(u.pos.y)
}

/// Every original aircraft (fighters and bombers, gunships, transports, constructors, seaplanes)
/// sent to a cliff's edge, told to stop over a hillside, or taken off a patrol over it, lands on
/// level ground near by: never on the slope, against the cliff or hanging over its edge.
#[test]
fn the_original_aircraft_roster_lands_on_level_ground_off_cliffs_and_hillsides() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    // Level at 60 to cell 16, a hillside 4 px a cell to 156 at cell 40 (more than any aircraft's
    // footprint may rise: 6 under 2 × 2 cells, 9 under 3 × 3), level to cell 64, a cliff 64 px up
    // over two cells, level at 220 beyond.
    let height = |x: usize| match x { 0..=16 => 60, 17..=40 => 60 + 4 * (x as u8 - 16), 41..=64 => 156, 65 => 188, _ => 220 };
    let mut checked = 0;
    for t in types.types.iter().filter(|t| t.locomotion == Locomotion::Air) {
        for case in ["cliff", "stop", "patrol"] {
            let mut sim = shaped_field(data, types, height);
            let at = if case == "cliff" { FxVec3::from_ints(700, 0, 1000) } else { FxVec3::from_ints(448, 0, 1000) };
            let aircraft = sim.spawn_airborne(t.id, 0, at, 16384, None).unwrap();
            sim.unit_mut(aircraft).unwrap().fire_state = 0;
            let order = match case {
                "cliff" => Order::Move(FxVec3::from_ints(1048, 0, 1000)),
                "stop" => Order::Stop,
                _ => Order::Patrol(FxVec3::from_ints(560, 0, 1000)),
            };
            sim.step(&[Command::Order { player: 0, units: vec![aircraft], order, queue: false }]);
            if case == "patrol" {
                for _ in 0..150 { sim.step(&[]); }
                sim.step(&[Command::Order { player: 0, units: vec![aircraft], order: Order::Stop, queue: false }]);
            }
            // (Where it came to a stop, with nothing more to do.)
            let mut stopped = None;
            for _ in 0..1500 {
                sim.step(&[]);
                let u = sim.unit(aircraft).unwrap();
                if stopped.is_none() && u.orders.is_empty() && u.speed.is_zero() {
                    stopped = Some(u.pos);
                }
                if u.landed { break; }
            }
            // (Settled: it touches down within a pixel of where it rests.)
            for _ in 0..5 { sim.step(&[]); }
            let u = sim.unit(aircraft).unwrap();
            assert!(rests_level(&sim, aircraft), "{} ({case}) must land on level ground: {:?}, landed {}, goal {:?}", t.name, u.pos, u.landed, u.path_goal);
            assert!(stopped.is_some_and(|s| u.pos.within_xz(s, Fx::from_int(16 * 16 + 24))), "{} ({case}): near where it stopped ({stopped:?}), not at {:?}", t.name, u.pos);
            checked += 1;
        }
    }
    assert!(checked >= 60, "checked {checked} original aircraft landings");
    println!("{checked} original aircraft landings on level ground");
}

/// TA's seaplanes (`Amphibious=1`) set down on the sea's surface; the other aircraft, the
/// fighters and torpedo bombers that use the carriers (`MaxWaterDepth=255` without it) too,
/// stay in the air over the open sea.
#[test]
fn original_seaplanes_alone_land_on_the_water() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    let mut seaplanes = Vec::new();
    for t in types.types.iter().filter(|t| t.locomotion == Locomotion::Air) {
        let mut sim = field(data, types, true);
        let aircraft = sim.spawn_airborne(t.id, 0, FxVec3::from_ints(1000, 0, 1000), 0, None).unwrap();
        sim.unit_mut(aircraft).unwrap().fire_state = 0;
        sim.step(&[Command::Order { player: 0, units: vec![aircraft], order: Order::Stop, queue: false }]);
        for _ in 0..900 { sim.step(&[]); }
        let u = sim.unit(aircraft).unwrap();
        if t.def.amphibious {
            assert!(u.landed && u.pos.y == sim.world.sea_level, "{} lands on the water: {:?}", t.name, u.pos);
            seaplanes.push(t.name.clone());
        } else {
            assert!(!u.landed && u.pos.y > sim.world.sea_level + Fx::from_int(40), "{} stays up over the sea: {:?}", t.name, u.pos);
        }
    }
    assert!(seaplanes.len() >= 8, "TA's seaplanes: {seaplanes:?}");
}

#[test]
fn original_carriers_use_both_authored_decks_and_carry_parked_aircraft() {
    let Some((data, types)) = original() else { eprintln!("TA install not found; skipping"); return };
    for name in ["ARMCARRY", "CORCARRY"] {
        let mut sim = field(data, types, true);
        let carrier = sim.spawn_unit(types.id(name).unwrap(), 0, FxVec3::from_ints(600, 150, 600), 0, true).unwrap();
        let mut planes = Vec::new();
        for x in [580, 620] {
            let plane = sim.spawn_unit(types.id("ARMFIG").unwrap(), 0, FxVec3::from_ints(x, 150, 600), 0, true).unwrap();
            sim.unit_mut(plane).unwrap().pos.y = Fx::from_int(230);
            sim.unit_mut(plane).unwrap().health = Fx::ONE;
            sim.unit_mut(plane).unwrap().fire_state = 0;
            planes.push(plane);
        }
        for _ in 0..1200 {
            sim.step(&[]);
            if planes.iter().all(|&id| sim.unit(id).is_some_and(|u| u.landed && u.health > Fx::ONE)) { break; }
        }
        for &id in &planes {
            let u = sim.unit(id).unwrap();
            assert!(u.landed && u.air_pad.is_some() && u.health > Fx::ONE, "{name}: aircraft {id:?} failed to land/repair: {:?}, {:?}", u.pos, u.air_pad);
            assert!(u.pos.y > sim.unit(carrier).unwrap().pos.y, "{name} aircraft must sit on the deck surface");
        }
        assert_ne!(sim.unit(planes[0]).unwrap().air_pad, sim.unit(planes[1]).unwrap().air_pad, "each aircraft reserves a distinct authored deck");
        sim.step(&[Command::Order { player: 0, units: vec![carrier], order: Order::Move(FxVec3::from_ints(600, 150, 900)), queue: false }]);
        for _ in 0..120 { sim.step(&[]); }
        for &id in &planes { assert!(sim.unit(id).unwrap().landed && sim.unit(id).unwrap().pos.within_xz(sim.unit(carrier).unwrap().pos, Fx::from_int(100))); }
    }
}
