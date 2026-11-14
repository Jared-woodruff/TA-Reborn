//! The simulation: world state and the fixed 30 Hz tick.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tnt::TntMap;

use crate::cob::{CobHost, CobInstance, ThreadTag, TICKS_PER_SECOND};
use crate::combat::{Projectile, UnitGrid};
use crate::defs::{Locomotion, UnitType, UnitTypes, WeaponTypes};
use crate::events::SimEvent;
use crate::fixed::{FULL_TURN, Fx, FxVec3, QUARTER_TURN, atan2, cos, heading_of, heading_vector, rotate_y, sin, wrap_angle};
use crate::host::{HostRequest, OtherUnit, UnitHost, script_angle};
use crate::path::{FlowField, PathQuery, Pathfinder};
use crate::rng::Rng;
use crate::unit::{BuildJob, CaptureJob, Drift, Economy, MAX_PLAYERS, Order, Player, PlayerSetup, Unit, UnitId, piece_world_position, player_bit};
use crate::vision::{self, Seen, Sensor, Vision};
use crate::world::{CELL, MoveSpec, Occupant, Region, World};

mod air;

/// Base resource storage every player has regardless of buildings (TA default).
const BASE_STORAGE: i64 = 1000;
const PATH_EXPANSIONS: usize = 20_000;
/// A* node expansions allowed per tick across all units. Searches that exceed it return a
/// partial path and re-plan on arrival, bounding the cost of army-wide orders on huge maps.
const PATH_BUDGET_PER_TICK: u32 = 60_000;
/// Smallest budget a single search is given (so paths still make progress).
const PATH_MIN_SEARCH: u32 = 2_000;
/// Units sent to the same place in one tick before they share a flow field.
const FLOW_MIN_GROUP: u32 = 12;
/// Ticks a flow field stays valid (the world changing invalidates it sooner).
const FLOW_TICKS: u64 = 30 * 30;
/// Ticks a flow field may go without being brought up to date with new obstacles.
const FLOW_REPAIR_TICKS: u64 = 8;
/// Flow fields kept at once (each costs 5 bytes per cell of its window, the map at most).
const FLOW_CACHE_MAX: usize = 32;
/// Cells of room a shared route's field gets around its goal and the units asking for it.
const FLOW_MARGIN: i32 = 32;
/// How far apart (cells) the places units are sent to may be for them to share a distance
/// field (`Sim::plan_ways`): 2,560 px, a square of thousands sent off together.
const GROUP_REACH: i32 = 160;
/// Most distance fields groups get built in one tick (the groups beyond wait a tick).
const GROUP_BUILDS: usize = 12;
/// Routes read off shared fields in one tick from which they are read on all cores.
const PARALLEL_ROUTES: usize = 64;
/// Units planning their way alone (`Sim::alone`) remembered before starting afresh.
const ALONE_KEPT: usize = 1 << 16;
/// Nodes a search looks at for a unit its group's field doesn't reach (cut off from where it
/// was sent: the nearest point it can get to is near).
const CUT_OFF_SEARCH: usize = 2_000;

/// The window a shared route's field covers: the goal and this tick's `starts` (min x, min z,
/// max x, max z) with room around them for routes that swing wide (a quarter of their extent
/// more), and what an older field covered.
pub(crate) fn flow_window(world: &World, goal: (i32, i32), starts: (i32, i32, i32, i32), old: Option<((i32, i32), (i32, i32))>) -> ((i32, i32), (i32, i32)) {
    let (x0, z0, x1, z1) = (starts.0.min(goal.0), starts.1.min(goal.1), starts.2.max(goal.0), starts.3.max(goal.1));
    let margin = FLOW_MARGIN + (x1 - x0).max(z1 - z0) / 4;
    let (mut from, mut to) = ((x0 - margin, z0 - margin), (x1 + margin + 1, z1 + margin + 1));
    if let Some((a, b)) = old {
        from = (from.0.min(a.0), from.1.min(a.1));
        to = (to.0.max(b.0), to.1.max(b.1));
    }
    let (w, h) = (world.width as i32, world.height as i32);
    ((from.0.clamp(0, w), from.1.clamp(0, h)), (to.0.clamp(0, w), to.1.clamp(0, h)))
}

/// The window a distance field for a detour a search gave up on gets (see `Sim::move_to`):
/// the goal and `starts` with half their extent again round them, and twice a shared route's
/// room at least.
fn detour_window(world: &World, goal: (i32, i32), starts: CellBounds) -> ((i32, i32), (i32, i32)) {
    let (x0, z0, x1, z1) = (starts.0.min(goal.0), starts.1.min(goal.1), starts.2.max(goal.0), starts.3.max(goal.1));
    let margin = 2 * FLOW_MARGIN + (x1 - x0).max(z1 - z0) / 2;
    let (w, h) = (world.width as i32, world.height as i32);
    (((x0 - margin).clamp(0, w), (z0 - margin).clamp(0, h)), ((x1 + margin + 1).clamp(0, w), (z1 + margin + 1).clamp(0, h)))
}

/// Most waypoints a factory's rally route keeps.
const RALLY_MAX: usize = 16;

/// Maximum queued products per factory, including the unit currently being built.
pub const FACTORY_QUEUE_MAX: usize = 1000;
/// Queued unit waypoints, including the implicit point closing a patrol loop.
pub const ORDER_QUEUE_MAX: usize = 256;

/// The fire orders a unit starts with: its `StandingFireOrder` (0 hold fire, 1 return fire, 2
/// fire at will), but hold fire for TA's `NoAutoFire=1` units, whatever that says, as TA set
/// them on construction: the Big Bertha, the Intimidator, the Vulcan, the Buzzsaw, the Doomsday
/// Machine, the Annihilator. The anti-nukes (an interceptor among their weapons) are the
/// exception: they keep their fire at will, and fire at nukes whatever their fire orders. (A
/// mission's units placed for the player were never built, and keep their file's: see
/// [`file_fire_state`].)
pub(crate) fn initial_fire_state(def: &ta_data::UnitDef, anti_nuke: bool) -> u8 {
    if def.no_auto_fire && !anti_nuke { 0 } else { file_fire_state(def) }
}

/// The fire orders a unit's file gives it, `StandingFireOrder` (0 hold fire, 1 return fire, 2
/// fire at will), `NoAutoFire` or not: a unit that was never built keeps them.
pub(crate) fn file_fire_state(def: &ta_data::UnitDef) -> u8 {
    def.standing_fire_order.min(2) as u8
}

/// Commands can come from peers or saved files. This range comfortably contains every
/// supported map and keeps cell-coordinate subtraction/margins inside i32.
fn valid_command_position(pos: FxVec3) -> bool {
    const LIMIT: Fx = Fx::from_int(1 << 20);
    [pos.x, pos.y, pos.z].into_iter().all(|v| v >= -LIMIT && v <= LIMIT)
}

fn valid_order_position(order: &Order) -> bool {
    use crate::unit::Target;
    match order {
        Order::Move(p) | Order::Fight(p) | Order::Patrol(p) | Order::Unload(p) | Order::Build { pos: p, .. }
        | Order::Attack(Target::Ground(p)) | Order::ManualFire(Target::Ground(p)) => valid_command_position(*p),
        _ => true,
    }
}

pub(crate) fn has_order_room(orders: &std::collections::VecDeque<Order>, order: &Order) -> bool {
    let added = if matches!(order, Order::Patrol(_)) && !(orders.len() >= 2 && matches!(orders.back(), Some(Order::Patrol(_)))) { 2 } else { 1 };
    orders.len().saturating_add(added) <= ORDER_QUEUE_MAX
}

/// Appends `order` to a unit's queue. A patrol point pairs up with where the unit will be
/// by then (the previous waypoint, else `pos`) so the patrol loops; further queued patrol
/// points slot in before that implicit start, which stays last and closes the loop.
fn queue_order(orders: &mut std::collections::VecDeque<Order>, order: &Order, queue: bool, pos: FxVec3) {
    if !has_order_room(orders, order) {
        return;
    }
    if let Order::Patrol(dest) = order {
        match orders.back().cloned() {
            Some(Order::Patrol(_)) if queue && orders.len() >= 2 => {
                let last = orders.len() - 1;
                orders.insert(last, Order::Patrol(*dest));
            }
            last => {
                let start = match last {
                    Some(Order::Move(p) | Order::Fight(p) | Order::Patrol(p)) if queue => p,
                    _ => pos,
                };
                orders.push_back(Order::Patrol(*dest));
                orders.push_back(Order::Patrol(start));
            }
        }
        return;
    }
    if *order != Order::Stop || !queue {
        orders.push_back(order.clone());
    }
}

/// A player's instruction for this tick (from local input, AI or the network).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Command {
    /// Give an order to units; `queue` appends instead of replacing.
    Order { player: u8, #[serde(deserialize_with = "crate::serde_limits::command_list")] units: Vec<UnitId>, order: Order, queue: bool },
    /// Change a factory's production queue by `count` units of a type (negative removes).
    Produce { player: u8, factory: UnitId, type_id: usize, count: i32 },
    SetRally { player: u8, factory: UnitId, pos: FxVec3 },
    /// Turn units on/off (metal makers, solar collectors, radar, cloak...).
    SetActive { player: u8, #[serde(deserialize_with = "crate::serde_limits::command_list")] units: Vec<UnitId>, on: bool },
    /// Cloak or decloak units that can (Commander, spies, mines).
    SetCloak { player: u8, #[serde(deserialize_with = "crate::serde_limits::command_list")] units: Vec<UnitId>, on: bool },
    /// Standing orders: fire (0 hold, 1 return fire, 2 fire at will) and movement (0 hold
    /// position, 1 maneuver, 2 roam); `None` leaves one unchanged.
    SetStates { player: u8, #[serde(deserialize_with = "crate::serde_limits::command_list")] units: Vec<UnitId>, fire: Option<u8>, movement: Option<u8> },
    /// Starts a five-second self-destruct countdown, or cancels it when every unit given is
    /// already counting down.
    SelfDestruct { player: u8, #[serde(deserialize_with = "crate::serde_limits::command_list")] units: Vec<UnitId> },
    /// Factories re-queue each finished unit.
    SetRepeat { player: u8, #[serde(deserialize_with = "crate::serde_limits::command_list")] factories: Vec<UnitId>, on: bool },
    /// Gives metal and energy to an ally (as much as the giver has).
    Share { player: u8, to: u8, metal: Fx, energy: Fx },
    /// Gives units to an ally (not Commanders; passengers go with their transport, and
    /// no more than the ally's unit limit allows).
    GiveUnits { player: u8, #[serde(deserialize_with = "crate::serde_limits::command_list")] units: Vec<UnitId>, to: u8 },
    /// The player left the game: its units (but its Commander) go to its first surviving
    /// ally, if it has one. The multiplayer host issues it.
    Abandon { player: u8 },
    /// The player gives up: defeated, and their units blow up as when a Commander dies.
    Resign { player: u8 },
    /// Moves entry `index` of a factory's queue to the front: right behind the unit in
    /// production, or first when nothing is being built.
    QueueFront { player: u8, factory: UnitId, index: u32 },
    /// A cheat (ignored unless [`Sim::cheats_allowed`]).
    Cheat { player: u8, cheat: crate::cheats::Cheat },
    /// What the player passes to its allies on its own (see [`crate::unit::Sharing`]).
    SetSharing { player: u8, sharing: crate::unit::Sharing },
    /// A group order laid out as `shape` (a line the player drew, a staged scene's group
    /// keeping its shape): under the rule "modern movement" a formation that marches there
    /// together; else each unit simply goes to its place.
    Formation { player: u8, #[serde(deserialize_with = "crate::serde_limits::command_list")] units: Vec<UnitId>, order: Order, queue: bool, shape: crate::formation::Shape },
    /// A player marks a spot for their allies and says what they mean by it (computer allies
    /// act on it: see [`crate::signal`]).
    Signal { player: u8, kind: crate::signal::SignalKind, at: FxVec3 },
    /// Moves a queued entry to the front (`to_front`), or drops it, as the queue stands when
    /// the command runs: a factory's entry of `type_id` (`pos` none; the unit in production
    /// keeps its place), or a constructor's order to build `type_id` at `pos`; the one at
    /// `index` if it is still that, else the nearest that is (the queue moved on since the
    /// player clicked: a unit or a building finished); none left, nothing. (`QueueFront` moved
    /// whatever had come to the index; a constructor's orders were sent again from the player's
    /// copy, bringing back what had been built since.)
    EditQueue { player: u8, unit: UnitId, index: u32, type_id: usize, pos: Option<FxVec3>, to_front: bool },
    /// TA's `+shootall`, on or off: the player's units' weapons take on every enemy in reach
    /// alike, their bad targets too (see [`crate::unit::Player::shoot_all`]).
    /// (Last: replays keep the earlier variants' numbers.)
    SetShootAll { player: u8, on: bool },
}

/// The index nearest `index` (the lower first, of two as near) of those below `len` that `is`
/// takes.
fn nearest_index(len: usize, index: usize, is: impl Fn(usize) -> bool) -> Option<usize> {
    let index = index.min(len);
    (0..=len.max(index)).flat_map(|d| [index.checked_sub(d), (d > 0).then_some(index + d)]).flatten().filter(|&j| j < len).find(|&j| is(j))
}

/// Per-unit phase timings are taken on one tick in this many (see `Sim::sampled`).
const PHASE_SAMPLE: u64 = 8;
/// Units from which the ground units' step is taken on all cores.
pub(crate) const PARALLEL_MOVERS: usize = 1_500;
/// Units from which their scripts run on all cores.
const PARALLEL_SCRIPTS: usize = 1_500;

/// How long construction may stand with nobody at it before it un-builds (see
/// `Sim::decay_abandoned_frames`): ten seconds, for a builder called away a moment.
const FRAME_GRACE: u64 = 10 * TICKS_PER_SECOND as u64;
/// How long a whole frame takes to un-build, in ticks (a hundred seconds; one half built, fifty).
const FRAME_DECAY: u64 = 100 * TICKS_PER_SECOND as u64;
/// Self-destruct countdown (five seconds, as in TA).
pub const SELF_DESTRUCT_TICKS: u64 = 5 * TICKS_PER_SECOND as u64;
/// How far a maneuvering unit will stray from its spot to answer fire.
const MANEUVER_LEASH: i64 = 420;
/// A unit on the last leg of its route that the crowd has kept from coming any nearer its goal
/// (by this much) for `ARRIVE_IDLE` ticks, within `ARRIVE_REACH` times its radius of it, has
/// arrived: a crowd holds the place.
const ARRIVE_GAIN: Fx = Fx::ONE;
const ARRIVE_IDLE: u32 = 30;
const ARRIVE_REACH: i64 = 12;
/// One on the way that the crowd has kept from its next waypoint this long goes on to the one
/// after if the way there is open.
const STALL_IDLE: u32 = 30;
/// One held up head on by its own side's units this long slips past them for `SQUEEZE_TICKS`.
const SQUEEZE_IDLE: u32 = 45;
const SQUEEZE_TICKS: u64 = 90;
/// An idle ground unit is looked at this often (ticks) for standing in a building, and placed
/// on the nearest free cell within so many rings of cells (`Sim::step_out_of_building`).
const BUILDING_CHECK_TICKS: u64 = 8;
const BUILDING_ESCAPE_RINGS: i32 = 12;
/// A construction aircraft at work faces what it builds and sways round it along an arc, out to
/// one side and back, as TA's do (`Sim::drift_round`): this far round it (65536 angle units a
/// turn: 45 degrees), there and back in no less than the first number of ticks and no more than
/// the second, slow enough that it turns to keep facing what it builds.
const AIR_BUILDER_SWEEP: i64 = 8192;
const AIR_BUILDER_SWEEP_TICKS: (i64, i64) = (240, 1200);
/// Starting further out than it keeps within reach of it all the way round, it comes in over
/// this many ticks.
const AIR_BUILDER_SETTLE_TICKS: u64 = 60;
/// It sways once it faces what it builds to within this many angle units (2.8 degrees).
const AIR_BUILDER_FACING: i32 = 512;
/// Within twice its turning circle's radius at its speed of its next waypoint (`LOOK_RADII`; at
/// least `LOOK_MIN` px, at most `LOOK_MAX`), a unit on its way looks up to this many waypoints
/// past it for one to steer for (`Sim::carrot`).
const CARROT_WAYPOINTS: usize = 4;
const LOOK_RADII: i64 = 2;
const LOOK_MIN: i64 = 32;
const LOOK_MAX: i64 = 96;
/// 2Ï€, 16.16: TA's turn rates (angle units a tick, 65536 a turn) in radians a tick.
const TWO_PI: Fx = Fx(411_775);

/// A unit type's turn rate Ï‰ in radians a tick: turning at speed v, it runs round a circle of
/// radius v / Ï‰.
fn turn_omega(t: &UnitType) -> Fx {
    Fx(t.turn_rate.max(1) as i64 * TWO_PI.0 / FULL_TURN as i64)
}

/// Whether `d` px lies within `radii` of a unit's turning circle's radius at `speed` (turning
/// at `omega`), and no less than `min` or more than `max` px: worked out without a division.
fn within_turns(d: Fx, speed: Fx, omega: Fx, radii: i64, min: i64, max: i64) -> bool {
    d <= Fx::from_int(min) || d <= Fx::from_int(max) && d * omega <= speed * radii
}

/// The middle of a footprint anchored at a cell (`World::anchor_to_pos` on the plane: where a
/// waypoint is, without the ground's height under it).
pub(crate) fn anchor_middle(c: (i32, i32), fp: (u32, u32)) -> FxVec3 {
    let (x0, z0) = World::footprint_origin(c, fp);
    FxVec3::new(Fx::from_int(x0 as i64 * CELL + fp.0 as i64 * CELL / 2), Fx::ZERO, Fx::from_int(z0 as i64 * CELL + fp.1 as i64 * CELL / 2))
}

/// Wall-clock cost of each tick phase (diagnostics only; never affects the simulation).
#[derive(Clone, Debug, Default)]
pub struct SimStats {
    pub ticks: u64,
    pub units_us: u64,
    /// Parts of `units_us`, in nanoseconds (per-unit spans are far below a microsecond):
    /// order logic, movement, weapons, sampled on one tick in eight and scaled up (reading the
    /// clock around every unit's phases on every tick cost a sixth of it); and the unit
    /// scripts, every tick (all of them run at once, on all cores).
    pub orders_ns: u64,
    pub motion_ns: u64,
    pub weapons_ns: u64,
    pub scripts_ns: u64,
    pub collisions_us: u64,
    pub projectiles_us: u64,
    pub vision_us: u64,
    pub deaths_us: u64,
    pub economy_us: u64,
    pub path_searches: u64,
    /// Flow fields built for large group moves.
    pub flow_builds: u64,
    /// Flow fields brought up to date with new buildings and wrecks instead of built again.
    pub flow_repairs: u64,
    /// Routes read off flow fields instead of searched.
    pub flow_routes: u64,
    /// Part of `path_us` spent building flow fields and bringing them up to date.
    pub flow_us: u64,
    /// Part of `path_us` spent reading routes off flow fields.
    pub route_us: u64,
    pub path_us: u64,
}

#[derive(Clone)]
struct CachedPath {
    path: Vec<(i32, i32)>,
    goal_pos: FxVec3,
    tick: u64,
    occupancy_version: u64,
}

/// Cached paths expire after this many ticks.
const PATH_CACHE_TICKS: u64 = 300;
/// Cached paths kept before the stale ones are cleared out.
const PATH_CACHE_PURGE: usize = 4096;

/// A map feature (tree, rock, metal deposit, wreck).
#[derive(Clone, Debug)]
pub struct Feature {
    pub def: usize,
    pub origin: (i32, i32),
    pub pos: FxVec3,
    pub alive: bool,
    pub heading: i32,
    pub health: Fx,
    /// Reclaim progress 0..=1, and the work behind it (see `Sim::update_reclaim_order`).
    pub reclaimed: Fx,
    pub reclaim_work: u64,
    /// Resurrection progress 0..=1 (unit wrecks only), and the work behind it (of the unit's
    /// build work).
    pub resurrected: Fx,
    pub resurrect_work: u64,
    /// Burning until this tick (0: not on fire); the next spread/scorch at `next_spark`.
    pub burn_until: u64,
    pub next_spark: u64,
}

impl Feature {
    /// A whole feature of definition `def` (footprint from `origin`), not on fire.
    pub(crate) fn new(def: usize, origin: (i32, i32), pos: FxVec3, heading: i32, health: Fx) -> Feature {
        Feature { def, origin, pos, alive: true, heading, health, reclaimed: Fx::ZERO, reclaim_work: 0, resurrected: Fx::ZERO, resurrect_work: 0, burn_until: 0, next_spark: 0 }
    }
}

/// A cell of the map's 16 px grid (x, z).
type GridCell = (i32, i32);
/// A shared route: movement class, start block (4 Ã— 4 cells), goal cell, arrival radius.
type RouteKey = (MoveSpec, GridCell, GridCell, i32);
/// Cells a route's users start from: min x, min z, max x, max z.
type CellBounds = (i32, i32, i32, i32);

/// Where a unit's way runs (`Sim::aim`): its anchor cell, the goal cell (and the point there it
/// makes for), its region, and how far a search for it may look.
#[derive(Clone, Copy)]
struct Aim {
    start: GridCell,
    goal: GridCell,
    goal_pos: FxVec3,
    region: Region,
    max_expansions: usize,
}

/// A ground unit's way to a place, put off until every unit has had its orders
/// (`Sim::plan_ways`). `far`: one of a formation, going its own way to its place: it leaves its
/// group's shared field as early as it can (`FlowField::route_to_with`).
#[derive(Clone)]
pub(crate) struct WayRequest {
    pub(crate) id: UnitId,
    pub(crate) spec: MoveSpec,
    pub(crate) footprint: (u32, u32),
    pub(crate) from: FxVec3,
    pub(crate) dest: FxVec3,
    pub(crate) far: bool,
}

/// Units of one movement class heading to places close together (`Sim::plan_ways`): the
/// first one's goal, the requests (by index), and the cells they start from and go to.
struct WayGroup {
    spec: MoveSpec,
    first: GridCell,
    members: Vec<usize>,
    bounds: CellBounds,
}

/// Units copied on all cores from this many slots (`Slots`).
const PARALLEL_COPY: usize = 2_048;

/// The units by slot: a vector, copied on all cores when big (the battle highlights copy the
/// game every 20 seconds, and tens of thousands of units each own a few allocations).
#[derive(Default)]
pub(crate) struct Slots(Vec<Option<Unit>>);

impl Clone for Slots {
    fn clone(&self) -> Slots {
        use rayon::prelude::*;
        if self.0.len() >= PARALLEL_COPY { Slots(self.0.par_iter().cloned().collect()) } else { Slots(self.0.clone()) }
    }
}

impl std::ops::Deref for Slots {
    type Target = Vec<Option<Unit>>;
    fn deref(&self) -> &Vec<Option<Unit>> {
        &self.0
    }
}

impl std::ops::DerefMut for Slots {
    fn deref_mut(&mut self) -> &mut Vec<Option<Unit>> {
        &mut self.0
    }
}

impl<'a> IntoIterator for &'a Slots {
    type Item = &'a Option<Unit>;
    type IntoIter = std::slice::Iter<'a, Option<Unit>>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a mut Slots {
    type Item = &'a mut Option<Unit>;
    type IntoIter = std::slice::IterMut<'a, Option<Unit>>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

/// The simulation. A clone plays on bit for bit as the original (the battle highlights keep
/// copies; `ta_ai`'s `snapshot` test checks it), so its state stays plain data.
#[derive(Clone)]
pub struct Sim {
    pub tick: u64,
    pub types: Arc<UnitTypes>,
    pub weapon_types: Arc<WeaponTypes>,
    pub data: Arc<GameData>,
    pub world: World,
    pub(crate) units: Slots,
    generations: Vec<u32>,
    free_slots: Vec<u32>,
    pub players: Vec<Player>,
    /// The map's start positions (its schema's, else its first schema's): where any player may
    /// have started, as a player choosing a start sees them (a computer player looks for its
    /// enemies there; which of them they took, it finds out).
    pub start_spots: Vec<FxVec3>,
    pub features: Vec<Feature>,
    /// The features by square (`Sim::features_near`).
    pub(crate) feature_grid: crate::features::FeatureGrid,
    pub projectiles: Vec<Projectile>,
    /// The projectiles that ended in the last tick (their `pos` where the tick's flight would
    /// have taken them), each with how far through the tick (0 to 1) it met what it hit: for
    /// the client to draw them on to their end. (Not part of the game: nothing reads them.)
    pub spent: Vec<(Projectile, Fx)>,
    pub(crate) next_projectile_id: u32,
    pub(crate) grid: UnitGrid,
    pub(crate) crowd: crate::crowd::CrowdGrid,
    pub(crate) scratch: Vec<u32>,
    pub(crate) aim_from_cache: rustc_hash::FxHashMap<(usize, u8), Option<i32>>,
    /// Recent paths shared by units starting in the same 64 px block with the same goal
    /// (army-wide orders would otherwise repeat thousands of identical searches).
    path_cache: rustc_hash::FxHashMap<RouteKey, CachedPath>,
    /// Size of `path_cache` at which its stale routes are cleared out next.
    path_cache_limit: usize,
    path_budget: u32,
    /// Gravity in pixels per tickÂ².
    pub gravity: Fx,
    /// The game's random seed (computer players draw theirs from it).
    pub seed: u64,
    pub(crate) rng: Rng,
    /// Features on fire (indices), in the order they caught.
    pub(crate) fires: Vec<usize>,
    pub(crate) events: Vec<SimEvent>,
    /// Signals given lately, oldest first (see [`crate::signal`]).
    pub signals: Vec<crate::signal::Signal>,
    pathfinder: Pathfinder,
    pub(crate) move_specs: Vec<MoveSpec>,
    pub stats: SimStats,
    /// TA's default rule: losing your commander loses the game.
    pub commander_ends_game: bool,
    /// Most units one player may own (factories and builders wait at the limit).
    pub unit_limit: u32,
    /// Feature definition of a unit's wreck â†’ the unit type (for resurrection).
    wreck_of: rustc_hash::FxHashMap<usize, usize>,
    /// Nearest cell of a region to an unreachable goal, per (spec, region, goal): static
    /// terrain, so the answers never go stale.
    reachable_near: rustc_hash::FxHashMap<(MoveSpec, Region, GridCell), Option<GridCell>>,
    /// Goals units can get to, per (spec, goal) (`path::open_goal`), and the
    /// `World::occupancy_version` they were found at.
    open_goals: rustc_hash::FxHashMap<(MoveSpec, GridCell), (u64, GridCell)>,
    /// Spread order-independent phases (crowd separation) over all cores. The result is
    /// identical either way; tests turn it off to prove that.
    pub parallel: bool,
    /// Current wind speed (energy per second for an unlimited wind generator).
    pub wind: Fx,
    /// Where the wind blows to (angle units); wind generators turn to face it.
    pub wind_heading: i32,
    /// Line of sight, radar and sonar coverage per player.
    pub vision: Vision,
    /// The running mission's objectives (campaign games).
    pub mission: Option<crate::mission::MissionRules>,
    /// Buildable unit types (mission restrictions, a skirmish's mutators'); `None` allows
    /// everything.
    pub allowed: Option<Vec<bool>>,
    /// The skirmish's mutators (see mutators.rs): as TA plays by default.
    pub mutators: crate::Mutators,
    /// Whether [`Command::Cheat`] is taken (games with one person in them); otherwise cheats
    /// are ignored.
    pub cheats_allowed: bool,
    /// Units destroyed per (owner, type), for mission objectives and statistics.
    pub deaths_by: rustc_hash::FxHashMap<(u8, usize), u64>,
    /// (owner, type) of every unit finished by construction.
    pub built: rustc_hash::FxHashSet<(u8, usize)>,
    /// (new owner, type) of every captured unit.
    pub captured: rustc_hash::FxHashSet<(u8, usize)>,
    /// Unit types weapons never pick automatically (mission capture targets).
    pub no_auto_target: rustc_hash::FxHashSet<usize>,
    /// Projectiles an interceptor is already chasing.
    pub(crate) intercepted: rustc_hash::FxHashSet<u32>,
    /// Attach/drop requests from the script that just ran.
    host_requests: Vec<HostRequest>,
    /// Distance fields shared by large groups heading to one place, and this tick's
    /// demand per destination.
    pub(crate) flow_cache: rustc_hash::FxHashMap<(MoveSpec, GridCell), Arc<FlowField>>,
    /// This tick's requests for each shared route: how many, and the cells they start from.
    flow_demand: rustc_hash::FxHashMap<(MoveSpec, GridCell), (u32, CellBounds)>,
    /// Ground units' ways put off while the units have their orders (`defer_ways`), planned
    /// together once they all have (`Sim::plan_ways`).
    pub(crate) way_requests: Vec<WayRequest>,
    defer_ways: bool,
    /// Group orders go as formations (the rule "modern movement", `formation.rs`): laid out
    /// in rows, formed up and marched together. Off, each unit goes to the point it is sent to
    /// as in TA.
    pub group_formations: bool,
    /// The formations on their way, in the order they were made, and the next one's id.
    pub formations: Vec<crate::formation::Formation>,
    pub(crate) next_formation: u32,
    /// Each unit's pace this tick in its formation, by slot: a cap on its speed.
    pub(crate) paces: Vec<Option<Fx>>,
    /// Units no group's field took to where they were sent (standing where it doesn't reach:
    /// walled in by trees, or cut off), that place and since when: they plan their way alone
    /// while sent there, for as long as a field is kept (else every tick would make their
    /// group, and its field, again).
    alone: rustc_hash::FxHashMap<UnitId, (FxVec3, u64)>,
    /// Fog of war: units outside a player's sensors are hidden from them and can't be
    /// targeted automatically. Off = everything visible (TA's "LOS off").
    pub los_enabled: bool,
    /// Units give way (the rule; `avoid.rs`): moving ground units bend round those ahead of
    /// them instead of queueing and shoving. Off keeps TA's movement exactly.
    pub local_avoidance: bool,
    /// Each unit's steer round the units ahead (by slot; see `avoid::plan`).
    steers: Vec<crate::avoid::Steer>,
    wind_range: (Fx, Fx),
    tidal: Fx,
    /// Acid seas (TA's `waterdoesdamage`): health a second lost by what is in the water.
    pub water_damage: Fx,
}

impl Sim {
    /// The map's tidal strength: energy per second per unit of a tidal generator's output.
    pub fn tidal_strength(&self) -> Fx {
        self.tidal
    }

    pub fn new(
        types: Arc<UnitTypes>,
        data: Arc<GameData>,
        map: &TntMap,
        info: &MapInfo,
        schema: usize,
        players: &[PlayerSetup],
        seed: u64,
    ) -> Sim {
        let mut world = World::new(map);
        let move_specs: Vec<MoveSpec> = types.types.iter().map(|t| MoveSpec::for_type(t, &data.move_classes)).collect();
        let t0 = std::time::Instant::now();
        let walkers: Vec<MoveSpec> = types.types.iter().zip(&move_specs).filter(|(t, _)| matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval)).map(|(_, s)| *s).collect();
        world.prepare_all(&walkers);
        log::info!("movement regions for {} unit types in {:.0} ms", walkers.len(), t0.elapsed().as_secs_f64() * 1000.0);
        assert!(players.len() <= MAX_PLAYERS, "{} players: a game holds at most {MAX_PLAYERS}", players.len());
        let surface_metal = info.schemas.get(schema).map_or(0.0, |s| s.surface_metal).clamp(0.0, u32::MAX as f64) as u32;
        world.surface_metal = surface_metal;
        for z in 0..world.height as i32 {
            for x in 0..world.width as i32 {
                world.set_metal(x, z, surface_metal);
            }
        }

        // Features from the map: metal deposits feed the metal map, solid ones block cells.
        let mut features = Vec::new();
        for (i, &f) in map.features.iter().enumerate() {
            let Some(name) = map.feature_names.get(f as usize) else { continue };
            let Some(def_id) = data.feature_id(name) else { continue };
            let def = &data.features[def_id];
            let origin = ((i as u32 % map.width) as i32, (i as u32 / map.width) as i32);
            let fp = (def.footprint_x, def.footprint_z);
            let deposit = def.is_metal_deposit();
            if deposit {
                for dz in 0..fp.1 as i32 {
                    for dx in 0..fp.0 as i32 {
                        world.set_metal(origin.0 + dx, origin.1 + dz, def.metal.clamp(0.0, u32::MAX as f64) as u32);
                    }
                }
            }
            let slot = features.len() as u32;
            // Vents default to blocking like other features, but plants are built over them.
            if def.blocking && !deposit && !def.geothermal {
                world.occupy(Occupant::Feature(slot), origin, fp, &[]);
            }
            if def.geothermal {
                world.mark_geo(origin, fp);
            }
            let x = Fx::from_int(origin.0 as i64 * CELL) + Fx::from_int(fp.0 as i64 * CELL) / 2;
            let z = Fx::from_int(origin.1 as i64 * CELL) + Fx::from_int(fp.1 as i64 * CELL) / 2;
            features.push(Feature::new(def_id, origin, FxVec3::new(x, world.height_at(x, z), z), 0, Fx::from_f64(def.damage.max(1.0))));
        }

        let start_spots: Vec<FxVec3> = info
            .schemas
            .get(schema)
            .or_else(|| info.schemas.first())
            .map_or_else(Vec::new, |s| s.start_positions.iter().map(|&(_, x, z)| FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z))).collect());
        let players: Vec<Player> = players
            .iter()
            .enumerate()
            .map(|(i, p)| Player {
                index: i as u8,
                side: p.side.clone(),
                color: p.color,
                ai: p.ai,
                start: p.start,
                economy: Economy {
                    metal: p.metal,
                    energy: p.energy,
                    metal_capacity: Fx::from_int(BASE_STORAGE),
                    energy_capacity: Fx::from_int(BASE_STORAGE),
                    build_factor: Fx::ONE,
                    ..Default::default()
                },
                defeated: false,
                had_commander: false,
                unit_count: 0,
                base_storage: (Fx::from_int(BASE_STORAGE), Fx::from_int(BASE_STORAGE)),
                last_alert: None,
                stats: Default::default(),
                team: i as u8,
                cheats: Default::default(),
                endless: false,
                sharing: Default::default(),
                perks: Default::default(),
                shoot_all: false,
            })
            .collect();
        let pathfinder = Pathfinder::new(world.width, world.height);
        let grid = UnitGrid::new(&world);
        let feature_grid = crate::features::FeatureGrid::new(&world, &features);
        // (Routes and distance fields start from the map as it stands now.)
        world.forget_changes();
        let weapon_types = Arc::new(WeaponTypes::compile(&data));
        // TA gravity is in pixels per secondÂ²; the default map value is 112.
        let gravity = Fx::from_f64(info.gravity.max(1.0) / (TICKS_PER_SECOND * TICKS_PER_SECOND) as f64);
        let wind_range = (Fx::from_f64(info.min_wind / 100.0), Fx::from_f64(info.max_wind.max(info.min_wind) / 100.0));
        let (map_w, map_h) = world.size_px();
        let vision = Vision::new(map_w.to_int(), map_h.to_int(), players.len());
        // Which unit each wreck is: the first type (lowest id) whose Corpse names it.
        let mut wreck_of = rustc_hash::FxHashMap::default();
        for t in &types.types {
            if let Some(f) = data.feature_id(&t.def.corpse) {
                wreck_of.entry(f).or_insert(t.id);
            }
        }
        let mut sim = Sim {
            tick: 0,
            types,
            weapon_types,
            data,
            world,
            units: Slots::default(),
            generations: Vec::new(),
            free_slots: Vec::new(),
            players,
            start_spots,
            features,
            feature_grid,
            projectiles: Vec::new(),
            spent: Vec::new(),
            next_projectile_id: 1,
            grid,
            crowd: Default::default(),
            scratch: Vec::new(),
            aim_from_cache: Default::default(),
            path_cache: Default::default(),
            path_cache_limit: PATH_CACHE_PURGE,
            path_budget: PATH_BUDGET_PER_TICK,
            gravity,
            stats: SimStats::default(),
            commander_ends_game: false,
            unit_limit: u32::MAX,
            wreck_of,
            reachable_near: Default::default(),
            open_goals: Default::default(),
            parallel: true,
            seed,
            rng: Rng::new(seed),
            fires: Vec::new(),
            events: Vec::new(),
            signals: Vec::new(),
            pathfinder,
            move_specs,
            wind: Fx::ZERO,
            wind_heading: 0,
            wind_range,
            tidal: Fx::from_f64(info.tidal_strength),
            water_damage: if info.water_does_damage { Fx::from_f64(info.water_damage.max(0.0)) } else { Fx::ZERO },
            vision,
            los_enabled: true,
            local_avoidance: false,
            steers: Vec::new(),
            mission: None,
            allowed: None,
            mutators: crate::Mutators::default(),
            cheats_allowed: false,
            deaths_by: Default::default(),
            built: Default::default(),
            captured: Default::default(),
            no_auto_target: Default::default(),
            intercepted: Default::default(),
            host_requests: Vec::new(),
            flow_cache: Default::default(),
            flow_demand: Default::default(),
            way_requests: Vec::new(),
            defer_ways: false,
            group_formations: false,
            formations: Vec::new(),
            next_formation: 1,
            paces: Vec::new(),
            alone: Default::default(),
        };
        sim.change_wind();
        sim
    }

    // ---- Queries ------------------------------------------------------------------------

    pub fn unit(&self, id: UnitId) -> Option<&Unit> {
        self.units.get(id.slot as usize)?.as_ref().filter(|u| u.id == id)
    }

    pub(crate) fn unit_mut(&mut self, id: UnitId) -> Option<&mut Unit> {
        self.units.get_mut(id.slot as usize)?.as_mut().filter(|u| u.id == id)
    }

    pub fn units(&self) -> impl Iterator<Item = &Unit> {
        self.units.iter().flatten()
    }

    /// Every unit slot, empty or not (a unit's `id.slot` is its place): for those who keep
    /// something of their own per unit beside the simulation's.
    pub fn unit_slots(&self) -> &[Option<Unit>] {
        &self.units
    }

    pub fn unit_count(&self) -> usize {
        self.units.iter().flatten().count()
    }

    pub fn unit_type(&self, u: &Unit) -> &UnitType {
        self.types.get(u.type_id)
    }

    pub fn take_events(&mut self) -> Vec<SimEvent> {
        std::mem::take(&mut self.events)
    }

    /// The features that may lie within `r` of `pos` (standing or not), in index order: every
    /// feature that does is among them. (A big map has tens of thousands.)
    pub fn features_near(&self, pos: FxVec3, r: Fx, out: &mut Vec<u32>) {
        self.feature_grid.near(pos, r, out);
    }

    /// Adds a feature to the map, returning its index (indices only grow).
    pub(crate) fn add_feature(&mut self, f: Feature) -> usize {
        let index = self.features.len();
        self.features.push(f);
        self.feature_grid.add(index, &self.features);
        index
    }

    /// Cloak upkeep (energy per tick; more while moving) and who can see through it: any
    /// enemy unit within the cloaked unit's `MinCloakDistance`.
    fn update_cloak(&mut self, id: UnitId, t: &UnitType) {
        let Some(u) = self.unit(id) else { return };
        let was = u.cloaked;
        if !u.cloak_on || self.tick < u.decloak_until {
            if let Some(u) = self.unit_mut(id) {
                u.cloaked = false;
            }
            if was {
                self.events.push(SimEvent::CloakChanged { id, cloaked: false });
            }
            return;
        }
        let moving = u.speed > Fx::ZERO;
        let per_second = if moving && t.def.cloak_cost_moving > 0.0 { t.def.cloak_cost_moving } else { t.def.cloak_cost };
        let cost = Fx::from_f64(per_second) / TICKS_PER_SECOND;
        let (owner, pos) = (u.owner, u.pos);
        let e = &mut self.players[owner as usize].economy;
        let paid = e.energy >= cost;
        if paid {
            e.energy -= cost;
            e.tick_energy_out += cost;
        }
        let reach = Fx::from_f64(t.def.min_cloak_distance.max(8.0));
        let mut near = std::mem::take(&mut self.scratch);
        self.grid.query(pos, reach + Fx::from_int(48), &mut near);
        let mut mask = 0u64;
        for &slot in &near {
            let Some(o) = self.units.get(slot as usize).and_then(|u| u.as_ref()) else { continue };
            if !o.dead && self.is_enemy(o.owner, owner) && o.pos.within_xz(pos, reach + self.types.get(o.type_id).radius) {
                mask |= player_bit(o.owner);
            }
        }
        self.scratch = near;
        if let Some(u) = self.unit_mut(id) {
            u.cloaked = paid;
            u.revealed_to = mask;
        }
        if was != paid {
            self.events.push(SimEvent::CloakChanged { id, cloaked: paid });
        }
    }

    /// How `player` currently perceives a unit: in line of sight, as a radar/sonar
    /// contact, or not at all. Own units (and everything, with LOS off) are visible.
    /// Cloaked units are hidden entirely unless one of the player's units is close.
    pub fn seen_by(&self, u: &Unit, player: u8) -> Seen {
        if self.allied(u.owner, player) {
            return Seen::Visible;
        }
        if u.cloaked && u.revealed_to & self.team_mask(player) == 0 {
            return Seen::Hidden;
        }
        if u.carried_by.is_some() && u.carry_piece.is_none() {
            return Seen::Hidden; // stowed inside a transport
        }
        if !self.los_enabled {
            return Seen::Visible;
        }
        let flags = self.vision.flags_at(self.vision_slot(player), u.pos);
        let jammed = self.vision.flags_at(self.vision_slot(u.owner), u.pos);
        let submerged = self.submerged(u);
        let sonar = (self.types.get(u.type_id).locomotion == Locomotion::Naval || submerged)
            && flags & vision::SONAR != 0 && jammed & vision::SONAR_JAM == 0;
        if submerged && !sonar { return Seen::Hidden; }
        if flags & vision::LOS != 0 {
            return Seen::Visible;
        }
        let t = self.types.get(u.type_id);
        if t.def.stealth && !sonar {
            return Seen::Hidden;
        }
        let radar = !t.def.stealth && !submerged && flags & vision::RADAR != 0 && jammed & vision::RADAR_JAM == 0;
        if radar || sonar { Seen::Radar } else { Seen::Hidden }
    }

    /// Airborne missile contacts use their current position, even after their launcher
    /// has been destroyed. Radar shares allied coverage and respects enemy jamming.
    pub fn projectile_seen_by(&self, projectile: &Projectile, player: u8) -> Seen {
        if projectile.dead {
            return Seen::Hidden;
        }
        self.missile_seen_at(projectile.owner_player, projectile.pos, player)
    }

    /// Detection of a missile's current position, including at the instant it is
    /// intercepted and its projectile is no longer present in the live list.
    pub fn missile_seen_at(&self, owner: u8, pos: FxVec3, player: u8) -> Seen {
        if !self.los_enabled || self.allied(owner, player) {
            return Seen::Visible;
        }
        let flags = self.vision.flags_at(self.vision_slot(player), pos);
        if flags & vision::LOS != 0 {
            return Seen::Visible;
        }
        let jammed = self.vision.flags_at(self.vision_slot(owner), pos);
        if flags & vision::RADAR != 0 && jammed & vision::RADAR_JAM == 0 { Seen::Radar } else { Seen::Hidden }
    }

    /// Whether a unit is under the water: a submarine (a ship with a `WaterLine`: the
    /// simulation keeps ships at the surface, the client draws submarines that far under it),
    /// or one on the sea bed with the water over its top (an amphibious kbot crossing; a tank
    /// in the shallows is not). As in TA, only weapons that go under the water (torpedoes,
    /// depth charges: `waterweapon`) reach it, and radar doesn't see it; sonar does.
    pub fn submerged(&self, u: &Unit) -> bool {
        let t = self.types.get(u.type_id);
        match t.locomotion {
            Locomotion::Naval => t.waterline > Fx::ZERO && u.model_pos(t).y < self.world.sea_level && self.world.depth_at(u.pos.x, u.pos.z) > Fx::ZERO,
            Locomotion::Air | Locomotion::Hover => false,
            _ => u.pos.y + t.model.height.max(Fx::from_int(8)) < self.world.sea_level,
        }
    }

    /// Whether a point is inside `player`'s line of sight (always true with LOS off).
    pub fn in_sight(&self, player: u8, pos: FxVec3) -> bool {
        !self.los_enabled || self.vision.flags_at(self.vision_slot(player), pos) & vision::LOS != 0
    }

    /// Whether `player`'s side has ever had a point in sight.
    pub fn explored(&self, player: u8, pos: FxVec3) -> bool {
        !self.los_enabled || self.vision.flags_at(self.vision_slot(player), pos) & (vision::LOS | vision::EXPLORED) != 0
    }

    /// TA's "true" line of sight: hills hide what is behind them (off: sight is a circle
    /// round each unit, TA's "circular").
    pub fn set_true_sight(&mut self, on: bool) {
        let heights = on.then(|| {
            let (w, h) = (self.vision.width as i32, self.vision.height as i32);
            let sea = self.world.sea_level.to_int() as i32;
            // (Vision cells are two ground cells across: their centre is a corner of four.)
            (0..h).flat_map(|z| (0..w).map(move |x| (x, z))).map(|(x, z)| (self.world.vertex_height(2 * x + 1, 2 * z + 1).to_int() as i32).max(sea)).collect()
        });
        self.vision.set_terrain(heights);
        if (self.tick as i64) > 0 {
            self.rebuild_vision();
        }
    }

    /// Acid seas (TA's `waterdoesdamage`): once a second, what is in the water, on the
    /// bottom or afloat, loses the map's `waterdamage` in health (scripts, alerts and
    /// kills aside); hovercraft ride over it and aircraft fly over.
    fn acid_seas(&mut self) {
        if self.water_damage <= Fx::ZERO || !self.tick.is_multiple_of(TICKS_PER_SECOND as u64) {
            return;
        }
        let sea = self.world.sea_level - Fx::from_int(2);
        let wet: Vec<UnitId> = self
            .units
            .iter()
            .flatten()
            .filter(|u| {
                let t = self.types.get(u.type_id);
                !u.dead
                    && u.carried_by.is_none()
                    && match t.locomotion {
                        Locomotion::Hover | Locomotion::Air => false,
                        Locomotion::Naval => true,
                        _ => self.world.height_at(u.pos.x, u.pos.z) < sea,
                    }
            })
            .map(|u| u.id)
            .collect();
        let nobody = UnitId { slot: u32::MAX, generation: 0 };
        let types = self.types.clone();
        for id in wet {
            let owner = self.unit(id).map_or(0, |u| u.owner);
            let damage = self.cheat_damage(self.water_damage, nobody, id, owner);
            let Some(u) = self.unit_mut(id) else { continue };
            let t = types.get(u.type_id);
            let damage = if u.armored { damage * Fx::from_f64(t.def.damage_modifier) } else { damage };
            u.health -= damage;
            u.last_damage = damage;
            if u.health <= Fx::ZERO {
                u.dead = true;
                u.killer = Some(nobody);
            }
        }
    }

    /// Whether `u` is at work: finished, switched on (or not switchable), its upkeep paid, neither
    /// paralysed nor carried. A metal maker switched off, or short of energy, is not; the scene
    /// shows it so (its lights out).
    pub fn working(&self, u: &Unit) -> bool {
        !u.dead && u.is_complete() && u.carried_by.is_none() && self.tick >= u.paralyzed_until
            && u.upkeep_paid && (u.activated || !self.unit_type(u).def.on_offable)
    }

    /// Advanced sensor coverage needs a working, powered unit ([`Sim::working`]). Optical sight
    /// is independent of the on/off switch and upkeep.
    pub fn sensors_active(&self, u: &Unit) -> bool {
        self.working(u)
    }

    /// Stamps every unit's sensors into the per-player coverage grids.
    fn rebuild_vision(&mut self) {
        let mut sensors = Vec::with_capacity(self.units.len() * 2);
        let slots: Vec<u8> = (0..self.players.len() as u8).map(|p| self.vision_slot(p)).collect();
        for u in self.units.iter().flatten().filter(|u| !u.dead) {
            let t = self.types.get(u.type_id);
            let player = slots.get(u.owner as usize).copied().unwrap_or(u.owner);
            let pos = u.pos;
            let complete = u.is_complete();
            // Nanoframes only reveal their immediate surroundings.
            let sight = if complete { t.sight.to_int() } else { t.sight.to_int().min(t.radius.to_int() * 2) };
            // (The eye: about the unit's size above where it stands, flies or floats.)
            let eye = pos.y.to_int() + (t.radius.to_int() + 8).clamp(16, 72);
            sensors.push(Sensor { player, pos, radius: sight.max(t.radius.to_int() + 16), flag: vision::LOS, eye });
            if !self.sensors_active(u) {
                continue;
            }
            for (range, flag) in [
                (t.radar.to_int(), vision::RADAR),
                (t.def.sonar_distance as i64, vision::SONAR),
                (t.def.radar_jam as i64, vision::RADAR_JAM),
                (t.def.sonar_jam as i64, vision::SONAR_JAM),
            ] {
                if range > 0 {
                    sensors.push(Sensor { player, pos, radius: range, flag, eye });
                }
            }
        }
        self.vision.rebuild(&sensors);
    }

    pub fn now_ms(&self) -> i64 {
        self.tick as i64 * 1000 / TICKS_PER_SECOND
    }

    // ---- Spawning ---------------------------------------------------------------------

    /// Creates a unit. `complete = false` starts it as a construction site (1% health).
    pub fn spawn_unit(&mut self, type_id: usize, owner: u8, pos: FxVec3, heading: i32, complete: bool) -> Option<UnitId> {
        // (Nobody's: a player not in the battle owns nothing.)
        if owner as usize >= self.players.len() {
            return None;
        }
        let types = self.types.clone();
        let t = types.get(type_id);
        let slot = match self.free_slots.pop() {
            Some(s) => s,
            None => {
                self.units.push(None);
                self.generations.push(0);
                (self.units.len() - 1) as u32
            }
        };
        self.generations[slot as usize] = self.generations[slot as usize].wrapping_add(1);
        let id = UnitId { slot, generation: self.generations[slot as usize] };
        let mut pos = pos;
        pos.y = self.ground_level(t, pos);

        let mut occupied_origin = None;
        // (One that takes no ground stays just where it is put: a mod's module, where the script
        // of the building that made it looks for it, within a few paces of its pad.)
        if t.locomotion == Locomotion::Static && !t.def.takes_no_ground {
            let anchor = World::pos_to_anchor(pos, t.footprint);
            let origin = World::footprint_origin(anchor, t.footprint);
            self.world.occupy_yard(slot, origin, t.footprint, &t.blocking_cells, &t.open_yard_cells, false);
            self.clear_ground(origin, t.footprint);
            occupied_origin = Some(origin);
            pos = self.world.anchor_to_pos(anchor, t.footprint);
            // (`anchor_to_pos` puts it on the highest ground under the footprint.)
            pos.y = self.ground_level(t, pos).max(pos.y);
        }

        let mut unit = Unit::new(id, type_id, owner, pos);
        unit.rng = crate::unit::unit_rng(self.seed, id);
        unit.heading = wrap_angle(heading);
        unit.health = if complete { t.max_health } else { (t.max_health / 100).max(Fx::ONE) };
        unit.set_build_work(if complete { t.build_work } else { 0 }, t.build_work);
        unit.fire_state = initial_fire_state(&t.def, t.weapons.iter().flatten().any(|&w| self.weapon_types.get(w).def.interceptor));
        unit.move_state = t.def.standing_move_order as u8;
        unit.occupied_origin = occupied_origin;
        unit.cob = t.script.clone().map(CobInstance::new);
        if matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval) {
            Self::settle(&mut unit, t, &self.world);
        }
        self.units[slot as usize] = Some(unit);
        if let Some(p) = self.players.get_mut(owner as usize) {
            p.unit_count += 1;
        }
        self.events.push(SimEvent::UnitCreated { id, type_id });
        if t.def.is_commander && let Some(p) = self.players.get_mut(owner as usize) {
            p.had_commander = true;
        }

        // Run Create immediately so the first rendered frame already has the script's pose.
        self.with_script(id, |cob, host| {
            cob.start("Create", &[], ThreadTag::None);
            cob.run(host);
        });
        // Then the longest reload of its weapons, in milliseconds (turrets rest after
        // twice that).
        if t.callins.set_max_reload_time {
            let reload = t.weapons.iter().flatten().map(|&w| self.weapon_types.get(w).reload_ticks as i64).max().unwrap_or(0);
            self.call_script(id, "SetMaxReloadTime", &[(reload * 1000 / TICKS_PER_SECOND) as i32]);
        }
        if complete {
            self.on_completed(id);
        }
        Some(id)
    }

    /// Where a unit of `type_id` put at `pos` stands, as [`Sim::spawn_unit`] puts it: a building
    /// on its footprint's cells, on the highest ground under them, or at the surface when it
    /// floats or is built in the water (a shipyard); others on the ground, or on the water when
    /// they float. What a placement's ghost shows.
    pub fn resting_pos(&self, type_id: usize, pos: FxVec3) -> FxVec3 {
        let t = self.types.get(type_id);
        let mut pos = pos;
        pos.y = self.ground_level(t, pos);
        if t.locomotion == Locomotion::Static && !t.def.takes_no_ground {
            let anchor = World::pos_to_anchor(pos, t.footprint);
            let mut p = self.world.anchor_to_pos(anchor, t.footprint);
            p.y = self.ground_level(t, p).max(p.y);
            return p;
        }
        pos
    }

    fn ground_level(&self, t: &UnitType, pos: FxVec3) -> Fx {
        let ground = self.world.height_at(pos.x, pos.z);
        match t.locomotion {
            Locomotion::Hover | Locomotion::Naval => ground.max(self.world.sea_level),
            Locomotion::Static if t.def.floater => ground.max(self.world.sea_level),
            // Stock surface water buildings specify WaterLine without Floater; their
            // Create scripts do not lift the root. Underwater plants omit WaterLine.
            // This compatibility rule is inferred from that installed-data distinction.
            Locomotion::Static if t.def.min_water_depth > 0.0 && t.def.water_line > 0.0 => {
                ground.max(self.world.sea_level - Fx::from_f64(t.def.water_line))
            }
            Locomotion::Air => ground.max(self.world.sea_level),
            _ => ground,
        }
    }

    /// Runs `f` with a unit's script instance and a host bound to that unit.
    fn with_script(&mut self, id: UnitId, f: impl FnOnce(&mut CobInstance, &mut dyn CobHost)) {
        let slot = id.slot as usize;
        if self.units.get(slot).and_then(|u| u.as_ref()).is_none_or(|u| u.id != id) {
            return;
        }
        let now_ms = self.now_ms();
        let (ally_masks, max_id) = (self.ally_masks(), self.units.len() as i32);
        // The unit, and the others its script may ask where they are (its own slot reads as no
        // one: a script knows its unit as itself).
        let (before, rest) = self.units.split_at_mut(slot);
        let (me, after) = rest.split_first_mut().expect("the unit's slot");
        let (before, after) = (&*before, &*after);
        let unit = me.as_mut().expect("the unit");
        if let Some(mut cob) = unit.cob.take() {
            let types = &self.types;
            let lookup = |sid: i32| -> Option<OtherUnit> {
                let i = usize::try_from(sid.checked_sub(1)?).ok()?;
                let other = match i.cmp(&slot) {
                    std::cmp::Ordering::Less => before.get(i),
                    std::cmp::Ordering::Greater => after.get(i - slot - 1),
                    std::cmp::Ordering::Equal => None,
                };
                let u = other?.as_ref()?;
                Some(OtherUnit::of(u, types))
            };
            let allies = ally_masks.get(unit.owner as usize).copied().unwrap_or(0);
            let mut host = UnitHost {
                unit_type: types.get(unit.type_id),
                requests: &mut self.host_requests,
                unit,
                world: &self.world,
                events: &mut self.events,
                now_ms,
                lookup: &lookup,
                max_id,
                allies,
            };
            f(&mut cob, &mut host);
            if let Some(u) = self.units[slot].as_mut() {
                u.cob = Some(cob);
            }
        }
        self.apply_host_requests();
    }

    /// Every unit's script for the tick: its pieces' animations advance, then the threads that
    /// are ready run (most sleep most ticks). A script works on its unit alone: its random
    /// numbers are its own, and it reads where the others are, which no script changes. So a
    /// big battle's scripts run on all cores, and what they ask of the rest of the game (effects
    /// and sounds, a transport's loading) is taken up afterwards, unit by unit in slot order.
    fn run_scripts(&mut self, types: &UnitTypes) {
        use rayon::prelude::*;
        let (now, tick) = (self.now_ms(), self.tick);
        // Where everyone is (and whose, and how far built), for scripts asking about another
        // unit.
        let others: Vec<Option<OtherUnit>> = self.units.iter().map(|u| u.as_ref().map(|u| OtherUnit::of(u, types))).collect();
        let (ally_masks, max_id) = (self.ally_masks(), self.units.len() as i32);
        let world = &self.world;
        let run = |(slot, u): (usize, &mut Option<Unit>)| -> Option<(Vec<SimEvent>, Vec<HostRequest>)> {
            let u = u.as_mut()?;
            // Paralyzed units are frozen (passengers' scripts run on).
            if u.carried_by.is_none() && tick < u.paralyzed_until {
                return None;
            }
            let cob = u.cob.as_mut()?;
            cob.animate();
            if !cob.any_ready(now) {
                return None;
            }
            let mut cob = u.cob.take()?;
            let (mut events, mut requests) = (Vec::new(), Vec::new());
            // (A script knows its own unit as itself, not among the others.)
            let lookup = |sid: i32| sid.checked_sub(1).and_then(|s| usize::try_from(s).ok()).filter(|&i| i != slot).and_then(|i| others.get(i).copied().flatten());
            let unit_type = types.get(u.type_id);
            let allies = ally_masks.get(u.owner as usize).copied().unwrap_or(0);
            {
                let mut host = UnitHost { unit_type, requests: &mut requests, unit: &mut *u, world, events: &mut events, now_ms: now, lookup: &lookup, max_id, allies };
                cob.run(&mut host);
            }
            u.cob = Some(cob);
            (!events.is_empty() || !requests.is_empty()).then_some((events, requests))
        };
        let asked: Vec<(Vec<SimEvent>, Vec<HostRequest>)> = if self.parallel && self.units.len() >= PARALLEL_SCRIPTS {
            self.units.par_iter_mut().enumerate().with_min_len(64).filter_map(run).collect()
        } else {
            self.units.iter_mut().enumerate().filter_map(run).collect()
        };
        for (events, requests) in asked {
            self.events.extend(events);
            self.host_requests.extend(requests);
            self.apply_host_requests();
        }
    }

    /// Takes up the script's world changes and activation callbacks in request order.
    fn apply_host_requests(&mut self) {
        if !self.host_requests.is_empty() {
            for r in std::mem::take(&mut self.host_requests) {
                match r {
                    HostRequest::Activation { unit, on } => {
                        if self.unit(unit).is_some_and(|u| !u.dead) {
                            self.start_script(unit, if on { "Activate" } else { "Deactivate" }, &[]);
                        }
                    }
                    HostRequest::Yard { unit, open } => {
                        if let Some(u) = self.unit(unit).filter(|u| !u.dead) && let Some(origin) = u.occupied_origin {
                            let t = self.types.get(u.type_id);
                            self.world.occupy_yard(unit.slot, origin, t.footprint, &t.blocking_cells, &t.open_yard_cells, open);
                        }
                    }
                    HostRequest::Attach { transport, unit, piece } => {
                        if let Some(u) = self.unit_by_script(unit) {
                            // A pickup animation may finish after Stop, a replacement order
                            // or capture. Its old request must not take that passenger anyway.
                            // Repositioning a passenger already aboard remains valid.
                            let allowed = self.unit(u).is_some_and(|cargo| cargo.carried_by == Some(transport))
                                || self.unit(transport).is_some_and(|t| !t.dead && t.orders.front() == Some(&Order::Load(u))
                                    && self.unit(u).is_some_and(|cargo| !cargo.dead && cargo.owner == t.owner))
                                // A unit that carries none takes on what its script attaches of its
                                // own player's, finished and loose: an upgrade module onto the
                                // building that made it (Total Mayhem's geothermal plants and moho
                                // mines, TA: Escalation's fusion reactors and towers).
                                || self.unit(transport).is_some_and(|t| !t.dead && self.types.get(t.type_id).transport_capacity == 0
                                    && self.unit(u).is_some_and(|m| !m.dead && m.is_complete() && m.carried_by.is_none() && m.owner == t.owner));
                            if allowed {
                                let model_piece = self.cob_to_model_piece(transport, piece);
                                self.attach(transport, u, model_piece);
                            }
                        }
                    }
                    HostRequest::Drop { transport, unit } => {
                        if self.unit(transport).is_some_and(|t| !t.dead && matches!(t.orders.front(), Some(Order::Unload(_))))
                            && let Some(u) = self.unit_by_script(unit)
                        {
                            let at = self.carried_position(u).unwrap_or(FxVec3::ZERO);
                            self.detach(transport, u, at);
                        }
                    }
                }
            }
        }
    }

    /// The unit a script refers to by its script id.
    pub(crate) fn unit_by_script(&self, sid: i32) -> Option<UnitId> {
        self.units.get(usize::try_from(sid.checked_sub(1)?).ok()?)?.as_ref().filter(|u| !u.dead).map(|u| u.id)
    }

    /// Model piece for a transport's COB piece (-1 or unmapped = stowed inside).
    fn cob_to_model_piece(&self, transport: UnitId, piece: i32) -> Option<usize> {
        let u = self.unit(transport)?;
        let cob = u.cob.as_ref()?;
        cob.program.piece_map.get(usize::try_from(piece).ok()?).copied().flatten()
    }

    /// Puts `unit` aboard `transport` (or moves it to another attach piece if already aboard).
    pub(crate) fn attach(&mut self, transport: UnitId, unit: UnitId, piece: Option<usize>) {
        if transport == unit || self.unit(transport).is_none() {
            return;
        }
        let Some(u) = self.unit(unit) else { return };
        if u.carried_by.is_some_and(|t| t != transport) {
            return;
        }
        let newly = u.carried_by.is_none();
        if newly {
            self.finish_capture_job(unit);
            self.finish_build_job(unit);
        }
        let Some(u) = self.unit_mut(unit) else { return };
        u.carried_by = Some(transport);
        u.carry_piece = piece;
        if newly {
            u.orders.clear();
            u.repair_trip = false;
            u.path.clear();
            u.path_goal = None;
            u.move_request = None;
            u.idle_anchor = None;
            u.formations.clear();
            u.face = None;
            u.transport_job = None;
            u.hover_alt = None;
            u.speed = Fx::ZERO;
            for w in &mut u.weapons {
                w.target = None;
            }
            if let Some(t) = self.unit_mut(transport) {
                t.cargo.push(unit);
            }
            self.events.push(SimEvent::UnitLoaded { transport, unit });
        }
    }

    /// Where a carried unit hangs right now.
    fn carried_position(&self, unit: UnitId) -> Option<FxVec3> {
        let u = self.unit(unit)?;
        let t = self.unit(u.carried_by?)?;
        let tt = self.types.get(t.type_id);
        Some(match u.carry_piece {
            Some(p) => piece_world_position(t, tt, t.cob.as_ref(), p),
            None => t.pos,
        })
    }

    /// Sets a carried unit down near `at`, on ground it can stand on. Returns false (and
    /// keeps it aboard) when there is none nearby.
    pub(crate) fn detach(&mut self, transport: UnitId, unit: UnitId, at: FxVec3) -> bool {
        let Some(u) = self.unit(unit).filter(|u| u.carried_by == Some(transport)) else { return false };
        let type_id = u.type_id;
        let Some(spot) = self.drop_spot(type_id, at) else { return false };
        let t = self.types.clone();
        let ground = self.ground_level(t.get(type_id), spot);
        if let Some(u) = self.unit_mut(unit) {
            u.carried_by = None;
            u.carry_piece = None;
            u.pos = FxVec3::new(spot.x, ground, spot.z);
            u.path_goal = None;
            u.move_request = None;
        }
        if let Some(t) = self.unit_mut(transport) {
            t.cargo.retain(|&c| c != unit);
        }
        self.events.push(SimEvent::UnitUnloaded { transport, unit });
        true
    }

    /// Nearest spot within a few cells of `at` where a unit of `type_id` can stand.
    fn drop_spot(&self, type_id: usize, at: FxVec3) -> Option<FxVec3> {
        let t = self.types.get(type_id);
        let spec = self.move_specs[type_id];
        let center = World::pos_to_anchor(at, t.footprint);
        for r in 0..=6i32 {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) != r {
                        continue;
                    }
                    let c = (center.0 + dx, center.1 + dz);
                    if self.world.passable(&spec, c, None) {
                        return Some(self.world.anchor_to_pos(c, t.footprint));
                    }
                }
            }
        }
        None
    }

    /// Carried units ride along with their transport (or die with it).
    fn follow_transport(&mut self, id: UnitId) {
        let Some(transport) = self.unit(id).and_then(|u| u.carried_by) else { return };
        let alive = self.unit(transport).is_some_and(|t| !t.dead);
        if !alive {
            let max = self.unit(id).map(|u| self.types.get(u.type_id).max_health).unwrap_or(Fx::ONE);
            self.damage_unit(id, max * 10, id, FxVec3::ZERO);
            // (It goes whatever shields it: a mod's module on the building it upgraded may armour
            // itself untouchable, TA: Escalation's heavy fusion reactor's, `DamageModifier=0`.)
            if let Some(u) = self.unit_mut(id) && !u.dead {
                u.health = Fx::ZERO;
                u.dead = true;
                u.killer = Some(id);
            }
            return;
        }
        let Some(pos) = self.carried_position(id) else { return };
        let heading = self.unit(transport).map_or(0, |t| t.heading);
        if let Some(u) = self.unit_mut(id) {
            u.pos = pos;
            u.heading = heading;
            u.speed = Fx::ZERO;
        }
    }

    /// Transports: fly (or sail) to a unit and take it aboard.
    fn update_load_order(&mut self, id: UnitId, t: &UnitType, target: UnitId) {
        let done = |sim: &mut Sim| {
            sim.stop(id);
            if let Some(u) = sim.unit_mut(id) {
                u.orders.pop_front();
                u.hover_alt = None;
                u.transport_job = None;
            }
        };
        let types = self.types.clone();
        let Some(u) = self.unit(id) else { return };
        let owner = u.owner;
        let full = u.cargo.len() as u32 >= t.transport_capacity;
        let Some(tu) = self.unit(target).filter(|tu| !tu.dead && tu.owner == owner && tu.is_complete()) else {
            done(self);
            return;
        };
        if tu.carried_by == Some(id) {
            done(self);
            return;
        }
        let tt = types.get(tu.type_id);
        if full || tu.carried_by.is_some() || !t.can_carry(tt) {
            self.events.push(SimEvent::OrderFailed { unit: id, reason: "cannot carry that" });
            done(self);
            return;
        }
        let (target_pos, target_top) = (tu.pos, tu.pos.y + tt.model.height);
        if t.locomotion == Locomotion::Air {
            // Hover just above the unit, then hook it (the engine attaches air cargo).
            let hover = target_top + Fx::from_int(8);
            let u = self.unit(id).unwrap();
            let over = u.pos.within_xz(target_pos, Fx::from_int(10));
            let low = (u.pos.y - hover).abs() <= Fx::from_int(6);
            if over && low {
                let piece = self.call_script(id, "QueryTransport", &[0]).and_then(|p| p.first().copied()).unwrap_or(-1);
                self.start_script(id, "BeginTransport", &[tt.model.height.to_cob()]);
                let model_piece = self.cob_to_model_piece(id, piece);
                self.attach(id, target, model_piece);
                done(self);
                return;
            }
            if let Some(u) = self.unit_mut(id) {
                u.hover_alt = over.then_some(hover);
                u.path_goal = Some(target_pos);
                u.move_request = Some(target_pos);
            }
            return;
        }
        // Ships and hovercraft pull up as close as they can get (shallows may stop them
        // short of the shore); the loading script then decides whether its boom reaches.
        // (Booms reach only so far: a scripted transport comes alongside first.)
        let has_script = u.cob.as_ref().is_some_and(|c| c.has_function("TransportPickup"));
        let reach = if has_script { t.radius + tt.radius } else { t.build_distance.max(Fx::from_int(48)) + t.radius + tt.radius };
        let arrived = u.path_goal.is_none() && u.move_request == Some(target_pos);
        if u.transport_job.is_none() && !u.pos.within_xz(target_pos, reach) && !arrived {
            if u.path_goal.is_none() {
                self.move_to(id, t, target_pos, (reach.to_int() / CELL).max(1) as i32);
            }
            return;
        }
        let job = u.transport_job;
        self.stop(id);
        match (has_script, job) {
            (true, None) => {
                let tick = self.tick;
                self.start_tagged(id, "TransportPickup", &[target.script_id()], ThreadTag::Transport);
                if let Some(u) = self.unit_mut(id) {
                    u.transport_job = Some((target, tick));
                }
            }
            (true, Some((_, since))) => {
                if self.tick > since + 30 * 12 || !self.transport_script_running(id) {
                    // The script never took it (out of the boom's reach).
                    self.events.push(SimEvent::OrderFailed { unit: id, reason: "out of reach" });
                    done(self);
                }
            }
            (false, _) => {
                self.attach(id, target, None);
                done(self);
            }
        }
    }

    /// Transports: take cargo to a point and set it down.
    fn update_unload_order(&mut self, id: UnitId, t: &UnitType, dest: FxVec3) {
        let done = |sim: &mut Sim| {
            sim.stop(id);
            if let Some(u) = sim.unit_mut(id) {
                u.orders.pop_front();
                u.hover_alt = None;
                u.transport_job = None;
            }
        };
        let types = self.types.clone();
        let Some(u) = self.unit(id) else { return };
        let Some(&cargo) = u.cargo.last() else {
            done(self);
            return;
        };
        let cargo_height = self.unit(cargo).map_or(Fx::from_int(16), |c| types.get(c.type_id).model.height);
        if t.locomotion == Locomotion::Air {
            let ground = self.world.height_at(dest.x, dest.z).max(self.world.sea_level);
            let hover = ground + cargo_height + Fx::from_int(6);
            let over = u.pos.within_xz(dest, Fx::from_int(10));
            let low = (u.pos.y - hover).abs() <= Fx::from_int(6);
            if over && low {
                if !self.detach(id, cargo, dest) {
                    self.events.push(SimEvent::OrderFailed { unit: id, reason: "no room to unload" });
                    done(self);
                } else if self.unit(id).is_some_and(|u| u.cargo.is_empty()) {
                    self.start_script(id, "EndTransport", &[]);
                    done(self);
                }
                return;
            }
            if let Some(u) = self.unit_mut(id) {
                u.hover_alt = over.then_some(hover);
                u.path_goal = Some(dest);
                u.move_request = Some(dest);
            }
            return;
        }
        let has_script = u.cob.as_ref().is_some_and(|c| c.has_function("TransportDrop"));
        let reach = t.build_distance.max(Fx::from_int(48)) + t.radius + Fx::from_int(24);
        let arrived = u.path_goal.is_none() && u.move_request == Some(dest);
        if u.transport_job.is_none() && !u.pos.within_xz(dest, reach) && !arrived {
            if u.path_goal.is_none() {
                self.move_to(id, t, dest, (reach.to_int() / CELL).max(1) as i32);
            }
            return;
        }
        let job = u.transport_job;
        self.stop(id);
        match (has_script, job) {
            (true, None) => {
                let tick = self.tick;
                self.start_tagged(id, "TransportDrop", &[cargo.script_id(), crate::host::pack_xz(dest.x, dest.z)], ThreadTag::Transport);
                if let Some(u) = self.unit_mut(id) {
                    u.transport_job = Some((cargo, tick));
                }
            }
            (true, Some((unit, since))) => {
                let dropped = self.unit(unit).is_none_or(|c| c.carried_by != Some(id));
                // The script done without letting go (out of its boom's reach), or taking too
                // long: set it down directly.
                if dropped || self.tick > since + 30 * 12 || !self.transport_script_running(id) {
                    if !dropped && !self.detach(id, unit, dest) {
                        self.events.push(SimEvent::OrderFailed { unit: id, reason: "no room to unload" });
                        done(self);
                        return;
                    }
                    if let Some(u) = self.unit_mut(id) {
                        u.transport_job = None;
                    }
                }
            }
            (false, _) => {
                // No script: everyone off at once, spread around the point.
                let all = self.unit(id).map(|u| u.cargo.clone()).unwrap_or_default();
                let mut failed = false;
                for (i, c) in all.into_iter().enumerate() {
                    let offset = crate::fixed::heading_vector((i as i32) * 9362).scale(Fx::from_int(24 * (i as i64 > 0) as i64 + 8 * i as i64));
                    failed |= !self.detach(id, c, dest + offset);
                }
                if failed {
                    self.events.push(SimEvent::OrderFailed { unit: id, reason: "no room to unload" });
                }
                done(self);
            }
        }
    }

    pub(crate) fn start_script(&mut self, id: UnitId, name: &str, args: &[i32]) {
        self.start_tagged(id, name, args, ThreadTag::None);
    }

    fn start_tagged(&mut self, id: UnitId, name: &str, args: &[i32], tag: ThreadTag) {
        if let Some(u) = self.unit_mut(id) && let Some(cob) = &mut u.cob {
            cob.start(name, args, tag);
        }
    }

    /// Whether a transport's loading or unloading script is still at work.
    fn transport_script_running(&self, id: UnitId) -> bool {
        self.unit(id).and_then(|u| u.cob.as_ref()).is_some_and(|c| c.is_running(ThreadTag::Transport))
    }

    pub(crate) fn call_script(&mut self, id: UnitId, name: &str, args: &[i32]) -> Option<Vec<i32>> {
        let mut out = None;
        self.with_script(id, |cob, host| out = cob.call(host, name, args).map(|r| r.params));
        out
    }

    /// A unit finished: its event, and what a unit does as it is finished. Once only: a unit
    /// taken apart by its own side and built back up is not finished again (it was counted,
    /// switched on and given its fire orders already; `Unit::finished`).
    fn on_completed(&mut self, id: UnitId) {
        let Some(u) = self.unit_mut(id).filter(|u| !u.finished) else { return };
        u.finished = true;
        let type_id = u.type_id;
        let t = self.types.get(type_id);
        // (A factory opens when it has work: `UnitType::is_plant`.)
        let activate = t.def.activate_when_built && !t.is_plant();
        // (TA's `NoAutoFire=1` units are set to hold fire as they are finished, whatever they
        // were set to while they were built, anti-nukes excepted: see `initial_fire_state`.)
        let hold_fire = t.def.no_auto_fire && !t.weapons.iter().flatten().any(|&w| self.weapon_types.get(w).def.interceptor);
        self.events.push(SimEvent::UnitCompleted { id, type_id });
        if hold_fire && let Some(u) = self.unit_mut(id) {
            u.fire_state = 0;
        }
        if self.types.get(type_id).def.init_cloaked && let Some(u) = self.unit_mut(id) {
            u.cloak_on = true;
        }
        if let Some(owner) = self.unit(id).map(|u| u.owner) {
            self.built.insert((owner, type_id));
        }
        // Scripts learn what drives their spinning parts before they start them (on
        // Activate): an extractor its rate (metal per second Ã— 500, as TA scales it), a
        // wind generator the wind.
        let t = self.types.clone();
        let t = t.get(type_id);
        if t.callins.set_speed && t.extracts_metal > Fx::ZERO {
            let pos = self.unit(id).map_or(FxVec3::ZERO, |u| u.pos);
            let rate = (self.extraction_at(type_id, pos) * 500).to_int().clamp(0, i32::MAX as i64) as i32;
            self.call_script(id, "SetSpeed", &[rate]);
        }
        if t.wind_generator > Fx::ZERO {
            self.tell_wind(id);
        }
        if activate {
            self.set_active(id, true);
        }
    }

    fn set_active(&mut self, id: UnitId, on: bool) {
        let Some(u) = self.unit_mut(id) else { return };
        if u.activated == on {
            return;
        }
        u.activated = on;
        self.start_script(id, if on { "Activate" } else { "Deactivate" }, &[]);
    }

    // ---- Commands ---------------------------------------------------------------------

    /// `player`'s order to one of its units (`queue`: after its others).
    /// The planes of a group sent at one target together take their places across a front (in
    /// the order the player's selection gives them: 0, 1, -1, 2, -2...; see [`Unit::fan`]), each
    /// other unit, and a plane sent alone, none.
    fn fan_out(&mut self, player: u8, units: &[UnitId]) {
        let types = self.types.clone();
        let mut rank = 0i32;
        for &id in units {
            let Some(u) = self.unit_mut(id).filter(|u| u.owner == player && !u.dead) else { continue };
            if !types.get(u.type_id).is_plane() {
                u.fan = 0;
                continue;
            }
            let place = if rank % 2 == 1 { (rank + 1) / 2 } else { -(rank / 2) };
            u.fan = place.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
            rank += 1;
        }
    }

    pub(crate) fn order_unit(&mut self, player: u8, id: UnitId, order: &Order, queue: bool) {
        if !valid_order_position(order) {
            return;
        }
        let Some(type_id) = self.unit(id).filter(|u| u.owner == player && u.is_complete() && !u.dead).map(|u| u.type_id) else { return };
        if !queue && let Some(u) = self.unit_mut(id) {
            u.fan = 0;
        }
        if let Order::Build { type_id: product, pos } = order {
            let t = self.types.get(type_id);
            if !t.is_builder || !self.is_allowed(*product) || !t.ai_build_options.contains(product) {
                return;
            }
            if self.ceasefire_bars(player, *pos) {
                self.events.push(SimEvent::OrderFailed { unit: id, reason: "too near an enemy base in the ceasefire" });
                return;
            }
        }
        if let Order::Guard(target) = order && self.unit(*target).is_none_or(|u| u.dead || u.id == id || !self.allied(player, u.owner)) {
            return;
        }
        let (factory, fires) = {
            let t = self.types.get(type_id);
            let mut weapons = t.weapons.iter().flatten().map(|&w| self.weapon_types.get(w));
            let fires = match order {
                Order::Attack(_) => weapons.any(|w| !w.def.interceptor && (!w.command_fire || w.def.stockpile)),
                Order::ManualFire(_) => weapons.any(|w| w.command_fire && !w.def.interceptor),
                _ => false,
            };
            (t.is_factory && t.locomotion == Locomotion::Static, fires)
        };
        // Factories keep building whatever they are told: movement orders set the route new
        // units take (queued ones add waypoints). Stop clears that route and empties the
        // production queue; the unit in production is given up and refunded, as removing it
        // from the queue would. An armed one fires when told, as TA's do, what it is making going
        // on: an attack order when it has a gun (Escalation's Big Bertha, Guardian and Sentinel
        // are builders), manual fire when it has a weapon for that (the "buildpad direction" of
        // Total Mayhem's and Escalation's factories turns their pad); Stop takes them back too.
        if factory {
            let Some(u) = self.unit_mut(id) else { return };
            match order {
                Order::Move(_) | Order::Patrol(_) | Order::Fight(_) | Order::Guard(_) => {
                    if !queue {
                        u.rally.clear();
                    }
                    if u.rally.len() < RALLY_MAX {
                        u.rally.push(order.clone());
                    }
                }
                Order::Attack(_) | Order::ManualFire(_) if fires => {
                    if !queue {
                        u.orders.clear();
                    }
                    let pos = u.pos;
                    queue_order(&mut u.orders, order, queue, pos);
                }
                Order::Stop => {
                    u.rally.clear();
                    u.orders.clear();
                    let producing = !u.factory_queue.is_empty();
                    u.factory_queue.clear();
                    if producing {
                        self.cancel_production(id);
                    }
                }
                _ => {}
            }
            return;
        }
        if queue && self.unit(id).is_none_or(|u| !has_order_room(&u.orders, order)) {
            return;
        }
        if !queue {
            self.finish_capture_job(id);
        }
        let Some(u) = self.unit_mut(id) else { return };
        u.idle_anchor = None;
        // A new first order starts afresh: the last one's way is no guide to it (given again to
        // the same place, its "as near as it gets" made a build or reclaim order fail at once).
        if !queue || u.orders.is_empty() {
            u.move_request = None;
        }
        if !queue {
            u.orders.clear();
            u.path.clear();
            u.path_goal = None;
            u.formations.clear();
            u.face = None;
            u.transport_job = None;
            u.hover_alt = None;
            // (A repair trip is called off. In the air it gives up its deck; landed on it, it
            // keeps it until the new order takes it off, as any order does.)
            if std::mem::take(&mut u.repair_trip) && !u.landed {
                u.air_pad = None;
            }
            if u.build.is_some() {
                self.finish_build_job(id);
            }
        }
        let Some(u) = self.unit_mut(id) else { return };
        let pos = u.pos;
        queue_order(&mut u.orders, order, queue, pos);
    }

    fn apply(&mut self, c: &Command) {
        let valid = match c {
            Command::Order { order, .. } => valid_order_position(order),
            Command::Formation { order, shape, .. } => valid_order_position(order) && match shape {
                crate::formation::Shape::Spots(points) => points.iter().all(|p| valid_command_position(*p)),
                _ => true,
            },
            Command::SetRally { pos, .. } => valid_command_position(*pos),
            Command::Cheat { cheat: crate::cheats::Cheat::Spawn { pos, .. }, .. } => valid_command_position(*pos),
            _ => true,
        };
        if !valid {
            return;
        }
        match c {
            Command::Order { player, units, order, queue } => {
                if let Order::Build { type_id, .. } = order && !self.is_allowed(*type_id) {
                    return;
                }
                // Under the rule, a group sent somewhere goes as a formation.
                if self.group_formations && units.len() >= 2 && crate::formation::order_point(order).is_some() {
                    self.form(*player, units, order, *queue, &crate::formation::Shape::Rows { facing: None });
                    return;
                }
                for &id in units {
                    self.order_unit(*player, id, order, *queue);
                }
                if matches!(order, Order::Attack(_)) {
                    self.fan_out(*player, units);
                }
            }
            Command::Formation { player, units, order, queue, shape } => {
                if crate::formation::order_point(order).is_none() {
                    return;
                }
                if self.group_formations {
                    self.form(*player, units, order, *queue, shape);
                } else {
                    // Without the rule, each unit goes to its own place of the shape.
                    let target = crate::formation::order_point(order).unwrap();
                    let plan = self.formation_plan(*player, units, target, shape, *queue);
                    for id in plan.others {
                        self.order_unit(*player, id, order, *queue);
                    }
                    for (id, _, at) in plan.places {
                        let to = match order {
                            Order::Fight(_) => Order::Fight(at),
                            Order::Patrol(_) => Order::Patrol(at),
                            _ => Order::Move(at),
                        };
                        self.order_unit(*player, id, &to, *queue);
                    }
                }
            }
            Command::Produce { player, factory, type_id, count } => {
                let types = self.types.clone();
                let Some(f) = self.unit_mut(*factory) else { return };
                if f.owner != *player || f.dead || !types.get(f.type_id).is_factory || !types.get(f.type_id).ai_build_options.contains(type_id) {
                    return;
                }
                if !self.is_allowed(*type_id) {
                    return;
                }
                let Some(f) = self.unit_mut(*factory) else { return };
                if *count >= 0 {
                    let room = FACTORY_QUEUE_MAX.saturating_sub(f.factory_queue.len());
                    for _ in 0..(*count as usize).min(room) {
                        f.factory_queue.push_back(*type_id);
                    }
                } else {
                    // The last of that kind goes; the one in production (always first) last of
                    // all, and then its build stops.
                    // Keep the earliest matches, including the active frame, until all later
                    // matches have gone. Two bounded scans even for i32::MIN on an empty queue.
                    let matches = f.factory_queue.iter().filter(|t| *t == type_id).count();
                    let mut keep = matches.saturating_sub(count.unsigned_abs() as usize);
                    let cancelled = keep == 0 && f.factory_queue.front() == Some(type_id);
                    f.factory_queue.retain(|t| {
                        if t != type_id {
                            true
                        } else if keep > 0 {
                            keep -= 1;
                            true
                        } else {
                            false
                        }
                    });
                    if cancelled {
                        self.cancel_production(*factory);
                    }
                }
            }
            Command::SetRally { player, factory, pos } => {
                if let Some(f) = self.unit_mut(*factory) && f.owner == *player {
                    f.rally = vec![Order::Move(*pos)];
                }
            }
            Command::SetActive { player, units, on } => {
                for &id in units {
                    if self.unit(id).is_some_and(|u| u.owner == *player && !u.dead && u.is_complete() && self.unit_type(u).def.on_offable) {
                        self.set_active(id, *on);
                    }
                }
            }
            Command::SetCloak { player, units, on } => {
                let types = self.types.clone();
                for &id in units {
                    if let Some(u) = self.unit_mut(id).filter(|u| u.owner == *player) && types.get(u.type_id).def.cloak_cost > 0.0 {
                        u.cloak_on = *on;
                    }
                }
            }
            Command::SetStates { player, units, fire, movement } => {
                for &id in units {
                    if let Some(u) = self.unit_mut(id).filter(|u| u.owner == *player) {
                        if let Some(f) = fire {
                            u.fire_state = (*f).min(2);
                        }
                        if let Some(m) = movement {
                            u.move_state = (*m).min(2);
                        }
                    }
                }
            }
            Command::SelfDestruct { player, units } => {
                if self.players.get(*player as usize).is_none_or(|p| p.defeated) {
                    return;
                }
                let tick = self.tick;
                let mine: Vec<UnitId> = units.iter().copied().filter(|&id| self.unit(id).is_some_and(|u| u.owner == *player && !u.dead)).collect();
                let cancel = !mine.is_empty() && mine.iter().all(|&id| self.unit(id).is_some_and(|u| u.self_destruct_at.is_some()));
                for id in mine {
                    let Some(u) = self.unit_mut(id) else { continue };
                    if cancel {
                        u.self_destruct_at = None;
                        u.self_destruct_manual = false;
                    } else if u.self_destruct_at.is_none() {
                        u.self_destruct_at = Some(tick + SELF_DESTRUCT_TICKS);
                        u.self_destruct_manual = true;
                    }
                }
            }
            Command::SetRepeat { player, factories, on } => {
                for &id in factories {
                    if let Some(u) = self.unit_mut(id).filter(|u| u.owner == *player) {
                        u.factory_repeat = *on;
                    }
                }
            }
            Command::Share { player, to, metal, energy } => {
                let (p, t) = (*player as usize, *to as usize);
                if p == t || !self.allied(*player, *to) || self.players.get(t).is_none_or(|q| q.defeated) || self.players.get(p).is_none_or(|q| q.defeated) {
                    return;
                }
                let give_m = (*metal).max(Fx::ZERO).min(self.players[p].economy.metal);
                let give_e = (*energy).max(Fx::ZERO).min(self.players[p].economy.energy);
                // What the ally cannot store is not taken.
                let room_m = (self.players[t].economy.metal_capacity - self.players[t].economy.metal).max(Fx::ZERO);
                let room_e = (self.players[t].economy.energy_capacity - self.players[t].economy.energy).max(Fx::ZERO);
                let (m, e) = (give_m.min(room_m), give_e.min(room_e));
                self.players[p].economy.metal -= m;
                self.players[p].economy.energy -= e;
                self.players[t].economy.metal += m;
                self.players[t].economy.energy += e;
                if m > Fx::ZERO || e > Fx::ZERO {
                    self.events.push(SimEvent::Shared { from: *player, to: *to, metal: m, energy: e });
                }
            }
            Command::GiveUnits { player, units, to } => {
                let (p, t) = (*player as usize, *to as usize);
                if p == t || !self.allied(*player, *to) || self.players.get(t).is_none_or(|q| q.defeated) || self.players.get(p).is_none_or(|q| q.defeated) {
                    return;
                }
                let mut count = 0;
                for &id in units {
                    let Some(u) = self.unit(id) else { continue };
                    // Not Commanders, passengers (they go with their transport) or units
                    // still in a factory's yard.
                    if u.owner != *player || u.dead || u.carried_by.is_some() || u.factory.is_some() && !u.is_complete() || self.types.get(u.type_id).def.is_commander
                        || self.carries_commander(id)
                    {
                        continue;
                    }
                    let n = self.transfer_size(id);
                    if self.players[t].unit_count.saturating_add(n) > self.unit_limit {
                        // A loaded transport or active factory may not fit while a
                        // later single unit does. Keep considering the selection.
                        continue;
                    }
                    self.change_owner(id, *to);
                    count += n;
                }
                if count > 0 {
                    self.events.push(SimEvent::UnitsGiven { from: *player, to: *to, count });
                }
            }
            Command::Abandon { player } => {
                let heir = (0..self.players.len() as u8).find(|&q| q != *player && self.allied(*player, q) && !self.players[q as usize].defeated);
                if let Some(to) = heir {
                    let units = self.units.iter().flatten().filter(|u| u.owner == *player && !u.dead).map(|u| u.id).collect();
                    self.apply(&Command::GiveUnits { player: *player, units, to });
                }
            }
            Command::QueueFront { player, factory, index } => {
                let Some(f) = self.unit_mut(*factory).filter(|f| f.owner == *player) else { return };
                let front = f.build.is_some() as usize;
                let i = *index as usize;
                if i > front && i < f.factory_queue.len() && let Some(t) = f.factory_queue.remove(i) {
                    f.factory_queue.insert(front, t);
                }
            }
            Command::EditQueue { player, unit, index, type_id, pos, to_front } => {
                let Some(u) = self.unit_mut(*unit).filter(|u| u.owner == *player) else { return };
                match pos {
                    None => {
                        let front = u.build.is_some() as usize;
                        let queue = &u.factory_queue;
                        let Some(j) = nearest_index(queue.len(), *index as usize, |j| j >= front && queue[j] == *type_id) else { return };
                        if let Some(t) = u.factory_queue.remove(j) && *to_front {
                            u.factory_queue.insert(front, t);
                        }
                    }
                    Some(at) => {
                        let orders = &u.orders;
                        let Some(j) = nearest_index(orders.len(), *index as usize, |j| matches!(orders[j], Order::Build { type_id: t, pos: p } if t == *type_id && p == *at)) else { return };
                        // The rest given again as the player gave them, from what they are now.
                        let mut orders: Vec<Order> = u.orders.iter().cloned().collect();
                        let order = orders.remove(j);
                        if *to_front {
                            orders.insert(0, order);
                        }
                        let (player, units) = (*player, vec![*unit]);
                        if orders.is_empty() {
                            self.apply(&Command::Order { player, units: units.clone(), order: Order::Stop, queue: false });
                        }
                        for (i, order) in orders.into_iter().enumerate() {
                            self.apply(&Command::Order { player, units: units.clone(), order, queue: i > 0 });
                        }
                    }
                }
            }
            Command::Cheat { player, cheat } => self.apply_cheat(*player, cheat),
            Command::SetSharing { player, sharing } => {
                if let Some(p) = self.players.get_mut(*player as usize) {
                    p.sharing = *sharing;
                }
            }
            Command::SetShootAll { player, on } => {
                if let Some(p) = self.players.get_mut(*player as usize) {
                    p.shoot_all = *on;
                }
            }
            Command::Signal { player, kind, at } => {
                if self.players.get(*player as usize).is_none_or(|p| p.defeated) {
                    return;
                }
                // (Kept on the map, the last few for a minute and a half.)
                let (w, h) = self.world.size_px();
                let at = FxVec3::new(at.x.max(Fx::ZERO).min(w), Fx::ZERO, at.z.max(Fx::ZERO).min(h));
                let tick = self.tick;
                self.signals.retain(|s| tick < s.tick + crate::signal::LIFE);
                if self.signals.len() >= crate::signal::KEPT {
                    self.signals.remove(0);
                }
                self.signals.push(crate::signal::Signal { player: *player, kind: *kind, at, tick });
                self.events.push(SimEvent::Signal { player: *player, kind: *kind, at });
            }
            Command::Resign { player } => {
                let p = *player as usize;
                if self.players.get(p).is_none_or(|q| q.defeated) {
                    return;
                }
                // In a team game the army goes to an ally (as a player who left); what is left
                // (the Commander, what the ally has no room for) blows up.
                if let Some(to) = (0..self.players.len() as u8).find(|&q| q != *player && self.allied(*player, q) && !self.players[q as usize].defeated) {
                    let units = self.units.iter().flatten().filter(|u| u.owner == *player && !u.dead).map(|u| u.id).collect();
                    self.apply(&Command::GiveUnits { player: *player, units, to });
                }
                self.players[p].defeated = true;
                self.events.push(SimEvent::PlayerDefeated { player: *player });
                let tick = self.tick;
                for u in self.units.iter_mut().flatten().filter(|u| u.owner == *player && !u.dead) {
                    u.self_destruct_at = Some(tick + 20 + (u.id.slot as u64 * 7) % 150);
                }
            }
        }
    }

    // ---- Tick -------------------------------------------------------------------------

    pub fn step(&mut self, commands: &[Command]) {
        self.tick += 1;
        self.path_budget = PATH_BUDGET_PER_TICK;
        self.flow_demand.clear();
        for c in commands {
            self.apply(c);
        }
        if self.tick.is_multiple_of(TICKS_PER_SECOND as u64 * 8) {
            self.change_wind();
        }
        self.economy_begin();
        self.grid.rebuild(self.units.iter().flatten().filter(|u| u.carried_by.is_none()));
        let tv = std::time::Instant::now();
        if (self.tick - 1).is_multiple_of(vision::VISION_INTERVAL) {
            self.rebuild_vision();
        }
        let t0 = std::time::Instant::now();
        self.stats.vision_us += (t0 - tv).as_micros() as u64;
        // Every unit's orders, then the ground units' move (all at once), then every unit's
        // weapons and script: each unit still sees orders, movement, fire and script in turn.
        // (One hold on the types for the whole loop rather than one for each unit.)
        let (types, weapons) = (self.types.clone(), self.weapon_types.clone());
        self.repair_flows();
        let n = self.units.len();
        self.defer_ways = true;
        for slot in 0..n {
            let Some(id) = self.units[slot].as_ref().map(|u| u.id) else { continue };
            self.unit_orders(id, &types);
        }
        self.defer_ways = false;
        self.plan_ways();
        self.pace_formations();
        self.move_ground_units();
        let n = self.units.len();
        for slot in 0..n {
            let Some(id) = self.units[slot].as_ref().map(|u| u.id) else { continue };
            self.unit_actions(id, &types, &weapons);
        }
        let ts = std::time::Instant::now();
        self.run_scripts(&types);
        self.stats.scripts_ns += ts.elapsed().as_nanos() as u64;
        let t1 = std::time::Instant::now();
        self.resolve_collisions();
        let t2 = std::time::Instant::now();
        self.update_interceptors();
        self.update_projectiles();
        self.update_fires();
        self.acid_seas();
        let t3 = std::time::Instant::now();
        self.decay_abandoned_frames();
        self.process_deaths();
        self.check_defeat();
        self.check_mission();
        let t4 = std::time::Instant::now();
        self.economy_end();
        self.sample_stats();
        self.remove_dead();
        let t5 = std::time::Instant::now();
        let us = |a: std::time::Instant, b: std::time::Instant| (b - a).as_micros() as u64;
        self.stats.ticks += 1;
        self.stats.units_us += us(t0, t1);
        self.stats.collisions_us += us(t1, t2);
        self.stats.projectiles_us += us(t2, t3);
        self.stats.deaths_us += us(t3, t4);
        self.stats.economy_us += us(t4, t5);
    }

    /// Defeat: a player without units, or (commander rule) without a commander, loses and
    /// the rest of their base self-destructs over a few seconds, as in TA.
    fn check_defeat(&mut self) {
        // One look at every unit: which players still have units, and a Commander; which units
        // are due to blow up.
        let n = self.players.len();
        let (mut any, mut commander) = (vec![false; n], vec![false; n]);
        let mut due: Vec<UnitId> = Vec::new();
        for u in self.units.iter().flatten().filter(|u| !u.dead) {
            if let Some(a) = any.get_mut(u.owner as usize) {
                *a = true;
                commander[u.owner as usize] |= self.types.get(u.type_id).def.is_commander;
            }
            if u.self_destruct_at.is_some_and(|t| t <= self.tick) {
                due.push(u.id);
            }
        }
        for p in 0..n {
            if self.players[p].defeated || self.players[p].endless {
                continue;
            }
            let owner = p as u8;
            let lost = !any[p] || self.commander_ends_game && self.players[p].had_commander && !commander[p];
            if !lost {
                continue;
            }
            self.players[p].defeated = true;
            self.events.push(SimEvent::PlayerDefeated { player: owner });
            let tick = self.tick;
            for u in self.units.iter_mut().flatten().filter(|u| u.owner == owner && !u.dead) {
                u.self_destruct_at = Some(tick + 20 + (u.id.slot as u64 * 7) % 150);
            }
        }
        // (A defeat just now has put off the countdowns of the loser's units.)
        let tick = self.tick;
        due.retain(|&id| self.unit(id).is_some_and(|u| u.self_destruct_at.is_some_and(|t| t <= tick)));
        for id in due {
            let max = self.unit(id).map(|u| self.types.get(u.type_id).max_health).unwrap_or(Fx::ONE);
            self.damage_unit(id, max * 2, id, FxVec3::ZERO);
        }
        if !self.events.is_empty() {
            self.process_deaths();
        }
    }

    fn change_wind(&mut self) {
        let (lo, hi) = self.wind_range;
        self.wind = lo + (hi - lo) * self.rng.unit_fx();
        self.wind_heading = self.rng.range_i32(0, 65535);
        let types = self.types.clone();
        let generators: Vec<UnitId> = self.units.iter().flatten().filter(|u| !u.dead && u.is_complete() && types.get(u.type_id).wind_generator > Fx::ZERO).map(|u| u.id).collect();
        for id in generators {
            self.tell_wind(id);
        }
    }

    /// A wind generator's blades spin with the wind and it turns to face it (TA's
    /// `SetSpeed`, blade speed in angle units per second, and `SetDirection`).
    fn tell_wind(&mut self, id: UnitId) {
        let Some(t) = self.unit(id).map(|u| self.types.get(u.type_id)) else { return };
        let (speed, direction) = (t.callins.set_speed, t.callins.set_direction);
        if speed {
            let spin = (self.wind * 3000).to_int().clamp(0, i32::MAX as i64) as i32;
            self.call_script(id, "SetSpeed", &[spin]);
        }
        if direction {
            let heading = self.wind_heading;
            self.call_script(id, "SetDirection", &[script_angle(heading)]);
        }
    }

    /// A unit's orders, production and flight: the first part of its tick, before the ground
    /// units move (all at once: `move_ground_units`).
    fn unit_orders(&mut self, id: UnitId, types: &UnitTypes) {
        let Some(u) = self.unit(id) else { return };
        let t = types.get(u.type_id);
        // Passengers ride along; paralyzed units are frozen: no orders, movement, weapons or
        // animation.
        if u.carried_by.is_some() || self.tick < u.paralyzed_until {
            return;
        }
        if u.is_complete() && !u.dead && t.def.kamikaze && (self.tick + id.slot as u64).is_multiple_of(3) {
            self.check_kamikaze(id, t);
        }
        let Some(u) = self.unit(id) else { return };
        if u.is_complete() && !u.dead {
            // (An aircraft on a repair trip holds its orders: `update_air` takes it to its deck.)
            let held = u.repair_trip;
            // Phase timings (diagnostics) on one tick in PHASE_SAMPLE, scaled up: reading the
            // clock for every unit on every tick costs about as much as a phase.
            let ta = self.sampled().then(std::time::Instant::now);
            if !held {
                self.update_orders(id, t);
            }
            if t.is_factory {
                self.update_factory(id, t);
            }
            if t.locomotion == Locomotion::Air {
                self.update_air(id, t);
            }
            if let Some(ta) = ta {
                self.stats.orders_ns += ta.elapsed().as_nanos() as u64 * PHASE_SAMPLE;
            }
        }
    }

    /// Whether this tick's phases are timed (diagnostics): one tick in [`PHASE_SAMPLE`], a
    /// different one of the weapons' retarget cycle each time (always the first, they timed
    /// the same units' target searches every time).
    fn sampled(&self) -> bool {
        self.tick % PHASE_SAMPLE == (self.tick / PHASE_SAMPLE) % PHASE_SAMPLE
    }

    /// The rest of a unit's tick, once everyone has moved: weapons, cloak, air pads (or riding
    /// along in a transport or on a build pad), then its script.
    fn unit_actions(&mut self, id: UnitId, types: &UnitTypes, weapons: &WeaponTypes) {
        let Some(u) = self.unit(id) else { return };
        let t = types.get(u.type_id);
        if u.carried_by.is_some() {
            self.follow_transport(id);
            return;
        }
        if t.locomotion == Locomotion::Air {
            self.follow_air_pad(id);
        }
        let Some(u) = self.unit(id) else { return };
        if self.tick < u.paralyzed_until {
            if let Some(u) = self.unit_mut(id) {
                u.speed = Fx::ZERO;
            }
            return;
        }
        let timed = self.sampled();
        let Some(u) = self.unit(id) else { return };
        if u.is_complete() && !u.dead {
            let tc = timed.then(std::time::Instant::now);
            if t.weapons.iter().any(|w| w.is_some()) {
                self.update_weapons(id, types, weapons);
                if t.stockpile {
                    self.update_stockpile(id);
                }
            }
            if let Some(tc) = tc {
                self.stats.weapons_ns += tc.elapsed().as_nanos() as u64 * PHASE_SAMPLE;
            }
            // (A script may cloak a unit its file gives no cloak cost: `UNIT_CLOAKED`.)
            if t.def.cloak_cost > 0.0 || self.unit(id).is_some_and(|u| u.cloak_on || u.cloaked) {
                self.update_cloak(id, t);
            }
            if t.def.is_airbase {
                self.update_air_base(id, t);
            }
        } else if let Some(fac) = u.factory {
            // Units under construction ride on the factory's build pad.
            self.follow_build_pad(id, fac);
        }
        self.feed_script(id, t);
    }

    /// Every finished ground, hover and naval unit takes its step along its path at once
    /// (`ground_step` touches only the unit it moves); those that got stuck then plan a new
    /// route, in slot order.
    fn move_ground_units(&mut self) {
        let t0 = self.sampled().then(std::time::Instant::now);
        let types = self.types.clone();
        let tick = self.tick;
        // Units give way: each mover's bend round those ahead of it, from where all stood at
        // the start of the tick.
        let mut steers = std::mem::take(&mut self.steers);
        if self.local_avoidance {
            crate::avoid::plan(&mut steers, &self.units, &types, &self.grid, tick, self.parallel);
        } else {
            steers.clear();
        }
        let (specs, world, paces) = (&self.move_specs, &self.world, &self.paces);
        let step = |u: &mut Unit| -> Option<(UnitId, FxVec3)> {
            if u.dead || !u.is_complete() || u.carried_by.is_some() || tick < u.paralyzed_until {
                return None;
            }
            let t = types.get(u.type_id);
            if !matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval) {
                return None;
            }
            let mut steer = steers.get(u.id.slot as usize).copied().unwrap_or_default();
            // Held to its formation's pace.
            if let Some(&Some(pace)) = paces.get(u.id.slot as usize) {
                steer.speed_cap = Some(steer.speed_cap.map_or(pace, |c| c.min(pace)));
            }
            Self::ground_step(u, t, specs[t.id], world, steer, tick).map(|goal| (u.id, goal))
        };
        let replan: Vec<(UnitId, FxVec3)> = if self.parallel && self.units.len() >= PARALLEL_MOVERS {
            use rayon::prelude::*;
            self.units.par_iter_mut().with_min_len(256).filter_map(|u| u.as_mut().and_then(step)).collect()
        } else {
            self.units.iter_mut().filter_map(|u| u.as_mut().and_then(step)).collect()
        };
        self.steers = steers;
        for (id, goal) in replan {
            let Some(type_id) = self.unit(id).map(|u| u.type_id) else { continue };
            let types = self.types.clone();
            self.move_to(id, types.get(type_id), goal, 0);
        }
        if let Some(t0) = t0 {
            self.stats.motion_ns += t0.elapsed().as_nanos() as u64 * PHASE_SAMPLE;
        }
    }

    // ---- Orders -------------------------------------------------------------------------

    /// Air repair pads mend the friendly aircraft landed on their decks (those that took one:
    /// not those parked on the ground beside them), sharing the pad's construction pace between
    /// them (TA repairs cost no resources). A pad switched off mends none.
    fn update_air_base(&mut self, id: UnitId, t: &UnitType) {
        const EVERY: u64 = 5;
        if !(self.tick + id.slot as u64).is_multiple_of(EVERY) {
            return;
        }
        let Some(u) = self.unit(id).filter(|u| u.is_complete() && (u.activated || !t.def.on_offable)) else { return };
        let (pos, owner) = (u.pos, u.owner);
        let reach = Fx::from_int(t.footprint.0.max(t.footprint.1) as i64 * 8 + 24);
        let mut near = std::mem::take(&mut self.scratch);
        self.grid.query(pos, reach + Fx::from_int(32), &mut near);
        let patients: Vec<UnitId> = near
            .iter()
            .filter_map(|&slot| self.units.get(slot as usize).and_then(|o| o.as_ref()))
            .filter(|o| {
                let ot = self.types.get(o.type_id);
                !o.dead && o.is_complete() && o.carried_by.is_none() && o.landed && o.air_pad.is_some_and(|(deck, _)| deck == id)
                    && ot.locomotion == Locomotion::Air && self.allied(owner, o.owner) && o.health < ot.max_health
            })
            .map(|o| o.id)
            .collect();
        self.scratch = near;
        if patients.is_empty() {
            return;
        }
        let rate = self.mutator_work_rate(self.perk_work_rate(id, t.work_rate));
        let share = rate.saturating_mul(EVERY) / patients.len() as u64;
        for p in patients {
            self.repair_health(p, share);
        }
    }

    /// Kamikaze units (mines, crawling bombs) blow up as their SelfDestructAs weapon when an
    /// enemy ground or sea unit comes within `KamikazeDistance` (mines give none: their
    /// own size plus a little).
    fn check_kamikaze(&mut self, id: UnitId, t: &UnitType) {
        let Some(u) = self.unit(id) else { return };
        if u.self_destruct_at.is_some() {
            return;
        }
        let (pos, owner) = (u.pos, u.owner);
        let reach = if t.def.kamikaze_distance > 0.0 { Fx::from_f64(t.def.kamikaze_distance) } else { t.radius + Fx::from_int(24) };
        let mut near = std::mem::take(&mut self.scratch);
        self.grid.query(pos, reach + Fx::from_int(48), &mut near);
        let triggered = near.iter().any(|&slot| {
            self.units.get(slot as usize).and_then(|o| o.as_ref()).is_some_and(|o| {
                let ot = self.types.get(o.type_id);
                !o.dead && o.carried_by.is_none() && ot.locomotion != Locomotion::Air && self.is_enemy(owner, o.owner) && o.pos.within_xz(pos, reach + ot.radius)
            })
        });
        self.scratch = near;
        if triggered {
            let tick = self.tick;
            if let Some(u) = self.unit_mut(id) {
                u.self_destruct_manual = true;
                u.self_destruct_at = Some(tick);
            }
        }
    }

    /// Idle armed units answer fire from beyond their reach (even unseen artillery: they head
    /// for where the shots come from), as TA's standing move orders say: Maneuver goes after
    /// the attacker without straying far and then walks back, Roam follows it anywhere, Hold
    /// Position stays put.
    fn idle_response(&mut self, id: UnitId, t: &UnitType) {
        if t.is_plane() {
            self.idle_plane_response(id, t);
            return;
        }
        if t.locomotion == Locomotion::Static || t.is_plane() || !(self.tick + id.slot as u64).is_multiple_of(10) {
            return;
        }
        let Some(u) = self.unit(id) else { return };
        if !u.orders.is_empty() || u.move_state == 0 || u.fire_state == 0 || u.path_goal.is_some() {
            return;
        }
        let (pos, owner, move_state) = (u.pos, u.owner, u.move_state);
        let armed: Vec<Fx> = t.weapons.iter().flatten().map(|&w| self.weapon_types.get(w)).filter(|w| !w.command_fire).map(|w| w.range).collect();
        if armed.is_empty() {
            return;
        }
        let reach = armed.iter().copied().max().unwrap_or(Fx::ZERO);
        // (Never after one it does not chase, TA's `NoChaseCategory`, nor one none of its weapons
        // takes on of its own accord, its bad targets: a tank does not run after a gunship.)
        let engages = |a: &Unit| {
            !self.types.get(a.type_id).category_mask.intersects(&t.no_chase_mask)
                && (0..3).any(|s| {
                    t.weapons[s].is_some_and(|w| {
                        let w = self.weapon_types.get(w);
                        !w.command_fire && self.engages(t.id, s, w, a)
                    })
                })
        };
        let attacker = u
            .last_attacker
            .filter(|_| self.tick <= u.last_hit_tick + 30)
            .and_then(|a| self.unit(a))
            .filter(|a| self.is_enemy(a.owner, owner) && !a.dead && engages(a))
            .map(|a| a.pos);
        match attacker {
            Some(apos) if !apos.within_xz(pos, reach) => {
                let anchor = u.idle_anchor.unwrap_or(pos);
                let mut dest = apos;
                if move_state == 1 {
                    // Maneuver: only as far as the leash allows, and not at all if even the
                    // end of the leash is out of reach.
                    let leash = Fx::from_int(MANEUVER_LEASH);
                    let off = apos - anchor;
                    let d = off.length_xz();
                    if d > leash + reach {
                        return;
                    }
                    if d > leash {
                        dest = anchor + FxVec3::new(off.x * leash / d, Fx::ZERO, off.z * leash / d);
                    }
                }
                if let Some(u) = self.unit_mut(id) {
                    u.idle_anchor = Some(anchor);
                    u.orders.push_back(Order::Fight(dest));
                }
            }
            _ => {
                // Back to where it stood once the excursion is over.
                let Some(anchor) = u.idle_anchor else { return };
                if let Some(u) = self.unit_mut(id) {
                    u.idle_anchor = None;
                    if move_state == 1 && !u.pos.within_xz(anchor, Fx::from_int(48)) {
                        u.orders.push_back(Order::Move(anchor));
                    }
                }
            }
        }
    }

    fn update_orders(&mut self, id: UnitId, t: &UnitType) {
        if self.unit(id).is_some_and(|u| !matches!(u.orders.front(), Some(Order::Capture(_)))) {
            self.finish_capture_job(id);
        }
        let Some(order) = self.unit(id).and_then(|u| u.orders.front().cloned()) else {
            self.idle_response(id, t);
            return;
        };
        match order {
            Order::Stop => {
                self.stop(id);
                if let Some(u) = self.unit_mut(id) {
                    u.orders.pop_front();
                }
            }
            Order::Move(_) if t.locomotion == Locomotion::Static => {
                // Buildings don't move (factories take movement orders as their rally route).
                if let Some(u) = self.unit_mut(id) {
                    u.orders.pop_front();
                }
            }
            Order::Move(dest) => {
                // (In a formation forming up, it goes to its place there first, and waits.)
                let (goal, forming) = self.formation_goal(id, dest);
                let u = self.unit(id).unwrap();
                let planned = u.move_request == Some(goal);
                let arrived = planned && u.path_goal.is_none() && u.path.is_empty();
                if arrived {
                    if !forming {
                        if let Some(u) = self.unit_mut(id) {
                            u.orders.pop_front();
                            u.move_request = None;
                        }
                        self.arrived(id, dest);
                    }
                } else if !planned {
                    self.move_to(id, t, goal, 0);
                }
            }
            Order::Build { type_id, pos } => self.update_build_order(id, t, type_id, pos),
            Order::Attack(target) | Order::ManualFire(target) => self.update_attack_order(id, target),
            Order::Fight(dest) => self.update_fight_order(id, t, dest),
            Order::Reclaim(feature) => self.update_reclaim_order(id, t, feature),
            Order::Resurrect(feature) => self.update_resurrect_order(id, t, feature),
            Order::ReclaimUnit(target) => self.update_reclaim_unit_order(id, t, target),
            Order::Repair(target) => self.update_repair_order(id, t, target),
            Order::Patrol(dest) => self.update_patrol_order(id, t, dest),
            Order::Guard(target) => self.update_guard_order(id, t, target),
            Order::Capture(target) => self.update_capture_order(id, t, target),
            Order::Wait(n) => {
                if let Some(u) = self.unit_mut(id) {
                    if n <= 1 {
                        u.orders.pop_front();
                    } else if let Some(front) = u.orders.front_mut() {
                        *front = Order::Wait(n - 1);
                    }
                }
            }
            Order::WaitAttacked(watch) => {
                let attacked = match watch {
                    None => self.unit(id).is_some_and(|u| u.last_attacker.is_some()),
                    Some(w) => self.unit(w).is_none_or(|w| w.dead || w.last_attacker.is_some()),
                };
                if attacked && let Some(u) = self.unit_mut(id) {
                    u.orders.pop_front();
                }
            }
            Order::AttackType(type_id) => {
                let Some(u) = self.unit(id) else { return };
                let (pos, owner) = (u.pos, u.owner);
                let target = self
                    .units()
                    .filter(|o| !o.dead && self.is_enemy(o.owner, owner) && o.type_id == type_id)
                    .min_by_key(|o| ((o.pos - pos).xz().length_sq_raw(), o.id.slot))
                    .map(|o| o.id);
                if let Some(u) = self.unit_mut(id) {
                    match target {
                        Some(t) => {
                            if let Some(front) = u.orders.front_mut() {
                                *front = Order::Attack(crate::unit::Target::Unit(t));
                            }
                        }
                        None => {
                            u.orders.pop_front();
                        }
                    }
                }
            }
            Order::Load(target) => self.update_load_order(id, t, target),
            Order::Unload(dest) => self.update_unload_order(id, t, dest),
            Order::SetStates { fire, movement } => {
                if let Some(u) = self.unit_mut(id) {
                    u.fire_state = fire.min(2);
                    u.move_state = movement.min(2);
                    u.orders.pop_front();
                }
            }
        }
    }

    fn update_capture_order(&mut self, id: UnitId, t: &UnitType, target: UnitId) {
        let done = |sim: &mut Sim| {
            sim.finish_capture_job(id);
            sim.finish_build_job(id);
            sim.stop(id);
            if let Some(u) = sim.unit_mut(id) {
                u.orders.pop_front();
                if !u.stance_is_scripted() {
                    u.in_build_stance = false;
                }
            }
        };
        let owner = self.unit(id).map_or(0, |u| u.owner);
        let types = self.types.clone();
        let Some(tu) = self.unit(target).filter(|tu| !tu.dead && tu.carried_by.is_none() && self.is_enemy(tu.owner, owner) && tu.is_complete() && self.seen_by(tu, owner) != Seen::Hidden) else {
            done(self);
            return;
        };
        let tt = types.get(tu.type_id);
        if !t.def.can_capture || tt.def.is_commander || self.carries_commander(target) {
            done(self);
            return;
        }
        // (No taking the enemy's units while the guns are silent.)
        if self.ceasefire_left().is_some() {
            self.events.push(SimEvent::OrderFailed { unit: id, reason: "no captures in the ceasefire" });
            done(self);
            return;
        }
        let target_pos = tu.pos;
        let ticks = tt.capture_ticks(tu.health, tu.kills);
        if self.unit(id).is_none_or(|u| u.capture.is_none_or(|job| job.target != target)) {
            self.finish_capture_job(id);
            if let Some(u) = self.unit_mut(id) {
                u.capture = Some(CaptureJob { target, ticks, elapsed: 0 });
            }
        }
        let u = self.unit(id).unwrap();
        let reach = t.build_distance + t.radius;
        if Self::distance_to_footprint(u.pos, target_pos, tt.footprint) > reach {
            if u.path_goal.is_none() && u.move_request == Some(target_pos) || t.locomotion == Locomotion::Static {
                self.give_up(id, Self::work_key(true, target.slot));
                done(self);
                return;
            }
            let replan = u.path_goal.is_none() || u.stuck_ticks > 60;
            self.finish_build_job(id);
            if replan {
                self.move_to_work(id, t, target_pos, tt.footprint);
            }
            return;
        }
        self.stop(id);
        // (Begun once its job is: a script may have left the stance raised from the last.)
        let started = self.unit(id).is_some_and(|u| u.build.is_some());
        if !started {
            let u = self.unit(id).unwrap();
            let to = target_pos - u.pos;
            let heading = wrap_angle(heading_of(to) - u.heading);
            let pitch = atan2(to.y, to.length_xz().max(Fx::ONE));
            let has_script = u.cob.as_ref().is_some_and(|c| c.has_function("StartBuilding"));
            if let Some(u) = self.unit_mut(id) {
                u.build = Some(BuildJob { target: id, started: true });
                if !has_script {
                    u.in_build_stance = true;
                }
            }
            self.start_script(id, "StartBuilding", &[script_angle(heading), script_angle(pitch)]);
            self.events.push(SimEvent::NanoSpray { builder: id, target });
            return;
        }
        if !self.unit(id).is_some_and(|u| u.in_build_stance) {
            return;
        }
        let job = {
            let Some(job) = self.unit_mut(id).and_then(|u| u.capture.as_mut()) else { return };
            job.elapsed = job.elapsed.saturating_add(1).min(job.ticks);
            *job
        };
        let progress = Fx::fraction(job.elapsed, job.ticks);
        if let Some(tu) = self.unit_mut(target) && progress >= tu.capture_progress {
            tu.capture_work = job.elapsed;
            tu.capture_progress = progress;
        }
        if job.elapsed >= job.ticks {
            // Capturing a factory or transport also transfers its active product/cargo.
            // Keep the finished work until the entire group fits the recipient's limit.
            if self.players[owner as usize].unit_count.saturating_add(self.transfer_size(target)) > self.unit_limit {
                return;
            }
            self.transfer_unit(target, owner);
            done(self);
        }
    }

    fn finish_capture_job(&mut self, id: UnitId) {
        let Some(job) = self.unit_mut(id).and_then(|u| u.capture.take()) else { return };
        let best = self.units().filter(|u| !u.dead)
            .filter(|u| self.unit(job.target).is_some_and(|tu| self.is_enemy(u.owner, tu.owner)))
            .filter_map(|u| u.capture.filter(|c| c.target == job.target))
            .max_by_key(|c| Fx::fraction(c.elapsed, c.ticks));
        if let Some(tu) = self.unit_mut(job.target) {
            tu.capture_work = best.map_or(0, |c| c.elapsed);
            tu.capture_progress = best.map_or(Fx::ZERO, |c| Fx::fraction(c.elapsed, c.ticks));
        }
    }

    /// Hands a unit to another player (capture): it drops its orders and targets.
    pub fn transfer_unit(&mut self, id: UnitId, to: u8) {
        let Some((from, type_id)) = self.unit(id).map(|u| (u.owner, u.type_id)) else { return };
        if from == to {
            return;
        }
        self.change_owner(id, to);
        self.captured.insert((to, type_id));
        self.events.push(SimEvent::UnitCaptured { id, from, to });
    }

    /// Moves a unit to another player's side. It drops its orders, targets, factory queue,
    /// routes and any self-destruct countdown; passengers change sides with their transport,
    /// and a factory finishes the unit in its yard for the new owner.
    fn change_owner(&mut self, id: UnitId, to: u8) {
        // Ground transports can themselves be carried: walk their contents without using
        // the call stack. Changing ownership before visiting children also breaks cycles.
        let mut pending = vec![id];
        while let Some(id) = pending.pop() {
            self.finish_capture_job(id);
            let Some(u) = self.unit_mut(id).filter(|u| !u.dead) else { continue };
            let from = u.owner;
            if from == to {
                continue;
            }
            u.owner = to;
            u.orders.clear();
            u.repair_trip = false;
            u.path.clear();
            u.path_goal = None;
            u.move_request = None;
            u.idle_anchor = None;
            u.capture_progress = Fx::ZERO;
            u.capture_work = 0;
            u.capture = None;
            u.transport_job = None;
            u.hover_alt = None;
            u.factory_queue.clear();
            u.rally.clear();
            u.self_destruct_at = None;
            u.self_destruct_manual = false;
            u.last_attacker = None;
            for w in &mut u.weapons {
                w.target = None;
            }
            let cargo = u.cargo.clone();
            let yard = u.build.as_ref().map(|b| b.target).filter(|&b| self.unit(b).is_some_and(|b| b.factory == Some(id) && b.owner == from));
            // (Its new owner's from now on, for the target searches still to come this tick.)
            self.grid.note_owner(id.slot, to);
            if yard.is_none() {
                self.finish_build_job(id);
            }
            if let Some(p) = self.players.get_mut(from as usize) {
                p.unit_count = p.unit_count.saturating_sub(1);
            }
            if let Some(p) = self.players.get_mut(to as usize) {
                p.unit_count += 1;
            }
            if let Some(b) = yard {
                // Keep the current frame represented in the new owner's queue. Completion
                // removes its front entry; an empty queue would instead consume the first
                // unit the recipient orders before this frame is finished.
                if let Some(type_id) = self.unit(b).map(|b| b.type_id) && let Some(f) = self.unit_mut(id) {
                    f.factory_queue.push_back(type_id);
                }
                pending.push(b);
            }
            for passenger in cargo {
                if self.unit(passenger).is_some_and(|u| u.carried_by == Some(id) && u.owner == from) {
                    pending.push(passenger);
                }
            }
        }
    }

    /// The units changing sides together, including cargo and a factory's current product.
    /// Check the entire group before a gift so it cannot put the recipient past its limit.
    /// Whether a Commander rides in `id` (or in what it carries).
    fn carries_commander(&self, id: UnitId) -> bool {
        let mut pending = vec![id];
        let mut seen = 0;
        while let Some(at) = pending.pop() {
            seen += 1;
            if seen > 256 {
                break;
            }
            let Some(u) = self.unit(at) else { continue };
            for &c in &u.cargo {
                if self.unit(c).is_some_and(|c| c.carried_by == Some(at)) {
                    if self.unit(c).is_some_and(|c| self.types.get(c.type_id).def.is_commander) {
                        return true;
                    }
                    pending.push(c);
                }
            }
        }
        false
    }

    fn transfer_size(&self, id: UnitId) -> u32 {
        let Some(u) = self.unit(id).filter(|u| !u.dead) else { return 0 };
        if u.cargo.is_empty() && u.build.is_none() {
            return 1;
        }
        let owner = u.owner;
        let mut pending = vec![id];
        let mut seen = rustc_hash::FxHashSet::default();
        while let Some(id) = pending.pop() {
            let Some(u) = self.unit(id).filter(|u| !u.dead && u.owner == owner) else { continue };
            if !seen.insert(id) {
                continue;
            }
            pending.extend(u.cargo.iter().copied().filter(|&c| self.unit(c).is_some_and(|c| c.carried_by == Some(id))));
            if let Some(job) = u.build && self.unit(job.target).is_some_and(|b| b.factory == Some(id)) {
                pending.push(job.target);
            }
        }
        seen.len() as u32
    }

    /// Patrol: advance like a fight order; on arrival the point moves to the back of the
    /// queue. Builders stop for nearby work (assist, repair, reclaim) every half second.
    fn update_patrol_order(&mut self, id: UnitId, t: &UnitType, dest: FxVec3) {
        if t.is_builder && t.locomotion != Locomotion::Static && (self.tick + id.slot as u64).is_multiple_of(15) && let Some(work) = self.patrol_work(id, t) {
            self.stop(id);
            if let Some(u) = self.unit_mut(id) {
                u.orders.push_front(work);
            }
            return;
        }
        // (On a short leg the far point counts as reached only near it: otherwise a unit
        // halfway along has reached both ends at once and stands still, turning its orders.)
        let leg = self.unit(id).and_then(|u| match u.orders.back() {
            Some(Order::Patrol(from)) if u.orders.len() > 1 => Some((dest - *from).xz().length() / 3),
            _ => None,
        });
        if self.advance_engaging(id, t, dest, leg) && let Some(u) = self.unit_mut(id) {
            if let Some(o) = u.orders.pop_front() {
                u.orders.push_back(o);
            }
            u.move_request = None;
        }
    }

    /// Identifies a work target: a unit slot or a feature index.
    fn work_key(unit: bool, index: u32) -> u64 {
        (unit as u64) << 32 | index as u64
    }

    /// Records an unreachable work target; explicit orders report the failure, automatic
    /// work picked up while patrolling or guarding fails silently.
    fn give_up(&mut self, id: UnitId, key: u64) {
        let Some(u) = self.unit_mut(id) else { return };
        let automatic = matches!(u.orders.get(1), Some(Order::Patrol(_) | Order::Guard(_)));
        if u.unreachable.len() >= 8 {
            u.unreachable.remove(0);
        }
        u.unreachable.push(key);
        if !automatic {
            self.events.push(SimEvent::OrderFailed { unit: id, reason: "unreachable" });
        }
    }

    /// Something useful for a patrolling builder close to its route: a friendly nanoframe
    /// to help finish, a damaged friendly unit, then a feature worth reclaiming.
    fn patrol_work(&mut self, id: UnitId, t: &UnitType) -> Option<Order> {
        let u = self.unit(id)?;
        let (pos, owner) = (u.pos, u.owner);
        let skip = u.unreachable.clone();
        let range = t.build_distance + t.radius + Fx::from_int(128);
        let mut candidates = std::mem::take(&mut self.scratch);
        self.grid.query(pos, range, &mut candidates);
        let mut best: Option<(u8, i128, UnitId)> = None;
        for &slot in &candidates {
            let Some(o) = self.units.get(slot as usize).and_then(|u| u.as_ref()) else { continue };
            // (Not one being taken apart: work on it would only mend what is left.)
            if o.id == id || o.dead || o.owner != owner || skip.contains(&Self::work_key(true, o.id.slot)) || self.being_taken_apart(o) {
                continue;
            }
            let ot = self.types.get(o.type_id);
            let rank = if !o.is_complete() {
                0
            } else if o.health < ot.max_health && ot.locomotion != Locomotion::Air {
                1
            } else {
                continue;
            };
            let d = (o.pos - pos).xz().length_sq_raw();
            if d <= (range.0 as i128).pow(2) && best.is_none_or(|(br, bd, bid)| (rank, d, o.id) < (br, bd, bid)) {
                best = Some((rank, d, o.id));
            }
        }
        self.scratch = candidates;
        if let Some((_, _, target)) = best {
            return Some(Order::Repair(target));
        }
        if !t.def.can_reclaim {
            return None;
        }
        // Features only right beside the route, so reclaiming doesn't drag the builder away.
        let mut best_feature: Option<(i128, usize)> = None;
        let reach = t.build_distance + t.radius + Fx::from_int(24);
        let r2 = (reach.0 as i128).pow(2);
        let mut near = Vec::new();
        self.feature_grid.near(pos, reach, &mut near);
        for i in near {
            let (i, f) = (i as usize, &self.features[i as usize]);
            if !f.alive || skip.contains(&Self::work_key(false, i as u32)) {
                continue;
            }
            let d = (f.pos - pos).xz().length_sq_raw();
            if d > r2 {
                continue;
            }
            let def = &self.data.features[f.def];
            if def.reclaimable && (def.metal > 0.0 || def.energy > 0.0) && best_feature.is_none_or(|(bd, _)| d < bd) {
                best_feature = Some((d, i));
            }
        }
        best_feature.map(|(_, i)| Order::Reclaim(i))
    }

    /// Guard: follow the unit; builders help with whatever it is building and repair it.
    fn update_guard_order(&mut self, id: UnitId, t: &UnitType, target: UnitId) {
        let owner = self.unit(id).unwrap().owner;
        // Mission scripts can follow an enemy by name/type (Can Rush's jammer follows the
        // player's Fusion Reactor). Player guard commands still require an ally above.
        let scripted = self.runs_mission_script(id);
        let Some(tu) = self.unit(target).filter(|u| !u.dead && target != id && (self.allied(owner, u.owner) || scripted)) else {
            if let Some(u) = self.unit_mut(id) {
                u.orders.pop_front();
            }
            self.stop(id);
            self.finish_build_job(id);
            return;
        };
        let tt = self.types.get(tu.type_id);
        let (target_pos, target_job, needs_work) = (tu.pos, tu.build, self.needs_work(tu, tt));
        let follow = tt.radius + t.radius + Fx::from_int(40);
        if t.is_builder && self.allied(owner, tu.owner) && (self.tick + id.slot as u64).is_multiple_of(8) {
            let assist = target_job.map(|j| j.target).filter(|&b| b != id && self.unit(b).is_some_and(|b| !b.dead && !b.is_complete() && b.carried_by.is_none() && self.allied(owner, b.owner)));
            let work = assist.or(needs_work.then_some(target));
            if let Some(w) = work {
                let reachable = t.locomotion != Locomotion::Static || {
                    let wu = self.unit(w).unwrap();
                    let wt = self.types.get(wu.type_id);
                    let u = self.unit(id).unwrap();
                    Self::distance_to_footprint(u.pos, wu.pos, wt.footprint) <= t.build_distance + t.radius
                };
                if reachable {
                    if let Some(u) = self.unit_mut(id) {
                        u.orders.push_front(Order::Repair(w));
                    }
                    return;
                }
            }
        }
        if t.locomotion == Locomotion::Static {
            return;
        }
        let u = self.unit(id).unwrap();
        let near = u.pos.within_xz(target_pos, follow + Fx::from_int(48));
        if !near {
            // Re-plan now and then while the guarded unit moves.
            let stale = u.path_goal.is_none_or(|g| !g.within_xz(target_pos, follow + Fx::from_int(64)));
            if u.path_goal.is_none() || stale && (self.tick + id.slot as u64).is_multiple_of(20) {
                let radius = (follow.to_int() / CELL).max(1) as i32;
                self.move_to(id, t, target_pos, radius);
            }
        } else if u.path_goal.is_some() && u.pos.within_xz(target_pos, follow) {
            self.stop(id);
        }
    }

    /// Paths towards a point (used by attack orders).
    pub(crate) fn move_to_point(&mut self, id: UnitId, dest: FxVec3) {
        let types = self.types.clone();
        let Some(u) = self.unit(id) else { return };
        let t = types.get(u.type_id);
        self.move_to(id, t, dest, 0);
    }

    pub(crate) fn halt(&mut self, id: UnitId) {
        self.stop(id);
    }

    fn stop(&mut self, id: UnitId) {
        if let Some(u) = self.unit_mut(id) {
            u.path.clear();
            u.path_goal = None;
            u.move_request = None;
        }
    }

    /// Plans a path towards `dest`. `radius` (cells) accepts nearby goals (builders).
    /// Unreachable destinations are replaced by the closest cell in the unit's own region.
    /// While the units have their orders, a ground unit's way to a place (`radius` 0) waits
    /// until they all have (`Sim::plan_ways`): those sent off together are planned together.
    fn move_to(&mut self, id: UnitId, t: &UnitType, dest: FxVec3, radius: i32) {
        let spec = self.move_specs[t.id];
        let Some((unit_pos, unit_factory)) = self.unit(id).map(|u| (u.pos, u.factory)) else { return };
        if t.locomotion == Locomotion::Air {
            let (w, h) = self.world.size_px();
            let goal = FxVec3::new(dest.x.clamp(Fx::ZERO, w), dest.y, dest.z.clamp(Fx::ZERO, h));
            if let Some(u) = self.unit_mut(id) {
                u.path = Vec::new();
                u.path_goal = Some(goal);
                u.move_request = Some(dest);
                u.path_partial = false;
                u.new_route();
            }
            return;
        }
        if radius == 0 && unit_factory.is_none() && self.defer_ways {
            // In a formation, its way to its place leaves the group's field as early as it can.
            let far = self.in_formation(id, dest);
            self.way_requests.push(WayRequest { id, spec, footprint: t.footprint, from: unit_pos, dest, far });
            return;
        }
        let aim = self.aim(t, spec, unit_pos, unit_factory, dest, radius);
        self.plan_way(id, t, spec, dest, radius, unit_factory, aim, true);
    }

    /// Where a unit's way to `dest` runs: from its anchor cell to the goal's, or to the nearest
    /// cell of its own region when that is out of it, off occupied cells and (`radius` 0) out
    /// of pockets walled in by trees; and how far a search for it may look.
    fn aim(&mut self, t: &UnitType, spec: MoveSpec, unit_pos: FxVec3, unit_factory: Option<UnitId>, dest: FxVec3, radius: i32) -> Aim {
        let (start, goal, region) = self.aim_cells(spec, t.footprint, unit_pos, dest);
        let near = (region != 0 && self.world.component(&spec, goal) != region).then(|| self.region_near(spec, region, goal));
        let mut aim = self.aim_free(spec, t.footprint, (start, goal, region), near, unit_factory, dest);
        if radius == 0 {
            let open = self.open_goal(spec, aim.goal);
            self.aim_open(&mut aim, open, t.footprint);
        }
        aim
    }

    /// [`Sim::aim`] for the ways put off (to a place, from no factory), its searches for the
    /// nearest cell of a region on all cores first, each once; but for pockets (see
    /// [`Sim::unpocket`]).
    fn aims(&mut self, requests: &[WayRequest]) -> Vec<Aim> {
        use rayon::prelude::*;
        let started = std::time::Instant::now();
        let parallel = self.parallel;
        let cells: Vec<(GridCell, GridCell, Region)> = requests.iter().map(|r| self.aim_cells(r.spec, r.footprint, r.from, r.dest)).collect();
        let outside = |r: &WayRequest, &(_, goal, region): &(GridCell, GridCell, Region)| region != 0 && self.world.component(&r.spec, goal) != region;
        let mut asked = rustc_hash::FxHashSet::default();
        let wanted: Vec<(MoveSpec, Region, GridCell)> = requests
            .iter()
            .zip(&cells)
            .filter(|&(r, c)| outside(r, c))
            .map(|(r, &(_, goal, region))| (r.spec, region, goal))
            .filter(|k| !self.reachable_near.contains_key(k) && asked.insert(*k))
            .collect();
        let world = &self.world;
        let look = |&(spec, region, goal): &(MoveSpec, Region, GridCell)| world.nearest_in_component(&spec, region, goal, 96);
        let found: Vec<Option<GridCell>> = if parallel && wanted.len() > 1 { wanted.par_iter().map(look).collect() } else { wanted.iter().map(look).collect() };
        let near: rustc_hash::FxHashMap<(MoveSpec, Region, GridCell), Option<GridCell>> = wanted.into_iter().zip(found).collect();
        let aims: Vec<Aim> = requests
            .iter()
            .zip(&cells)
            .map(|(r, c)| {
                let n = outside(r, c).then(|| {
                    let key = (r.spec, c.2, c.1);
                    near.get(&key).copied().unwrap_or_else(|| self.reachable_near[&key])
                });
                self.aim_free(r.spec, r.footprint, *c, n, None, r.dest)
            })
            .collect();
        for (key, n) in near {
            self.remember_near(key, n);
        }
        self.stats.path_us += started.elapsed().as_micros() as u64;
        aims
    }

    /// The last of [`Sim::aim`] for some of the ways put off (`which`, by index): goals walled
    /// in by trees, rocks or buildings aimed at from outside them, each looked at once, on all
    /// cores. (A goal a group's field reaches is in no pocket: those are left alone.)
    fn unpocket(&mut self, requests: &[WayRequest], aims: &mut [Aim], which: &[usize]) {
        use rayon::prelude::*;
        if which.is_empty() {
            return;
        }
        let started = std::time::Instant::now();
        let version = self.world.occupancy_version;
        let mut asked = rustc_hash::FxHashSet::default();
        let wanted: Vec<(MoveSpec, GridCell)> = which
            .iter()
            .map(|&i| (requests[i].spec, aims[i].goal))
            .filter(|k| self.open_goals.get(k).is_none_or(|&(v, _)| v != version) && asked.insert(*k))
            .collect();
        let world = &self.world;
        let look = |&(spec, goal): &(MoveSpec, GridCell)| crate::path::open_goal(world, &spec, goal);
        let found: Vec<GridCell> = if self.parallel && wanted.len() > 1 { wanted.par_iter().map(look).collect() } else { wanted.iter().map(look).collect() };
        let opened: rustc_hash::FxHashMap<(MoveSpec, GridCell), GridCell> = wanted.into_iter().zip(found).collect();
        for &i in which {
            let key = (requests[i].spec, aims[i].goal);
            let open = opened.get(&key).copied().unwrap_or_else(|| self.open_goals[&key].1);
            let mut aim = aims[i];
            self.aim_open(&mut aim, open, requests[i].footprint);
            aims[i] = aim;
        }
        for (key, open) in opened {
            self.remember_open_goal(key, version, open);
        }
        self.stats.path_us += started.elapsed().as_micros() as u64;
    }

    /// A unit's anchor cell, its goal's, and its region.
    fn aim_cells(&self, spec: MoveSpec, footprint: (u32, u32), unit_pos: FxVec3, dest: FxVec3) -> (GridCell, GridCell, Region) {
        let start = World::pos_to_anchor(unit_pos, footprint);
        (start, World::pos_to_anchor(dest, footprint), self.world.component(&spec, start))
    }

    /// The nearest cell of `region` to `goal` (96 cells off at most), remembered: static
    /// terrain, so the answer never goes stale.
    fn region_near(&mut self, spec: MoveSpec, region: Region, goal: GridCell) -> Option<GridCell> {
        if let Some(&n) = self.reachable_near.get(&(spec, region, goal)) {
            return n;
        }
        let tp = std::time::Instant::now();
        let n = self.world.nearest_in_component(&spec, region, goal, 96);
        self.stats.path_us += tp.elapsed().as_micros() as u64;
        self.remember_near((spec, region, goal), n);
        n
    }

    fn remember_near(&mut self, key: (MoveSpec, Region, GridCell), near: Option<GridCell>) {
        if self.reachable_near.len() >= 8192 {
            self.reachable_near.clear();
        }
        self.reachable_near.insert(key, near);
    }

    /// The goal moved into the unit's region (`near`: when it was out of it, the nearest cell
    /// of it, if any: else a short search) and off an occupied cell.
    fn aim_free(&self, spec: MoveSpec, footprint: (u32, u32), (start, mut goal, region): (GridCell, GridCell, Region), near: Option<Option<GridCell>>, unit_factory: Option<UnitId>, dest: FxVec3) -> Aim {
        let mut goal_pos = dest;
        let mut max_expansions = PATH_EXPANSIONS;
        match near {
            Some(Some(g)) => {
                goal = g;
                goal_pos = self.world.anchor_to_pos(g, footprint);
            }
            Some(None) => max_expansions = 2_000,
            // Standing somewhere impassable (e.g. inside a factory yard): short search.
            None if region == 0 => max_expansions = 4_000,
            None => {}
        }
        // Goals on occupied cells (an enemy building, a wreck) can never be entered:
        // aim for the nearest free cell instead of letting A* exhaust its budget.
        if !self.world.passable(&spec, goal, unit_factory.map(|f| f.slot)) {
            'free: for r in 1i32..=12 {
                for dz in -r..=r {
                    for dx in -r..=r {
                        if dx.abs() != r && dz.abs() != r {
                            continue;
                        }
                        let c = (goal.0 + dx, goal.1 + dz);
                        if (region == 0 || self.world.component(&spec, c) == region) && self.world.passable(&spec, c, None) {
                            goal = c;
                            goal_pos = self.world.anchor_to_pos(c, footprint);
                            break 'free;
                        }
                    }
                }
            }
        }
        Aim { start, goal, goal_pos, region, max_expansions }
    }

    /// A goal walled in by trees, rocks or buildings is aimed at from outside them (`open`, as
    /// `path::open_goal` has it).
    fn aim_open(&self, aim: &mut Aim, open: GridCell, footprint: (u32, u32)) {
        if open != aim.goal {
            aim.goal = open;
            aim.goal_pos = self.world.anchor_to_pos(open, footprint);
        }
    }

    /// Plans a unit's way as [`Sim::aim`] aimed it: off a shared distance field (`use_flow`:
    /// the one to its goal, or one to a place near it a group shares), a path planned from the
    /// same block, or a search.
    #[allow(clippy::too_many_arguments)]
    fn plan_way(&mut self, id: UnitId, t: &UnitType, spec: MoveSpec, dest: FxVec3, radius: i32, unit_factory: Option<UnitId>, aim: Aim, use_flow: bool) {
        let Aim { start, goal, mut goal_pos, region, max_expansions } = aim;
        let mut partial = false;
        let key = (spec, (start.0 >> 2, start.1 >> 2), goal, radius);
        // Cells opening up never invalidate a route; only new obstacles do.
        let version = self.world.blocking_version;
        // Big groups share one distance field to the destination.
        let mut flow_route = None;
        if use_flow && radius == 0 && unit_factory.is_none() {
            let fkey = (spec, goal);
            let (demand, starts) = {
                let d = self.flow_demand.entry(fkey).or_insert((0, (start.0, start.1, start.0, start.1)));
                d.0 += 1;
                d.1 = (d.1.0.min(start.0), d.1.1.min(start.1), d.1.2.max(start.0), d.1.3.max(start.1));
                *d
            };
            let tick = self.tick;
            let cached = self.current_flow(fkey);
            let field = match cached {
                // A field that doesn't cover this start is widened once a group needs it
                // (at most once a tick); until then the unit searches.
                Some(f) if f.covers(start) || demand < FLOW_MIN_GROUP || f.tick == tick => Some(f),
                cached if demand >= FLOW_MIN_GROUP => {
                    let tp = std::time::Instant::now();
                    let (from, to) = flow_window(&self.world, goal, starts, cached.map(|f| f.window()));
                    let f = Arc::new(FlowField::build_within(&self.world, &spec, goal, tick, from, to));
                    self.stats.flow_builds += 1;
                    let us = tp.elapsed().as_micros() as u64;
                    self.stats.path_us += us;
                    self.stats.flow_us += us;
                    self.keep_flow(fkey, f.clone());
                    Some(f)
                }
                _ => None,
            };
            // Else the one a group sent to places round a point near the goal shares.
            let field = match field {
                Some(f) if f.covers(start) => Some(f),
                field => self.nearby_flow(spec, start, goal).or(field),
            };
            let tr = std::time::Instant::now();
            flow_route = field.and_then(|f| f.route_to(start, goal));
            let us = tr.elapsed().as_micros() as u64;
            self.stats.path_us += us;
            self.stats.route_us += us;
            if flow_route.is_some() {
                self.stats.flow_routes += 1;
            }
        }
        // (A path planned from another cell of the block serves only if its first waypoint is
        // in a clear line from this one: a tree off to the side can be in the way.)
        let cached = self
            .path_cache
            .get(&key)
            .filter(|c| c.occupancy_version == version && self.tick < c.tick + PATH_CACHE_TICKS && unit_factory.is_none())
            .filter(|c| c.path.first().map_or(start == goal, |&w| crate::path::line_clear(&self.world, &spec, start, w, None)));
        let path = if let Some(route) = flow_route {
            route
        } else if let Some(c) = cached {
            // The route is shared by anchor cell; a successful route still ends at
            // this unit's exact requested point, not its neighbour's point in that cell.
            if c.path.last().copied().unwrap_or(start) != goal { goal_pos = c.goal_pos; }
            c.path.clone()
        } else if self.path_budget < PATH_MIN_SEARCH {
            // Out of search budget this tick: leave the request unplanned (retried).
            return;
        } else {
            let max_expansions = max_expansions.min(self.path_budget as usize);
            let q = PathQuery { spec, start, goal, goal_radius: radius, ignore: unit_factory.map(|f| f.slot), max_expansions };
            let tp = std::time::Instant::now();
            let (path, reached, expanded) = self.pathfinder.find(&self.world, &q);
            self.path_budget = self.path_budget.saturating_sub(expanded as u32);
            self.stats.path_searches += 1;
            self.stats.path_us += tp.elapsed().as_micros() as u64;
            // A goal in the unit's own region that the search gave up on: a long detour
            // (round a ridge from its foot, through a forest) floods more ground than the
            // allowance. A distance field to the goal finds the way, and is kept for whoever
            // goes there next (one such build a tick): over a wide window round the unit,
            // the others asking for the goal this tick and the goal, where most detours
            // lie (a fraction of a big map's cells, and of the work of keeping it up to date
            // as wrecks fall), and over the whole map when that one doesn't reach.
            if !reached && expanded >= max_expansions && max_expansions == PATH_EXPANSIONS && radius == 0 && region != 0 && self.world.component(&spec, goal) == region && self.path_budget > 0 {
                let tp = std::time::Instant::now();
                let starts = self.flow_demand.get(&(spec, goal)).map_or((start.0, start.1, start.0, start.1), |d| d.1);
                let (from, to) = detour_window(&self.world, goal, starts);
                let mut field = FlowField::build_within(&self.world, &spec, goal, self.tick, from, to);
                let mut route = field.route(start);
                if route.is_none() {
                    field = FlowField::build(&self.world, &spec, goal, self.tick);
                    route = field.route(start);
                }
                self.path_budget = 0;
                self.stats.flow_builds += 1;
                let us = tp.elapsed().as_micros() as u64;
                self.stats.path_us += us;
                self.stats.flow_us += us;
                self.keep_flow((spec, goal), Arc::new(field));
                if let Some(route) = route {
                    self.stats.flow_routes += 1;
                    return self.set_path(id, route, dest, false);
                }
            }
            // Partial only when this tick's budget (not the normal search allowance)
            // stopped the search; genuinely unreachable goals end at the closest point.
            partial = !reached && !path.is_empty() && expanded >= max_expansions && max_expansions < PATH_EXPANSIONS;
            if partial {
                // Don't share partial paths: other units would stop short too.
                return self.set_path(id, path, dest, true);
            }
            if !reached {
                // A sealed pocket may have no route at all. Finish here rather than
                // driving straight at the inaccessible destination and retrying forever.
                let here = self.unit(id).map_or(goal_pos, |u| u.pos);
                goal_pos = path.last().map_or(here, |&c| self.world.anchor_to_pos(c, t.footprint));
            }
            if self.path_cache.len() > self.path_cache_limit {
                // (Only stale routes go, which are never used. Looked at again once the cache
                // has doubled, not after every new route while it is full of fresh ones.)
                let now = self.tick;
                self.path_cache.retain(|_, c| now < c.tick + PATH_CACHE_TICKS);
                self.path_cache_limit = (self.path_cache.len() * 2).max(PATH_CACHE_PURGE);
            }
            self.path_cache.insert(key, CachedPath { path: path.clone(), goal_pos, tick: self.tick, occupancy_version: version });
            path
        };
        if radius > 0 && let Some(&c) = path.last() {
            goal_pos = self.world.anchor_to_pos(c, t.footprint);
        }
        if let Some(u) = self.unit_mut(id) {
            u.path = path;
            u.path_goal = Some(goal_pos);
            u.move_request = Some(dest);
            u.path_partial = partial;
            u.new_route();
        }
    }

    /// Plans the ways put off while the units had their orders (see `move_to`). Those of one
    /// movement class heading to places close together (each within [`GROUP_REACH`] cells of
    /// the first's) are a group: a selection sent off with each unit to its own place round the
    /// point, an army sent to attack. One of [`FLOW_MIN_GROUP`] or more shares a distance field
    /// to the place nearest the middle of theirs, which each leaves for its own
    /// (`FlowField::route_to`). The fields are built, and the routes read off them, on all
    /// cores ([`GROUP_BUILDS`] fields a tick: the groups beyond wait a tick); the others' ways
    /// are planned one by one, in the order asked for.
    fn plan_ways(&mut self) {
        use rayon::prelude::*;
        let requests = std::mem::take(&mut self.way_requests);
        if requests.is_empty() {
            return;
        }
        let mut aims = self.aims(&requests);
        let tick = self.tick;
        let cheb = |a: GridCell, b: GridCell| (a.0 - b.0).abs().max((a.1 - b.1).abs());
        let take_in = |b: CellBounds, c: GridCell| (b.0.min(c.0), b.1.min(c.1), b.2.max(c.0), b.3.max(c.1));
        // Each request joins the first group (in the order they were made) whose first goal is
        // within reach of its own, looked for in the squares round that; or makes a new one.
        let mut groups: Vec<WayGroup> = Vec::new();
        let mut squares: rustc_hash::FxHashMap<(MoveSpec, GridCell), Vec<usize>> = Default::default();
        let solo: Vec<bool> = requests.iter().map(|r| self.alone.get(&r.id).is_some_and(|&(dest, since)| dest == r.dest && tick < since + FLOW_TICKS)).collect();
        for (i, (r, aim)) in requests.iter().zip(&aims).enumerate() {
            if solo[i] {
                continue;
            }
            let goal = aim.goal;
            let square = (goal.0.div_euclid(GROUP_REACH), goal.1.div_euclid(GROUP_REACH));
            let mut found: Option<usize> = None;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    for &g in squares.get(&(r.spec, (square.0 + dx, square.1 + dz))).into_iter().flatten() {
                        if cheb(groups[g].first, goal) <= GROUP_REACH && found.is_none_or(|f| g < f) {
                            found = Some(g);
                        }
                    }
                }
            }
            match found {
                Some(g) => {
                    let group = &mut groups[g];
                    group.members.push(i);
                    group.bounds = take_in(take_in(group.bounds, goal), aim.start);
                }
                None => {
                    squares.entry((r.spec, square)).or_default().push(groups.len());
                    groups.push(WayGroup { spec: r.spec, first: goal, members: vec![i], bounds: take_in((goal.0, goal.1, goal.0, goal.1), aim.start) });
                }
            }
        }
        // Each group's field: one kept that covers them all, to a place near the middle of their
        // goals (groups form again as the units left over ask again); else one to the place
        // nearest that middle (the first such; out of its pocket): the one kept when it covers
        // them all, else (a group big enough) a new one, or next tick when this one's are all
        // made.
        let mut fields: Vec<Option<Arc<FlowField>>> = vec![None; groups.len()];
        let mut waiting = vec![false; groups.len()];
        let mut builds: Vec<(usize, GridCell, (GridCell, GridCell))> = Vec::new();
        for (g, group) in groups.iter().enumerate() {
            let n = group.members.len() as i64;
            let (sx, sz) = group.members.iter().fold((0i64, 0i64), |(x, z), &i| (x + aims[i].goal.0 as i64, z + aims[i].goal.1 as i64));
            let middle = (sx.div_euclid(n), sz.div_euclid(n));
            let b = group.bounds;
            let kept = self
                .flow_cache
                .iter()
                .filter(|(k, f)| k.0 == group.spec && tick < f.tick + FLOW_TICKS && cheb(k.1, (middle.0 as i32, middle.1 as i32)) <= GROUP_REACH && f.covers((b.0, b.1)) && f.covers((b.2, b.3)))
                .min_by_key(|(k, _)| (cheb(k.1, (middle.0 as i32, middle.1 as i32)), k.1.1, k.1.0))
                .map(|(k, _)| *k);
            if let Some(f) = kept.and_then(|key| self.current_flow(key)) {
                fields[g] = Some(f);
                continue;
            }
            let goal = group
                .members
                .iter()
                .map(|&i| aims[i].goal)
                .min_by_key(|&c| (c.0 as i64 - middle.0).pow(2) + (c.1 as i64 - middle.1).pow(2))
                .unwrap();
            let goal = self.open_goal(group.spec, goal);
            let b = take_in(b, goal);
            let cached = self.current_flow((group.spec, goal));
            // (A formation's units share a field however few: their ways side by side.)
            let big = group.members.len() as u32 >= FLOW_MIN_GROUP || group.members.iter().any(|&i| requests[i].far);
            match cached {
                Some(f) if f.covers((b.0, b.1)) && f.covers((b.2, b.3)) || f.tick == tick => fields[g] = Some(f),
                cached if big && builds.len() < GROUP_BUILDS => builds.push((g, goal, flow_window(&self.world, goal, b, cached.map(|f| f.window())))),
                _ if big => waiting[g] = true,
                cached => fields[g] = cached,
            }
        }
        if !builds.is_empty() {
            let started = std::time::Instant::now();
            let world = &self.world;
            let build = |&(g, goal, (from, to)): &(usize, GridCell, (GridCell, GridCell))| FlowField::build_within(world, &groups[g].spec, goal, tick, from, to);
            let made: Vec<FlowField> = if self.parallel && builds.len() > 1 { builds.par_iter().map(build).collect() } else { builds.iter().map(build).collect() };
            for (&(g, goal, _), field) in builds.iter().zip(made) {
                let field = Arc::new(field);
                self.keep_flow((groups[g].spec, goal), field.clone());
                fields[g] = Some(field);
            }
            let us = started.elapsed().as_micros() as u64;
            self.stats.flow_builds += builds.len() as u64;
            self.stats.path_us += us;
            self.stats.flow_us += us;
        }
        // Out of their pockets: those planned alone, and those whose goal their group's field
        // doesn't reach.
        let pocketed: Vec<usize> = {
            let aims = &aims;
            let mut v: Vec<usize> = groups
                .iter()
                .zip(&fields)
                .zip(&waiting)
                .filter(|&(_, &wait)| !wait)
                .flat_map(|((group, f), _)| group.members.iter().copied().filter(move |&i| f.as_ref().is_none_or(|f| !f.reaches(aims[i].goal))))
                .chain((0..requests.len()).filter(|&i| solo[i]))
                .collect();
            v.sort_unstable();
            v
        };
        self.unpocket(&requests, &mut aims, &pocketed);
        // The routes off them, on all cores.
        let jobs: Vec<(usize, &FlowField)> = groups.iter().zip(&fields).filter_map(|(group, f)| f.as_deref().map(|f| (group, f))).flat_map(|(group, f)| group.members.iter().map(move |&i| (i, f))).collect();
        let started = std::time::Instant::now();
        let route = |&(i, f): &(usize, &FlowField)| f.route_to_with(aims[i].start, aims[i].goal, requests[i].far);
        let routes: Vec<Option<Vec<GridCell>>> = if self.parallel && jobs.len() >= PARALLEL_ROUTES { jobs.par_iter().map(route).collect() } else { jobs.iter().map(route).collect() };
        let us = started.elapsed().as_micros() as u64;
        self.stats.path_us += us;
        self.stats.route_us += us;
        let mut ways: Vec<Option<Option<Vec<GridCell>>>> = vec![None; requests.len()];
        for (&(i, _), route) in jobs.iter().zip(routes) {
            ways[i] = Some(route);
        }
        // A field that misses a good part of its group (their way round lies outside its window)
        // is made again over a wider one (as a search that gave up gets: `detour_window`), once,
        // and they try again. (A few that it misses are cut off where they stand, mostly.)
        let wider: Vec<(usize, (GridCell, GridCell))> = (0..groups.len())
            .filter_map(|g| {
                let f = fields[g].as_ref()?;
                let missed = groups[g].members.iter().filter(|&&i| matches!(ways[i], Some(None))).count();
                let window = detour_window(&self.world, f.goal, groups[g].bounds);
                let (now, wide) = (f.window(), window);
                let wider = wide.0.0 < now.0.0 || wide.0.1 < now.0.1 || wide.1.0 > now.1.0 || wide.1.1 > now.1.1;
                (wider && missed as u32 >= FLOW_MIN_GROUP && missed * 8 >= groups[g].members.len()).then_some((g, window))
            })
            .take(GROUP_BUILDS)
            .collect();
        if !wider.is_empty() {
            let started = std::time::Instant::now();
            let world = &self.world;
            let build = |&(g, (from, to)): &(usize, (GridCell, GridCell))| fields[g].as_ref().map(|f| FlowField::build_within(world, &f.spec, f.goal, tick, (from.0.min(f.window().0.0), from.1.min(f.window().0.1)), (to.0.max(f.window().1.0), to.1.max(f.window().1.1)))).unwrap();
            let made: Vec<FlowField> = if self.parallel && wider.len() > 1 { wider.par_iter().map(build).collect() } else { wider.iter().map(build).collect() };
            for (&(g, _), field) in wider.iter().zip(made) {
                let field = Arc::new(field);
                self.keep_flow((field.spec, field.goal), field.clone());
                fields[g] = Some(field);
            }
            let us = started.elapsed().as_micros() as u64;
            self.stats.flow_builds += wider.len() as u64;
            self.stats.path_us += us;
            self.stats.flow_us += us;
            let (fields, ways_now) = (&fields, &ways);
            let again: Vec<(usize, &FlowField)> = wider.iter().flat_map(|&(g, _)| groups[g].members.iter().filter(move |&&i| matches!(ways_now[i], Some(None))).map(move |&i| (i, &**fields[g].as_ref().unwrap()))).collect();
            let started = std::time::Instant::now();
            let routes: Vec<Option<Vec<GridCell>>> = if self.parallel && again.len() >= PARALLEL_ROUTES { again.par_iter().map(route).collect() } else { again.iter().map(route).collect() };
            let us = started.elapsed().as_micros() as u64;
            self.stats.path_us += us;
            self.stats.route_us += us;
            for (&(i, _), route) in again.iter().zip(routes) {
                ways[i] = Some(route);
            }
        }
        let mut later = vec![false; requests.len()];
        for (group, _) in groups.iter().zip(&waiting).filter(|&(_, &wait)| wait) {
            for &i in &group.members {
                later[i] = true;
            }
        }
        let types = self.types.clone();
        for ((((r, aim), way), later), solo) in requests.into_iter().zip(aims).zip(ways).zip(later).zip(solo) {
            let Some(type_id) = self.unit(r.id).map(|u| u.type_id) else { continue };
            if later {
                // (Its group's field comes next tick: it asks again then.)
                continue;
            }
            if matches!(way, Some(None)) {
                if self.alone.len() >= ALONE_KEPT {
                    self.alone.clear();
                }
                self.alone.insert(r.id, (r.dest, tick));
            }
            match way {
                Some(Some(route)) => {
                    self.stats.flow_routes += 1;
                    // (Ending short of a goal walled in.)
                    let goal_pos = match route.last() {
                        Some(&end) if end != aim.goal => self.world.anchor_to_pos(end, r.footprint),
                        _ => aim.goal_pos,
                    };
                    if let Some(u) = self.unit_mut(r.id) {
                        u.path = route;
                        u.path_goal = Some(goal_pos);
                        u.move_request = Some(r.dest);
                        u.path_partial = false;
                        u.new_route();
                    }
                }
                // (No way off the group's field: cut off from it. A short search, for the nearest
                // point it can get to.)
                Some(None) => self.plan_way(r.id, types.get(type_id), r.spec, r.dest, 0, None, Aim { max_expansions: aim.max_expansions.min(CUT_OFF_SEARCH), ..aim }, false),
                None if solo => self.plan_way(r.id, types.get(type_id), r.spec, r.dest, 0, None, Aim { max_expansions: aim.max_expansions.min(CUT_OFF_SEARCH), ..aim }, false),
                None => self.plan_way(r.id, types.get(type_id), r.spec, r.dest, 0, None, aim, true),
            }
        }
    }

    /// A shared distance field (brought up to date) to a place within [`GROUP_REACH`] cells of
    /// `goal` that covers `start` and reaches `goal`: the one a group sent to places round a
    /// point shares, for one of them planning its way again. The nearest (ties by place).
    fn nearby_flow(&mut self, spec: MoveSpec, start: GridCell, goal: GridCell) -> Option<Arc<FlowField>> {
        let tick = self.tick;
        let cheb = |a: GridCell, b: GridCell| (a.0 - b.0).abs().max((a.1 - b.1).abs());
        let key = self
            .flow_cache
            .iter()
            .filter(|(k, f)| k.0 == spec && k.1 != goal && tick < f.tick + FLOW_TICKS && cheb(k.1, goal) <= GROUP_REACH && f.covers(start) && f.reaches(goal))
            .min_by_key(|(k, _)| (cheb(k.1, goal), k.1.1, k.1.0))
            .map(|(k, _)| *k)?;
        self.current_flow(key).filter(|f| f.covers(start) && f.reaches(goal))
    }

    /// Brings every shared route's distance field that is due (new obstacles since it was last
    /// brought up to date, [`FLOW_REPAIR_TICKS`] ago or more) up to date, on all cores: each is
    /// its own work. Those that can't be are dropped, to be built again when wanted.
    fn repair_flows(&mut self) {
        use rayon::prelude::*;
        let (tick, version) = (self.tick, self.world.blocking_version);
        let due = |f: &FlowField| f.blocking_version != version && tick >= f.repaired + FLOW_REPAIR_TICKS && tick < f.tick + FLOW_TICKS;
        let count = self.flow_cache.values().filter(|f| due(f)).count();
        if count == 0 {
            return;
        }
        let started = std::time::Instant::now();
        let world = &self.world;
        let failed: Vec<(MoveSpec, GridCell)> = self.flow_cache.par_iter_mut().filter(|(_, f)| due(f)).filter_map(|(key, f)| (!Arc::make_mut(f).repair(world, tick)).then_some(*key)).collect();
        for key in failed {
            self.flow_cache.remove(&key);
        }
        let us = started.elapsed().as_micros() as u64;
        self.stats.flow_repairs += count as u64;
        self.stats.path_us += us;
        self.stats.flow_us += us;
    }

    /// `path::open_goal` for a goal cell, remembered until the buildings and features change.
    fn open_goal(&mut self, spec: MoveSpec, goal: GridCell) -> GridCell {
        let version = self.world.occupancy_version;
        if let Some(&(v, open)) = self.open_goals.get(&(spec, goal))
            && v == version
        {
            return open;
        }
        let started = std::time::Instant::now();
        let open = crate::path::open_goal(&self.world, &spec, goal);
        self.stats.path_us += started.elapsed().as_micros() as u64;
        self.remember_open_goal((spec, goal), version, open);
        open
    }

    fn remember_open_goal(&mut self, key: (MoveSpec, GridCell), version: u64, open: GridCell) {
        if self.open_goals.len() >= 4096 {
            self.open_goals.clear();
        }
        self.open_goals.insert(key, (version, open));
    }

    /// The distance field shared by those heading to `key`'s goal, brought up to date with the
    /// buildings and features (`FlowField::repair`: a new wreck no longer means a new field
    /// for everyone). None when there is none, it has run its time, or it can't be brought up
    /// to date (it goes, to be built again).
    pub(crate) fn current_flow(&mut self, key: (MoveSpec, GridCell)) -> Option<Arc<FlowField>> {
        let tick = self.tick;
        let field = self.flow_cache.get_mut(&key)?;
        if tick >= field.tick + FLOW_TICKS {
            return None;
        }
        // (In a battle wrecks fall every tick, mostly along the front, where those behind took
        // their way: a few ticks' worth brought up to date at once cost little more than one.
        // Meanwhile a route may lead into a new wreck, which units slide round or plan again
        // from.)
        if field.blocking_version != self.world.blocking_version && tick >= field.repaired + FLOW_REPAIR_TICKS {
            let started = std::time::Instant::now();
            let repaired = Arc::make_mut(field).repair(&self.world, tick);
            let us = started.elapsed().as_micros() as u64;
            self.stats.flow_repairs += 1;
            self.stats.path_us += us;
            self.stats.flow_us += us;
            if !repaired {
                self.flow_cache.remove(&key);
                return None;
            }
        }
        self.flow_cache.get(&key).cloned()
    }

    /// Keeps a distance field for others heading to the same goal (the oldest goes when the
    /// cache is full; ties by key, for determinism).
    pub(crate) fn keep_flow(&mut self, key: (MoveSpec, GridCell), field: Arc<FlowField>) {
        if self.flow_cache.len() >= FLOW_CACHE_MAX
            && !self.flow_cache.contains_key(&key)
            // (Ties broken by the movement class too: a formation of tanks and hovercraft makes
            // fields to one goal for both in one tick, and the map's order is no order.)
            && let Some(oldest) = self.flow_cache.iter().min_by_key(|(k, f)| (f.tick, k.1, k.0.footprint, k.0.max_slope, k.0.max_depth, k.0.min_depth, k.0.hover)).map(|(k, _)| *k)
        {
            self.flow_cache.remove(&oldest);
        }
        self.flow_cache.insert(key, field);
    }

    /// Stores a path whose final waypoint is the goal for this leg.
    pub(crate) fn set_path(&mut self, id: UnitId, path: Vec<(i32, i32)>, dest: FxVec3, partial: bool) {
        let types = self.types.clone();
        let Some(u) = self.unit(id) else { return };
        let t = types.get(u.type_id);
        let leg_end = path.last().map(|&c| self.world.anchor_to_pos(c, t.footprint)).unwrap_or(dest);
        if let Some(u) = self.unit_mut(id) {
            u.path = path;
            u.path_goal = Some(leg_end);
            u.move_request = Some(dest);
            u.path_partial = partial;
            u.new_route();
        }
    }

    /// Distance from a point to a building footprint rectangle (horizontal).
    fn distance_to_footprint(p: FxVec3, center: FxVec3, fp: (u32, u32)) -> Fx {
        let hx = Fx::from_int(fp.0 as i64 * CELL) / 2;
        let hz = Fx::from_int(fp.1 as i64 * CELL) / 2;
        let dx = ((p.x - center.x).abs() - hx).max(Fx::ZERO);
        let dz = ((p.z - center.z).abs() - hz).max(Fx::ZERO);
        FxVec3::new(dx, Fx::ZERO, dz).length_xz()
    }

    /// Whether the builder can reach this footprint with its nano-lathe.
    pub fn in_build_range(&self, builder: &Unit, pos: FxVec3, footprint: (u32, u32)) -> bool {
        let t = self.unit_type(builder);
        Self::distance_to_footprint(builder.pos, pos, footprint) <= t.build_distance + t.radius
    }

    /// A work order's arrival square must fit inside its actual work range. A square
    /// of `reach / CELL` stopped diagonal approaches too far away, especially after
    /// `aim` moved a blocked goal to a free cell beside the target. Leave room for the
    /// movement arrival tolerance as well.
    fn move_to_work(&mut self, id: UnitId, t: &UnitType, pos: FxVec3, footprint: (u32, u32)) {
        if t.locomotion == Locomotion::Air {
            self.move_to(id, t, pos, 0);
            return;
        }
        let Some(u) = self.unit(id) else { return };
        let (from, factory) = (u.pos, u.factory);
        let spec = self.move_specs[t.id];
        let mut aim = self.aim(t, spec, from, factory, pos, 1);
        let centre = self.world.anchor_to_pos(aim.goal, t.footprint);
        aim.goal_pos = centre;
        let reach = (t.build_distance + t.radius - t.max_speed.max(Fx::from_int(3)) - Fx::from_int(2)).max(Fx::ZERO);
        let max_radius = (reach.to_int() / CELL + footprint.0.max(footprint.1) as i64 / 2).min(self.world.width.max(self.world.height) as i64) as i32;
        let radius = (0..=max_radius).rev().find(|&r| {
            let offset = r as i64 * CELL;
            [(-offset, -offset), (-offset, offset), (offset, -offset), (offset, offset)].iter().all(|&(x, z)| {
                Self::distance_to_footprint(centre + FxVec3::from_ints(x, 0, z), pos, footprint) <= reach
            })
        }).unwrap_or(0);
        self.plan_way(id, t, spec, pos, radius, factory, aim, false);
    }

    fn update_build_order(&mut self, id: UnitId, t: &UnitType, type_id: usize, pos: FxVec3) {
        // Orders may also come from mission scripts or a previously queued job.
        if !t.is_builder || !self.is_allowed(type_id) || !t.ai_build_options.contains(&type_id) {
            self.finish_build_job(id);
            self.stop(id);
            if let Some(u) = self.unit_mut(id) {
                u.orders.pop_front();
            }
            return;
        }
        let types = self.types.clone();
        let bt = types.get(type_id);
        let owner = self.unit(id).unwrap().owner;
        if let Some(job) = self.unit(id).and_then(|u| u.build) {
            let done = self.unit(job.target).is_none_or(|target| target.dead || target.is_complete() || !self.allied(owner, target.owner));
            if done {
                self.finish_build_job(id);
                if let Some(u) = self.unit_mut(id) {
                    u.orders.pop_front();
                }
                return;
            }
            self.work_on(id, t, job);
            return;
        }

        let u = self.unit(id).unwrap();
        let anchor = World::pos_to_anchor(pos, bt.footprint);
        let site = self.world.anchor_to_pos(anchor, bt.footprint);
        let reach = t.build_distance + t.radius;
        let gap = Self::distance_to_footprint(u.pos, site, bt.footprint);
        if gap > reach {
            if u.path_goal.is_none() && u.move_request == Some(site) && gap > reach + Fx::from_int(32) {
                // Already at the closest reachable point: the site is out of reach.
                self.events.push(SimEvent::OrderFailed { unit: id, reason: "can't reach the build site" });
                if let Some(u) = self.unit_mut(id) {
                    u.orders.pop_front();
                    u.move_request = None;
                }
                return;
            }
            if u.path_goal.is_none() || u.stuck_ticks > 60 {
                self.move_to_work(id, t, site, bt.footprint);
            }
            return;
        }
        self.stop(id);

        // Assist an unfinished unit of the same type already on the site, else place a new one.
        let origin = World::footprint_origin(anchor, bt.footprint);
        let existing = match self.world.occupant(anchor.0, anchor.1) {
            Occupant::Unit(slot) => self.units.get(slot as usize).and_then(|u| u.as_ref()).filter(|u| u.type_id == type_id && !u.dead && !u.is_complete() && self.allied(owner, u.owner)).map(|u| u.id),
            _ => None,
        };
        let mut standing = Vec::new();
        let target = match existing {
            Some(e) => e,
            None => {
                let slope = self.move_specs[type_id].max_slope.max(bt.def.max_slope as i32);
                let owner = self.unit(id).unwrap().owner;
                // Units standing where it goes: friends step off it as it is laid (the
                // builder too), an enemy blocks it as a building would.
                standing = self.units_on_site(type_id, origin);
                let enemy = standing.iter().any(|&o| self.unit(o).is_some_and(|o| self.is_enemy(owner, o.owner)));
                if enemy || !self.world.can_place(bt, slope, origin) {
                    self.events.push(SimEvent::OrderFailed { unit: id, reason: "build site blocked" });
                    if let Some(u) = self.unit_mut(id) {
                        u.orders.pop_front();
                    }
                    return;
                }
                if self.at_unit_limit(owner) {
                    return;
                }
                match self.spawn_unit(type_id, owner, site, 0, false) {
                    Some(n) => {
                        let tick = self.tick;
                        if let Some(frame) = self.unit_mut(n) {
                            frame.worked_at = Some(tick);
                        }
                        n
                    }
                    None => return,
                }
            }
        };
        if let Some(u) = self.unit_mut(id) {
            u.build = Some(BuildJob { target, started: false });
        }
        // (Off the site and off the builder's next ones too: a builder stepping onto the site it
        // builds next was walled in by its own row of buildings.)
        let next: Vec<(i32, i32, (u32, u32))> = self
            .unit(id)
            .map(|u| {
                u.orders
                    .iter()
                    .skip(1)
                    .filter_map(|o| match o {
                        Order::Build { type_id, pos } => {
                            let fp = types.get(*type_id).footprint;
                            let (x, z) = World::footprint_origin(World::pos_to_anchor(*pos, fp), fp);
                            Some((x, z, fp))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        for o in standing {
            self.step_aside(o, &next);
        }
    }

    /// Mobile units (finished, on the ground or landed) standing on the blocking cells a
    /// building of `type_id` would cover from `origin`, by slot.
    fn units_on_site(&mut self, type_id: usize, origin: (i32, i32)) -> Vec<UnitId> {
        let types = self.types.clone();
        let bt = types.get(type_id);
        let fp = bt.footprint;
        let blocks = |x: i32, z: i32| {
            let (dx, dz) = (x - origin.0, z - origin.1);
            dx >= 0 && dz >= 0 && dx < fp.0 as i32 && dz < fp.1 as i32 && bt.blocking_cells.get((dz as u32 * fp.0 + dx as u32) as usize).copied().unwrap_or(true)
        };
        self.units_on_cells(origin, fp, blocks)
    }

    /// Mobile units (finished, on the ground or landed) with a cell under them that `blocks`
    /// says is taken, among those near the footprint `fp` from `origin`, by slot.
    pub(crate) fn units_on_cells(&mut self, origin: (i32, i32), fp: (u32, u32), blocks: impl Fn(i32, i32) -> bool) -> Vec<UnitId> {
        let types = self.types.clone();
        let centre = FxVec3::from_ints((origin.0 as i64 * 2 + fp.0 as i64) * CELL / 2, 0, (origin.1 as i64 * 2 + fp.1 as i64) * CELL / 2);
        let mut near = std::mem::take(&mut self.scratch);
        self.grid.query(centre, Fx::from_int(fp.0.max(fp.1) as i64 * CELL + 64), &mut near);
        let mut on: Vec<UnitId> = near
            .iter()
            .filter_map(|&slot| self.units.get(slot as usize).and_then(|u| u.as_ref()))
            .filter(|u| {
                let ut = types.get(u.type_id);
                let mobile = matches!(ut.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval) || ut.locomotion == Locomotion::Air && u.landed;
                if u.dead || !u.is_complete() || u.carried_by.is_some() || !mobile {
                    return false;
                }
                let (ux, uz) = World::footprint_origin(World::pos_to_anchor(u.pos, ut.footprint), ut.footprint);
                (uz..uz + ut.footprint.1 as i32).any(|z| (ux..ux + ut.footprint.0 as i32).any(|x| blocks(x, z)))
            })
            .map(|u| u.id)
            .collect();
        self.scratch = near;
        on.sort_unstable();
        on
    }

    /// The nearest place within `rings` cells a unit can stand (with a cell to spare round it
    /// where there is one: a unit stopping a pixel or two short of the spot still stands
    /// clear), off the footprints `avoid` (each its first cell and size) where it can be.
    fn clear_spot(&self, u: &Unit, rings: i32, avoid: &[(i32, i32, (u32, u32))]) -> Option<FxVec3> {
        let t = self.types.get(u.type_id);
        let spec = self.move_specs[t.id];
        let here = World::pos_to_anchor(u.pos, t.footprint);
        let off = |c: (i32, i32)| {
            let (x0, z0) = World::footprint_origin(c, t.footprint);
            let (x1, z1) = (x0 + t.footprint.0 as i32, z0 + t.footprint.1 as i32);
            !avoid.iter().any(|&(ax, az, (w, h))| x0 < ax + w as i32 && ax < x1 && z0 < az + h as i32 && az < z1)
        };
        let fits = |c: (i32, i32)| if t.locomotion == Locomotion::Air { self.world.in_bounds(c.0, c.1) && self.world.occupant(c.0, c.1) == Occupant::Free } else { self.world.passable(&spec, c, None) };
        if !avoid.is_empty()
            && let Some(spot) = self.clear_spot_where(here, rings, &|c| fits(c) && off(c))
        {
            return Some(self.world.anchor_to_pos(spot, t.footprint));
        }
        let spot = self.clear_spot_where(here, rings, &fits)?;
        Some(self.world.anchor_to_pos(spot, t.footprint))
    }

    /// The nearest cell within `rings` of `here` that `fits` takes, one with fitting cells all
    /// round it first.
    fn clear_spot_where(&self, here: (i32, i32), rings: i32, fits: &dyn Fn((i32, i32)) -> bool) -> Option<(i32, i32)> {
        let roomy = |c: (i32, i32)| (-1..=1).all(|dz| (-1..=1).all(|dx| fits((c.0 + dx, c.1 + dz))));
        let ring = |r: i32| (-r..=r).flat_map(move |dz| (-r..=r).map(move |dx| (dx, dz))).filter(move |&(dx, dz)| dx.abs().max(dz.abs()) == r).map(move |(dx, dz)| (here.0 + dx, here.1 + dz));
        (1..=rings).find_map(|r| ring(r).find(|&c| roomy(c))).or_else(|| (1..=rings).find_map(|r| ring(r).find(|&c| fits(c))))
    }

    /// A unit a wreck has fallen on is thrown clear of it, a cell or two, its orders and route
    /// untouched (none: it stays).
    pub(crate) fn throw_clear(&mut self, id: UnitId) {
        let Some(spot) = self.unit(id).and_then(|u| self.clear_spot(u, 2, &[])) else { return };
        let types = self.types.clone();
        let sea = self.world.sea_level;
        let ground = self.world.height_at(spot.x, spot.z);
        if let Some(u) = self.unit_mut(id) {
            let t = types.get(u.type_id);
            u.pos.x = spot.x;
            u.pos.z = spot.z;
            u.pos.y = match t.locomotion {
                Locomotion::Hover | Locomotion::Naval => ground.max(sea),
                _ => ground,
            };
        }
    }

    /// A unit standing where something is put down (a building being laid) or in a factory's
    /// way (on its pad) walks off it, to the nearest place it can stand off the footprints
    /// `avoid` (the builder's next sites) where there is one, then carries on with what it was
    /// doing. (Out of an obstacle's cells it walks straight: from inside them no route is
    /// planned, and the crowd's pushing can't move it. A walk another building stops leaves it
    /// to `Sim::step_out_of_building`.)
    fn step_aside(&mut self, id: UnitId, avoid: &[(i32, i32, (u32, u32))]) {
        let Some(spot) = self.unit(id).and_then(|u| self.clear_spot(u, 12, avoid)) else { return };
        if let Some(u) = self.unit_mut(id) {
            u.orders.push_front(Order::Move(spot));
            u.path.clear();
            u.path_goal = Some(spot);
            u.move_request = Some(spot);
            u.path_partial = false;
            u.new_route();
        }
    }

    /// Reclaiming: walk into range, aim the nano-lathe, then dissolve the feature into
    /// metal and energy. Reclaim time scales with the feature's value, speed with the
    /// builder's worker time.
    fn update_reclaim_order(&mut self, id: UnitId, t: &UnitType, index: usize) {
        let done = |sim: &mut Sim| {
            sim.finish_build_job(id);
            sim.stop(id);
            if let Some(u) = sim.unit_mut(id) {
                u.orders.pop_front();
                if !u.stance_is_scripted() {
                    u.in_build_stance = false;
                }
            }
        };
        let Some(f) = self.features.get(index).filter(|f| f.alive).cloned() else {
            done(self);
            return;
        };
        let def = &self.data.features[f.def];
        if !def.reclaimable || !t.def.can_reclaim {
            done(self);
            return;
        }
        let (metal, energy) = (Fx::from_f64(def.metal.max(0.0)), Fx::from_f64(def.energy.max(0.0)));
        let fp = (def.footprint_x, def.footprint_z);
        let u = self.unit(id).unwrap();
        let reach = t.build_distance + t.radius;
        if Self::distance_to_footprint(u.pos, f.pos, fp) > reach {
            if u.path_goal.is_none() && u.move_request == Some(f.pos) || t.locomotion == Locomotion::Static {
                // Already at the closest reachable point: the feature is out of reach.
                self.give_up(id, Self::work_key(false, index as u32));
                done(self);
                return;
            }
            let replan = u.path_goal.is_none() || u.stuck_ticks > 60;
            self.finish_build_job(id);
            if replan {
                self.move_to_work(id, t, f.pos, fp);
            }
            return;
        }
        self.stop(id);
        // (Begun once its job is: a script may have left the stance raised from the last.)
        let started = self.unit(id).is_some_and(|u| u.build.is_some());
        if !started {
            let u = self.unit(id).unwrap();
            let to = f.pos - u.pos;
            let heading = wrap_angle(heading_of(to) - u.heading);
            let pitch = atan2(to.y, to.length_xz().max(Fx::ONE));
            let has_script = u.cob.as_ref().is_some_and(|c| c.has_function("StartBuilding"));
            if let Some(u) = self.unit_mut(id) {
                u.build = Some(BuildJob { target: id, started: true });
                if !has_script { u.in_build_stance = true; }
            }
            self.start_script(id, "StartBuilding", &[script_angle(heading), script_angle(pitch)]);
            self.events.push(SimEvent::Reclaiming { builder: id, feature: index });
            if has_script {
                // Wait for the arm animation to reach its stance.
                return;
            }
        }
        if !self.unit(id).is_some_and(|u| u.in_build_stance) {
            return;
        }
        // Work units: TA-like pacing where a 300-worker Commander reclaims a 100-metal
        // rock in about two seconds (twenty times its metal and a tenth of its energy, at
        // least a hundred, counted as construction counts it). What the work done so far is
        // worth is paid out, so the whole of it comes to the feature's metal and energy.
        let work = (metal + energy / 10).max(Fx::from_int(5)) * 20;
        let total = (work.0 as u64).saturating_mul(TICKS_PER_SECOND as u64);
        let f = &mut self.features[index];
        let before = f.reclaim_work;
        f.reclaim_work = before.saturating_add(t.work_rate.max(1)).min(total);
        f.reclaimed = Fx::fraction(f.reclaim_work, total);
        let (now, finished) = (f.reclaim_work, f.reclaim_work >= total);
        let gained_metal = metal.share(now, total) - metal.share(before, total);
        let gained_energy = energy.share(now, total) - energy.share(before, total);
        let owner = self.unit(id).map_or(0, |u| u.owner) as usize;
        let e = &mut self.players[owner].economy;
        e.metal = (e.metal + gained_metal).min(e.metal_capacity.max(e.metal));
        e.energy = (e.energy + gained_energy).min(e.energy_capacity.max(e.energy));
        e.tick_metal_in += gained_metal;
        e.tick_energy_in += gained_energy;
        if finished {
            self.remove_feature(index, true);
            done(self);
        }
    }

    /// Reclaiming one of our own units: construction in reverse. The unit falls back into a
    /// nanoframe at the builder's construction pace, returning its metal as it goes, and is
    /// gone (no wreck, no blast) when nothing is left. Other builders' work on it meanwhile
    /// only mends it, as TA's repair does: it does not undo the reclaim ([`Sim::being_taken_apart`]).
    fn update_reclaim_unit_order(&mut self, id: UnitId, t: &UnitType, target: UnitId) {
        let done = |sim: &mut Sim| {
            sim.finish_build_job(id);
            sim.stop(id);
            if let Some(u) = sim.unit_mut(id) {
                u.orders.pop_front();
                if !u.stance_is_scripted() {
                    u.in_build_stance = false;
                }
            }
        };
        let owner = self.unit(id).map_or(0, |u| u.owner);
        let Some(tu) = self.unit(target).filter(|tu| !tu.dead && tu.carried_by.is_none() && tu.owner == owner && tu.id != id && t.def.can_reclaim) else {
            done(self);
            return;
        };
        let types = self.types.clone();
        let tt = types.get(tu.type_id);
        let (target_pos, work) = (tu.pos, tu.build_work);
        let u = self.unit(id).unwrap();
        let reach = t.build_distance + t.radius;
        if Self::distance_to_footprint(u.pos, target_pos, tt.footprint) > reach {
            if u.path_goal.is_none() && u.move_request == Some(target_pos) || t.locomotion == Locomotion::Static {
                self.give_up(id, Self::work_key(true, target.slot));
                done(self);
                return;
            }
            let replan = u.path_goal.is_none() || u.stuck_ticks > 60;
            self.finish_build_job(id);
            if replan {
                self.move_to_work(id, t, target_pos, tt.footprint);
            }
            return;
        }
        self.stop(id);
        // (Begun once its job is: a script may have left the stance raised from the last.)
        let started = self.unit(id).is_some_and(|u| u.build.is_some());
        if !started {
            let u = self.unit(id).unwrap();
            let to = target_pos - u.pos;
            let heading = wrap_angle(heading_of(to) - u.heading);
            let pitch = atan2(to.y, to.length_xz().max(Fx::ONE));
            let has_script = u.cob.as_ref().is_some_and(|c| c.has_function("StartBuilding"));
            if !has_script && let Some(u) = self.unit_mut(id) {
                u.in_build_stance = true;
            }
            self.start_script(id, "StartBuilding", &[script_angle(heading), script_angle(pitch)]);
            self.events.push(SimEvent::NanoSpray { builder: id, target });
            if let Some(u) = self.unit_mut(id) {
                // The builder's own id: no construction job for the target to share.
                u.build = Some(BuildJob { target: id, started: true });
            }
            if has_script {
                return;
            }
        }
        if !self.unit(id).is_some_and(|u| u.in_build_stance) {
            return;
        }
        // Construction in reverse: what the work undone was worth comes back. At TA's pace, by
        // the unit's metal as a feature's (twenty times it in worker time): a metal maker, which
        // costs none, is gone at once.
        let total = tt.build_work;
        let reclaim_total = ((tt.cost_metal * 20).0.max(0) as u64).saturating_mul(TICKS_PER_SECOND as u64).max(1);
        let step = ((t.work_rate.max(1) as u128 * total as u128) / reclaim_total as u128).clamp(1, u64::MAX as u128) as u64;
        let left = work.saturating_sub(step);
        let refund = tt.cost_metal.share(work, total) - tt.cost_metal.share(left, total);
        let e = &mut self.players[owner as usize].economy;
        e.metal = (e.metal + refund).min(e.metal_capacity.max(e.metal));
        e.tick_metal_in += refund;
        let tick = self.tick;
        if let Some(tu) = self.unit_mut(target) {
            tu.set_build_work(left, total);
            tu.health = tu.health.min(tt.max_health * tu.build_progress).max(Fx::ONE);
            tu.taken_apart_at = Some(tick);
            // Taken apart: it stops working.
            tu.orders.clear();
            tu.factory_queue.clear();
        }
        if left == 0 {
            if let Some(tu) = self.unit_mut(target) {
                tu.dead = true;
                // Gone without a Killed script, wreck or blast; taken apart by its own side,
                // so no battle loss.
                tu.death_processed = true;
                tu.removed = true;
            }
            done(self);
        }
    }

    /// A finished unit its own side is taking apart (one of its builders took work off it in
    /// the last second): other builders' work on it mends it and no more, and nobody is sent
    /// to finish it. One left part taken apart longer than that is built back up as a
    /// construction is (paid for), whole again without being finished a second time.
    pub fn being_taken_apart(&self, u: &Unit) -> bool {
        u.finished && !u.is_complete() && u.taken_apart_at.is_some_and(|at| self.tick < at + TICKS_PER_SECOND as u64)
    }

    /// Construction left with nobody at it ([`Unit::worked_at`] [`FRAME_GRACE`] ago or more; not
    /// a factory's, in its yard) un-builds: the whole of a frame in [`FRAME_DECAY`], its metal
    /// coming back as the work goes (not its energy), and gone (no wreck, no blast) when nothing
    /// of it is left; one set down and never worked on goes at once. Not one a scene or a
    /// mission placed unfinished, nobody having worked on it.
    fn decay_abandoned_frames(&mut self) {
        let tick = self.tick;
        let mut decaying = Vec::new();
        for u in self.units.iter().flatten() {
            if u.dead || u.finished || u.is_complete() || u.carried_by.is_some() || u.worked_at.is_none_or(|at| tick < at + FRAME_GRACE) {
                continue;
            }
            // (A factory's, in its yard: the factory's to finish or give up.)
            if u.factory.and_then(|f| self.unit(f)).is_some_and(|f| !f.dead) {
                continue;
            }
            decaying.push(u.id);
        }
        for id in decaying {
            let Some(u) = self.unit(id) else { continue };
            let tt = self.types.get(u.type_id);
            let (total, work, owner, cost, max_health) = (tt.build_work, u.build_work, u.owner, tt.cost_metal, tt.max_health);
            let left = work.saturating_sub((total / FRAME_DECAY).max(1));
            let refund = cost.share(work, total) - cost.share(left, total);
            if let Some(p) = self.players.get_mut(owner as usize) {
                let e = &mut p.economy;
                e.metal = (e.metal + refund).min(e.metal_capacity.max(e.metal));
                e.tick_metal_in += refund;
            }
            if let Some(u) = self.unit_mut(id) {
                u.set_build_work(left, total);
                u.health = u.health.min(max_health * u.build_progress).max(Fx::ONE);
                if left == 0 {
                    // Gone without a Killed script, wreck or blast; given up, so no battle loss.
                    u.dead = true;
                    u.death_processed = true;
                    u.removed = true;
                }
            }
        }
    }

    /// Whether a builder has anything to do for `u` (of type `t`): finish it, or mend it (one
    /// being taken apart only up to what is left of it).
    fn needs_work(&self, u: &Unit, t: &UnitType) -> bool {
        if self.being_taken_apart(u) {
            u.health < t.max_health * u.build_progress
        } else {
            !u.is_complete() || u.health < t.max_health
        }
    }

    /// The unit type a feature is the wreck of, if any.
    pub fn wreck_type(&self, feature: usize) -> Option<usize> {
        self.features.get(feature).filter(|f| f.alive).and_then(|f| self.wreck_of.get(&f.def).copied())
    }

    /// Resurrection: walk into range and pour the unit's energy cost into its wreck at the
    /// builder's construction pace (the wreck stands in for the metal). The finished unit
    /// rises whole for the builder's side; an energy shortfall stalls the work.
    fn update_resurrect_order(&mut self, id: UnitId, t: &UnitType, index: usize) {
        let done = |sim: &mut Sim| {
            sim.finish_build_job(id);
            sim.stop(id);
            if let Some(u) = sim.unit_mut(id) {
                u.orders.pop_front();
                if !u.stance_is_scripted() {
                    u.in_build_stance = false;
                }
            }
        };
        let Some(type_id) = self.wreck_type(index).filter(|&type_id| t.def.can_resurrect && self.is_allowed(type_id)) else {
            done(self);
            return;
        };
        let f = self.features[index].clone();
        let def = &self.data.features[f.def];
        let fp = (def.footprint_x, def.footprint_z);
        let u = self.unit(id).unwrap();
        let reach = t.build_distance + t.radius;
        if Self::distance_to_footprint(u.pos, f.pos, fp) > reach {
            if u.path_goal.is_none() && u.move_request == Some(f.pos) || t.locomotion == Locomotion::Static {
                self.give_up(id, Self::work_key(false, index as u32));
                done(self);
                return;
            }
            let replan = u.path_goal.is_none() || u.stuck_ticks > 60;
            self.finish_build_job(id);
            if replan {
                self.move_to_work(id, t, f.pos, fp);
            }
            return;
        }
        self.stop(id);
        // (Begun once its job is: a script may have left the stance raised from the last.)
        let started = self.unit(id).is_some_and(|u| u.build.is_some());
        if !started {
            let u = self.unit(id).unwrap();
            let to = f.pos - u.pos;
            let heading = wrap_angle(heading_of(to) - u.heading);
            let pitch = atan2(to.y, to.length_xz().max(Fx::ONE));
            let has_script = u.cob.as_ref().is_some_and(|c| c.has_function("StartBuilding"));
            if let Some(u) = self.unit_mut(id) {
                u.build = Some(BuildJob { target: id, started: true });
                if !has_script { u.in_build_stance = true; }
            }
            self.start_script(id, "StartBuilding", &[script_angle(heading), script_angle(pitch)]);
            self.events.push(SimEvent::Reclaiming { builder: id, feature: index });
            if has_script {
                return;
            }
        }
        if !self.unit(id).is_some_and(|u| u.in_build_stance) {
            return;
        }
        // The unit's build work, paying what each tick's share of its energy cost is worth.
        let rt = self.types.get(type_id);
        let total = rt.build_work;
        let before = self.features[index].resurrect_work;
        let full = self.mutator_work_rate(self.perk_work_rate(id, t.work_rate)).min(total.saturating_sub(before));
        let owner = self.unit(id).map_or(0, |u| u.owner);
        let e = &mut self.players[owner as usize].economy;
        // Resurrection spends no metal: unrelated metal-starved construction must not
        // reduce its pace. Share energy with the other jobs and still cap payment to stores.
        let factor = if rt.cost_energy > Fx::ZERO && e.energy_demand > Fx::ZERO { e.energy / e.energy_demand } else { Fx::ONE };
        let work = e.spend_work(before, total, full, Fx::ZERO, rt.cost_energy, factor);
        let after = before + work;
        let at_limit = self.at_unit_limit(owner);
        let f = &mut self.features[index];
        f.resurrect_work = after;
        f.resurrected = Fx::fraction(after, total);
        if after < total || at_limit {
            return;
        }
        // The wreck goes (no remnant) and the unit rises where it lay.
        let (pos, heading, origin) = (f.pos, f.heading, f.origin);
        f.alive = false;
        self.world.release(Occupant::Feature(index as u32), origin, fp);
        self.events.push(SimEvent::FeatureDestroyed { index, pos });
        self.spawn_unit(type_id, owner, pos, heading, true);
        done(self);
    }

    /// Takes the features within `radius` of `at` off the map, remnants and all (a clear
    /// stage round a unit on show); metal deposits stay.
    pub fn clear_features(&mut self, at: FxVec3, radius: Fx) {
        let r2 = radius.to_f32().powi(2);
        let (x, z) = (at.x.to_f32(), at.z.to_f32());
        self.clear_features_where(|_, p| (p.x.to_f32() - x).powi(2) + (p.z.to_f32() - z).powi(2) < r2);
    }

    /// Takes off the map the features `take` picks (given each one's index and place), remnants
    /// and all; metal deposits stay. Returns how many went.
    pub fn clear_features_where(&mut self, take: impl Fn(usize, FxVec3) -> bool) -> usize {
        let gone: Vec<usize> = self
            .features
            .iter()
            .enumerate()
            .filter(|(i, f)| f.alive && !self.data.features[f.def].is_metal_deposit() && take(*i, f.pos))
            .map(|(i, _)| i)
            .collect();
        let count = gone.len();
        for index in gone {
            self.take_off_map(index);
        }
        count
    }

    /// Takes a feature off the map, leaving nothing in its place.
    fn take_off_map(&mut self, index: usize) {
        let f = &mut self.features[index];
        f.alive = false;
        let (origin, pos, def) = (f.origin, f.pos, f.def);
        let fp = (self.data.features[def].footprint_x, self.data.features[def].footprint_z);
        self.world.release(Occupant::Feature(index as u32), origin, fp);
        self.events.push(SimEvent::FeatureDestroyed { index, pos });
    }

    /// A building taking its ground (its footprint, from `origin`) clears it of what lies loose
    /// on it: the features that do not block (TA lets a building be put over them) and are
    /// neither wreckage holding metal nor deposits, vents or what cannot be destroyed (TA's ferns
    /// and fronds, small stones, scars). Their 3D stand-ins would stand through it, so they go,
    /// leaving nothing. A heap's metal stays to be reclaimed; anemones and corals, indestructible,
    /// stay too.
    fn clear_ground(&mut self, origin: (i32, i32), fp: (u32, u32)) {
        let (x0, z0) = (Fx::from_int(origin.0 as i64 * CELL), Fx::from_int(origin.1 as i64 * CELL));
        let (x1, z1) = (x0 + Fx::from_int(fp.0 as i64 * CELL), z0 + Fx::from_int(fp.1 as i64 * CELL));
        let centre = FxVec3::new((x0 + x1) / 2, Fx::ZERO, (z0 + z1) / 2);
        // (Out to the footprint's corners, and more.)
        let reach = Fx::from_int((fp.0 + fp.1) as i64 * CELL);
        let mut near = Vec::new();
        self.features_near(centre, reach, &mut near);
        for index in near.into_iter().map(|i| i as usize) {
            let f = &self.features[index];
            let d = &self.data.features[f.def];
            let loose = f.alive && !d.blocking && d.metal <= 0.0 && !d.indestructible && !d.geothermal && !d.is_metal_deposit();
            if loose && f.pos.x >= x0 && f.pos.x < x1 && f.pos.z >= z0 && f.pos.z < z1 {
                self.take_off_map(index);
            }
        }
    }

    /// Removes a feature; `reclaimed` swaps in its reclaim remnant (e.g. a smudge).
    pub(crate) fn remove_feature(&mut self, index: usize, reclaimed: bool) {
        let Some(f) = self.features.get_mut(index) else { return };
        if !f.alive {
            return;
        }
        f.alive = false;
        let (origin, def_id, pos, heading) = (f.origin, f.def, f.pos, f.heading);
        let def = self.data.features[def_id].clone();
        self.world.release(Occupant::Feature(index as u32), origin, (def.footprint_x, def.footprint_z));
        self.events.push(SimEvent::FeatureDestroyed { index, pos });
        let next = if reclaimed { &def.feature_reclamate } else { &def.feature_dead };
        if let Some(nid) = self.data.feature_id(next) {
            let nd = &self.data.features[nid];
            let slot = self.features.len() as u32;
            if nd.blocking && !nd.is_metal_deposit() {
                self.world.occupy(Occupant::Feature(slot), origin, (nd.footprint_x, nd.footprint_z), &[]);
            }
            let health = Fx::from_f64(nd.damage.max(1.0));
            self.add_feature(Feature::new(nid, origin, pos, heading, health));
            self.events.push(SimEvent::FeatureCreated { index: slot as usize, wreck_of: None });
        }
    }

    fn update_repair_order(&mut self, id: UnitId, t: &UnitType, target: UnitId) {
        let types = self.types.clone();
        let owner = self.unit(id).unwrap().owner;
        let Some(tu) = self.unit(target).filter(|u| !u.dead && u.carried_by.is_none() && t.is_builder && self.allied(owner, u.owner)) else {
            if let Some(u) = self.unit_mut(id) {
                u.orders.pop_front();
            }
            self.finish_build_job(id);
            self.stop(id);
            return;
        };
        let tt = types.get(tu.type_id);
        let needs_work = self.needs_work(tu, tt);
        if !needs_work {
            self.finish_build_job(id);
            self.stop(id);
            if let Some(u) = self.unit_mut(id) {
                u.orders.pop_front();
            }
            return;
        }
        let target_pos = tu.pos;
        let u = self.unit(id).unwrap();
        let reach = t.build_distance + t.radius;
        if Self::distance_to_footprint(u.pos, target_pos, tt.footprint) > reach {
            if u.path_goal.is_none() && u.move_request == Some(target_pos) || t.locomotion == Locomotion::Static {
                // Closest reachable point reached (or can't move) and still out of range.
                self.give_up(id, Self::work_key(true, target.slot));
                self.finish_build_job(id);
                if let Some(u) = self.unit_mut(id) {
                    u.orders.pop_front();
                }
                return;
            }
            // The target may have moved, or this builder may have been pushed away
            // since its previous tick of work. Stop the old beam while approaching.
            self.finish_build_job(id);
            let u = self.unit(id).unwrap();
            if u.path_goal.is_none() || u.stuck_ticks > 60 {
                self.move_to_work(id, t, target_pos, tt.footprint);
            }
            return;
        }
        self.stop(id);
        let job = self.unit(id).and_then(|u| u.build).filter(|j| j.target == target).unwrap_or(BuildJob { target, started: false });
        if let Some(u) = self.unit_mut(id) {
            u.build = Some(job);
        }
        self.work_on(id, t, job);
    }

    /// Aims the nano-lathe (StartBuilding) and applies build power once the script is in
    /// its build stance.
    fn work_on(&mut self, id: UnitId, t: &UnitType, job: BuildJob) {
        let Some((target_pos, footprint)) = self.unit(job.target).map(|u| (u.pos, self.unit_type(u).footprint)) else { return };
        let u = self.unit(id).unwrap();
        if !t.is_factory && !self.in_build_range(u, target_pos, footprint) {
            self.finish_build_job(id);
            self.move_to_work(id, t, target_pos, footprint);
            return;
        }
        // TA's construction aircraft face what they build and sway round it, rather than hang
        // still over it.
        if t.locomotion == Locomotion::Air && !t.is_factory {
            self.drift_round(id, t, job.target, target_pos, footprint);
        }
        let Some(u) = self.unit(id) else { return };
        let has_script = u.cob.as_ref().is_some_and(|c| c.has_function("StartBuilding"));
        if !job.started {
            let to = target_pos - u.pos;
            let heading = wrap_angle(heading_of(to) - u.heading);
            let pitch = atan2(to.y, to.length_xz().max(Fx::ONE));
            if let Some(u) = self.unit_mut(id) {
                u.build = Some(BuildJob { started: true, ..job });
                if !has_script {
                    u.in_build_stance = true;
                }
            }
            self.start_script(id, "StartBuilding", &[script_angle(heading), script_angle(pitch)]);
            self.events.push(SimEvent::NanoSpray { builder: id, target: job.target });
            return;
        }
        let in_stance = self.unit(id).is_some_and(|u| u.in_build_stance) || !has_script;
        if in_stance {
            self.apply_build_power(id, t, job.target);
        }
    }

    /// A construction aircraft at work, a tick of it. It turns to face what it builds (`target`,
    /// its footprint `footprint` about `center`) and, once it has come to a stop facing it,
    /// sways round it sideways along an arc, as a hovering aircraft can: [`AIR_BUILDER_SWEEP`]
    /// round to one side and back again, easing off at each end, slowly enough that it turns to
    /// keep facing it all the way (its fastest turn round it is three quarters of its turn rate).
    /// It sways at a distance that keeps it within reach of the footprint all the way round
    /// (straight out from a side is the farthest it gets from it), easing in to that over
    /// [`AIR_BUILDER_SETTLE_TICKS`] when it came to a stop further out; a step that would still
    /// take it out of reach is not taken.
    fn drift_round(&mut self, id: UnitId, t: &UnitType, target: UnitId, center: FxVec3, footprint: (u32, u32)) {
        let tick = self.tick;
        let Some(u) = self.unit(id) else { return };
        let from = (u.pos - center).xz();
        let dist = from.length_xz();
        let facing_now = if dist < Fx::ONE { u.heading } else { heading_of(-from) };
        let sway = u.drift.filter(|d| d.target == target && d.last + 1 == tick).or_else(|| {
            // (Braking from its flight in, or turning to face it: it sways once it has stopped,
            // facing it.)
            let faces = wrap_angle(facing_now - u.heading).abs() <= AIR_BUILDER_FACING;
            (u.speed.is_zero() && faces).then(|| Drift {
                target,
                since: tick,
                last: tick,
                angle: if dist < Fx::ONE { wrap_angle(u.heading + 32768) } else { heading_of(from) },
                distance: dist,
                side: if (id.slot as u64 + tick).is_multiple_of(2) { 1 } else { -1 },
            })
        });
        let mut to = u.pos;
        if let Some(d) = sway {
            let half = Fx::from_int(footprint.0.min(footprint.1) as i64 * CELL) / 2;
            let reach = t.build_distance + t.radius;
            let most = half + reach * 3 / 4;
            let tau = tick - d.since;
            // (Round it and back: the sweep's fastest turn round it is pi * sweep / period.)
            let period = (4 * 31416 * AIR_BUILDER_SWEEP / (3 * 10000 * t.turn_rate.max(1) as i64))
                .clamp(AIR_BUILDER_SWEEP_TICKS.0, AIR_BUILDER_SWEEP_TICKS.1) as u64;
            let phase = ((tau % period) * 65536 / period) as i32;
            let round = ((Fx::ONE - crate::fixed::cos(phase)) * AIR_BUILDER_SWEEP / 2).to_int() as i32;
            let settled = d.distance.min(most);
            let settle = (tau.min(AIR_BUILDER_SETTLE_TICKS) * 32768 / AIR_BUILDER_SETTLE_TICKS) as i32;
            let out = settled + (d.distance - settled) * (Fx::ONE + crate::fixed::cos(settle)) / 2;
            let p = center + heading_vector(wrap_angle(d.angle + d.side as i32 * round)).scale(out);
            if Self::distance_to_footprint(p, center, footprint) <= reach {
                to = FxVec3::new(p.x, u.pos.y, p.z);
            }
        }
        let facing = if (center - to).xz().length_xz() < Fx::ONE { u.heading } else { heading_of((center - to).xz()) };
        if let Some(u) = self.unit_mut(id) {
            u.drift = sway.map(|d| Drift { last: tick, ..d });
            u.pos = to;
            let diff = wrap_angle(facing - u.heading);
            u.heading = wrap_angle(u.heading + diff.clamp(-t.turn_rate, t.turn_rate));
        }
    }

    fn finish_build_job(&mut self, id: UnitId) {
        let is_factory = self.unit(id).is_some_and(|u| self.unit_type(u).is_factory);
        let Some(u) = self.unit_mut(id) else { return };
        u.drift = None;
        // A factory's stance is its activation readiness, shared by successive products. A
        // builder's with a `StartBuilding` script is its script's, as in TA: it drops it in
        // `StopBuilding`, or keeps it while its arm stays out (`Unit::stance_is_scripted`).
        if !is_factory && !u.stance_is_scripted() {
            u.in_build_stance = false;
        }
        if u.build.take().is_some() {
            self.start_script(id, "StopBuilding", &[]);
        }
    }

    /// Accumulate the work of all repairers before rounding healing to Fx precision. A unit
    /// being taken apart mends only up to what is left of it (as the reclaim leaves it).
    fn repair_health(&mut self, target: UnitId, rate: u64) {
        let Some(u) = self.unit(target).filter(|u| !u.dead && (u.is_complete() || u.finished) && u.carried_by.is_none()) else { return };
        let t = self.unit_type(u);
        let (total, max_health) = (t.build_work, t.max_health);
        // (Whole, all of it; else what is left of it, and never less than it has.)
        let most = if u.is_complete() { max_health } else { (max_health * u.build_progress).max(u.health) };
        let rate = rate.min(total);
        let before = u.repair_work.min(total);
        let left = total - before;
        let (after, heal) = if rate >= left {
            (rate - left, max_health - max_health.share(before, total) + max_health.share(rate - left, total))
        } else {
            (before + rate, max_health.share(before + rate, total) - max_health.share(before, total))
        };
        if let Some(u) = self.unit_mut(target) {
            u.health = (u.health + heal).min(most);
            u.repair_work = if u.health == most { 0 } else { after };
        }
    }

    /// Adds one tick of the builder's work to `target`, drawing resources with stall scaling.
    /// Work is counted in whole units (see [`UnitType::build_work`]), and the resources and
    /// health a tick brings are what the work done so far is worth less what it was worth
    /// before, so a build of any length takes exactly its build time at full pace and costs
    /// exactly its price.
    fn apply_build_power(&mut self, builder: UnitId, t: &UnitType, target: UnitId) {
        let types = self.types.clone();
        let Some(owner) = self.unit(builder).map(|u| u.owner) else { return };
        let Some(tu) = self.unit(target) else { return };
        if tu.dead || tu.carried_by.is_some() || !t.is_builder || !self.allied(owner, tu.owner) {
            return;
        }
        let tt = types.get(tu.type_id);
        // A unit is finished once. While its own side takes it apart, work on it mends it and
        // no more, as TA's repair does (the reclaim goes on, not undone); one left part taken
        // apart is built back up, paid for as a build is, and is simply whole again.
        let repairing = tu.is_complete() || self.being_taken_apart(tu);
        let (total, done, finished) = (tt.build_work, tu.build_work, tu.finished);
        let rate = self.mutator_work_rate(self.perk_work_rate(builder, t.work_rate));
        if self.players.get(owner as usize).is_some_and(|p| p.cheats.instant_build) {
            // The instant-build cheat: finished (or mended) at once, for nothing.
            let mut owner = None;
            if let Some(tu) = self.unit_mut(target) {
                if !repairing {
                    tu.set_build_work(total, total);
                }
                // (All of it when whole; one being taken apart, what is left of it.)
                tu.health = (tt.max_health * tu.build_progress).max(tu.health);
                tu.repair_work = 0;
                owner = Some(tu.owner);
            }
            if !repairing && !finished {
                if let Some(p) = owner.and_then(|o| self.players.get_mut(o as usize)) {
                    p.stats.built += 1;
                }
                self.on_completed(target);
            }
            return;
        }
        if repairing {
            // Repair: restore health at the build rate (all of it in the time a build takes);
            // costs nothing (TA behaviour).
            self.repair_health(target, rate);
            return;
        }
        let tick = self.tick;
        if let Some(tu) = self.unit_mut(target) {
            tu.worked_at = Some(tick);
        }
        let full = rate.min(total.saturating_sub(done));
        if full == 0 {
            return;
        }
        let eco = &mut self.players[owner as usize].economy;
        let work = eco.spend_work(done, total, full, tt.cost_metal, tt.cost_energy, eco.build_factor);
        let heal = tt.max_health.share(done + work, total) - tt.max_health.share(done, total);
        let mut completed = false;
        if let Some(tu) = self.unit_mut(target) {
            tu.set_build_work(done + work, total);
            tu.health = (tu.health + heal).min(tt.max_health);
            completed = tu.is_complete();
        }
        if completed && !finished {
            if let Some(owner) = self.unit(target).map(|u| u.owner) && let Some(p) = self.players.get_mut(owner as usize) {
                p.stats.built += 1;
            }
            self.on_completed(target);
        }
    }

    // ---- Factories ----------------------------------------------------------------------

    /// The unit in production was taken off the queue: its frame is taken apart (the metal
    /// spent on it comes back, as reclaiming it would give) and the factory moves on to the
    /// next in its queue. (Before, the frame was finished anyway, and the next unit queued was
    /// dropped in its place.)
    fn cancel_production(&mut self, factory: UnitId) {
        let Some(frame) = self.unit(factory).and_then(|f| f.build).map(|b| b.target) else { return };
        let Some(u) = self.unit(frame).filter(|u| u.factory == Some(factory) && !u.is_complete() && !u.dead) else { return };
        let tt = self.types.get(u.type_id);
        let (owner, refund) = (u.owner, tt.cost_metal.share(u.build_work, tt.build_work));
        let e = &mut self.players[owner as usize].economy;
        e.metal = (e.metal + refund).min(e.metal_capacity.max(e.metal));
        e.tick_metal_in += refund;
        if let Some(u) = self.unit_mut(frame) {
            // Gone without a Killed script, wreck or blast; given up, so no battle loss.
            u.dead = true;
            u.death_processed = true;
            u.removed = true;
        }
        if let Some(f) = self.unit_mut(factory) {
            f.build = None;
        }
        self.start_script(factory, "StopBuilding", &[]);
    }

    fn update_factory(&mut self, id: UnitId, t: &UnitType) {
        let types = self.types.clone();
        // Its units on their way out are let go as they clear its footprint.
        let exit = self.factory_exit(id, t);
        let leaving = self.unit(id).is_some_and(|f| !f.yard.is_empty()) && self.yard_has_leaving_units(id, exit);
        let Some(f) = self.unit(id) else { return };
        let current = f.build;
        let has_queue = !f.factory_queue.is_empty();

        if let Some(job) = current {
            let Some(target) = self.unit(job.target) else {
                self.finish_build_job(id);
                return;
            };
            if target.is_complete() {
                // Release the finished unit: walk out of the yard to the rally point (one that
                // cannot move, a module or an upgrade, stays where it was made).
                let unit_id = job.target;
                let tt = types.get(target.type_id);
                let rally = self.unit(id).map(|f| f.rally.clone()).unwrap_or_default();
                if tt.locomotion != Locomotion::Static && let Some(nu) = self.unit_mut(unit_id) && nu.orders.is_empty() {
                    nu.orders.push_back(Order::Move(exit));
                    for o in &rally {
                        queue_order(&mut nu.orders, o, true, exit);
                    }
                }
                let _ = tt;
                if let Some(f) = self.unit_mut(id) {
                    f.build = None;
                    let done = f.factory_queue.pop_front();
                    if let (true, Some(t)) = (f.factory_repeat, done) {
                        f.factory_queue.push_back(t);
                    }
                }
                self.start_script(id, "StopBuilding", &[]);
                return;
            }
            // A unit under way is built on whatever the factory's script does with its stance or
            // its switch since: the stance readies a factory to start one (and shows its nano
            // spray). The mods' buildings that make their own upgrade drop it as they start, to
            // build with no spray, and TA built on: Total Mayhem's geothermal plant, which also
            // switches itself off a moment after it readies; TA: Escalation's Sentinel, which has
            // no Activate at all.
            self.apply_build_power(id, t, job.target);
            return;
        }

        if !has_queue {
            // A factory closes up with nothing to make, as TA's do; another unit that builds stays
            // as it is (`UnitType::is_plant`).
            if self.unit(id).is_some_and(|f| f.activated) && !leaving && t.is_plant() {
                self.set_active(id, false);
            }
            return;
        }
        // Open up (the script opens the yard and sets INBUILDSTANCE when ready).
        if !self.unit(id).unwrap().activated {
            self.set_active(id, true);
            return;
        }
        if !self.unit(id).unwrap().in_build_stance || leaving {
            return;
        }
        let Some(&type_id) = self.unit(id).unwrap().factory_queue.front() else { return };
        if self.at_unit_limit(self.unit(id).unwrap().owner) {
            return;
        }
        let pad_piece = self.call_script(id, "QueryBuildInfo", &[0]).and_then(|p| p.first().copied()).unwrap_or(-1);
        let f = self.unit(id).unwrap();
        let pad_pos = self.cob_piece_position(f, t, pad_piece).unwrap_or(f.pos);
        let owner = f.owner;
        let heading = f.heading;
        // The pad clear first: one of its own side's units standing still where the next would
        // go (built in no time, the instant-build cheat or many builders helping; given an
        // order as it was done: to guard the factory, or to build beside it) is sent off it, to
        // carry on from there; one passing over it is waited for, as is anyone else's.
        let on_pad = self.on_pad(type_id, pad_pos);
        if !on_pad.is_empty() {
            for o in on_pad {
                if self.unit(o).is_some_and(|u| u.owner == owner && u.path_goal.is_none()) {
                    self.step_aside(o, &[]);
                }
            }
            return;
        }
        self.start_script(id, "StartBuilding", &[]);
        if let Some(new) = self.spawn_unit(type_id, owner, pad_pos, heading, false) {
            if let Some(nu) = self.unit_mut(new) {
                nu.factory = Some(id);
            }
            if let Some(f) = self.unit_mut(id) {
                f.build = Some(BuildJob { target: new, started: true });
                f.yard.push(new);
            }
            self.events.push(SimEvent::NanoSpray { builder: id, target: new });
        }
    }

    /// The finished units on the ground standing (their middle) where a unit of `type_id`
    /// built at `pad_pos` would be, by slot.
    fn on_pad(&mut self, type_id: usize, pad_pos: FxVec3) -> Vec<UnitId> {
        let types = self.types.clone();
        let fp = types.get(type_id).footprint;
        let (x, z) = World::footprint_origin(World::pos_to_anchor(pad_pos, fp), fp);
        let (x0, z0) = (Fx::from_int(x as i64 * CELL), Fx::from_int(z as i64 * CELL));
        let (x1, z1) = (x0 + Fx::from_int(fp.0 as i64 * CELL), z0 + Fx::from_int(fp.1 as i64 * CELL));
        let mut near = std::mem::take(&mut self.scratch);
        self.grid.query(pad_pos, Fx::from_int(fp.0.max(fp.1) as i64 * CELL + 64), &mut near);
        let mut on: Vec<UnitId> = near
            .iter()
            .filter_map(|&slot| self.units.get(slot as usize).and_then(|u| u.as_ref()))
            .filter(|u| {
                let ut = types.get(u.type_id);
                !u.dead && u.is_complete() && u.carried_by.is_none() && matches!(ut.locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval) && u.pos.x >= x0 && u.pos.x < x1 && u.pos.z >= z0 && u.pos.z < z1
            })
            .map(|u| u.id)
            .collect();
        self.scratch = near;
        on.sort_unstable();
        on
    }

    /// Whether a unit the factory built is finished and on its way out of its yard (heading
    /// for `exit`, the way out it was given, or moving on): the next is started once it is out
    /// of the factory's footprint, whatever its route on (a unit used to hold the next up until
    /// it had one order left, which a patrol never has, and with nowhere to go not at all, so
    /// that units built in no time piled up in the yard), or has stopped in the yard (given
    /// another order there: the pad is cleared of it, `Sim::update_factory`). The factory notes
    /// the units it builds in its `yard` until they are out of its footprint, and lets go of
    /// those still tied to it then (`factory`: one that never moves).
    fn yard_has_leaving_units(&mut self, factory: UnitId, exit: FxVec3) -> bool {
        let Some(mut yard) = self.unit_mut(factory).map(|f| std::mem::take(&mut f.yard)) else { return false };
        let Some((owner, (fx, fz), (fw, fh))) = self.unit(factory).map(|f| {
            let t = self.types.get(f.type_id);
            let origin = f.occupied_origin.unwrap_or_else(|| World::footprint_origin(World::pos_to_anchor(f.pos, t.footprint), t.footprint));
            (f.owner, origin, t.footprint)
        }) else {
            return false;
        };
        let (mut leaving, mut gone) = (false, Vec::new());
        yard.retain(|&id| {
            let Some(u) = self.unit(id).filter(|u| !u.dead && u.owner == owner && u.carried_by.is_none()) else {
                gone.push(id);
                return false;
            };
            if !u.is_complete() {
                return true;
            }
            let ut = self.types.get(u.type_id);
            // (One that cannot move stays where it was made, holding nothing up.)
            if ut.locomotion == Locomotion::Static {
                gone.push(id);
                return false;
            }
            let (ux, uz) = World::footprint_origin(World::pos_to_anchor(u.pos, ut.footprint), ut.footprint);
            let out = ux >= fx + fw as i32 || uz >= fz + fh as i32 || ux + ut.footprint.0 as i32 <= fx || uz + ut.footprint.1 as i32 <= fz;
            if out {
                gone.push(id);
                return false;
            }
            leaving |= u.path_goal.is_some() || u.orders.front() == Some(&Order::Move(exit));
            true
        });
        // (Out of the yard: no longer the factory's to see out.)
        for id in gone {
            if let Some(u) = self.unit_mut(id)
                && u.factory == Some(factory)
            {
                u.factory = None;
            }
        }
        if let Some(f) = self.unit_mut(factory) {
            f.yard = yard;
        }
        leaving
    }

    /// A point just outside the factory's front (south side at heading 0).
    fn factory_exit(&self, id: UnitId, t: &UnitType) -> FxVec3 {
        let f = self.unit(id).unwrap();
        let forward = heading_vector(f.heading);
        let dist = Fx::from_int(t.footprint.1 as i64 * CELL / 2 + 48);
        let p = f.pos + forward.scale(dist);
        FxVec3::new(p.x, self.world.height_at(p.x, p.z), p.z)
    }

    fn follow_build_pad(&mut self, id: UnitId, factory: UnitId) {
        let types = self.types.clone();
        let Some(f) = self.unit(factory) else { return };
        let t = types.get(f.type_id);
        let pad = self.call_script(factory, "QueryBuildInfo", &[0]).and_then(|p| p.first().copied()).unwrap_or(-1);
        let Some(f) = self.unit(factory) else { return };
        if let Some(mut p) = self.cob_piece_position(f, t, pad) {
            let heading = f.heading;
            // A ship is built at the surface, where it floats once done: a shipyard's pad is under
            // the water (the Core's yard stands 22 px deep). A hovercraft on a pad under the water
            // (a floating hover plant's) is on the water too.
            if let Some(ut) = self.unit(id).map(|u| types.get(u.type_id)) {
                match ut.locomotion {
                    Locomotion::Naval => p.y = self.ground_level(ut, p),
                    Locomotion::Hover => p.y = p.y.max(self.ground_level(ut, p)),
                    _ => {}
                }
            }
            if let Some(u) = self.unit_mut(id) {
                u.pos = p;
                u.heading = heading;
            }
        }
    }

    fn cob_piece_position(&self, u: &Unit, t: &UnitType, cob_piece: i32) -> Option<FxVec3> {
        let cob = u.cob.as_ref()?;
        let mp = cob.program.piece_map.get(usize::try_from(cob_piece).ok()?).copied().flatten()?;
        Some(piece_world_position(u, t, Some(cob), mp))
    }

    // ---- Motion -------------------------------------------------------------------------

    /// One tick of a ground, hover or naval unit's movement along its path: steering, speed,
    /// sliding along obstacles, terrain following. It reads the world and changes only the
    /// unit, so every unit's step can be taken at once; returns the goal to plan a new route
    /// to when the unit got stuck.
    fn ground_step(u: &mut Unit, t: &UnitType, spec: MoveSpec, world: &World, steer: crate::avoid::Steer, tick: u64) -> Option<FxVec3> {
        let Some(goal) = u.path_goal else {
            // Standing in a building (a constructor left on a site as the building went up round
            // it, its way off blocked; a unit pushed into one): onto the nearest free cell, as a
            // unit stuck in a factory's walls is placed (below). From in there no route leads
            // anywhere, so it would never move again.
            if (tick + u.id.slot as u64).is_multiple_of(BUILDING_CHECK_TICKS) && u.speed.is_zero() {
                Self::step_out_of_building(u, t, &spec, world);
            }
            // Idle: brake to a halt.
            u.speed = u.speed.approach(Fx::ZERO, t.brake);
            if u.speed.is_zero() && u.moving {
                u.moving = false;
                if let Some(c) = &mut u.cob {
                    c.start("StopMoving", &[], ThreadTag::None);
                }
            }
            // Arrived with its formation: those that turn standing face the way it faces.
            if let Some(face) = u.face
                && u.speed.is_zero()
            {
                let left = wrap_angle(face - u.heading);
                if t.def.steering_mode < 2 || left.abs() <= t.turn_rate {
                    u.heading = if t.def.steering_mode < 2 { u.heading } else { face };
                    u.face = None;
                } else {
                    u.heading = wrap_angle(u.heading + left.clamp(-t.turn_rate, t.turn_rate));
                }
            }
            return None;
        };

        // Standing on the cell of a waypoint past the next (but the last, the goal's own), the
        // route goes on from there: one that steered for it ahead of the next (`Sim::carrot`).
        let here = World::pos_to_anchor(u.pos, t.footprint);
        if u.path.len() > 2
            && let Some(k) = u.path[1..u.path.len() - 1].iter().take(CARROT_WAYPOINTS).position(|&c| c == here)
        {
            u.path.drain(..=k + 1);
            u.new_leg();
        }

        // Current waypoint: next path cell, or the exact goal on the final leg. (Only where it
        // lies on the plane counts: the ground's height under it was looked up for nothing.)
        let mut waypoint = if u.path.len() > 1 { anchor_middle(u.path[0], t.footprint) } else { goal };
        let mut to = waypoint - u.pos;
        let mut dist = to.length_xz();
        let final_leg = u.path.len() <= 1;
        let reach = if final_leg { u.speed.max(Fx::from_int(3)) } else { Fx::from_int(10).max(u.speed * 3) };
        // In a crowd the way may be taken. Blocked by others, each step still succeeds and the
        // crowd pushes it back (the stuck count never rises), so a unit counts how long the
        // crowd has held it off where it is heading (its goal on the last leg, else its next
        // waypoint):
        // - on the last leg, near its goal (a crowd's reach: about 150 of its kind round one
        //   spot), pushed back for a second without coming any nearer, it has arrived;
        // - on the way, with others on its next waypoint, it goes on to the one after when the
        //   way there is open from where it stands (below);
        // - held up head on by its own side's units coming the other way (two meeting in a gap
        //   one unit wide), or standing idle in its way there (an army gathered in a pass), it
        //   slips past them for a while (`Sim::resolve_collisions`).
        let left = (if final_leg { goal } else { waypoint } - u.pos).length_xz();
        if left + ARRIVE_GAIN <= u.approach_best {
            u.approach_best = left;
            u.approach_idle = 0;
            u.head_on_idle = 0;
        } else {
            // (Pushed back by the crowd: a unit turning round, or held up by the ground, is not
            // counted.)
            if tick <= u.jostled_tick + 1 {
                u.approach_idle += 1;
            }
            if tick <= u.head_on_tick + 1 {
                u.head_on_idle += 1;
            }
        }
        // (One of a formation comes to its own place: held off it, it stops a couple of its sizes
        // short at most, not a crowd's reach, in another's place.)
        let reach_radii = if u.formations.is_empty() { ARRIVE_REACH } else { 2 };
        if final_leg && (u.stuck_ticks > 12 && dist < t.radius * 3 || u.approach_idle >= ARRIVE_IDLE && left < t.radius * reach_radii) {
            u.path.clear();
            u.path_goal = None;
            // (The end of a partial leg: planned on from here.)
            if u.path_partial {
                u.path_partial = false;
                u.move_request = None;
            }
            return None;
        }
        if u.head_on_idle >= SQUEEZE_IDLE {
            u.head_on_idle = 0;
            u.squeeze_until = tick + SQUEEZE_TICKS;
        }
        // A waypoint only counts once the next leg is open from where the unit stands: in
        // reach of it but a cell short, the straight line on can graze a tree the route itself
        // goes round (and a new plan would lead it back to the same spot).
        let next_leg_open = |u: &Unit| match (u.path.first(), u.path.get(1)) {
            (Some(&w), Some(&n)) => here == w || crate::path::line_clear(world, &spec, here, n, u.factory.map(|f| f.slot)),
            _ => true,
        };
        let mut next = dist <= reach && next_leg_open(u);
        // Come past it close by (on the far side of it from the next leg's point of view: a
        // turning circle too wide to come within reach, a tank at a corner), it goes on from
        // where it is when the next leg is open from there, rather than come round for it.
        let omega = turn_omega(t);
        if !next
            && let Some(&n) = u.path.get(1)
            && within_turns(dist, u.speed, omega, 1, 2 * CELL, LOOK_MAX)
            && (u.pos - waypoint).xz().dot((anchor_middle(n, t.footprint) - waypoint).xz()) > Fx::ZERO
        {
            next = next_leg_open(u);
        }
        if !next && !final_leg && u.approach_idle >= STALL_IDLE {
            u.approach_idle = 0;
            next = next_leg_open(u);
        }
        if next {
            if !u.path.is_empty() {
                u.path.remove(0);
                u.new_leg();
            }
            if u.path.is_empty() && dist <= u.speed.max(Fx::from_int(3)) {
                // Arrived (at the goal, or at the end of a partial leg: re-plan from here).
                u.path_goal = None;
                if u.path_partial {
                    u.path_partial = false;
                    u.move_request = None;
                }
                return None;
            }
            waypoint = if u.path.len() > 1 { anchor_middle(u.path[0], t.footprint) } else { goal };
            to = waypoint - u.pos;
            dist = to.length_xz();
        }

        // Steering (bent round the units ahead when they give way), for the next waypoint or, close
        // to it, one further down a zigzag (`Sim::carrot`). TA's `SteeringMode`: most kbots and
        // the Commanders (2) turn standing; tanks, hovercraft, ships and the kbots that build or
        // see radar (1) must be moving to turn, setting off the way they face and coming round
        // in an arc.
        let target = if u.path.len() >= 2 { Self::carrot(u, t, &spec, world, here, waypoint, dist) } else { waypoint };
        let to_target = (target - u.pos).xz();
        let desired = wrap_angle(heading_of(to_target) + steer.bend);
        // (Standing where it can't, a building laid over it or a wreck fallen on it, a unit turns
        // on the spot to walk straight out: a construction vehicle stepping aside came round in
        // an arc over a cliff.)
        let ignore = u.factory.map(|f| f.slot);
        let current_ok = world.passable(&spec, here, ignore);
        let pivots = t.def.steering_mode >= 2 || !current_ok;
        if pivots || u.speed > Fx::ZERO {
            u.heading = wrap_angle(u.heading + wrap_angle(desired - u.heading).clamp(-t.turn_rate, t.turn_rate));
        }
        let off = wrap_angle(desired - u.heading);

        // Speed: slowing into turns, and for the final point. Turning at its turn rate Ï‰ a unit
        // at speed v runs round a circle of radius v / Ï‰; the place it steers for, d away and Î¸
        // off its heading, lies on the circle along its heading of radius d / (2 sin Î¸): faster
        // than Ï‰Â·d / (2 sin Î¸) it would circle the place (and run past waypoints, round and
        // round). Behind it (Î¸ past a right angle), it comes round on a circle down to d / 4 at
        // the most (2 - sin Î¸ for sin Î¸). Units that turn standing also slow down by how far they
        // face from their way, and stop to turn round; the others keep way on.
        let mut target_speed = if pivots { t.max_speed * cos(off).max(Fx::ZERO) } else { t.max_speed };
        let sin_off = if off.abs() >= QUARTER_TURN { Fx::from_int(2) - sin(off).abs() } else { sin(off).abs() };
        // (Ï‰Â·d over 2 sin Î¸ worked out only where it is the slower: no division going straight.)
        let round = omega * if target == waypoint { dist } else { to_target.length_xz() };
        if round < target_speed * sin_off * 2 {
            target_speed = round / (sin_off * 2);
        }
        if !pivots {
            target_speed = target_speed.max(t.acceleration);
        }
        // Behind a slower unit with no room to pass: keep behind it.
        if let Some(cap) = steer.speed_cap {
            target_speed = target_speed.min(cap.max(Fx::ZERO));
        }
        if u.path.len() <= 1 {
            // vÂ² = 2Â·aÂ·d â†’ brake in time for the goal.
            let stop_speed = (Fx::from_int(2) * t.brake * dist).sqrt();
            target_speed = target_speed.min(stop_speed.max(Fx::from_ratio(1, 4)));
        }
        u.speed = if u.speed < target_speed { (u.speed + t.acceleration).min(target_speed) } else { (u.speed - t.brake).max(target_speed) };

        if !u.moving && u.speed > Fx::ZERO {
            u.moving = true;
            if let Some(c) = &mut u.cob {
                c.start("StartMoving", &[], ThreadTag::None);
            }
        }

        // Move, sliding along obstacles when the direct step is blocked.
        let step = heading_vector(u.heading).scale(u.speed);
        let try_pos = |p: FxVec3| world.ground_segment_open(&spec, u.pos, p, ignore, !current_ok);
        let candidates = [u.pos + step, u.pos + FxVec3::new(step.x, Fx::ZERO, Fx::ZERO), u.pos + FxVec3::new(Fx::ZERO, Fx::ZERO, step.z)];
        match candidates.iter().position(|&p| try_pos(p)) {
            Some(k) if u.speed > Fx::ZERO => {
                // Sliding along an obstacle gets round a corner; sliding without getting any
                // closer is a wall across the way (a unit that drifted off its route): after a
                // while the route is planned again from here.
                let closer = (waypoint - candidates[k]).length_xz() < dist - u.speed / 4;
                u.pos.x = candidates[k].x;
                u.pos.z = candidates[k].z;
                if k == 0 || closer {
                    u.stuck_ticks = u.stuck_ticks.saturating_sub(1);
                } else {
                    u.stuck_ticks += 1;
                }
            }
            _ => {
                // (Turning on the spot is not being stuck: a slow-turning unit counted as stuck
                // before it faced its way, planned again from its cell's middle and turned again;
                // hovercraft setting off on a long trip dithered so for up to ten seconds.)
                if u.speed > Fx::ZERO || dist > reach && off.abs() <= t.turn_rate {
                    u.stuck_ticks += 1;
                }
                u.speed = u.speed.min(t.acceleration);
            }
        }
        // Leaving the factory: it ignores the factory's walls (its way out was planned through
        // them) until it is out of the factory's footprint, where the factory lets it go
        // (`Sim::yard_has_leaving_units`; let go on its first free cell, on the pad, a unit
        // found its way out blocked by the walls and went round the factory). A unit stuck
        // inside the walls (exit blocked by terrain) is placed on the nearest free cell so the
        // factory never jams (before the stuck count starts over and a new route is asked
        // for, below).
        let mut placed = false;
        if u.factory.is_some() {
            let here = World::pos_to_anchor(u.pos, t.footprint);
            if u.stuck_ticks > 20 && !world.passable(&spec, here, None) {
                'search: for r in 1i32..16 {
                    for dz in -r..=r {
                        for dx in -r..=r {
                            if dx.abs() != r && dz.abs() != r {
                                continue;
                            }
                            let c = (here.0 + dx, here.1 + dz);
                            if world.passable(&spec, c, None) {
                                let p = world.anchor_to_pos(c, t.footprint);
                                u.pos.x = p.x;
                                u.pos.z = p.z;
                                u.factory = None;
                                placed = true;
                                break 'search;
                            }
                        }
                    }
                }
            }
        }

        Self::settle(u, t, world);

        if u.stuck_ticks > 20 || placed {
            u.stuck_ticks = 0;
            // Back onto the grid its route is planned on: off the middle of its cell, the
            // straight line to the next waypoint can clip a tree's or a building's corner,
            // again after every new plan (at most half a cell's nudge).
            let here = World::pos_to_anchor(u.pos, t.footprint);
            if world.passable(&spec, here, ignore) {
                let p = world.anchor_to_pos(here, t.footprint);
                u.pos.x = p.x;
                u.pos.z = p.z;
            }
            return u.path_goal;
        }
        None
    }

    /// A ground unit standing on a building's cells (not one the terrain alone keeps it off) is
    /// placed on the nearest cell it can stand on, within [`BUILDING_ESCAPE_RINGS`]: whether it
    /// was.
    fn step_out_of_building(u: &mut Unit, t: &UnitType, spec: &MoveSpec, world: &World) -> bool {
        let here = World::pos_to_anchor(u.pos, t.footprint);
        if world.passable(spec, here, None) {
            return false;
        }
        let (x0, z0) = World::footprint_origin(here, t.footprint);
        let in_building = (z0..z0 + t.footprint.1 as i32).any(|z| (x0..x0 + t.footprint.0 as i32).any(|x| matches!(world.occupant(x, z), Occupant::Unit(_))));
        if !in_building {
            return false;
        }
        for r in 1..=BUILDING_ESCAPE_RINGS {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue;
                    }
                    let c = (here.0 + dx, here.1 + dz);
                    if world.passable(spec, c, None) {
                        let p = world.anchor_to_pos(c, t.footprint);
                        u.pos.x = p.x;
                        u.pos.z = p.z;
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Where a unit on its way steers for: its next waypoint (`waypoint`, `dist` away), or, close
    /// to it, further down a zigzag. Routes never cut a blocked corner, so through a corridor
    /// running diagonally across the grid (a narrow pass) they turn at every cell, one way then
    /// the other; a tank steering for each ran past it, its turning circle too wide, and came
    /// round for it again (`Sim::ground_step` slows a unit for turns, but a Flash brakes little).
    /// Within twice its turning circle's radius of its waypoint (`LOOK_RADII`), the unit steers
    /// for the furthest waypoint of a zigzag after it (each a cell's step from the last, the
    /// steps turning one way then back, two of them at least) that it can drive straight to from
    /// its cell `here`, sliding past a corner where the other side is open
    /// (`path::line_slides`): it drives down the corridor's middle.
    fn carrot(u: &Unit, t: &UnitType, spec: &MoveSpec, world: &World, here: (i32, i32), waypoint: FxVec3, dist: Fx) -> FxVec3 {
        if !within_turns(dist, u.speed, turn_omega(t), LOOK_RADII, LOOK_MIN, LOOK_MAX) {
            return waypoint;
        }
        let ignore = u.factory.map(|f| f.slot);
        let mut best = waypoint;
        // (Only down a zigzag: at a single corner the unit goes to the corner. Steering for the
        // cell round it, it slid along the wall that makes the corner, slower and slower as it
        // turned towards that cell.)
        let mut steps: [Option<(i32, i32)>; 2] = [None, None];
        for (k, w) in u.path.windows(2).take(CARROT_WAYPOINTS).enumerate() {
            let step = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            if step.0.abs() + step.1.abs() != 1 || steps[k % 2].is_some_and(|s| s != step) || steps[(k + 1) % 2] == Some(step) {
                break;
            }
            steps[k % 2] = Some(step);
            if k == 0 {
                continue;
            }
            if !crate::path::line_slides(world, spec, here, w[1], ignore) {
                break;
            }
            best = anchor_middle(w[1], t.footprint);
        }
        best
    }

    /// Terrain following and tilt: on the ground (hovercraft and ships on the water) and,
    /// unless it stands level (upright, or a big walker: `UnitType::level`), tilted with the
    /// slope under it.
    fn settle(u: &mut Unit, t: &UnitType, world: &World) {
        let ground = world.height_at(u.pos.x, u.pos.z);
        u.pos.y = match t.locomotion {
            Locomotion::Hover | Locomotion::Naval => ground.max(world.sea_level),
            _ => ground,
        };
        if !t.level() && t.locomotion != Locomotion::Naval {
            // The slope under its whole footprint (half its length and width each way, 16
            // units a cell): a tank rides its tracks over a bump rather than tipping with
            // every cell of the height map under its middle.
            let (half_l, half_w) = (Fx::from_int(t.footprint.1.clamp(1, 8) as i64 * 8), Fx::from_int(t.footprint.0.clamp(1, 8) as i64 * 8));
            let fwd = heading_vector(u.heading).scale(half_l);
            let left = rotate_y(FxVec3::new(half_w, Fx::ZERO, Fx::ZERO), u.heading);
            let h = |p: FxVec3| {
                let g = world.height_at(p.x, p.z);
                if t.locomotion == Locomotion::Hover { g.max(world.sea_level) } else { g }
            };
            let (hf, hb) = (h(u.pos + fwd), h(u.pos - fwd));
            let (hl, hr) = (h(u.pos + left), h(u.pos - left));
            u.pitch = -atan2(hf - hb, half_l * 2);
            u.roll = atan2(hl - hr, half_w * 2);
        }
    }

    /// Tells a script what TA's engine tells it every so often: what the unit moves over
    /// (`SetSFXoccupy`: 4 land, 1 shallow water, 2 deep water, 0 carried; amphibians swim
    /// by it), and for a builder at work where its nano spray leaves from.
    fn feed_script(&mut self, id: UnitId, t: &UnitType) {
        let c = t.callins;
        if !c.set_sfx_occupy && !c.query_nano_piece {
            return;
        }
        let Some(u) = self.unit(id) else { return };
        let phase = self.tick + id.slot as u64;
        let (carried, pos, told, working) = (u.carried_by.is_some(), u.pos, u.sfx_occupy, u.in_build_stance || u.build.is_some());
        if c.set_sfx_occupy && phase.is_multiple_of(8) {
            let depth = self.world.sea_level - self.world.height_at(pos.x, pos.z);
            let occupy = if carried {
                0
            } else if depth > Fx::from_int(8) {
                2
            } else if depth > Fx::ZERO {
                1
            } else {
                4
            };
            if occupy != told {
                if let Some(u) = self.unit_mut(id) {
                    u.sfx_occupy = occupy;
                }
                self.call_script(id, "SetSFXoccupy", &[occupy as i32]);
            }
        }
        if c.query_nano_piece && phase.is_multiple_of(3) {
            let piece = if working { self.call_script(id, "QueryNanoPiece", &[0]).and_then(|p| p.first().copied()).and_then(|p| u32::try_from(p).ok()) } else { None };
            if let Some(u) = self.unit_mut(id) {
                u.nano_piece = piece;
            }
        }
    }

    // ---- Economy ------------------------------------------------------------------------

    fn economy_begin(&mut self) {
        for p in &mut self.players {
            let e = &mut p.economy;
            e.metal_demand = e.metal_demand_next;
            e.energy_demand = e.energy_demand_next;
            e.metal_demand_next = Fx::ZERO;
            e.energy_demand_next = Fx::ZERO;
            let fm = if e.metal_demand > Fx::ZERO { (e.metal / e.metal_demand).min(Fx::ONE) } else { Fx::ONE };
            let fe = if e.energy_demand > Fx::ZERO { (e.energy / e.energy_demand).min(Fx::ONE) } else { Fx::ONE };
            e.build_factor = fm.min(fe).max(Fx::ZERO);
            e.tick_metal_in = Fx::ZERO;
            e.tick_energy_in = Fx::ZERO;
        }
    }

    fn economy_end(&mut self) {
        let types = self.types.clone();
        let n = self.players.len();
        let mut metal_cap: Vec<Fx> = self.players.iter().map(|p| p.base_storage.0).collect();
        let mut energy_cap: Vec<Fx> = self.players.iter().map(|p| p.base_storage.1).collect();
        debug_assert_eq!(metal_cap.len(), n);
        let wind = self.wind;
        let tidal = self.tidal;
        let resources = self.mutators.resources;
        let per_tick = |v: Fx| v / TICKS_PER_SECOND;
        // A transport's cargo makes and pays nothing; a module on the building that took it on (a
        // unit that carries none: TA: Escalation's fusion reactor shield upgrade, +250 energy)
        // works as it would standing. (The cargo's slots, in order: none, mostly.)
        let cargo: Vec<usize> = self
            .units
            .iter()
            .enumerate()
            .filter(|(_, u)| u.as_ref().and_then(|u| u.carried_by).is_some_and(|c| self.unit(c).is_none_or(|c| types.get(c.type_id).transport_capacity > 0)))
            .map(|(slot, _)| slot)
            .collect();
        for (slot, u) in self.units.iter_mut().enumerate() {
            let Some(u) = u else { continue };
            if !u.is_complete() || u.dead {
                continue;
            }
            let t = types.get(u.type_id);
            let o = u.owner as usize;
            metal_cap[o] += t.metal_storage;
            energy_cap[o] += t.energy_storage;
            if cargo.binary_search(&slot).is_ok() || self.tick < u.paralyzed_until {
                u.upkeep_paid = false;
                continue;
            }
            let e = &mut self.players[o].economy;
            let mut energy_in = t.energy_make;
            let mut metal_in = t.metal_make;
            if t.wind_generator > Fx::ZERO {
                energy_in += per_tick(wind.min(t.wind_generator));
            }
            if t.tidal_generator > Fx::ZERO {
                energy_in += per_tick(tidal * t.tidal_generator);
            }
            // EnergyUse counts only while the unit is switched on, which TA does when it is built
            // (ActivateWhenBuilt), the player does (OnOffable) or its script does (ACTIVATION):
            // upkeep is paid then, and a negative one is production then (solar collectors). One
            // that is none idles for free (the Galactic Gate), and makes no more than its
            // EnergyMake: Total Mayhem's fusion reactor, 1,050 ("Produces Energy (1K)", its
            // EnergyUse of -1,050 never switched on), TA: Escalation's 1,000. A unit with no
            // EnergyUse and no switch extracts and makes metal as it stands.
            let active = u.activated || !t.def.on_offable && t.energy_use.is_zero();
            let mut upkeep = Fx::ZERO;
            if active {
                if t.energy_use < Fx::ZERO {
                    energy_in -= t.energy_use;
                } else {
                    upkeep = t.energy_use;
                }
            }
            let paid = upkeep.is_zero() || e.energy >= upkeep;
            // (For the figures the interface shows: `Sim::unit_flows`.)
            u.upkeep_paid = paid;
            if paid && upkeep > Fx::ZERO {
                e.energy -= upkeep;
                e.tick_energy_out += upkeep;
            }
            if active && paid {
                if t.extracts_metal > Fx::ZERO && let Some(origin) = u.occupied_origin {
                    let under = self.world.metal_under(origin, t.footprint);
                    metal_in += per_tick(t.extracts_metal * Fx::from_int(under));
                }
                if t.makes_metal > Fx::ZERO {
                    metal_in += t.makes_metal;
                }
            }
            // (The resources mutator's flow.)
            let (energy_in, metal_in) = (crate::mutators::income(energy_in, resources), crate::mutators::income(metal_in, resources));
            e.energy += energy_in;
            e.metal += metal_in;
            e.tick_energy_in += energy_in;
            e.tick_metal_in += metal_in;
        }
        // What a player shares (by default what full storage would waste) goes to allies with
        // room, in player order (see `Sharing`).
        let n = self.players.len();
        let spill: Vec<(Fx, Fx)> = self
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let s = &p.sharing;
                let keep_m = s.keep_metal.map_or(metal_cap[i], |k| k.max(Fx::ZERO).min(metal_cap[i]));
                let keep_e = s.keep_energy.map_or(energy_cap[i], |k| k.max(Fx::ZERO).min(energy_cap[i]));
                let m = if s.metal { (p.economy.metal - keep_m).max(Fx::ZERO) } else { Fx::ZERO };
                let e = if s.energy { (p.economy.energy - keep_e).max(Fx::ZERO) } else { Fx::ZERO };
                (m, e)
            })
            .collect();
        for (i, &(mut metal, mut energy)) in spill.iter().enumerate() {
            if metal <= Fx::ZERO && energy <= Fx::ZERO {
                continue;
            }
            let (offered_m, offered_e) = (metal, energy);
            let allies: Vec<usize> = (0..n).filter(|&j| j != i && self.allied(i as u8, j as u8) && !self.players[j].defeated).collect();
            for j in allies {
                if metal <= Fx::ZERO && energy <= Fx::ZERO {
                    break;
                }
                let e = &mut self.players[j].economy;
                let m = metal.min((metal_cap[j] - e.metal).max(Fx::ZERO));
                let en = energy.min((energy_cap[j] - e.energy).max(Fx::ZERO));
                e.metal += m;
                e.energy += en;
                metal -= m;
                energy -= en;
            }
            // (What went over the storage would go anyway; what was kept back under it is
            // taken from the giver as it is given.)
            let e = &mut self.players[i].economy;
            e.metal -= offered_m - metal;
            e.energy -= offered_e - energy;
        }
        for (i, p) in self.players.iter_mut().enumerate() {
            let e = &mut p.economy;
            e.metal_capacity = metal_cap[i];
            e.energy_capacity = energy_cap[i];
            e.metal = e.metal.clamp(Fx::ZERO, e.metal_capacity);
            e.energy = e.energy.clamp(Fx::ZERO, e.energy_capacity);
            if p.cheats.unlimited {
                e.metal = e.metal_capacity;
                e.energy = e.energy_capacity;
            }
            p.stats.metal_produced += e.tick_metal_in;
            p.stats.energy_produced += e.tick_energy_in;
            p.stats.metal_spent += e.tick_metal_out;
            p.stats.energy_spent += e.tick_energy_out;
            e.metal_income = e.tick_metal_in * TICKS_PER_SECOND;
            e.energy_income = e.tick_energy_in * TICKS_PER_SECOND;
            e.metal_expense = e.tick_metal_out * TICKS_PER_SECOND;
            e.energy_expense = e.tick_energy_out * TICKS_PER_SECOND;
            e.tick_metal_out = Fx::ZERO;
            e.tick_energy_out = Fx::ZERO;
        }
    }

    /// Every thirty seconds: income, army value and unit count per player, for the
    /// statistics graphs.
    fn sample_stats(&mut self) {
        if !self.tick.is_multiple_of(crate::unit::PlayerStats::SAMPLE_TICKS) && self.tick != 1 {
            return;
        }
        let n = self.players.len();
        let mut army = vec![Fx::ZERO; n];
        for u in self.units.iter().flatten() {
            let t = self.types.get(u.type_id);
            if !u.dead && u.is_complete() && t.locomotion != Locomotion::Static && t.weapons.iter().any(|w| w.is_some()) && let Some(a) = army.get_mut(u.owner as usize) {
                *a += t.cost_metal + t.cost_energy / 60;
            }
        }
        let tick = self.tick;
        for (i, p) in self.players.iter_mut().enumerate() {
            let sample = crate::unit::StatSample { tick, metal_income: p.economy.metal_income, energy_income: p.economy.energy_income, army_value: army[i], units: p.unit_count };
            p.stats.history.push(sample);
        }
    }

    fn remove_dead(&mut self) {
        let types = self.types.clone();
        let tick = self.tick;
        for slot in 0..self.units.len() {
            let dead = self.units[slot].as_ref().is_some_and(|u| u.dead && u.death_processed);
            if !dead {
                continue;
            }
            if let Some(id) = self.units[slot].as_ref().map(|u| u.id) {
                self.finish_capture_job(id);
            }
            let u = self.units[slot].take().unwrap();
            let t = types.get(u.type_id);
            let complete = u.is_complete();
            // Reclaim and scenario removal bypass death explosions and their cleanup.
            // Release every product here too, including unfinished frames that another
            // constructor can finish once the factory is gone.
            // (An unfinished one has the while from now before it un-builds: a builder may yet be
            // sent to finish it.)
            for product in u.yard {
                if let Some(product) = self.unit_mut(product) && product.factory == Some(u.id) {
                    product.factory = None;
                    if !product.is_complete() {
                        product.worked_at = Some(tick);
                    }
                }
            }
            if let Some(carrier) = u.carried_by.and_then(|id| self.unit_mut(id)) {
                carrier.cargo.retain(|&passenger| passenger != u.id);
            }
            if let Some(origin) = u.occupied_origin {
                self.world.release(Occupant::Unit(slot as u32), origin, t.footprint);
            }
            self.events.push(SimEvent::UnitDestroyed { id: u.id, type_id: u.type_id, owner: u.owner, pos: u.pos, killer: u.killer, removed: u.removed, complete });
            if !u.removed {
                *self.deaths_by.entry((u.owner, u.type_id)).or_default() += 1;
            }
            if let Some(p) = self.players.get_mut(u.owner as usize) {
                p.unit_count = p.unit_count.saturating_sub(1);
                // (One the set-up took off the map was not lost.)
                p.stats.lost += !u.removed as u64;
            }
            self.free_slots.push(slot as u32);
        }
    }

    /// Puts a player on a team (see [`Player::team`]).
    pub fn set_team(&mut self, player: u8, team: u8) {
        if let Some(p) = self.players.get_mut(player as usize) {
            p.team = team;
        }
    }

    /// Same player or same team.
    #[inline]
    /// Each player's allies, its own number among them (bit n: player n), as scripts ask
    /// (`UNIT_ALLIED`).
    fn ally_masks(&self) -> Vec<u64> {
        let players = self.players.len().min(64);
        (0..players as u8).map(|a| (0..players as u8).filter(|&b| self.allied(a, b)).fold(0u64, |mask, b| mask | 1 << b)).collect()
    }

    pub fn allied(&self, a: u8, b: u8) -> bool {
        a == b || self.players.get(a as usize).zip(self.players.get(b as usize)).is_some_and(|(pa, pb)| pa.team == pb.team)
    }

    #[inline]
    pub fn is_enemy(&self, a: u8, b: u8) -> bool {
        !self.allied(a, b)
    }

    /// Mask of the players `player` is at war with (a [`player_bit`] each).
    pub fn enemy_mask(&self, player: u8) -> u64 {
        (0..self.players.len() as u8).filter(|&p| self.is_enemy(player, p)).fold(0, |m, p| m | player_bit(p))
    }

    /// The vision grid a player sees through: teams share their lowest-numbered member's.
    #[inline]
    pub fn vision_slot(&self, player: u8) -> u8 {
        let Some(team) = self.players.get(player as usize).map(|p| p.team) else { return player };
        self.players.iter().position(|p| p.team == team).map_or(player, |i| i as u8)
    }

    /// Mask of `player` and its allies (for per-player sets such as cloak reveals).
    fn team_mask(&self, player: u8) -> u64 {
        let mut mask = player_bit(player);
        for (i, p) in self.players.iter().enumerate() {
            if self.players.get(player as usize).is_some_and(|me| me.team == p.team) {
                mask |= player_bit(i as u8);
            }
        }
        mask
    }

    /// Whether a player owns as many units as the limit allows.
    pub fn at_unit_limit(&self, player: u8) -> bool {
        self.players.get(player as usize).is_some_and(|p| p.unit_count >= self.unit_limit)
    }

    /// Sets a player's starting resources; storage grows to hold them.
    pub fn set_starting_resources(&mut self, player: u8, metal: Fx, energy: Fx) {
        let Some(p) = self.players.get_mut(player as usize) else { return };
        p.economy.metal = metal;
        p.economy.energy = energy;
        p.base_storage = (metal.max(Fx::from_int(BASE_STORAGE)), energy.max(Fx::from_int(BASE_STORAGE)));
    }

    /// Unit type ids a unit can build.
    pub fn build_options(&self, id: UnitId) -> &[usize] {
        self.unit(id).map_or(&[], |u| &self.types.get(u.type_id).build_options)
    }

    /// Whether a building of `type_id` fits centred at `pos` (for placement previews).
    pub fn can_place(&self, type_id: usize, pos: FxVec3) -> bool {
        self.placement_error(type_id, pos).is_none()
    }

    pub fn placement_error(&self, type_id: usize, pos: FxVec3) -> Option<crate::world::PlacementError> {
        let t = self.types.get(type_id);
        let anchor = World::pos_to_anchor(pos, t.footprint);
        let origin = World::footprint_origin(anchor, t.footprint);
        self.world.placement_error(t, self.move_specs[type_id].max_slope.max(t.def.max_slope as i32), origin)
    }

    /// Sets a unit's experience (scenario set-up: veterans brought into a battle).
    pub fn set_experience(&mut self, id: UnitId, xp: Fx) {
        if let Some(u) = self.unit_mut(id) {
            u.xp = xp.max(Fx::ZERO);
        }
    }

    /// Adds orders to a unit's queue (scenario set-up: patrols, standing orders).
    pub fn give_orders(&mut self, id: UnitId, orders: impl IntoIterator<Item = Order>) {
        if let Some(u) = self.unit_mut(id) {
            u.orders.extend(orders);
        }
    }

    /// Sets a unit's health, kept between 1 and its most (scenario set-up: the tutorial's
    /// damaged building to mend).
    pub fn set_health(&mut self, id: UnitId, health: Fx) {
        let Some(most) = self.unit(id).map(|u| self.unit_type(u).max_health) else { return };
        if let Some(u) = self.unit_mut(id) {
            u.health = health.max(Fx::ONE).min(most);
        }
    }

    /// Takes a unit off the map without a Killed script, wreck or blast (scenario set-up,
    /// e.g. the tutorial's). It is gone after the next tick.
    pub fn remove_unit(&mut self, id: UnitId) {
        if let Some(u) = self.unit_mut(id) {
            u.dead = true;
            u.death_processed = true;
            u.removed = true;
        }
    }

    /// Nearest valid site for a building around `near` (spiral search up to `max_cells`).
    pub fn find_build_site(&self, type_id: usize, near: FxVec3, max_cells: i32) -> Option<FxVec3> {
        let t = self.types.get(type_id);
        let center = World::pos_to_anchor(near, t.footprint);
        let slope = self.move_specs[type_id].max_slope.max(t.def.max_slope as i32);
        for r in 0..=max_cells {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue;
                    }
                    let anchor = (center.0 + dx, center.1 + dz);
                    let origin = World::footprint_origin(anchor, t.footprint);
                    if self.world.can_place(t, slope, origin) {
                        return Some(self.world.anchor_to_pos(anchor, t.footprint));
                    }
                }
            }
        }
        None
    }

    /// Metal per second an extractor of `type_id` built at `pos` would yield (zero for
    /// other buildings).
    pub fn extraction_at(&self, type_id: usize, pos: FxVec3) -> Fx {
        let t = self.types.get(type_id);
        let origin = World::footprint_origin(World::pos_to_anchor(pos, t.footprint), t.footprint);
        t.extracts_metal * Fx::from_int(self.world.metal_under(origin, t.footprint))
    }

    /// Where within `radius` cells of `pos` an extractor of `type_id` would yield the most
    /// metal (among sites it can be built on; the nearest of equals). `pos` snapped to the
    /// grid when nothing around beats it.
    pub fn best_extraction_site(&self, type_id: usize, pos: FxVec3, radius: i32) -> FxVec3 {
        let t = self.types.get(type_id);
        let slope = self.move_specs[type_id].max_slope.max(t.def.max_slope as i32);
        let centre = World::pos_to_anchor(pos, t.footprint);
        let mut best: Option<(i64, i32, (i32, i32))> = None;
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let anchor = (centre.0 + dx, centre.1 + dz);
                let origin = World::footprint_origin(anchor, t.footprint);
                if !self.world.can_place(t, slope, origin) {
                    continue;
                }
                let (metal, d2) = (self.world.metal_under(origin, t.footprint), dx * dx + dz * dz);
                if best.is_none_or(|(m, d, _)| metal > m || metal == m && d2 < d) {
                    best = Some((metal, d2, anchor));
                }
            }
        }
        let anchor = best.filter(|&(m, _, _)| m > 0).map_or(centre, |(_, _, a)| a);
        self.world.anchor_to_pos(anchor, t.footprint)
    }

    /// Snaps a position to the building grid for `type_id`.
    /// How units of a type move (footprint, terrain limits), for movement maps.
    pub fn move_spec(&self, type_id: usize) -> crate::world::MoveSpec {
        self.move_specs[type_id]
    }

    pub fn snap_to_grid(&self, type_id: usize, pos: FxVec3) -> FxVec3 {
        let t = self.types.get(type_id);
        self.world.anchor_to_pos(World::pos_to_anchor(pos, t.footprint), t.footprint)
    }
}

impl Sim {
    /// A hash of the simulation state (tick, units, projectiles, economies) for detecting
    /// desynchronised lockstep peers. Deterministic: integers only, slot order.
    pub fn checksum(&self) -> u64 {
        use crate::unit::Target;
        fn target(mix: &mut (impl FnMut(i64) + ?Sized), target: Option<Target>) {
            match target {
                None => mix(0),
                Some(Target::Ground(p)) => { mix(1); mix(p.x.0); mix(p.y.0); mix(p.z.0); }
                Some(Target::Unit(id)) => { mix(2); mix(id.slot as i64); mix(id.generation as i64); }
                Some(Target::Projectile(id)) => { mix(3); mix(id as i64); }
            }
        }
        fn order(mix: &mut dyn FnMut(i64), order: &Order) {
            match order {
                Order::Move(p) => { mix(1); target(mix, Some(Target::Ground(*p))); }
                Order::Attack(t) => { mix(2); target(mix, Some(*t)); }
                Order::Fight(p) => { mix(3); target(mix, Some(Target::Ground(*p))); }
                Order::Reclaim(i) => { mix(4); mix(*i as i64); }
                Order::Resurrect(i) => { mix(5); mix(*i as i64); }
                Order::ReclaimUnit(id) => { mix(6); target(mix, Some(Target::Unit(*id))); }
                Order::ManualFire(t) => { mix(7); target(mix, Some(*t)); }
                Order::Build { type_id, pos } => { mix(8); mix(*type_id as i64); target(mix, Some(Target::Ground(*pos))); }
                Order::Repair(id) => { mix(9); target(mix, Some(Target::Unit(*id))); }
                Order::Patrol(p) => { mix(10); target(mix, Some(Target::Ground(*p))); }
                Order::Guard(id) => { mix(11); target(mix, Some(Target::Unit(*id))); }
                Order::Capture(id) => { mix(12); target(mix, Some(Target::Unit(*id))); }
                Order::Wait(t) => { mix(13); mix(*t as i64); }
                Order::WaitAttacked(id) => { mix(14); target(mix, id.map(Target::Unit)); }
                Order::AttackType(t) => { mix(15); mix(*t as i64); }
                Order::SetStates { fire, movement } => { mix(16); mix(*fire as i64); mix(*movement as i64); }
                Order::Load(id) => { mix(17); target(mix, Some(Target::Unit(*id))); }
                Order::Unload(p) => { mix(18); target(mix, Some(Target::Ground(*p))); }
                Order::Stop => mix(19),
            }
        }
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |v: i64| {
            for b in v.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        mix(self.tick as i64);
        mix(self.local_avoidance as i64);
        mix(self.group_formations as i64);
        mix(self.next_formation as i64);
        mix(self.formations.len() as i64);
        for f in &self.formations { f.hash_state(&mut mix); }
        mix(self.steers.len() as i64);
        for s in &self.steers { mix(s.bend as i64); mix(s.speed_cap.map_or(-1, |v| v.0)); }
        for u in self.units.iter().flatten() {
            mix(u.id.slot as i64);
            mix(u.id.generation as i64);
            mix(u.type_id as i64);
            mix(u.owner as i64);
            mix(u.pos.x.0);
            mix(u.pos.y.0);
            mix(u.pos.z.0);
            mix(u.fire_state as i64);
            mix(u.move_state as i64);
            mix(u.path.len() as i64);
            for &(x, z) in &u.path { mix(x as i64); mix(z as i64); }
            mix(u.path_partial as i64);
            mix(u.stuck_ticks as i64);
            mix(u.approach_best.0);
            mix(u.approach_idle as i64);
            mix(u.head_on_idle as i64);
            mix(u.jostled_tick as i64);
            mix(u.head_on_tick as i64);
            mix(u.squeeze_until as i64);
            mix(u.face.map_or(-1, |v| v as i64));
            target(&mut mix, u.idle_anchor.map(Target::Ground));
            mix(u.formations.len() as i64);
            for &(id, pos) in &u.formations { mix(id as i64); target(&mut mix, Some(Target::Ground(pos))); }
            mix(u.speed.0);
            mix(u.moving as i64);
            mix(u.landed as i64);
            mix(u.air_pass as i64);
            mix(u.pass_runout as i64);
            mix(u.fan as i64);
            mix(u.orbit.map_or(-1, |o| o.x.0 ^ o.z.0.rotate_left(17)));
            mix(u.drift.map_or(-1, |d| {
                (d.target.slot as i64) ^ (d.since as i64).rotate_left(9) ^ (d.last as i64).rotate_left(23) ^ (d.angle as i64).rotate_left(37)
                    ^ d.distance.0.rotate_left(45) ^ (d.side as i64).rotate_left(57)
            }));
            mix(u.pass_ticks as i64);
            mix(u.idle_ticks as i64);
            mix(u.move_rate as i64);
            mix(u.pitch as i64);
            mix(u.roll as i64);
            mix(u.cruise_alt.map_or(-1, |v| v.0));
            mix(u.hover_alt.map_or(-1, |v| v.0));
            target(&mut mix, u.path_goal.map(Target::Ground));
            target(&mut mix, u.move_request.map(Target::Ground));
            target(&mut mix, u.carried_by.map(Target::Unit));
            mix(u.carry_piece.map_or(-1, |p| p as i64));
            mix(u.cargo.len() as i64);
            for &id in &u.cargo { target(&mut mix, Some(Target::Unit(id))); }
            target(&mut mix, u.transport_job.map(|(id, _)| Target::Unit(id)));
            mix(u.transport_job.map_or(0, |(_, tick)| tick as i64));
            target(&mut mix, u.air_pad.map(|(id, _)| Target::Unit(id)));
            mix(u.air_pad.and_then(|(_, piece)| piece).map_or(-1, |p| p as i64));
            mix(u.repair_trip as i64);
            mix(u.health.0);
            target(&mut mix, u.last_attacker.map(Target::Unit));
            mix(u.last_damage.0);
            mix(u.heading as i64);
            mix(u.build_progress.0);
            mix(u.build_work as i64);
            mix(u.repair_work as i64);
            mix(u.finished as i64);
            mix(u.taken_apart_at.map_or(-1, |t| t as i64));
            mix(u.worked_at.map_or(-1, |t| t as i64));
            mix(u.capture_work as i64);
            target(&mut mix, u.capture.map(|c| Target::Unit(c.target)));
            mix(u.capture.map_or(0, |c| c.ticks as i64));
            mix(u.capture.map_or(0, |c| c.elapsed as i64));
            mix(u.in_build_stance as i64);
            mix(u.yard_open as i64);
            mix(u.activated as i64);
            mix(u.upkeep_paid as i64);
            mix(u.armored as i64);
            mix(u.busy as i64);
            target(&mut mix, u.build.map(|b| Target::Unit(b.target)));
            mix(u.build.is_some_and(|b| b.started) as i64);
            target(&mut mix, u.factory.map(Target::Unit));
            mix(u.orders.len() as i64);
            for o in &u.orders { order(&mut mix, o); }
            mix(u.factory_queue.len() as i64);
            for &t in &u.factory_queue { mix(t as i64); }
            mix(u.factory_repeat as i64);
            mix(u.rally.len() as i64);
            for o in &u.rally { order(&mut mix, o); }
            mix(u.yard.len() as i64);
            for &id in &u.yard { target(&mut mix, Some(Target::Unit(id))); }
            mix(u.stockpile as i64);
            mix(u.stockpile_work as i64);
            for weapon in &u.weapons {
                mix(weapon.reload as i64);
                mix(weapon.burst_left as i64);
                mix(weapon.burst_timer as i64);
                mix(weapon.aimed as i64);
                target(&mut mix, weapon.target);
            }
        }
        mix(self.next_projectile_id as i64);
        mix(self.projectiles.len() as i64);
        for p in &self.projectiles {
            mix(p.id as i64);
            mix(p.weapon as i64);
            mix(p.owner.slot as i64);
            mix(p.owner.generation as i64);
            mix(p.owner_player as i64);
            for v in [p.pos, p.vel, p.target_pos] { mix(v.x.0); mix(v.y.0); mix(v.z.0); }
            target(&mut mix, p.target);
            mix(p.age as i64);
            mix(p.ttl as i64);
            mix(p.dead as i64);
            mix(p.hits.len() as i64);
            for id in &p.hits { mix(id.slot as i64); mix(id.generation as i64); }
        }
        for p in &self.players {
            mix(p.economy.metal.0);
            mix(p.economy.energy.0);
            mix(p.economy.metal_demand_next.0);
            mix(p.economy.energy_demand_next.0);
            mix(p.sharing.metal as i64 | (p.sharing.energy as i64) << 1);
            mix(p.sharing.keep_metal.map_or(-1, |k| k.0));
            mix(p.sharing.keep_energy.map_or(-1, |k| k.0));
            mix(p.shoot_all as i64);
        }
        mix(self.mission.is_some() as i64);
        if let Some(mission) = &self.mission {
            mission.hash_state(&mut mix, &order);
            for player in &self.players {
                mix(player.team as i64); mix(player.defeated as i64); mix(player.had_commander as i64);
                mix(player.endless as i64); mix(player.stats.metal_produced.0);
            }
            // Hash collections in key order, independent of hash-table insertion order.
            let mut deaths: Vec<_> = self.deaths_by.iter().collect();
            deaths.sort_by_key(|(key, _)| **key);
            mix(deaths.len() as i64);
            for (&(owner, unit), &count) in deaths { mix(owner as i64); mix(unit as i64); mix(count as i64); }
            for set in [&self.built, &self.captured] {
                let mut units: Vec<_> = set.iter().copied().collect(); units.sort_unstable();
                mix(units.len() as i64);
                for (owner, unit) in units { mix(owner as i64); mix(unit as i64); }
            }
            mix(self.allowed.is_some() as i64);
            if let Some(allowed) = &self.allowed { mix(allowed.len() as i64); for &v in allowed { mix(v as i64); } }
            let mut targets: Vec<_> = self.no_auto_target.iter().copied().collect(); targets.sort_unstable();
            mix(targets.len() as i64); for unit in targets { mix(unit as i64); }
        }
        mix(self.features.len() as i64);
        for f in &self.features {
            mix(f.alive as i64);
            if !f.alive { continue; }
            mix(f.def as i64);
            mix(f.pos.x.0); mix(f.pos.y.0); mix(f.pos.z.0);
            mix(f.health.0);
            mix(f.reclaim_work as i64);
            mix(f.resurrect_work as i64);
        }
        h
    }
}

impl Sim {
    /// Set-up helper (staged scenes): a frame finished, as the instant-build cheat finishes one
    /// (its health full, its scripts told it is built), with nobody building it.
    pub fn finish_frame(&mut self, id: UnitId) {
        let Some(type_id) = self.unit(id).filter(|u| !u.dead && !u.is_complete()).map(|u| u.type_id) else { return };
        let (total, health) = {
            let t = self.types.get(type_id);
            (t.build_work, t.max_health)
        };
        if let Some(u) = self.unit_mut(id) {
            u.set_build_work(total, total);
            u.health = health;
        }
        self.on_completed(id);
    }

    /// Set-up helper (staged scenes): a feature of definition `name` (a wreck, a rock, a tree)
    /// put down at `pos` facing `heading`, as a unit's wreck is laid; its index, or None when
    /// there is no such feature.
    pub fn place_feature(&mut self, name: &str, pos: FxVec3, heading: i32) -> Option<usize> {
        let def_id = self.data.feature_id(name)?;
        let def = &self.data.features[def_id];
        let fp = (def.footprint_x, def.footprint_z);
        let (blocking, health) = (def.blocking, Fx::from_f64(def.damage.max(1.0)));
        let origin = World::footprint_origin(World::pos_to_anchor(pos, fp), fp);
        let slot = self.features.len() as u32;
        if blocking {
            self.world.occupy(Occupant::Feature(slot), origin, fp, &[]);
        }
        let p = FxVec3::new(pos.x, self.world.height_at(pos.x, pos.z), pos.z);
        let index = self.add_feature(Feature::new(def_id, origin, p, heading, health));
        self.events.push(SimEvent::FeatureCreated { index, wreck_of: None });
        Some(index)
    }

    /// Set-up helper (staged scenes): every flammable feature within `radius` of `pos` set on
    /// fire (as a weapon that always starts fires would); how many caught.
    pub fn ignite_around(&mut self, pos: FxVec3, radius: Fx) -> usize {
        let mut near = Vec::new();
        self.feature_grid.near(pos, radius, &mut near);
        near.sort_unstable();
        let mut lit = 0;
        for i in near {
            let i = i as usize;
            let f = &self.features[i];
            if f.alive && f.burn_until == 0 && f.pos.within_xz(pos, radius) && self.data.features[f.def].flamable {
                self.ignite(i);
                lit += 1;
            }
        }
        lit
    }

    /// Set-up helper (staged scenes): a frame's work done brought up to `work` (never down, and
    /// short of finished: what finishes it is a build's own end, or `finish_frame`), with the
    /// health that work brings as building gives it (`apply_build_power`).
    pub fn advance_frame(&mut self, id: UnitId, work: u64) {
        let Some(type_id) = self.unit(id).filter(|u| !u.dead && !u.is_complete()).map(|u| u.type_id) else { return };
        let (total, max_health) = {
            let t = self.types.get(type_id);
            (t.build_work, t.max_health)
        };
        if let Some(u) = self.unit_mut(id) {
            let done = u.build_work;
            let work = work.min(total.saturating_sub(1)).max(done);
            let heal = max_health.share(work, total) - max_health.share(done, total);
            u.set_build_work(work, total);
            u.health = (u.health + heal).min(max_health);
        }
    }

    /// Scenario/test helper: gives a unit ready-made stockpile missiles.
    pub fn grant_stockpile(&mut self, id: UnitId, missiles: u32) {
        if let Some(u) = self.unit_mut(id) {
            u.stockpile = u.stockpile.saturating_add(missiles).min(crate::combat::MAX_STOCKPILE);
        }
    }

    /// Set-up helper (staged scenes): an aircraft's own cruising height over the ground, in
    /// place of its type's `CruiseAlt` (`None` gives it back). It climbs or sinks to it as it
    /// flies, as to its cruising height.
    pub fn set_cruise_alt(&mut self, id: UnitId, height: Option<Fx>) {
        if let Some(u) = self.unit_mut(id) {
            u.cruise_alt = height.map(|h| h.max(Fx::ONE));
        }
    }

    /// Set-up helper (staged scenes, demos): a finished aircraft already in flight at `pos`,
    /// at its cruising height over the ground there (`height`, if given, is its own from then
    /// on: see [`Sim::set_cruise_alt`]), heading `heading` at full speed, its wings out and its
    /// engines going as an aircraft's are once it has taken off. From there it flies as any
    /// does: given nothing to do it slows and lands, so give it its orders. Other units are
    /// spawned as [`Sim::spawn_unit`] spawns them.
    pub fn spawn_airborne(&mut self, type_id: usize, owner: u8, pos: FxVec3, heading: i32, height: Option<Fx>) -> Option<UnitId> {
        let id = self.spawn_unit(type_id, owner, pos, heading, true)?;
        self.put_in_flight(id, height);
        Some(id)
    }

    /// A finished aircraft just placed, put in flight where it is, as [`Sim::spawn_airborne`]
    /// puts one (a scenario's aircraft coming in over the map's edge). Other units are left as
    /// they are.
    pub fn put_in_flight(&mut self, id: UnitId, height: Option<Fx>) {
        let Some(u) = self.unit(id) else { return };
        let pos = u.pos;
        let types = self.types.clone();
        let t = types.get(u.type_id);
        if t.locomotion != Locomotion::Air {
            return;
        }
        self.set_cruise_alt(id, height);
        let ground = self.world.height_at(pos.x, pos.z).max(self.world.sea_level);
        let top = if t.is_plane() { 2 } else { 3 };
        if let Some(u) = self.unit_mut(id) {
            u.pos.y = ground + u.cruise_alt.unwrap_or(t.cruise_alt);
            u.speed = t.max_speed;
            u.landed = false;
            u.moving = true;
            u.move_rate = top;
        }
        self.set_active(id, true);
        self.start_script(id, "StartMoving", &[]);
        if t.callins.move_rate[top as usize] {
            self.start_script(id, ["MoveRate0", "MoveRate1", "MoveRate2", "MoveRate3"][top as usize], &[]);
        }
    }
}

#[cfg(test)]
#[path = "sim_tests.rs"]
mod tests;
