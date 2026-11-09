//! Events emitted by the simulation for presentation (effects, sounds, UI). They carry no
//! authority: the client may drop or reorder them without affecting the game state.

use crate::fixed::{Fx, FxVec3};
use crate::unit::UnitId;

#[derive(Clone, Debug)]
pub enum SimEvent {
    UnitCreated { id: UnitId, type_id: usize },
    /// Construction finished.
    UnitCompleted { id: UnitId, type_id: usize },
    /// (`killer`: the unit whose shot or blast finished it, if any; `removed`: taken off the map
    /// rather than killed: reclaimed, its construction given up, a scenario's set-up; `complete`:
    /// finished, not a frame still being built. A wreck it left came just before, as a
    /// `FeatureCreated` naming it.)
    UnitDestroyed { id: UnitId, type_id: usize, owner: u8, pos: FxVec3, killer: Option<UnitId>, removed: bool, complete: bool },
    /// COB `emit-sfx` (smoke, wakes, thruster effects).
    Sfx { unit: UnitId, pos: FxVec3, kind: i32 },
    /// COB `explode` on a piece: debris to throw. `flags` are TA's explode types
    /// (SHATTER 1, EXPLODE_ON_HIT 2, FALL 4, SMOKE 8, FIRE 16, BITMAPONLY 32, BITMAP1-5
    /// 256..4096).
    PieceExplode { unit: UnitId, type_id: usize, model_piece: usize, pos: FxVec3, heading: i32, flags: i32 },
    /// A builder started spraying nano particles at a target.
    NanoSpray { builder: UnitId, target: UnitId },
    /// An order could not be carried out (e.g. build site blocked).
    OrderFailed { unit: UnitId, reason: &'static str },
    WeaponFired { unit: UnitId, weapon: usize, pos: FxVec3, target: FxVec3 },
    /// A projectile or death explosion went off; `when`: how far through the tick (0 to 1) a
    /// projectile met what it hit (0 for the others), so the client can set it off as the
    /// projectile it draws gets there; `player`: whose weapon or unit it was (none for a staged
    /// blast).
    Explosion { weapon: usize, pos: FxVec3, water: bool, when: Fx, player: Option<u8> },
    /// A unit died without an `ExplodeAs` weapon (generic blast).
    UnitExploded { id: UnitId, pos: FxVec3, big: bool },
    /// (`wreck_of`: the unit whose death left it, its wreck or heap, when a death did.)
    FeatureCreated { index: usize, wreck_of: Option<UnitId> },
    FeatureDestroyed { index: usize, pos: FxVec3 },
    /// A tree (or other flammable feature) caught fire.
    FeatureIgnited { index: usize, pos: FxVec3 },
    /// A builder started reclaiming a feature.
    Reclaiming { builder: UnitId, feature: usize },
    /// A transport took a unit aboard, or set it down.
    UnitLoaded { transport: UnitId, unit: UnitId },
    UnitUnloaded { transport: UnitId, unit: UnitId },
    /// An anti-missile interceptor destroyed a missile in flight.
    Intercepted { pos: FxVec3, defender: u8, attacker: u8 },
    /// A unit changed hands (captured).
    UnitCaptured { id: UnitId, from: u8, to: u8 },
    /// A player gave `count` units to an ally.
    UnitsGiven { from: u8, to: u8, count: u32 },
    /// A player gave an ally metal and energy (`Command::Share`, not what full stores pass on).
    Shared { from: u8, to: u8, metal: Fx, energy: Fx },
    /// A unit's cloak took effect or dropped.
    CloakChanged { id: UnitId, cloaked: bool },
    /// Enemy fire hit one of `player`'s units (at most once every ten seconds per player).
    UnderAttack { player: u8, unit: UnitId, pos: FxVec3 },
    /// The mission's objectives were decided.
    MissionEnded { victory: bool },
    /// A player lost (commander destroyed or no units left).
    PlayerDefeated { player: u8 },
    /// A scenario's wave of `units` units arrived for `owner` at `pos`.
    WaveArrived { owner: u8, units: u32, pos: FxVec3 },
    /// A player signalled their allies (see [`crate::signal`]).
    Signal { player: u8, kind: crate::signal::SignalKind, at: FxVec3 },
}
