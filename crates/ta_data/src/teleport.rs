//! Teleporting (`CanTeleport`, `TeleportEnergy`, `TeleportTime`, in a unit's `[UNITINFO]` or in one
//! of its enhancements'): TA Reborn's own. Told to teleport to a place, the unit stands and pays its
//! charge, as construction pays (slowed by the economy as a build is), then lands on the nearest
//! spot to it that it can stand on, anywhere on the map. Supreme Commander's commanders' Personal
//! Teleporter is one.
//!
//! ```text
//! [UNITINFO]
//! {
//!     UnitName=MYCOMMANDER;
//!     CanTeleport=1;
//!     TeleportEnergy=20000 4;   (energy: 20,000, and 4 more for every pixel of the way)
//!     TeleportTime=10;          (seconds: 10, and as many more for every pixel as a second number says)
//!     ...
//! }
//! ```

use ta_formats::tdf::TdfSection;

/// A unit's (or an enhancement's) teleport: its charge, in energy and in seconds, each a base and
/// so much more for every pixel of the way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TeleportDef {
    pub energy: f64,
    pub energy_per_px: f64,
    pub time: f64,
    pub time_per_px: f64,
}

/// The time a charge takes where `TeleportTime` gives none (seconds).
pub const DEFAULT_TELEPORT_TIME: f64 = 1.0;

/// A key's one or two numbers (spaces or commas between), none below 0: (the first, the second; 0
/// where it gives none).
fn pair(s: &TdfSection, key: &str) -> Option<(f64, f64)> {
    let text = s.get(key)?;
    let mut numbers = text.split([' ', ',', '\t']).filter(|w| !w.trim().is_empty()).map(|w| w.trim().parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0));
    let first = numbers.next().unwrap_or(0.0);
    Some((first, numbers.next().unwrap_or(0.0)))
}

impl TeleportDef {
    /// The teleport a section gives (`CanTeleport=1`), with its charge; none without it.
    pub fn of(s: &TdfSection) -> Option<TeleportDef> {
        if !s.get_bool_or("CanTeleport", false) {
            return None;
        }
        let (energy, energy_per_px) = pair(s, "TeleportEnergy").unwrap_or((0.0, 0.0));
        let (time, time_per_px) = pair(s, "TeleportTime").unwrap_or((DEFAULT_TELEPORT_TIME, 0.0));
        Some(TeleportDef { energy, energy_per_px, time, time_per_px })
    }

    /// Whether a section gives a teleport's charge without `CanTeleport=1` (it cannot teleport: the
    /// mod check tells it).
    pub fn stray(s: &TdfSection) -> bool {
        !s.get_bool_or("CanTeleport", false) && (s.get("TeleportEnergy").is_some() || s.get("TeleportTime").is_some())
    }

    /// Its charge for a way of `distance` pixels: energy and seconds.
    pub fn charge(&self, distance: f64) -> (f64, f64) {
        let d = distance.max(0.0);
        (self.energy + self.energy_per_px * d, self.time + self.time_per_px * d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::tdf;

    #[test]
    fn a_teleport_is_read_with_its_charge() {
        let doc = tdf::parse("[UNITINFO]{UnitName=ACU; CanTeleport=1; TeleportEnergy=20000 4; TeleportTime= 10 ,0.01;}");
        let t = TeleportDef::of(doc.section("UNITINFO").unwrap()).unwrap();
        assert_eq!(t, TeleportDef { energy: 20000.0, energy_per_px: 4.0, time: 10.0, time_per_px: 0.01 });
        assert_eq!(t.charge(1000.0), (24000.0, 20.0));
        let plain = TeleportDef::of(tdf::parse("[UNITINFO]{UnitName=X; CanTeleport=1;}").section("UNITINFO").unwrap()).unwrap();
        assert_eq!((plain.energy, plain.time), (0.0, DEFAULT_TELEPORT_TIME), "free, and a second, where none is given");
        let stray = tdf::parse("[UNITINFO]{UnitName=Y; TeleportEnergy=5;}");
        assert!(TeleportDef::of(stray.section("UNITINFO").unwrap()).is_none() && TeleportDef::stray(stray.section("UNITINFO").unwrap()));
        let negative = TeleportDef::of(tdf::parse("[UNITINFO]{UnitName=Z; CanTeleport=1; TeleportEnergy=-5 x;}").section("UNITINFO").unwrap()).unwrap();
        assert_eq!((negative.energy, negative.energy_per_px), (0.0, 0.0), "none below 0");
    }
}
