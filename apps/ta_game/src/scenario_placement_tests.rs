//! Every built-in scenario's starting forces and arriving waves held to the simulation's own
//! placement rules, on every difficulty: buildings where a constructor could build them and
//! squared to the grid as TA builds them, units where they can stand and move off, nothing
//! inside anything else, and the goals on ground the forces can reach.
//!
//! `scenario_layout_maps` (ignored) draws each one from above for a look, and lists what is
//! wrong: `TA_LAYOUT_DIR=<folder>` (`TA_SCENARIO=<id>`, a custom one of the settings folder's
//! too; `TA_SCENARIO_DIFFICULTY` for one difficulty; `TA_REGION_UNIT` for its region picture).

use super::tests::{battle, bare_battle, test_assets};
use super::*;
use ta_sim::world::{MoveSpec, Occupant, World};
use ta_sim::{Locomotion, Unit};

const DIFFICULTIES: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];
/// Farthest a wave's ground unit may come in from where the wave says it appears.
const WAVE_SPREAD: i64 = 900;

/// A unit's footprint as world pixels: (x0, z0, x1, z1).
fn rect(t: &ta_sim::UnitType, pos: FxVec3) -> (i64, i64, i64, i64) {
    let (x, z) = (pos.x.to_int(), pos.z.to_int());
    let (w, h) = (t.footprint.0 as i64 * 8, t.footprint.1 as i64 * 8);
    (x - w, z - h, x + w, z + h)
}

fn overlap(a: (i64, i64, i64, i64), b: (i64, i64, i64, i64)) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

fn anchor(spec: &MoveSpec, pos: FxVec3) -> (i32, i32) {
    World::pos_to_anchor(pos, spec.footprint)
}

fn ground_unit(t: &ta_sim::UnitType) -> bool {
    matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval)
}

/// Whether two units stand inside each other: a building's footprint is its square; a unit's,
/// the circle its rings keep it to (half its footprint's width round it). An aircraft in the
/// air is over everything.
fn inside_each_other(sim: &Sim, u: &Unit, v: &Unit) -> bool {
    let (t, s) = (sim.unit_type(u), sim.unit_type(v));
    let flying = |u: &Unit, t: &ta_sim::UnitType| t.locomotion == Locomotion::Air && u.pos.y > sim.world.height_at(u.pos.x, u.pos.z).max(sim.world.sea_level) + Fx::from_int(8);
    if flying(u, t) != flying(v, s) {
        return false;
    }
    if t.locomotion == Locomotion::Static || s.locomotion == Locomotion::Static {
        return overlap(rect(t, u.pos), rect(s, v.pos));
    }
    let reach = (t.footprint.0.max(t.footprint.1) + s.footprint.0.max(s.footprint.1)) as i64 * 8;
    u.pos.within_xz(v.pos, Fx::from_int(reach - 1))
}

/// What is wrong with where one unit stands (buildings: as a constructor would find the
/// ground in `bare`, the battle before anything was placed).
fn unit_issues(sim: &Sim, bare: &Sim, u: &Unit, name: &str, issues: &mut Vec<String>) {
    let t = sim.unit_type(u);
    let (w, h) = sim.world.size_px();
    let inside = u.pos.x > Fx::from_int(32) && u.pos.z > Fx::from_int(32) && u.pos.x < w - Fx::from_int(32) && u.pos.z < h - Fx::from_int(32);
    if !inside {
        issues.push(format!("{name}: at the map's edge"));
    }
    match t.locomotion {
        Locomotion::Static => {
            // The ground as a constructor would find it: no feature, slope, water or (for an
            // extractor) bare rock in the way.
            if let Some(e) = bare.placement_error(u.type_id, u.pos) {
                issues.push(format!("{name}: could not be built there ({e:?})"));
            }
            // TA builds every building the same way round (its yard to the south).
            if u.heading != 0 {
                issues.push(format!("{name}: turned {} degrees off the grid", u.heading as i64 * 360 / 65536));
            }
            if t.is_factory && !layout::factory_exit_clear(sim, t, u.pos) {
                issues.push(format!("{name}: its exit is blocked"));
            }
            if !layout::clear_of_features(sim, t, u.pos) {
                issues.push(format!("{name}: up against a tree, rock or wreck"));
            }
            if !layout::off_the_ore(sim, t, u.pos) {
                issues.push(format!("{name}: on a metal deposit an extractor could have"));
            }
            if t.extracts_metal > Fx::ZERO && sim.extraction_at(u.type_id, u.pos) <= Fx::ZERO {
                issues.push(format!("{name}: an extractor off the metal"));
            }
        }
        Locomotion::Air => {
            let ground = sim.world.height_at(u.pos.x, u.pos.z).max(sim.world.sea_level);
            if u.pos.y <= ground + Fx::from_int(8) && sim.world.air_landing_height(u.pos, &t.landing()).is_none() {
                // Parked where an aircraft of its kind may not set down (a building, a feature,
                // water, a hillside).
                let origin = World::footprint_origin(World::pos_to_anchor(u.pos, t.footprint), t.footprint);
                let blocked = (origin.1..origin.1 + t.footprint.1 as i32).any(|z| (origin.0..origin.0 + t.footprint.0 as i32).any(|x| sim.world.occupant(x, z) != Occupant::Free));
                issues.push(format!("{name}: parked where it cannot land{}", if blocked { " (on a building or feature)" } else { "" }));
            }
        }
        _ => {
            let spec = sim.move_spec(u.type_id);
            if !sim.world.passable(&spec, anchor(&spec, u.pos), None) {
                let why = if !sim.world.terrain_passable(&spec, anchor(&spec, u.pos)) { "ground it cannot stand on" } else { "a building or feature" };
                issues.push(format!("{name}: inside {why}"));
            }
            if t.locomotion == Locomotion::Ground && spec.min_depth <= 0 {
                let origin = World::footprint_origin(anchor(&spec, u.pos), spec.footprint);
                let wet = (origin.1..origin.1 + spec.footprint.1 as i32).any(|z| (origin.0..origin.0 + spec.footprint.0 as i32).any(|x| sim.world.cell_depth(x, z) > 0));
                if wet {
                    issues.push(format!("{name}: a land unit standing in water"));
                }
            }
        }
    }
}

/// Where a wave's units would come in, as `Sim::send_waves` brings them (on a copy of the
/// battle as it starts): each with the target it attack-moves to. (Of the waves that come the
/// same way, the biggest: a repeating wave's last.)
pub(super) fn wave_arrivals(sim: &Sim) -> Vec<(usize, ta_sim::mission::Wave, FxVec3, Vec<ta_sim::UnitId>, Sim)> {
    let waves = sim.mission.as_ref().map(|m| m.waves.clone()).unwrap_or_default();
    let mut ways: Vec<(usize, ta_sim::mission::Wave)> = Vec::new();
    for (k, wave) in waves.into_iter().enumerate() {
        let size = |w: &ta_sim::mission::Wave| w.units.iter().map(|&(_, n)| n as usize).sum::<usize>();
        let kinds = |w: &ta_sim::mission::Wave| w.units.iter().map(|&(t, _)| t).collect::<Vec<_>>();
        match ways.iter_mut().find(|(_, w)| w.owner == wave.owner && w.from == wave.from && w.target == wave.target && kinds(w) == kinds(&wave)) {
            Some(way) if size(&way.1) < size(&wave) => *way = (k, wave),
            Some(_) => {}
            None => ways.push((k, wave)),
        }
    }
    ways.into_iter()
        .map(|(k, wave)| {
            let mut copy = sim.clone();
            let nearest = copy
                .units()
                .filter(|u| !u.dead && u.carried_by.is_none() && copy.allied(u.owner, 0))
                .map(|u| u.pos)
                .min_by_key(|p| (*p - wave.from).xz().length_sq_raw());
            let target = wave.target.or_else(|| if wave.friendly { None } else { nearest }).unwrap_or(wave.from);
            let heading = ta_sim::fixed::heading_of(target - wave.from);
            let mut arrived = Vec::new();
            for &(type_id, count) in &wave.units {
                arrived.extend(copy.place_units_bound(type_id, wave.owner, wave.from, count, heading, Some(target)));
            }
            for &id in &arrived {
                copy.put_in_flight(id, None);
            }
            (k, wave, target, arrived, copy)
        })
        .collect()
}

/// What is wrong with a scenario's placements, one line each (none: all is well). `bare` is
/// the same battle before the scenario placed anything (the map's own features on it).
pub(super) fn placement_issues(sim: &Sim, bare: &Sim, scenario: &Scenario, ground: &Ground) -> Vec<String> {
    let mut issues = Vec::new();
    let starts = scenario.starts(ground).unwrap();
    let fx = |(x, z): (f32, f32)| FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z));
    let units: Vec<&Unit> = sim.units().filter(|u| !u.dead).collect();
    let name = |sim: &Sim, u: &Unit| format!("p{} {} at ({}, {})", u.owner, sim.unit_type(u).name, u.pos.x.to_int(), u.pos.z.to_int());
    for (i, u) in units.iter().enumerate() {
        unit_issues(sim, bare, u, &name(sim, u), &mut issues);
        // Nothing inside anything else (aircraft parked on the ground too).
        for v in &units[i + 1..] {
            if inside_each_other(sim, u, v) {
                issues.push(format!("{} overlaps {}", name(sim, u), name(sim, v)));
            }
        }
    }
    let name = |u: &Unit| name(sim, u);
    // A force's ground units of one kind can all get together (none stranded on a ledge of its
    // own, none shut in), and get to its start.
    for (owner, _) in scenario.forces().enumerate() {
        let mut regions: Vec<(MoveSpec, ta_sim::world::Region, String)> = Vec::new();
        for u in units.iter().filter(|u| u.owner as usize == owner && ground_unit(sim.unit_type(u))) {
            let spec = sim.move_spec(u.type_id);
            let region = sim.world.component(&spec, anchor(&spec, u.pos));
            if let Some((_, other, who)) = regions.iter().find(|(s, r, _)| *s == spec && *r != region) {
                issues.push(format!("{} cannot get to {who} (regions {region} and {other})", name(u)));
            } else if !regions.iter().any(|(s, _, _)| *s == spec) {
                regions.push((spec, region, name(u)));
            }
            let sizes = sim.world.region_sizes(&spec);
            if sizes.get(region as usize).is_some_and(|&n| n < 2_000) {
                issues.push(format!("{}: stranded on {} cells of its own", name(u), sizes[region as usize]));
            }
            // A patrol goes somewhere it can get to.
            for order in &u.orders {
                if let ta_sim::Order::Patrol(p) = order
                    && sim.world.component(&spec, anchor(&spec, *p)) != region
                {
                    issues.push(format!("{}: patrols to ({}, {}), where it cannot go", name(u), p.x.to_int(), p.z.to_int()));
                }
            }
        }
    }
    // Waves come in where they can, near where they are meant to, and can get where they go.
    for (k, wave, target, arrived, copy) in wave_arrivals(sim) {
        let wanted: usize = wave.units.iter().map(|&(_, n)| n as usize).sum();
        if arrived.len() < wanted {
            issues.push(format!("wave {k} (p{} at {}s): {} of {wanted} units came in", wave.owner, wave.tick / 30, arrived.len()));
        }
        let tag = |u: &Unit| format!("wave {k} (p{} at {}s): {} at ({}, {})", wave.owner, wave.tick / 30, copy.unit_type(u).name, u.pos.x.to_int(), u.pos.z.to_int());
        for &id in &arrived {
            let u = copy.unit(id).unwrap();
            unit_issues(&copy, bare, u, &tag(u), &mut issues);
            if let Some(v) = copy.units().find(|v| !v.dead && v.id != id && inside_each_other(&copy, u, v)) {
                issues.push(format!("{} came in inside {}", tag(u), name(v)));
            }
        }
        for &id in &arrived {
            let u = copy.unit(id).unwrap();
            let t = copy.unit_type(u);
            if !ground_unit(t) {
                continue;
            }
            let spec = copy.move_spec(u.type_id);
            let far = (u.pos - wave.from).xz().length().to_int();
            if far > WAVE_SPREAD {
                issues.push(format!("wave {k} (p{} at {}s): {} came in {far} px from ({}, {})", wave.owner, wave.tick / 30, t.name, wave.from.x.to_int(), wave.from.z.to_int()));
            }
            let goal = copy.world.component(&spec, anchor(&spec, target));
            let here = copy.world.component(&spec, anchor(&spec, u.pos));
            if goal != 0 && here != goal {
                issues.push(format!("wave {k} (p{} at {}s): {} cannot get to its target ({}, {})", wave.owner, wave.tick / 30, t.name, target.x.to_int(), target.z.to_int()));
            }
        }
    }
    // Areas to reach or hold are on ground the player's army (its Commander, for a Commander's
    // goal) can get to.
    for goal in scenario.victory.iter().chain(&scenario.defeat) {
        let (words, radius, who) = match goal {
            GoalSpec::Hold { hold, radius, .. } => (hold, *radius, None),
            GoalSpec::EnemyHolds { enemy_holds, radius, .. } => (enemy_holds, *radius, None),
            GoalSpec::Reach { reach, radius, unit } => (reach, *radius, unit.as_deref()),
            _ => continue,
        };
        let at = fx(ground.place(words).unwrap());
        let candidates: Vec<&Unit> = units
            .iter()
            .copied()
            .filter(|u| u.owner == 0 && ground_unit(sim.unit_type(u)))
            .filter(|u| match who {
                Some(w) if w.eq_ignore_ascii_case("commander") => sim.unit_type(u).def.is_commander,
                Some(w) if !w.eq_ignore_ascii_case("any") => sim.unit_type(u).name.eq_ignore_ascii_case(w),
                _ => true,
            })
            .collect();
        let reaches = |u: &Unit| {
            let spec = sim.move_spec(u.type_id);
            let region = sim.world.component(&spec, anchor(&spec, u.pos));
            let r = (radius / 16.0) as i32;
            let c = World::pos_to_anchor(at, spec.footprint);
            region != 0 && sim.world.nearest_in_component(&spec, region, c, r).is_some_and(|p| {
                let (dx, dz) = (p.0 - c.0, p.1 - c.1);
                dx * dx + dz * dz <= r * r
            })
        };
        if !candidates.is_empty() && !candidates.iter().any(|&u| reaches(u)) {
            issues.push(format!("goal at {words}: nothing of the player's can get there"));
        }
    }
    // The opening view is on the player's own forces.
    if let Some(&start) = starts.first() {
        let ours: Vec<&Unit> = units.iter().copied().filter(|u| u.owner == 0).collect();
        let near = ours.iter().filter(|u| u.pos.within_xz(fx(start), Fx::from_int(900))).count();
        if !ours.is_empty() && near * 2 < ours.len() {
            issues.push(format!("the opening view ({:.0}, {:.0}) shows {near} of the player's {} units", start.0, start.1, ours.len()));
        }
    }
    issues
}

/// Every built-in scenario, on every difficulty, places what it places by the simulation's
/// own rules (see the module's comment).
#[test]
fn every_scenario_places_its_forces_by_the_simulations_rules() {
    let Some(assets) = test_assets() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let entries: Vec<Entry> = catalogue(&assets).into_iter().filter(|e| !e.custom).collect();
    let mut all = Vec::new();
    for entry in &entries {
        for difficulty in DIFFICULTIES {
            let (bare, ground) = bare_battle(&assets, entry, difficulty);
            let (sim, _, _) = battle(&assets, entry, difficulty);
            for issue in placement_issues(&sim, &bare, &entry.scenario, &ground) {
                all.push(format!("{} ({difficulty:?}): {issue}", entry.scenario.id));
            }
        }
    }
    assert!(all.is_empty(), "{} placement problems:\n{}", all.len(), all.join("\n"));
}

// ---- Pictures --------------------------------------------------------------------------------

type Picture = image::RgbaImage;

fn blend(img: &mut Picture, x: i64, y: i64, c: [u8; 4]) {
    if x < 0 || y < 0 || x >= img.width() as i64 || y >= img.height() as i64 {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    let a = c[3] as u32;
    for k in 0..3 {
        p.0[k] = ((p.0[k] as u32 * (255 - a) + c[k] as u32 * a) / 255) as u8;
    }
    p.0[3] = 255;
}

struct View {
    /// World pixels at the picture's top-left, and world pixels per picture pixel.
    x0: f32,
    z0: f32,
    scale: f32,
}

impl View {
    fn at(&self, x: f32, z: f32) -> (i64, i64) {
        (((x - self.x0) / self.scale).round() as i64, ((z - self.z0) / self.scale).round() as i64)
    }

    fn rect(&self, img: &mut Picture, (x0, z0, x1, z1): (f32, f32, f32, f32), fill: Option<[u8; 4]>, line: [u8; 4]) {
        let (a, b) = (self.at(x0, z0), self.at(x1, z1));
        for y in a.1..=b.1 {
            for x in a.0..=b.0 {
                let edge = x == a.0 || x == b.0 || y == a.1 || y == b.1;
                if edge {
                    blend(img, x, y, line);
                } else if let Some(f) = fill {
                    blend(img, x, y, f);
                }
            }
        }
    }

    fn circle(&self, img: &mut Picture, (x, z): (f32, f32), r: f32, fill: Option<[u8; 4]>, line: [u8; 4]) {
        let c = self.at(x, z);
        let r = (r / self.scale).max(1.5);
        let n = r.ceil() as i64 + 1;
        for dy in -n..=n {
            for dx in -n..=n {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if (d - r).abs() < 0.7 {
                    blend(img, c.0 + dx, c.1 + dy, line);
                } else if d < r && let Some(f) = fill {
                    blend(img, c.0 + dx, c.1 + dy, f);
                }
            }
        }
    }

    fn line(&self, img: &mut Picture, a: (f32, f32), b: (f32, f32), c: [u8; 4]) {
        let (p, q) = (self.at(a.0, a.1), self.at(b.0, b.1));
        let n = (q.0 - p.0).abs().max((q.1 - p.1).abs()).max(1);
        for i in 0..=n {
            blend(img, p.0 + (q.0 - p.0) * i / n, p.1 + (q.1 - p.1) * i / n, c);
        }
    }

    fn cross(&self, img: &mut Picture, (x, z): (f32, f32), size: i64, c: [u8; 4]) {
        let p = self.at(x, z);
        for d in -size..=size {
            blend(img, p.0 + d, p.1 + d, c);
            blend(img, p.0 + d, p.1 - d, c);
        }
    }

    /// Digits in a 3x5 font, `k` pixels a dot.
    fn number(&self, img: &mut Picture, (x, z): (f32, f32), n: usize, k: i64, c: [u8; 4]) {
        const DIGITS: [u16; 10] = [0x7b6f, 0x2c97, 0x73e7, 0x73cf, 0x5bc9, 0x79cf, 0x79ef, 0x7249, 0x7bef, 0x7bcf];
        let p = self.at(x, z);
        for (i, ch) in n.to_string().bytes().enumerate() {
            let bits = DIGITS[(ch - b'0') as usize];
            for row in 0..5 {
                for col in 0..3 {
                    if bits >> (14 - (row * 3 + col)) & 1 == 1 {
                        for yy in 0..k {
                            for xx in 0..k {
                                blend(img, p.0 + (i as i64 * 4 + col) * k + xx, p.1 + row * k + yy, c);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn owner_colour(scenario: &Scenario, owner: u8) -> [u8; 4] {
    if owner == 0 {
        [40, 110, 255, 255]
    } else if (owner as usize) <= scenario.enemies.len() {
        [[230, 40, 30, 255], [255, 140, 0, 255], [200, 0, 160, 255]][(owner as usize - 1) % 3]
    } else {
        [0, 220, 220, 255]
    }
}

/// The ground under a view: heights in grey, water blue, cliffs a building could not stand on
/// tinted red, blocking features green, metal yellow.
fn terrain(sim: &Sim, view: &View, size: (u32, u32)) -> Picture {
    let mut img = Picture::new(size.0, size.1);
    let world = &sim.world;
    for y in 0..size.1 {
        for x in 0..size.0 {
            let (wx, wz) = (view.x0 + x as f32 * view.scale, view.z0 + y as f32 * view.scale);
            let (cx, cz) = ((wx / 16.0).floor() as i32, (wz / 16.0).floor() as i32);
            let px = if !world.in_bounds(cx, cz) || world.is_void(cx, cz) {
                [0, 0, 0, 255]
            } else {
                let height = world.vertex_height(cx, cz).to_int().clamp(0, 255) as u32;
                let g = (50 + height * 150 / 255) as u8;
                let mut c = [g, g, g, 255];
                let depth = world.cell_depth(cx, cz);
                if depth > 0 {
                    let d = depth.min(60) as u32;
                    c = [20, (60 + 80 - d) as u8, (150 + 100 - d) as u8, 255];
                } else {
                    let slope = world.slope(cx, cz) as u32;
                    if slope > 10 {
                        let a = (slope.min(40) * 5) as u8;
                        c = [c[0].saturating_add(a / 2), c[1].saturating_sub(a / 2), c[2].saturating_sub(a / 2), 255];
                    }
                }
                let i = cz as usize * world.width as usize + cx as usize;
                if world.metal[i] > world.surface_metal {
                    c = [230, 200, 30, 255];
                }
                if let Occupant::Feature(_) = world.occupant(cx, cz) {
                    c = [c[0] / 3, (c[1] / 3).saturating_add(90), c[2] / 3, 255];
                }
                c
            };
            img.put_pixel(x, y, image::Rgba(px));
        }
    }
    img
}

/// A scenario drawn from above: the whole map, and close up round each force's start.
#[test]
#[ignore]
fn scenario_layout_maps() {
    let Some(assets) = test_assets() else {
        eprintln!("TA install not found; skipping");
        return;
    };
    let dir = std::path::PathBuf::from(std::env::var("TA_LAYOUT_DIR").unwrap_or_else(|_| "scenario_layouts".into()));
    std::fs::create_dir_all(&dir).unwrap();
    let wanted = std::env::var("TA_SCENARIO").ok();
    let difficulties: Vec<Difficulty> = match std::env::var("TA_SCENARIO_DIFFICULTY") {
        Ok(_) => vec![difficulty_from_env()],
        Err(_) => DIFFICULTIES.to_vec(),
    };
    let mut report = String::new();
    // (A custom scenario of the settings folder's too, when named: a layout tried out there.)
    for entry in catalogue(&assets).iter().filter(|e| (!e.custom || wanted.is_some()) && wanted.as_deref().is_none_or(|w| e.scenario.id == w)) {
        let s = &entry.scenario;
        for &difficulty in &difficulties {
            let (bare, ground) = bare_battle(&assets, entry, difficulty);
            let (sim, run, _) = battle(&assets, entry, difficulty);
            let issues = placement_issues(&sim, &bare, s, &ground);
            report.push_str(&format!("== {} ({difficulty:?}) on {}: {} units, {} issues\n", s.id, s.map, sim.units().count(), issues.len()));
            for (n, i) in issues.iter().enumerate() {
                report.push_str(&format!("  [{n}] {i}\n"));
            }
            let starts = s.starts(&ground).unwrap();
            for (owner, f) in s.forces().enumerate() {
                report.push_str(&format!("  p{owner} {} start {} = ({:.0}, {:.0})\n", f.side, f.start, starts[owner].0, starts[owner].1));
                let mut kinds: std::collections::BTreeMap<String, Vec<String>> = Default::default();
                for u in sim.units().filter(|u| u.owner as usize == owner) {
                    kinds.entry(sim.unit_type(u).name.clone()).or_default().push(format!("({},{})", u.pos.x.to_int(), u.pos.z.to_int()));
                }
                for (k, v) in kinds {
                    // (A building's reach: its longest weapon's range.)
                    let t = sim.types.get(sim.types.id(&k).unwrap());
                    let range = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w).range.to_int()).max();
                    let reach = range.filter(|_| t.locomotion == Locomotion::Static).map_or(String::new(), |r| format!(" (range {r})"));
                    report.push_str(&format!("     {k} x{}{reach}: {}\n", v.len(), v.join(" ")));
                }
            }
            let arrivals = wave_arrivals(&sim);
            for (k, wave, target, arrived, copy) in &arrivals {
                let far = arrived.iter().filter_map(|&id| copy.unit(id)).map(|u| (u.pos - wave.from).xz().length().to_int()).max().unwrap_or(0);
                report.push_str(&format!("  wave {k}: p{} at {}s from ({}, {}) to ({}, {}): {} units, farthest {far} px in\n", wave.owner, wave.tick / 30, wave.from.x.to_int(), wave.from.z.to_int(), target.x.to_int(), target.z.to_int(), arrived.len()));
            }
            if difficulty != Difficulty::Normal && difficulties.len() > 1 {
                continue;
            }
            // The whole map.
            let (w, h) = (sim.world.width, sim.world.height);
            let scale = (w.max(h) as f32 * 16.0 / 1400.0).max(4.0);
            let view = View { x0: 0.0, z0: 0.0, scale };
            let size = ((w as f32 * 16.0 / scale) as u32, (h as f32 * 16.0 / scale) as u32);
            let mut img = terrain(&sim, &view, size);
            draw_units(&mut img, &view, &sim, s, &issues);
            for (_, wave, target, arrived, copy) in &arrivals {
                let c = owner_colour(s, wave.owner);
                view.cross(&mut img, (wave.from.x.to_f32(), wave.from.z.to_f32()), 6, c);
                view.line(&mut img, (wave.from.x.to_f32(), wave.from.z.to_f32()), (target.x.to_f32(), target.z.to_f32()), [c[0], c[1], c[2], 90]);
                for &id in arrived {
                    let u = copy.unit(id).unwrap();
                    view.circle(&mut img, (u.pos.x.to_f32(), u.pos.z.to_f32()), 12.0, None, [255, 255, 255, 200]);
                }
            }
            for &(x, z, r) in &run.areas {
                view.circle(&mut img, (x, z), r, None, [255, 255, 255, 255]);
            }
            for (k, &(x, z)) in starts.iter().enumerate() {
                view.cross(&mut img, (x, z), 8, [255, 255, 0, 255]);
                view.number(&mut img, (x + 40.0 * scale / 4.0, z), k + 1, 2, [255, 255, 0, 255]);
            }
            img.save(dir.join(format!("{}_map.png", s.id))).unwrap();
            // Where a tank can go (`TA_REGION_UNIT`, ARMSTUMP by default), a cell a pixel: the
            // ground the player's start is on green, other ground in colours of its own, what it
            // cannot stand on black; brighter higher.
            let probe = std::env::var("TA_REGION_UNIT").unwrap_or_else(|_| "ARMSTUMP".into());
            if let Some(type_id) = sim.types.id(&probe) {
                let spec = sim.move_spec(type_id);
                let home = sim.world.component(&spec, World::pos_to_anchor(FxVec3::new(Fx::from_f32(starts[0].0), Fx::ZERO, Fx::from_f32(starts[0].1)), spec.footprint));
                let mut img = Picture::new(w, h);
                for z in 0..h as i32 {
                    for x in 0..w as i32 {
                        let region = sim.world.component(&spec, (x, z));
                        let light = 80 + sim.world.vertex_height(x, z).to_int().clamp(0, 255) as u32 * 175 / 255;
                        let base: [u32; 3] = if region == 0 {
                            [0, 0, 0]
                        } else if region == home {
                            [60, 255, 60]
                        } else {
                            let hash = region.wrapping_mul(2654435761);
                            [100 + (hash >> 8 & 0x7f), 60 + (hash >> 16 & 0x7f), 100 + (hash >> 24 & 0x7f)]
                        };
                        let c = base.map(|v| (v * light / 255).min(255) as u8);
                        img.put_pixel(x as u32, z as u32, image::Rgba([c[0], c[1], c[2], 255]));
                    }
                }
                let view = View { x0: 0.0, z0: 0.0, scale: 16.0 };
                for u in sim.units().filter(|u| sim.unit_type(u).locomotion == Locomotion::Static) {
                    view.cross(&mut img, (u.pos.x.to_f32(), u.pos.z.to_f32()), 2, owner_colour(s, u.owner));
                }
                for &(x, z, r) in &run.areas {
                    view.circle(&mut img, (x, z), r, None, [255, 255, 255, 255]);
                }
                img.save(dir.join(format!("{}_regions.png", s.id))).unwrap();
            }
            // Close up round each force's start, and round each wave's way in.
            let mut spots: Vec<(String, (f32, f32))> = starts.iter().enumerate().map(|(k, &p)| (format!("p{k}"), p)).collect();
            for (k, wave, _, _, _) in &arrivals {
                let p = (wave.from.x.to_f32(), wave.from.z.to_f32());
                if !spots.iter().any(|(_, q)| (q.0 - p.0).hypot(q.1 - p.1) < 600.0) {
                    spots.push((format!("wave{k}"), p));
                }
            }
            for (label, (x, z)) in spots {
                let view = View { x0: x - 1200.0, z0: z - 1200.0, scale: 3.0 };
                let mut img = terrain(&sim, &view, (800, 800));
                for (_, wave, _, arrived, copy) in &arrivals {
                    for &id in arrived {
                        let u = copy.unit(id).unwrap();
                        let t = copy.unit_type(u);
                        let r = t.footprint.0.max(t.footprint.1) as f32 * 8.0;
                        view.circle(&mut img, (u.pos.x.to_f32(), u.pos.z.to_f32()), r, None, [255, 255, 255, 220]);
                    }
                    view.cross(&mut img, (wave.from.x.to_f32(), wave.from.z.to_f32()), 8, owner_colour(s, wave.owner));
                }
                draw_units(&mut img, &view, &sim, s, &issues);
                for &(ax, az, r) in &run.areas {
                    view.circle(&mut img, (ax, az), r, None, [255, 255, 255, 255]);
                }
                img.save(dir.join(format!("{}_{label}.png", s.id))).unwrap();
            }
        }
    }
    std::fs::write(dir.join("report.txt"), &report).unwrap();
    println!("{report}");
}

/// Footprints filled in their owner's colour (factories' aprons outlined, aircraft hollow),
/// the units with a problem ringed in magenta and numbered as the report numbers them.
fn draw_units(img: &mut Picture, view: &View, sim: &Sim, s: &Scenario, issues: &[String]) {
    for u in sim.units().filter(|u| !u.dead) {
        let t = sim.unit_type(u);
        let c = owner_colour(s, u.owner);
        let (x, z) = (u.pos.x.to_f32(), u.pos.z.to_f32());
        let (hw, hh) = (t.footprint.0 as f32 * 8.0, t.footprint.1 as f32 * 8.0);
        match t.locomotion {
            Locomotion::Static => {
                view.rect(img, (x - hw, z - hh, x + hw, z + hh), Some([c[0], c[1], c[2], 150]), [255, 255, 255, 255]);
                if t.is_factory {
                    view.rect(img, (x - hw, z + hh, x + hw, z + hh + 144.0), None, [255, 230, 0, 200]);
                }
                if t.extracts_metal > Fx::ZERO {
                    view.circle(img, (x, z), hw * 0.5, Some([255, 230, 0, 255]), [0, 0, 0, 255]);
                }
                // Which way it faces.
                let a = u.heading as f32 / 65536.0 * std::f32::consts::TAU;
                view.line(img, (x, z), (x + a.sin() * hw * 1.4, z + a.cos() * hh * 1.4), [255, 255, 255, 255]);
            }
            Locomotion::Air => view.rect(img, (x - hw, z - hh, x + hw, z + hh), None, c),
            _ => view.circle(img, (x, z), hw.max(hh), Some([c[0], c[1], c[2], 200]), [0, 0, 0, 255]),
        }
        let tag = format!("({}, {})", u.pos.x.to_int(), u.pos.z.to_int());
        for (n, issue) in issues.iter().enumerate() {
            if issue.contains(&tag) && issue.contains(&t.name) {
                view.circle(img, (x, z), hw.max(hh) + 24.0, None, [255, 0, 255, 255]);
                view.number(img, (x + hw + 16.0, z - hh), n, 2, [255, 0, 255, 255]);
            }
        }
    }
}
