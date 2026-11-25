//! Shields (TA Reborn's: a unit type's `ShieldHealth` and the other shield keys, or an
//! enhancement's; docs/MODDING.md, "Shields"): Supreme Commander's shield generators, the units with
//! shields of their own, the commanders' shield enhancements, the Fatboy's.
//!
//! - **A personal shield** (no `ShieldRadius`) takes the damage the unit takes before its health
//!   does (`Sim::damage_unit`, through [`absorb`]): as much as it has, the rest to the unit. Its own
//!   blast (a self-destruct) goes through.
//! - **A bubble** is a dome round the unit, of radius `ShieldRadius` across and `ShieldHeight` high
//!   (and as far down: an ellipsoid round its foot), that stops its enemies' shots coming into it
//!   from outside (`Sim::bubble_entry`, in the projectiles' sweep): a shot fired from inside goes
//!   out, its own side's and its allies' pass, as do strategic missiles (Forged Alliance's nukes go
//!   through shields) and what runs under the water (torpedoes, depth charges, their blasts too). An aircraft in
//!   flight counts as over every dome (TA's flying heights are low, under a big dome's top): its shot
//!   fired under an enemy's dome bursts on it at once. The shot bursts on
//!   the dome, which takes its whole damage; any blast going off outside a dome takes what of it
//!   reaches the dome, as a unit there would (its falloff), and spares what is under the dome
//!   (`Sim::blast_on_bubbles`), which takes only the share the dome could not hold (its spill: none
//!   while it holds). Overlapping domes each take what reaches them. A blast inside a dome is not
//!   stopped by it.
//!
//! Either kind has its own hit points: it regains `ShieldRegen` a second while it is up, from
//! `ShieldRegenDelay` seconds after its last hit; run out, it collapses ([`SimEvent::ShieldChanged`]
//! with `collapsed`), and comes back whole once it has charged for `ShieldRechargeTime` seconds. It
//! is up only while its unit is finished, not carried, switched on (or not switchable) and its
//! upkeep paid (none is, for a stunned unit): the unit's upkeep (`Sim::economy_end`) takes its
//! `ShieldEnergyUse` while it is switched on, up or charging, so short of energy it drops as a radar
//! does; switched on again, or with its energy again, it takes [`RESTART_TICKS`] to come back up,
//! its hit points as they were. A collapsed shield charges only while it is so powered. Every tick
//! ([`Sim::update_shields`], before the projectiles fly) the domes up are listed where their units
//! stand ([`Bubble`]). Nothing of it runs in a game whose units have none (`UnitTypes::shields`:
//! TA's own), and a unit's state ([`Unit::shield`]) is in the checksum only where it has one.
//!
//! [`Unit::shield`]: crate::unit::Unit::shield

use crate::cob::TICKS_PER_SECOND;
use crate::combat::{Projectile, segment_entry};
use crate::defs::{Shield, UnitType, WeaponType};
use crate::events::SimEvent;
use crate::fixed::{Fx, FxVec3};
use crate::unit::{Unit, UnitId};

use super::Sim;

/// Ticks a shield takes to come back up once switched on again, given its energy again or set down
/// by a transport (Forged Alliance's shields all take five seconds), its hit points as they were.
pub const RESTART_TICKS: u64 = 5 * TICKS_PER_SECOND as u64;

/// A unit's shield as it stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShieldState {
    /// Its hit points now (none while it is down after a collapse).
    pub health: Fx,
    /// Up: a bubble stops shots, a personal shield takes the unit's damage.
    pub up: bool,
    /// Ticks of charge still to come before it is up (none: it comes up as soon as it is powered).
    pub charge: u64,
    /// Down after a collapse: it comes back whole once charged.
    pub collapsed: bool,
    /// The last tick it was hit (it regains its hit points from so long after).
    pub hit_at: u64,
}

impl ShieldState {
    /// A shield of `def` just made: whole, up as soon as it is powered.
    fn whole(def: &Shield) -> ShieldState {
        ShieldState { health: def.health, up: false, charge: 0, collapsed: false, hit_at: 0 }
    }

    /// `amount` of damage on it at `tick` (up): what it takes, and whether that collapsed it (its
    /// charge then the `recharge` of its make).
    fn take(&mut self, amount: Fx, recharge: u64, tick: u64) -> (Fx, bool) {
        let taken = self.health.min(amount).max(Fx::ZERO);
        self.health -= taken;
        self.hit_at = tick;
        let collapsed = self.health <= Fx::ZERO;
        if collapsed {
            self.health = Fx::ZERO;
            self.up = false;
            self.collapsed = true;
            self.charge = recharge.max(1);
        }
        (taken, collapsed)
    }
}

/// A bubble up this tick, as its unit stood when the domes were listed: its unit and owner, the
/// middle of its dome (the unit's foot) and its radius across and height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bubble {
    pub unit: UnitId,
    pub owner: u8,
    pub centre: FxVec3,
    pub radius: Fx,
    pub height: Fx,
}

impl Bubble {
    /// `p` with its height from the dome's middle stretched (or squeezed) as the dome's height is
    /// to its radius: the dome then a ball of its radius.
    #[inline]
    fn round(&self, p: FxVec3) -> FxVec3 {
        FxVec3::new(p.x, self.centre.y + (p.y - self.centre.y) * self.radius / self.height, p.z)
    }

    /// Whether `p` is under the dome.
    #[inline]
    pub fn contains(&self, p: FxVec3) -> bool {
        self.round(p).within(self.centre, self.radius)
    }

    /// How far `p`, outside the dome, lies from it (none within). A dome not a ball (its height not
    /// its radius) is an ellipsoid, the nearest place on it no closed sum: this is its first-order
    /// estimate (how far out it is in the dome's round space, over how fast that grows here), exact
    /// straight out from the middle, over its top and along the ground, and nowhere more than the
    /// true distance, so a blast reaching the dome never passes it by.
    fn distance_out(&self, p: FxVec3) -> Fx {
        let along = self.round(p).distance(self.centre);
        if along <= self.radius {
            return Fx::ZERO;
        }
        let k = self.radius / self.height;
        let rel = p - self.centre;
        // (The round space's distance grows by |(x, k²y, z)| / along a pixel out.)
        let growth = FxVec3::new(rel.x, rel.y * k * k, rel.z).length();
        if growth.is_zero() {
            return along - self.radius;
        }
        (along - self.radius) * along / growth
    }

    /// Where on the dome the line from its middle to `p` meets it.
    fn surface_towards(&self, p: FxVec3) -> FxVec3 {
        let along = self.round(p).distance(self.centre);
        if along.is_zero() {
            return self.centre + FxVec3::new(Fx::ZERO, self.height, Fx::ZERO);
        }
        self.centre + (p - self.centre).scale(self.radius / along)
    }

    /// How far along the segment `a`..`b` (0 to 1) it comes into the dome from outside: none for one
    /// starting under it (a shot fired from inside goes out).
    fn entry(&self, a: FxVec3, b: FxVec3) -> Option<Fx> {
        let (a, b) = (self.round(a), self.round(b));
        if a.within(self.centre, self.radius) {
            return None;
        }
        segment_entry(a, b, self.centre, self.radius)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The distance to a dome flatter than a ball is estimated never more than the true one (a blast
    /// reaching the dome meets it), close to it, and exactly straight out, over the top and along the
    /// ground.
    #[test]
    fn the_distance_to_a_flat_dome_is_never_overestimated() {
        let bubble = Bubble { unit: UnitId { slot: 1, generation: 0 }, owner: 0, centre: FxVec3::ZERO, radius: Fx::from_int(400), height: Fx::from_int(80) };
        // (The true distance: the nearest of the ellipse's points, sampled finely: a test may use
        // floats.)
        let truth = |x: f64, y: f64| (0..=20000).map(|i| {
            let a = std::f64::consts::PI * i as f64 / 20000.0;
            ((x - 400.0 * a.cos()).powi(2) + (y - 80.0 * a.sin()).powi(2)).sqrt()
        }).fold(f64::MAX, f64::min);
        for (x, y) in [(400, 80), (300, 100), (500, 0), (0, 130), (450, 40), (380, 60), (100, 200)] {
            let estimate = bubble.distance_out(FxVec3::from_ints(x, y, 0)).to_f64();
            let true_distance = truth(x as f64, y as f64);
            assert!(estimate <= true_distance + 0.5, "({x}, {y}): {estimate} past the true {true_distance}");
            assert!(estimate >= true_distance * 0.6, "({x}, {y}): {estimate} far under the true {true_distance}");
        }
        assert_eq!(bubble.distance_out(FxVec3::from_ints(500, 0, 0)), Fx::from_int(100), "straight out along the ground");
        assert_eq!(bubble.distance_out(FxVec3::from_ints(0, 130, 0)), Fx::from_int(50), "over the top");
        assert_eq!(bubble.distance_out(FxVec3::from_ints(100, 10, 0)), Fx::ZERO, "within");
    }
}

/// What a blast met of the domes (`Sim::blast_on_bubbles`): each it reached from outside, with the
/// share of the blast it let through to what is under it.
pub(crate) struct Cover(Vec<(Bubble, Fx)>);

impl Cover {
    /// The share of the blast that reaches what has its middle at `c`: all of it outside every dome
    /// the blast met, else the least those over it let through.
    pub(crate) fn through(&self, c: FxVec3) -> Fx {
        self.0.iter().filter(|(b, _)| b.contains(c)).map(|&(_, spill)| spill).min().unwrap_or(Fx::ONE)
    }
}

/// The energy a tick `u`'s shield takes while it is switched on (none without one): the economy
/// charges it with the unit's upkeep (`Sim::economy_end`, `flows.rs`).
#[inline]
pub(crate) fn upkeep(u: &Unit, t: &UnitType) -> Fx {
    u.abilities(t).shield.map_or(Fx::ZERO, |s| s.energy)
}

/// What of `damage` reaches `u` through its personal shield, at `tick`: all of it without one up
/// (or with a bubble: domes stop shots, `Sim::bubble_entry`); else what the shield could not take.
/// With what the shield took, and whether that collapsed it, where it took some.
pub(crate) fn absorb(u: &mut Unit, t: &UnitType, damage: Fx, tick: u64) -> (Fx, Option<(Fx, bool)>) {
    let Some(def) = u.abilities(t).shield.filter(|s| !s.bubble()) else { return (damage, None) };
    let Some(s) = u.shield.as_mut().filter(|s| s.up) else { return (damage, None) };
    let (taken, collapsed) = s.take(damage, def.recharge, tick);
    (damage - taken, Some((taken, collapsed)))
}

impl Sim {
    /// Whether `u`'s shield is switched on: as the economy charges a unit's upkeep, while it is
    /// switched on (TA's `ActivateWhenBuilt`, or its player), or has no switch and no upkeep of its
    /// own (`Sim::economy_end`).
    pub fn shield_switched_on(&self, u: &Unit) -> bool {
        let t = self.unit_type(u);
        u.activated || !t.def.on_offable && t.energy_use.is_zero()
    }

    /// `u`'s shield, if it has one (its type's, or an enhancement's): its make, and how it stands
    /// (none yet for a frame, or one finished this tick).
    pub fn shield(&self, u: &Unit) -> Option<(Shield, Option<ShieldState>)> {
        let def = u.abilities(self.unit_type(u)).shield?;
        Some((def, u.shield))
    }

    /// The bubbles up this tick (as their units stood when they were listed).
    pub fn bubbles(&self) -> &[Bubble] {
        &self.bubbles
    }

    /// Whether `id`'s shield is up now (a dome listed this tick may have collapsed since).
    pub(crate) fn bubble_up(&self, id: UnitId) -> bool {
        self.unit(id).is_some_and(|u| !u.dead && u.shield.is_some_and(|s| s.up))
    }

    /// Every shield's tick, in slot order: a new one made whole, its power looked at (down when
    /// its unit is not powered, charging back up once it is), its regaining, and each dome up
    /// listed where its unit stands ([`Bubble`]).
    pub(super) fn update_shields(&mut self) {
        self.bubbles.clear();
        if !self.types.shields {
            return;
        }
        let (types, tick) = (self.types.clone(), self.tick);
        for u in self.units.iter_mut().flatten() {
            let t = types.get(u.type_id);
            let Some(def) = u.abilities(t).shield else {
                // (One whose enhancement giving it is taken out.)
                u.shield = None;
                continue;
            };
            if u.dead || !u.is_complete() {
                continue;
            }
            // (Switched on as its upkeep is charged, `Sim::shield_switched_on`; that upkeep paid.)
            let powered = u.carried_by.is_none() && u.upkeep_paid && (u.activated || !t.def.on_offable && t.energy_use.is_zero());
            let s = u.shield.get_or_insert(ShieldState::whole(&def));
            s.health = s.health.min(def.health);
            if !powered {
                if s.up {
                    s.up = false;
                    self.events.push(SimEvent::ShieldChanged { unit: u.id, up: false, collapsed: false });
                }
                // (Charging after a collapse waits; otherwise it comes back up a while after.)
                if !s.collapsed {
                    s.charge = RESTART_TICKS;
                }
                continue;
            }
            if s.charge > 0 {
                s.charge -= 1;
                if s.charge > 0 {
                    continue;
                }
                if s.collapsed {
                    s.health = def.health;
                    s.collapsed = false;
                }
            }
            if !s.up {
                s.up = true;
                self.events.push(SimEvent::ShieldChanged { unit: u.id, up: true, collapsed: false });
            }
            if s.health < def.health && tick >= s.hit_at.saturating_add(def.regen_delay) {
                s.health = (s.health + def.regen).min(def.health);
            }
            if def.bubble() {
                self.bubbles.push(Bubble { unit: u.id, owner: u.owner, centre: u.pos, radius: def.radius, height: def.height });
            }
        }
    }

    /// The first dome a shot's flight this tick comes into from outside, if any: how far along it
    /// (0 to 1), and whose. Only its enemies' domes; none for a strategic missile (Forged Alliance's
    /// nukes go through shields), an interceptor, or a shot that runs under the water.
    pub(crate) fn bubble_entry(&self, p: &Projectile, w: &WeaponType) -> Option<(Fx, UnitId)> {
        if self.bubbles.is_empty() || w.def.interceptor || w.water || w.def.targetable_classes() & ta_data::STRATEGIC_CLASS != 0 {
            return None;
        }
        let (a, b) = (p.prev_pos, p.pos);
        let reach = (b - a).length();
        // (An aircraft in flight is over every dome: TA's flying heights are low, under the tops
        // of the domes round what they shield, so a shot it fires under an enemy's dome, at its
        // first stretch, bursts on it there.)
        let from_air = p.age <= 1 && self.unit(p.owner).is_some_and(|o| self.airborne(o));
        let mut first: Option<(Fx, UnitId)> = None;
        for bubble in &self.bubbles {
            if !a.within_xz(bubble.centre, bubble.radius + reach) || !self.is_enemy(p.owner_player, bubble.owner) || !self.bubble_up(bubble.unit) {
                continue;
            }
            let entry = if from_air && bubble.contains(a) { Some(Fx::ZERO) } else { bubble.entry(a, b) };
            if let Some(when) = entry
                && first.is_none_or(|(before, _)| when < before)
            {
                first = Some((when, bubble.unit));
            }
        }
        first
    }

    /// The domes a blast of `w` at `pos`, reaching `r` round it, meets from outside: each up, of an
    /// enemy of `player` (of anybody, for a blast of nobody's), with `pos` outside it and within the
    /// blast's reach of it, or `struck` (the one its shot burst on). Each takes what reaches it, as
    /// a unit there would (its falloff; `struck` all of it), and lets through to what is under it
    /// the share it could not take. A stun (TA's paralyzers) is stopped whole and takes nothing off
    /// a dome. None where no dome stands.
    pub(crate) fn blast_on_bubbles(&mut self, w: &WeaponType, pos: FxVec3, r: Fx, source: UnitId, player: Option<u8>, struck: Option<UnitId>) -> Option<Cover> {
        // (What runs under the water passes under the domes, as its shots do, `bubble_entry`: its
        // blast takes nothing off them and is held off nothing under them.)
        if self.bubbles.is_empty() || w.water {
            return None;
        }
        let mut met: Vec<(Bubble, Fx)> = Vec::new();
        for &b in &self.bubbles {
            let whole = struck == Some(b.unit);
            if player.is_some_and(|p| !self.is_enemy(p, b.owner)) || !self.bubble_up(b.unit) || !whole && b.contains(pos) {
                continue;
            }
            let d = if whole { Fx::ZERO } else { b.distance_out(pos) };
            if !whole && d >= r {
                continue;
            }
            let base = self.unit(b.unit).map_or(w.default_damage, |u| w.damage_to(u.type_id));
            // (Spring's and TA's falloff, as a unit's: `Sim::explode_at`.)
            let m = if whole { Fx::ONE } else { ((r - d) / (r - d * w.edge_effectiveness).max(Fx::EPSILON)).clamp(Fx::ZERO, Fx::ONE) };
            met.push((b, base * m));
        }
        if met.is_empty() {
            return None;
        }
        let mut cover = Vec::with_capacity(met.len());
        for (b, amount) in met {
            let at = if struck == Some(b.unit) { pos } else { b.surface_towards(pos) };
            let spill = if w.paralyzer { Fx::ZERO } else { self.bubble_takes(b.unit, amount, source, at) };
            cover.push((b, spill));
        }
        Some(Cover(cover))
    }

    /// `amount` of damage from `source` on `id`'s dome at `at` (up): as on a unit, cheats, the
    /// Commanders' perks and the mutators first (a ceasefire, none). What its hit points take; the
    /// share of `amount` they could not (0 to 1), which goes through to what is under it.
    fn bubble_takes(&mut self, id: UnitId, amount: Fx, source: UnitId, at: FxVec3) -> Fx {
        if self.ceasefire_spares(source, id) {
            return Fx::ZERO;
        }
        let Some(owner) = self.unit(id).map(|u| u.owner) else { return Fx::ONE };
        let amount = self.mutator_damage(self.perk_damage(self.cheat_damage(amount, source, id, owner), source, id));
        if amount <= Fx::ZERO {
            return Fx::ZERO;
        }
        let (types, tick) = (self.types.clone(), self.tick);
        let Some(u) = self.unit_mut(id) else { return Fx::ONE };
        let Some(def) = u.abilities(types.get(u.type_id)).shield else { return Fx::ONE };
        let Some(s) = u.shield.as_mut().filter(|s| s.up) else { return Fx::ONE };
        let (taken, collapsed) = s.take(amount, def.recharge, tick);
        if collapsed {
            // (Its dome gone from the list too.)
            self.bubbles.retain(|b| b.unit != id);
        }
        self.shield_struck(id, at, taken, collapsed, source);
        (amount - taken) / amount
    }

    /// What follows a shield's hit: the events (its ripple, a collapse), and its owner told the
    /// enemy is shooting at them (as `Sim::damage_unit` tells it).
    pub(crate) fn shield_struck(&mut self, id: UnitId, at: FxVec3, taken: Fx, collapsed: bool, source: UnitId) {
        self.events.push(SimEvent::ShieldHit { unit: id, pos: at, damage: taken });
        if collapsed {
            self.events.push(SimEvent::ShieldChanged { unit: id, up: false, collapsed: true });
        }
        let Some(owner) = self.unit(id).map(|u| u.owner) else { return };
        let attacker = self.unit(source).map(|s| s.owner);
        let tick = self.tick;
        if attacker.is_none_or(|a| self.is_enemy(a, owner))
            && let Some(p) = self.players.get_mut(owner as usize)
            && p.last_alert.is_none_or(|t| tick >= t + 30 * 10)
        {
            p.last_alert = Some(tick);
            self.events.push(SimEvent::UnderAttack { player: owner, unit: id, pos: at });
        }
    }

    /// Whether what has its middle at `c` is under a dome up of an enemy of `player` (of anybody's
    /// for none) that `from` is outside: a stun going off at `from` spares it.
    pub(crate) fn shielded_from(&self, c: FxVec3, from: FxVec3, player: Option<u8>) -> bool {
        self.bubbles.iter().any(|b| player.is_none_or(|p| self.is_enemy(p, b.owner)) && b.contains(c) && !b.contains(from) && self.bubble_up(b.unit))
    }
}
