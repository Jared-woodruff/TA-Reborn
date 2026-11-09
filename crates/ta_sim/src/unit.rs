//! Units, orders and players.

use std::collections::VecDeque;

use crate::cob::CobInstance;
use crate::defs::UnitType;
use crate::fixed::{Fx, FxVec3, rotate_x, rotate_y, rotate_z};
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
}

/// Progress of a builder's current job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildJob {
    pub target: UnitId,
    /// `StartBuilding` has been called.
    pub started: bool,
}

#[derive(Clone)]
pub struct Unit {
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
    pub activated: bool,
    /// The last economy tick charged its upkeep (or it has none): its conditional output (an
    /// extractor's, a metal maker's) came too. Without the energy, neither (`Sim::economy_end`).
    pub upkeep_paid: bool,
    pub in_build_stance: bool,
    pub yard_open: bool,
    pub armored: bool,
    pub busy: bool,
    pub fire_state: u8,
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
    /// Aircraft: flying an attack pass (no braking at the pass goal).
    pub air_pass: bool,
    /// Aircraft: ticks without anything to do (landing starts after a short delay).
    pub idle_ticks: u32,
    /// Aircraft: ticks spent on the current attack pass.
    pub pass_ticks: u32,
    /// Aircraft: past the target, flying the straight run-out before turning back.
    pub pass_runout: bool,
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
    /// Factory production queue (unit type ids), and the orders every unit it releases
    /// is given (its rally route: move, patrol, fight and guard orders).
    pub factory_queue: VecDeque<usize>,
    pub rally: Vec<Order>,
    /// Factory that is building / releasing this unit.
    pub factory: Option<UnitId>,
    /// Factories: the units they built that may still be leaving their yard (those whose
    /// `factory` is this one; the others are dropped as they are found).
    pub yard: Vec<UnitId>,
    /// Footprint origin cell for buildings that occupy the grid.
    pub occupied_origin: Option<(i32, i32)>,
    pub cob: Option<CobInstance>,
    pub weapons: [WeaponState; 3],
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
            activated: false,
            upkeep_paid: true,
            in_build_stance: false,
            yard_open: false,
            armored: false,
            busy: false,
            fire_state: 2,
            move_state: 1,
            orders: VecDeque::new(),
            path: Vec::new(),
            path_goal: None,
            move_request: None,
            path_partial: false,
            unreachable: Vec::new(),
            landed: false,
            air_pad: None,
            air_pass: false,
            idle_ticks: 0,
            pass_ticks: 0,
            pass_runout: false,
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
            factory_queue: VecDeque::new(),
            rally: Vec::new(),
            factory: None,
            yard: Vec::new(),
            occupied_origin: None,
            cob: None,
            weapons: [WeaponState::default(); 3],
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

    /// Sets the work done building it (of `total`, its type's [`UnitType::build_work`]) and
    /// the progress that is: ONE exactly when it is all done.
    pub fn set_build_work(&mut self, work: u64, total: u64) {
        self.build_work = work.min(total);
        self.build_progress = Fx::fraction(self.build_work, total);
    }

    /// Health as a percentage 0..=100 (COB `HEALTH`).
    pub fn health_percent(&self, t: &UnitType) -> i32 {
        if t.max_health.is_zero() {
            return 100;
        }
        ((self.health * Fx::from_int(100)) / t.max_health).to_int().clamp(0, 100) as i32
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
    pub(crate) tick_metal_in: Fx,
    pub(crate) tick_energy_in: Fx,
    pub(crate) tick_metal_out: Fx,
    pub(crate) tick_energy_out: Fx,
}

impl Economy {
    /// Pay for the affordable part of a construction tick. Prices are differences of
    /// cumulative shares, so fractional work never rounds into an unpaid discount.
    /// The caller supplies the stall factor for the resources its job consumes.
    pub(crate) fn spend_work(&mut self, before: u64, total: u64, full: u64, metal: Fx, energy: Fx, factor: Fx) -> u64 {
        let worth = |amount: Fx, work: u64| amount.share(before + work, total) - amount.share(before, total);
        self.metal_demand_next += worth(metal, full);
        self.energy_demand_next += worth(energy, full);
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
