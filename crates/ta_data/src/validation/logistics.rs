//! The mod check of TA Reborn's logistics (docs/MODDING.md: air transports with slots,
//! teleporting, drones, factories that move): a part given where it does nothing, or without what
//! makes it work, is told; a drone's unit is a reference as a weapon is; the pieces its keys name
//! are looked for in the unit's model; and what the mod's own units use of them is told among how
//! TA Reborn plays the mod (`engine.logistics`).

use super::*;

/// What the mod's own units use of the logistics: air transports with slots, units that teleport
/// and units with drones (of their own or by an enhancement), factories that move.
#[derive(Default)]
pub(super) struct LogisticsUses {
    slotted: usize,
    teleports: usize,
    drones: usize,
    mobile: usize,
}

impl Scan<'_> {
    /// A unit's logistics (`s`: its `[UNITINFO]`): slots on a unit that does not fly, a teleport's
    /// charge without `CanTeleport`, a teleport or a factory that moves on a building, a factory that
    /// moves with no build power, a `[DRONES]` giving none; its drones' units (references).
    pub(super) fn unit_logistics(&mut self, path: &str, s: &TdfSection, unit: &crate::UnitDef, refs: &mut Vec<Reference>) {
        let name = || Fill::name(&unit.name);
        let enhancements: Vec<&TdfSection> = s.sections.iter().find(|e| e.name.eq_ignore_ascii_case("ENHANCEMENTS")).map(|list| list.sections.iter().collect()).unwrap_or_default();
        let teleports = unit.teleport.is_some() || unit.enhancements.iter().any(|e| e.teleport.is_some());
        let drones: Vec<&crate::DroneDef> = unit.drones.iter().chain(unit.enhancements.iter().flat_map(|e| e.drones.iter())).collect();
        if self.own(path) {
            let uses = &mut self.engine.logistics;
            uses.slotted += (!unit.transport_slots.is_empty() && unit.can_fly) as usize;
            uses.teleports += teleports as usize;
            uses.drones += (!drones.is_empty()) as usize;
            uses.mobile += (unit.mobile_factory && !unit.is_building()) as usize;
        }
        for d in &drones {
            refs.push(Reference::new("unit", &d.unit, enf("{name} drone", [("name", name())]), Severity::Error));
        }
        if !unit.transport_slots.is_empty() && !unit.can_fly {
            let words = enf("{unit} names transport slots (TransportSlots) but does not fly: TA Reborn plays slots for air transports alone.", [("unit", name())]);
            self.issue(Severity::Warning, "unit.transport_slots", path, words, en("Leave TransportSlots out, or give it to an aircraft (CanFly=1)."));
            if self.halted { return; }
        }
        let stray = crate::TeleportDef::stray(s) || enhancements.iter().any(|e| crate::TeleportDef::stray(e));
        if stray {
            let words = enf("{unit} gives a teleport's charge (TeleportEnergy, TeleportTime) without CanTeleport=1: it cannot teleport.", [("unit", name())]);
            self.issue(Severity::Warning, "unit.teleport", path, words, en("Give it CanTeleport=1 beside its charge, where it gives one (in its own section or an enhancement's)."));
            if self.halted { return; }
        }
        if teleports && unit.is_building() {
            let words = enf("{unit} can teleport but never moves: a building does not teleport.", [("unit", name())]);
            self.issue(Severity::Warning, "unit.teleport", path, words, en("Leave CanTeleport out, or give it to a unit that moves."));
            if self.halted { return; }
        }
        if crate::DroneDef::empty(s) || enhancements.iter().any(|e| crate::DroneDef::empty(e)) {
            let words = enf("{unit} has a [DRONES] section that gives no drone (no Unit, or a Count of none): it keeps none.", [("unit", name())]);
            self.issue(Severity::Warning, "unit.drones", path, words, en("Give each kind of drone its Unit, and a Count of one or more."));
            if self.halted { return; }
        }
        if unit.mobile_factory && unit.is_building() {
            let words = enf("{unit} is a factory that moves (MobileFactory=1) but never moves: it is a factory as any other.", [("unit", name())]);
            self.issue(Severity::Warning, "unit.mobile_factory", path, words, en("Leave MobileFactory out, or give it a speed (MaxVelocity) to move."));
        } else if unit.mobile_factory && unit.worker_time <= 0.0 {
            let words = enf("{unit} is a factory that moves (MobileFactory=1) but has no build power: it builds nothing.", [("unit", name())]);
            self.issue(Severity::Warning, "unit.mobile_factory", path, words, en("Give it a WorkerTime, and its units on its build menu."));
        }
    }

    /// The pieces a unit's logistics keys name (its transport slots, its build piece, its drones')
    /// that its model has not (`model_names`: its model's pieces, lower case).
    pub(super) fn logistics_pieces(&mut self, path: &str, unit: &UnitFacts, model: &str, model_names: &BTreeSet<String>) {
        let mut missing: Vec<&String> = unit.named_pieces.iter().filter(|p| !model_names.contains(&p.to_ascii_lowercase())).collect();
        missing.dedup();
        if missing.is_empty() {
            return;
        }
        let words = enf("{unit} names pieces its model {model} has not: {pieces}. What hangs or is built there is put at the model's middle.", [
            ("unit", Fill::name(&unit.name)),
            ("model", Fill::name(model)),
            ("pieces", Fill::List(missing.iter().map(|p| Fill::name(p.as_str())).collect())),
        ]);
        self.issue(Severity::Warning, "unit.logistics_piece", path, words, en("Name pieces of its model (TransportSlots, BuildPiece, a drone's Piece), or add them to the model."));
    }

    /// What the mod's own units use of the logistics, for how TA Reborn plays it.
    pub(super) fn logistics_uses(uses: &LogisticsUses) -> Option<(Words, &'static str)> {
        (uses.slotted + uses.teleports + uses.drones + uses.mobile > 0).then(|| {
            let words = enf("TA Reborn's logistics. Air transports with slots: {slotted}; units that teleport: {teleports}; units with drones: {drones}; factories that move: {mobile}.", [
                ("slotted", Fill::number(uses.slotted)),
                ("teleports", Fill::number(uses.teleports)),
                ("drones", Fill::number(uses.drones)),
                ("mobile", Fill::number(uses.mobile)),
            ]);
            (words, en("Played as docs/MODDING.md tells them (TransportSlots, TransportClass, CanTeleport, [DRONES], MobileFactory). TA's own engine never read these keys."))
        })
    }
}
