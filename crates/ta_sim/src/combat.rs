//! Weapons, projectiles, damage and death.
//!
//! Aiming follows TA: the engine computes a heading/pitch to the target relative to the unit
//! and starts the script's `AimPrimary/Secondary/Tertiary` thread; when that thread returns
//! non-zero the weapon is aimed. Firing asks `QueryPrimary` for the muzzle piece, calls
//! `FirePrimary` for recoil/flash animation and launches a projectile from the muzzle.

use ta_data::RenderType;

use crate::cob::{TICKS_PER_SECOND, ThreadTag};
use crate::defs::{Locomotion, Trajectory, WeaponType};
use crate::events::SimEvent;
use crate::fixed::{Fx, FxVec3, atan2, cos, heading_of, heading_vector, rotate_x, rotate_y, sin, wrap_angle};
use crate::host::script_angle;
use crate::sim::{Feature, Sim};
use crate::unit::{Order, Target, Unit, UnitId, piece_world_position, player_bit};
use crate::world::{CELL, Occupant, World};

mod projectiles;

#[cfg(test)]
mod targeting_tests;

const AIM_FUNCS: [&str; 3] = ["AimPrimary", "AimSecondary", "AimTertiary"];
const AIM_FROM_FUNCS: [&str; 3] = ["AimFromPrimary", "AimFromSecondary", "AimFromTertiary"];
const QUERY_FUNCS: [&str; 3] = ["QueryPrimary", "QuerySecondary", "QueryTertiary"];
const FIRE_FUNCS: [&str; 3] = ["FirePrimary", "FireSecondary", "FireTertiary"];

/// How far apart the planes of a group sent at one target come in on it, across their front
/// (see `Sim::air_attack_pass`).
pub(crate) const FAN_SPACING: Fx = Fx::from_int(48);
/// Ticks between target searches for an idle weapon.
const RETARGET_TICKS: u64 = 8;
/// How far past its bombs' lead (where they must be let go to land on a target: 460 to 565 px for
/// TA's bombers) a bomber on patrol or attack-move looks for targets its side sees (px): far enough
/// to line up on one ahead, near its way.
const BOMBER_LOOKOUT_PAST_LEAD: i64 = 160;
/// How long automatic targeting passes over a target the ground hides (ticks).
const BLOCKED_TICKS: u64 = 60;
/// Ticks a look for friends in the way of a shot at a target holds for.
const FRIEND_CHECK_TICKS: u64 = 6;
/// How far from the line of a shot a friend's centre is looked for (px; the widest hit
/// spheres): the cells within it of the line are searched.
const FRIEND_PAD: i64 = 48;
/// A shell higher than this over the ground passes over every unit.
const FRIEND_TOP: i64 = 120;
/// Blasts at least this wide (px) mustn't come down within half their width of a friend.
const FRIEND_BLAST: i64 = 48;
/// A moving target is aimed at again at most this often (Spring's half second).
const REAIM_TICKS: u64 = 15;
/// How far (angle units) the target must have moved from the last aim to be aimed at again.
const REAIM_DRIFT: i32 = 300;
/// A target this far (about 22°) from the last aim is aimed at again as soon as it answers.
const FAR_DRIFT: i32 = 4096;
/// An aim the script answered "no" (a Commander building) is asked again this often.
const RETRY_TICKS: u64 = 6;
/// An aim unanswered this long is given up on and asked for again.
const AIM_PATIENCE: u64 = 90;
const GRID_CELL: i64 = 128;

/// Most missiles a silo holds.
pub const MAX_STOCKPILE: u32 = 10;

/// A projectile in flight.
#[derive(Clone, Debug)]
pub struct Projectile {
    pub id: u32,
    pub weapon: usize,
    pub owner: UnitId,
    pub owner_player: u8,
    pub pos: FxVec3,
    pub prev_pos: FxVec3,
    pub vel: FxVec3,
    pub target: Option<Target>,
    pub target_pos: FxVec3,
    pub age: u32,
    pub ttl: u32,
    /// Units already damaged by a pass-through weapon.
    pub hits: Vec<UnitId>,
    pub dead: bool,
}

/// Uniform grid of unit slots for proximity queries, rebuilt every tick.
#[derive(Clone, Default)]
pub struct UnitGrid {
    width: i32,
    height: i32,
    cells: Vec<Vec<u32>>,
    /// Per cell, a [`player_bit`] for each player with a unit in it: target searches skip
    /// crowds with no enemy in them without looking at every unit.
    owners: Vec<u64>,
    /// Each unit slot's cell at the rebuild (`u32::MAX`: none), so that a unit changing sides
    /// in the tick is marked in its cell's owners at once (see [`UnitGrid::note_owner`]).
    slot_cells: Vec<u32>,
}

impl UnitGrid {
    pub fn new(world: &World) -> UnitGrid {
        let width = (world.width as i64 * CELL / GRID_CELL + 1) as i32;
        let height = (world.height as i64 * CELL / GRID_CELL + 1) as i32;
        UnitGrid { width, height, cells: vec![Vec::new(); (width * height) as usize], owners: vec![0; (width * height) as usize], slot_cells: Vec::new() }
    }

    fn cell_of(&self, p: FxVec3) -> (i32, i32) {
        (((p.x.to_int() / GRID_CELL) as i32).clamp(0, self.width - 1), ((p.z.to_int() / GRID_CELL) as i32).clamp(0, self.height - 1))
    }

    pub fn rebuild<'a>(&mut self, units: impl Iterator<Item = &'a Unit>) {
        for c in &mut self.cells {
            c.clear();
        }
        self.owners.clear();
        self.owners.resize(self.cells.len(), 0);
        self.slot_cells.fill(u32::MAX);
        for u in units {
            if u.dead {
                continue;
            }
            let (x, z) = self.cell_of(u.pos);
            let i = (z * self.width + x) as usize;
            self.cells[i].push(u.id.slot);
            self.owners[i] |= player_bit(u.owner);
            let slot = u.id.slot as usize;
            if self.slot_cells.len() <= slot {
                self.slot_cells.resize(slot + 1, u32::MAX);
            }
            self.slot_cells[slot] = i as u32;
        }
    }

    /// The unit in `slot` changed sides in the tick, to `owner`: its cell holds one of theirs.
    pub fn note_owner(&mut self, slot: u32, owner: u8) {
        if let Some(&cell) = self.slot_cells.get(slot as usize) && cell != u32::MAX {
            self.owners[cell as usize] |= player_bit(owner);
        }
    }

    /// Whether any unit of the `players` mask ([`player_bit`]s) is in the cells
    /// [`UnitGrid::query`] would look at.
    pub fn any_of(&self, p: FxVec3, r: Fx, players: u64) -> bool {
        let (x0, z0) = self.cell_of(p - FxVec3::new(r, Fx::ZERO, r));
        let (x1, z1) = self.cell_of(p + FxVec3::new(r, Fx::ZERO, r));
        (z0..=z1).any(|z| (x0..=x1).any(|x| self.owners.get((z * self.width + x) as usize).is_some_and(|&o| o & players != 0)))
    }

    /// The cells overlapping the square of half-size `r` around `p` (indices, added to `out`).
    fn cells_near(&self, p: FxVec3, r: Fx, out: &mut Vec<usize>) {
        let (x0, z0) = self.cell_of(p - FxVec3::new(r, Fx::ZERO, r));
        let (x1, z1) = self.cell_of(p + FxVec3::new(r, Fx::ZERO, r));
        for z in z0..=z1 {
            for x in x0..=x1 {
                out.push((z * self.width + x) as usize);
            }
        }
    }

    /// [`UnitGrid::query`] in the cells holding a unit of the `players` mask alone (a target
    /// search passes over a crowd of friends without looking at each).
    pub fn query_owned(&self, p: FxVec3, r: Fx, players: u64, out: &mut Vec<u32>) {
        out.clear();
        let (x0, z0) = self.cell_of(p - FxVec3::new(r, Fx::ZERO, r));
        let (x1, z1) = self.cell_of(p + FxVec3::new(r, Fx::ZERO, r));
        for z in z0..=z1 {
            for x in x0..=x1 {
                let i = (z * self.width + x) as usize;
                if self.owners[i] & players != 0 {
                    out.extend_from_slice(&self.cells[i]);
                }
            }
        }
    }

    /// Unit slots in cells overlapping the square of half-size `r` around `p`.
    pub fn query(&self, p: FxVec3, r: Fx, out: &mut Vec<u32>) {
        out.clear();
        let (x0, z0) = self.cell_of(p - FxVec3::new(r, Fx::ZERO, r));
        let (x1, z1) = self.cell_of(p + FxVec3::new(r, Fx::ZERO, r));
        for z in z0..=z1 {
            for x in x0..=x1 {
                out.extend_from_slice(&self.cells[(z * self.width + x) as usize]);
            }
        }
    }
}

/// Collision sphere of a unit: centre and radius.
fn hit_sphere(u: &Unit, sim: &Sim) -> (FxVec3, Fx) {
    let t = sim.types.get(u.type_id);
    let h = t.model.height.max(Fx::from_int(8));
    let r = t.collision_radius();
    // A surface hull intersects the water. Tall masts must not lift its entire
    // collision sphere above torpedo depth.
    let y = if t.locomotion == Locomotion::Naval && t.waterline.is_zero() { (h / 2).min(r / 2) } else { h / 2 };
    let center = u.model_pos(t) + FxVec3::new(Fx::ZERO, y, Fx::ZERO);
    (center, r)
}

/// Closest distance between segment a→b and point c.
fn segment_distance(a: FxVec3, b: FxVec3, c: FxVec3) -> Fx {
    let d = b - a;
    let len2 = d.dot(d);
    let t = if len2 <= Fx::ZERO { Fx::ZERO } else { ((c - a).dot(d) / len2).clamp(Fx::ZERO, Fx::ONE) };
    (a + d.scale(t)).distance(c)
}

/// How far along the segment `a`..`b` (0 to 1) it first comes within `r` of `c`: 0 if it starts
/// within, None if it never does.
fn segment_entry(a: FxVec3, b: FxVec3, c: FxVec3, r: Fx) -> Option<Fx> {
    if a.distance(c) <= r {
        return Some(Fx::ZERO);
    }
    let d = b - a;
    let len2 = d.dot(d);
    if len2 <= Fx::ZERO {
        return None;
    }
    // The line's closest approach, then back along it to where it crosses the sphere.
    let t = (c - a).dot(d) / len2;
    let off = (a + d.scale(t)).distance(c);
    if off > r {
        return None;
    }
    let entry = t - (r * r - off * off).sqrt() / len2.sqrt();
    (Fx::ZERO..=Fx::ONE).contains(&entry).then_some(entry)
}

/// First contact with the terrain along a tick's flight. The terrain is linear inside
/// each triangle: test the crossings of its x/z cell edges and x-z diagonals, then solve
/// the first crossing of the ground between them. Endpoint tests alone miss narrow ridges.
fn ground_entry(world: &World, a: FxVec3, b: FxVec3) -> Option<Fx> {
    // Map heights are bytes; most high-flying shells and missiles need no terrain walk.
    if a.y.min(b.y) > Fx::from_int(u8::MAX as i64) {
        return None;
    }
    let mut above = a.y - world.height_at(a.x, a.z);
    if above <= Fx::ZERO {
        return Some(Fx::ZERO);
    }
    let delta = b - a;
    // Only the segment inside the map matters. This also bounds work for unusually fast
    // weapons: an off-map endpoint must not walk arbitrarily far past the terrain.
    let (width, height) = world.size_px();
    let mut end = Fx::ONE;
    for (start, finish, length) in [(a.x, b.x, width), (a.z, b.z, height)] {
        if finish < Fx::ZERO {
            end = end.min((Fx::ZERO - start) / (finish - start));
        } else if finish > length {
            end = end.min((length - start) / (finish - start));
        }
    }
    let end = end.clamp(Fx::ZERO, Fx::ONE);
    let step = Fx::from_int(CELL);
    // (Coordinate at the start, its change, next triangle edge.)
    let mut edges = [(a.x, delta.x), (a.z, delta.z), (a.x - a.z, delta.x - delta.z)].map(|(start, change)| {
        let cell = start.0.div_euclid(step.0);
        let edge = Fx::from_int((cell + i64::from(change > Fx::ZERO)) * CELL);
        (start, change, edge)
    });
    let crossing = |(start, change, edge): (Fx, Fx, Fx)| if change.is_zero() { Fx::MAX } else { (edge - start) / change };
    let mut before = Fx::ZERO;
    loop {
        let next = edges.iter().copied().map(crossing).min().unwrap_or(end).clamp(before, end);
        if next > before {
            let p = a + delta.scale(next);
            let clearance = p.y - world.height_at(p.x, p.z);
            if clearance <= Fx::ZERO {
                return Some(before + (next - before) * (above / (above - clearance)));
            }
            above = clearance;
            before = next;
        }
        if next == end {
            return None;
        }
        for edge in &mut edges {
            if crossing(*edge) <= next {
                edge.2 += if edge.1 > Fx::ZERO { step } else { -step };
            }
        }
    }
}

impl Sim {
    /// Landed aircraft are ground targets until they take off.
    pub(crate) fn airborne(&self, u: &Unit) -> bool {
        !u.landed && self.types.get(u.type_id).locomotion == Locomotion::Air
    }

    /// Surface guns cannot operate while their unit is fully submerged. Amphibious
    /// movement does not make a weapon capable of firing underwater.
    fn weapon_can_fire_from(&self, u: &Unit, w: &WeaponType) -> bool {
        w.def.fire_submersed || !self.submerged(u)
    }

    /// A body actually in the water, including surface ships and wading units.
    pub fn in_water(&self, u: &Unit) -> bool {
        !u.dead && u.carried_by.is_none() && !self.airborne(u)
            && u.model_pos(self.unit_type(u)).y <= self.world.sea_level
            && self.world.depth_at(u.pos.x, u.pos.z) > Fx::ZERO
    }

    fn target_in_water(&self, target: Target) -> bool {
        match target {
            Target::Unit(id) => self.unit(id).is_some_and(|u| self.in_water(u)),
            Target::Ground(p) => p.y <= self.world.sea_level && self.world.depth_at(p.x, p.z) > Fx::ZERO,
            Target::Projectile(_) => false,
        }
    }

    /// Torpedoes intercept the wet part of a surface hull, not its deck in the air.
    fn weapon_target_point(&self, w: &WeaponType, target: Target) -> Option<FxVec3> {
        let mut p = self.target_point(target)?;
        if w.torpedo {
            if !self.target_in_water(target) { return None; }
            p.y = p.y.min(self.world.sea_level - Fx::from_int(2));
        }
        Some(p)
    }

    /// Unit orders may follow a visible or radar contact, never its hidden live position.
    pub(crate) fn target_known(&self, target: Target, player: u8) -> bool {
        match target {
            Target::Unit(id) => self.unit(id).is_some_and(|u| !u.dead && u.carried_by.is_none() && self.seen_by(u, player) != crate::vision::Seen::Hidden),
            Target::Ground(_) | Target::Projectile(_) => true,
        }
    }

    /// Target point of a target (unit centre or ground point).
    pub(crate) fn target_point(&self, t: Target) -> Option<FxVec3> {
        match t {
            Target::Unit(id) => self.unit(id).filter(|u| !u.dead).map(|u| hit_sphere(u, self).0),
            Target::Ground(p) => Some(p),
            Target::Projectile(pid) => self.projectiles.iter().find(|p| p.id == pid && !p.dead).map(|p| p.pos),
        }
    }

    fn target_radius(&self, t: Target) -> Fx {
        match t {
            Target::Unit(id) => self.unit(id).map_or(Fx::ZERO, |u| self.types.get(u.type_id).radius),
            Target::Ground(_) | Target::Projectile(_) => Fx::ZERO,
        }
    }

    /// Stockpile weapons (nukes, anti-nukes) build their missiles over the reload time,
    /// paying as they go like construction (and stalling with the economy).
    pub(crate) fn update_stockpile(&mut self, id: UnitId) {
        let types = self.types.clone();
        let weapons = self.weapon_types.clone();
        let Some(u) = self.unit(id) else { return };
        let t = types.get(u.type_id);
        let Some(w) = t.weapons.iter().flatten().map(|&w| weapons.get(w)).find(|w| w.def.stockpile) else { return };
        if u.stockpile >= MAX_STOCKPILE || !(u.activated || !t.def.on_offable) {
            return;
        }
        let owner = u.owner as usize;
        // A missile takes its reload time of full ticks (each a fraction of a tick by the stall
        // factor, kept whole in 16.16), and each tick pays what its share of the missile's price
        // is worth, so the missile costs exactly that price.
        let total = (w.reload_ticks.max(1) as u64) << crate::fixed::FRAC_BITS;
        let before = u.stockpile_work;
        let worth = |amount: Fx, work: u64| amount.share(before + work, total) - amount.share(before, total);
        let full = (Fx::ONE.0 as u64).min(total - before.min(total));
        let e = &mut self.players[owner].economy;
        e.metal_demand_next += worth(w.metal_per_shot, full);
        e.energy_demand_next += worth(w.energy_per_shot, full);
        let mut work = (e.build_factor.0.max(0) as u64).min(full);
        // Last tick's demand cannot predict a newly started silo or what another builder
        // has already spent. Limit work to the stores that remain, including rounding at
        // fractional progress, so an empty store never pays for free missile progress.
        if w.metal_per_shot > Fx::ZERO && e.metal <= Fx::ZERO || w.energy_per_shot > Fx::ZERO && e.energy <= Fx::ZERO {
            work = 0;
        } else if worth(w.metal_per_shot, work) > e.metal || worth(w.energy_per_shot, work) > e.energy {
            let (mut lo, mut hi) = (0, work);
            while lo < hi {
                let mid = lo + (hi - lo).div_ceil(2);
                if worth(w.metal_per_shot, mid) <= e.metal && worth(w.energy_per_shot, mid) <= e.energy {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            work = lo;
        }
        let (metal, energy) = (worth(w.metal_per_shot, work), worth(w.energy_per_shot, work));
        e.metal -= metal;
        e.energy -= energy;
        e.tick_metal_out += metal;
        e.tick_energy_out += energy;
        let Some(u) = self.unit_mut(id) else { return };
        u.stockpile_work = before + work;
        u.stockpile_progress = Fx::fraction(u.stockpile_work, total);
        if u.stockpile_work >= total {
            u.stockpile_work = 0;
            u.stockpile_progress = Fx::ZERO;
            u.stockpile += 1;
        }
    }

    /// Anti-missile defence: an interceptor with missiles in stock fires at enemy
    /// interceptable projectiles (nukes) headed into its coverage.
    pub(crate) fn update_interceptors(&mut self) {
        if !self.tick.is_multiple_of(5) || !self.projectiles.iter().any(|p| self.weapon_types.get(p.weapon).def.targetable) {
            return;
        }
        let types = self.types.clone();
        let weapons = self.weapon_types.clone();
        let defenders: Vec<(UnitId, usize)> = self
            .units
            .iter()
            .flatten()
            .filter(|u| !u.dead && u.is_complete() && self.tick >= u.paralyzed_until && u.carried_by.is_none())
            .flat_map(|u| {
                let t = types.get(u.type_id);
                let weapons = &weapons;
                (0..3).filter_map(move |slot| {
                    let w = weapons.get(t.weapons[slot]?);
                    (w.def.interceptor && (!w.def.stockpile || u.stockpile > 0)).then_some((u.id, slot))
                })
            })
            .collect();
        let mut last_defender = None;
        let mut assigned = rustc_hash::FxHashSet::default();
        for (id, slot) in defenders {
            if last_defender != Some(id) {
                assigned.clear();
                last_defender = Some(id);
            }
            let Some(u) = self.unit(id) else { continue };
            let t = types.get(u.type_id);
            let w = weapons.get(t.weapons[slot].unwrap());
            if u.weapons[slot].reload > 0 || !(u.activated || !t.def.on_offable) || !self.weapon_can_fire_from(u, w) {
                continue;
            }
            let coverage = w.interceptor_coverage();
            let (pos, owner) = (u.pos, u.owner);
            let current = u.weapons[slot].target;
            let threat = self
                .projectiles
                .iter()
                .filter(|p| !p.dead && self.is_enemy(p.owner_player, owner) && weapons.get(p.weapon).def.targetable && !self.intercepted.contains(&p.id) && !assigned.contains(&p.id))
                .filter(|p| p.target_pos.within_xz(pos, coverage) || p.pos.within_xz(pos, coverage))
                .min_by_key(|p| (current != Some(Target::Projectile(p.id)), (p.pos - pos).xz().length_sq_raw(), p.id))
                .map(|p| (p.id, p.pos));
            let Some((pid, ppos)) = threat else {
                if let Some(u) = self.unit_mut(id) { u.retarget(slot, None); }
                continue;
            };
            assigned.insert(pid);
            if let Some(u) = self.unit_mut(id) { u.retarget(slot, Some(Target::Projectile(pid))); }
            let from = self.aim_from_point(id, slot);
            let heading = wrap_angle(heading_of(ppos - from) - self.unit(id).unwrap().heading);
            let pitch = self.launch_pitch(w, ppos - from).unwrap_or(16384);
            if !self.aim_weapon(id, slot, heading, pitch) { continue; }
            let before = self.projectiles.len();
            self.fire(id, slot, w, Target::Projectile(pid), ppos, pitch);
            if self.projectiles.len() > before {
                self.intercepted.insert(pid);
                if let Some(u) = self.unit_mut(id) { u.retarget(slot, None); }
            }
        }
    }


    /// Weapon update for one unit: acquire targets, aim via script, fire.
    pub(crate) fn update_weapons(&mut self, id: UnitId, types: &crate::defs::UnitTypes, weapons: &crate::defs::WeaponTypes) {
        let Some(u) = self.unit(id) else { return };
        let t = types.get(u.type_id);
        if u.dead || !u.is_complete() {
            return;
        }
        let (forced, manual) = match u.orders.front() {
            Some(Order::Attack(tg)) => (Some(*tg), false),
            Some(Order::ManualFire(tg)) => (Some(*tg), true),
            _ => (None, false),
        };
        // A plane on patrol or attack-move: its bombs (command-fire in TA's data, otherwise
        // dropped only on an attack order) go on what it comes upon near its way.
        let roving = t.is_plane() && matches!(u.orders.front(), Some(Order::Patrol(_) | Order::Fight(_)));
        let fire_state = u.fire_state;
        let owner = u.owner;
        // Cloaking can happen after this unit's order phase but before its weapon phase.
        let forced = forced.filter(|&target| self.target_known(target, owner));
        // Nothing in its sights, no attack order and not its tick to look round: only the
        // reload and burst timers run down (as below, for most units on most ticks).
        let looking = fire_state >= 2 && (self.tick + id.slot as u64).is_multiple_of(RETARGET_TICKS);
        if forced.is_none() && !looking && u.weapons.iter().all(|ws| ws.target.is_none()) {
            let armed = t.weapons;
            if let Some(u) = self.unit_mut(id) {
                for (ws, _) in u.weapons.iter_mut().zip(armed).filter(|(_, w)| w.is_some()) {
                    ws.reload = ws.reload.saturating_sub(1);
                    ws.burst_timer = ws.burst_timer.saturating_sub(1);
                }
            }
            return;
        }
        let available = t.weapons.map(|w| w.is_some_and(|w| self.weapon_can_fire_from(u, weapons.get(w))));
        // Command-fire weapons (D-gun, bombs) fire on manual-fire orders, and on plain
        // attack orders when they are the primary weapon (bombers, the Phoenix's bombs
        // beside its laser) or nothing else the unit carries can hit the target; the
        // Commander's D-gun (third weapon) waits for manual fire.
        let flying = match forced {
            Some(Target::Unit(tid)) => self.unit(tid).is_some_and(|tu| self.airborne(tu)),
            _ => false,
        };
        let can_hit = |w: &WeaponType| if flying { w.hits_air } else { w.hits_ground };
        let has_normal = t.weapons.iter().enumerate().any(|(slot, w)| available[slot] && w.is_some_and(|w| !weapons.get(w).command_fire && can_hit(weapons.get(w))));
        let aircraft = t.locomotion == Locomotion::Air;

        // (Slots index the type's weapons, the unit's weapon states and the aim functions.)
        #[allow(clippy::needless_range_loop)]
        for slot in 0..3 {
            let Some(wid) = t.weapons[slot] else {
                // No weapon here: nothing may hold a target in this slot.
                if let Some(u) = self.unit_mut(id) {
                    u.retarget(slot, None);
                }
                continue;
            };
            let w = weapons.get(wid);
            // Reload and burst timers.
            if let Some(u) = self.unit_mut(id) {
                let ws = &mut u.weapons[slot];
                ws.reload = ws.reload.saturating_sub(1);
                ws.burst_timer = ws.burst_timer.saturating_sub(1);
            }
            // Interceptors are launched exclusively by update_interceptors. Neither an
            // automatic target, a retained target nor an explicit order may turn one into
            // a gun against ordinary units or the ground.
            if w.def.interceptor {
                let pending = self.unit(id).and_then(|u| u.weapons[slot].target);
                let valid = matches!(pending, Some(Target::Projectile(pid)) if !self.intercepted.contains(&pid)
                    && self.projectiles.iter().any(|p| p.id == pid && !p.dead && self.is_enemy(owner, p.owner_player) && weapons.get(p.weapon).def.targetable));
                if !valid && let Some(u) = self.unit_mut(id) {
                    u.retarget(slot, None);
                }
                continue;
            }
            // A roving bomber's bombs, with fire at will: they find their own targets near its way
            // (`Sim::bomber_lookout`) and keep them for its runs (not cleared after a drop; a
            // building through the turn out of its sight as well, `Sim::held_building`).
            let roving_bombs = roving && w.command_fire && w.trajectory == Trajectory::Dropped && !w.def.stockpile && fire_state >= 2;
            // Choose a target: forced (attack order) or automatic when allowed.
            let current = self.unit(id).and_then(|u| u.weapons[slot].target);
            // Hold fire: only what an attack order names (a target kept from before is dropped).
            let mut target = if !available[slot] {
                None
            } else if w.command_fire {
                forced
                    .filter(|_| manual || (slot == 0 || !has_normal) && can_hit(w))
                    .filter(|&tg| !w.torpedo || self.target_in_water(tg))
                    .or(current.filter(|&tg| roving_bombs && (self.target_valid(id, w, tg) || self.held_building(tg, owner) && self.within_reach(id, w, tg))))
            } else {
                forced.filter(|&tg| !w.torpedo || self.target_in_water(tg)).or(current.filter(|&tg| fire_state >= 1 && self.target_valid(id, w, tg)))
            };
            // Nothing to shoot, or only a bad target (taken on last): a look round on the weapon's
            // tick for a better one, which it turns to.
            let on_bad = match target {
                Some(Target::Unit(tid)) if forced != target => self.unit(tid).is_some_and(|tu| self.bad_target(t.id, slot, tu)) && !self.players.get(owner as usize).is_some_and(|p| p.shoot_all),
                _ => false,
            };
            if available[slot] && (target.is_none() || on_bad) && (!w.command_fire || roving_bombs) && fire_state >= 2 && (self.tick + id.slot as u64).is_multiple_of(RETARGET_TICKS) {
                let tick = self.tick;
                let skip = self.unit(id).and_then(|u| u.weapons[slot].blocked).filter(|&(_, until)| tick < until).map(|(t, _)| t);
                let reach = if roving_bombs { self.bomber_lookout(t).min(w.range) } else { w.range };
                let found = self.find_target(id, slot, w, owner, skip, reach);
                if target.is_none() || matches!(found, Some(Target::Unit(tid)) if self.unit(tid).is_some_and(|tu| !self.bad_target(t.id, slot, tu))) {
                    target = found;
                }
            }
            if target != current && let Some(u) = self.unit_mut(id) {
                u.retarget(slot, target);
            }
            // A target lost: turrets and arms go back to rest (the weapon's TA number).
            if target.is_none() && current.is_some() && t.callins.target_cleared {
                self.start_script(id, "TargetCleared", &[slot as i32 + 1]);
            }
            let Some(target) = target else {
                continue;
            };
            let Some(tp) = self.weapon_target_point(w, target) else {
                if let Some(u) = self.unit_mut(id) {
                    u.retarget(slot, None);
                }
                continue;
            };
            let from = self.aim_from_point(id, slot);
            let dist = from.distance_xz(tp);
            let reach = w.range + self.target_radius(target);
            if dist > reach + Fx::from_int(16) {
                continue;
            }
            // Heading relative to the body; pitch from a ballistic solution when needed.
            let to = tp - from;
            let heading = wrap_angle(heading_of(to) - self.unit(id).unwrap().heading);
            // Under the water only torpedoes and depth charges reach (an ordered target is
            // left to the order: the unit has nothing to hit it with).
            let under = matches!(target, Target::Unit(tid) if !w.water && self.unit(tid).is_some_and(|tu| self.submerged(tu)));
            let pitch = if under { None } else { self.launch_pitch(w, to) };
            let Some(pitch) = pitch else {
                // Out of the shells' reach (or under the water): an automatic target is passed
                // over for a while, for one it can hit.
                if forced != Some(target) && let Target::Unit(tid) = target {
                    let until = self.tick + BLOCKED_TICKS;
                    if let Some(u) = self.unit_mut(id) {
                        u.weapons[slot].blocked = Some((tid, until));
                        u.retarget(slot, None);
                    }
                }
                continue;
            };
            let aimed_now = self.aim_weapon(id, slot, heading, pitch);
            let ready = self.unit(id).is_some_and(|u| u.weapons[slot].reload == 0 || u.weapons[slot].burst_left > 0 && u.weapons[slot].burst_timer == 0);
            // Aircraft weapons fire forward: bombs when they would land on the target,
            // fixed guns and missiles when the nose points within the weapon's tolerance.
            let lined_up = !aircraft || w.def.turret || match w.trajectory {
                Trajectory::Dropped => self.bomb_on_target(id, from, tp, w, target),
                _ => heading.abs() <= if w.tolerance > 0 { w.tolerance } else { 3000 },
            };
            if aimed_now && ready && dist <= reach && lined_up {
                // A shot the ground would stop: automatic targeting passes over that target
                // for a while (another may be in the open); an ordered target is fired at
                // regardless, as in TA.
                if forced != Some(target) && !self.line_of_fire_clear(from, tp, w, pitch) {
                    if let Target::Unit(tid) = target {
                        let until = self.tick + BLOCKED_TICKS;
                        if let Some(u) = self.unit_mut(id) {
                            u.weapons[slot].blocked = Some((tid, until));
                            u.retarget(slot, None);
                        }
                    }
                    continue;
                }
                // Nor a shot that would hurt its own side: a friend in the way, or under a big
                // blast. An automatic target is passed over for one in the clear; an attack
                // order waits for a clear shot. What the player fires deliberately goes as
                // ordered: the D-gun and other manual-fire weapons (they take what they touch,
                // as in TA), shots at the ground, at one's own units, at missiles.
                let deliberate = w.command_fire && !(roving_bombs && forced != Some(target))
                    || matches!(target, Target::Ground(_) | Target::Projectile(_))
                    || matches!(target, Target::Unit(tid) if self.unit(tid).is_some_and(|tu| self.allied(tu.owner, owner)));
                if !deliberate && !self.clear_of_friends(id, slot, owner, from, tp, w, pitch, target) {
                    if forced != Some(target) && let Target::Unit(tid) = target {
                        let until = self.tick + BLOCKED_TICKS;
                        if let Some(u) = self.unit_mut(id) {
                            u.weapons[slot].blocked = Some((tid, until));
                            u.retarget(slot, None);
                        }
                    }
                    continue;
                }
                let fired_before = self.projectiles.len();
                self.fire(id, slot, w, target, tp, pitch);
                let mut completed_order = false;
                if self.projectiles.len() > fired_before && let Some(u) = self.unit_mut(id) {
                    // Aimed again for the next shot.
                    u.weapons[slot].aim_asked = false;
                    // Manual fire and a stockpiled launch each spend one shot.
                    // A silo's Attack must not keep launching at the same place.
                    // (A roving bomber keeps the target it found for the rest of its run.)
                    if (w.command_fire || w.def.stockpile) && !(roving_bombs && forced != Some(target)) {
                        if matches!(u.orders.front(), Some(Order::ManualFire(_)))
                            || w.def.stockpile && matches!(u.orders.front(), Some(Order::Attack(_)))
                        {
                            u.orders.pop_front();
                            completed_order = true;
                        }
                        u.retarget(slot, None);
                    }
                }
                if completed_order { self.halt(id); }
            }
        }
    }

    /// Ask the weapon's script to aim and wait for its newest readiness answer. Shared
    /// by ordinary weapons and missile defence, including deployment delays/refusals.
    /// Pending deployment finishes before another aim is started; the script's own
    /// signals cancel older threads, and only the newest answer updates readiness.
    fn aim_weapon(&mut self, id: UnitId, slot: usize, heading: i32, pitch: i32) -> bool {
        let Some(u) = self.unit(id) else { return false };
        let tag = ThreadTag::Aim(slot as u16);
        let has_aim = u.cob.as_ref().is_some_and(|c| c.has_function(AIM_FUNCS[slot]));
        let tick = self.tick;
        let Some(u) = self.unit_mut(id) else { return false };
        match u.cob.as_mut().filter(|_| has_aim) {
            // No script to ask: always aimed.
            None => true,
            Some(c) => {
                // The newest aim's answer, when it comes.
                if let Some(r) = c.take_result(tag) {
                    u.weapons[slot].aimed = r.value != 0;
                }
                let ws = u.weapons[slot];
                let drift = wrap_angle(heading - ws.aim_heading).abs().max(wrap_angle(pitch - ws.aim_pitch).abs());
                let since = tick.saturating_sub(ws.last_aim_tick);
                let newest = c.is_running(tag);
                let ask = if newest || c.is_running(ThreadTag::OldAim(slot as u16)) {
                    // An aim unanswered for long (its target gone, or waiting on something
                    // that never comes): asked again anyway.
                    since >= AIM_PATIENCE && (!newest || drift > REAIM_DRIFT || !ws.aimed)
                } else {
                    !ws.aim_asked || drift > FAR_DRIFT || drift > REAIM_DRIFT && since >= REAIM_TICKS || !ws.aimed && since >= RETRY_TICKS
                };
                if ask {
                    c.supersede_aims(slot as u16);
                    // (As TA hands them, 0..65535: see `script_angle`.)
                    c.start(AIM_FUNCS[slot], &[script_angle(heading), script_angle(pitch)], tag);
                    let ws = &mut u.weapons[slot];
                    ws.aim_heading = heading;
                    ws.aim_pitch = pitch;
                    ws.last_aim_tick = tick;
                    ws.aim_asked = true;
                }
                ws.aimed
            }
        }
    }

    /// Whether a bomb released now would land on the target (fall time from the drop
    /// height, carried forward at the aircraft's speed).
    fn bomb_on_target(&self, id: UnitId, from: FxVec3, tp: FxVec3, w: &WeaponType, target: Target) -> bool {
        let Some(u) = self.unit(id) else { return false };
        // Bombs burst on the water surface over submerged targets.
        let height = (from.y - tp.y.max(self.world.sea_level)).max(Fx::ONE);
        let fall = (height * 2 / self.gravity).sqrt();
        let travel = heading_vector(u.heading).scale(u.speed * fall);
        let impact = FxVec3::new(from.x + travel.x, tp.y, from.z + travel.z);
        impact.within_xz(tp, w.aoe_radius.max(Fx::from_int(24)) + self.target_radius(target) + Fx::from_int(32))
    }

    /// How far past the target a plane flies before turning back: enough to come round
    /// (turn radius) and line up again, plus the lead a falling bomb needs.
    fn pass_overshoot(&self, t: &crate::defs::UnitType) -> Fx {
        // Turn radius = speed / angular rate (65536 angle units per turn).
        let radius = t.max_speed * 10430 / t.turn_rate.max(1) as i64;
        let bombs = t.weapons.iter().flatten().any(|&w| self.weapon_types.get(w).trajectory == Trajectory::Dropped);
        let lead = if bombs { t.max_speed * (t.cruise_alt * 2 / self.gravity).sqrt() } else { Fx::ZERO };
        (lead + radius * 3 / 2 + Fx::from_int(100)).clamp(Fx::from_int(200), Fx::from_int(1600))
    }

    /// Plane attack runs. Approach: steer straight at the target (bombs and forward guns
    /// fire on the way in), a plane of a group from its place across the group's front, coming
    /// in on the target from there ([`Unit::fan`]: [`FAN_SPACING`] apart, drawn in over the
    /// last stretch). Once over it, run out straight for the overshoot distance, then turn back
    /// for the next approach.
    pub(crate) fn air_attack_pass(&mut self, id: UnitId, tp: FxVec3) {
        let Some(u) = self.unit(id) else { return };
        // (Its place across the front, from where it is now: wide far off, none over the target.)
        let aim = if u.fan == 0 {
            tp
        } else {
            let to = (tp - u.pos).xz();
            let dist = to.length_xz();
            if dist <= Fx::ONE {
                tp
            } else {
                let side = FxVec3::new(-to.z / dist.to_int().max(1), Fx::ZERO, to.x / dist.to_int().max(1));
                let drawn_in = ((dist - Fx::from_int(160)) / Fx::from_int(640)).clamp(Fx::ZERO, Fx::ONE);
                let (w, h) = self.world.size_px();
                let p = tp + side.scale(FAN_SPACING * u.fan as i64 * drawn_in);
                FxVec3::new(p.x.clamp(Fx::ZERO, w), tp.y, p.z.clamp(Fx::ZERO, h))
            }
        };
        let overshoot = self.pass_overshoot(self.types.get(u.type_id));
        let (w, h) = self.world.size_px();
        let forward = heading_vector(u.heading);
        let (pos, runout, goal, ticks) = (u.pos, u.pass_runout && u.air_pass, u.path_goal, u.pass_ticks);
        let (next_goal, next_runout) = if runout {
            let finished = goal.is_none_or(|g| pos.within_xz(g, Fx::from_int(48))) || ticks > 300;
            if finished { (tp, false) } else { (goal.unwrap(), true) }
        } else {
            let to = (tp - pos).xz();
            let ahead = to.x * forward.x + to.z * forward.z;
            let over = to.length_xz() < Fx::from_int(24) || ahead < Fx::ZERO && to.length_xz() < overshoot / 2;
            if over {
                let end = pos + forward.scale(overshoot);
                (FxVec3::new(end.x.clamp(Fx::ZERO, w), tp.y, end.z.clamp(Fx::ZERO, h)), true)
            } else {
                (aim, false)
            }
        };
        if let Some(u) = self.unit_mut(id) {
            if next_runout != u.pass_runout || !u.air_pass {
                u.pass_ticks = 0;
            } else {
                u.pass_ticks += 1;
            }
            u.path.clear();
            u.path_goal = Some(next_goal);
            u.move_request = Some(next_goal);
            u.air_pass = true;
            u.pass_runout = next_runout;
        }
    }

    /// A maneuvering fighter answers a contact with an attack run and returns to its
    /// original position through the existing Fight order's engagement/arrival rules.
    pub(crate) fn idle_plane_response(&mut self, id: UnitId, t: &crate::defs::UnitType) {
        let Some(u) = self.unit(id) else { return };
        if !u.orders.is_empty() || u.move_state == 0 || u.fire_state == 0 { return; }
        // (Not after a bad target, which it takes on only when nothing else is near: as units do
        // not chase one.)
        let engaged = u.weapons.iter().zip(t.weapons).enumerate().any(|(slot, (state, weapon))| {
            weapon.is_some_and(|w| {
                let w = self.weapon_types.get(w);
                state.target.is_some_and(|target| self.target_valid(id, w, target) && matches!(target, Target::Unit(tid) if self.unit(tid).is_some_and(|tu| !self.bad_target(t.id, slot, tu))))
            })
        });
        if engaged && let Some(u) = self.unit_mut(id) { u.orders.push_back(Order::Fight(u.pos)); }
    }

    /// Whether `target` is one of the bad targets of `u`'s weapon in `slot`: of a category it
    /// takes on last (`wpri_`/`wsec_`/`wspe_badTargetCategory`, else the unit's
    /// `BadTargetCategory`): only when no other enemy is in reach, as TA's missile towers fire
    /// on tanks with no aircraft about, and its destroyers' depth charges on an amphibious tank
    /// under the water.
    pub(crate) fn bad_target(&self, shooter_type: usize, slot: usize, target: &Unit) -> bool {
        self.types.get(target.type_id).category_mask.intersects(&self.types.get(shooter_type).weapon_bad_masks[slot])
    }

    /// Whether the weapon in `slot` of a unit of `shooter_type` can take `target` on of its own
    /// accord, wherever they stand: it hits that kind (an aircraft in flight; what is on the
    /// ground or the water), and reaches it (only `waterweapon`s under the water, torpedoes only
    /// what is in it). Its bad targets too, last ([`Sim::bad_target`]). (Range, sight, the ground
    /// between and the shooter's own state apart: what automatic targeting, kept targets and
    /// return fire ask.)
    pub(crate) fn can_engage(&self, w: &WeaponType, target: &Unit) -> bool {
        (if self.airborne(target) { w.hits_air } else { w.hits_ground })
            && !(!w.water && self.submerged(target) || w.torpedo && !self.in_water(target))
    }

    /// Whether the weapon in `slot` takes `target` on first ([`Sim::can_engage`], and not one of
    /// its bad targets): what a unit chases and a computer player's hunters go after.
    pub(crate) fn engages(&self, shooter_type: usize, slot: usize, w: &WeaponType, target: &Unit) -> bool {
        self.can_engage(w, target) && !self.bad_target(shooter_type, slot, target)
    }

    /// How far a bomber on patrol or attack-move looks for targets its side sees: past where its
    /// bombs must be let go to land on one (their fall from its cruising height at its top
    /// speed), so that it can line up on one ahead; at least as far as it sees.
    fn bomber_lookout(&self, t: &crate::defs::UnitType) -> Fx {
        let lead = t.max_speed * (t.cruise_alt * 2 / self.gravity).sqrt();
        t.sight.max(lead + Fx::from_int(BOMBER_LOOKOUT_PAST_LEAD))
    }

    /// An enemy building, not cloaked: a bomber that found it keeps it as its target when it
    /// goes out of sight in the turn for the next run (it cannot have moved), as it does what is
    /// in sight or on radar ([`Sim::target_known`]).
    pub(crate) fn held_building(&self, target: Target, owner: u8) -> bool {
        matches!(target, Target::Unit(id) if self.unit(id).is_some_and(|tu| {
            !tu.dead && tu.carried_by.is_none() && !tu.cloaked && self.is_enemy(owner, tu.owner) && self.types.get(tu.type_id).locomotion == Locomotion::Static
        }))
    }

    /// Whether `target` is something the weapon hits and within its reach of the unit (as
    /// [`Sim::target_valid`] measures it), seen or not.
    fn within_reach(&self, id: UnitId, w: &WeaponType, target: Target) -> bool {
        let Target::Unit(tid) = target else { return false };
        let (Some(u), Some(tu)) = (self.unit(id), self.unit(tid)) else { return false };
        self.can_engage(w, tu) && u.pos.within_xz(tu.pos, w.range + Fx::from_int(48))
    }

    /// Whether an automatic target is still worth shooting by the weapon in `slot`. Ground is
    /// never one: only an attack order names ground, and once the order is gone (Stop, another
    /// order) so is it. Nor is one gone out of its side's sight, a radar or sonar contact now:
    /// only an order fires on those. (A bad target is, until a better one comes into reach: see
    /// `update_weapons`.)
    fn target_valid(&self, id: UnitId, w: &WeaponType, t: Target) -> bool {
        match t {
            Target::Ground(_) => false,
            Target::Projectile(_) => false,
            Target::Unit(tid) => {
                let (Some(u), Some(tu)) = (self.unit(id), self.unit(tid)) else { return false };
                if tu.dead || tu.carried_by.is_some() || !self.is_enemy(u.owner, tu.owner) || self.seen_by(tu, u.owner) != crate::vision::Seen::Visible {
                    return false;
                }
                self.can_engage(w, tu) && u.pos.within_xz(tu.pos, w.range + Fx::from_int(48))
            }
        }
    }

    /// The nearest enemy the weapon in `weapon_slot` can engage, its bad targets last
    /// ([`Sim::bad_target`]: TA's anti-air units list NOTAIR and pick a tank only with no
    /// aircraft about, its other armed units VTOL and an aircraft only with nothing on the ground
    /// in reach; with the owner's `+shootall` they count as any other), for a computer player's
    /// unit a mission's priority targets first, and among those the one that has just hit the
    /// unit. Computer players' units never pick what a mission tells them to ignore. Only what
    /// its side sees: a radar or sonar contact in the fog is fired on by order alone, as players
    /// asked (long-range guns kept firing at will on radar alone, never spotted, past what their
    /// cost was balanced for). `reach`: how far it looks (the weapon's range, a roving bomber's
    /// sight).
    fn find_target(&mut self, id: UnitId, weapon_slot: usize, w: &WeaponType, owner: u8, skip: Option<UnitId>, reach: Fx) -> Option<Target> {
        let u = self.unit(id)?;
        let pos = u.pos;
        let bad_mask = self.types.get(u.type_id).weapon_bad_masks[weapon_slot];
        let attacker = u.last_attacker.filter(|_| self.tick <= u.last_hit_tick + 2 * TICKS_PER_SECOND as u64);
        // (Shells: from about where the gun stands, to the target's middle.)
        let gun_y = u.model_pos(self.unit_type(u)).y + self.unit_type(u).model.height / 2;
        // The units in the cells within reach holding an enemy (a crowd of friends is passed
        // over unlooked at).
        let mut candidates = std::mem::take(&mut self.scratch);
        self.grid.query_owned(pos, reach + self.types.target_reach, self.enemy_mask(owner), &mut candidates);
        if candidates.is_empty() {
            self.scratch = candidates;
            return None;
        }
        let ai = self.mission.is_some() && self.players.get(owner as usize).is_some_and(|p| p.ai);
        let shoot_all = self.players.get(owner as usize).is_some_and(|p| p.shoot_all);
        let mut best: Option<(bool, bool, bool, i128, UnitId)> = None;
        for &slot in &candidates {
            let Some(o) = self.units.get(slot as usize).and_then(|u| u.as_ref()) else { continue };
            // TA measures range horizontally to the target's edge (first: most of those in
            // the cells round it are out of range).
            let ot = self.types.get(o.type_id);
            let d = (o.pos - pos).xz().length_sq_raw();
            let r = (reach + ot.radius).0 as i128;
            if d > r * r || o.dead || o.carried_by.is_some() || !self.is_enemy(owner, o.owner) || self.no_auto_target.contains(&o.type_id) || skip == Some(o.id) || ai && self.ai_ignores(o.id) {
                continue;
            }
            let bad = !shoot_all && ot.category_mask.intersects(&bad_mask);
            let key = (bad, !(ai && self.is_priority_target(o.id)), attacker != Some(o.id), d, o.id);
            // Eligibility is read-only: a candidate that cannot beat the current winner
            // needs no visibility, submersion or ballistic checks.
            if best.is_some_and(|b| key >= b) { continue; }
            if self.seen_by(o, owner) != crate::vision::Seen::Visible { continue; }
            // (What it hits, only torpedoes and depth charges under the water.)
            if !self.can_engage(w, o) {
                continue;
            }
            // A shell must also carry that far (to the target's middle).
            if w.trajectory != Trajectory::Ballistic || self.shell_carries(w, o.pos - pos, hit_sphere(o, self).0.y - gun_y) {
                best = Some(key);
            }
        }
        self.scratch = candidates;
        best.map(|(.., t)| Target::Unit(t))
    }

    /// Point weapons aim from: the `AimFrom*` piece, else the unit's centre.
    fn aim_from_point(&mut self, id: UnitId, slot: usize) -> FxVec3 {
        // Scripts return a fixed piece here; cache it per unit type and weapon slot.
        let type_id = self.unit(id).map_or(0, |u| u.type_id);
        let piece = match self.aim_from_cache.get(&(type_id, slot as u8)) {
            Some(&p) => p,
            None => {
                let p = self.call_script(id, AIM_FROM_FUNCS[slot], &[0]).and_then(|p| p.first().copied());
                self.aim_from_cache.insert((type_id, slot as u8), p);
                p
            }
        };
        let u = self.unit(id).unwrap();
        let t = self.types.get(u.type_id);
        piece
            .and_then(|p| {
                let cob = u.cob.as_ref()?;
                let mp = cob.program.piece_map.get(usize::try_from(p).ok()?).copied().flatten()?;
                Some(piece_world_position(u, t, Some(cob), mp))
            })
            .unwrap_or(u.model_pos(t) + FxVec3::new(Fx::ZERO, t.model.height / 2, Fx::ZERO))
    }

    /// The model piece a unit's weapon in `slot` aims from, as its script answered the first
    /// time the weapon aimed (none before that). Perspective puts the eye by it.
    pub fn aim_from_piece(&self, u: &Unit, slot: usize) -> Option<usize> {
        let p = (*self.aim_from_cache.get(&(u.type_id, slot as u8))?)?;
        let cob = u.cob.as_ref()?;
        cob.program.piece_map.get(usize::try_from(p).ok()?).copied().flatten()
    }

    /// Whether the ground lets a shot from `from` reach `tp`: straight shots along the line,
    /// shells along their arc (guided missiles and bombs find their own way). Checked every
    /// 16 px short of the last 24 (the target may stand on a slope).
    fn line_of_fire_clear(&self, from: FxVec3, tp: FxVec3, w: &WeaponType, pitch: i32) -> bool {
        let to = tp - from;
        let dist = to.length_xz();
        let (step, margin) = (Fx::from_int(16), Fx::from_int(24));
        if dist <= step + margin {
            return true;
        }
        let (sn, cs) = (crate::fixed::sin(pitch), crate::fixed::cos(pitch));
        // Height of the shot once it has gone `s` across.
        let height = |s: Fx| match w.trajectory {
            Trajectory::Straight => Some(from.y + to.y * (s / dist)),
            // y = y0 + s tan θ − g s² / (2 v² cos² θ)
            Trajectory::Ballistic if cs > Fx::from_ratio(1, 20) && w.velocity > Fx::ZERO => {
                Some(from.y + s * sn / cs - self.gravity * s * s / (Fx::from_int(2) * w.velocity * w.velocity * cs * cs))
            }
            _ => None,
        };
        let mut s = step;
        while s < dist - margin {
            let Some(y) = height(s) else { return true };
            let f = s / dist;
            if self.world.height_at(from.x + to.x * f, from.z + to.z * f) > y + Fx::from_int(2) {
                return false;
            }
            s += step;
        }
        true
    }

    /// Whether a shot of `w` at `target` is clear of the shooter's side ([`Sim::friend_in_the_way`]),
    /// looked at again every [`FRIEND_CHECK_TICKS`].
    #[allow(clippy::too_many_arguments)]
    fn clear_of_friends(&mut self, id: UnitId, slot: usize, owner: u8, from: FxVec3, tp: FxVec3, w: &WeaponType, pitch: i32, target: Target) -> bool {
        let tick = self.tick;
        if let Some((_, _, clear)) = self.unit(id).and_then(|u| u.weapons[slot].friend_check).filter(|&(t, until, _)| t == target && tick < until) {
            return clear;
        }
        let clear = !self.friend_in_the_way(id, owner, from, tp, w, pitch, target);
        if let Some(u) = self.unit_mut(id) {
            u.weapons[slot].friend_check = Some((target, tick + FRIEND_CHECK_TICKS, clear));
        }
        clear
    }

    /// Whether a shot of `w` from `from` at `tp` would hurt one of `owner`'s side: a friend
    /// (not the shooter or the target, nor a plane in flight) in the way of a straight shot or a
    /// missile, or under the low parts of a shell's arc (high over the ground it passes over
    /// everything), or within half a big blast's width of where it comes down. Bombs, dropped
    /// from above, only by their blast. Only the grid's cells with a friend in them are looked
    /// at.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn friend_in_the_way(&self, id: UnitId, owner: u8, from: FxVec3, tp: FxVec3, w: &WeaponType, pitch: i32, target: Target) -> bool {
        let friends = (0..self.players.len() as u8).filter(|&p| self.allied(owner, p)).fold(0u64, |m, p| m | player_bit(p));
        let target_id = if let Target::Unit(t) = target { Some(t) } else { None };
        let pad = Fx::from_int(FRIEND_PAD);
        let blast = (w.aoe_radius >= Fx::from_int(FRIEND_BLAST)).then(|| w.aoe_radius / 2);
        let to = tp - from;
        let dist = to.length_xz();
        let path = w.trajectory != Trajectory::Dropped && dist > Fx::from_int(16);
        let (sn, cs) = (crate::fixed::sin(pitch), crate::fixed::cos(pitch));
        let ballistic = w.trajectory == Trajectory::Ballistic && cs > Fx::from_ratio(1, 20) && w.velocity > Fx::ZERO;
        // A shell's height once it has gone `s` across.
        let shell = |s: Fx| from.y + s * sn / cs - self.gravity * s * s / (Fx::from_int(2) * w.velocity * w.velocity * cs * cs);
        let mut cells: Vec<usize> = Vec::new();
        if let Some(r) = blast {
            self.grid.cells_near(tp, r + pad, &mut cells);
        }
        if path {
            let step = Fx::from_int(32);
            let mut s = Fx::ZERO;
            while s <= dist {
                let p = from + to.scale(s / dist);
                // (Where a shell is high over the ground, nothing below is in its way.)
                if !ballistic || shell(s) - self.world.height_at(p.x, p.z) <= Fx::from_int(FRIEND_TOP) {
                    self.grid.cells_near(p, pad, &mut cells);
                }
                s += step;
            }
        }
        cells.sort_unstable();
        cells.dedup();
        for &cell in &cells {
            if self.grid.owners[cell] & friends == 0 {
                continue;
            }
            for &slot in &self.grid.cells[cell] {
                let Some(u) = self.units.get(slot as usize).and_then(|u| u.as_ref()) else { continue };
                if u.dead || u.id == id || Some(u.id) == target_id || u.carried_by.is_some() || !self.allied(u.owner, owner) {
                    continue;
                }
                if !u.landed && self.types.get(u.type_id).locomotion == Locomotion::Air {
                    continue;
                }
                let (c, r) = hit_sphere(u, self);
                if blast.is_some_and(|b| c.distance(tp) <= b + r) {
                    return true;
                }
                if !path {
                    continue;
                }
                if ballistic {
                    // Under the arc: across the line between the two (not behind the shooter
                    // or past the target), within reach of it sideways, and the shell at its
                    // height there.
                    let rel = c - from;
                    let along = (rel.x * to.x + rel.z * to.z) / dist;
                    if along <= Fx::ZERO || along >= dist {
                        continue;
                    }
                    let side = (rel.x * to.z - rel.z * to.x).abs() / dist;
                    if side <= r && (shell(along) - c.y).abs() <= r {
                        return true;
                    }
                } else {
                    // On the line between the two (not beside the muzzle or past the target).
                    let len2 = to.dot(to);
                    if len2 <= Fx::ZERO {
                        continue;
                    }
                    let k = (c - from).dot(to) / len2;
                    if k <= Fx::ZERO || k >= Fx::ONE {
                        continue;
                    }
                    if (from + to.scale(k)).distance(c) <= r {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// How far across a weapon's shot carries to a point `dy` above where it leaves: its range,
    /// or less for a shell that gravity brings down first (v² / g on level ground: the
    /// Guardian's, Punisher's and the cruisers' and battleships' shells carry about 1,094 at
    /// TA's gravity of 112, their range is 1,200 to 1,300).
    pub(crate) fn weapon_reach(&self, w: &WeaponType, dy: Fx) -> Fx {
        if w.trajectory != Trajectory::Ballistic {
            return w.range;
        }
        // Farthest x with a solution: v⁴ − g (g x² + 2 dy v²) ≥ 0.
        let (v2, g) = (w.velocity * w.velocity, self.gravity.max(Fx::EPSILON));
        let r = v2 * v2 - Fx::from_int(2) * g * dy * v2;
        if r <= Fx::ZERO {
            return Fx::ZERO;
        }
        (r.sqrt() / g).min(w.range)
    }

    /// Whether a shell of `w` carries across `to` (its horizontal part) to a point `dy` above
    /// where it leaves: [`Sim::weapon_reach`] without the square root (target searches ask it
    /// of every candidate).
    fn shell_carries(&self, w: &WeaponType, to: FxVec3, dy: Fx) -> bool {
        let (v2, g) = (w.velocity * w.velocity, self.gravity.max(Fx::EPSILON));
        let d2 = to.x * to.x + to.z * to.z;
        g * g * d2 <= v2 * v2 - Fx::from_int(2) * g * dy * v2
    }

    /// Launch pitch (TA angle units, positive = up) to reach `to`, or None if out of reach.
    fn launch_pitch(&self, w: &WeaponType, to: FxVec3) -> Option<i32> {
        let dx = to.length_xz().max(Fx::ONE);
        let dy = to.y;
        match w.trajectory {
            Trajectory::Ballistic => {
                // Low-arc solution: tan θ = (v² - sqrt(v⁴ - g(g x² + 2 y v²))) / (g x).
                let v = w.velocity;
                let g = self.gravity;
                let v2 = v * v;
                let disc = v2 * v2 - g * (g * dx * dx + Fx::from_int(2) * dy * v2);
                if disc < Fx::ZERO {
                    return None;
                }
                let tan = (v2 - disc.sqrt()) / (g * dx);
                Some(atan2(tan, Fx::ONE))
            }
            Trajectory::Dropped => Some(0),
            _ => Some(atan2(dy, dx)),
        }
    }

    /// What a shot is sent at, and the point it makes for: as aimed, but a missile that strikes a
    /// place ([`WeaponType::strikes_a_place`]: TA's nukes) aimed at a unit is sent at the ground
    /// under it now (the sea's surface over water), and goes there wherever the unit goes after.
    fn launch_target(&self, w: &WeaponType, target: Target, tp: FxVec3) -> (Target, FxVec3) {
        match target {
            Target::Unit(_) if w.strikes_a_place() => {
                let place = FxVec3::new(tp.x, self.world.height_at(tp.x, tp.z).max(self.world.sea_level), tp.z);
                (Target::Ground(place), place)
            }
            _ => (target, tp),
        }
    }

    fn fire(&mut self, id: UnitId, slot: usize, w: &WeaponType, target: Target, tp: FxVec3, pitch: i32) {
        let Some(owner) = self.unit(id).map(|u| u.owner) else { return };
        if w.def.interceptor && !matches!(target, Target::Projectile(pid) if self.projectiles.iter().any(|p| p.id == pid && !p.dead && self.is_enemy(owner, p.owner_player) && self.weapon_types.get(p.weapon).def.targetable)) {
            return;
        }
        let (target, tp) = self.launch_target(w, target, tp);
        if w.def.stockpile {
            // Stockpiled missiles were paid for while being built.
            let Some(u) = self.unit_mut(id) else { return };
            if u.stockpile == 0 {
                return;
            }
            u.stockpile -= 1;
        } else {
            // Shot costs (the D-gun uses energy).
            let energy = self.shot_energy(id, w);
            let e = &mut self.players[owner as usize].economy;
            if e.energy < energy || e.metal < w.metal_per_shot {
                return;
            }
            e.energy -= energy;
            e.metal -= w.metal_per_shot;
            e.tick_energy_out += energy;
            e.tick_metal_out += w.metal_per_shot;
        }
        let muzzle_piece = self.call_script(id, QUERY_FUNCS[slot], &[0]).and_then(|p| p.first().copied());
        self.start_script(id, FIRE_FUNCS[slot], &[]);
        // The recoil rocks the hull away from the shot, in the unit's own frame (TA's
        // RockUnit: tilts about the x and z axes, 500 at most).
        if let Some(u) = self.unit(id)
            && self.types.get(u.type_id).callins.rock_unit
        {
            let away = wrap_angle(heading_of(tp - u.pos) - u.heading + 32768);
            let rock = [(cos(away) * 500).to_int() as i32, (sin(away) * 500).to_int() as i32];
            self.start_script(id, "RockUnit", &rock);
        }
        let u = self.unit(id).unwrap();
        let t = self.types.get(u.type_id);
        let muzzle = muzzle_piece
            .and_then(|p| {
                let cob = u.cob.as_ref()?;
                let mp = cob.program.piece_map.get(usize::try_from(p).ok()?).copied().flatten()?;
                Some(piece_world_position(u, t, Some(cob), mp))
            })
            .unwrap_or(u.model_pos(t) + FxVec3::new(Fx::ZERO, t.model.height / 2, Fx::ZERO));
        let (unit_heading, unit_speed) = (u.heading, u.speed);
        // Re-solve the launch angle from the actual muzzle (the aim-from piece differs).
        let pitch = self.launch_pitch(w, tp - muzzle).unwrap_or(pitch);

        // Direction: heading to the target plus accuracy error; launch pitch as solved.
        // Veterans aim better (up to 60% less scatter).
        let vet = self.unit(id).map_or(Fx::ZERO, |u| u.veterancy());
        let spread = ((Fx::from_int(w.spread as i64 / 4) * (Fx::ONE - vet * 6 / 10)).to_int()) as i32;
        let err_h = if spread > 0 { self.rng.range_i32(-spread, spread) } else { 0 };
        let err_p = if spread > 0 { self.rng.range_i32(-spread / 2, spread / 2) } else { 0 };
        let heading = heading_of(tp - muzzle) + err_h;
        let dir = match w.trajectory {
            Trajectory::Dropped => FxVec3::ZERO,
            _ => rotate_y(rotate_x(FxVec3::new(Fx::ZERO, Fx::ZERO, Fx::ONE), -(pitch + err_p)), heading),
        };
        let speed = if w.trajectory == Trajectory::Guided && w.start_velocity > Fx::ZERO { w.start_velocity } else { w.velocity };
        let mut vel = dir.scale(speed);
        if w.vlaunch {
            // Straight up out of its tube at its start speed, none for most: it gathers speed as
            // it climbs (`weaponacceleration`). At full speed through the whole climb a Merl's
            // rocket rose 2,000 px, out of sight, and came down short of its target as its fuel
            // ran out; a nuke climbed 1,750 px, an anti-nuke 3,200.
            let rise = if w.acceleration > Fx::ZERO { w.start_velocity } else { speed };
            vel = FxVec3::new(Fx::ZERO, rise, Fx::ZERO);
        }
        if w.trajectory == Trajectory::Dropped {
            // Bombs inherit the aircraft's horizontal velocity.
            vel = heading_vector(unit_heading).scale(unit_speed);
        }
        let pid = self.next_projectile_id;
        self.next_projectile_id = self.next_projectile_id.wrapping_add(1);
        self.projectiles.push(Projectile {
            id: pid,
            weapon: w.id,
            owner: id,
            owner_player: owner,
            pos: muzzle,
            prev_pos: muzzle,
            vel,
            target: Some(target),
            target_pos: tp,
            age: 0,
            ttl: w.flight_ticks,
            hits: Vec::new(),
            dead: false,
        });
        self.events.push(SimEvent::WeaponFired { unit: id, weapon: w.id, pos: muzzle, target: tp });
        // Firing gives a cloaked unit away for a moment.
        let tick = self.tick;
        if let Some(u) = self.unit_mut(id) && u.cloak_on {
            u.decloak_until = tick + 45;
        }
        if let Some(u) = self.unit_mut(id) {
            let ws = &mut u.weapons[slot];
            if ws.burst_left == 0 {
                ws.burst_left = w.burst.saturating_sub(1);
                // Stockpile reload is the missile's build time; launches only need the
                // silo to cycle.
                let reload = if w.def.stockpile { w.reload_ticks.min(90) } else { w.reload_ticks };
                // Veterans reload up to 30% faster.
                ws.reload = (Fx::from_int(reload as i64) * (Fx::ONE - vet * 3 / 10)).to_int().max(1) as u32;
            } else {
                ws.burst_left -= 1;
            }
            ws.burst_timer = w.burst_ticks;
        }
    }

    /// Area damage at `pos`. `direct` takes full damage. Only torpedoes and depth charges
    /// reach what is under the water; `spare_flyers` (a plane's death blast in the air) leaves
    /// aircraft in flight alone, and `spare_side` (flames) the units of that player and its
    /// allies.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn explode(&mut self, w: &WeaponType, pos: FxVec3, source: UnitId, direct: Option<UnitId>, water: bool, spare_flyers: bool, spare_side: Option<u8>) {
        let player = self.unit(source).map(|u| u.owner);
        self.explode_at(w, pos, source, player, direct, water, spare_flyers, spare_side, Fx::ZERO);
    }

    /// [`Sim::explode`], `when` (0 to 1) through the tick: where a projectile met what it hit.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn explode_at(&mut self, w: &WeaponType, pos: FxVec3, source: UnitId, source_player: Option<u8>, direct: Option<UnitId>, water: bool, spare_flyers: bool, spare_side: Option<u8>, when: Fx) {
        self.events.push(SimEvent::Explosion { weapon: w.id, pos, water, when, player: source_player });
        let r = w.aoe_radius;
        let mut near = std::mem::take(&mut self.scratch);
        self.grid.query(pos, r + self.types.collision_reach, &mut near);
        let mut victims: Vec<(UnitId, Fx)> = Vec::new();
        for &slot in &near {
            let Some(u) = self.units.get(slot as usize).and_then(|u| u.as_ref()) else { continue };
            if u.dead || u.carried_by.is_some()
                || !w.water && self.submerged(u)
                || spare_flyers && !u.landed && self.types.get(u.type_id).locomotion == Locomotion::Air
                || spare_side.is_some_and(|p| self.allied(u.owner, p))
                || source_player.is_some_and(|p| self.ceasefire_spares_side(p, u.owner))
            {
                continue;
            }
            let base = w.damage_to(u.type_id);
            if Some(u.id) == direct {
                victims.push((u.id, base));
                continue;
            }
            let (c, ur) = hit_sphere(u, self);
            let d = (c.distance(pos) - ur).max(Fx::ZERO);
            if d >= r {
                continue;
            }
            // Spring/TA falloff: (r - d) / (r - d · edge).
            let m = ((r - d) / (r - d * w.edge_effectiveness).max(Fx::EPSILON)).clamp(Fx::ZERO, Fx::ONE);
            victims.push((u.id, base * m));
        }
        self.scratch = near;
        if w.paralyzer {
            for (id, amount) in victims {
                self.paralyze_unit(id, amount, source);
            }
            return;
        }
        for (id, dmg) in victims {
            let dir = self.unit(id).map(|u| u.pos - pos).unwrap_or(FxVec3::ZERO);
            self.damage_unit(id, dmg, source, dir);
        }
        // Scorch/knock over trees and other destructible features.
        self.damage_features(pos, r, w.default_damage, w.def.fire_starter);
    }

    /// Paralyzer (EMP) hits stun instead of damaging. TA's damage figure is the stun time
    /// at 60 per second (the Spider's 900 is "15 seconds of paralysis"); a hit extends the
    /// stun to at least its own length, never past a minute. Aircraft and
    /// `ImmuneToParalyzer` units shrug it off.
    pub(crate) fn paralyze_unit(&mut self, id: UnitId, amount: Fx, source: UnitId) {
        // (Not an enemy's in a ceasefire.)
        if self.ceasefire_spares(source, id) {
            return;
        }
        let ticks = (amount * TICKS_PER_SECOND / 60).to_int().max(0) as u64;
        if ticks == 0 {
            return;
        }
        let tick = self.tick;
        let Some(u) = self.unit(id) else { return };
        let t = self.types.get(u.type_id);
        if u.dead || t.def.immune_to_paralyzer || t.locomotion == Locomotion::Air {
            return;
        }
        if let Some(u) = self.unit_mut(id) {
            u.paralyzed_until = u.paralyzed_until.max(tick + ticks).min(tick + 60 * TICKS_PER_SECOND as u64);
            u.last_attacker = Some(source);
            u.last_hit_tick = tick;
        }
    }

    pub(crate) fn damage_unit(&mut self, id: UnitId, damage: Fx, source: UnitId, dir: FxVec3) {
        if damage <= Fx::ZERO {
            return;
        }
        // Cheats: double or half damage, god mode; then the Commanders' perks; then the
        // mutators (a ceasefire: no harm to an enemy at all; how tough units are).
        let damage = self.cheat_damage(damage, source, id, self.unit(id).map_or(0, |u| u.owner));
        let damage = self.perk_damage(damage, source, id);
        if self.ceasefire_spares(source, id) {
            return;
        }
        let damage = self.mutator_damage(damage);
        let types = self.types.clone();
        let source_owner = self.unit(source).map(|s| s.owner);
        let tick = self.tick;
        let Some(u) = self.unit_mut(id) else { return };
        if u.dead {
            return;
        }
        let (owner, pos) = (u.owner, u.pos);
        let t = types.get(u.type_id);
        let dmg = if u.armored { damage * Fx::from_f64(t.def.damage_modifier) } else { damage };
        // ...veterans shrug off up to a third of it.
        let dmg = dmg * (Fx::ONE - u.veterancy() * 3 / 10);
        u.health -= dmg;
        u.last_attacker = Some(source);
        u.last_hit_tick = tick;
        u.last_damage = dmg;
        // HitByWeapon(x, z) as RockUnit's: 500 × the direction the hit pushes the unit, in its
        // own frame (x from its z component, z from its x). The scripts jolt their hull by
        // that many angle units (about 3°); read as angles of up to 45°, hits tipped tanks
        // half over.
        let local = rotate_y(dir, -u.heading);
        let flat = local.length_xz();
        let (ax, az) = if flat > Fx::ZERO { (((local.z * 500) / flat).to_int() as i32, ((local.x * 500) / flat).to_int() as i32) } else { (0, 0) };
        let killed = u.health <= Fx::ZERO;
        if killed {
            u.dead = true;
            u.killer = Some(source);
        }
        // Tell the owner, now and then, that the enemy is shooting at them.
        if source_owner.is_none_or(|s| self.is_enemy(s, owner)) && let Some(p) = self.players.get_mut(owner as usize) && p.last_alert.is_none_or(|t| tick >= t + 30 * 10) {
            p.last_alert = Some(tick);
            self.events.push(SimEvent::UnderAttack { player: owner, unit: id, pos });
        }
        self.start_script(id, "HitByWeapon", &[ax, az]);
        // Return fire at an enemy when allowed: only with real, automatic weapons that can hit
        // the attacker and for which it is no bad target (an AA K-bot shot at by a tank does not
        // turn on it, as in TA) (a target on an empty slot would never clear, leaving the unit
        // forever "busy"; damaged aircraft then never landed), and only once finished (an
        // unfinished tower's weapons never look at a target, which it kept for good). A weapon
        // takes it on when free, or from a target of its own choosing that can't fight back,
        // the attacker in reach (a Commander burnt by two Pyros used to finish off the solar
        // collector it had picked first).
        let Some(attacker) = self.unit(source).filter(|a| !a.dead && self.is_enemy(a.owner, owner)) else { return };
        let (apos, at) = (attacker.pos, self.types.get(attacker.type_id));
        let Some(u) = self.unit(id) else { return };
        let ordered = match u.orders.front() {
            Some(Order::Attack(tg) | Order::ManualFire(tg)) => Some(*tg),
            _ => None,
        };
        let harmless = |tg: Target| match tg {
            Target::Unit(tid) => ordered != Some(tg) && self.unit(tid).is_none_or(|tu| self.types.get(tu.type_id).weapons.iter().all(|w| w.is_none())),
            _ => false,
        };
        let slots: Vec<usize> = (0..3)
            .filter(|&s| {
                t.weapons[s].is_some_and(|w| {
                    let w = self.weapon_types.get(w);
                    !w.command_fire && !w.def.stockpile && self.weapon_can_fire_from(u, w) && self.can_engage(w, attacker) && u.weapons[s].target.is_none_or(|tg| harmless(tg) && u.pos.within_xz(apos, w.range + at.radius))
                })
            })
            .collect();
        if let Some(u) = self.unit_mut(id) && u.fire_state >= 1 && source != id && u.is_complete() {
            for s in slots {
                u.retarget(s, Some(Target::Unit(source)));
            }
        }
    }

    /// Resolves deaths: `Killed` script, wreckage, death explosion (which may kill more).
    pub(crate) fn process_deaths(&mut self) {
        let types = self.types.clone();
        let weapons = self.weapon_types.clone();
        let mut rounds = 0;
        let (mut dying, mut jobs) = (Vec::new(), Vec::new());
        loop {
            // One look at every unit a round: who is dying, and who is building or mending what
            // (the builders of the dying stop). Nothing a death sets off starts or ends a job but
            // the stopping itself, so the jobs stand as they are found here.
            dying.clear();
            jobs.clear();
            for u in self.units.iter().flatten() {
                if u.dead && !u.death_processed {
                    dying.push(u.id);
                }
                if let Some(b) = u.build {
                    jobs.push((b.target, u.id));
                }
            }
            if dying.is_empty() || rounds > 8 {
                break;
            }
            rounds += 1;
            // By target; each target's builders still in slot order.
            jobs.sort_by_key(|&(target, _)| target);
            for &id in &dying {
                let Some(u) = self.unit_mut(id) else { continue };
                u.death_processed = true;
                let type_id = u.type_id;
                let pos = u.pos;
                let heading = u.heading;
                let killer = u.killer;
                let last_damage = u.last_damage;
                let complete = u.is_complete();
                let self_destructed = u.self_destruct_manual;
                let t = types.get(type_id);
                // A plane shot down in the air: its blast is a ground unit's (TA gives aircraft
                // the same SMALL_UNITEX and BIG_UNITEX as tanks: shells, which never engage
                // aircraft) and spares the planes flying beside it (a fighter group's losses
                // used to chain through it).
                let in_flight = t.locomotion == Locomotion::Air && !u.landed;
                // Experience: a kill is worth the victim's cost relative to the killer's.
                let value = |t: &crate::defs::UnitType| (t.cost_metal + t.cost_energy / 60).max(Fx::ONE);
                let victim_value = value(t);
                let victim_owner = self.unit(id).map_or(255, |u| u.owner);
                let killer_value = killer.and_then(|k| self.unit(k)).filter(|k| self.is_enemy(k.owner, victim_owner)).map(|k| value(types.get(k.type_id)));
                if let (Some(_), Some(owner)) = (killer_value, killer.and_then(|k| self.unit(k)).map(|k| k.owner)) && let Some(p) = self.players.get_mut(owner as usize) {
                    p.stats.kills += 1;
                }
                if let (Some(kv), Some(k)) = (killer_value, killer) {
                    let xp = self.perk_experience(k, (victim_value / kv).clamp(Fx::from_ratio(1, 10), Fx::from_int(3)));
                    if let Some(k) = self.unit_mut(k) {
                        k.kills += 1;
                        k.xp += xp;
                    }
                }
                // Killed(severity, corpsetype): scripts explode pieces and pick the wreck.
                // Severity is the killing blow as a percentage of max health, so units worn
                // down by small arms leave wrecks while huge explosions leave nothing.
                let severity = ((last_damage * Fx::from_int(100)) / t.max_health.max(Fx::ONE)).to_int().clamp(0, 100) as i32;
                let corpse_type = self.call_script(id, "Killed", &[severity, 0]).and_then(|p| p.get(1).copied()).unwrap_or(1);
                if complete {
                    self.spawn_wreck(&t.def.corpse, corpse_type, pos, heading, id);
                }
                // Its blast, `ExplodeAs` (`SelfDestructAs` when it blew itself up) by name. A
                // name no archive has (the Leveler's MEDIUM_UNITEX, the moho metal makers'
                // BIG_BUILDINGEX) is no blast, as no name (walls) is: what TA keeps for none is
                // its weapon 0, NOWEAPON, of no damage (see docs/FORMATS.md). The client draws a
                // burst.
                let boom = match (complete, self_destructed) {
                    (false, _) => "",
                    (true, true) if !t.def.self_destruct_as.is_empty() => &t.def.self_destruct_as,
                    (true, _) => &t.def.explode_as,
                };
                if let Some(w) = weapons.by_name(boom) {
                    self.explode(w, pos + FxVec3::new(Fx::ZERO, Fx::from_int(4), Fx::ZERO), id, None, false, in_flight, None);
                } else {
                    self.events.push(SimEvent::UnitExploded { id, pos, big: t.footprint.0 * t.footprint.1 >= 9 });
                }
                // Builders working on this unit stop; factories release it.
                self.stop_builders_of(id, &jobs);
                // A factory's units still leaving its yard are on their own now.
                let yard = self.unit_mut(id).map(|u| std::mem::take(&mut u.yard)).unwrap_or_default();
                for y in yard {
                    if let Some(yu) = self.unit_mut(y)
                        && yu.factory == Some(id)
                    {
                        yu.factory = None;
                    }
                }
            }
        }
    }

    /// The unit being built or repaired died: its builders stop (`jobs`: (target, builder)
    /// by target). A mobile builder's build order ends with its nanoframe, as in TA (it would
    /// otherwise lay a fresh frame at once); factories keep their queue and start over.
    fn stop_builders_of(&mut self, target: UnitId, jobs: &[(UnitId, UnitId)]) {
        let from = jobs.partition_point(|&(t, _)| t < target);
        let builders = jobs[from..].iter().take_while(|&&(t, _)| t == target).map(|&(_, b)| b);
        for b in builders {
            let factory = self.unit(b).is_some_and(|u| self.types.get(u.type_id).is_factory);
            if let Some(u) = self.unit_mut(b) {
                u.build = None;
                if !factory && matches!(u.orders.front(), Some(Order::Build { .. })) {
                    u.orders.pop_front();
                }
            }
            self.start_script(b, "StopBuilding", &[]);
        }
    }

    /// Places the corpse feature of unit `of`, dead (`corpse_type` 1 = wreck, 2 = heap, 3 =
    /// nothing).
    fn spawn_wreck(&mut self, corpse: &str, corpse_type: i32, pos: FxVec3, heading: i32, of: UnitId) {
        if corpse.is_empty() || corpse_type >= 3 {
            return;
        }
        let Some(mut def_id) = self.data.feature_id(corpse) else { return };
        if corpse_type == 2 {
            match self.data.feature_id(&self.data.features[def_id].feature_dead) {
                Some(heap) => def_id = heap,
                None => return,
            }
        }
        let def = &self.data.features[def_id];
        let fp = (def.footprint_x, def.footprint_z);
        let blocking = def.blocking;
        let anchor = World::pos_to_anchor(pos, fp);
        let origin = World::footprint_origin(anchor, fp);
        let slot = self.features.len() as u32;
        if blocking {
            self.world.occupy(Occupant::Feature(slot), origin, fp, &[]);
        }
        let mut p = pos;
        p.y = self.world.height_at(p.x, p.z);
        let health = Fx::from_f64(def.damage.max(1.0));
        self.add_feature(Feature::new(def_id, origin, p, heading, health));
        self.events.push(SimEvent::FeatureCreated { index: slot as usize, wreck_of: Some(of) });
        // Units beside the dead that the wreck falls on are thrown clear of it (they stood in it
        // for good: the crowd's pushing can't move a unit out of an obstacle). A cell or two,
        // their orders untouched: in a big battle, sending each off it and back made thousands
        // of routes again.
        if blocking {
            let inside = |x: i32, z: i32| x >= origin.0 && z >= origin.1 && x < origin.0 + fp.0 as i32 && z < origin.1 + fp.1 as i32;
            for u in self.units_on_cells(origin, fp, inside) {
                self.throw_clear(u);
            }
        }
    }

    /// Applies explosion damage to destructible features: flammable ones (trees) may catch
    /// fire instead, with the weapon's `firestarter` percent chance; the others take the
    /// damage and, destroyed, leave their `featuredead` (a smudge, rubble).
    fn damage_features(&mut self, pos: FxVec3, r: Fx, damage: Fx, fire_starter: f64) {
        let r = r.max(Fx::from_int(12));
        let ignite = fire_starter.clamp(0.0, 100.0) as u32;
        // (In index order, as they were once all looked at: the chances of catching fire are
        // drawn in that order. What a destroyed one leaves is not hit by the same blast.)
        let mut near = Vec::new();
        self.feature_grid.near(pos, r + Fx::from_int(8), &mut near);
        for i in near {
            let i = i as usize;
            let f = &self.features[i];
            if !f.alive || !f.pos.within_xz(pos, r + Fx::from_int(8)) {
                continue;
            }
            self.hit_feature(i, damage, ignite);
        }
    }

    /// A piercing shot (TA's `noexplode`: the Commanders' D-gun) over the features in its way:
    /// those within its blast's reach of its flight this tick, measured to where they stand,
    /// are hit as by its blast. (What a destroyed one leaves lies in its way still, and is hit
    /// as the shot passes on: a wreck it crosses goes, heap and all.)
    fn sweep_features(&mut self, from: FxVec3, to: FxVec3, w: &WeaponType) {
        let r = w.aoe_radius.max(Fx::from_int(12)) + Fx::from_int(8);
        let ignite = w.def.fire_starter.clamp(0.0, 100.0) as u32;
        let mut near = Vec::new();
        self.feature_grid.near(from + (to - from).scale(Fx::HALF), (to - from).length_xz() / 2 + r, &mut near);
        for i in near {
            let i = i as usize;
            let f = &self.features[i];
            if !f.alive || segment_distance(from, to, f.pos) > r {
                continue;
            }
            self.hit_feature(i, w.default_damage, ignite);
        }
    }

    /// A blast or a piercing shot reaches a feature: a flammable one (a tree) may catch fire
    /// instead, with `ignite` percent chance; the others take the damage and, destroyed, leave
    /// their `featuredead`.
    fn hit_feature(&mut self, i: usize, damage: Fx, ignite: u32) {
        let f = &self.features[i];
        let def = &self.data.features[f.def];
        if def.indestructible || f.burn_until != 0 {
            // Burning features burn down on their own.
            return;
        }
        if def.flamable && ignite > 0 && self.rng.chance(ignite, 100) {
            self.ignite(i);
            return;
        }
        let f = &mut self.features[i];
        f.health -= damage;
        if f.health <= Fx::ZERO {
            let next = self.data.features[f.def].feature_dead.clone();
            self.replace_feature(i, &next);
        }
    }

    /// Removes a feature, leaving `successor` (when it names one) in its place.
    fn replace_feature(&mut self, i: usize, successor: &str) {
        let f = &mut self.features[i];
        f.alive = false;
        f.burn_until = 0;
        let (origin, fdef, fpos, heading) = (f.origin, f.def, f.pos, f.heading);
        let def = &self.data.features[fdef];
        self.world.release(Occupant::Feature(i as u32), origin, (def.footprint_x, def.footprint_z));
        self.events.push(SimEvent::FeatureDestroyed { index: i, pos: fpos });
        if let Some(nid) = self.data.feature_id(successor) {
            let nd = &self.data.features[nid];
            let slot = self.features.len() as u32;
            if nd.blocking {
                self.world.occupy(Occupant::Feature(slot), origin, (nd.footprint_x, nd.footprint_z), &[]);
            }
            let health = Fx::from_f64(nd.damage.max(1.0));
            self.add_feature(Feature::new(nid, origin, fpos, heading, health));
            self.events.push(SimEvent::FeatureCreated { index: slot as usize, wreck_of: None });
        }
    }

    /// Features on fire right now (indices into `features`).
    pub fn burning_features(&self) -> &[usize] {
        &self.fires
    }

    /// Puts every fire out: what burns stands as it was, unburnt (the menu's film between its
    /// scenes, so no fire spreads on through a world while nobody looks).
    pub fn extinguish_fires(&mut self) {
        for i in std::mem::take(&mut self.fires) {
            if let Some(f) = self.features.get_mut(i) {
                f.burn_until = 0;
            }
        }
    }

    /// Sets a flammable feature on fire for `burnmin`..`burnmax` seconds.
    pub fn ignite(&mut self, i: usize) {
        let def = &self.data.features[self.features[i].def];
        let (lo, hi) = (def.burn_min.max(1.0) as i32, def.burn_max.max(def.burn_min).max(1.0) as i32);
        let spark = (def.spark_time.max(1.0) as u64).saturating_mul(TICKS_PER_SECOND as u64);
        let secs = self.rng.range_i32(lo, hi) as u64;
        let tick = self.tick;
        let f = &mut self.features[i];
        f.burn_until = tick + secs * TICKS_PER_SECOND as u64;
        f.next_spark = tick + spark;
        let pos = f.pos;
        self.fires.push(i);
        self.events.push(SimEvent::FeatureIgnited { index: i, pos });
    }

    /// Fires: every `sparktime` seconds a burning feature may set flammable neighbours
    /// alight (`spreadchance` percent each) and scorches the units around it with its
    /// `burnweapon`; when its time is up it becomes its `featureburnt` (a charred tree).
    pub(crate) fn update_fires(&mut self) {
        if self.fires.is_empty() {
            return;
        }
        let tick = self.tick;
        let burning = std::mem::take(&mut self.fires);
        let mut still = Vec::with_capacity(burning.len());
        for i in burning {
            let f = &self.features[i];
            if !f.alive || f.burn_until == 0 {
                continue; // reclaimed or destroyed meanwhile
            }
            let (pos, until, spark, def_id) = (f.pos, f.burn_until, f.next_spark, f.def);
            if tick >= until {
                let burnt = self.data.features[def_id].feature_burnt.clone();
                self.replace_feature(i, &burnt);
                continue;
            }
            if tick >= spark {
                let def = &self.data.features[def_id];
                let (chance, every, weapon) = (def.spread_chance.clamp(0.0, 100.0) as u32, (def.spark_time.max(1.0) as u64).saturating_mul(TICKS_PER_SECOND as u64), self.data.weapon_id(&def.burn_weapon));
                self.features[i].next_spark = tick.saturating_add(every);
                let mut near = Vec::new();
                self.feature_grid.near(pos, Fx::from_int(40), &mut near);
                near.retain(|&j| {
                    let g = &self.features[j as usize];
                    j as usize != i && g.alive && g.burn_until == 0 && g.pos.within_xz(pos, Fx::from_int(40)) && self.data.features[g.def].flamable
                });
                for j in near {
                    let j = j as usize;
                    if self.rng.chance(chance, 100) {
                        self.ignite(j);
                    }
                }
                if let Some(w) = weapon {
                    self.scorch_units(w, pos);
                }
            }
            still.push(i);
        }
        // Fires that caught during the loop were pushed onto `self.fires`.
        still.append(&mut self.fires);
        self.fires = still;
    }

    /// Area damage to units only (a burning tree's heat), with the weapon's falloff.
    fn scorch_units(&mut self, weapon: usize, pos: FxVec3) {
        let w = self.weapon_types.types[weapon].clone();
        let r = w.aoe_radius.max(Fx::from_int(16));
        let mut near = std::mem::take(&mut self.scratch);
        self.grid.query(pos, r + Fx::from_int(32), &mut near);
        let mut victims = Vec::new();
        for &slot in &near {
            let Some(u) = self.units.get(slot as usize).and_then(|u| u.as_ref()) else { continue };
            if u.dead || self.types.get(u.type_id).locomotion == crate::defs::Locomotion::Air {
                continue;
            }
            let d = u.pos.distance_xz(pos);
            if d >= r {
                continue;
            }
            let m = ((r - d) / (r - d * w.edge_effectiveness).max(Fx::EPSILON)).clamp(Fx::ZERO, Fx::ONE);
            victims.push((u.id, w.damage_to(u.type_id) * m));
        }
        self.scratch = near;
        // Nobody's weapon: no one gets the kill.
        let nobody = UnitId { slot: u32::MAX, generation: 0 };
        for (id, dmg) in victims {
            self.damage_unit(id, dmg, nobody, FxVec3::ZERO);
        }
    }

    /// Attack-move: advance on `dest`, but halt while any weapon has a target in range.
    pub(crate) fn update_fight_order(&mut self, id: UnitId, t: &crate::defs::UnitType, dest: FxVec3) {
        // In a formation: to its place there first while it forms up (and waits there), and all
        // the way to its place (not a unit's reach short of it, a row off).
        let (goal, forming) = self.formation_goal(id, dest);
        let near = self.in_formation(id, dest).then_some(Fx::from_int(8));
        if self.advance_engaging(id, t, goal, near) && !forming && let Some(u) = self.unit_mut(id) {
            u.orders.pop_front();
            u.move_request = None;
            self.arrived(id, dest);
        }
    }

    /// Moves towards `dest`, pausing while any weapon has a target in range. Returns true
    /// once the unit has arrived (or reached the closest reachable point).
    /// `near`: at most this close counts as there (a short patrol leg's third).
    pub(crate) fn advance_engaging(&mut self, id: UnitId, t: &crate::defs::UnitType, dest: FxVec3, near: Option<Fx>) -> bool {
        let Some(u) = self.unit(id) else { return false };
        if t.is_plane() {
            let engaged = u
                .weapons
                .iter()
                .zip(t.weapons.iter())
                .find_map(|(ws, w)| w.and(ws.target))
                .filter(|&tg| self.target_known(tg, u.owner) || self.held_building(tg, u.owner))
                .and_then(|tg| self.target_point(tg));
            if let Some(tp) = engaged {
                self.air_attack_pass(id, tp);
                return false;
            }
            if u.air_pass {
                if let Some(u) = self.unit_mut(id) {
                    u.air_pass = false;
                    u.path_goal = None;
                    u.move_request = None;
                }
                return false;
            }
        }
        let Some(u) = self.unit(id) else { return false };
        let there = near.map_or(t.radius * 3 + Fx::from_int(24), |n| n.min(t.radius * 3 + Fx::from_int(24)).max(Fx::from_int(8)));
        if u.pos.within_xz(dest, there) {
            self.halt(id);
            return true;
        }
        let engaged = u.weapons.iter().zip(t.weapons.iter()).any(|(ws, w)| {
            w.is_some_and(|wid| {
                let weapon = self.weapon_types.get(wid);
                self.weapon_can_fire_from(u, weapon) && ws.target.is_some_and(|tg| self.target_known(tg, u.owner) && self.target_point(tg).is_some_and(|p| u.pos.within_xz(p, weapon.range + Fx::from_int(24))))
            })
        });
        let planned = u.move_request == Some(dest);
        let arrived = planned && u.path_goal.is_none();
        if engaged {
            if u.path_goal.is_some() {
                self.halt(id);
            }
        } else if arrived {
            // Reached the closest reachable point.
            return true;
        } else if !planned {
            self.move_to_point(id, dest);
        }
        false
    }

    /// Order handling for attack orders: close to weapon range.
    pub(crate) fn update_attack_order(&mut self, id: UnitId, target: Target) {
        let types = self.types.clone();
        let weapons = self.weapon_types.clone();
        let Some(owner) = self.unit(id).map(|u| u.owner) else { return };
        let Some(tp) = self.target_point(target).filter(|_| self.target_known(target, owner)) else {
            // Contact lost: end this order and stop the pursuit. Queued orders remain.
            self.halt(id);
            if let Some(u) = self.unit_mut(id) {
                u.orders.pop_front();
                for slot in 0..u.weapons.len() {
                    if u.weapons[slot].target == Some(target) {
                        u.retarget(slot, None);
                    }
                }
            }
            return;
        };
        let u = self.unit(id).unwrap();
        let t = types.get(u.type_id);
        if t.weapons.iter().flatten().next().is_some() && t.weapons.iter().flatten().all(|&w| weapons.get(w).torpedo) && !self.target_in_water(target) {
            self.halt(id);
            if let Some(u) = self.unit_mut(id) {
                u.orders.pop_front();
                for slot in 0..u.weapons.len() { u.retarget(slot, None); }
            }
            return;
        }
        // How near to go: the reach of the weapons this order fires, as `update_weapons` has
        // them (a Commander's D-gun waits for manual fire: going by its 240 it stopped beyond
        // its laser's 200 from a mine; a shell's carry where it is less than its range), else
        // the farthest any of them reaches.
        let manual = matches!(u.orders.front(), Some(Order::ManualFire(_)));
        let flying = match target {
            Target::Unit(tid) => self.unit(tid).is_some_and(|tu| self.airborne(tu)),
            _ => false,
        };
        let can_hit = |w: &WeaponType| self.weapon_can_fire_from(u, w) && (!w.torpedo || self.target_in_water(target)) && if flying { w.hits_air } else { w.hits_ground };
        let has_normal = t.weapons.iter().flatten().any(|&w| !weapons.get(w).command_fire && can_hit(weapons.get(w)));
        let fires = |slot: usize, w: &WeaponType| if w.command_fire { (manual || slot == 0 || !has_normal) && can_hit(w) } else { !manual && can_hit(w) };
        let dy = tp.y - (u.model_pos(t).y + t.model.height / 2);
        let reaches = (0..3).filter_map(|s| t.weapons[s].map(|w| (s, weapons.get(w))));
        let available = reaches.clone().filter(|&(_, w)| self.weapon_can_fire_from(u, w));
        let submerged_guns = reaches.clone().next().is_some() && available.clone().next().is_none();
        let firing = available.clone().filter(|&(s, w)| fires(s, w)).map(|(_, w)| self.weapon_reach(w, dy)).max().filter(|r| !r.is_zero());
        let range = firing.or_else(|| available.map(|(_, w)| w.range).max()).unwrap_or(Fx::ZERO);
        if range.is_zero() && (t.def.kamikaze || submerged_guns) && t.locomotion != Locomotion::Static {
            // Crawling bombs approach for their proximity check. A submerged tank keeps
            // approaching too: it must reach the shore before its surface gun can fire.
            if u.path_goal.is_none() || u.stuck_ticks > 30 || self.tick % 30 == id.slot as u64 % 30 {
                self.move_to_point(id, tp);
            }
            return;
        }
        if range.is_zero() || t.locomotion == Locomotion::Static {
            if range.is_zero() && let Some(u) = self.unit_mut(id) {
                u.orders.pop_front();
            }
            return;
        }
        if t.is_plane() {
            self.air_attack_pass(id, tp);
            return;
        }
        let dist = u.pos.distance_xz(tp);
        if dist > range * 9 / 10 {
            if u.path_goal.is_none() || u.stuck_ticks > 30 || self.tick % 30 == id.slot as u64 % 30 {
                self.move_to_point(id, tp);
            }
        } else if u.path_goal.is_some() {
            self.halt(id);
        }
    }
}
