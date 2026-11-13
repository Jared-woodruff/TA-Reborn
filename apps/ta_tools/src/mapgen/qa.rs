//! The built-in maps as a player finds them, judged by the game's own rules rather than the
//! generator's: every movement class's way from base to base round the features, an extractor on
//! every deposit, a geothermal plant on every vent, room for a base, the same metal by every
//! base, tidal generators where the map promises tides, a shipyard near every base on the maps
//! for fleets and every ship's way between the yards, what the features hold and what a map costs
//! to load. Skipped without a TA install.
//!
//! `cargo test --release -p ta_tools mapgen` holds every map to it;
//! `cargo test --release -p ta_tools maps_as_played -- --ignored --nocapture` prints the report
//! (`TA_MAPS=a,b` for some, TA's own maps too, to compare with; `TA_REBORN_ASSETS=<folder>` for
//! maps made into a scratch folder).

use std::collections::{BTreeMap, BinaryHeap};
use std::sync::Arc;
use std::time::Instant;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::world::{MoveSpec, World};
use ta_sim::{Fx, FxVec3, Locomotion, PlayerSetup, Sim, UnitTypes};

struct Game {
    vfs: ta_vfs::Vfs,
    data: Arc<GameData>,
    types: Arc<UnitTypes>,
}

fn game() -> Option<Game> {
    let install = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&install.root, "base").unwrap();
    // (TA Reborn's own features, the festive ones of the winter maps.)
    vfs.mount_builtin();
    let data = Arc::new(GameData::load(&vfs));
    let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    Some(Game { vfs, data, types })
}

/// The folder the maps are read from.
fn maps_dir() -> std::path::PathBuf {
    match std::env::var_os("TA_REBORN_ASSETS") {
        Some(d) => std::path::PathBuf::from(d).join("maps"),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/reborn/maps"),
    }
}

/// The cells reached from `from` over `open` anchors, as the pathfinder moves (8 neighbours, no
/// corner cut), with the way's length to each (tenths of a cell; `u32::MAX` not reached).
fn distances(w: usize, h: usize, open: &[bool], from: (i32, i32)) -> Vec<u32> {
    let mut dist = vec![u32::MAX; w * h];
    let i0 = from.1 as usize * w + from.0 as usize;
    if !open[i0] {
        return dist;
    }
    let mut heap = BinaryHeap::new();
    dist[i0] = 0;
    heap.push(std::cmp::Reverse((0u32, i0 as u32)));
    let ok = |x: i32, z: i32| x >= 0 && z >= 0 && (x as usize) < w && (z as usize) < h && open[z as usize * w + x as usize];
    while let Some(std::cmp::Reverse((d, i))) = heap.pop() {
        let i = i as usize;
        if d > dist[i] {
            continue;
        }
        let (x, z) = ((i % w) as i32, (i / w) as i32);
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let (nx, nz) = (x + dx, z + dz);
            if !ok(nx, nz) || (dx != 0 && dz != 0 && !(ok(x + dx, z) && ok(x, z + dz))) {
                continue;
            }
            let n = nz as usize * w + nx as usize;
            let nd = d + if dx != 0 && dz != 0 { 14 } else { 10 };
            if nd < dist[n] {
                dist[n] = nd;
                heap.push(std::cmp::Reverse((nd, n as u32)));
            }
        }
    }
    dist
}

/// The open anchor nearest `at` within `r` cells, if any.
fn nearest_open(w: usize, h: usize, open: &[bool], at: (i32, i32), r: i32) -> Option<(i32, i32)> {
    for ring in 0..=r {
        for dz in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs() != ring && dz.abs() != ring {
                    continue;
                }
                let (x, z) = (at.0 + dx, at.1 + dz);
                if x >= 0 && z >= 0 && (x as usize) < w && (z as usize) < h && open[z as usize * w + x as usize] {
                    return Some((x, z));
                }
            }
        }
    }
    None
}

/// The fleet maps also need a depth regression on CI machines with no TA install.
#[test]
fn fleet_maps_connect_submarines_in_generated_and_shipped_terrain() {
    for id in ["crescent-keys", "stormbreak-archipelago", "shore-to-shore-reborn"] {
        let recipe = super::recipe(id).unwrap();
        for generated in [false, true] {
            let (map, starts) = if generated {
                let land = super::terrain::build(&recipe.shape);
                let map = TntMap {
                    width: land.w as u32, height: land.h as u32, sea_level: land.sea as i32,
                    heights: land.height.data.iter().map(|h| h.round().clamp(0.0, 255.0) as u8).collect(),
                    features: vec![ta_formats::tnt::FEATURE_NONE; land.w * land.h],
                    tiles: Vec::new(), tile_map: Vec::new(), feature_names: Vec::new(), minimap: ta_formats::image::IndexedImage::new(0, 0, 0),
                    flags: Vec::new(),
                };
                (map, land.starts)
            } else {
                let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/reborn/maps");
                let map = TntMap::parse(&std::fs::read(dir.join(format!("{}.tnt", recipe.name))).unwrap()).unwrap();
                let info = MapInfo::from_ota(&tdf::parse_bytes(&std::fs::read(dir.join(format!("{}.ota", recipe.name))).unwrap()));
                (map, info.schemas[0].start_positions.iter().map(|&(_, x, z)| (x / 16.0, z / 16.0)).collect())
            };
            let mut world = World::new(&map);
            let spec = MoveSpec { footprint: (3, 3), min_depth: 25, max_depth: 10_000, max_slope: 255, hover: false };
            world.prepare(&spec);
            let (w, h) = (map.width as usize, map.height as usize);
            let open = world.passable_all(&spec);
            let first = nearest_open(w, h, &open, (starts[0].0 as i32, starts[0].1 as i32), 56).expect("deep water near the first base");
            let d = distances(w, h, &open, first);
            let reached: Vec<bool> = d.iter().map(|&d| d != u32::MAX).collect();
            for (k, &(x, z)) in starts.iter().enumerate() {
                assert!(nearest_open(w, h, &reached, (x as i32, z as i32), 56).is_some(), "{}: start {}, generated {generated} cannot reach the common deep sea", recipe.name, k + 1);
            }
        }
    }
}

type SpecKey = ((u32, u32), i32, i32, i32, bool);

fn key(s: &MoveSpec) -> SpecKey {
    (s.footprint, s.max_slope, s.max_depth, s.min_depth, s.hover)
}

/// What a map is found to be: the report's lines, and what the test holds it to.
#[derive(Default)]
struct Judged {
    lines: Vec<String>,
    /// Feature names the game has no definition of.
    unknown: Vec<String>,
    /// The starts a tank and a kbot cannot drive to from the first (1-based), and (two coasts
    /// across an ocean) from the first on their own coast.
    land_missed: Vec<usize>,
    coast_missed: Vec<usize>,
    /// Deposits no extractor (land or under water, either side's) can be built on, and vents no
    /// geothermal plant can.
    bad_deposits: Vec<String>,
    bad_vents: Vec<String>,
    /// Per start: usable deposits within 36 cells and the metal they hold (what extractors on
    /// them would sum), whether a lab fits within 12 cells, the share of the ground within 20 a
    /// vehicle plant fits on, the nearest shipyard site (cells, as the crow flies and as the
    /// computer players search: in rings).
    base_deposits: Vec<usize>,
    base_metal: Vec<i64>,
    lab_near: Vec<bool>,
    room: Vec<f32>,
    yard: Vec<Option<(f32, i32)>>,
    /// The yards each ship class (by its names) cannot reach from the first's (1-based).
    ships_missed: Vec<(String, Vec<usize>)>,
    /// The submarines (by their names) that cannot sail out of a base's yard (1-based) into its
    /// sea: less than half of the water they sail in the sea the yard's battleships sail.
    subs_shut_in: Vec<(String, usize)>,
    tidal: f64,
    tidal_sites: [usize; 2],
    metal_world: bool,
}

/// `coasts`: the map is two coasts across an ocean, its bases judged coast by coast too.
fn judge(g: &Game, name: &str, coasts: bool) -> Option<Judged> {
    // (A map of TA's own, named in `TA_MAPS`, from the install: to compare with.)
    let dir = maps_dir();
    let read = |ext: &str| std::fs::read(dir.join(format!("{name}.{ext}"))).ok().or_else(|| g.vfs.read_opt(&format!("maps/{name}.{ext}")));
    let (tnt, ota) = (read("tnt")?, read("ota")?);
    let mut j = Judged::default();
    let t0 = Instant::now();
    let map = TntMap::parse(&tnt).unwrap();
    let parse_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let info = MapInfo::from_ota(&tdf::parse_bytes(&ota));
    let schema = &info.schemas[0];
    let starts: Vec<(f32, f32)> = schema.start_positions.iter().map(|&(_, x, z)| (x, z)).collect();
    let players: Vec<PlayerSetup> = (0..2)
        .map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::new(Fx::from_f32(starts[i].0), Fx::ZERO, Fx::from_f32(starts[i].1)), metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
        .collect();
    let t1 = Instant::now();
    let sim = Sim::new(g.types.clone(), g.data.clone(), &map, &info, 0, &players, 1);
    let sim_ms = t1.elapsed().as_secs_f64() * 1000.0;
    let world = &sim.world;
    let (w, h) = (world.width as usize, world.height as usize);
    let start_cells: Vec<(i32, i32)> = starts.iter().map(|&s| ((s.0 / 16.0) as i32, (s.1 / 16.0) as i32)).collect();
    j.tidal = info.tidal_strength;
    j.metal_world = schema.surface_metal >= 100.0;
    j.lines.push(format!(
        "{name}: {w}x{h} cells ({}), {} players, \"{}\" planet {}, sea {}; {} tiles, {:.1} MB; parse {parse_ms:.0} ms, sim set-up {sim_ms:.0} ms",
        info.size,
        starts.len(),
        info.num_players,
        info.planet,
        map.sea_level,
        map.tiles.len(),
        tnt.len() as f64 / 1e6
    ));
    j.lines.push(format!("  wind {}-{}, tidal {}, solar {}, gravity {}, surface metal {}, moho {}", info.min_wind, info.max_wind, info.tidal_strength, info.solar_strength, info.gravity, schema.surface_metal, schema.moho_metal));
    j.lines.push(format!("  \"{}\"", info.description));
    // The water.
    let cells = (w * h) as f64;
    let (mut wet, mut deep, mut void) = (0usize, 0usize, 0usize);
    for z in 0..h as i32 {
        for x in 0..w as i32 {
            if world.is_void(x, z) {
                void += 1;
            } else if world.cell_depth(x, z) > 0 {
                wet += 1;
                if world.cell_depth(x, z) >= 15 {
                    deep += 1;
                }
            }
        }
    }
    j.lines.push(format!("  water {:.1}% of the map ({:.1}% 15 or deeper), void {:.1}%", wet as f64 * 100.0 / cells, deep as f64 * 100.0 / cells, void as f64 * 100.0 / cells));
    // The features: what each holds, and which stand in the way.
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let (mut blocking, mut metal_held, mut energy_held) = (0usize, 0.0f64, 0.0f64);
    for f in &sim.features {
        let def = &g.data.features[f.def];
        *kinds.entry(def.category.to_ascii_lowercase()).or_default() += 1;
        if def.blocking && !def.is_metal_deposit() && !def.geothermal {
            blocking += 1;
        }
        if def.reclaimable {
            metal_held += def.metal;
            energy_held += def.energy;
        }
    }
    j.unknown = map.feature_names.iter().filter(|n| g.data.feature_id(n).is_none()).cloned().collect();
    j.lines.push(format!("  {} features ({blocking} in the way), {:.0} per thousand cells; reclaim {metal_held:.0} metal, {energy_held:.0} energy; by kind {kinds:?}", sim.features.len(), sim.features.len() as f64 * 1000.0 / cells));
    // The unit types the checks build.
    let tid = |n: &str| g.types.id(n).unwrap_or_else(|| panic!("no {n}"));
    let slope_of = |id: usize| sim.move_spec(id).max_slope.max(g.types.get(id).def.max_slope as i32);
    let can = |id: usize, origin: (i32, i32)| world.placement_error(g.types.get(id), slope_of(id), origin);
    // Every movement class of the two sides' units: the way from the first base to the others.
    let mut specs: BTreeMap<SpecKey, (MoveSpec, Locomotion, Vec<String>)> = BTreeMap::new();
    for t in &g.types.types {
        if !(t.name.starts_with("ARM") || t.name.starts_with("COR")) || !matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval) || t.max_speed <= Fx::ZERO {
            continue;
        }
        let s = sim.move_spec(t.id);
        specs.entry(key(&s)).or_insert((s, t.locomotion, Vec::new())).2.push(t.name.clone());
    }
    let (tank, kbot) = (sim.move_spec(tid("ARMSTUMP")), sim.move_spec(tid("ARMPW")));
    for (spec, loco, names) in specs.values() {
        if *loco == Locomotion::Naval {
            continue;
        }
        let open = world.passable_all(spec);
        let a0 = nearest_open(w, h, &open, World::pos_to_anchor(FxVec3::new(Fx::from_f32(starts[0].0), Fx::ZERO, Fx::from_f32(starts[0].1)), spec.footprint), 6);
        let reach: Vec<bool> = match a0 {
            None => vec![false; starts.len()],
            Some(a0) => {
                let d = distances(w, h, &open, a0);
                start_cells.iter().map(|&s| nearest_open(w, h, &open, s, 6).is_some_and(|a| d[a.1 as usize * w + a.0 as usize] != u32::MAX)).collect()
            }
        };
        let missed: Vec<usize> = reach.iter().enumerate().filter(|(_, r)| !**r).map(|(k, _)| k + 1).collect();
        if !missed.is_empty() {
            j.lines.push(format!("  {:?} slope {} depth {} ({:?}{}): cannot reach start(s) {missed:?} from start 1", spec.footprint, spec.max_slope, spec.max_depth, names.iter().take(4).collect::<Vec<_>>(), if names.len() > 4 { "..." } else { "" }));
            if *spec == tank || *spec == kbot {
                j.land_missed.extend(missed.iter().filter(|k| !j.land_missed.contains(k)).copied().collect::<Vec<_>>());
            }
        }
    }
    // The tanks' ways: each base's nearest other by road and as the crow flies.
    let open_tank = world.passable_all(&tank);
    let mut road = Vec::new();
    for (k, &s) in start_cells.iter().enumerate() {
        let Some(a) = nearest_open(w, h, &open_tank, s, 6) else {
            road.push(format!("{}: nowhere to stand", k + 1));
            continue;
        };
        let d = distances(w, h, &open_tank, a);
        let mut best: Option<(f32, f32, usize)> = None;
        for (i, &o) in start_cells.iter().enumerate() {
            if i == k {
                continue;
            }
            let crow = ((o.0 - s.0) as f32).hypot((o.1 - s.1) as f32);
            let by_road = nearest_open(w, h, &open_tank, o, 6).map(|b| d[b.1 as usize * w + b.0 as usize]).filter(|&v| v != u32::MAX).map_or(f32::INFINITY, |v| v as f32 / 10.0);
            if best.is_none_or(|b| crow < b.0) {
                best = Some((crow, by_road, i));
            }
        }
        if let Some((crow, by_road, i)) = best {
            road.push(format!("{}->{} {by_road:.0}/{crow:.0}", k + 1, i + 1));
        }
    }
    j.lines.push(format!("  tanks, nearest base by road/crow (cells): {}", road.join(", ")));
    // Two coasts across an ocean: each base from the first on its own coast (the west half's or
    // the east's), for a tank and a kbot.
    if coasts {
        let middle = w as i32 / 2;
        for spec in [tank, kbot] {
            let open = world.passable_all(&spec);
            for west in [true, false] {
                let mine: Vec<usize> = (0..start_cells.len()).filter(|&k| (start_cells[k].0 < middle) == west).collect();
                let Some(&first) = mine.first() else { continue };
                let d = nearest_open(w, h, &open, start_cells[first], 6).map(|a| distances(w, h, &open, a));
                for &k in &mine[1..] {
                    let reached = d.as_ref().is_some_and(|d| nearest_open(w, h, &open, start_cells[k], 6).is_some_and(|a| d[a.1 as usize * w + a.0 as usize] != u32::MAX));
                    if !reached && !j.coast_missed.contains(&(k + 1)) {
                        j.coast_missed.push(k + 1);
                    }
                }
            }
        }
        j.lines.push(format!("  each coast's bases by land: {}", if j.coast_missed.is_empty() { String::from("all joined") } else { format!("cannot reach start(s) {:?}", j.coast_missed) }));
    }
    // Metal: an extractor on every deposit, on land or under the sea, of either side.
    let (mexes, geos) = (["ARMMEX", "ARMUWMEX", "CORMEX", "CORUWMEX"].map(tid), ["ARMGEO", "CORGEO"].map(tid));
    let mut deposits: Vec<((f32, f32), i64, Option<String>)> = Vec::new();
    let mut vents: Vec<((f32, f32), Option<String>)> = Vec::new();
    for f in &sim.features {
        let def = &g.data.features[f.def];
        let (fx, fz) = (def.footprint_x as i32, def.footprint_z as i32);
        let centre = (f.origin.0 as f32 + fx as f32 * 0.5, f.origin.1 as f32 + fz as f32 * 0.5);
        // Whether a building of `id` fits over the feature (every origin putting its footprint
        // over a cell of it), and what fits best.
        let over = |id: usize| {
            let (bx, bz) = g.types.get(id).footprint;
            let (bx, bz) = (bx as i32, bz as i32);
            let mut fits = Vec::new();
            for oz in f.origin.1 - bz + 1..=f.origin.1 + fz - 1 {
                for ox in f.origin.0 - bx + 1..=f.origin.0 + fx - 1 {
                    if can(id, (ox, oz)).is_none() {
                        fits.push(world.metal_under((ox, oz), (bx as u32, bz as u32)));
                    }
                }
            }
            (fits.iter().copied().max(), can(id, (f.origin.0 - (bx - fx) / 2, f.origin.1 - (bz - fz) / 2)))
        };
        let why = |e: Option<ta_sim::world::PlacementError>| format!("({:.0}, {:.0}) {} (depth {}, slope {})", centre.0, centre.1, e.map_or(String::from("off its middle"), |e| format!("{e:?}")), world.cell_depth(f.origin.0, f.origin.1), world.slope(f.origin.0, f.origin.1));
        if def.is_metal_deposit() {
            // (Each side must be able to mine it: on land, or under the sea with its own
            // underwater extractor.)
            let sides: Vec<Option<i64>> = mexes.chunks(2).map(|pair| pair.iter().filter_map(|&id| over(id).0).max()).collect();
            let value = sides.iter().flatten().copied().max().unwrap_or(0);
            deposits.push((centre, value, sides.iter().any(|s| s.is_none()).then(|| why(over(mexes[0]).1))));
        } else if def.geothermal {
            let fits = geos.iter().all(|&id| over(id).0.is_some());
            vents.push((centre, (!fits).then(|| why(over(geos[0]).1))));
        }
    }
    j.bad_deposits = deposits.iter().filter_map(|d| d.2.clone()).collect();
    let wet_metal = deposits.iter().filter(|(c, _, _)| world.cell_depth(c.0 as i32, c.1 as i32) > 0).count();
    j.lines.push(format!("  metal: {} deposits ({wet_metal} under water), {} where a side cannot extract it", deposits.len(), j.bad_deposits.len()));
    if !j.bad_deposits.is_empty() {
        j.lines.push(format!("    unusable: {}", j.bad_deposits.iter().take(12).cloned().collect::<Vec<_>>().join("; ")));
    }
    j.bad_vents = vents.iter().filter_map(|v| v.1.clone()).collect();
    j.lines.push(format!("  vents: {} ({} where a side cannot build a geothermal plant)", vents.len(), j.bad_vents.len()));
    if !j.bad_vents.is_empty() {
        j.lines.push(format!("    unusable: {}", j.bad_vents.iter().take(12).cloned().collect::<Vec<_>>().join("; ")));
    }
    // Tides: where either side's tidal generator can stand (the menus show the map's tides).
    for (k, t) in ["ARMTIDE", "CORTIDE"].iter().enumerate() {
        let id = tid(t);
        j.tidal_sites[k] = (0..h as i32).step_by(2).flat_map(|z| (0..w as i32).step_by(2).map(move |x| (x, z))).filter(|&o| can(id, o).is_none()).count();
    }
    j.lines.push(format!("  tidal generators fit at {} (ARM) and {} (CORE) of every 4 cells' spots", j.tidal_sites[0], j.tidal_sites[1]));
    // Each base: its metal, its nearest vent, its room, what stands round it, its shipyard.
    let (lab, plant, yard) = (tid("ARMLAB"), tid("ARMVP"), tid("ARMSY"));
    let seaworthy = wet as f64 / cells > 0.03 && !info.lava_world;
    let mut yards: Vec<Option<(i32, i32)>> = Vec::new();
    for (k, &s) in start_cells.iter().enumerate() {
        let sf = (s.0 as f32 + 0.5, s.1 as f32 + 0.5);
        let near: Vec<_> = deposits.iter().filter(|d| (d.0.0 - sf.0).hypot(d.0.1 - sf.1) < 36.0).collect();
        let usable = near.iter().filter(|d| d.2.is_none()).count();
        let value: i64 = near.iter().filter(|d| d.2.is_none()).map(|d| d.1).sum();
        let vent = vents.iter().filter(|v| v.1.is_none()).map(|v| (v.0.0 - sf.0).hypot(v.0.1 - sf.1)).fold(f32::INFINITY, f32::min);
        // Room: the share of the ground within 20 cells a vehicle plant fits on, and a lab within 12.
        let (mut fits, mut tried) = (0, 0);
        let (px, pz) = g.types.get(plant).footprint;
        for dz in (-20..=20).step_by(2) {
            for dx in (-20..=20).step_by(2) {
                if dx * dx + dz * dz > 400 {
                    continue;
                }
                tried += 1;
                if can(plant, (s.0 + dx - px as i32 / 2, s.1 + dz - pz as i32 / 2)).is_none() {
                    fits += 1;
                }
            }
        }
        let (lx, lz) = g.types.get(lab).footprint;
        let lab_near = (-12..=12).any(|dz: i32| (-12..=12).any(|dx: i32| dx * dx + dz * dz <= 144 && can(lab, (s.0 + dx - lx as i32 / 2, s.1 + dz - lz as i32 / 2)).is_none()));
        let in_way = sim
            .features
            .iter()
            .filter(|f| {
                let def = &g.data.features[f.def];
                def.blocking && !def.is_metal_deposit() && !def.geothermal && ((f.origin.0 - s.0) as f32).hypot((f.origin.1 - s.1) as f32) < 16.0
            })
            .count();
        // The nearest shipyard site, on a sea map, found as the computer players look, ring by ring
        // round the base: the yard fits, 3 cells off the map's edge, its biggest ship can sail a
        // few cells straight out of its front (a factory's units leave by its south side), and the
        // Commander can walk within building reach of it.
        let mut yard_at = None;
        if seaworthy {
            let (yx, yz) = g.types.get(yard).footprint;
            let (yx, yz) = (yx as i32, yz as i32);
            let ship = g.types.get(yard).build_options.iter().map(|&o| sim.move_spec(o)).filter(|sp| sp.min_depth > 0).max_by_key(|sp| (sp.footprint.0 * sp.footprint.1, sp.min_depth)).unwrap_or(sim.move_spec(tid("ARMBATS")));
            let commander = sim.move_spec(tid("ARMCOM"));
            let walks = world.passable_all(&commander);
            let home = nearest_open(w, h, &walks, s, 6).map(|a| distances(w, h, &walks, a));
            let reach = (g.types.get(tid("ARMCOM")).build_distance.to_int() / 16) as i32 + (yx.max(yz) + 1) / 2 + 1;
            let usable = |anchor: (i32, i32)| {
                let o = (anchor.0 - yx / 2, anchor.1 - yz / 2);
                if o.0 < 3 || o.1 < 3 || o.0 + yx + 3 > w as i32 || o.1 + yz + 3 > h as i32 || can(yard, o).is_some() {
                    return false;
                }
                let (cx, first) = (o.0 + yx / 2, o.1 + yz + ship.footprint.1 as i32 / 2);
                if !(0..4).all(|dz| (-1..=1).all(|dx| world.passable(&ship, (cx + dx, first + dz), None))) {
                    return false;
                }
                home.as_ref().is_some_and(|d| (-reach..=reach).any(|dz| (-reach..=reach).any(|dx| {
                    let (x, z) = (anchor.0 + dx, anchor.1 + dz);
                    x >= 0 && z >= 0 && (x as usize) < w && (z as usize) < h && d[z as usize * w + x as usize] != u32::MAX
                })))
            };
            'search: for ring in 0..=110i32 {
                for dz in -ring..=ring {
                    for dx in -ring..=ring {
                        if dx.abs() != ring && dz.abs() != ring {
                            continue;
                        }
                        if usable((s.0 + dx, s.1 + dz)) {
                            yard_at = Some(((s.0 + dx, s.1 + dz), ring));
                            break 'search;
                        }
                    }
                }
            }
        }
        yards.push(yard_at.map(|y| y.0));
        let crow = |y: (i32, i32)| ((y.0 - s.0) as f32).hypot((y.1 - s.1) as f32);
        j.yard.push(yard_at.map(|(y, ring)| (crow(y), ring)));
        let yard_text = if seaworthy { yard_at.map_or(String::from(", no shipyard within 110 cells"), |(y, ring)| format!(", shipyard {:.0} cells off ({ring} in rings)", crow(y))) } else { String::new() };
        j.lines.push(format!(
            "  start {} at ({}, {}) px: metal {usable}/{} deposits within 36 cells (holding {value}), vent {vent:.0} cells, room {:.0}%, lab within 12 {}, {in_way} in the way within 16{yard_text}",
            k + 1,
            starts[k].0,
            starts[k].1,
            near.len(),
            fits as f32 * 100.0 / tried as f32,
            if lab_near { "yes" } else { "NO" },
        ));
        j.base_deposits.push(usable);
        j.base_metal.push(value);
        j.lab_near.push(lab_near);
        j.room.push(fits as f32 / tried as f32);
    }
    // The fleets: from the first base's yard to the others', for every class of ship.
    if seaworthy && yards.iter().flatten().count() >= 2 {
        for (spec, loco, names) in specs.values() {
            if *loco != Locomotion::Naval {
                continue;
            }
            let shown = format!("{:?}", names.iter().take(3).collect::<Vec<_>>());
            let open = world.passable_all(spec);
            let Some(first) = yards.iter().flatten().next() else { continue };
            let Some(a0) = nearest_open(w, h, &open, *first, 8) else {
                j.lines.push(format!("  ships {:?} min depth {} ({shown}): cannot float at the first yard", spec.footprint, spec.min_depth));
                j.ships_missed.push((shown, (1..=starts.len()).collect()));
                continue;
            };
            let d = distances(w, h, &open, a0);
            let missed: Vec<usize> = yards.iter().enumerate().filter(|(_, y)| y.is_some_and(|y| nearest_open(w, h, &open, y, 8).is_none_or(|a| d[a.1 as usize * w + a.0 as usize] == u32::MAX))).map(|(k, _)| k + 1).collect();
            let sea = d.iter().filter(|&&v| v != u32::MAX).count();
            j.lines.push(format!(
                "  ships {:?} min depth {} ({shown}): {:.1}% of the map to sail from the first yard{}",
                spec.footprint,
                spec.min_depth,
                sea as f64 * 100.0 / cells,
                if missed.is_empty() { String::new() } else { format!(", cannot reach the yards of start(s) {missed:?}") }
            ));
            if !missed.is_empty() {
                j.ships_missed.push((shown, missed));
            }
        }
        // The submarines out of every yard into its sea: the sea that yard's battleships sail
        // (the deepest-drawing of the biggest surface ships), of which the submarines must reach
        // half the water they can sail at least (Twin Reefs' harbour channels ended on a sill, and
        // its submarines never left their harbours).
        let surface = specs.values().filter(|(sp, loco, _)| *loco == Locomotion::Naval && sp.min_depth < 20).map(|(sp, ..)| sp).max_by_key(|sp| (sp.footprint.0 * sp.footprint.1, sp.min_depth));
        if let Some(surface) = surface {
            let wide = world.passable_all(surface);
            for (spec, _, names) in specs.values().filter(|(sp, loco, _)| *loco == Locomotion::Naval && sp.min_depth >= 20) {
                let shown = format!("{:?}", names.iter().take(3).collect::<Vec<_>>());
                let open = world.passable_all(spec);
                for (k, yard) in yards.iter().enumerate() {
                    let Some(yard) = yard else { continue };
                    let (Some(a), Some(b)) = (nearest_open(w, h, &open, *yard, 8), nearest_open(w, h, &wide, *yard, 8)) else { continue };
                    let (reach, sea) = (distances(w, h, &open, a), distances(w, h, &wide, b));
                    let reached = reach.iter().filter(|&&d| d != u32::MAX).count();
                    let theirs = (0..w * h).filter(|&i| open[i] && sea[i] != u32::MAX).count();
                    if reached * 2 < theirs {
                        j.lines.push(format!("  submarines {:?} min depth {} ({shown}) from start {}'s yard: {reached} cells of the {theirs} they sail in its sea", spec.footprint, spec.min_depth, k + 1));
                        j.subs_shut_in.push((shown.clone(), k + 1));
                    }
                }
            }
        }
    }
    Some(j)
}

/// Every built-in map holds to the rules a player takes for granted (see the module's note).
#[test]
fn the_built_in_maps_play_by_the_games_rules() {
    let Some(g) = game() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let mut wrong = Vec::new();
    for id in super::IDS {
        let r = super::recipe(id).unwrap();
        let Some(j) = judge(&g, r.name, r.shape.coasts.is_some()) else { panic!("{}: not found", r.name) };
        let mut fault = |what: String| wrong.push(format!("{}: {what}", r.name));
        if !j.unknown.is_empty() {
            fault(format!("features the game has no definition of: {:?}", j.unknown));
        }
        // Land maps: a tank and a kbot drive from every base to every other, round the
        // features too.
        if r.shape.connect && !j.land_missed.is_empty() {
            fault(format!("no way by land to start(s) {:?}", j.land_missed));
        }
        // Two coasts: every base on a coast from the others there.
        if !j.coast_missed.is_empty() {
            fault(format!("no way by land to start(s) {:?} from the first base on their coast", j.coast_missed));
        }
        if !j.bad_deposits.is_empty() {
            fault(format!("{} deposits a side cannot mine, e.g. {:?}", j.bad_deposits.len(), &j.bad_deposits[..j.bad_deposits.len().min(3)]));
        }
        // (The first maps keep a vent or two too steep for a plant, as first made.)
        let new = r.kit.base_metal > 0;
        if new && !j.bad_vents.is_empty() {
            fault(format!("{} vents a side cannot build a geothermal plant on, e.g. {:?}", j.bad_vents.len(), &j.bad_vents[..j.bad_vents.len().min(3)]));
        }
        // Every base: its own deposits, the same metal as every other base's, room to build.
        if !j.metal_world {
            let want = if new { r.kit.base_metal } else { 4 };
            if let Some(k) = j.base_deposits.iter().position(|&n| n < want) {
                fault(format!("start {} has {} deposits near it", k + 1, j.base_deposits[k]));
            }
            let (lo, hi) = (j.base_metal.iter().min().copied().unwrap_or(0), j.base_metal.iter().max().copied().unwrap_or(0));
            if new && hi as f64 > lo as f64 * 1.15 {
                fault(format!("the bases' metal runs from {lo} to {hi}: {:?}", j.base_metal));
            }
        }
        if let Some(k) = j.lab_near.iter().position(|&l| !l) {
            fault(format!("no room for a lab by start {}", k + 1));
        }
        if let Some(k) = j.room.iter().position(|&s| s < 0.5) {
            fault(format!("a vehicle plant fits on {:.0}% of the ground round start {}", j.room[k] * 100.0, k + 1));
        }
        // The tides the menus show can be put to use.
        if j.tidal > 0.0 && j.tidal_sites.contains(&0) {
            fault(format!("tides of {}, but no tidal generator fits ({:?})", j.tidal, j.tidal_sites));
        }
        // The maps for fleets: a shipyard within the computer players' search (56 cells, in
        // rings) of every base, and on those whose seas are one, every ship from every yard to
        // every other (Twin Reefs' sand bar parts the deep-water ships' seas, as it is meant to).
        if r.shape.harbours > 0.0 && r.shape.symmetric {
            if let Some(k) = j.yard.iter().position(|y| y.is_none_or(|(_, ring)| ring > 56)) {
                fault(format!("start {}'s nearest shipyard site {:?}", k + 1, j.yard[k]));
            }
            if !r.shape.connect && !j.ships_missed.is_empty() {
                fault(format!("ships cannot sail between the yards: {:?}", j.ships_missed));
            }
        }
        // Every map with harbours: each base's submarines sail out of its yard into its sea.
        if r.shape.harbours > 0.0 && !j.subs_shut_in.is_empty() {
            fault(format!("submarines shut in their harbours (names, start): {:?}", j.subs_shut_in));
        }
    }
    assert!(wrong.is_empty(), "{} faults:\n{}", wrong.len(), wrong.join("\n"));
}

/// The report: every map (or `TA_MAPS`'s, TA's own too) as a player finds it.
#[test]
#[ignore]
fn maps_as_played() {
    let Some(g) = game() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let names: Vec<String> = match std::env::var("TA_MAPS") {
        Ok(v) => v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        Err(_) => super::IDS.iter().map(|id| super::recipe(id).unwrap().name.to_string()).collect(),
    };
    for name in names {
        let coasts = super::IDS.iter().filter_map(|id| super::recipe(id)).any(|r| r.name == name && r.shape.coasts.is_some());
        let Some(j) = judge(&g, &name, coasts) else {
            println!("{name}: no such map\n");
            continue;
        };
        for l in &j.lines {
            println!("{l}");
        }
        println!();
    }
}
