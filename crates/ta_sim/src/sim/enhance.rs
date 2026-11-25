//! Enhancements (a unit's `[ENHANCEMENTS]`: Supreme Commander's commander enhancements): parts a
//! unit installs on itself, one a slot. Given [`Order::Enhance`] it stands and builds one with its
//! own build power (and its helpers'), paying its metal and energy over its build time as
//! construction does, as an upgrade is built (`upgrade.rs`); done, it is installed in its slot in
//! place of what was there, and the unit's figures are its type's with every enhancement installed
//! added ([`Boost`]): more health (its health rising by as much), faster mending, more build power,
//! farther sight, radar and sonar, weapon slots filled or swapped, units added to its menu, a cloak
//! or stealth. One needing another (`Prerequisite`) waits for it to be installed. Removed
//! ([`Order::Enhance`] with `remove`), its slot is empty again at once and for nothing, its effects
//! gone (its health falling to its most, no further). Called off, an install gives its metal back.

use ta_data::unit::MAX_WEAPONS;

use crate::defs::UnitType;
use crate::events::SimEvent;
use crate::fixed::Fx;
use crate::unit::{Boost, Order, SelfJob, SelfWork, Unit, UnitId};

use super::Sim;

/// Whether an enhancement may be installed, and if not why (for the interface).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// It may be installed (in its slot's place: `replaces`, the one there, if any).
    Free { replaces: Option<u16> },
    /// It is installed.
    Installed,
    /// It is being installed now.
    Installing,
    /// It needs another installed first (that one).
    Needs(u16),
    /// The unit is not finished, or has no such enhancement.
    No,
}

impl Sim {
    /// Whether `u` may install its type's enhancement `index` now.
    pub fn enhancement_fit(&self, u: &Unit, index: u16) -> Fit {
        let t = self.types.get(u.type_id);
        let Some(e) = t.enhancements.get(index as usize) else { return Fit::No };
        if !u.is_complete() || u.dead {
            return Fit::No;
        }
        if u.enhancements.contains(&index) {
            return Fit::Installed;
        }
        if u.self_work.is_some_and(|w| w.job == SelfJob::Enhance(index)) {
            return Fit::Installing;
        }
        if let Some(p) = e.prerequisite
            && !u.enhancements.contains(&p)
        {
            return Fit::Needs(p);
        }
        let replaces = u.enhancements.iter().copied().find(|&i| t.enhancements.get(i as usize).is_some_and(|o| o.slot == e.slot));
        Fit::Free { replaces }
    }

    /// The enhancement installed in a slot of a unit's, if any.
    pub fn installed_in(&self, u: &Unit, slot: u8) -> Option<u16> {
        let t = self.types.get(u.type_id);
        u.enhancements.iter().copied().find(|&i| t.enhancements.get(i as usize).is_some_and(|e| e.slot == slot))
    }

    /// A unit's enhancement order at the front of its orders: a removal done at once; an install
    /// started (it stands still) and built on a tick at a time with its own build power. One it
    /// may not install (none such, or its prerequisite missing when its turn comes) fails.
    pub(super) fn update_enhance_order(&mut self, id: UnitId, t: &UnitType, index: u16, remove: bool) {
        let pop = |sim: &mut Sim| {
            if let Some(u) = sim.unit_mut(id) {
                u.orders.pop_front();
            }
        };
        let Some(u) = self.unit(id) else { return };
        if remove {
            if u.enhancements.contains(&index) {
                self.set_enhancement(id, index, false);
            }
            pop(self);
            return;
        }
        match self.enhancement_fit(u, index) {
            Fit::Free { .. } | Fit::Installing => {}
            Fit::Installed => {
                pop(self);
                return;
            }
            Fit::Needs(_) | Fit::No => {
                self.events.push(SimEvent::OrderFailed { unit: id, reason: "needs another enhancement first" });
                self.cancel_self_work(id, true);
                pop(self);
                return;
            }
        }
        let rate = u.work_rate(t);
        if rate == 0 {
            self.events.push(SimEvent::OrderFailed { unit: id, reason: "it has no build power" });
            pop(self);
            return;
        }
        if u.self_work.is_none_or(|w| w.job != SelfJob::Enhance(index)) {
            self.cancel_self_work(id, true);
            self.stop(id);
            if let Some(u) = self.unit_mut(id) {
                u.self_work = Some(SelfWork { job: SelfJob::Enhance(index), work: 0, metal_saved: Fx::ZERO });
                u.speed = Fx::ZERO;
            }
        }
        // (It stands while it works on itself.)
        self.stop(id);
        let rate = self.mutator_work_rate(self.perk_work_rate(id, rate));
        let owner = self.unit(id).map_or(0, |u| u.owner);
        self.work_on_self(id, rate, owner, id);
    }

    /// Set-up helper (staged scenes): a finished unit's enhancement `index` installed at once, as
    /// though built (what it needs first installed before it).
    pub fn install_enhancement(&mut self, id: UnitId, index: u16) {
        let Some(u) = self.unit(id).filter(|u| u.is_complete() && !u.dead) else { return };
        let Some(e) = self.types.get(u.type_id).enhancements.get(index as usize) else { return };
        if let Some(p) = e.prerequisite.filter(|p| !u.enhancements.contains(p)) {
            self.set_enhancement(id, p, true);
        }
        self.set_enhancement(id, index, true);
    }

    /// The install done: in its slot in place of what was there; its order off the front.
    pub(super) fn finish_enhancement(&mut self, id: UnitId, index: u16) {
        if let Some(u) = self.unit_mut(id) {
            u.self_work = None;
            if matches!(u.orders.front(), Some(Order::Enhance { index: i, remove: false }) if *i == index) {
                u.orders.pop_front();
            }
        }
        self.set_enhancement(id, index, true);
    }

    /// Installs (`on`) or removes an enhancement of a unit's, at once: what its slot held goes, the
    /// unit's figures are made again, its health rising with its most (falling only to it), and
    /// its script told which (`SetEnhancement(index, on)`, where it has one: the parts shown).
    pub(crate) fn set_enhancement(&mut self, id: UnitId, index: u16, on: bool) {
        let types = self.types.clone();
        let Some(u) = self.unit(id) else { return };
        let t = types.get(u.type_id);
        let Some(e) = t.enhancements.get(index as usize) else { return };
        let mut list = u.enhancements.clone();
        let mut gone: Vec<u16> = Vec::new();
        if on {
            list.retain(|&i| {
                let keep = t.enhancements.get(i as usize).is_none_or(|o| o.slot != e.slot);
                if !keep {
                    gone.push(i);
                }
                keep
            });
            list.push(index);
        } else {
            list.retain(|&i| i != index);
            gone.push(index);
        }
        list.sort_unstable();
        let toggled = u.toggled;
        self.refit(id, list, toggled);
        // (The drones it gives made, or taken with it: `drones.rs`.)
        self.make_drones(id);
        if t.callins.set_enhancement {
            for i in gone {
                self.call_script(id, "SetEnhancement", &[i as i32, 0]);
            }
            if on {
                self.call_script(id, "SetEnhancement", &[index as i32, 1]);
            }
        }
        self.events.push(SimEvent::UnitEnhanced { id, index, on });
    }

    /// A unit's figures made again with the enhancements `installed` and its weapon toggle on or
    /// not (`toggled`): its health rising with its most (falling only to it), a slot whose weapon
    /// changed starting afresh, a stockpiling launcher swapped or gone taking its missiles with it,
    /// a cloak it no longer has off.
    pub(crate) fn refit(&mut self, id: UnitId, installed: Vec<u16>, toggled: bool) {
        let types = self.types.clone();
        let Some(u) = self.unit(id) else { return };
        let t = types.get(u.type_id);
        let before = u.max_health(t);
        let shield_before = u.abilities(t).shield;
        let boost = boost(t, &installed, toggled);
        let slots_before = *u.weapon_slots(t);
        // The weapon that builds its missiles (`stockpile`), before and after: another one (or
        // none) does not fire those built for this one, nor go on with the one under way.
        let slots_after = boost.as_ref().map_or(t.weapons, |b| b.weapons);
        let launcher = |slots: &[Option<usize>; MAX_WEAPONS]| slots.iter().flatten().copied().find(|&w| self.weapon_types.get(w).def.stockpile);
        let restock = launcher(&slots_before) != launcher(&slots_after);
        if let Some(u) = self.unit_mut(id) {
            u.enhancements = installed;
            u.toggled = toggled;
            u.boost = boost.map(Box::new);
            let after = u.max_health(t);
            if after > before {
                u.health += after - before;
            }
            u.health = u.health.min(after);
            // A slot whose weapon changed starts afresh, no target, aim or burst of the other's;
            // but its reload runs on (counting down while the slot is empty too): switching never
            // readies a gun sooner, as a toggle flipped and flipped back would.
            for slot in 0..MAX_WEAPONS {
                if slots_after[slot] != slots_before[slot] {
                    let reload = u.weapons[slot].reload;
                    u.weapons[slot] = Default::default();
                    u.weapons[slot].reload = reload;
                    if let Some(c) = &mut u.cob {
                        c.supersede_aims(slot as u16);
                    }
                }
            }
            // (A cloak it no longer has is off.)
            if u.cloak_costs(t).is_none() {
                u.cloak_on = false;
            }
            // (A shield of another make starts afresh, whole: `sim/shield.rs`.)
            if u.abilities(t).shield != shield_before {
                u.shield = None;
            }
            if restock {
                u.stockpile = 0;
                u.stockpile_work = 0;
                u.stockpile_progress = Fx::ZERO;
            }
        }
    }
}

/// A unit of type `t`'s figures with the enhancements `installed` and its weapon toggle on or not
/// (`toggled`; none of either: its type's own). The toggle's slots go over the enhancements'.
pub(crate) fn boost(t: &UnitType, installed: &[u16], toggled: bool) -> Option<Boost> {
    let toggle = t.abilities.toggle.as_ref().filter(|_| toggled);
    if installed.is_empty() && toggle.is_none() {
        return None;
    }
    let mut b = Boost {
        max_health: t.max_health,
        heal_ticks: t.heal_ticks,
        work_rate: t.work_rate,
        worker_rate: t.worker_rate,
        sight: t.sight,
        radar: t.radar,
        sonar: Fx::from_f64(t.def.sonar_distance),
        weapons: t.weapons,
        stockpile: t.stockpile,
        build_options: t.build_options.clone(),
        ai_build_options: t.ai_build_options.clone(),
        cloak: (t.def.cloak_cost > 0.0).then(|| (Fx::from_f64(t.def.cloak_cost), Fx::from_f64(t.def.cloak_cost_moving), Fx::from_f64(t.def.min_cloak_distance))),
        stealth: t.def.stealth,
        abilities: crate::defs::Abilities { toggle: None, ..t.abilities.clone() },
        metal_make: t.metal_make,
        energy_make: t.energy_make,
        upkeep: Fx::ZERO,
    };
    // Mending: its own (100 hit points every `HealTime` seconds) and theirs, in hit points a second.
    let own = if t.heal_ticks > 0 { Fx::from_int(100 * 30) / t.heal_ticks as i64 } else { Fx::ZERO };
    let mut mend = own;
    // (A shield an enhancement gives takes the place of the unit's own: of several, the strongest.)
    let mut shield = None;
    for e in installed.iter().filter_map(|&i| t.enhancements.get(i as usize)) {
        b.max_health += e.max_health;
        mend += e.heal_rate;
        b.work_rate += e.work_rate;
        b.worker_rate += e.worker_rate;
        b.sight += e.sight;
        b.radar += e.radar;
        b.sonar += e.sonar;
        for slot in 0..MAX_WEAPONS {
            if let Some(w) = e.weapons[slot] {
                b.weapons[slot] = Some(w);
            }
        }
        b.stockpile |= e.stockpile;
        for &o in &e.build_options {
            if !b.build_options.contains(&o) {
                b.build_options.push(o);
            }
            if !b.ai_build_options.contains(&o) {
                b.ai_build_options.push(o);
            }
        }
        if let Some(c) = e.cloak {
            b.cloak = Some(b.cloak.map_or(c, |o| (o.0.max(c.0), o.1.max(c.1), o.2.max(c.2))));
        }
        b.stealth |= e.stealth;
        // (TA Reborn's abilities: the widest jamming and fields, remote viewing and aura count;
        // what it makes and takes adds up.)
        let (a, e_a) = (&mut b.abilities, &e.abilities);
        a.radar_jam = a.radar_jam.max(e_a.radar_jam);
        a.sonar_jam = a.sonar_jam.max(e_a.sonar_jam);
        a.radar_field = a.radar_field.max(e_a.radar_field);
        a.sonar_field = a.sonar_field.max(e_a.sonar_field);
        a.remote_view = match (a.remote_view, e_a.remote_view) {
            (Some(x), Some(y)) => Some(if y.radius > x.radius { y } else { x }),
            (x, y) => x.or(y),
        };
        a.aura = crate::defs::Aura::stronger(a.aura, e_a.aura, t.max_health);
        shield = crate::defs::Shield::stronger(shield, e_a.shield);
        b.metal_make += e.metal_make;
        b.energy_make += e.energy_make;
        b.upkeep += e.upkeep;
    }
    b.abilities.shield = shield.or(b.abilities.shield);
    // (With none installed, its own mending as it is: toggled alone, nothing else changes.)
    if !installed.is_empty() {
        b.heal_ticks = if mend > Fx::ZERO { (Fx::from_int(100 * 30) / mend).to_int().max(1) as u64 } else { 0 };
    }
    // Its weapon toggle, on: its slots over the enhancements'.
    if let Some(toggle) = toggle {
        toggle.apply(&mut b.weapons);
        b.stockpile |= toggle.stockpile;
    }
    Some(b)
}
