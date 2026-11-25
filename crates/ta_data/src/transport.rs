//! An air transport's slots (`TransportSlots` in its `[UNITINFO]`): TA Reborn's own. TA's air
//! transports carry one unit, whatever their files say (its Atlas says 5); one that names its slots
//! carries a unit in each, hanging from the model piece the slot names, drawn there. A slot may
//! give a class after a colon (1, small, where none is given); a passenger (its `TransportClass`,
//! 1 where none is given) takes a free slot of its own class, else one of the smallest larger class
//! free. A larger slot overlaps the smaller ones nearest it: taken, they are not, and taken, it is
//! not (Supreme Commander's transports carry six small units, two medium or one large so).
//!
//! ```text
//! [UNITINFO]
//! {
//!     UnitName=MYLIFTER;
//!     CanFly=1;
//!     TransportSize=3;
//!     TransportSlots=LeftFront LeftBack RightFront RightBack LeftMiddle:2 RightMiddle:2 Middle:3;
//!     ...
//! }
//! ```

use ta_formats::tdf::TdfSection;

/// The most slots a transport's file may give (the rest are left out, with a warning).
pub const MAX_TRANSPORT_SLOTS: usize = 64;

/// The largest slot class (and passenger class) read.
pub const MAX_TRANSPORT_CLASS: u32 = 255;

/// One of an air transport's slots: the piece a passenger hangs from, and its class.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransportSlotDef {
    pub piece: String,
    /// 1 small, 2 medium, 3 large (any number from 1): what passengers it takes (its own class and
    /// smaller), and which slots it overlaps.
    pub class: u32,
}

impl TransportSlotDef {
    /// A unit's slots: its `TransportSlots`, pieces apart by spaces or commas, each with its class
    /// after a colon (`Middle:3`); none given, none.
    pub fn of_unit(s: &TdfSection) -> Vec<TransportSlotDef> {
        let Some(list) = s.get("TransportSlots") else { return Vec::new() };
        let mut out: Vec<TransportSlotDef> = Vec::new();
        for word in list.split([' ', ',', '\t']).map(str::trim).filter(|w| !w.is_empty()) {
            let (piece, class) = match word.split_once(':') {
                Some((piece, class)) => (piece.trim(), class.trim().parse::<i64>().unwrap_or(1).clamp(1, MAX_TRANSPORT_CLASS as i64) as u32),
                None => (word, 1),
            };
            if piece.is_empty() {
                continue;
            }
            if out.len() >= MAX_TRANSPORT_SLOTS {
                log::warn!("{}: more than {MAX_TRANSPORT_SLOTS} transport slots; the rest are left out", s.get("UnitName").unwrap_or("?"));
                break;
            }
            out.push(TransportSlotDef { piece: piece.to_string(), class });
        }
        out
    }
}

/// A unit's class as a passenger of a transport with slots (`TransportClass`, 1 at least; 1 where
/// none is given).
pub fn transport_class(s: &TdfSection) -> u32 {
    s.get_i64_or("TransportClass", 1).clamp(1, MAX_TRANSPORT_CLASS as i64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::tdf;

    #[test]
    fn slots_are_read_with_their_classes() {
        let doc = tdf::parse("[UNITINFO]{UnitName=LIFTER; TransportSlots= Left1 Left2,Right1 Mid:2  Big:3 Odd:x :2; TransportClass=0;}");
        let s = doc.section("UNITINFO").unwrap();
        let slots = TransportSlotDef::of_unit(s);
        let got: Vec<(&str, u32)> = slots.iter().map(|s| (s.piece.as_str(), s.class)).collect();
        assert_eq!(got, [("Left1", 1), ("Left2", 1), ("Right1", 1), ("Mid", 2), ("Big", 3), ("Odd", 1)], "a class that is no number is 1; no piece, no slot");
        assert_eq!(transport_class(s), 1, "1 at least");
        let none = tdf::parse("[UNITINFO]{UnitName=ARMATLAS; TransportCapacity=5;}");
        assert!(TransportSlotDef::of_unit(none.section("UNITINFO").unwrap()).is_empty(), "TA's Atlas names none");
        assert_eq!(transport_class(tdf::parse("[UNITINFO]{UnitName=T; TransportClass=3;}").section("UNITINFO").unwrap()), 3);
    }
}
