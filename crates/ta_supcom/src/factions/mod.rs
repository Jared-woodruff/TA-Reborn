//! Forged Alliance's factions made into a mod of TA units (docs/SUPCOM_UNITS.md): each unit a
//! player can make becomes a TA unit, its mesh a TA model. Phase by phase:
//!
//! - [`model`]: a unit's mesh as a TA model (its bones its pieces) and its surface;
//! - [`textures`]: its paint as the surface's textures;
//! - [`pictures`]: its build picture;
//! - [`script`]: its script;
//! - [`units`]: its unit file and its weapons';
//! - [`enhance`]: a commander's enhancements;
//! - [`abilities`]: TA Reborn's abilities (minimum ranges, stuns, jamming and stealth fields,
//!   remote viewing, weapon toggles) in its files;
//! - [`faction`]: its faction's side, build menus, wrecks and weapons;
//! - [`emblem`]: its faction's emblem in the classic interface;
//! - [`anims`], [`work`]: its walk and deaths, and what it does as it works;
//! - [`make`]: the mod made whole.
//!
//! What is written must be the same, byte for byte, on every computer (a mod of units is part
//! of the data players compare before a multiplayer battle): plain arithmetic only, in a fixed
//! order.

pub mod abilities;
pub mod anims;
pub mod effects;
pub mod emblem;
pub mod enhance;
pub mod faction;
pub mod make;
pub mod model;
pub mod pictures;
pub mod script;
pub mod sounds;
pub mod textures;
pub mod units;
pub mod work;

/// TA pixels per ogrid for units: twice the maps' 16 (a T1 tank about a TA tank's size, as the
/// player chose: docs/SUPCOM_UNITS.md). Sizes, footprints, speeds and ranges are all taken so.
pub const UNIT_SCALE: f64 = 32.0;
