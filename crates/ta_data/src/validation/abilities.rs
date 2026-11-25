//! The mod check of TA Reborn's abilities (docs/MODDING.md, "Abilities"): a part of one given
//! without what makes it work does nothing, and is told; a weapon toggle's weapons are references
//! as the unit's own are; and what the mod's own units and weapons use of them is told among how
//! TA Reborn plays the mod (`engine.abilities`).

use super::*;

/// What the mod's own files use of the abilities: weapons with a minimum range or a stun, units
/// with a toggle, a stealth field, remote viewing or an aura (their own, or an enhancement's).
#[derive(Default)]
pub(super) struct AbilityUses {
    min_ranges: usize,
    stuns: usize,
    toggles: usize,
    fields: usize,
    views: usize,
    auras: usize,
}

impl Scan<'_> {
    /// A unit's abilities: a part without what makes it work, and a toggle's weapons (references).
    pub(super) fn unit_abilities(&mut self, path: &str, unit: &crate::UnitDef, refs: &mut Vec<Reference>) {
        let name = || Fill::name(&unit.name);
        if self.own(path) {
            // (Of its own, or by an enhancement it can install.)
            let field = unit.radar_stealth_field > 0.0 || unit.sonar_stealth_field > 0.0
                || unit.enhancements.iter().any(|e| e.radar_stealth_field > 0.0 || e.sonar_stealth_field > 0.0);
            let view = unit.remote_view_radius > 0.0 || unit.enhancements.iter().any(|e| e.remote_view_radius > 0.0);
            let aura = unit.regen_aura_radius > 0.0 || unit.enhancements.iter().any(|e| e.regen_aura_radius > 0.0);
            let uses = &mut self.engine.abilities;
            uses.toggles += unit.toggle.is_some() as usize;
            uses.fields += field as usize;
            uses.views += view as usize;
            uses.auras += aura as usize;
        }
        if let Some(toggle) = &unit.toggle {
            for (slot, w) in toggle.named() {
                refs.push(Reference::new("weapon", w, enf("{name} toggle {key}", [("name", name()), ("key", Fill::name(format!("Weapon{}", slot + 1)))]), Severity::Error));
            }
        }
        let aura_half = (unit.regen_aura_radius > 0.0) != (unit.regen_aura_rate > 0.0 || unit.regen_aura_share > 0.0);
        if aura_half {
            let words = enf("{unit} has half a regeneration aura (RegenAuraRadius without RegenAuraRate or RegenAuraShare, or one of those without the radius): it mends nothing.", [("unit", name())]);
            self.issue(Severity::Warning, "unit.regen_aura", path, words, en("Give it RegenAuraRadius (px) and RegenAuraRate (hit points a second) or RegenAuraShare (a share of each unit's most health a second)."));
            if self.halted { return; }
        }
        if unit.remote_view_energy > 0.0 && unit.remote_view_radius <= 0.0 {
            let words = enf("{unit} gives RemoteViewEnergy without RemoteViewRadius: it cannot view from afar.", [("unit", name())]);
            self.issue(Severity::Warning, "unit.remote_view", path, words, en("Give it RemoteViewRadius, how far round the place it views it sees (px)."));
        }
    }

    /// A weapon's abilities: a minimum range it can never fire past, and a stun's parts without its
    /// time.
    pub(super) fn weapon_abilities(&mut self, path: &str, weapon: &crate::WeaponDef) {
        let name = || Fill::name(&weapon.name);
        if self.own(path) {
            self.engine.abilities.min_ranges += (weapon.min_range > 0.0) as usize;
            self.engine.abilities.stuns += weapon.stuns() as usize;
        }
        if weapon.min_range > 0.0 && weapon.min_range >= weapon.range {
            let words = enf("{name}'s minimum range ({min}) is no less than its range ({range}): it never fires.", [("name", name()), ("min", Fill::number(weapon.min_range)), ("range", Fill::number(weapon.range))]);
            self.issue(Severity::Warning, "weapon.min_range", path, words, en("Give it a minrange under its range, or none."));
            if self.halted { return; }
        }
        let parts = weapon.paralyze_radius > 0.0 || weapon.paralyze_on_fire || !weapon.paralyze_exempt.trim().is_empty();
        if weapon.paralyzer && weapon.paralyze_time > 0.0 {
            let words = enf("{name} is one of TA's paralyzers (paralyzer=1), whose damage is its stun: its paralyzetime is not played.", [("name", name())]);
            self.issue(Severity::Warning, "weapon.paralyze", path, words, en("Leave out paralyzer=1 to stun beside its damage, or paralyzetime to stun by its damage alone."));
        } else if parts && weapon.paralyze_time <= 0.0 {
            let words = enf("{name} gives a stun's reach or the units it spares but no paralyzetime: it stuns nothing.", [("name", name())]);
            self.issue(Severity::Warning, "weapon.paralyze", path, words, en("Give it paralyzetime, how many seconds the units it hits are stunned for."));
        }
    }

    /// What the mod's own files use of the abilities, for how TA Reborn plays it.
    pub(super) fn ability_uses(uses: &AbilityUses) -> Option<(Words, &'static str)> {
        let any = uses.min_ranges + uses.stuns + uses.toggles + uses.fields + uses.views + uses.auras > 0;
        any.then(|| {
            let words = enf("TA Reborn's own abilities. Weapons with a minimum range: {min}; with a stun: {stuns}. Units with a weapon toggle: {toggles}; a stealth field: {fields}; remote viewing: {views}; a regeneration aura: {auras}.",
                [
                    ("min", Fill::number(uses.min_ranges)),
                    ("stuns", Fill::number(uses.stuns)),
                    ("toggles", Fill::number(uses.toggles)),
                    ("fields", Fill::number(uses.fields)),
                    ("views", Fill::number(uses.views)),
                    ("auras", Fill::number(uses.auras)),
                ],
            );
            (words, en("Played as docs/MODDING.md tells them (minrange, paralyzetime, [TOGGLE], RadarStealthField, RemoteViewRadius, RegenAuraRadius). TA's own engine never read these keys."))
        })
    }
}
