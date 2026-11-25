//! A unit's drones (`[DRONES]` within its `[UNITINFO]`, or within one of its enhancements'): TA
//! Reborn's own. Units of another type the unit keeps by it, made as it is finished (or the
//! enhancement installed), each guarding it within a leash, rebuilt at a price after a loss, and
//! dying or changing sides with it: Supreme Commander's Kennel's engineering drone and its UEF
//! commanders' pods. A section a kind of drone (any name), or the keys straight in `[DRONES]` for
//! one kind.
//!
//! ```text
//! [UNITINFO]
//! {
//!     UnitName=MYSTATION;
//!     ...
//!     [DRONES]
//!     {
//!         [PodOne]
//!         {
//!             Unit=MYDRONE;
//!             Count=2;
//!             RebuildTime=40;          (seconds: one lost is made again in that long)
//!             RebuildCostMetal=250;    (its price, paid as construction is paid)
//!             RebuildCostEnergy=2500;
//!             Piece=PodBay;            (where it is made, a piece of the unit's model)
//!             Leash=480;               (how far from the unit it may go, pixels)
//!         }
//!     }
//! }
//! ```

use ta_formats::tdf::TdfSection;

/// The most kinds of drones a unit (or an enhancement) may give, and the most of a kind.
pub const MAX_DRONE_KINDS: usize = 16;
pub const MAX_DRONES: u32 = 64;

/// One kind of drone, as its section gives it: what unit, how many, and how one lost is made again.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DroneDef {
    /// The drone's unit (`Unit`, upper case).
    pub unit: String,
    /// How many (`Count`; 1 where none is given).
    pub count: u32,
    /// Seconds a lost one takes to make again (`RebuildTime`; none given: the drone's `BuildTime`
    /// at the unit's build power, else thirty seconds).
    pub rebuild_time: Option<f64>,
    /// Its price (`RebuildCostMetal`, `RebuildCostEnergy`; none given: the drone's own).
    pub rebuild_metal: Option<f64>,
    pub rebuild_energy: Option<f64>,
    /// The piece of the unit's model it is made at (`Piece`; none: the unit's middle).
    pub piece: String,
    /// How far from the unit it may go (`Leash`, px; none given: [`DEFAULT_LEASH`]).
    pub leash: Option<f64>,
}

/// How far a drone may go from its unit where its section gives no `Leash` (px).
pub const DEFAULT_LEASH: f64 = 512.0;

impl DroneDef {
    fn from_tdf(s: &TdfSection) -> Option<DroneDef> {
        let unit = s.get("Unit").map(|u| u.trim().to_ascii_uppercase()).filter(|u| !u.is_empty())?;
        let positive = |k: &str| s.get_f64(k).filter(|v| v.is_finite() && *v >= 0.0);
        Some(DroneDef {
            unit,
            count: s.get_i64_or("Count", 1).clamp(0, MAX_DRONES as i64) as u32,
            rebuild_time: positive("RebuildTime"),
            rebuild_metal: positive("RebuildCostMetal"),
            rebuild_energy: positive("RebuildCostEnergy"),
            piece: s.get("Piece").unwrap_or("").trim().to_string(),
            leash: positive("Leash"),
        })
    }

    /// The drones a section (a unit's `[UNITINFO]`, or an enhancement's) gives: its `[DRONES]`
    /// section's kinds, each a section of its own, or the keys straight in it for one; none
    /// without.
    pub fn of(info: &TdfSection) -> Vec<DroneDef> {
        let Some(list) = info.sections.iter().find(|s| s.name.eq_ignore_ascii_case("DRONES")) else { return Vec::new() };
        let mut out: Vec<DroneDef> = DroneDef::from_tdf(list).into_iter().collect();
        out.extend(list.sections.iter().filter_map(DroneDef::from_tdf));
        if out.len() > MAX_DRONE_KINDS {
            log::warn!("{}: more than {MAX_DRONE_KINDS} kinds of drones; the rest are left out", info.get("UnitName").unwrap_or("?"));
            out.truncate(MAX_DRONE_KINDS);
        }
        out.retain(|d| d.count > 0);
        out
    }

    /// Whether a section has a `[DRONES]` section that gives no drone (the mod check tells it).
    pub fn empty(info: &TdfSection) -> bool {
        info.sections.iter().any(|s| s.name.eq_ignore_ascii_case("DRONES")) && DroneDef::of(info).is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::tdf;

    #[test]
    fn drones_are_read_kind_by_kind() {
        let doc = tdf::parse("[UNITINFO]{UnitName=STATION; [DRONES]{ [PodOne]{ Unit= pod7 ; Count=2; RebuildTime=12.5; RebuildCostMetal=80; Piece=Dock01; Leash=300; } [Bad]{ Count=3; } [None]{ Unit=X; Count=0; } } }");
        let d = DroneDef::of(doc.section("UNITINFO").unwrap());
        assert_eq!(d.len(), 1, "a kind with no unit, or none of it, left out");
        assert_eq!((d[0].unit.as_str(), d[0].count, d[0].rebuild_time, d[0].rebuild_metal, d[0].rebuild_energy), ("POD7", 2, Some(12.5), Some(80.0), None));
        assert_eq!((d[0].piece.as_str(), d[0].leash), ("Dock01", Some(300.0)));
        // One kind straight in the section, one by default.
        let flat = tdf::parse("[UNITINFO]{UnitName=ACU; [DRONES]{ Unit=POD; } }");
        let d = DroneDef::of(flat.section("UNITINFO").unwrap());
        assert_eq!((d.len(), d[0].count, d[0].leash), (1, 1, None));
        assert!(DroneDef::of(tdf::parse("[UNITINFO]{UnitName=ARMCOM;}").section("UNITINFO").unwrap()).is_empty());
        assert!(!DroneDef::empty(doc.section("UNITINFO").unwrap()) && DroneDef::empty(tdf::parse("[UNITINFO]{UnitName=E; [DRONES]{ Count=2; } }").section("UNITINFO").unwrap()));
    }
}
