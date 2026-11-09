//! Cheats: TA's own (`+atm`, `+doubleshot`...) and more. They are commands like any other, so
//! replays and saved games play them back; the simulation takes them only when
//! [`Sim::cheats_allowed`] (games with one person in them).

use std::sync::Arc;

use crate::defs::Locomotion;
use crate::events::SimEvent;
use crate::fixed::{Fx, FxVec3, cos, sin};
use crate::path::FlowField;
use crate::sim::Sim;
use crate::unit::UnitId;
use crate::world::World;

/// Most units one spawn places.
pub const MAX_SPAWN: u16 = 100;
/// Experience that makes a unit elite (veterancy 3/4; rank 3 starts at 2/3).
const ELITE_XP: i64 = 9;

/// A cheat; see [`Command::Cheat`](crate::Command::Cheat).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Cheat {
    /// Metal and energy storage filled (TA's `+atm`).
    Resources,
    /// The player's weapons do double (1), normal (0) or half (-1) damage (TA's `+doubleshot`
    /// and `+halfshot`).
    Damage(i8),
    /// The player's units take no harm (but from their own self-destruct).
    Invulnerable(bool),
    /// Whatever the player builds is finished at once, for nothing; repairs too.
    InstantBuild(bool),
    /// The player's storage stays full.
    Unlimited(bool),
    /// `count` finished units of a type for `owner` (any player), around `pos`.
    Spawn { type_id: usize, owner: u8, pos: FxVec3, count: u16 },
    /// Units blow up.
    Destroy(#[serde(deserialize_with = "crate::serde_limits::command_list")] Vec<UnitId>),
    /// Units are mended.
    Heal(#[serde(deserialize_with = "crate::serde_limits::command_list")] Vec<UnitId>),
    /// Units become elite.
    Promote(#[serde(deserialize_with = "crate::serde_limits::command_list")] Vec<UnitId>),
    /// Anything may be built (a mission's restrictions are lifted).
    Unlock,
    /// The player's enemies are defeated, and a mission is won.
    Win,
}

/// A player's standing cheats.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cheats {
    /// The player's weapons: 1 double damage, -1 half, 0 as they are.
    pub damage: i8,
    pub invulnerable: bool,
    pub instant_build: bool,
    pub unlimited: bool,
    /// Whether the player cheated at all (records leave the game out).
    pub used: bool,
}

impl Sim {
    pub(crate) fn apply_cheat(&mut self, player: u8, cheat: &Cheat) {
        let p = player as usize;
        if !self.cheats_allowed || self.players.get(p).is_none_or(|q| q.defeated) {
            return;
        }
        self.players[p].cheats.used = true;
        match cheat {
            Cheat::Resources => {
                let e = &mut self.players[p].economy;
                e.metal = e.metal.max(e.metal_capacity);
                e.energy = e.energy.max(e.energy_capacity);
            }
            Cheat::Damage(d) => self.players[p].cheats.damage = (*d).clamp(-1, 1),
            Cheat::Invulnerable(on) => self.players[p].cheats.invulnerable = *on,
            Cheat::InstantBuild(on) => self.players[p].cheats.instant_build = *on,
            Cheat::Unlimited(on) => self.players[p].cheats.unlimited = *on,
            Cheat::Spawn { type_id, owner, pos, count } => {
                if *type_id < self.types.types.len() && (*owner as usize) < self.players.len() {
                    self.place_units(*type_id, *owner, *pos, (*count).min(MAX_SPAWN));
                }
            }
            Cheat::Destroy(units) => {
                for &id in units {
                    let Some(max) = self.unit(id).filter(|u| !u.dead).map(|u| self.types.get(u.type_id).max_health) else { continue };
                    if let Some(u) = self.unit_mut(id) {
                        // A full-blooded death: its Killed script blows it to pieces.
                        u.health = Fx::ZERO;
                        u.last_damage = max;
                        u.dead = true;
                        u.killer = None;
                    }
                }
            }
            Cheat::Heal(units) => {
                for &id in units {
                    let Some(max) = self.unit(id).filter(|u| !u.dead).map(|u| self.types.get(u.type_id).max_health) else { continue };
                    if let Some(u) = self.unit_mut(id) {
                        u.health = u.health.max(max * u.build_progress).max(Fx::ONE);
                        if u.is_complete() {
                            u.health = max;
                        }
                        u.paralyzed_until = 0;
                    }
                }
            }
            Cheat::Promote(units) => {
                for &id in units {
                    if let Some(u) = self.unit_mut(id).filter(|u| !u.dead) {
                        u.xp = u.xp.max(Fx::from_int(ELITE_XP));
                    }
                }
            }
            Cheat::Unlock => self.allowed = None,
            Cheat::Win => {
                let tick = self.tick;
                for q in 0..self.players.len() {
                    let rival = q as u8;
                    if self.allied(player, rival) || self.players[q].defeated {
                        continue;
                    }
                    self.players[q].defeated = true;
                    self.events.push(SimEvent::PlayerDefeated { player: rival });
                    for u in self.units.iter_mut().flatten().filter(|u| u.owner == rival && !u.dead) {
                        u.self_destruct_at = Some(tick + 20 + (u.id.slot as u64 * 7) % 150);
                    }
                }
                if let Some(m) = &mut self.mission
                    && m.human == player
                    && m.outcome.is_none()
                {
                    m.outcome = Some(true);
                    self.events.push(SimEvent::MissionEnded { victory: true });
                }
            }
        }
    }

    /// Damage a hit does after the cheats of the players involved: `source`'s owner's weapons
    /// (double or half), `victim_owner`'s god mode (and a mission's immune units). A unit's own
    /// blast (self-destruct, the defeat) is never changed.
    pub(crate) fn cheat_damage(&self, damage: Fx, source: UnitId, victim: UnitId, victim_owner: u8) -> Fx {
        if source == victim {
            return damage;
        }
        if self.players.get(victim_owner as usize).is_some_and(|p| p.cheats.invulnerable) || self.is_immune(victim) {
            return Fx::ZERO;
        }
        match self.unit(source).and_then(|s| self.players.get(s.owner as usize)).map_or(0, |p| p.cheats.damage) {
            1 => damage * 2,
            -1 => damage / 2,
            _ => damage,
        }
    }

    /// Places up to `count` finished units of a type for `owner` around `near`: buildings on
    /// the nearest free sites, others on rings around it where they can stand (aircraft
    /// anywhere on the map), facing `heading`. Stops at the owner's unit limit or when there is
    /// no room. Returns the new units.
    pub fn place_units(&mut self, type_id: usize, owner: u8, near: FxVec3, count: u16) -> Vec<UnitId> {
        self.place_units_facing(type_id, owner, near, count, 0)
    }

    pub fn place_units_facing(&mut self, type_id: usize, owner: u8, near: FxVec3, count: u16, heading: i32) -> Vec<UnitId> {
        self.place_units_bound(type_id, owner, near, count, heading, None)
    }

    /// As [`Sim::place_units_facing`]; ground units only where they can get to `bound` (where
    /// they are going) when given, else on the ground of the first spot found: a unit on a
    /// ledge of its own, or in a clearing trees shut off, would never get anywhere. If no
    /// spot will do, anywhere will. None is put inside another unit: they keep clear of those
    /// standing there already (a wave's other kinds, placed just before them).
    pub fn place_units_bound(&mut self, type_id: usize, owner: u8, near: FxVec3, count: u16, heading: i32, bound: Option<FxVec3>) -> Vec<UnitId> {
        let types = self.types.clone();
        let t = types.get(type_id);
        let spec = self.move_spec(type_id);
        let (w, h) = self.world.size_px();
        let margin = Fx::from_int(32);
        let air = t.locomotion == Locomotion::Air;
        let region_at = |sim: &Sim, p: FxVec3| sim.world.component(&spec, World::pos_to_anchor(p, spec.footprint));
        let region = bound.filter(|_| !air).map(|b| region_at(self, b)).filter(|&r| r != 0);
        // Where the way to `bound` starts from (trees and buildings included): its distance
        // field, kept for the units' own routes.
        let field = match bound.filter(|_| !air && t.locomotion != Locomotion::Static) {
            Some(b) => {
                let goal = World::pos_to_anchor(b, spec.footprint);
                let at = World::pos_to_anchor(near, spec.footprint);
                let cached = self.current_flow((spec, goal)).filter(|f| f.covers(at));
                Some(cached.unwrap_or_else(|| {
                    let (from, to) = crate::sim::flow_window(&self.world, goal, (at.0, at.1, at.0, at.1), None);
                    let f = Arc::new(FlowField::build_within(&self.world, &spec, goal, self.tick, from, to));
                    self.keep_flow((spec, goal), f.clone());
                    f
                }))
            }
            None => None,
        };
        // Spots on rings, one unit's width (and a little) apart.
        let spacing = Fx::from_int((t.footprint.0.max(t.footprint.1) as i64 * 16 + 14).max(36));
        const SPOTS: i32 = 2_000;
        // The units standing round there already, each with half its footprint's width (the
        // ground a building closes is closed to them anyway): a spot keeps clear of them all,
        // and of those placed here, as the rings keep units of one kind apart.
        let half = |t: &crate::defs::UnitType| Fx::from_int(t.footprint.0.max(t.footprint.1) as i64 * 8);
        let reach = spacing * (ring_of(SPOTS) as i64 + 1) + Fx::from_int(256);
        let mut standing: Vec<(FxVec3, Fx)> = if t.locomotion == Locomotion::Static {
            Vec::new()
        } else {
            self.units()
                .filter(|u| !u.dead && u.carried_by.is_none() && types.get(u.type_id).locomotion != Locomotion::Static && u.pos.within_xz(near, reach))
                .map(|u| (u.pos, half(types.get(u.type_id))))
                .collect()
        };
        let own = half(t);
        // Where a spot must be, as long as each holds: where the way to `bound` reaches it; then
        // on the ground of `bound` (a target inside a building has no way to it that a field
        // finds); then anywhere, on the ground of the first of them.
        let mut way = region.is_some() && field.is_some();
        let mut ground = region;
        let mut out = Vec::new();
        let mut k = 0i32;
        for _ in 0..count {
            if self.at_unit_limit(owner) {
                break;
            }
            let spot = if t.locomotion == Locomotion::Static {
                self.find_build_site(type_id, near, 24)
            } else {
                let search = |sim: &Sim, k: &mut i32, way: bool, ground: Option<crate::world::Region>| {
                    while *k < SPOTS {
                        let p = ring_spot(near, spacing, *k);
                        *k += 1;
                        let inside = p.x > margin && p.z > margin && p.x < w - margin && p.z < h - margin;
                        let free = air || sim.world.passable(&spec, World::pos_to_anchor(p, spec.footprint), None);
                        let clear = standing.iter().all(|&(q, r)| !p.within_xz(q, r + own));
                        let reaches = !way || field.as_ref().is_some_and(|f| f.reaches(World::pos_to_anchor(p, spec.footprint)));
                        if inside && free && clear && reaches && ground.is_none_or(|r| region_at(sim, p) == r) {
                            return Some(p);
                        }
                    }
                    None
                };
                let start = k;
                let mut found = search(self, &mut k, way, ground);
                // (A rule given up for one unit is given up for the rest of them: the search
                // goes on round the rings from where it got to, never back to the middle.)
                if found.is_none() && way {
                    way = false;
                    k = start;
                    found = search(self, &mut k, way, ground);
                }
                if found.is_none() && ground.is_some() {
                    ground = None;
                    k = start;
                    found = search(self, &mut k, way, ground);
                }
                if ground.is_none() && !air {
                    ground = found.map(|p| region_at(self, p)).filter(|&r| r != 0);
                }
                found
            };
            let Some(pos) = spot else { break };
            if let Some(id) = self.spawn_unit(type_id, owner, pos, heading, true) {
                standing.push((pos, own));
                out.push(id);
            }
        }
        out
    }
}

/// The ring the `k`th spot of [`ring_spot`] is on (0: the centre).
fn ring_of(k: i32) -> i32 {
    let (mut ring, mut first) = (0, 1);
    while k >= first {
        ring += 1;
        first += 6 * ring;
    }
    ring
}

/// The `k`th spot on rings around `centre`: the centre, then six on the first ring, twelve on
/// the second and so on, `spacing` apart.
fn ring_spot(centre: FxVec3, spacing: Fx, k: i32) -> FxVec3 {
    if k == 0 {
        return centre;
    }
    let (mut ring, mut first) = (1, 1);
    while k >= first + 6 * ring {
        first += 6 * ring;
        ring += 1;
    }
    let slot = k - first;
    let angle = slot * (65536 / (6 * ring)) + ring * 3000;
    let r = spacing * ring as i64;
    centre + FxVec3::new(cos(angle) * r, Fx::ZERO, sin(angle) * r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_spots_fill_ring_by_ring_without_repeats() {
        let c = FxVec3::from_ints(1000, 0, 1000);
        let s = Fx::from_int(40);
        assert_eq!(ring_spot(c, s, 0), c);
        let spots: Vec<FxVec3> = (0..19).map(|k| ring_spot(c, s, k)).collect();
        for (i, a) in spots.iter().enumerate() {
            for b in &spots[i + 1..] {
                assert!(!a.within_xz(*b, Fx::from_int(20)), "spots overlap: {a:?} {b:?}");
            }
        }
        // The first ring's six sit one spacing out, the second ring's twelve two.
        for (k, ring) in [(1, 1), (6, 1), (7, 2), (18, 2), (19, 3)] {
            let d = (ring_spot(c, s, k) - c).xz().length().to_int();
            assert!((d - 40 * ring).abs() <= 1, "spot {k}: {d} from the centre");
        }
    }
}
