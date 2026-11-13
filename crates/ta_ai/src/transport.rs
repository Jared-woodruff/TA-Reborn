//! Transports: when the ground army cannot get at the enemy (island maps, an enemy cut off
//! by water or cliffs), transports carry it across. Air transports go anywhere; hover and
//! naval transports only where their movement class does: they take on units they can come
//! alongside and set them down where they can get close to the shore. So those are built
//! where they suit (their class gets from home to the enemy, or the sea does) when no air
//! transport is on offer or air lifts have lately been shot down.
//!
//! A lift starts once transports and passengers are ready: with nothing known over there, an
//! air transport first flies over the enemy for a look; then the transports pick up stranded
//! units together (the most valuable first), gather, go over together (one at a time they
//! are picked off one by one) to a landing zone near a weakly defended enemy target, away
//! from anti-aircraft cover and towers, set their loads down and come back for more, while
//! the units set down fight their way on over there. A lift that loses too many transports
//! is called off. Transports and passengers are told apart by their properties (capacity,
//! what a transport can carry, where a unit can go), never by name, so mods work.

use ta_sim::world::{CELL, MoveSpec, Region};

use super::*;

/// How a transport gets about: flying, or over the ground and water of its movement class
/// (the part of the map `region` it is in), picking units up within `load` cells of where it
/// can go and setting them down within `unload` cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Way {
    Air,
    Surface { spec: MoveSpec, region: Region, load: i32, unload: i32 },
}

/// A lift under way.
#[derive(Clone, Debug)]
pub(crate) struct Lift {
    /// What the landing goes for.
    pub target: FxVec3,
    /// Where passengers of a movement class are set down by transports going a way (none
    /// when there is no such place).
    pub zones: Vec<(MoveSpec, Way, Option<FxVec3>)>,
    pub since: u64,
    /// An air lift (its losses tell against air lifts for a while).
    pub air: bool,
    /// Transports that have gone on it (to count those lost), transports lost, units set
    /// down.
    pub flown: Vec<UnitId>,
    pub losses: u32,
    pub delivered: u32,
    /// The load going over now: its transports, when they started loading, and whether
    /// they have set out.
    pub wave: Vec<UnitId>,
    pub wave_since: u64,
    pub launched: bool,
}

/// A passenger promised to a transport.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rider {
    pub unit: UnitId,
    pub transport: UnitId,
    /// Seen aboard (set down once it is off again).
    pub aboard: bool,
    pub since: u64,
    /// Where it was picked up (set down far from there, it is across).
    pub from: FxVec3,
}

/// Least time between two lifts.
const LIFT_COOLDOWN: u64 = 40 * SECOND;
/// A lift that has lost this many transports (while setting down fewer than two units for
/// each) goes into too much fire: it is called off.
const LIFT_LOSSES: u32 = 3;
/// How far from its target a landing zone should be (px): out of the reach of what guards
/// it, near enough to go for it at once; for hovercraft and ships (which can't look first)
/// from an enemy start we know nothing of, well out of the base.
const LANDING_DISTANCE: i64 = 320;
const LANDING_DISTANCE_UNKNOWN: i64 = 900;
/// After an air lift is called off, surface transports are preferred for this long.
const AIR_WARY: u64 = 6 * MINUTE;

impl SkirmishAi {
    /// A ground or hover unit that cannot get from home to the enemy by itself.
    pub(crate) fn stranded(&self, u: &Unit) -> bool {
        matches!(self.info[u.type_id].kind, Locomotion::Ground | Locomotion::Hover) && !self.crosses[u.type_id]
    }

    /// Transport room wanted (units carried at once): none while our land units can get at
    /// the enemy by themselves (or early on); else some, more for a bigger stranded army.
    pub(crate) fn want_transports(&mut self, sim: &Sim, s: &Survey) -> usize {
        if s.minutes < 5 {
            return 0;
        }
        let stranded = s.army.iter().filter(|u| self.stranded(u)).count();
        let factories: Vec<usize> = s.factories.iter().map(|f| f.type_id).collect();
        let land_locked = factories.iter().any(|&f| matches!(self.info[f].kind, Locomotion::Ground | Locomotion::Hover) && !self.factory_reaches(sim, f));
        if stranded == 0 && !land_locked {
            return 0;
        }
        (2 + stranded / 2).min(self.p.transports)
    }

    /// Room a transport type offers (air transports carry one unit at a time).
    pub(crate) fn room(&self, t: usize) -> usize {
        self.info[t].transport.max(1) as usize
    }

    /// The transport a factory should make for our stranded units: the one that moves the
    /// most of them, fastest, per cost. Aircraft go anywhere; hovercraft and ships only
    /// where they suit (their movement class gets from home to the enemy, or the sea does),
    /// and only when no air transport is on offer or air lifts were shot down lately.
    pub(crate) fn pick_transport(&self, sim: &Sim, s: &Survey, opts: &[usize]) -> Option<usize> {
        // What needs carrying: our stranded units, else what our land factories make.
        let mut cargo: Vec<usize> = s.army.iter().filter(|u| self.stranded(u)).map(|u| u.type_id).collect();
        if cargo.is_empty() {
            cargo = s.factories.iter().flat_map(|f| sim.unit_type(f).ai_build_options.iter().copied()).filter(|&o| self.info[o].role == Role::Combat && !self.crosses[o]).collect();
        }
        cargo.sort_unstable();
        cargo.dedup();
        let is_lift = |o: usize| self.info[o].transport > 0 && sim.is_allowed(o) && cargo.iter().any(|&c| sim.types.get(o).can_carry(sim.types.get(c)));
        // Air transports within reach: from a factory we have or one our builders can put up.
        let air_from = |f: usize| sim.types.get(f).ai_build_options.iter().any(|&o| is_lift(o) && self.info[o].kind == Locomotion::Air);
        let air_on_offer = s.factory_types.iter().any(|&f| air_from(f)) || s.builders.iter().any(|b| sim.unit_type(b).ai_build_options.iter().any(|&f| sim.is_allowed(f) && air_from(f)));
        let surface = !air_on_offer || s.now < self.air_lift_failed + AIR_WARY;
        let suits = |o: usize| match self.info[o].kind {
            Locomotion::Air => true,
            Locomotion::Hover => surface && self.crosses[o],
            Locomotion::Naval => surface && self.naval.as_ref().is_some_and(|n| n.reaches),
            _ => false,
        };
        opts.iter()
            .copied()
            .filter(|&o| is_lift(o) && suits(o))
            .max_by_key(|&o| (self.room(o) as i64 * self.info[o].speed.0 / self.info[o].cost.max(1), Reverse(o)))
    }

    /// Where idle air transports wait: behind the base, away from raiders.
    fn transport_pad(&self, sim: &Sim) -> FxVec3 {
        Self::clamp_to_map(sim, self.base - self.front_dir().scale(fx(160)))
    }

    /// How transport `t` gets about.
    pub(crate) fn way(&self, sim: &Sim, t: &Unit) -> Way {
        let i = &self.info[t.type_id];
        match (i.kind, i.spec) {
            (Locomotion::Hover | Locomotion::Naval, Some(spec)) => {
                let tt = sim.unit_type(t);
                let region = map::region_at(&sim.world, &spec, t.pos);
                // It comes alongside to load (booms reach only so far); it unloads from
                // where the simulation lets it (its working distance).
                let load = ((tt.radius.to_int() + 24) / CELL) as i32 + 1;
                let unload = ((tt.build_distance.max(fx(48)) + tt.radius).to_int() / CELL) as i32 + 1;
                Way::Surface { spec, region, load, unload }
            }
            _ => Way::Air,
        }
    }

    /// Cells between `p` and the part of the map a surface transport can go (0 for air).
    fn cells_to(&self, way: &Way, p: FxVec3) -> i32 {
        match way {
            Way::Air => 0,
            Way::Surface { spec, region, .. } => self.reaches.iter().find(|r| r.spec == *spec && r.region == *region).map_or(i32::MAX, |r| r.cells_to(p)),
        }
    }

    fn can_pick_up(&self, way: &Way, p: FxVec3) -> bool {
        match way {
            Way::Air => true,
            Way::Surface { load, .. } => self.cells_to(way, p) <= *load,
        }
    }

    fn can_set_down(&self, way: &Way, p: FxVec3) -> bool {
        match way {
            Way::Air => true,
            Way::Surface { unload, .. } => self.cells_to(way, p) <= *unload,
        }
    }

    /// Where a transport goes back to between loads: air transports to their pad, ships off
    /// the yard, hovercraft to the rally point.
    fn transport_home(&self, sim: &Sim, t: &Unit) -> FxVec3 {
        match self.info[t.type_id].kind {
            Locomotion::Air => self.transport_pad(sim),
            Locomotion::Naval => self.naval.as_ref().map_or(t.pos, |n| n.post),
            _ => self.rally,
        }
    }

    /// Anti-aircraft cover over a point: enemy units able to shoot at aircraft seen now, and
    /// the anti-aircraft buildings we know of.
    fn aa_cover(&self, sites: &[(FxVec3, i64, i64)], p: FxVec3) -> i64 {
        self.enemy_aa.at(p) + sites.iter().filter(|(q, r, _)| q.within_xz(p, fx(*r))).map(|(_, _, c)| *c).sum::<i64>()
    }

    /// Enemy buildings that shoot at aircraft: position, reach, cost.
    fn aa_sites(&self) -> Vec<(FxVec3, i64, i64)> {
        self.enemy_buildings.iter().filter(|(_, _, t)| self.info[*t].air_dps > 0).map(|&(_, p, t)| (p, 560, self.info[t].cost)).collect()
    }

    /// A spot near `target` to set units of movement class `spec` down: ground they can
    /// fight from (connected to the target), clear, about `off` px from the target on the
    /// side the transports come from (`from`), away from towers and enemy units, and for
    /// aircraft from anti-aircraft cover on the way and at the spot; for a surface transport,
    /// where it can come close enough to set them down.
    pub(crate) fn landing_zone(&self, sim: &Sim, target: FxVec3, spec: &MoveSpec, from: FxVec3, way: &Way, off: i64) -> Option<FxVec3> {
        let region = map::region_at(&sim.world, spec, target);
        if region == 0 {
            return None;
        }
        let air = *way == Way::Air;
        let sites = if air { self.aa_sites() } else { Vec::new() };
        // (A shore a ship can get to may lie further off.)
        let rings: &[i64] = if air && off <= LANDING_DISTANCE { &[180, 320, 460, 620] } else { &[180, 320, 460, 620, 820, 1050, 1300] };
        let mut best: Option<(i64, FxVec3)> = None;
        for &r in rings {
            for k in 0..16 {
                let p = Self::clamp_to_map(sim, target + heading_vector(k * 4096).scale(fx(r)));
                let a = World::pos_to_anchor(p, spec.footprint);
                if sim.world.component(spec, a) != region || !sim.world.passable(spec, a, None) || !self.can_set_down(way, p) {
                    continue;
                }
                let d = dist(from, p).max(1);
                // Fire on the way in: sample the way there (as the crow flies). Transports are
                // flimsy (aircraft) or slow (hovercraft, ships): fire on the way costs more
                // than a longer trip.
                let steps = (d / 200).clamp(1, 40);
                let danger = if air {
                    let path: i64 = (1..steps).map(|i| self.aa_cover(&sites, from + (p - from).scale(Fx::from_ratio(i, steps)))).sum();
                    self.aa_cover(&sites, p) * 3 + path + self.defence_at(p) * 2
                } else {
                    let path: i64 = (1..steps).map(|i| self.defence_at(from + (p - from).scale(Fx::from_ratio(i, steps)))).sum();
                    self.defence_at(p) * 3 + path
                };
                // Enemy buildings near the spot besides the target may have guards we haven't
                // seen yet.
                let crowd = self.enemy_buildings.iter().filter(|(_, q, _)| q.within_xz(p, fx(300)) && !q.within_xz(target, fx(64))).count() as i64;
                let score = danger * 2 + crowd * 150 + d / 4 + (r - off).abs();
                if best.is_none_or(|(bs, _)| score < bs) {
                    best = Some((score, p));
                }
            }
        }
        best.map(|(_, p)| p)
    }

    /// What a lift should go for, and where the passengers of class `spec` are set down: the
    /// enemy building on land worth the most for the least defence (towers, units seen,
    /// anti-aircraft cover) that a force of `strength` can take, nearest `near` (transports
    /// come in from `from`, going `way`). Far stronger than the enemy (`finish`), what ends
    /// the game first: its factories and Commander, and more defence is taken on. Knowing of
    /// no enemy building on land at all, an enemy start (to be looked at first); knowing only
    /// of ones too well defended, nothing yet. Also whether the target is one we have seen.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lift_target(&self, sim: &Sim, spec: &MoveSpec, near: FxVec3, from: FxVec3, strength: i64, finish: bool, way: &Way) -> Option<(FxVec3, FxVec3, bool)> {
        let sites = if *way == Way::Air { self.aa_sites() } else { Vec::new() };
        let takes = |defence: i64| if finish { defence <= strength } else { defence * 3 <= strength * 2 };
        let mut cands: Vec<(i64, u32, FxVec3)> = Vec::new();
        for &(id, p, t) in &self.enemy_buildings {
            if !self.info[t].land || self.cleared.iter().any(|(c, _)| c.within_xz(p, fx(300))) {
                continue;
            }
            let defence = self.defence_at(p);
            if !takes(defence) {
                continue;
            }
            let value = self.target_value(t) + if finish && self.info[t].role == Role::Factory { 3000 } else { 0 };
            cands.push((dist(near, p) / 2 + defence * 2 + self.aa_cover(&sites, p) * 2 - value, id.slot, p));
        }
        if let Some((_, p, _)) = self.enemy_commander.filter(|_| finish) {
            let defence = self.defence_at(p);
            if takes(defence) {
                cands.push((dist(near, p) / 2 + defence * 2 + self.aa_cover(&sites, p) * 2 - 6000, u32::MAX, p));
            }
        }
        cands.sort_unstable_by_key(|&(score, slot, _)| (score, slot));
        for (_, _, p) in cands.into_iter().take(6) {
            if let Some(lz) = self.landing_zone(sim, p, spec, from, way, LANDING_DISTANCE) {
                return Some((p, lz, true));
            }
        }
        if self.enemy_buildings.iter().any(|(_, _, t)| self.info[*t].land) {
            return None;
        }
        // Nothing known over there yet: the nearest enemy start.
        let mut starts = self.enemy_bases.clone();
        starts.sort_unstable_by_key(|p| (dist(near, *p), p.x.0, p.z.0));
        let off = if *way == Way::Air { LANDING_DISTANCE } else { LANDING_DISTANCE_UNKNOWN };
        starts.into_iter().filter(|p| !self.cleared.iter().any(|(c, _)| c.within_xz(*p, fx(300)))).find_map(|p| self.landing_zone(sim, p, spec, from, way, off).map(|lz| (p, lz, false)))
    }

    /// The landing zone of the current lift for a movement class and a transport way
    /// (looked for once per pair).
    fn zone_for(&mut self, sim: &Sim, spec: MoveSpec, way: Way) -> Option<FxVec3> {
        let lift = self.lift.as_ref()?;
        if let Some(&(_, _, p)) = lift.zones.iter().find(|(s, w, _)| *s == spec && *w == way) {
            return p;
        }
        let from = self.transport_pad(sim);
        let unknown = self.enemy_bases.iter().any(|b| b.within_xz(lift.target, fx(64))) && !self.enemy_buildings.iter().any(|(_, p, _)| p.within_xz(lift.target, fx(250)));
        let off = if unknown && way != Way::Air { LANDING_DISTANCE_UNKNOWN } else { LANDING_DISTANCE };
        let lz = self.landing_zone(sim, lift.target, &spec, from, &way, off);
        if let Some(l) = self.lift.as_mut() {
            l.zones.push((spec, way, lz));
        }
        lz
    }

    pub(crate) fn run_transports(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let now = s.now;
        let me = self.player;
        self.landed.retain(|&id| sim.unit(id).is_some_and(|u| !u.dead && u.owner == me));
        if s.transports.is_empty() && self.riders.is_empty() && self.landed.is_empty() {
            if self.lift.take().is_some() {
                self.last_lift = now;
            }
            return;
        }
        let pad = self.transport_pad(sim);
        let rally = self.rally;
        let transports: Vec<&Unit> = s.transports.clone();
        let surface: Vec<&Unit> = transports.iter().copied().filter(|t| self.info[t.type_id].kind != Locomotion::Air).collect();
        self.prepare_reaches(sim, &surface);
        let ways: Vec<(UnitId, Way)> = transports.iter().map(|t| (t.id, self.way(sim, t))).collect();
        let way_of = |id: UnitId| ways.iter().find(|w| w.0 == id).map_or(Way::Air, |w| w.1);

        // 1. Passengers: aboard, set down (over there, or back home when a landing failed),
        // or no longer awaited.
        let mut set_down: Vec<UnitId> = Vec::new();
        let mut released: Vec<UnitId> = Vec::new();
        let mut riders = std::mem::take(&mut self.riders);
        riders.retain_mut(|r| {
            let transport = sim.unit(r.transport).filter(|t| !t.dead && t.owner == me);
            let Some(u) = sim.unit(r.unit).filter(|u| !u.dead && u.owner == me) else {
                if transport.is_some() && !r.aboard {
                    released.push(r.transport);
                }
                return false;
            };
            let Some(t) = transport else { return false };
            match u.carried_by {
                Some(c) if c == r.transport => {
                    r.aboard = true;
                    true
                }
                Some(_) => false,
                None if r.aboard => {
                    // Off again: across, or back home (a landing that failed).
                    if !u.pos.within_xz(r.from, fx(1200)) {
                        set_down.push(r.unit);
                    }
                    false
                }
                None => {
                    // (Hovercraft and ships are slow to call on everyone.)
                    let wait = if t.orders.iter().filter(|o| matches!(o, Order::Load(_))).count() > 1 { 180 } else { 90 } * SECOND;
                    let coming = t.orders.iter().any(|o| *o == Order::Load(r.unit));
                    if coming && now < r.since + wait {
                        true
                    } else {
                        released.push(r.transport);
                        false
                    }
                }
            }
        });
        self.riders = riders;
        for id in released {
            // Waiting for no one now (unless it still has others to pick up).
            let Some(t) = sim.unit(id) else { continue };
            if t.cargo.is_empty() && !self.riders.iter().any(|r| r.transport == id) {
                out.push(Command::Order { player: me, units: vec![id], order: Order::Move(self.transport_home(sim, t)), queue: false });
            }
        }
        if let Some(lift) = self.lift.as_mut() {
            let before = lift.flown.len();
            lift.flown.retain(|&id| sim.unit(id).is_some_and(|t| !t.dead && t.owner == me));
            lift.losses += (before - lift.flown.len()) as u32;
            lift.delivered += set_down.len() as u32;
        }
        if !set_down.is_empty() {
            let target = self.lift.as_ref().map(|l| l.target);
            log::debug!("{} min: P{me} lift sets {} units down", s.minutes, set_down.len());
            self.landed.extend(&set_down);
            if let Some(target) = target {
                out.push(Command::Order { player: me, units: set_down, order: Order::Fight(target), queue: false });
            }
        }

        // 2. Units set down fight on over there: idle ones go for the next target they can
        // reach.
        if !self.landed.is_empty() && (now / THINK_TICKS) % 6 == me as u64 % 6 {
            self.push_landed(sim, s, out);
        }

        // 3. The lift: started once transports and passengers are ready (after a look at the
        // enemy when we know nothing of it), retargeted when its target falls, called off
        // when it has cost too much or nobody is left to carry.
        let taken = |id: UnitId, ai: &SkirmishAi| {
            Some(id) == ai.ground_scout || Some(id) == ai.scout || ai.riders.iter().any(|r| r.unit == id) || ai.landed.contains(&id) || ai.attacking.contains(&id) || ai.defending.iter().any(|d| d.0 == id) || ai.repairing.iter().any(|r| r.0 == id)
        };
        let carriable = |u: &Unit| transports.iter().any(|t| sim.unit_type(t).can_carry(sim.unit_type(u)) && self.can_pick_up(&way_of(t.id), u.pos));
        let mut waiting: Vec<&Unit> = s
            .army
            .iter()
            .copied()
            .filter(|u| self.stranded(u) && !taken(u.id, self) && u.carried_by.is_none() && matches!(u.orders.front(), None | Some(Order::Move(_))) && carriable(u))
            .collect();
        waiting.sort_by_key(|u| (Reverse(self.info[u.type_id].cost), u.id.slot));
        let idle: Vec<&Unit> = transports.iter().copied().filter(|t| t.orders.is_empty() && t.cargo.is_empty() && !self.riders.iter().any(|r| r.transport == t.id)).collect();
        let room = |ts: &[&Unit]| ts.iter().map(|t| self.room(t.type_id)).sum::<usize>();
        let (idle_room, all_room) = (room(&idle), room(&transports));
        // Air transports lead (they go anywhere), else the first surface one ready.
        let lead = idle.iter().map(|t| way_of(t.id)).find(|w| *w == Way::Air).or_else(|| idle.first().map(|t| way_of(t.id)));
        let spec = waiting.first().and_then(|u| self.info[u.type_id].spec);
        let min = if self.difficulty == Difficulty::Easy { 2 } else { 3 };
        if let Some((target, since, losses, delivered, air)) = self.lift.as_ref().map(|l| (l.target, l.since, l.losses, l.delivered, l.air)) {
            let aborted = losses >= LIFT_LOSSES && delivered < losses * 2;
            let finished = waiting.is_empty() && self.riders.is_empty();
            if aborted || finished || now > since + 8 * MINUTE {
                log::debug!("{} min: P{me} lift ends (delivered {delivered}, lost {losses}{})", s.minutes, if aborted { ", called off" } else { "" });
                if aborted {
                    // Don't go back there for a while (nor by air, if that was the way).
                    self.cleared.push((target, now + 4 * MINUTE));
                    if air {
                        self.air_lift_failed = now;
                    }
                }
                self.lift = None;
                self.last_lift = now;
            } else if (now / THINK_TICKS) % 10 == me as u64 % 10 && !self.target_standing(sim, target) {
                // Target taken: land the next ones near where the landed units fight on.
                let near = self.landed_centre(sim).unwrap_or(target);
                let strength: i64 = self.landed.iter().filter_map(|&id| sim.unit(id)).map(|u| self.info[u.type_id].cost).sum::<i64>() + waiting.iter().take(idle_room.max(1)).map(|u| self.info[u.type_id].cost).sum::<i64>();
                let way = lead.unwrap_or(if air { Way::Air } else { transports.first().map_or(Way::Air, |t| way_of(t.id)) });
                if let Some(sp) = spec && let Some((target, lz, _)) = self.lift_target(sim, &sp, near, pad, strength, self.dominant(s), &way) {
                    log::debug!("{} min: P{me} lift retargets ({}, {})", s.minutes, target.x.to_int(), target.z.to_int());
                    if let Some(l) = self.lift.as_mut() {
                        l.target = target;
                        l.zones = vec![(sp, way, Some(lz))];
                    }
                }
            }
        } else if let (Some(sp), Some(way)) = (spec, lead) {
            // Most of the transport room wanted, and passengers for it.
            let ready = min.max(self.want_transports(sim, s) * 2 / 3);
            if idle_room >= ready && waiting.len() >= min && !s.peace && now >= self.last_lift + LIFT_COOLDOWN {
                let first: i64 = waiting.iter().take(idle_room).map(|u| self.info[u.type_id].cost).sum();
                match self.lift_target(sim, &sp, pad, pad, first, self.dominant(s), &way) {
                    Some((target, _, false)) if way == Way::Air && now >= self.scouted + 3 * MINUTE => {
                        // Nothing seen over there yet: one transport flies round it for a look
                        // (near side first; round the edge of what guards it rather than
                        // straight over), then home.
                        let scout = idle.iter().find(|t| way_of(t.id) == Way::Air).map_or(idle[0].id, |t| t.id);
                        log::debug!("{} min: P{me} transport scouts ({}, {})", s.minutes, target.x.to_int(), target.z.to_int());
                        self.scouted = now;
                        let back = (pad - target).xz().normalize();
                        let side = FxVec3::new(back.z, Fx::ZERO, -back.x);
                        for (k, dir) in [back, side, back.scale(-Fx::ONE), side.scale(-Fx::ONE)].into_iter().enumerate() {
                            let at = Self::clamp_to_map(sim, target + dir.scale(fx(450)));
                            out.push(Command::Order { player: me, units: vec![scout], order: Order::Move(at), queue: k > 0 });
                        }
                        out.push(Command::Order { player: me, units: vec![scout], order: Order::Move(pad), queue: true });
                        return;
                    }
                    Some((target, lz, _)) => {
                        log::debug!("{} min: P{me} lift of {} transports ({} waiting) for ({}, {}), landing at ({}, {})", s.minutes, idle.len(), waiting.len(), target.x.to_int(), target.z.to_int(), lz.x.to_int(), lz.z.to_int());
                        self.lift = Some(Lift {
                            target,
                            zones: vec![(sp, way, Some(lz))],
                            since: now,
                            air: way == Way::Air,
                            flown: Vec::new(),
                            losses: 0,
                            delivered: 0,
                            wave: Vec::new(),
                            wave_since: now,
                            launched: false,
                        });
                    }
                    None => {
                        // Nothing we can take yet: look again in a while.
                        log::debug!("{} min: P{me} finds no landing for a lift ({} transports, {} waiting)", s.minutes, idle.len(), waiting.len());
                        self.last_lift = (now + 20 * SECOND).saturating_sub(LIFT_COOLDOWN);
                    }
                }
            }
        }

        // 4. Waves: once most of the transports are back, each idle one picks up passengers
        // (the most valuable first; as many as fit) for whom it has a landing zone; aircraft
        // then wait at the rally point, surface transports where they are. When all are
        // loaded they go over together, set their loads down around the landing zone and
        // come back for more.
        let mut wave = self.lift.as_ref().map(|l| (l.wave.clone(), l.wave_since, l.launched));
        if let Some((members, since, launched)) = wave.as_mut() {
            members.retain(|&id| sim.unit(id).is_some_and(|t| !t.dead && t.owner == me));
            let units: Vec<&Unit> = members.iter().filter_map(|&id| sim.unit(id)).collect();
            if members.is_empty() {
                let ready = idle_room >= min.min(all_room) && (idle_room * 3 >= all_room * 2 || idle_room >= 6);
                if ready {
                    for t in &idle {
                        let way = way_of(t.id);
                        let tt = sim.unit_type(t);
                        let mut took = 0;
                        // (A hovercraft or ship calls on its passengers one by one: the ones
                        // nearest it, rather than the most valuable.)
                        let mut order: Vec<usize> = (0..waiting.len()).collect();
                        if way != Way::Air {
                            order.sort_by_key(|&k| (dist(waiting[k].pos, t.pos), waiting[k].id.slot));
                        }
                        let mut picked: Vec<usize> = Vec::new();
                        for k in order {
                            if picked.len() >= self.room(t.type_id) {
                                break;
                            }
                            let p = waiting[k];
                            if !tt.can_carry(sim.unit_type(p)) || !self.can_pick_up(&way, p.pos) {
                                continue;
                            }
                            if self.info[p.type_id].spec.is_some_and(|sp| self.zone_for(sim, sp, way).is_some()) {
                                picked.push(k);
                            }
                        }
                        for (n, &k) in picked.iter().enumerate() {
                            let p = waiting[k];
                            out.push(Command::Order { player: me, units: vec![t.id], order: Order::Load(p.id), queue: n > 0 });
                            self.riders.push(Rider { unit: p.id, transport: t.id, aboard: false, since: now, from: p.pos });
                            took += 1;
                        }
                        picked.sort_unstable();
                        for k in picked.into_iter().rev() {
                            waiting.remove(k);
                        }
                        if took > 0 {
                            if way == Way::Air {
                                out.push(Command::Order { player: me, units: vec![t.id], order: Order::Move(rally), queue: true });
                            }
                            members.push(t.id);
                        }
                    }
                    *since = now;
                    *launched = false;
                }
            } else if !*launched {
                let loading = units.iter().any(|t| t.orders.iter().any(|o| matches!(o, Order::Load(_))));
                let gathered = units.iter().all(|t| t.cargo.is_empty() || t.orders.is_empty());
                // (Surface transports are slow to load up: they have longer.)
                let patience = if units.iter().all(|t| way_of(t.id) == Way::Air) { MINUTE } else { 2 * MINUTE };
                if !loading && gathered || now > *since + patience {
                    let loaded: Vec<&Unit> = units.iter().copied().filter(|t| !t.cargo.is_empty()).collect();
                    let n = loaded.len().max(1) as i32;
                    for (k, t) in loaded.iter().enumerate() {
                        let spec = t.cargo.first().and_then(|&c| sim.unit(c)).and_then(|c| self.info[c.type_id].spec);
                        let Some(lz) = spec.and_then(|sp| self.zone_for(sim, sp, way_of(t.id))) else { continue };
                        // Spread around the zone (the loads come down at once).
                        let spot = Self::clamp_to_map(sim, lz + heading_vector(k as i32 * 65536 / n).scale(fx(if n > 1 { 48 } else { 0 })));
                        out.push(Command::Order { player: me, units: vec![t.id], order: Order::Unload(spot), queue: false });
                        out.push(Command::Order { player: me, units: vec![t.id], order: Order::Move(self.transport_home(sim, t)), queue: true });
                    }
                    log::debug!("{} min: P{me} lift wave of {} sets out", s.minutes, loaded.len());
                    *members = loaded.iter().map(|t| t.id).collect();
                    *launched = true;
                    *since = now;
                }
            } else if units.iter().all(|t| t.cargo.is_empty() && !t.orders.iter().any(|o| matches!(o, Order::Unload(_)))) || now > *since + 4 * MINUTE {
                // All set down (or given up): the next wave forms once they are back.
                members.clear();
            }
        }
        if let (Some(l), Some((members, since, launched))) = (self.lift.as_mut(), wave) {
            for &id in &members {
                if !l.flown.contains(&id) {
                    l.flown.push(id);
                }
            }
            l.wave = members;
            l.wave_since = since;
            l.launched = launched;
        }

        // 5. Cargo aboard an idle transport outside a wave (no room to set it down, or the
        // lift is over): try again a little way off, or set it down at home.
        let forming: Vec<UnitId> = self.lift.as_ref().filter(|l| !l.launched).map(|l| l.wave.clone()).unwrap_or_default();
        for t in transports.iter().filter(|t| t.orders.is_empty() && !t.cargo.is_empty() && !forming.contains(&t.id)) {
            let spec = t.cargo.first().and_then(|&c| sim.unit(c)).and_then(|c| self.info[c.type_id].spec);
            let home = self.transport_home(sim, t);
            let at = match spec.and_then(|sp| self.zone_for(sim, sp, way_of(t.id))) {
                Some(lz) => {
                    let jitter = heading_vector(self.rng.range_i32(0, 65535)).scale(fx(self.rng.range_i32(48, 160) as i64));
                    Self::clamp_to_map(sim, lz + jitter)
                }
                None => rally,
            };
            out.push(Command::Order { player: me, units: vec![t.id], order: Order::Unload(at), queue: false });
            out.push(Command::Order { player: me, units: vec![t.id], order: Order::Move(home), queue: true });
        }

        // 6. Idle transports with nothing to do wait at home.
        if (now / THINK_TICKS) % 8 == me as u64 % 8 {
            for t in transports.iter().filter(|t| t.orders.is_empty() && t.cargo.is_empty() && !self.riders.iter().any(|r| r.transport == t.id)) {
                let home = self.transport_home(sim, t);
                if !t.pos.within_xz(home, fx(400)) {
                    out.push(Command::Order { player: me, units: vec![t.id], order: Order::Move(home), queue: false });
                }
            }
        }
    }

    /// Whether an enemy building still stands near a lift's target (or it is an enemy start
    /// nobody has found empty yet).
    fn target_standing(&self, _sim: &Sim, target: FxVec3) -> bool {
        self.enemy_buildings.iter().any(|(_, p, _)| p.within_xz(target, fx(250)))
            || self.enemy_bases.iter().any(|b| b.within_xz(target, fx(64))) && !self.cleared.iter().any(|(c, _)| c.within_xz(target, fx(300)))
    }

    fn landed_centre(&self, sim: &Sim) -> Option<FxVec3> {
        let group: Vec<FxVec3> = self.landed.iter().filter_map(|&id| sim.unit(id)).map(|u| u.pos).collect();
        let n = group.len() as i64;
        (n > 0).then(|| {
            let sum = group.iter().fold(FxVec3::ZERO, |a, &p| a + p);
            FxVec3::new(sum.x / n, Fx::ZERO, sum.z / n)
        })
    }

    /// Units set down across the water: idle ones go for the next target they can reach
    /// from where they stand (what ends the game when we are far stronger).
    fn push_landed(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let me = self.player;
        let group: Vec<&Unit> = self.landed.iter().filter_map(|&id| sim.unit(id)).collect();
        let idle: Vec<&Unit> = group.iter().copied().filter(|u| u.orders.is_empty()).collect();
        let Some(&probe) = idle.first() else { return };
        let Some(centre) = self.landed_centre(sim) else { return };
        let strength: i64 = group.iter().map(|u| self.info[u.type_id].cost).sum();
        if idle.len() * 2 >= group.len() && self.opposition(s, centre) == 0 && !self.enemy_buildings.iter().any(|(_, p, _)| p.within_xz(centre, fx(400))) {
            self.cleared.push((centre, s.now + 3 * MINUTE));
        }
        let finish = if self.dominant(s) { self.finishing_target(sim, &idle) } else { None };
        let target = finish.or_else(|| self.weak_target(sim, centre, probe, strength, true)).or_else(|| self.nearest_target(sim, centre, probe));
        if let Some(target) = target {
            let units: Vec<UnitId> = idle.iter().filter(|u| self.unit_reaches(sim, u, target)).map(|u| u.id).collect();
            if !units.is_empty() {
                out.push(Command::Order { player: me, units, order: Order::Fight(target), queue: false });
            }
        }
    }
}
