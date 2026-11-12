//! Deterministic Total Annihilation simulation.
//!
//! The simulation runs at TA's fixed 30 ticks per second using only integer/fixed-point
//! arithmetic ([`fixed`]), so identical inputs give bit-identical results on every machine
//! — the basis for lockstep multiplayer and replays. Units are driven by their original COB
//! scripts ([`cob`]) exactly as in the 1997 game.

pub mod avoid;
pub mod cheats;
pub mod cob;
pub mod combat;
pub mod crowd;
pub mod defs;
pub mod events;
pub mod features;
pub mod formation;
pub mod fixed;
pub mod flows;
pub mod host;
pub mod mission;
pub mod mutators;
pub mod path;
pub mod perks;
pub mod rng;
pub mod serde_limits;
pub mod signal;
pub mod sim;
pub mod unit;
pub mod vision;
pub mod world;

pub use cheats::{Cheat, Cheats};
pub use combat::Projectile;
pub use defs::{Locomotion, Trajectory, UnitType, UnitTypes, WeaponType, WeaponTypes};
pub use events::SimEvent;
pub use fixed::{Fx, FxVec3};
pub use mutators::Mutators;
pub use perks::Perks;
pub use signal::{Signal, SignalKind};
pub use sim::{Command, Feature, Sim, SimStats};
pub use unit::{Drift, GameRules, Order, Player, PlayerSetup, PlayerStats, Sharing, StatSample, Target, Unit, UnitId, WeaponState, piece_world_position};
pub use vision::{Seen, Vision};
