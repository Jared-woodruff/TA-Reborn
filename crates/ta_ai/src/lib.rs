//! Skirmish AI.
//!
//! The AI is a deterministic virtual player: it reads only simulation state, iterates in
//! slot order, uses fixed-point math and its own seeded RNG, and emits the same
//! [`Command`]s a human would. Every machine in a lockstep game can therefore run it and
//! reach identical decisions. New replays retain those decisions in their command stream;
//! legacy replays regenerate them. Playback still advances the AI's internal state so a
//! saved game can resume, discarding replacement orders when historical AI orders exist.
//!
//! Unit choices are derived from unit *properties* (extractors, generators, factories,
//! armed units) rather than hard-coded names, so the AI also plays mods sensibly.
//!
//! Each think (every `THINK_TICKS` ticks) the AI surveys its units, updates what it
//! remembers (losses, damage, enemy sightings, who holds which metal deposit), then runs
//! its builders (economy first: extractors, power, factories, tech, defences, strategic
//! buildings), factories, army (home defence and attack waves), air wing, commander and
//! missile silos. Orders only go to places the unit can reach: sending units at another
//! island makes the pathfinder search the whole map again and again.
//!
//! Modules: `info` rates every unit type once per game, `map` keeps metal deposits,
//! vents, strength grids and terrain reachability, `economy` runs constructors, factories
//! and metal makers, `army` the military, `navy` the shipyard and the fleet, `transport`
//! lifts over water and cliffs. Allies (`Sim::allied`) are never targets and are kept out
//! of the line of fire; only our own units are commanded.

mod ally;
mod army;
mod duty;
mod economy;
mod info;
mod map;
mod navy;
mod transport;

#[cfg(test)]
mod tests;

use std::cmp::Reverse;

use info::{ROLES, Role, TypeInfo};
use map::{Deposit, Grid, Vent};
use ta_sim::fixed::{heading_of, heading_vector};
use ta_sim::rng::Rng;
use ta_sim::world::{Occupant, World};
use ta_sim::{Command, Fx, FxVec3, Locomotion, Order, Seen, SignalKind, Sim, Target, Unit, UnitId};

pub use ally::Answer;
pub use duty::{Duty, DutyEvent, DutyOfficer, DutyStatus, IdleReason, MAX_SAVED_BYTES};

/// Answers `SkirmishAi::unit_reaches` keeps before starting afresh.
const REACH_KNOWN: usize = 1 << 16;

/// Whether units of a movement class in a region get to a cell, as `SkirmishAi::unit_reaches`
/// found (the ground's regions never change). A copy starts without: they are found again.
#[derive(Default)]
struct ReachKnown(std::cell::RefCell<rustc_hash::FxHashMap<ReachQuestion, bool>>);

/// A movement class, a region of it and a cell.
type ReachQuestion = (ta_sim::world::MoveSpec, ta_sim::world::Region, (i32, i32));

impl Clone for ReachKnown {
    fn clone(&self) -> ReachKnown {
        ReachKnown::default()
    }
}

/// A map's start within this (px) of ours or an ally's is that start.
const START_NEAR: i64 = 400;
/// Enemy buildings within this (px) of one of `enemy_bases` are that base.
const BASE_NEAR: i64 = 900;
/// An enemy factory further than this (px) from every base known is a base of its own.
const FOUND_APART: i64 = 1500;
/// A radar contact, of a kind not known, as a threat: the reach and worth of an armed unit.
const CONTACT_RANGE: i64 = 300;
const CONTACT_WORTH: i64 = 150;
/// Missiles an enemy anti-missile defence seen is taken to stop (how many it holds is not seen).
const ANTI_STOCK: u32 = 2;

/// Decisions are made every this many ticks (staggered by player).
const THINK_TICKS: u64 = 15;
const SECOND: u64 = 30;
const MINUTE: u64 = 60 * SECOND;
/// Ticks a spot stays off-limits after we lost a building there (times the number of
/// losses, up to four).
const LOST_SPOT_TICKS: u64 = 75 * SECOND;
/// A builder that has not started its job after this long is given something else to do.
const STUCK_TICKS: u64 = 40 * SECOND;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Difficulty {
    Easy,
    Normal,
    Hard,
}

/// Tuning per difficulty.
#[derive(Clone)]
struct Params {
    /// Earliest minute for the first advanced factory, and the metal income it needs.
    tech_minute: u64,
    tech_income: i64,
    /// Constructors wanted: `builders_base + minutes / builders_div`, capped.
    builders_base: usize,
    builders_div: u64,
    builders_cap: usize,
    adv_builders: usize,
    towers: bool,
    /// Extractors go this far into contested ground: a deposit's distance to our base may
    /// be this percentage of its distance to the enemy's.
    reach_pct: i64,
    /// Attack waves: army cost needed for the first wave, growth per wave, pause between.
    wave_base: i64,
    wave_growth: i64,
    wave_cooldown: u64,
    max_factories: usize,
    /// Metal makers at most (they need power we could spend on units).
    max_makers: usize,
    nuke_minute: u64,
    anti_nukes: usize,
    silos: usize,
    strike_group: usize,
    /// Combat units at most (more only slows the game down).
    army_cap: usize,
    /// Least time between two units from one factory (a slower hand on Easy).
    produce_gap: u64,
    /// Pick combat units from the best few (else from everything on offer).
    choose_best: bool,
    /// Rate combat units by what a budget of them is worth in a fight (see
    /// `TypeInfo::efficiency`) rather than per unit.
    efficient: bool,
    /// While the army is worth less than this percentage of our buildings, builders help
    /// the factories instead of starting optional economy projects (0: never).
    army_ratio: i64,
    /// Battle handling: hurt units fall back to be repaired, fire is concentrated.
    micro: bool,
    /// Constructors that fly or hover kept for the metal on other islands (at most).
    expanders: usize,
    /// Extractor upgrades under way at once, power plants going up at once for a fusion
    /// to wait, towers beyond the usual, transports for a stranded army (at most).
    upgrades: usize,
    power_plants: usize,
    extra_towers: usize,
    transports: usize,
    /// Island wars (no land route to the enemy): a scout plane finds the enemy Commander and
    /// bombers massed for it strike it (`army.rs`). Hard only (at sea against Hard without
    /// it: +19.5% margin over 80 games, 24 outright wins to 6).
    air_doctrine: bool,
}

impl Params {
    fn for_difficulty(d: Difficulty) -> Params {
        match d {
            Difficulty::Easy => Params {
                tech_minute: 14,
                tech_income: 10,
                builders_base: 2,
                builders_div: 4,
                builders_cap: 4,
                adv_builders: 1,
                towers: false,
                reach_pct: 80,
                // Small waves, sent as soon as they are ready: easy to beat one by one.
                wave_base: 600,
                wave_growth: 150,
                wave_cooldown: 90 * SECOND,
                max_factories: 1,
                max_makers: 6,
                nuke_minute: u64::MAX,
                anti_nukes: 1,
                silos: 0,
                strike_group: 5,
                army_cap: 25,
                produce_gap: 45 * SECOND,
                choose_best: false,
                efficient: false,
                army_ratio: 0,
                micro: false,
                expanders: 1,
                upgrades: 0,
                power_plants: 1,
                extra_towers: 0,
                transports: 2,
                air_doctrine: false,
            },
            Difficulty::Normal => Params {
                tech_minute: 7,
                tech_income: 7,
                builders_base: 3,
                builders_div: 2,
                builders_cap: 8,
                adv_builders: 3,
                towers: true,
                reach_pct: 105,
                // Concentrated waves: a few units at a time only feed the enemy's defences.
                wave_base: 1600,
                wave_growth: 400,
                wave_cooldown: 50 * SECOND,
                max_factories: 5,
                max_makers: 40,
                nuke_minute: 24,
                anti_nukes: 1,
                silos: 1,
                strike_group: 4,
                army_cap: 120,
                produce_gap: 0,
                choose_best: true,
                efficient: false,
                army_ratio: 60,
                micro: false,
                expanders: 2,
                upgrades: 1,
                power_plants: 1,
                extra_towers: 0,
                transports: 8,
                air_doctrine: false,
            },
            Difficulty::Hard => Params {
                tech_minute: 7,
                tech_income: 7,
                builders_base: 3,
                builders_div: 2,
                builders_cap: 8,
                adv_builders: 3,
                towers: true,
                reach_pct: 105,
                // Normal's waves (bigger ones only give the enemy time), but fought better.
                wave_base: 1600,
                wave_growth: 400,
                wave_cooldown: 50 * SECOND,
                max_factories: 5,
                max_makers: 60,
                nuke_minute: 24,
                anti_nukes: 2,
                silos: 2,
                strike_group: 4,
                army_cap: 160,
                produce_gap: 0,
                choose_best: true,
                efficient: true,
                army_ratio: 60,
                micro: true,
                expanders: 3,
                upgrades: 2,
                power_plants: 2,
                extra_towers: 1,
                transports: 12,
                air_doctrine: true,
            },
        }
    }
}

#[derive(Clone)]
pub struct SkirmishAi {
    pub player: u8,
    pub difficulty: Difficulty,
    rng: Rng,
    p: Params,
    /// This brain works for a person (`DutyOfficer`): it goes only where the player has been,
    /// and never blows up a building of theirs.
    helper: bool,
    base: FxVec3,
    /// Where the enemy is, or may be: the map's starts but ours and our allies' (as a player
    /// knows them from the start), until looked at; then those seen to be bases (enemy
    /// buildings round them), and bases found elsewhere. A start found empty is dropped.
    enemy_bases: Vec<FxVec3>,
    /// Whose each of `enemy_bases` and `ally_bases` is (none known: 255, never forgotten).
    enemy_base_owners: Vec<u8>,
    /// Of `enemy_bases`, those seen to be bases (the others are starts not yet looked at).
    enemy_base_seen: Vec<bool>,
    ally_base_owners: Vec<u8>,
    info: Vec<TypeInfo>,
    deposits: Vec<Deposit>,
    vents: Vec<Vent>,
    /// Seen enemy ground firepower, enemy anti-air cover and our own buildings, per cell.
    threat: Grid,
    enemy_aa: Grid,
    presence: Grid,
    /// Our allies' buildings, per cell, and their starts: their bases, helped when raided,
    /// their metal left to them.
    ally_presence: Grid,
    ally_bases: Vec<FxVec3>,
    /// What our allies asked of us (`ally.rs`), the first tick of signals not yet heard, and
    /// our answers, for the game to put in words.
    requests: ally::Requests,
    signals_from: u64,
    answers: Vec<Answer>,
    /// When we last called our allies to defend our base, and to one of our attacks.
    help_called: Option<u64>,
    attack_called: Option<u64>,
    reach_known: ReachKnown,
    /// Our buildings and unit health last think (to notice losses and damage).
    buildings: Vec<(UnitId, FxVec3)>,
    health: Vec<(UnitId, Fx)>,
    /// Where we recently lost buildings, and where our units were hurt.
    lost_spots: Vec<(FxVec3, u64)>,
    alerts: Vec<(FxVec3, u64)>,
    /// Enemy buildings we have seen (id, place, type), until we see their place without them;
    /// and of those, the ones last seen unfinished. (Never looked up unseen: a building
    /// destroyed out of our sight is remembered until we look.)
    enemy_buildings: Vec<(UnitId, FxVec3, usize)>,
    enemy_frames: std::collections::BTreeSet<UnitId>,
    /// Enemy fighting units seen in the last few minutes: id, cost, when last seen, and
    /// whether a ship; and where an enemy Commander was last seen (and its type).
    enemy_army: Vec<(UnitId, i64, u64, bool)>,
    enemy_commander: Option<(UnitId, FxVec3, u64)>,
    enemy_commander_type: usize,
    /// Places our army reached and found nothing to fight (not targets again until then).
    cleared: Vec<(FxVec3, u64)>,
    /// Last tick enemy aircraft were seen, and how many.
    enemy_air_tick: Option<u64>,
    enemy_air: usize,
    enemy_silo: bool,
    /// Builders waiting on a job: front order, where they stood, since when.
    stuck: Vec<(UnitId, Order, FxVec3, u64)>,
    /// Build sites that failed (type, position, tick).
    failed: Vec<(usize, FxVec3, u64)>,
    /// Build orders just given (builder, type, site, tick): a builder idle again right after
    /// had its order refused (a site it cannot reach), and the site counts as failed.
    issued: Vec<(UnitId, usize, FxVec3, u64)>,
    /// Reclaim orders just given (builder, feature, tick), likewise; and features a builder
    /// could not get to, left alone until a tick.
    issued_reclaims: Vec<(UnitId, usize, u64)>,
    unreachable_features: Vec<(usize, u64)>,
    /// Repairs (and help finishing) just ordered (builder, unit, tick), likewise; and units
    /// a builder could not get to.
    issued_repairs: Vec<(UnitId, UnitId, u64)>,
    unreachable_units: Vec<(UnitId, u64)>,
    /// Orders refused per builder (builder, when, where it stood): one refused again and
    /// again without moving is boxed in (not the sites' fault), and is left alone a while.
    refusals: Vec<(UnitId, u64, FxVec3)>,
    /// (The player's helper only.) How often each feature has been found out of a builder's reach
    /// (the memory of it grows each time: the same trip is not made every few minutes for ever).
    feature_strikes: Vec<(usize, u8)>,
    /// Constructors told to go home lately (builder, tick), and those that could
    /// not (until when they are not told again).
    issued_homes: Vec<(UnitId, u64)>,
    homeless: Vec<(UnitId, u64)>,
    /// Constructors getting away from armed enemies: until when they hold back.
    retreating: Vec<(UnitId, u64)>,
    /// Types no site was found for lately, and until when not to search again.
    place_failed: Vec<(usize, u64)>,
    /// Strips in front of our factories that must stay free (x0, z0, x1, z1).
    lanes: Vec<(i64, i64, i64, i64)>,
    /// Where finished units gather.
    rally: FxVec3,
    /// Room kept free for the advanced factory (position, the type it was sized for).
    tech_site: Option<(FxVec3, usize)>,
    /// Extractors being replaced by better ones.
    upgrades: Vec<Upgrade>,
    /// Trees and rocks cluttering the base (feature indices): cleared so units can move.
    clutter: Vec<usize>,
    /// Smoothed storage fill (0..1) and wind.
    energy_avg: Fx,
    metal_avg: Fx,
    wind_avg: Fx,
    /// Energy per second left for metal makers (income minus everything else spends),
    /// averaged over a quarter of a minute or so (wind and tides make the moment's figure
    /// jump), and how much of the time lately the energy store has been full (income thrown
    /// away).
    maker_room: Fx,
    brimming: Fx,
    /// Per factory type: whether its combat units can reach the enemy by land/sea.
    reach: Vec<Option<bool>>,
    wave: u32,
    last_attack_tick: u64,
    /// The next wave, gathering before it goes in.
    muster: Option<army::Muster>,
    /// Units committed to the current attack.
    attacking: Vec<UnitId>,
    /// Units answering a raid: until when, and where.
    defending: Vec<(UnitId, u64, FxVec3)>,
    /// Hurt units sent home to be repaired, and since when.
    repairing: Vec<(UnitId, u64)>,
    /// Where idle units wait: the rally point and the most exposed extractor fields.
    posts: Vec<FxVec3>,
    last_air_strike: u64,
    /// The air strike under way: its targets and planes with their cost.
    air_strike: Option<army::AirStrike>,
    /// What air strikes have destroyed and lost (worth, decaying with each strike), and
    /// until when planes are not built because they do not pay.
    air_ledger: (i64, i64),
    air_pause_until: u64,
    /// Air strikes launched (for the ladder's statistics), and those at the enemy Commander.
    pub air_strikes: u32,
    pub commander_strikes: u32,
    /// No land route from our start to the enemy's (the Commander's kind can't get there).
    island: bool,
    /// The scout plane looking for the enemy Commander, and since when bombers have been kept
    /// home for a strike on it.
    scout: Option<UnitId>,
    /// The unit sent to look at the enemy's possible starts (`run_scouting`), and when the
    /// last one was lost (another is sent a while after).
    ground_scout: Option<UnitId>,
    scout_lost: Option<u64>,
    massing_since: Option<u64>,
    /// When a plane last swept the map for what is left of an all-but-beaten enemy, and the
    /// most enemy buildings known at once.
    last_hunt: u64,
    enemy_buildings_peak: usize,
    /// Factories that started a unit lately, and when (Easy's slower production).
    produced: Vec<(UnitId, u64)>,
    /// Our unfinished units in factory yards last think (unit, factory), units lost there
    /// unfinished (factory, when), and factories that stopped work because of it (until when):
    /// something shoots into the yard, and every unit started there would be fed to its guns.
    yard: Vec<(UnitId, UnitId)>,
    yard_losses: Vec<(UnitId, u64)>,
    paused: Vec<(UnitId, u64)>,
    /// The commander is falling back until this tick.
    retreat_until: u64,
    /// Where a shipyard can go (once looked for), when we last looked and how often.
    naval: Option<navy::Naval>,
    naval_checked: u64,
    naval_tries: u8,
    /// Ships out on a strike, and when the last one set out.
    fleet: Vec<UnitId>,
    last_fleet_attack: u64,
    /// What drove the fleet back last, and when (the next strike wants more than that).
    fleet_repelled: (i64, u64),
    /// Where the fleet last got anywhere (moved on, or wore the enemy around it down: their
    /// health then), and when; places it could not get anything done at (shooting up a bank
    /// at what it can't hit), until when.
    fleet_anchor: Option<(FxVec3, u64, i64)>,
    fleet_avoid: Vec<(FxVec3, u64)>,
    /// Per movement class and sea (warships, hover and naval transports): how far every
    /// cell lies from it.
    reaches: Vec<map::Reach>,
    /// Per unit type: whether it can get from home to the nearest enemy base by itself
    /// (aircraft and ships always count as able; ships are the fleet's business).
    crosses: Vec<bool>,
    /// The lift under way, passengers promised to transports, the units set down across
    /// the water (they fight on over there) and when the last lift ended.
    lift: Option<transport::Lift>,
    riders: Vec<transport::Rider>,
    landed: Vec<UnitId>,
    last_lift: u64,
    /// When a transport last flew over the enemy for a look, and when an air lift was last
    /// called off (surface transports are preferred for a while after).
    scouted: u64,
    air_lift_failed: u64,
}

/// One think's view of our forces and the enemy's.
struct Survey<'a> {
    now: u64,
    minutes: u64,
    /// A ceasefire with more than a march left to run (no rush): no attack sets out.
    peace: bool,
    own: Vec<&'a Unit>,
    /// Finished mobile builders (commander included).
    builders: Vec<&'a Unit>,
    /// Finished factories.
    factories: Vec<&'a Unit>,
    /// Finished combat units out of the factory (gunships included, planes not).
    army: Vec<&'a Unit>,
    /// Finished planes (bombers, fighters).
    planes: Vec<&'a Unit>,
    /// Finished mobile anti-aircraft units.
    aa_army: Vec<&'a Unit>,
    /// Finished transports.
    transports: Vec<&'a Unit>,
    /// Our unfinished buildings (not those being taken apart).
    nanoframes: Vec<&'a Unit>,
    defenses: Vec<FxVec3>,
    /// Our units per role, finished or not.
    count: [usize; ROLES],
    /// Constructors (finished or in production), excluding the commander.
    builders_t1: usize,
    builders_t2: usize,
    aa_units: usize,
    adv_factory: bool,
    adv_factory_done: bool,
    factory_types: Vec<usize>,
    kinds: Vec<Locomotion>,
    commander: Option<&'a Unit>,
    /// Enemy units in sight (what they are known); and where those only on radar are (what
    /// they are, not).
    enemies: Vec<&'a Unit>,
    contacts: Vec<FxVec3>,
    /// Our allies' units (never targets; kept out of the line of fire).
    allies: Vec<&'a Unit>,
    m_inc: Fx,
    e_inc: Fx,
    e_exp: Fx,
    metal: Fx,
    energy: Fx,
    /// Storage room for energy.
    e_cap: Fx,
    /// Energy per second our running metal makers use, and all of them (switched off too).
    maker_upkeep: Fx,
    maker_capacity: Fx,
    makers: usize,
    big_plants: usize,
    radars: usize,
    /// What our finished army and buildings are worth (cost).
    army_value: i64,
    building_value: i64,
}

/// Jobs handed out this think.
#[derive(Default)]
struct Plan {
    reserved: Vec<(FxVec3, Fx)>,
    /// Build orders in progress per role.
    pending: [usize; ROLES],
    tech: bool,
    /// (The player's helper only.) What each builder is at or was sent to this think, of the
    /// jobs that two are not sent at: a hurt unit to mend, a feature to take apart.
    claims: Vec<(UnitId, Claim)>,
}

/// A job a builder has to itself (a frame is another matter: any number of hands may build it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Claim {
    Mend(UnitId),
    Reclaim(usize),
}

impl Plan {
    fn free(&self, pos: FxVec3, r: Fx) -> bool {
        !self.reserved.iter().any(|(p, pr)| p.within_xz(pos, *pr + r))
    }

    /// Whether a builder other than `who` is at the job `claim`.
    fn claimed(&self, who: UnitId, claim: Claim) -> bool {
        self.claims.iter().any(|&(by, c)| by != who && c == claim)
    }
}

/// How the economy stands this think, as the builders' choices read it (`SkirmishAi::ledger`).
struct Ledger {
    /// Power is short: more of it is wanted.
    e_low: bool,
    /// Power has all but run out, and income does not cover what is spent.
    e_crit: bool,
    /// The energy store keeps filling up: income is being thrown away.
    overflowing: bool,
    /// Metal holds everything up.
    metal_starved: bool,
    /// Power to spare.
    power_spare: bool,
}

/// Replacing one of our extractors with a better one (a moho mine): the builder walks
/// up and reclaims the old extractor (or it self-destructs), then the new one goes on the
/// cleared deposit.
#[derive(Clone, Copy, Debug)]
struct Upgrade {
    deposit: usize,
    builder: UnitId,
    old: UnitId,
    new_type: usize,
    since: u64,
    /// Reclaim orders given for the old extractor (after two that came to nothing it is
    /// blown up instead: buildings around it can keep the builder out of reach).
    reclaims: u8,
}

#[derive(Debug)]
enum Task {
    Build(usize, FxVec3),
    /// Clear a wreck off a site, then build there.
    ReclaimBuild(usize, usize, FxVec3),
    Repair(UnitId),
    Guard(UnitId),
    Move(FxVec3),
    /// Replace our extractor on a deposit with a better type.
    Upgrade(usize, UnitId, usize),
    Reclaim(usize),
}

/// Where around the base a building goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Zone {
    /// Away from the enemy (economy, strategic buildings).
    Back,
    /// Towards the enemy (defences).
    Front,
    /// Anywhere around the base.
    Ring,
}

fn dist(a: FxVec3, b: FxVec3) -> i64 {
    (a - b).length_xz().to_int()
}

fn fx(v: i64) -> Fx {
    Fx::from_int(v)
}

/// Units already given orders during this think, before the simulation applies them.
fn ordered_units(out: &[Command]) -> rustc_hash::FxHashSet<UnitId> {
    out.iter().filter_map(|c| match c {
        Command::Order { units, .. } | Command::Formation { units, .. } => Some(units),
        _ => None,
    }).flatten().copied().collect()
}

/// How near a point `n` units sent there together count as there: 320 px; under the rule
/// "modern movement" more for more of them, as their formation stands wider (40 px for each of
/// the square root of their number: a hundred reach 720 px out).
fn gathered_within(sim: &Sim, n: usize) -> Fx {
    fx(if sim.group_formations { 320 + 40 * (n as u64).isqrt() as i64 } else { 320 })
}

/// Whether `p` lies within `r` of the segment `a`–`b` (horizontal plane).
fn segment_near(a: FxVec3, b: FxVec3, p: FxVec3, r: Fx) -> bool {
    let ab = (b - a).xz();
    let ap = (p - a).xz();
    let len2 = ab.x * ab.x + ab.z * ab.z;
    if len2 <= Fx::ZERO {
        return p.within_xz(a, r);
    }
    let t = ((ap.x * ab.x + ap.z * ab.z) / len2).clamp(Fx::ZERO, Fx::ONE);
    let closest = FxVec3::new(a.x + ab.x * t, Fx::ZERO, a.z + ab.z * t);
    p.within_xz(closest, r)
}

impl SkirmishAi {
    /// Changes one of the level's parameters (for experiments such as the ladder's
    /// `TA_LADDER_A_TWEAKS`): `micro`, `efficient`, `towers`, `choose_best` (0 or 1),
    /// `army_cap`, `max_makers`, `max_factories`, `silos`, `anti_nukes`, `strike_group`,
    /// `builders_cap`, `wave_base`, `wave_growth`, `wave_cooldown` (seconds), `army_ratio`,
    /// `reach_pct`, `tech_minute`, `expanders`, `upgrades`, `power_plants`, `extra_towers`,
    /// `transports`, `nuke_minute`, `air_doctrine` (0 or 1). False for an unknown name.
    pub fn tweak(&mut self, name: &str, value: i64) -> bool {
        let p = &mut self.p;
        let n = value.max(0) as usize;
        match name {
            "micro" => p.micro = value != 0,
            "efficient" => p.efficient = value != 0,
            "towers" => p.towers = value != 0,
            "choose_best" => p.choose_best = value != 0,
            "army_cap" => p.army_cap = n,
            "max_makers" => p.max_makers = n,
            "max_factories" => p.max_factories = n,
            "silos" => p.silos = n,
            "anti_nukes" => p.anti_nukes = n,
            "strike_group" => p.strike_group = n,
            "builders_cap" => p.builders_cap = n,
            "wave_base" => p.wave_base = value,
            "wave_growth" => p.wave_growth = value,
            "wave_cooldown" => p.wave_cooldown = value.max(0) as u64 * SECOND,
            "army_ratio" => p.army_ratio = value,
            "reach_pct" => p.reach_pct = value,
            "tech_minute" => p.tech_minute = value.max(0) as u64,
            "expanders" => p.expanders = n,
            "nuke_minute" => p.nuke_minute = value.max(0) as u64,
            "upgrades" => p.upgrades = n,
            "power_plants" => p.power_plants = n,
            "extra_towers" => p.extra_towers = n,
            "transports" => p.transports = n,
            "air_doctrine" => p.air_doctrine = value != 0,
            _ => return false,
        }
        true
    }

    pub fn new(player: u8, sim: &Sim, difficulty: Difficulty) -> SkirmishAi {
        // Home is the commander, else (missions) the middle of our buildings or units.
        let centroid = |mine: bool, static_only: bool| {
            let pts: Vec<FxVec3> = sim
                .units()
                .filter(|u| if mine { u.owner == player } else { sim.is_enemy(player, u.owner) && !sim.ai_ignores(u.id) })
                .filter(|u| !static_only || sim.unit_type(u).locomotion == Locomotion::Static)
                .map(|u| u.pos)
                .collect();
            (!pts.is_empty()).then(|| {
                let n = pts.len() as i64;
                let sum = pts.iter().fold(FxVec3::ZERO, |a, &p| a + p);
                FxVec3::new(sum.x / n, Fx::ZERO, sum.z / n)
            })
        };
        let base = sim
            .units()
            .find(|u| u.owner == player && sim.unit_type(u).def.is_commander)
            .map(|u| u.pos)
            .or_else(|| centroid(true, true))
            .or_else(|| centroid(true, false))
            .unwrap_or_default();
        let allies: Vec<(u8, FxVec3)> = sim.units().filter(|u| u.owner != player && sim.allied(player, u.owner) && sim.unit_type(u).def.is_commander).map(|u| (u.owner, u.pos)).collect();
        // Where the enemy may be: the map's starts but ours and our allies' (which of them it
        // took is found by looking: see `update_bases`); an enemy start no spot of the map's
        // stands near (a mission, a map without) as the setup has it; a mission with neither,
        // the middle of the enemy's buildings, else its units.
        let taken = |p: FxVec3| p.within_xz(base, fx(START_NEAR)) || allies.iter().any(|a| p.within_xz(a.1, fx(START_NEAR)));
        let mut enemy_bases: Vec<FxVec3> = Vec::new();
        for &p in &sim.start_spots {
            if !taken(p) && !enemy_bases.iter().any(|b| b.within_xz(p, fx(START_NEAR))) {
                enemy_bases.push(p);
            }
        }
        for (k, pl) in sim.players.iter().enumerate() {
            let p = pl.start;
            if sim.is_enemy(player, k as u8) && p != FxVec3::ZERO && !taken(p) && !enemy_bases.iter().any(|b| b.within_xz(p, fx(START_NEAR))) {
                enemy_bases.push(p);
            }
        }
        if enemy_bases.is_empty() {
            enemy_bases.extend(centroid(false, true).or_else(|| centroid(false, false)));
        }
        let enemy_base_owners: Vec<u8> = enemy_bases.iter().map(|_| u8::MAX).collect();
        let enemy_base_seen: Vec<bool> = enemy_bases.iter().map(|_| false).collect();
        let info = info::classify(sim);
        // Which movement classes get from our start to the nearest enemy start.
        let enemy = enemy_bases.iter().copied().min_by_key(|p| ((*p - base).xz().length_sq_raw(), p.x.0, p.z.0));
        let mut by_spec: Vec<(ta_sim::world::MoveSpec, bool)> = Vec::new();
        let crosses: Vec<bool> = info
            .iter()
            .map(|i| match (i.spec, enemy) {
                (Some(sp), Some(e)) if i.kind != Locomotion::Naval => match by_spec.iter().find(|(s, _)| *s == sp) {
                    Some(&(_, r)) => r,
                    None => {
                        let r = map::can_reach(&sim.world, &sp, base, e, 8);
                        by_spec.push((sp, r));
                        r
                    }
                },
                _ => true,
            })
            .collect();
        // An island war: no basic tank or kbot gets from our start to the nearest enemy start
        // (the Commanders walk under water; they don't count).
        let island = enemy.is_some() && !info.iter().enumerate().any(|(t, i)| i.role == Role::Combat && i.kind == Locomotion::Ground && i.tech == 1 && crosses[t]);
        let mut deposits = map::find_deposits(sim);
        // Rich metal ground no feature marks, sized for our smallest extractor.
        let ext_fp = sim.types.types.iter().filter(|t| info[t.id].role == Role::Extractor && info[t.id].land && !t.needs_geo).map(|t| t.footprint).min();
        if let Some(fp) = ext_fp {
            let spots = map::find_metal_spots(sim, &deposits, fp, base);
            deposits.extend(spots);
        }
        SkirmishAi {
            player,
            difficulty,
            // From the game's seed: the same game replays the same way, another one doesn't.
            rng: Rng::new(0xA1 ^ ((player as u64) << 32) ^ sim.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            p: Params::for_difficulty(difficulty),
            helper: false,
            base,
            enemy_bases,
            enemy_base_owners,
            enemy_base_seen,
            ally_base_owners: allies.iter().map(|a| a.0).collect(),
            deposits,
            vents: map::find_vents(sim),
            threat: Grid::new(sim),
            enemy_aa: Grid::new(sim),
            presence: Grid::new(sim),
            ally_presence: Grid::new(sim),
            ally_bases: allies.iter().map(|a| a.1).collect(),
            requests: Default::default(),
            signals_from: 0,
            answers: Vec::new(),
            help_called: None,
            attack_called: None,
            reach_known: Default::default(),
            buildings: Vec::new(),
            health: Vec::new(),
            lost_spots: Vec::new(),
            alerts: Vec::new(),
            enemy_buildings: Vec::new(),
            enemy_army: Vec::new(),
            enemy_commander: None,
            enemy_commander_type: 0,
            enemy_frames: Default::default(),
            cleared: Vec::new(),
            enemy_air_tick: None,
            enemy_air: 0,
            enemy_silo: false,
            stuck: Vec::new(),
            failed: Vec::new(),
            issued: Vec::new(),
            issued_reclaims: Vec::new(),
            unreachable_features: Vec::new(),
            issued_repairs: Vec::new(),
            unreachable_units: Vec::new(),
            refusals: Vec::new(),
            feature_strikes: Vec::new(),
            issued_homes: Vec::new(),
            homeless: Vec::new(),
            retreating: Vec::new(),
            place_failed: Vec::new(),
            lanes: Vec::new(),
            rally: base,
            tech_site: None,
            upgrades: Vec::new(),
            clutter: Vec::new(),
            energy_avg: Fx::HALF,
            metal_avg: Fx::HALF,
            wind_avg: sim.wind,
            maker_room: Fx::ZERO,
            brimming: Fx::ZERO,
            reach: vec![None; info.len()],
            info,
            wave: 0,
            last_attack_tick: 0,
            muster: None,
            attacking: Vec::new(),
            defending: Vec::new(),
            repairing: Vec::new(),
            posts: Vec::new(),
            last_air_strike: 0,
            air_strike: None,
            air_ledger: (0, 0),
            air_pause_until: 0,
            air_strikes: 0,
            commander_strikes: 0,
            island,
            scout: None,
            ground_scout: None,
            scout_lost: None,
            massing_since: None,
            last_hunt: 0,
            enemy_buildings_peak: 0,
            produced: Vec::new(),
            yard: Vec::new(),
            yard_losses: Vec::new(),
            paused: Vec::new(),
            retreat_until: 0,
            naval: None,
            naval_checked: 0,
            naval_tries: 0,
            fleet: Vec::new(),
            last_fleet_attack: 0,
            fleet_repelled: (0, 0),
            fleet_anchor: None,
            fleet_avoid: Vec::new(),
            reaches: Vec::new(),
            crosses,
            lift: None,
            riders: Vec::new(),
            landed: Vec::new(),
            last_lift: 0,
            scouted: 0,
            air_lift_failed: 0,
        }
    }

    /// Returns this tick's commands.
    pub fn think(&mut self, sim: &Sim) -> Vec<Command> {
        let mut out = Vec::new();
        // Players take turns spread evenly over the cycle (none sees the others' moves sooner).
        let phase = self.player as u64 * THINK_TICKS / sim.players.len().max(1) as u64;
        if !(sim.tick + phase).is_multiple_of(THINK_TICKS) {
            return out;
        }
        if sim.players.get(self.player as usize).is_none_or(|p| p.defeated) {
            return out;
        }
        let s = self.survey(sim);
        if s.own.is_empty() {
            return out;
        }
        self.remember(sim, &s);
        self.forget_defeated(sim);
        self.hear_allies(sim, &s, &mut out);
        if s.now % MINUTE < THINK_TICKS {
            self.choose_rally(sim, &s);
        }
        if s.now % (10 * SECOND) < THINK_TICKS {
            self.mark_far_deposits(sim, &s);
        }
        self.run_makers(sim, &s, &mut out);
        self.arm_defenses(sim, &s, &mut out);
        self.run_commander(sim, &s, &mut out);
        self.plan_navy(sim, &s);
        self.run_builders(sim, &s, &mut out);
        self.run_factories(sim, &s, &mut out);
        self.run_scouting(sim, &s, &mut out);
        self.run_army(sim, &s, &mut out);
        self.run_transports(sim, &s, &mut out);
        self.run_navy(sim, &s, &mut out);
        self.run_air(sim, &s, &mut out);
        self.run_silos(sim, &s, &mut out);
        out
    }

    /// Its fixed guns set to fire at will once finished. TA builds its long-range guns, the
    /// Doomsday Machine and the Annihilator on hold fire (`NoAutoFire=1`, `StandingFireOrder=0`)
    /// for the player to set going; a computer player sets its own going as a player would. (Not
    /// its silos, bombers, crawling bombs or snipers: its orders send those.)
    fn arm_defenses(&self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let holding: Vec<UnitId> = s
            .own
            .iter()
            .filter(|u| u.fire_state == 0 && Self::can_command(sim, u))
            .filter(|u| {
                let t = sim.unit_type(u);
                t.locomotion == Locomotion::Static
                    && t.weapons.iter().flatten().any(|&w| {
                        let w = sim.weapon_types.get(w);
                        !w.command_fire && !w.def.stockpile && !w.def.interceptor
                    })
            })
            .map(|u| u.id)
            .collect();
        if !holding.is_empty() {
            out.push(Command::SetStates { player: self.player, units: holding, fire: Some(2), movement: None });
        }
    }

    // ---- Survey and memory -----------------------------------------------------------------

    /// Units under script control or aboard a transport retain their own orders.
    fn can_command(sim: &Sim, u: &Unit) -> bool {
        !u.dead && u.is_complete() && u.carried_by.is_none() && !sim.runs_mission_script(u.id)
            && !matches!(u.orders.front(), Some(Order::Wait(_) | Order::WaitAttacked(_) | Order::AttackType(_) | Order::SetStates { .. } | Order::Patrol(_)))
    }

    fn survey<'a>(&self, sim: &'a Sim) -> Survey<'a> {
        let now = sim.tick;
        let eco = &sim.players[self.player as usize].economy;
        let mut s = Survey {
            now,
            minutes: now / MINUTE,
            peace: sim.ceasefire_left().is_some_and(|left| left > 45 * SECOND),
            own: Vec::new(),
            builders: Vec::new(),
            factories: Vec::new(),
            army: Vec::new(),
            planes: Vec::new(),
            aa_army: Vec::new(),
            transports: Vec::new(),
            nanoframes: Vec::new(),
            defenses: Vec::new(),
            count: [0; ROLES],
            builders_t1: 0,
            builders_t2: 0,
            aa_units: 0,
            adv_factory: false,
            adv_factory_done: false,
            factory_types: Vec::new(),
            kinds: Vec::new(),
            commander: None,
            enemies: Vec::new(),
            contacts: Vec::new(),
            allies: Vec::new(),
            m_inc: eco.metal_income,
            e_inc: eco.energy_income,
            e_exp: eco.energy_expense,
            metal: eco.metal,
            energy: eco.energy,
            e_cap: eco.energy_capacity,
            maker_upkeep: Fx::ZERO,
            maker_capacity: Fx::ZERO,
            makers: 0,
            big_plants: 0,
            radars: 0,
            army_value: 0,
            building_value: 0,
        };
        for u in sim.units() {
            if u.dead {
                continue;
            }
            if u.owner != self.player {
                if !sim.is_enemy(self.player, u.owner) {
                    s.allies.push(u);
                } else if u.carried_by.is_none() && !sim.ai_ignores(u.id) {
                    // (A mission's `AiIgnore` units: as though they weren't there.)
                    match sim.seen_by(u, self.player) {
                        Seen::Visible => s.enemies.push(u),
                        Seen::Radar => s.contacts.push(u.pos),
                        Seen::Hidden => {}
                    }
                }
                continue;
            }
            s.own.push(u);
            let i = &self.info[u.type_id];
            s.count[i.role as usize] += 1;
            // Units following a mission script (waiting, patrolling...) are left to it.
            let done = Self::can_command(sim, u);
            let out_of_yard = done && u.factory.is_none();
            match i.role {
                Role::Builder => {
                    if i.commander {
                        if done {
                            s.commander = Some(u);
                        }
                    } else if i.builds_mex || i.builds_energy || i.builds_factory {
                        if i.tech >= 2 {
                            s.builders_t2 += 1;
                        } else {
                            s.builders_t1 += 1;
                        }
                    }
                    if done && i.kind != Locomotion::Static {
                        s.builders.push(u);
                    }
                }
                Role::Factory => {
                    let adv = self.advanced_factory(sim, u.type_id);
                    s.adv_factory |= adv;
                    s.adv_factory_done |= adv && done;
                    s.factory_types.push(u.type_id);
                    s.kinds.push(i.kind);
                    if done {
                        s.factories.push(u);
                    }
                }
                _ if i.transport > 0 => {
                    if out_of_yard {
                        s.transports.push(u);
                    }
                }
                Role::Combat if out_of_yard => {
                    if i.kind == Locomotion::Air && !i.gunship {
                        s.planes.push(u);
                    } else {
                        s.army.push(u);
                    }
                }
                Role::AntiAirUnit => {
                    s.aa_units += 1;
                    if out_of_yard {
                        s.aa_army.push(u);
                    }
                }
                Role::MetalMaker if done => {
                    s.makers += 1;
                    s.maker_capacity += i.maker_upkeep;
                    if u.activated {
                        s.maker_upkeep += i.maker_upkeep;
                    }
                }
                Role::Generator if i.energy >= fx(300) => s.big_plants += 1,
                Role::Defense => s.defenses.push(u.pos),
                _ => {}
            }
            let building = sim.unit_type(u).locomotion == Locomotion::Static;
            // (Not one of ours being taken apart, an old extractor making way for a better one:
            // a builder sent to finish it would only mend it as it went.)
            if !u.is_complete() && u.factory.is_none() && building && !sim.being_taken_apart(u) {
                s.nanoframes.push(u);
            }
            if building && !sim.unit_type(u).radar.is_zero() {
                s.radars += 1;
            }
            if done {
                if building {
                    s.building_value += i.cost;
                } else if matches!(i.role, Role::Combat | Role::AntiAirUnit) {
                    s.army_value += i.cost;
                }
            }
        }
        s
    }

    /// A factory that turns out advanced constructors (a real tech step, unlike a hover
    /// plant tagged LEVEL2).
    fn advanced_factory(&self, sim: &Sim, type_id: usize) -> bool {
        self.info[type_id].role == Role::Factory
            && sim.types.get(type_id).ai_build_options.iter().any(|&o| {
                let i = &self.info[o];
                i.role == Role::Builder && i.tech >= 2 && (i.builds_energy || i.builds_mex)
            })
    }

    fn remember(&mut self, sim: &Sim, s: &Survey) {
        let now = s.now;
        let eco = &sim.players[self.player as usize].economy;
        let frac = |v: Fx, cap: Fx| if cap > Fx::ZERO { (v / cap).clamp(Fx::ZERO, Fx::ONE) } else { Fx::ZERO };
        self.energy_avg = self.energy_avg + (frac(eco.energy, eco.energy_capacity) - self.energy_avg) / 6;
        self.metal_avg = self.metal_avg + (frac(eco.metal, eco.metal_capacity) - self.metal_avg) / 6;
        self.wind_avg = self.wind_avg + (sim.wind - self.wind_avg) / 40;
        let room = eco.energy_income - eco.energy_expense + s.maker_upkeep;
        self.maker_room = self.maker_room + (room - self.maker_room) / 30;
        let full = if eco.energy * 100 >= eco.energy_capacity * 97 { Fx::ONE } else { Fx::ZERO };
        self.brimming = self.brimming + (full - self.brimming) / 60;

        // Buildings lost (destroyed, or nanoframes shot down): avoid the spot for a while.
        let now_buildings: Vec<(UnitId, FxVec3)> = s.own.iter().filter(|u| sim.unit_type(u).locomotion == Locomotion::Static).map(|u| (u.id, u.pos)).collect();
        let lost: Vec<FxVec3> = self.buildings.iter()
            .filter(|(id, _)| sim.unit(*id).is_none_or(|u| u.owner != self.player || u.dead))
            // (An extractor taken apart to make way for a better one is no loss.)
            .filter(|(id, _)| !self.upgrades.iter().any(|up| up.old == *id))
            .map(|(_, p)| *p).collect();
        for pos in lost {
            self.lost_spots.push((pos, now));
            self.alert(pos, now);
            for d in &mut self.deposits {
                if d.pos.within_xz(pos, fx(48)) {
                    d.losses += 1;
                    d.avoid_until = now + LOST_SPOT_TICKS * d.losses.min(4) as u64;
                }
            }
            for v in &mut self.vents {
                if v.pos.within_xz(pos, fx(64)) {
                    v.avoid_until = now + LOST_SPOT_TICKS * 2;
                }
            }
        }
        self.buildings = now_buildings;
        self.lost_spots.retain(|(_, t)| now < t + 4 * MINUTE);
        self.failed.retain(|(_, _, t)| now < t + 3 * MINUTE);

        // Units lost unfinished in a factory's yard (finished ones walk out alive).
        let yard: Vec<(UnitId, UnitId)> = s.own.iter().filter(|u| !u.is_complete()).filter_map(|u| u.factory.map(|f| (u.id, f))).collect();
        let me = self.player;
        let lost: Vec<(UnitId, FxVec3)> = std::mem::replace(&mut self.yard, yard)
            .into_iter()
            .filter(|&(id, _)| sim.unit(id).is_none_or(|u| u.dead))
            .filter_map(|(_, f)| sim.unit(f).filter(|fu| !fu.dead && fu.owner == me).map(|fu| (f, fu.pos)))
            .collect();
        for (f, at) in lost {
            self.yard_losses.push((f, now));
            self.alert(at, now);
        }
        self.yard_losses.retain(|&(_, t)| now < t + MINUTE);

        // Damage since last think.
        let mut health: Vec<(UnitId, Fx)> = Vec::with_capacity(s.own.len());
        for u in &s.own {
            if let Ok(i) = self.health.binary_search_by_key(&u.id, |(id, _)| *id) && u.health + Fx::ONE < self.health[i].1 {
                self.alert(u.pos, now);
            }
            health.push((u.id, u.health));
        }
        health.sort_by_key(|(id, _)| *id);
        self.health = health;
        self.alerts.retain(|(_, t)| now < t + 30 * SECOND);

        // What we see of the enemy, and what we remember of it (looked up by unit: a big battle
        // shows thousands). Radar shows where something is, not what: a threat of a kind not
        // known. Nothing out of our sight is looked up: a building is remembered until we see
        // its place without it, an army for a few minutes, a Commander until we see it gone
        // from where it was.
        self.threat.clear();
        self.enemy_aa.clear();
        let mut air = 0;
        let mut known_buildings: rustc_hash::FxHashSet<UnitId> = self.enemy_buildings.iter().map(|b| b.0).collect();
        let mut known_army: rustc_hash::FxHashMap<UnitId, usize> = self.enemy_army.iter().enumerate().map(|(k, a)| (a.0, k)).collect();
        let seen_now: rustc_hash::FxHashSet<UnitId> = s.enemies.iter().map(|e| e.id).collect();
        for e in &s.enemies {
            let i = &self.info[e.type_id];
            let kind = sim.unit_type(e).locomotion;
            if !e.is_complete() {
                // Nanoframes are harmless, but still buildings worth knowing about.
            } else if kind == Locomotion::Air {
                if i.ground_dps > 0 {
                    air += 1;
                }
            } else {
                if i.ground_dps > 0 {
                    self.threat.add_disc(e.pos, i.range.max(150) + 64, i.worth);
                }
                if i.air_dps > 0 {
                    self.enemy_aa.add_disc(e.pos, 500, i.worth);
                }
            }
            if i.role == Role::Silo {
                self.enemy_silo = true;
            }
            if kind == Locomotion::Static {
                if known_buildings.insert(e.id) {
                    self.enemy_buildings.push((e.id, e.pos, e.type_id));
                }
                if e.is_complete() {
                    self.enemy_frames.remove(&e.id);
                } else {
                    self.enemy_frames.insert(e.id);
                }
            }
            if e.is_complete() && kind != Locomotion::Static && (i.ground_dps > 0 || i.air_dps > 0) {
                match known_army.get(&e.id) {
                    Some(&k) => self.enemy_army[k].2 = now,
                    None => {
                        known_army.insert(e.id, self.enemy_army.len());
                        self.enemy_army.push((e.id, i.worth, now, kind == Locomotion::Naval));
                    }
                }
            }
            if i.commander {
                self.enemy_commander = Some((e.id, e.pos, now));
                self.enemy_commander_type = e.type_id;
            }
        }
        for &c in &s.contacts {
            self.threat.add_disc(c, CONTACT_RANGE, CONTACT_WORTH);
        }
        if air > 0 {
            self.enemy_air_tick = Some(now);
            self.enemy_air = air.max(self.enemy_air * 3 / 4);
        }
        // Gone from where we look (destroyed, or taken: a building of ours or an ally's there
        // now is no enemy's).
        let friendly: rustc_hash::FxHashSet<_> = s.own.iter().chain(&s.allies).map(|u| u.id).collect();
        self.enemy_buildings.retain(|&(id, p, _)| !friendly.contains(&id) && (seen_now.contains(&id) || !sim.in_sight(me, p)));
        let kept: rustc_hash::FxHashSet<UnitId> = self.enemy_buildings.iter().map(|b| b.0).collect();
        self.enemy_frames.retain(|id| kept.contains(id));
        self.enemy_buildings_peak = self.enemy_buildings_peak.max(self.enemy_buildings.len());
        // The player's helper keeps clear of the armed buildings it has seen when they are out of
        // sight, too: a tower is where it was, and the way back to it is no safer for the builder
        // having run from it.
        if self.helper {
            for &(id, pos, ty) in &self.enemy_buildings {
                if seen_now.contains(&id) || self.enemy_frames.contains(&id) {
                    continue;
                }
                let i = &self.info[ty];
                if i.ground_dps > 0 {
                    self.threat.add_disc(pos, i.range.max(150) + 64, i.worth);
                }
                if i.air_dps > 0 {
                    self.enemy_aa.add_disc(pos, 500, i.worth);
                }
            }
        }
        self.enemy_army.retain(|&(id, _, seen, _)| !friendly.contains(&id) && now < seen + 3 * MINUTE);
        if self.enemy_commander.is_some_and(|(id, p, seen)| friendly.contains(&id) || now >= seen + 5 * MINUTE || !seen_now.contains(&id) && sim.in_sight(me, p)) {
            self.enemy_commander = None;
        }
        if self.update_bases(sim, s) {
            self.replan(sim);
        }
        self.cleared.retain(|&(_, until)| now < until);
        self.presence.clear();
        for u in &s.own {
            if sim.unit_type(u).locomotion == Locomotion::Static {
                self.presence.add(u.pos, 1);
            }
        }
        self.ally_presence.clear();
        for u in &s.allies {
            if sim.unit_type(u).locomotion == Locomotion::Static {
                self.ally_presence.add(u.pos, 1);
            }
        }

        // Who holds which deposit, as far as we see: ours and our allies' always, the enemy's
        // where we look (remembered; one never seen is taken as free, until a constructor sent
        // there finds it taken), and what blocks the free ones.
        for d in &mut self.deposits {
            if d.owner.is_none_or(|o| !sim.is_enemy(me, o)) || sim.in_sight(me, d.pos) {
                d.owner = None;
            }
            d.ours = None;
            d.wreck = None;
        }
        for u in s.own.iter().chain(&s.allies).chain(&s.enemies) {
            if self.info[u.type_id].role != Role::Extractor {
                continue;
            }
            if let Some(d) = self.deposits.iter_mut().find(|d| d.pos.within_xz(u.pos, fx(40))) {
                d.owner = Some(u.owner);
                if u.owner == me {
                    d.ours = Some((u.id, u.type_id));
                }
            }
        }
        for d in &mut self.deposits {
            if d.owner.is_some() {
                continue;
            }
            let (cx, cz) = World::cell_of(d.pos);
            'cells: for dz in -1..=1 {
                for dx in -1..=1 {
                    if let Occupant::Feature(f) = sim.world.occupant(cx + dx, cz + dz) {
                        d.wreck = Some(f as usize);
                        break 'cells;
                    }
                }
            }
        }
    }

    /// What the enemy army we have seen lately is worth (cost).
    fn enemy_army_value(&self) -> i64 {
        self.enemy_army.iter().map(|a| a.1).sum()
    }

    /// Our army is so much stronger than anything the enemy has shown that it should go for
    /// the kill (never on Easy).
    fn dominant(&self, s: &Survey) -> bool {
        // (Not while the enemy is not found: seeing nothing of it is no sign of strength.)
        self.difficulty != Difficulty::Easy && s.minutes >= 10 && self.enemy_base_seen.iter().any(|&b| b) && s.army_value >= self.enemy_army_value() * 5 / 2 + 1500
    }

    /// The enemy is all but gone: we dominate, and of the many buildings of theirs we have
    /// seen, a handful are left (not merely few seen: on a sea map we may never have looked).
    /// Every plane then hunts them down (see `run_air`).
    fn mopping_up(&self, s: &Survey) -> bool {
        let known = self.enemy_buildings.len();
        self.dominant(s) && s.army_value >= self.enemy_army_value() * 5 + 3000 && known <= 12 && self.enemy_buildings_peak >= 20 && known * 4 <= self.enemy_buildings_peak
    }

    /// We dominate and see nothing of the enemy but a few buildings, if any: the planes at
    /// home may strike in ones and twos, and one sweeps the map when nothing is known (idle
    /// aircraft only; production stays as it is).
    fn hunting(&self, s: &Survey) -> bool {
        self.mopping_up(s) || self.dominant(s) && s.minutes >= 15 && self.enemy_army_value() == 0 && self.enemy_buildings.len() <= 8
    }

    fn alert(&mut self, pos: FxVec3, now: u64) {
        if let Some(a) = self.alerts.iter_mut().find(|(p, _)| p.within_xz(pos, fx(200))) {
            *a = (pos, now);
        } else {
            self.alerts.push((pos, now));
        }
    }

    fn air_threat(&self, now: u64) -> bool {
        self.enemy_air_tick.is_some_and(|t| now < t + 4 * MINUTE)
    }

    /// The bases of players out of the game are no longer theirs: an enemy's is no target, and
    /// no ground of theirs; an ally's no claim on the metal round it. (The last enemy base is
    /// kept: the battle is over then anyway.)
    fn forget_defeated(&mut self, sim: &Sim) {
        let out = |owner: u8| sim.players.get(owner as usize).is_some_and(|p| p.defeated);
        if self.enemy_base_owners.iter().any(|&o| out(o)) && self.enemy_base_owners.iter().any(|&o| !out(o)) {
            let keep: Vec<bool> = self.enemy_base_owners.iter().map(|&o| !out(o)).collect();
            self.keep_bases(&keep);
            self.replan(sim);
        }
        if self.ally_base_owners.iter().any(|&o| out(o)) {
            let keep: Vec<bool> = self.ally_base_owners.iter().map(|&o| !out(o)).collect();
            let mut k = keep.iter();
            self.ally_bases.retain(|_| *k.next().unwrap_or(&true));
            self.ally_base_owners.retain(|&o| !out(o));
        }
    }

    // ---- Geometry helpers --------------------------------------------------------------------

    /// The nearest to `from` of the enemy bases seen, else of the starts where one may be.
    fn nearest_enemy_base(&self, from: FxVec3) -> Option<FxVec3> {
        let nearest = |seen: bool| self.enemy_bases.iter().zip(&self.enemy_base_seen).filter(|&(_, &b)| b == seen).map(|(&p, _)| p).min_by_key(|p| ((*p - from).xz().length_sq_raw(), p.x.0, p.z.0));
        nearest(true).or_else(|| nearest(false))
    }

    /// Unit vector from our base towards the enemy.
    fn front_dir(&self) -> FxVec3 {
        match self.nearest_enemy_base(self.base) {
            Some(e) if !e.within_xz(self.base, fx(32)) => (e - self.base).xz().normalize(),
            _ => FxVec3::new(Fx::ZERO, Fx::ZERO, Fx::ONE),
        }
    }

    fn clamp_to_map(sim: &Sim, p: FxVec3) -> FxVec3 {
        let (w, h) = sim.world.size_px();
        FxVec3::new(p.x.clamp(fx(48), w - fx(48)), Fx::ZERO, p.z.clamp(fx(48), h - fx(48)))
    }

    /// A point around the base in a zone, `radius` (plus a little) from the centre.
    fn zone_point(&mut self, sim: &Sim, zone: Zone, radius: i64) -> FxVec3 {
        let front = heading_of(self.front_dir());
        let (centre, spread) = match zone {
            Zone::Back => (front + 32768, 13000),
            Zone::Front => (front, 7000),
            Zone::Ring => (0, 32767),
        };
        let angle = centre + self.rng.range_i32(-spread, spread);
        let r = radius + self.rng.range_i32(0, 90) as i64;
        Self::clamp_to_map(sim, self.base + heading_vector(angle).scale(fx(r)))
    }

    /// Picks the rally point: in front of the base, on ground our units can reach.
    fn choose_rally(&mut self, sim: &Sim, s: &Survey) {
        let d = match self.nearest_enemy_base(self.base) {
            Some(e) => dist(e, self.base),
            None => 0,
        };
        let probe = s.commander.into_iter().chain(s.builders.iter().copied()).chain(s.army.iter().copied()).find_map(|u| self.info[u.type_id].spec.map(|sp| (u.pos, sp)));
        // The middle of what we hold (base and extractor fields), so raids anywhere are
        // answered quickly.
        let (mut sum, mut n) = (self.base.scale(fx(4)), 4i64);
        for d in self.deposits.iter().filter(|d| d.ours.is_some()) {
            sum += d.pos;
            n += 1;
        }
        let centre = FxVec3::new(sum.x / n, Fx::ZERO, sum.z / n);
        let mut r = (d / 8).clamp(120, 350);
        loop {
            let p = Self::clamp_to_map(sim, centre + self.front_dir().scale(fx(r)));
            let reachable = probe.is_none_or(|(from, spec)| {
                let a = World::pos_to_anchor(p, spec.footprint);
                let region = map::region_at(&sim.world, &spec, from);
                region != 0 && sim.world.component(&spec, a) == region
            });
            if reachable || r <= 0 {
                self.rally = if reachable { p } else { self.base };
                break;
            }
            r -= 60;
        }
        // Guard posts: the rally point, and up to two outlying extractor fields worth
        // holding (the richest, most exposed ones), a little towards home.
        self.posts = vec![self.rally];
        let mut fields: Vec<(i64, FxVec3)> = Vec::new();
        for d in self.deposits.iter().filter(|d| d.ours.is_some()) {
            let db = dist(d.pos, self.base);
            if db < 900 || fields.iter().any(|(_, p)| p.within_xz(d.pos, fx(700))) {
                continue;
            }
            let near: Vec<&Deposit> = self.deposits.iter().filter(|e| e.ours.is_some() && e.pos.within_xz(d.pos, fx(500))).collect();
            let value: i64 = near.iter().map(|e| e.metal).sum::<i64>() + near.iter().map(|e| e.losses as i64 * 600).sum::<i64>();
            if value < 2000 {
                continue;
            }
            let n = near.len() as i64;
            let sum = near.iter().fold(FxVec3::ZERO, |a, e| a + e.pos);
            let centre = FxVec3::new(sum.x / n, Fx::ZERO, sum.z / n);
            let home = centre + (self.base - centre).xz().normalize().scale(fx(120));
            let reachable = probe.is_none_or(|(from, spec)| map::can_reach(&sim.world, &spec, from, home, 3));
            if reachable {
                fields.push((value, home));
            }
        }
        fields.sort_by_key(|&(v, p)| (Reverse(v), p.x.0, p.z.0));
        self.posts.extend(fields.into_iter().take(2).map(|(_, p)| p));
    }

    /// A legal site for `type_id` near `near` (spiral search) that `ok` accepts. Ordinary
    /// buildings keep a free lane around them (so the base stays walkable and nobody gets
    /// boxed in), stay out of factory exits and off metal deposits and vents; factories
    /// need an open exit themselves.
    fn find_site(&self, sim: &Sim, type_id: usize, near: FxVec3, max_cells: i32, ok: &dyn Fn(FxVec3) -> bool) -> Option<FxVec3> {
        let t = sim.types.get(type_id);
        let fp = t.footprint;
        let centre = World::pos_to_anchor(near, fp);
        let resource = self.info[type_id].role == Role::Extractor || t.needs_geo;
        let factory = self.info[type_id].role == Role::Factory;
        for r in 0..=max_cells {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue;
                    }
                    let anchor = (centre.0 + dx, centre.1 + dz);
                    let pos = sim.world.anchor_to_pos(anchor, fp);
                    // (The player's helper builds only on ground the player has seen, whatever it
                    // builds: the search outwards from a crowded base never takes it into the dark.)
                    if self.helper && !sim.explored(self.player, pos) {
                        continue;
                    }
                    if !sim.can_place(type_id, pos) {
                        continue;
                    }
                    if !resource && (!Self::lane_free(sim, anchor, fp) || self.covers_resource(pos, fp) || self.blocks_exit(pos, fp)) {
                        continue;
                    }
                    if factory && !self.exit_open(sim, t, pos) {
                        continue;
                    }
                    // Not where this building failed lately (refused or blocked).
                    if self.failed.iter().any(|(ft, fp, _)| *ft == type_id && fp.within_xz(pos, fx(32))) {
                        continue;
                    }
                    if ok(pos) {
                        return Some(pos);
                    }
                }
            }
        }
        None
    }

    /// The strip in front of a factory that its units drive out through (pixels:
    /// x0, z0, x1, z1).
    fn exit_lane(pos: FxVec3, fp: (u32, u32), heading: i32) -> (i64, i64, i64, i64) {
        let dir = heading_vector(heading);
        let front = fp.0.max(fp.1) as i64 * 8;
        let a = pos + dir.scale(fx(front));
        let b = pos + dir.scale(fx(front + 150));
        let hw = fp.0.max(fp.1) as i64 * 8 + 16;
        let (ax, az, bx, bz) = (a.x.to_int(), a.z.to_int(), b.x.to_int(), b.z.to_int());
        (ax.min(bx) - hw, az.min(bz) - hw / 2, ax.max(bx) + hw, az.max(bz) + hw / 2)
    }

    fn blocks_exit(&self, pos: FxVec3, fp: (u32, u32)) -> bool {
        let (x, z) = (pos.x.to_int(), pos.z.to_int());
        let (hx, hz) = (fp.0 as i64 * 8, fp.1 as i64 * 8);
        self.lanes.iter().any(|&(x0, z0, x1, z1)| x + hx > x0 && x - hx < x1 && z + hz > z0 && z - hz < z1)
    }

    /// Whether the ground in front of a new factory (units leave southwards) is open, and
    /// stays so (no deposit or vent an extractor or plant would later go on).
    fn exit_open(&self, sim: &Sim, t: &ta_sim::UnitType, pos: FxVec3) -> bool {
        let lane = Self::exit_lane(pos, t.footprint, 0);
        let inside = |p: FxVec3| {
            let (x, z) = (p.x.to_int(), p.z.to_int());
            x + 24 > lane.0 && x - 24 < lane.2 && z + 24 > lane.1 && z - 24 < lane.3
        };
        if self.deposits.iter().any(|d| d.marked && inside(d.pos)) || self.vents.iter().any(|v| inside(v.pos)) {
            return false;
        }
        // The biggest unit it makes must be able to drive a few cells straight out.
        let Some(spec) = t.ai_build_options.iter().filter_map(|&o| self.info[o].spec).max_by_key(|sp| (sp.footprint.0 * sp.footprint.1, sp.max_slope, sp.max_depth)) else {
            return true;
        };
        let fp = t.footprint;
        let origin = World::footprint_origin(World::pos_to_anchor(pos, fp), fp);
        let cx = origin.0 + fp.0 as i32 / 2;
        let first = origin.1 + fp.1 as i32 + spec.footprint.1 as i32 / 2;
        (0..4).all(|dz| (-1..=1).all(|dx| sim.world.passable(&spec, (cx + dx, first + dz), None)))
    }

    /// No building (or map edge) within `LANE` cells of a footprint: vehicles are three
    /// cells wide, and a base with narrower gaps traps its own constructors.
    fn lane_free(sim: &Sim, anchor: (i32, i32), fp: (u32, u32)) -> bool {
        const LANE: i32 = 3;
        let o = World::footprint_origin(anchor, fp);
        let (w, h) = (fp.0 as i32, fp.1 as i32);
        for z in o.1 - LANE..o.1 + h + LANE {
            for x in o.0 - LANE..o.0 + w + LANE {
                let inside = x >= o.0 && x < o.0 + w && z >= o.1 && z < o.1 + h;
                if !inside && matches!(sim.world.occupant(x, z), Occupant::Unit(_)) {
                    return false;
                }
            }
        }
        true
    }

    fn covers_resource(&self, pos: FxVec3, fp: (u32, u32)) -> bool {
        let r = fx(fp.0.max(fp.1) as i64 * 8 + 40);
        self.deposits.iter().any(|d| d.marked && d.pos.within_xz(pos, r)) || self.vents.iter().any(|v| v.pos.within_xz(pos, r))
    }

    /// Cells of slack a builder needs around a site to work on it.
    fn build_slack(sim: &Sim, b: &Unit, type_id: usize) -> i32 {
        let t = sim.types.get(type_id);
        (sim.unit_type(b).build_distance.to_int() / 16) as i32 + (t.footprint.0.max(t.footprint.1) as i32 + 1) / 2 + 1
    }

    fn builder_reaches(&self, sim: &Sim, b: &Unit, site: FxVec3, type_id: usize) -> bool {
        match self.info[b.type_id].spec {
            None => true,
            Some(spec) => map::can_reach(&sim.world, &spec, b.pos, site, Self::build_slack(sim, b, type_id)),
        }
    }

    /// Whether a unit can get to a point (6 cells off at most). The answers are kept: the
    /// ground's regions never change, and an army of thousands asks about the same few points.
    fn unit_reaches(&self, sim: &Sim, u: &Unit, to: FxVec3) -> bool {
        let Some(spec) = self.info[u.type_id].spec else { return true };
        let region = map::region_at(&sim.world, &spec, u.pos);
        if region == 0 {
            return false;
        }
        let key = (spec, region, World::pos_to_anchor(to, spec.footprint));
        if let Some(&known) = self.reach_known.0.borrow().get(&key) {
            return known;
        }
        let reaches = sim.world.region_within(&spec, region, key.2, 6);
        let mut known = self.reach_known.0.borrow_mut();
        if known.len() >= REACH_KNOWN {
            known.clear();
        }
        known.insert(key, reaches);
        reaches
    }

    /// A site for a building in a zone around the base, reachable by the builder, not
    /// planned for something else, not under fire.
    fn place(&mut self, sim: &Sim, b: &Unit, type_id: usize, zone: Zone, radius: i64, plan: &Plan) -> Option<FxVec3> {
        if self.place_failed.iter().any(|&(t, until)| t == type_id && sim.tick < until) {
            return None;
        }
        let points = [self.zone_point(sim, zone, radius), self.zone_point(sim, zone, radius + 90)];
        let r = fx(sim.types.get(type_id).radius.to_int() + 16);
        let ok = |site: FxVec3| {
            plan.free(site, r)
                && !self.failed.iter().any(|(t, fp, _)| *t == type_id && fp.within_xz(site, fx(32)))
                && self.threat.at(site) == 0
                && !self.lost_spots.iter().any(|(p, t)| sim.tick < t + MINUTE && p.within_xz(site, fx(96)))
                && self.builder_reaches(sim, b, site, type_id)
        };
        let site = points
            .iter()
            .find_map(|&p| self.find_site(sim, type_id, p, 20, &ok))
            // A crowded base: search outwards from the centre.
            .or_else(|| self.find_site(sim, type_id, self.base, 64, &ok));
        if site.is_none() {
            log::debug!("P{} found no site for {} ({:?} zone)", self.player, sim.types.get(type_id).name, zone as u8);
            self.place_failed.retain(|&(t, _)| t != type_id);
            self.place_failed.push((type_id, sim.tick + 15 * SECOND));
        }
        site
    }

    /// Armed enemy strength (cost) within `r` of `pos`.
    fn enemies_near(&self, s: &Survey, pos: FxVec3, r: i64) -> i64 {
        // (Radar contacts: of a kind not known, each taken for an armed one.)
        self.seen_enemies_near(s, pos, r) + s.contacts.iter().filter(|c| c.within_xz(pos, fx(r))).count() as i64 * CONTACT_WORTH
    }

    /// As [`SkirmishAi::enemies_near`], of the armed ground units in sight alone.
    fn seen_enemies_near(&self, s: &Survey, pos: FxVec3, r: i64) -> i64 {
        s.enemies
            .iter()
            .filter(|e| e.is_complete() && self.info[e.type_id].ground_dps > 0 && self.info[e.type_id].kind != Locomotion::Air && e.pos.within_xz(pos, fx(r)))
            .map(|e| self.info[e.type_id].worth)
            .sum()
    }

    /// The enemy bases kept, by `keep` (one flag each), their owners and seen flags with them.
    fn keep_bases(&mut self, keep: &[bool]) {
        let mut k = keep.iter();
        self.enemy_bases.retain(|_| *k.next().unwrap_or(&true));
        let mut k = keep.iter();
        self.enemy_base_owners.retain(|_| *k.next().unwrap_or(&true));
        let mut k = keep.iter();
        self.enemy_base_seen.retain(|_| *k.next().unwrap_or(&true));
    }

    /// What we have seen of where the enemy is, into `enemy_bases`: a start with enemy
    /// buildings in sight round it is a base of theirs (and whose); one looked at with none
    /// there, seen or remembered, is dropped (a base too, gone); an enemy factory far from
    /// every one is a base of theirs found. The last one is kept whatever we see (the enemy is
    /// somewhere: see `run_scouting`). Whether anything changed.
    fn update_bases(&mut self, sim: &Sim, s: &Survey) -> bool {
        let me = self.player;
        let mut changed = false;
        for k in 0..self.enemy_bases.len() {
            let b = self.enemy_bases[k];
            let near = s.enemies.iter().filter(|e| sim.unit_type(e).locomotion == Locomotion::Static && e.pos.within_xz(b, fx(BASE_NEAR))).min_by_key(|e| (dist(e.pos, b), e.id.slot));
            if let Some(e) = near
                && (!self.enemy_base_seen[k] || self.enemy_base_owners[k] != e.owner)
            {
                log::debug!("{} min: P{me} finds P{}'s base at ({}, {})", s.minutes, e.owner, b.x.to_int(), b.z.to_int());
                self.enemy_base_seen[k] = true;
                self.enemy_base_owners[k] = e.owner;
                changed = true;
            }
        }
        let empty: Vec<bool> = self
            .enemy_bases
            .iter()
            .map(|&b| sim.in_sight(me, b) && !self.enemy_buildings.iter().any(|(_, p, _)| p.within_xz(b, fx(BASE_NEAR))))
            .collect();
        if empty.iter().any(|&e| e) {
            // (The last kept: a base seen if there is one, else the first.)
            let mut keep: Vec<bool> = empty.iter().map(|&e| !e).collect();
            if !keep.iter().any(|&k| k) {
                let last = (0..keep.len()).max_by_key(|&k| (self.enemy_base_seen[k], Reverse(k))).unwrap_or(0);
                keep[last] = true;
            }
            if keep.iter().any(|&k| !k) {
                log::debug!("{} min: P{me} finds {} of the enemy's places empty", s.minutes, keep.iter().filter(|&&k| !k).count());
                self.keep_bases(&keep);
                changed = true;
            }
        }
        // Every enemy found (a base of each seen): the starts not looked at are nobody's.
        let found = |o: u8| self.enemy_base_owners.iter().zip(&self.enemy_base_seen).any(|(&w, &seen)| seen && w == o);
        let mut enemies = (0..sim.players.len() as u8).filter(|&o| sim.is_enemy(me, o) && !sim.players[o as usize].defeated).peekable();
        if enemies.peek().is_some() && enemies.all(found) && self.enemy_base_seen.iter().any(|&seen| !seen) {
            log::debug!("{} min: P{me} has found every enemy", s.minutes);
            let keep = self.enemy_base_seen.clone();
            self.keep_bases(&keep);
            changed = true;
        }
        for e in s.enemies.iter().filter(|e| self.info[e.type_id].role == Role::Factory) {
            if !self.enemy_bases.iter().any(|b| b.within_xz(e.pos, fx(FOUND_APART))) {
                log::debug!("{} min: P{me} finds a base of P{}'s at ({}, {})", s.minutes, e.owner, e.pos.x.to_int(), e.pos.z.to_int());
                self.enemy_bases.push(e.pos);
                self.enemy_base_owners.push(e.owner);
                self.enemy_base_seen.push(true);
                changed = true;
            }
        }
        changed
    }

    /// The plans made against the enemy's bases made again, as they change: which units get
    /// to the nearest by land, whether this is an island war, and what each factory's units
    /// and ships reach (found again as asked).
    fn replan(&mut self, sim: &Sim) {
        let Some(enemy) = self.nearest_enemy_base(self.base) else { return };
        let mut by_spec: Vec<(ta_sim::world::MoveSpec, bool)> = Vec::new();
        let base = self.base;
        self.crosses = self
            .info
            .iter()
            .map(|i| match i.spec {
                Some(sp) if i.kind != Locomotion::Naval => match by_spec.iter().find(|(s, _)| *s == sp) {
                    Some(&(_, r)) => r,
                    None => {
                        let r = map::can_reach(&sim.world, &sp, base, enemy, 8);
                        by_spec.push((sp, r));
                        r
                    }
                },
                _ => true,
            })
            .collect();
        self.island = !self.info.iter().enumerate().any(|(t, i)| i.role == Role::Combat && i.kind == Locomotion::Ground && i.tech == 1 && self.crosses[t]);
        self.reach.iter_mut().for_each(|r| *r = None);
        self.reaches.clear();
        if let Some(n) = &self.naval {
            let reaches = self.yard_reaches(sim, n.yard, n.site);
            self.naval.as_mut().unwrap().reaches = reaches;
        }
    }
}
