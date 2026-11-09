//! Fixtures for the officer's tests on real game data (skipped without a TA install): the
//! game's data once, a map by name, and a scene of two players on it, ours with a
//! [`DutyOfficer`].
//!
//! `TA_TEST_MAP` picks the map `Scene::new` plays on (default Sherwood), `TA_TEST_LOG=1` prints
//! the officer's debug log (what each constructor was sent to do).

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use ta_ai::{Duty, DutyOfficer};
use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim, Target, UnitId, UnitTypes};

/// Prints the officer's debug log.
struct PrintLog;

impl log::Log for PrintLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target().starts_with("ta_ai")
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            println!("[{}] {}", r.level(), r.args());
        }
    }
    fn flush(&self) {}
}

/// The game's data, once for every test.
pub struct Data {
    pub vfs: ta_vfs::Vfs,
    pub data: Arc<GameData>,
    pub types: Arc<UnitTypes>,
}

pub fn data() -> Option<&'static Data> {
    static DATA: OnceLock<Option<Data>> = OnceLock::new();
    DATA.get_or_init(|| {
        if std::env::var("TA_TEST_LOG").is_ok() && log::set_logger(&PrintLog).is_ok() {
            log::set_max_level(log::LevelFilter::Debug);
        }
        let Ok(install) = ta_vfs::install::discover(None) else {
            eprintln!("TA install not found; skipping");
            return None;
        };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let data = Arc::new(GameData::load(&vfs));
        let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
        Some(Data { vfs, data, types })
    })
    .as_ref()
}

/// A map and what its OTA says of it.
pub struct MapData {
    pub map: TntMap,
    pub info: MapInfo,
}

/// A map of the install by name (kept: parsing one takes a moment).
pub fn map(name: &str) -> Option<Arc<MapData>> {
    static MAPS: OnceLock<Mutex<HashMap<String, Arc<MapData>>>> = OnceLock::new();
    let d = data()?;
    let mut maps = MAPS.get_or_init(Default::default).lock().unwrap();
    if let Some(m) = maps.get(name) {
        return Some(m.clone());
    }
    let tnt = d.vfs.read_opt(&format!("maps/{name}.tnt"))?;
    let ota = d.vfs.read_opt(&format!("maps/{name}.ota"))?;
    let m = Arc::new(MapData { map: TntMap::parse(&tnt).ok()?, info: MapInfo::from_ota(&tdf::parse_bytes(&ota)) });
    maps.insert(name.to_string(), m.clone());
    Some(m)
}

/// The map `Scene::new` plays on.
pub fn default_map() -> String {
    std::env::var("TA_TEST_MAP").unwrap_or_else(|_| "Sherwood".into())
}

/// Two players on a map, ours (player 0, the officer's) and an idle enemy (player 1) at another
/// start, each with a Commander at its start.
pub struct Scene {
    pub sim: Sim,
    pub officer: DutyOfficer,
    /// Our start, and the enemy's.
    pub start: FxVec3,
    pub foe: FxVec3,
    pub map: String,
}

impl Scene {
    /// ARM against CORE on the default map, the ground known from the start ("mapped").
    pub fn new() -> Option<Scene> {
        Scene::with(true)
    }

    /// With the rule "mapped" (the ground known from the start) or "unmapped" (dark until explored).
    pub fn with(mapped: bool) -> Option<Scene> {
        Scene::on(&default_map(), ["ARM", "CORE"], mapped)
    }

    /// On `map`, the sides `[ours, theirs]`; each starts with a thousand of both stores.
    pub fn on(map_name: &str, sides: [&str; 2], mapped: bool) -> Option<Scene> {
        Scene::on_with(map_name, sides, mapped, (0, 1))
    }

    /// As [`Scene::on`], at the map's start spots number `spots` (our, theirs).
    pub fn on_with(map_name: &str, sides: [&str; 2], mapped: bool, spots: (usize, usize)) -> Option<Scene> {
        let d = data()?;
        let m = map(map_name)?;
        let starts = &m.info.schemas[0].start_positions;
        let spot = |k: usize| FxVec3::new(Fx::from_f32(starts[k].1), Fx::ZERO, Fx::from_f32(starts[k].2));
        let at = [spot(spots.0), spot(spots.1)];
        let players: Vec<PlayerSetup> = (0..2)
            .map(|i| PlayerSetup { side: sides[i].into(), color: i as u8, start: at[i], metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false })
            .collect();
        let mut sim = Sim::new(d.types.clone(), d.data.clone(), &m.map, &m.info, 0, &players, 7);
        sim.cheats_allowed = true;
        for (i, side) in sides.iter().enumerate() {
            let name = if side.eq_ignore_ascii_case("ARM") { "ARMCOM" } else { "CORCOM" };
            sim.spawn_unit(d.types.id(name).expect("the commander"), i as u8, players[i].start, 0, true);
        }
        // TA's default rule "mapped": the ground is known from the start.
        if mapped {
            sim.reveal_map_for(0);
        }
        let officer = DutyOfficer::new(0, &sim);
        Some(Scene { sim, officer, start: at[0], foe: at[1], map: map_name.to_string() })
    }

    pub fn type_id(&self, name: &str) -> usize {
        data().unwrap().types.id(name).unwrap_or_else(|| panic!("no unit type {name}"))
    }

    /// Our Commander.
    pub fn commander(&self) -> UnitId {
        self.sim.units().find(|u| u.owner == 0 && self.sim.unit_type(u).def.is_commander).expect("our Commander").id
    }

    /// A finished unit of ours (player 0) `dx`, `dz` from the start.
    pub fn spawn(&mut self, name: &str, dx: i64, dz: i64) -> UnitId {
        let at = self.start + FxVec3::from_ints(dx, 0, dz);
        self.spawn_at(name, 0, at)
    }

    pub fn spawn_at(&mut self, name: &str, owner: u8, at: FxVec3) -> UnitId {
        let t = self.type_id(name);
        self.sim.spawn_unit(t, owner, at, 0, true).unwrap_or_else(|| panic!("no room for {name}"))
    }

    /// An unfinished building (a nanoframe) of ours.
    pub fn spawn_frame(&mut self, name: &str, at: FxVec3) -> UnitId {
        let t = self.type_id(name);
        self.sim.spawn_unit(t, 0, at, 0, false).unwrap_or_else(|| panic!("no room for {name}"))
    }

    /// Plays `seconds` seconds, the officer thinking each tick; everything it ordered, with the
    /// tick of each order.
    pub fn play(&mut self, seconds: u64) -> Vec<(u64, Command)> {
        self.play_with(seconds, &[])
    }

    /// As [`Scene::play`], with `extra` commands (from the player or the enemy) in the first tick.
    pub fn play_with(&mut self, seconds: u64, extra: &[Command]) -> Vec<(u64, Command)> {
        self.play_watched(seconds, extra, |_, _| {})
    }

    /// As [`Scene::play_with`], `watch` seeing the game before each tick (and what the officer ordered
    /// for it), once a tick.
    pub fn play_watched(&mut self, seconds: u64, extra: &[Command], mut watch: impl FnMut(&Sim, &[Command])) -> Vec<(u64, Command)> {
        let mut ordered = Vec::new();
        for k in 0..30 * seconds {
            let mut commands = self.officer.think(&self.sim);
            ordered.extend(commands.iter().map(|c| (self.sim.tick, c.clone())));
            if k == 0 {
                commands.extend_from_slice(extra);
            }
            watch(&self.sim, &commands);
            self.sim.step(&commands);
            self.sim.take_events();
        }
        ordered
    }

    pub fn extractors(&self) -> usize {
        self.sim.units().filter(|u| u.owner == 0 && !u.dead && self.sim.unit_type(u).extracts_metal > Fx::ZERO).count()
    }

    pub fn generators(&self) -> usize {
        self.sim
            .units()
            .filter(|u| {
                let t = self.sim.unit_type(u);
                u.owner == 0 && !u.dead && (t.energy_make > t.energy_use && t.extracts_metal <= Fx::ZERO && t.makes_metal <= Fx::ZERO || t.wind_generator > Fx::ZERO || t.tidal_generator > Fx::ZERO)
            })
            .count()
    }

    /// Our finished and unfinished units of this type.
    pub fn count(&self, name: &str) -> usize {
        let t = self.type_id(name);
        self.sim.units().filter(|u| u.owner == 0 && !u.dead && u.type_id == t).count()
    }

    /// What of ours stands, by type name, finished or not (buildings and units).
    pub fn census(&self) -> std::collections::BTreeMap<String, usize> {
        let mut out = std::collections::BTreeMap::new();
        for u in self.sim.units().filter(|u| u.owner == 0 && !u.dead) {
            *out.entry(self.sim.unit_type(u).name.clone()).or_insert(0) += 1;
        }
        out
    }

    pub fn metal(&self) -> i64 {
        self.sim.players[0].economy.metal.to_int()
    }

    pub fn energy(&self) -> i64 {
        self.sim.players[0].economy.energy.to_int()
    }

    /// A command from the enemy.
    pub fn attack(&self, units: &[UnitId], target: UnitId) -> Command {
        Command::Order { player: 1, units: units.to_vec(), order: Order::Attack(Target::Unit(target)), queue: false }
    }
}

/// The distance in px between two points on the ground.
pub fn px(a: FxVec3, b: FxVec3) -> i64 {
    (a - b).length_xz().to_int()
}

/// The duty by its English name.
pub fn duty_named(name: &str) -> Duty {
    Duty::ALL.into_iter().find(|d| d.name() == name).unwrap_or_else(|| panic!("no duty {name}"))
}

// ---- The audit: what a player must be able to trust ------------------------------------------------

/// What a command asks, in a few words.
pub fn words(sim: &ta_sim::Sim, c: &Command) -> String {
    match c {
        Command::Order { units, order, queue, .. } => {
            let q = if *queue { "+" } else { "" };
            let what = match order {
                Order::Build { type_id, pos } => format!("build {} at ({}, {})", sim.types.get(*type_id).name, pos.x.to_int(), pos.z.to_int()),
                Order::Move(p) => format!("move ({}, {})", p.x.to_int(), p.z.to_int()),
                Order::Reclaim(f) => format!("reclaim feature {f}"),
                other => format!("{other:?}"),
            };
            format!("{q}{what} <- {:?}", units.iter().map(|u| u.slot).collect::<Vec<_>>())
        }
        other => format!("{other:?}"),
    }
}

use std::collections::BTreeMap;
use ta_sim::{Locomotion, Unit};

/// What a stretch of play showed of the constructors on duty.
#[derive(Default, Debug)]
pub struct Audit {
    /// Every order the officer gave, with its tick.
    pub ordered: Vec<(u64, Command)>,
    /// The nearest a unit on duty came to the enemy's start (px).
    pub nearest_foe: Option<i64>,
    /// Where a unit on duty stood within reach of the guns of something armed of the enemy's.
    pub under_guns: Vec<String>,
    /// Two builders sent at one job.
    pub conflicts: Vec<String>,
    /// Orders repeated for a unit at once, or too many.
    pub spam: Vec<String>,
    /// Seconds the stores stood empty.
    pub metal_dry: u64,
    pub energy_dry: u64,
    /// Seconds in which no unit on duty had an order.
    pub idle: u64,
    pub seconds: u64,
}

/// How far an armed unit's longest gun reaches.
pub fn armed_reach(sim: &Sim, u: &Unit) -> i64 {
    sim.unit_type(u).weapons.iter().flatten().map(|&w| sim.weapon_types.get(w).range.to_int()).max().unwrap_or(0)
}

/// A job a builder is at, as a conflict is told by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Job {
    Build(FxVec3),
    Reclaim(usize),
    Repair(UnitId),
}

/// The job at the front of a builder's orders, if one that two should not share.
pub fn front_job(sim: &Sim, b: &Unit) -> Option<Job> {
    match b.orders.front()? {
        Order::Build { pos, .. } => Some(Job::Build(*pos)),
        Order::Reclaim(f) => Some(Job::Reclaim(*f)),
        // (Two builders on a nanoframe are a team; on a unit that is only hurt, a waste.)
        Order::Repair(id) => sim.unit(*id).filter(|t| t.is_complete()).map(|_| Job::Repair(*id)),
        _ => None,
    }
}

pub fn same_job(a: Job, b: Job) -> bool {
    match (a, b) {
        (Job::Build(p), Job::Build(q)) => px(p, q) < 24,
        (a, b) => a == b,
    }
}

/// Plays `seconds`, the officer thinking, and audits the units on duty at the start.
pub fn audit(scene: &mut Scene, seconds: u64, extra: &[Command]) -> Audit {
    let duty: Vec<UnitId> = scene.officer.units().map(|(id, _)| id).collect();
    let foe = scene.foe;
    let mut a = Audit { seconds, ..Default::default() };
    let mut idle = 0u64;
    a.ordered = scene.play_watched(seconds, extra, |sim, _| {
        if sim.tick % 30 != 0 {
            return;
        }
        let eco = &sim.players[0].economy;
        a.metal_dry += (eco.metal < Fx::ONE) as u64;
        a.energy_dry += (eco.energy < Fx::ONE) as u64;
        let mine: Vec<&Unit> = duty.iter().filter_map(|&id| sim.unit(id)).filter(|u| !u.dead && u.carried_by.is_none()).collect();
        idle += (!mine.is_empty() && mine.iter().all(|u| u.orders.is_empty() && u.build.is_none())) as u64;
        let armed: Vec<(&Unit, i64)> = sim.units().filter(|e| e.owner == 1 && !e.dead && e.is_complete()).map(|e| (e, armed_reach(sim, e))).filter(|&(_, r)| r > 0).collect();
        for u in &mine {
            let d = px(u.pos, foe);
            a.nearest_foe = Some(a.nearest_foe.map_or(d, |n| n.min(d)));
            for &(e, r) in &armed {
                if px(u.pos, e.pos) <= r + 16 {
                    a.under_guns.push(format!("t={} {} at ({}, {}) in reach of {} at ({}, {})", sim.tick, sim.unit_type(u).name, u.pos.x.to_int(), u.pos.z.to_int(), sim.unit_type(e).name, e.pos.x.to_int(), e.pos.z.to_int()));
                    break;
                }
            }
        }
        // Two builders at one job (the player's own among them).
        let builders: Vec<(&Unit, Job)> = sim.units().filter(|u| u.owner == 0 && !u.dead && sim.unit_type(u).is_builder && sim.unit_type(u).locomotion != Locomotion::Static).filter_map(|u| front_job(sim, u).map(|j| (u, j))).collect();
        for (i, &(x, jx)) in builders.iter().enumerate() {
            for &(y, jy) in &builders[i + 1..] {
                if same_job(jx, jy) && (duty.contains(&x.id) || duty.contains(&y.id)) {
                    a.conflicts.push(format!("t={} {:?} and {:?} both at {:?}", sim.tick, x.id.slot, y.id.slot, jx));
                }
            }
        }
    });
    a.idle = idle;
    // Orders repeated at once, and the rate of orders.
    let mut last: BTreeMap<UnitId, (u64, Order)> = BTreeMap::new();
    let mut count: BTreeMap<UnitId, usize> = BTreeMap::new();
    for (tick, c) in &a.ordered {
        if let Command::Order { units, order, queue: false, .. } = c {
            for &u in units {
                *count.entry(u).or_default() += 1;
                if let Some((t, o)) = last.get(&u)
                    && o == order
                    && tick - t < 10 * 30
                    && !matches!(order, Order::Stop)
                {
                    let target = match order {
                        Order::Repair(id) => scene.sim.unit(*id).map(|t| format!(" ({} at {}%, complete {})", scene.sim.unit_type(t).name, (t.health * 100 / scene.sim.unit_type(t).max_health).to_int(), t.is_complete())).unwrap_or_default(),
                        _ => String::new(),
                    };
                    a.spam.push(format!("t={tick} unit {} given {:?}{target} again after {} ticks", u.slot, order, tick - t));
                }
                last.insert(u, (*tick, order.clone()));
            }
        }
    }
    for (u, n) in count {
        if n as u64 * 60 > seconds * 20 && n > 20 {
            a.spam.push(format!("unit {} given {n} orders in {seconds} s", u.slot));
        }
    }
    a
}

impl Audit {
    pub fn report(&self, what: &str) {
        let builds = self.ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Build { .. }, .. })).count();
        let reclaims = self.ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Reclaim(_) | Order::ReclaimUnit(_), .. })).count();
        let repairs = self.ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Repair(_), .. })).count();
        let moves = self.ordered.iter().filter(|(_, c)| matches!(c, Command::Order { order: Order::Move(_), .. })).count();
        println!(
            "{what}: {} orders ({builds} build, {reclaims} reclaim, {repairs} repair, {moves} move), nearest the enemy {:?}, under guns {}, conflicts {}, spam {}, dry m/e {}/{} s, idle {} of {} s",
            self.ordered.len(),
            self.nearest_foe,
            self.under_guns.len(),
            self.conflicts.len(),
            self.spam.len(),
            self.metal_dry,
            self.energy_dry,
            self.idle,
            self.seconds
        );
        for line in self.under_guns.iter().take(3).chain(self.conflicts.iter().take(3)).chain(self.spam.iter().take(3)) {
            println!("    ! {line}");
        }
    }
}


/// A spot for a unit of this type near `from`: land for most, water for ships.
pub fn spot_for(scene: &Scene, name: &str, from: FxVec3, dx: i64, dz: i64) -> FxVec3 {
    let t = scene.sim.types.get(scene.type_id(name));
    let near = from + FxVec3::from_ints(dx, 0, dz);
    if t.locomotion != Locomotion::Naval {
        return near;
    }
    let sea = scene.sim.world.sea_level;
    let (w, h) = scene.sim.world.size_px();
    for r in (96..4000).step_by(48) {
        for k in 0..32 {
            let p = from + ta_sim::fixed::heading_vector(k * 2048).scale(Fx::from_int(r));
            if p.x > Fx::from_int(64) && p.z > Fx::from_int(64) && p.x < w - Fx::from_int(64) && p.z < h - Fx::from_int(64) && scene.sim.world.height_at(p.x, p.z) < sea - Fx::from_int(24) {
                return p;
            }
        }
    }
    near
}

/// A constructor put on a duty beside our start (a ship on the nearest water).
pub fn serve(scene: &mut Scene, name: &str, duty: Duty, k: i64) -> UnitId {
    let at = spot_for(scene, name, scene.start, 60 + 40 * k, 60);
    let id = scene.spawn_at(name, 0, at);
    scene.officer.set(id, Some(duty));
    id
}

/// A spot on dry land the unit type could stand on but cannot walk (or float) to from our start: another island, a
/// walled plateau.
pub fn cut_off_spot(scene: &Scene, name: &str) -> Option<FxVec3> {
    use ta_sim::world::{MoveSpec, World};
    let t = scene.sim.types.get(scene.type_id(name));
    let spec = MoveSpec::for_type(t, &scene.sim.data.move_classes);
    let home = scene.sim.world.component(&spec, World::pos_to_anchor(scene.start, spec.footprint));
    let (w, h) = scene.sim.world.size_px();
    let mut best: Option<(i64, FxVec3)> = None;
    for z in (96..h.to_int() - 96).step_by(64) {
        for x in (96..w.to_int() - 96).step_by(64) {
            let p = FxVec3::from_ints(x, 0, z);
            let region = scene.sim.world.component(&spec, World::pos_to_anchor(p, spec.footprint));
            if region != 0 && region != home && scene.sim.world.passable(&spec, World::pos_to_anchor(p, spec.footprint), None) {
                let d = px(p, scene.start);
                if best.is_none_or(|(bd, _)| d < bd) && d > 700 {
                    best = Some((d, p));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

/// The free metal deposits (and features that are no wrecks) near our start, nearest first: where an extractor may go.
pub fn deposits_near(scene: &Scene, within: i64) -> Vec<FxVec3> {
    let mut spots: Vec<FxVec3> = scene
        .sim
        .features
        .iter()
        .filter(|f| f.alive && scene.sim.data.features[f.def].is_metal_deposit() && px(f.pos, scene.start) <= within)
        .map(|f| f.pos)
        .collect();
    spots.sort_by_key(|&p| px(p, scene.start));
    spots
}
