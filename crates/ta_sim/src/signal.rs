//! Signals: a player marks a spot for their allies and says what they mean by it, a ping with
//! a meaning ([`SignalKind`]). The simulation keeps the last few, the same on every machine and
//! in every replay, so computer allies can act on them.

use serde::{Deserialize, Serialize};

use crate::fixed::FxVec3;

/// What a signal means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignalKind {
    /// Look here (a plain ping).
    Look,
    /// Attack here.
    Attack,
    /// Defend here.
    Defend,
    /// On my way (to the spot).
    OnMyWay,
    /// I need metal, or energy: allies with some to spare send it.
    NeedMetal,
    NeedEnergy,
}

impl SignalKind {
    pub const ALL: [SignalKind; 6] = [SignalKind::Look, SignalKind::Attack, SignalKind::Defend, SignalKind::OnMyWay, SignalKind::NeedMetal, SignalKind::NeedEnergy];
}

/// A signal given, as the simulation keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signal {
    pub player: u8,
    pub kind: SignalKind,
    pub at: FxVec3,
    /// The tick it was given on.
    pub tick: u64,
}

/// How many signals the simulation keeps, and for how long (ticks: a minute and a half).
pub const KEPT: usize = 32;
pub const LIFE: u64 = 90 * 30;
