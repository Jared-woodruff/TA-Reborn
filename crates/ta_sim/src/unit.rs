//! Units, orders and players.

use std::collections::VecDeque;

use crate::cob::CobInstance;
use crate::defs::UnitType;
use crate::fixed::{Fx, FxVec3, rotate_x, rotate_y, rotate_z, wrap_angle};
use crate::rng::Rng;

/// Most players a game can have: per-player sets (who sees through a cloak, which players
/// have units in a cell of the unit grid, a player's enemies) are `u64` bit masks.
pub const MAX_PLAYERS: usize = 64;

/// A player's bit in a per-player mask (none for players past [`MAX_PLAYERS`]).
#[inline]
pub fn player_bit(player: u8) -> u64 {
    1u64.checked_shl(player as u32).unwrap_or(0)
}

/// Stable handle to a unit: slot index plus a generation that changes when the slot is
/// reused, so stale references never alias a new unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct UnitId {
    pub slot: u32,
    pub generation: u32,
}

impl UnitId {
    /// Integer id exposed to COB scripts (`UNIT_XZ(id)` etc.).
    pub fn script_id(self) -> i32 {
        self.slot as i32 + 1
    }
}

/// What a weapon or attack order is aimed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Target {
    /// A projectile in flight (anti-missile interceptors).
    Projectile(u32),
    Unit(UnitId),
    Ground(FxVec3),
}

/// Per-weapon runtime state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponState {
    pub reload: u32,
    pub target: Option<Target>,
    /// The newest aim the script answered (for the current target) said yes.
    pub aimed: bool,
    /// The solution the newest aim was asked with, and when.
    pub aim_heading: i32,
    pub aim_pitch: i32,
    pub last_aim_tick: u64,
    /// The script was asked to aim since the weapon last fired, or since its target changed:
    /// it is asked again after every shot, as TA did (turrets rest twice the reload time after
    /// their last aim, `SetMaxReloadTime`, so a gun firing on keeps its aim).
    pub aim_asked: bool,
    pub burst_left: u32,
    pub burst_timer: u32,
    /// A target the ground hides from this weapon's shots, passed over by automatic
    /// targeting until the tick given.
    pub blocked: Option<(UnitId, u64)>,
    /// The last look for friends in the way of a shot at a target: the target, the tick it
    /// holds until, and whether the way was clear.
    pub friend_check: Option<(Target, u64, bool)>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Order {
    /// Move to a point.
    Move(FxVec3),
    /// Attack a unit or a ground position.
    Attack(Target),
    /// Attack-move: head for a point, engaging enemies met on the way.
    Fight(FxVec3),
    /// Reclaim a feature (wreck, rock, tree) for its metal and energy.
    Reclaim(usize),
    /// Raise a unit wreck (feature index) back into the unit it was.
    Resurrect(usize),
    /// Take one of our own units apart for its metal.
    ReclaimUnit(UnitId),
    /// Fire command-only weapons (the Commander's D-gun) once at a target.
    ManualFire(Target),
    /// Construct a unit of `type_id` centred at `pos` (mobile builders).
    Build { type_id: usize, pos: FxVec3 },
    /// Help finish / repair another unit.
    Repair(UnitId),
    /// Patrol towards a point; reached patrol points go to the back of the queue so
    /// consecutive patrol orders loop. Engages enemies on the way; builders also repair,
    /// assist and reclaim what they pass.
    Patrol(FxVec3),
    /// Stay with a unit and protect it; builders help it build and keep it repaired.
    Guard(UnitId),
    /// Take over an enemy unit (units with `CanCapture`, e.g. the Commander).
    Capture(UnitId),
    /// Mission scripts: do nothing for this many ticks.
    Wait(u32),
    /// Mission scripts: do nothing until this unit (or the given one) is attacked.
    WaitAttacked(Option<UnitId>),
    /// Mission scripts: attack the nearest enemy unit of a type.
    AttackType(usize),
    /// Mission scripts: set standing fire and move orders.
    SetStates { fire: u8, movement: u8 },
    /// Transports: pick up a unit.
    Load(UnitId),
    /// Transports: set cargo down at a point.
    Unload(FxVec3),
    Stop,
    /// Become the unit type given, built in place on itself (its type's `UpgradesTo`: Supreme
    /// Commander's factories, extractors, radars...), with its own build power and that of the
    /// builders helping it, paying as construction does (see `Sim::update_upgrade_order`). Taken
    /// off its orders, the upgrade is called off. (After `Stop`: records keep the earlier
    /// variants' numbers.)
    Upgrade(usize),
    /// Install (or with `remove`, take out) its type's enhancement `index` (`[ENHANCEMENTS]`:
    /// Supreme Commander's commanders'): installed, it stands and builds it on itself as an
    /// upgrade is built, in its slot in place of the one there; removed, at once, for nothing
    /// (see `sim/enhance.rs`).
    Enhance { index: u16, remove: bool },
}

/// What a unit builds on itself: the type it is becoming ([`Order::Upgrade`]), or an enhancement
/// of its type's ([`Order::Enhance`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfJob {
    Upgrade(usize),
    Enhance(u16),
}

/// A unit's figures with the enhancements it has installed (its type's with each one's added: see
/// `sim/enhance.rs`), where it has any; [`Unit`]'s accessors give its type's otherwise.
#[derive(Clone, Debug, PartialEq)]
pub struct Boost {
    pub max_health: Fx,
    /// Ticks it takes to mend 100 hit points (none: it does not).
    pub heal_ticks: u64,
    pub work_rate: u64,
    pub worker_rate: Fx,
    pub sight: Fx,
    pub radar: Fx,
    pub sonar: Fx,
    /// The weapon in each slot.
    pub weapons: [Option<usize>; ta_data::unit::MAX_WEAPONS],
    pub stockpile: bool,
    /// Its menu, and all it may be ordered to build.
    pub build_options: Vec<usize>,
    pub ai_build_options: Vec<usize>,
    /// Its cloak, as `UnitType`'s def gives one (energy a second, on the move, seen within).
    pub cloak: Option<(Fx, Fx, Fx)>,
    pub stealth: bool,
}

/// What the buildings of its player beside a building give it (Supreme Commander's adjacency:
/// `sim/adjacency.rs`), summed: the share each of its figures changes by, and who gives it what.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Adjacency {
    /// The share added to each figure (by [`ta_data::AdjacencyAffect::index`]): the sum of what
    /// its neighbours give it.
    pub change: [Fx; ADJACENCY_AFFECTS],
    /// The neighbours whose bonuses it takes, by slot, each with the figures it changes (a bit for
    /// each affect's index).
    pub from: Vec<(UnitId, u8)>,
}

/// How many figures adjacency changes ([`ta_data::AdjacencyAffect::ALL`]).
pub const ADJACENCY_AFFECTS: usize = ta_data::AdjacencyAffect::ALL.len();

impl Adjacency {
    /// What a figure is multiplied by: one and the shares its neighbours add, never below none.
    #[inline]
    pub fn factor(&self, affect: ta_data::AdjacencyAffect) -> Fx {
        (Fx::ONE + self.change[affect.index()]).max(Fx::ZERO)
    }
}

/// A unit's work on itself under way: what, and the work done on it so far (of its whole, as
/// construction counts work: [`crate::UnitType::build_work`] for an upgrade). Called off, the
/// metal paid for it comes back (`Sim::cancel_self_work`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelfWork {
    pub job: SelfJob,
    pub work: u64,
    /// The metal adjacency spared those working on it so far: called off, what was paid comes
    /// back, not its price.
    pub metal_saved: Fx,
}

/// A construction aircraft's sway round what it builds, facing it (`Sim::drift_round`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drift {
    /// What it sways round.
    pub target: UnitId,
    /// The tick the sway began.
    pub since: u64,
    /// The last tick it swayed: a sway not kept up begins again where the aircraft is.
    pub last: u64,
    /// Where round what it builds the sway began (a heading from it) and how far out.
    pub angle: i32,
    pub distance: Fx,
    /// The way it sways first: 1 anticlockwise seen from above, -1 clockwise.
    pub side: i8,
}

/// A unit made by a factory with ways out (`UnitType::exits`, Supreme Commander's roll-off
/// points): the way it is built facing and leaves by, and once it is let go its way on the
/// factory's floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RollOff {
    /// The factory's way out (an index of its type's `exits`).
    pub exit: u8,
    /// Once it is let go, its way out on the factory's floor.
    pub way: Option<Way>,
}

/// A unit's way out of a factory once it is let go ([`RollOff::way`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Way {
    /// Where it set off from: the pad, at the pad's height.
    pub from: FxVec3,
    /// Where the way ends, on the ground.
    pub to: FxVec3,
    /// Pixels along the way: the pad's height kept to the first, the ground reached at the
    /// second.
    pub ramp: (Fx, Fx),
}

impl Way {
    /// How high the factory's floor stands at `p` on the way out, `ground` the ground's height
    /// there: the pad's as far as the ramp's top (measured along the way), down evenly to the
    /// ground at its foot; none past it (or for a way of no length), only the ground.
    pub fn floor(&self, p: FxVec3, ground: Fx) -> Option<Fx> {
        let along = (self.to - self.from).xz();
        let length = along.length_xz();
        if length <= Fx::ZERO {
            return None;
        }
        let d = (p - self.from).xz().dot(along) / length;
        let (top, foot) = self.ramp;
        if d <= top {
            Some(self.from.y)
        } else if d >= foot {
            None
        } else {
            Some(self.from.y + (ground - self.from.y) * ((d - top) / (foot - top)))
        }
    }
}

/// Progress of a builder's current job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildJob {
    pub target: UnitId,
    /// `StartBuilding` has been called.
    pub started: bool,
}

/// Ticks a driven unit waits for its owner's next input before it is let go (three seconds): the
/// view sends one whenever its input changes and once a second besides, so only a player gone, or
/// a game that stopped sending, lets it go this way.
pub const DIRECT_CONTROL_TICKS: u64 = 90;

/// A player's hands on a unit they drive (`Command::DirectControl`, from the first-person view).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DirectInput {
    /// Ahead (1), astern (-1) or neither (0).
    pub throttle: i8,
    /// The hull turning right (1), left (-1) or neither (0).
    pub turn: i8,
    /// Where the sight looks: the world's heading (TA's angle units, -32768..32767)...
    pub heading: i32,
    /// ...and its pitch above the level (-16384..16384, straight down to straight up).
    pub pitch: i32,
    /// The trigger held (or clicked since the last input).
    pub fire: bool,
}

/// A unit driven by its owner (`Sim::set_direct_control`): their latest input, held until the next.
#[derive(Clone, Copy, Debug)]
pub struct DirectControl {
    /// The piece the unit's first weapon fires from (its query's answer), found on its first tick
    /// driven: where the view's sight is drawn from.
    pub aim_piece: Option<usize>,
    pub player: u8,
    pub input: DirectInput,
    /// The tick it is let go at unless another input comes first ([`DIRECT_CONTROL_TICKS`]).
    pub expires: u64,
}

#[derive(Clone)]
pub struct Unit {
    /// Driven by its owner from the first-person view: their input, its orders waiting meanwhile.
    pub direct_control: Option<DirectControl>,
    pub id: UnitId,
    pub type_id: usize,
    pub owner: u8,
    pub pos: FxVec3,
    /// Yaw in TA angle units (0 = facing +z/south).
    pub heading: i32,
    /// Terrain tilt (visual and piece positions).
    pub pitch: i32,
    pub roll: i32,
    /// Forward speed in pixels per tick.
    pub speed: Fx,
    pub health: Fx,
    /// 0..=1; ONE when finished (see [`Unit::set_build_work`]).
    pub build_progress: Fx,
    /// Work done building it, of its type's [`UnitType::build_work`]: `build_progress` is
    /// derived from it (exact, where adding fractions of a tick's work would round to nothing
    /// on huge build times).
    pub build_work: u64,
    /// Work accumulated through a repair cycle, retaining healing below Fx precision.
    pub repair_work: u64,
    /// It has been finished (built whole, or put down whole), and is never finished again:
    /// taken apart by its own side it falls back below whole (`build_progress`), and built back
    /// up it is simply whole once more, without a second completion (`Sim::apply_build_power`).
    pub finished: bool,
    /// The last tick one of its own side's builders took work off it, reclaiming it: for a
    /// second after that it is being taken apart (`Sim::being_taken_apart`).
    pub taken_apart_at: Option<u64>,
    /// The metal its side's reclaiming has taken off it so far, paid when it is gone: TA pays
    /// reclaiming at the end, not as the work goes. Lost with it if it dies first.
    pub reclaim_owed: Fx,
    /// The metal adjacency spared its builders (`sim/adjacency.rs`; for one upgraded in place, those
    /// who worked on the upgrade): given up unfinished, or taken apart by its side, it gives back
    /// what was paid for it, not its price.
    pub metal_saved: Fx,
    /// The last tick a builder worked on it unfinished (or set it down to build): construction
    /// left with nobody at it un-builds after a while, its metal coming back
    /// (`Sim::decay_abandoned_frames`). None for one nobody has worked on (a scene's or a
    /// mission's, placed unfinished).
    pub worked_at: Option<u64>,
    /// The site a mobile unit built in the field (not in a factory) was laid on: the builders
    /// ordered to build at that site work on it, wherever it has been pushed since; a frame laid
    /// for another site, however near, is not theirs (`Sim::field_frame`).
    pub field_site: Option<FxVec3>,
    pub activated: bool,
    /// The last economy tick charged its upkeep (or it has none): its conditional output (an
    /// extractor's, a metal maker's) came too. Without the energy, neither (`Sim::economy_end`).
    pub upkeep_paid: bool,
    /// Ready to build (`INBUILDSTANCE`): the work goes on only while it is. A builder with a
    /// `StartBuilding` script has it as its script sets it, as in TA, also between jobs
    /// ([`Unit::stance_is_scripted`]); the game raises and drops a builder's without one.
    pub in_build_stance: bool,
    pub yard_open: bool,
    pub armored: bool,
    pub busy: bool,
    /// Its script holds it still (`IMMOBILE`: a weapon of Supreme Commander's unpacked that keeps
    /// its unit where it stands, `WeaponUnpackLocksMotion`): it neither moves nor turns, its way
    /// waiting; with somewhere to go, its script is told `StartMoving` so that it packs up and
    /// lets it go (`Sim::ground_step`), firing nothing meanwhile ([`Unit::packing_to_go`]).
    pub immobile: bool,
    pub fire_state: u8,
    /// Fires at will on what its side has on radar or sonar as well as on what it sees: guns a
    /// spotter directs, a scenario's (`radar_guided`: Iron Rain's battery, which a radar post
    /// spots for). Every other unit's fire at will takes on only what its side sees, but where
    /// its player has a targeting facility at work ([`Player::radar_targeting`]).
    pub radar_guided: bool,
    pub move_state: u8,
    pub orders: VecDeque<Order>,
    /// Remaining waypoints (anchor cells) of the current path.
    pub path: Vec<(i32, i32)>,
    pub path_goal: Option<FxVec3>,
    /// Destination as requested by the current order (the path goal may be a reachable
    /// substitute for it).
    pub move_request: Option<FxVec3>,
    /// The current path stops short of the goal (search budget); re-plan on arrival.
    pub path_partial: bool,
    /// Recent work targets found to be out of reach (see `Sim::work_key`), so automatic
    /// behaviours (patrol, guard) don't retry them forever.
    pub unreachable: Vec<u64>,
    /// Aircraft: sitting on the ground (idle aircraft land, as in TA).
    pub landed: bool,
    /// Reserved repair deck and model piece (None for an unscripted platform).
    pub air_pad: Option<(UnitId, Option<usize>)>,
    /// Aircraft: broken off its patrol, attack-move or attack, worn down, for the deck it has
    /// reserved (`air_pad`): its orders wait until it is mended there, or the deck is lost or
    /// switched off (`Sim::start_repair_trip`).
    pub repair_trip: bool,
    /// Aircraft: flying an attack pass (no braking at the pass goal).
    pub air_pass: bool,
    /// Aircraft: ticks without anything to do (landing starts after a short delay).
    pub idle_ticks: u32,
    /// Aircraft: ticks spent on the current attack pass.
    pub pass_ticks: u32,
    /// Aircraft: past the target, flying the straight run-out before turning back.
    pub pass_runout: bool,
    /// A plane's place in a group sent at one target together: 0 for the first (or one sent
    /// alone), then 1, -1, 2, -2... to either side, where it comes in on its attack runs (the
    /// group fanned out across a front, not in single file: see `Sim::air_attack_pass`).
    pub fan: i8,
    /// Where an idle plane that cannot set down here (over the sea) circles round, as TA's do:
    /// where it came to rest in the air.
    pub orbit: Option<FxVec3>,
    /// A construction aircraft at work: its sway round what it builds (`Sim::drift_round`).
    pub drift: Option<Drift>,
    /// Capture progress 0..=1 accumulated by enemy capturers, and the work behind it (out of
    /// the cost a capture takes, see `Sim::update_capture_order`).
    pub capture_progress: Fx,
    pub capture_work: u64,
    /// Cloaking: wanted by the player, active this tick (paid for, not just fired),
    /// shimmering visible after firing until this tick, and the players whose units are
    /// close enough to see through the cloak (a [`player_bit`] each).
    pub cloak_on: bool,
    pub cloaked: bool,
    pub decloak_until: u64,
    pub revealed_to: u64,
    /// Stockpiled missiles ready to fire (nukes, anti-nukes) and progress on the next one:
    /// 0..1, from the ticks of build power behind it (each a fraction of a tick by the stall
    /// factor, raw 16.16) out of the missile's reload ticks.
    pub stockpile: u32,
    pub stockpile_progress: Fx,
    pub stockpile_work: u64,
    /// Transport this unit rides in, and the model piece it hangs from (`None` = stowed
    /// inside, not drawn).
    pub carried_by: Option<UnitId>,
    pub carry_piece: Option<usize>,
    /// Transports: units aboard, in loading order.
    pub cargo: Vec<UnitId>,
    /// Transports: the unit being loaded/unloaded by script and since when.
    pub transport_job: Option<(UnitId, u64)>,
    /// Aircraft: altitude to hold instead of cruising height (transports picking up).
    pub hover_alt: Option<Fx>,
    /// Aircraft: a cruising height of its own over the ground, in place of its type's
    /// `CruiseAlt` (set up for a staged scene; see [`crate::Sim::set_cruise_alt`]).
    pub cruise_alt: Option<Fx>,
    pub stuck_ticks: u32,
    /// The nearest it has come to where it is heading now (its goal on the last leg of its
    /// route, else its next waypoint), the ticks the crowd has pushed it back since it last
    /// came nearer, and of those the ticks its own side's units coming the other way pushed it
    /// (a crowd at the goal, or others on its next waypoint, keep it off: see
    /// `Sim::ground_step`).
    pub approach_best: Fx,
    pub approach_idle: u32,
    pub head_on_idle: u32,
    /// The last tick the crowd pushed it, and the last its own side's units coming the other
    /// way did (`Sim::resolve_collisions`).
    pub jostled_tick: u64,
    pub head_on_tick: u64,
    /// Until this tick it slips past its own side's units coming the other way (two held up
    /// head on in a gap one unit wide).
    pub squeeze_until: u64,
    /// Whether `StartMoving` has been signalled to the script.
    pub moving: bool,
    pub build: Option<BuildJob>,
    /// What it builds on itself now (an upgrade into another type, an enhancement), and how far
    /// along.
    pub self_work: Option<SelfWork>,
    /// Its type's enhancements it has installed (indices, one a slot, in order), and its figures
    /// with them (none without).
    pub enhancements: Vec<u16>,
    pub boost: Option<Box<Boost>>,
    /// What the buildings of its player beside it give it (none: nothing, as every unit of TA's).
    pub adjacency: Option<Box<Adjacency>>,
    /// Where it stood for adjacency when last looked at (`Sim::update_adjacency`): its player and
    /// its footprint (the top-left cell, its size), while it stands finished; none otherwise.
    pub adjacency_key: Option<(u8, (i32, i32), (u32, u32))>,
    /// Its only order is work it found itself (a building that works round it: `RepairPatrol`,
    /// `AssistPatrol`): the player's next order, queued or not, takes its place.
    pub auto_work: bool,
    /// Factory production queue (unit type ids), and the orders every unit it releases
    /// is given (its rally route: move, patrol, fight and guard orders).
    pub factory_queue: VecDeque<usize>,
    pub rally: Vec<Order>,
    /// Factory that is building / releasing this unit.
    pub factory: Option<UnitId>,
    /// Made by a factory with ways out (`UnitType::exits`): the one it is built facing and leaves
    /// by, and once it is let go the floor under it on its way out (`Sim::settle`). Gone once it is
    /// out of the factory's footprint.
    pub roll_off: Option<RollOff>,
    /// Factories: the units they built that may still be leaving their yard (those whose
    /// `factory` is this one; the others are dropped as they are found).
    pub yard: Vec<UnitId>,
    /// Footprint origin cell for buildings that occupy the grid.
    pub occupied_origin: Option<(i32, i32)>,
    pub cob: Option<CobInstance>,
    /// Its weapons' states by slot ([`ta_data::unit::MAX_WEAPONS`]).
    pub weapons: [WeaponState; ta_data::unit::MAX_WEAPONS],
    pub last_attacker: Option<UnitId>,
    /// Size of the most recent hit (determines death severity).
    pub last_damage: Fx,
    pub killer: Option<UnitId>,
    pub kills: u64,
    /// Experience: kills weighted by the victim's value relative to this unit's.
    pub xp: Fx,
    pub dead: bool,
    /// Death effects (Killed script, wreck, explosion) have been applied.
    pub death_processed: bool,
    /// Taken off the map by the game's set-up (`Sim::remove_unit`), not lost in battle.
    pub removed: bool,
    /// Tick at which the unit self-destructs (defeated players' bases blow up).
    pub self_destruct_at: Option<u64>,
    /// The countdown was started by the player (the unit blows up as `SelfDestructAs`).
    pub self_destruct_manual: bool,
    /// Factories: finished units go back on the end of the queue.
    pub factory_repeat: bool,
    /// Tick the unit last took damage.
    pub last_hit_tick: u64,
    /// Stunned by a paralyzer (EMP) weapon until this tick.
    pub paralyzed_until: u64,
    /// Where an idle unit stood before it went after an attacker (it returns there).
    pub idle_anchor: Option<FxVec3>,
    /// Aircraft: the `MoveRate` last signalled (0..3; 255 none yet).
    pub move_rate: u8,
    /// What the script was last told the unit moves over (`SetSFXoccupy`; 255 not yet).
    pub sfx_occupy: u8,
    /// Builders: the COB piece the nano spray leaves from, as the script last answered.
    pub nano_piece: Option<u32>,
    /// The random numbers its script draws (its own, from the game's seed and its id: a
    /// script's draws don't depend on any other unit's, so the scripts can run all at once).
    pub rng: Rng,
    /// The formations it is in (the rule "modern movement", `formation.rs`): each one's id
    /// and the place in it its order goes to, one for each group order in its queue.
    pub formations: Vec<(u32, FxVec3)>,
    /// The way to turn to once it stands (its formation's, on arriving; for units that turn
    /// standing).
    pub face: Option<i32>,
}

/// A unit's own random numbers: from the game's `seed` and its id.
pub fn unit_rng(seed: u64, id: UnitId) -> Rng {
    Rng::new(seed ^ (id.slot as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (id.generation as u64).rotate_left(32))
}

impl Unit {
    /// Veterancy 0..1 from experience, with diminishing returns: one even kill gives 0.25,
    /// three give 0.5, six about 0.67.
    pub fn veterancy(&self) -> Fx {
        self.xp / (self.xp + Fx::from_int(3))
    }

    /// Rank shown to players: 0 recruit, 1 veteran, 2 hardened, 3 elite.
    pub fn rank(&self) -> u8 {
        let v = self.veterancy();
        if v >= Fx::from_ratio(2, 3) {
            3
        } else if v >= Fx::from_ratio(1, 2) {
            2
        } else if v >= Fx::from_ratio(1, 4) {
            1
        } else {
            0
        }
    }

    /// A unit with default state; used by the simulation and for previews.
    pub fn new(id: UnitId, type_id: usize, owner: u8, pos: FxVec3) -> Unit {
        Unit {
            direct_control: None,
            id,
            type_id,
            owner,
            pos,
            heading: 0,
            pitch: 0,
            roll: 0,
            speed: Fx::ZERO,
            health: Fx::ONE,
            build_progress: Fx::ONE,
            build_work: 0,
            repair_work: 0,
            finished: false,
            taken_apart_at: None,
            reclaim_owed: Fx::ZERO,
            metal_saved: Fx::ZERO,
            worked_at: None,
            field_site: None,
            activated: false,
            upkeep_paid: true,
            in_build_stance: false,
            yard_open: false,
            armored: false,
            busy: false,
            immobile: false,
            fire_state: 2,
            radar_guided: false,
            move_state: 1,
            orders: VecDeque::new(),
            path: Vec::new(),
            path_goal: None,
            move_request: None,
            path_partial: false,
            unreachable: Vec::new(),
            landed: false,
            air_pad: None,
            repair_trip: false,
            air_pass: false,
            idle_ticks: 0,
            pass_ticks: 0,
            pass_runout: false,
            fan: 0,
            orbit: None,
            drift: None,
            capture_progress: Fx::ZERO,
            capture_work: 0,
            cloak_on: false,
            cloaked: false,
            decloak_until: 0,
            revealed_to: 0,
            stockpile: 0,
            stockpile_progress: Fx::ZERO,
            stockpile_work: 0,
            carried_by: None,
            carry_piece: None,
            cargo: Vec::new(),
            transport_job: None,
            hover_alt: None,
            cruise_alt: None,
            stuck_ticks: 0,
            approach_best: Fx::MAX,
            approach_idle: 0,
            head_on_idle: 0,
            jostled_tick: 0,
            head_on_tick: 0,
            squeeze_until: 0,
            moving: false,
            build: None,
            self_work: None,
            enhancements: Vec::new(),
            boost: None,
            adjacency: None,
            adjacency_key: None,
            auto_work: false,
            factory_queue: VecDeque::new(),
            rally: Vec::new(),
            factory: None,
            roll_off: None,
            yard: Vec::new(),
            occupied_origin: None,
            cob: None,
            weapons: [WeaponState::default(); ta_data::unit::MAX_WEAPONS],
            last_attacker: None,
            last_damage: Fx::ZERO,
            killer: None,
            kills: 0,
            xp: Fx::ZERO,
            dead: false,
            death_processed: false,
            removed: false,
            self_destruct_at: None,
            self_destruct_manual: false,
            factory_repeat: false,
            last_hit_tick: 0,
            paralyzed_until: 0,
            idle_anchor: None,
            move_rate: u8::MAX,
            sfx_occupy: u8::MAX,
            nano_piece: None,
            rng: unit_rng(0, id),
            formations: Vec::new(),
            face: None,
        }
    }
}

impl Unit {
    pub fn is_complete(&self) -> bool {
        self.build_progress >= Fx::ONE
    }

    /// Its most health: its type's, with its enhancements'.
    #[inline]
    pub fn max_health(&self, t: &UnitType) -> Fx {
        self.boost.as_ref().map_or(t.max_health, |b| b.max_health)
    }

    /// The weapon in each of its slots: its type's, as its enhancements fill and swap them.
    #[inline]
    pub fn weapon_slots<'a>(&'a self, t: &'a UnitType) -> &'a [Option<usize>; ta_data::unit::MAX_WEAPONS] {
        self.boost.as_ref().map_or(&t.weapons, |b| &b.weapons)
    }

    /// Whether it has a weapon at all.
    #[inline]
    pub fn armed(&self, t: &UnitType) -> bool {
        self.weapon_slots(t).iter().any(Option::is_some)
    }

    /// Whether one of its weapons builds missiles to launch.
    #[inline]
    pub fn stockpiles(&self, t: &UnitType) -> bool {
        self.boost.as_ref().map_or(t.stockpile, |b| b.stockpile)
    }

    /// Its build power: work a tick (as [`UnitType::work_rate`]), and points a tick.
    #[inline]
    pub fn work_rate(&self, t: &UnitType) -> u64 {
        self.boost.as_ref().map_or(t.work_rate, |b| b.work_rate)
    }

    #[inline]
    pub fn worker_rate(&self, t: &UnitType) -> Fx {
        self.boost.as_ref().map_or(t.worker_rate, |b| b.worker_rate)
    }

    /// Its menu (its type's, and what its enhancements add), and all it may be ordered to build.
    #[inline]
    pub fn build_options<'a>(&'a self, t: &'a UnitType) -> &'a [usize] {
        self.boost.as_ref().map_or(&t.build_options, |b| &b.build_options)
    }

    #[inline]
    pub fn ai_build_options<'a>(&'a self, t: &'a UnitType) -> &'a [usize] {
        self.boost.as_ref().map_or(&t.ai_build_options, |b| &b.ai_build_options)
    }

    /// How far it sees, and its radar's and sonar's reach.
    #[inline]
    pub fn sight(&self, t: &UnitType) -> Fx {
        self.boost.as_ref().map_or(t.sight, |b| b.sight)
    }

    #[inline]
    pub fn radar(&self, t: &UnitType) -> Fx {
        self.boost.as_ref().map_or(t.radar, |b| b.radar)
    }

    #[inline]
    pub fn sonar(&self, t: &UnitType) -> Fx {
        self.boost.as_ref().map_or_else(|| Fx::from_f64(t.def.sonar_distance), |b| b.sonar)
    }

    /// Ticks it takes to mend 100 hit points (none: it does not).
    #[inline]
    pub fn heal_ticks(&self, t: &UnitType) -> u64 {
        self.boost.as_ref().map_or(t.heal_ticks, |b| b.heal_ticks)
    }

    /// Its cloak, if it can cloak: energy a second, on the move, and how near an enemy sees
    /// through it.
    pub fn cloak_costs(&self, t: &UnitType) -> Option<(Fx, Fx, Fx)> {
        match &self.boost {
            Some(b) => b.cloak,
            None => (t.def.cloak_cost > 0.0).then(|| (Fx::from_f64(t.def.cloak_cost), Fx::from_f64(t.def.cloak_cost_moving), Fx::from_f64(t.def.min_cloak_distance))),
        }
    }

    /// Whether radar does not show it.
    #[inline]
    pub fn stealthy(&self, t: &UnitType) -> bool {
        self.boost.as_ref().map_or(t.def.stealth, |b| b.stealth)
    }

    /// What one of its figures is multiplied by for the buildings beside it ([`Adjacency::factor`]);
    /// none: it stands by none that give it anything.
    #[inline]
    pub fn adjacency_factor(&self, affect: ta_data::AdjacencyAffect) -> Option<Fx> {
        self.adjacency.as_ref().filter(|a| !a.change[affect.index()].is_zero()).map(|a| a.factor(affect))
    }

    /// Whether its build stance is its script's to raise and drop (it has `StartBuilding`), as
    /// in TA: the game leaves it as the script last set it, also after a job. TA's scripts drop
    /// it in `StopBuilding`; Total Mayhem's construction kbots keep their nano arm out a while
    /// after a job, the stance raised, and do not raise it again for a job that comes meanwhile.
    pub fn stance_is_scripted(&self) -> bool {
        self.cob.as_ref().is_some_and(|c| c.has_function("StartBuilding"))
    }

    /// Held still by its script (`IMMOBILE`) and told to go: its script is packing it up so that
    /// it can, and its weapons fire nothing meanwhile, as Supreme Commander's fire nothing as
    /// they pack (`WeaponPackingState`).
    pub fn packing_to_go(&self) -> bool {
        self.immobile && self.moving
    }

    /// Physical model root. Naval movement keeps a surface anchor; submarine
    /// geometry sits WaterLine below it, as the client draws it.
    pub fn model_pos(&self, t: &UnitType) -> FxVec3 {
        self.pos - FxVec3::new(Fx::ZERO, t.waterline, Fx::ZERO)
    }

    /// A new route: the stuck count and the approach to its goal start over.
    pub(crate) fn new_route(&mut self) {
        self.stuck_ticks = 0;
        self.new_leg();
    }

    /// A new leg of its route (the next waypoint): the approach to it starts over.
    pub(crate) fn new_leg(&mut self) {
        self.approach_best = Fx::MAX;
        self.approach_idle = 0;
        self.head_on_idle = 0;
    }

    /// Points weapon `slot` at `target` (or at nothing). Its aims so far were for something
    /// else: none of their answers count any more (those still running run on, see
    /// [`CobInstance::supersede_aims`]), and it aims anew.
    pub(crate) fn retarget(&mut self, slot: usize, target: Option<Target>) {
        let ws = &mut self.weapons[slot];
        if ws.target == target {
            return;
        }
        ws.target = target;
        ws.aimed = false;
        ws.aim_asked = false;
        if let Some(c) = &mut self.cob {
            c.supersede_aims(slot as u16);
        }
    }

    /// Its weapons' aims so far count no more, whatever they were at (their answers none of them:
    /// those still running run on, see [`CobInstance::supersede_aims`]): each aims anew before it
    /// fires. A unit its script holds still (`IMMOBILE`), told to go, packs up: an aim it answered
    /// unpacked would otherwise fire it once let go, packed and on its way.
    pub(crate) fn forget_aims(&mut self) {
        for slot in 0..self.weapons.len() {
            let ws = &mut self.weapons[slot];
            ws.aimed = false;
            ws.aim_asked = false;
            if let Some(c) = &mut self.cob {
                c.supersede_aims(slot as u16);
            }
        }
    }

    /// Sets the work done building it (of `total`, its type's [`UnitType::build_work`]) and
    /// the progress that is: ONE exactly when it is all done.
    pub fn set_build_work(&mut self, work: u64, total: u64) {
        self.build_work = work.min(total);
        self.build_progress = Fx::fraction(self.build_work, total);
    }

    /// Health as a percentage 0..=100 (COB `HEALTH`).
    pub fn health_percent(&self, t: &UnitType) -> i32 {
        let most = self.max_health(t);
        if most.is_zero() {
            return 100;
        }
        ((self.health * Fx::from_int(100)) / most).to_int().clamp(0, 100) as i32
    }

    /// Animation offset and rotation of a model piece (identity without a script).
    pub fn piece_anim(&self, t: &UnitType, model_piece: usize) -> ([i32; 3], [i32; 3], bool) {
        match (self.cob.as_ref(), t.model_to_cob.get(model_piece).copied().flatten()) {
            (Some(cob), Some(c)) => {
                let p = &cob.pieces[c];
                (p.offset, p.rotation, p.visible)
            }
            _ => ([0; 3], [0; 3], true),
        }
    }

    /// The model pieces whose animated textures play (bit n for piece n, the pieces past 63
    /// sharing bit 63): those its script has `dont-cache`d ([`crate::cob::PieceAnim::textures_play`]).
    /// Without a script, none: TA held them on their first frame.
    pub fn textures_playing(&self, t: &UnitType) -> u64 {
        let Some(cob) = self.cob.as_ref() else { return 0 };
        let mut playing = 0u64;
        for (piece, c) in t.model_to_cob.iter().enumerate() {
            if c.and_then(|c| cob.pieces.get(c)).is_some_and(|p| p.textures_play) {
                playing |= 1 << piece.min(63);
            }
        }
        playing
    }
}

/// Position of a model piece's origin in world space (deterministic fixed point). Piece
/// transform order matches the renderer: T(offset + move) · Ry · Rx · Rz, and the unit
/// transform is T(pos) · Ry(heading) · Rx(pitch) · Rz(roll).
pub fn piece_world_position(unit: &Unit, t: &UnitType, cob: Option<&CobInstance>, model_piece: usize) -> FxVec3 {
    let mut v = FxVec3::ZERO;
    let mut cur = Some(model_piece);
    let mut guard = 0;
    while let Some(i) = cur {
        let Some(piece) = t.model.pieces.get(i) else { break };
        let (off, rot) = match (cob, t.model_to_cob.get(i).copied().flatten()) {
            (Some(c), Some(ci)) => (c.pieces[ci].offset, c.pieces[ci].rotation),
            _ => ([0; 3], [0; 3]),
        };
        // Script z-axis turns and x-axis moves are negated in our frame (FORMATS.md); the
        // rest apply as they are.
        v = rotate_z(v, rot[2].wrapping_neg());
        v = rotate_x(v, rot[0]);
        v = rotate_y(v, rot[1]);
        v = v + piece.offset + FxVec3::new(Fx(-(off[0] as i64)), Fx(off[1] as i64), Fx(off[2] as i64));
        cur = piece.parent;
        guard += 1;
        if guard > 256 {
            break;
        }
    }
    v = rotate_z(v, unit.roll);
    v = rotate_x(v, unit.pitch);
    v = rotate_y(v, unit.heading);
    unit.model_pos(t) + v
}

/// The way a model piece points (its +z), turned as the piece and every piece it hangs from are
/// turned by the unit's script, and by the hull's tilt on the ground: [`piece_world_position`]'s
/// turns, without the offsets. (A hidden muzzle points as it is turned, drawn or not.)
pub fn piece_world_direction(unit: &Unit, t: &UnitType, model_piece: usize) -> FxVec3 {
    let mut direction = FxVec3::new(Fx::ZERO, Fx::ZERO, Fx::ONE);
    let mut current = Some(model_piece);
    for _ in 0..256 {
        let Some(index) = current else { break };
        let Some(piece) = t.model.pieces.get(index) else { break };
        if let Some(animation) = unit.cob.as_ref().and_then(|script| t.model_to_cob.get(index).copied().flatten().and_then(|index| script.pieces.get(index))) {
            direction = rotate_y(rotate_x(rotate_z(direction, animation.rotation[2].wrapping_neg()), animation.rotation[0]), animation.rotation[1]);
        }
        current = piece.parent;
    }
    rotate_y(rotate_x(rotate_z(direction, unit.roll), unit.pitch), unit.heading)
}

/// The heading of what stands on a model piece: the unit's own, turned as the piece and every
/// piece it hangs from are turned about y by the unit's script (a factory's build pad spinning as
/// it builds: TA's plants `spin pad around y-axis` from `StartBuilding` to `StopBuilding`). Turns
/// about x and z, which would tilt it, are left out: no factory of TA's tilts its pad.
pub fn piece_world_heading(unit: &Unit, t: &UnitType, cob: Option<&CobInstance>, model_piece: usize) -> i32 {
    let mut heading = unit.heading;
    let mut cur = Some(model_piece);
    let mut guard = 0;
    while let Some(i) = cur {
        let Some(piece) = t.model.pieces.get(i) else { break };
        if let (Some(c), Some(ci)) = (cob, t.model_to_cob.get(i).copied().flatten()) {
            // (Turns about y apply as they are, as `piece_world_position` turns them.)
            heading = wrap_angle(heading.wrapping_add(c.pieces[ci].rotation[1]));
        }
        cur = piece.parent;
        guard += 1;
        if guard > 256 {
            break;
        }
    }
    heading
}

/// Starting conditions for a player.
#[derive(Clone, Debug)]
pub struct PlayerSetup {
    pub side: String,
    /// Team colour index 0..9.
    pub color: u8,
    pub start: FxVec3,
    pub metal: Fx,
    pub energy: Fx,
    pub ai: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Economy {
    pub metal: Fx,
    pub energy: Fx,
    pub metal_capacity: Fx,
    pub energy_capacity: Fx,
    /// Per-second rates measured over the last tick.
    pub metal_income: Fx,
    pub energy_income: Fx,
    pub metal_expense: Fx,
    pub energy_expense: Fx,
    /// Resources requested by builders last tick, used to share them fairly this tick.
    pub(crate) metal_demand: Fx,
    pub(crate) energy_demand: Fx,
    pub(crate) metal_demand_next: Fx,
    pub(crate) energy_demand_next: Fx,
    /// Fraction (0..=1) of requested build resources granted this tick.
    pub build_factor: Fx,
    /// What was asked for over the last tick, per second, paid or not: builders and stockpiles
    /// at their full pace, upkeep, cloaking and shots. With a store empty only what comes in is
    /// paid, so the expense equals the income; the interface shows this as the use instead, by
    /// how much the asking outruns the income. Nothing in the battle reads it.
    pub metal_wanted: Fx,
    pub energy_wanted: Fx,
    pub(crate) tick_metal_in: Fx,
    pub(crate) tick_energy_in: Fx,
    pub(crate) tick_metal_out: Fx,
    pub(crate) tick_energy_out: Fx,
    pub(crate) tick_metal_wanted: Fx,
    pub(crate) tick_energy_wanted: Fx,
}

impl Economy {
    /// Pay for the affordable part of a construction tick. Prices are differences of
    /// cumulative shares, so fractional work never rounds into an unpaid discount.
    /// The caller supplies the stall factor for the resources its job consumes.
    pub(crate) fn spend_work(&mut self, before: u64, total: u64, full: u64, metal: Fx, energy: Fx, factor: Fx) -> u64 {
        let worth = |amount: Fx, work: u64| amount.share(before + work, total) - amount.share(before, total);
        self.metal_demand_next += worth(metal, full);
        self.energy_demand_next += worth(energy, full);
        self.tick_metal_wanted += worth(metal, full);
        self.tick_energy_wanted += worth(energy, full);
        let factor = factor.clamp(Fx::ZERO, Fx::ONE);
        let mut work = if factor > Fx::ZERO && full > 0 {
            ((full as u128 * factor.0 as u128) >> crate::fixed::FRAC_BITS).max(1) as u64
        } else { 0 };
        if metal > Fx::ZERO && self.metal <= Fx::ZERO || energy > Fx::ZERO && self.energy <= Fx::ZERO {
            work = 0;
        } else if worth(metal, work) > self.metal || worth(energy, work) > self.energy {
            let (mut lo, mut hi) = (0, work);
            while lo < hi {
                let mid = lo + (hi - lo).div_ceil(2);
                if worth(metal, mid) <= self.metal && worth(energy, mid) <= self.energy { lo = mid; }
                else { hi = mid - 1; }
            }
            work = lo;
        }
        let (paid_metal, paid_energy) = (worth(metal, work), worth(energy, work));
        self.metal -= paid_metal;
        self.energy -= paid_energy;
        self.tick_metal_out += paid_metal;
        self.tick_energy_out += paid_energy;
        work
    }
}

#[derive(Clone, Debug)]
pub struct Player {
    pub index: u8,
    pub side: String,
    pub color: u8,
    pub ai: bool,
    /// Where it started (a ceasefire keeps its enemies from building near it).
    pub start: FxVec3,
    pub economy: Economy,
    pub defeated: bool,
    /// Whether this player ever had a commander (the commander rule only applies then).
    pub had_commander: bool,
    /// Units owned, finished or not (the unit limit counts them all).
    pub unit_count: u32,
    /// Storage every player has without buildings (metal, energy).
    pub base_storage: (Fx, Fx),
    /// Tick of the last under-attack alert.
    pub last_alert: Option<u64>,
    /// Running totals for the end-of-game statistics.
    pub stats: PlayerStats,
    /// Alliance: players on one team share vision, never target each other, and win or
    /// lose together. Defaults to the player's own index (everyone for themselves).
    pub team: u8,
    /// Standing cheats, and whether the player used any.
    pub cheats: crate::cheats::Cheats,
    /// A scenario's attackers, who come in waves: not defeated for having no units.
    pub endless: bool,
    /// What the player passes to its allies on its own (TA's +sharemetal and kin).
    pub sharing: Sharing,
    /// Its Conquest Commander's perks (none outside Conquest).
    pub perks: crate::perks::Perks,
    /// TA's `+shootall`: its units' weapons take on every enemy in reach alike, their bad targets
    /// included (see `Sim::find_target`), nearest first.
    pub shoot_all: bool,
    /// A targeting facility of its own at work (TA's `IsTargetingUpgrade`: Core Contingency's
    /// Targeting Facilities, "Automatic Radar Targeting"): its units fire at will on what its side
    /// has on radar as on what it sees, as a spotter's guns do ([`Unit::radar_guided`]). Its own
    /// units, not its allies'. Set at each tick's economy end for the next: a facility finished,
    /// switched on, its upkeep paid (1,250 energy a second for the Arm's) and not paralysed, as a
    /// radar's coverage counts (`Sim::working`).
    pub radar_targeting: bool,
}

/// What a player passes to its allies on its own, each tick (TA's `+sharemetal`,
/// `+shareenergy`, `+setsharemetal N` and `+setshareenergy N`): metal and energy above what it
/// keeps, by default what its full storage would waste. Allies with room take it in player
/// order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sharing {
    pub metal: bool,
    pub energy: bool,
    /// How much to keep before sharing (`None`: all the storage holds; never more than it).
    pub keep_metal: Option<Fx>,
    pub keep_energy: Option<Fx>,
}

impl Default for Sharing {
    fn default() -> Self {
        Sharing { metal: true, energy: true, keep_metal: None, keep_energy: None }
    }
}

/// A player's running totals, plus a sample every [`PlayerStats::SAMPLE_TICKS`].
#[derive(Clone, Debug, Default)]
pub struct PlayerStats {
    /// Units finished by construction (not placed at the start).
    pub built: u64,
    pub lost: u64,
    pub kills: u64,
    pub metal_produced: Fx,
    pub energy_produced: Fx,
    pub metal_spent: Fx,
    pub energy_spent: Fx,
    pub history: Vec<StatSample>,
}

impl PlayerStats {
    /// Thirty seconds between history samples.
    pub const SAMPLE_TICKS: u64 = 30 * 30;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StatSample {
    pub tick: u64,
    pub metal_income: Fx,
    pub energy_income: Fx,
    /// Metal plus energy/60 cost of armed mobile units.
    pub army_value: Fx,
    pub units: u32,
}

/// Skirmish and multiplayer options every peer must agree on.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GameRules {
    /// Starting metal and energy (storage grows to hold them).
    pub start_metal: u64,
    pub start_energy: u64,
    /// Fog of war; off shows every unit to everyone.
    pub fog_of_war: bool,
    /// Terrain known from the start (TA's "mapped"); otherwise a shroud until explored.
    pub mapped: bool,
    /// Losing your commander loses the game.
    pub commander_ends_game: bool,
    /// Most units one player may own.
    pub unit_limit: u32,
    /// Players start at start positions drawn at random (from the game's seed) rather than
    /// in slot order.
    pub random_starts: bool,
    /// "Modern movement": moving ground units bend round those in their way (`Sim::
    /// local_avoidance`), and a group sent somewhere goes as a formation, formed up and marching
    /// together (`Sim::group_formations`); off moves them as TA did.
    pub give_way: bool,
    /// TA's "true" line of sight: hills hide what is behind them (`Sim::set_true_sight`);
    /// off, sight is a circle round each unit (TA's "circular").
    #[serde(default)]
    pub true_sight: bool,
    /// The mutators (see mutators.rs): as TA plays by default.
    #[serde(default)]
    pub mutators: crate::Mutators,
}

impl Default for GameRules {
    fn default() -> GameRules {
        GameRules {
            start_metal: 1000,
            start_energy: 1000,
            fog_of_war: true,
            mapped: true,
            commander_ends_game: true,
            unit_limit: 1500,
            random_starts: false,
            give_way: true,
            true_sight: false,
            mutators: crate::Mutators::default(),
        }
    }
}
