//! Sea power: a shipyard where the sea reaches the enemy, warships from it, and a fleet that
//! hunts enemy ships and shells whatever stands within gun range of the water: coastal
//! buildings, extractors under water, fleets. Knowing of nothing to hit, it sails for the
//! enemy's coast to find something; outgunned, it falls back and comes again once stronger
//! than what drove it off, and ships finished meanwhile join a push that is going well.

use rustc_hash::FxHashSet;
use ta_sim::world::{CELL, MoveSpec};

use super::*;

/// What the sea offers us: a shipyard our builders can put up, a site for it near home, and
/// whether warships from there can get within gun range of an enemy base.
#[derive(Clone, Debug)]
pub(crate) struct Naval {
    pub yard: usize,
    pub site: FxVec3,
    pub reaches: bool,
    /// Reachable open water off the yard, where idle ships wait.
    pub post: FxVec3,
}

/// Looks for a shipyard site this many times (a minute apart) before giving up.
const NAVAL_TRIES: u8 = 3;

impl SkirmishAi {
    /// Cells within which a ship with this gun range must get to a target to hit it (a ring
    /// of cells reaches out diagonally, so a little less than range / 16).
    fn gun_slack(range: i64) -> i32 {
        (range / 20).clamp(3, 30) as i32
    }

    /// Maximum compatible automatic ground-weapon reach, using known type/position
    /// data without looking up a hidden live target.
    pub(crate) fn ground_target_range(&self, sim: &Sim, attacker: &Unit, target: usize, pos: FxVec3) -> Option<Fx> {
        let t = sim.types.get(target);
        let wet = pos.y - t.waterline <= sim.world.sea_level && sim.world.depth_at(pos.x, pos.z) > Fx::ZERO;
        let submerged = match t.locomotion {
            Locomotion::Naval => t.waterline > Fx::ZERO && wet,
            Locomotion::Air | Locomotion::Hover => false,
            _ => pos.y + t.model.height.max(fx(8)) < sim.world.sea_level,
        };
        sim.unit_type(attacker).weapons.iter().flatten().map(|&id| sim.weapon_types.get(id)).filter(|w| {
            crate::info::hits_ground(w)
                && (w.def.fire_submersed || !sim.submerged(attacker))
                && (w.water || !submerged) && (!w.torpedo || wet)
        }).map(|w| w.range).max()
    }

    /// Open water for the yard's usable products, in their respective seas and
    /// outside the planned yard. Both large footprints and deep drafts count.
    fn naval_post(&self, sim: &Sim, yard: usize, site: FxVec3) -> Option<FxVec3> {
        let t = sim.types.get(yard);
        let mut specs = Vec::new();
        for &o in t.ai_build_options.iter().filter(|&&o| sim.is_allowed(o) && self.info[o].kind == Locomotion::Naval) {
            let Some(spec) = self.info[o].spec else { continue };
            let region = map::region_at(&sim.world, &spec, site);
            if region != 0 && !specs.contains(&(spec, region)) { specs.push((spec, region)); }
        }
        let &(spec, _) = specs.iter().max_by_key(|(s, _)| (s.footprint.0 * s.footprint.1, s.min_depth))?;
        let want = site + FxVec3::new(Fx::ZERO, Fx::ZERO, fx(t.footprint.1 as i64 * 8 + 120));
        let goal = World::pos_to_anchor(want, spec.footprint);
        for r in 0i32..=24 {
            let mut ring: Vec<_> = (-r..=r).flat_map(|dz| (-r..=r).map(move |dx| (dx, dz))).filter(|&(dx, dz)| dx.abs().max(dz.abs()) == r).collect();
            ring.sort_unstable_by_key(|&(dx, dz)| (dx * dx + dz * dz, dz, dx));
            for (dx, dz) in ring {
                let mut point = sim.world.anchor_to_pos((goal.0 + dx, goal.1 + dz), spec.footprint);
                if !specs.iter().all(|(s, region)| {
                    let cell = World::pos_to_anchor(point, s.footprint);
                    let clear_x = (point.x - site.x).abs() >= fx((t.footprint.0 + s.footprint.0) as i64 * CELL / 2);
                    let clear_z = (point.z - site.z).abs() >= fx((t.footprint.1 + s.footprint.1) as i64 * CELL / 2);
                    (clear_x || clear_z) && sim.world.component(s, cell) == *region && sim.world.passable(s, cell, None)
                }) { continue; }
                point.y = sim.world.sea_level;
                return Some(point);
            }
        }
        None
    }

    /// Whether any warship this yard builds can get within gun range of an enemy base.
    pub(crate) fn yard_reaches(&self, sim: &Sim, yard: usize, site: FxVec3) -> bool {
        let mut specs: Vec<(MoveSpec, i64, i64)> = Vec::new();
        for &o in sim.types.get(yard).ai_build_options.iter().filter(|&&o| sim.is_allowed(o)) {
            let i = &self.info[o];
            if let (Role::Combat, Some(sp)) = (i.role, i.spec) {
                let t = sim.types.get(o);
                let weapons: Vec<_> = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w))
                    .filter(|w| crate::info::hits_ground(w) && (t.waterline.is_zero() || w.def.fire_submersed)).collect();
                let wet = weapons.iter().map(|w| w.range.to_int()).max().unwrap_or(0);
                let dry = weapons.iter().filter(|w| !w.torpedo).map(|w| w.range.to_int()).max().unwrap_or(0);
                match specs.iter_mut().find(|(s, _, _)| *s == sp) {
                    Some(e) => { e.1 = e.1.max(dry); e.2 = e.2.max(wet); }
                    None => specs.push((sp, dry, wet)),
                }
            }
        }
        specs.iter().any(|(sp, dry, wet)| self.enemy_bases.iter().any(|&e| {
            let range = if sim.world.depth_at(e.x, e.z) > Fx::ZERO { *wet } else { *dry };
            range > 0 && map::can_reach(&sim.world, sp, site, e, Self::gun_slack(range))
        }))
    }

    /// The distance field of the sea a ship sails in (see `prepare_reaches`).
    pub(crate) fn reach_of(&self, sim: &Sim, u: &Unit) -> Option<&map::Reach> {
        let spec = self.info[u.type_id].spec?;
        let region = map::region_at(&sim.world, &spec, u.pos);
        self.reaches.iter().find(|r| r.spec == spec && r.region == region)
    }

    /// Distance fields (and the enemy's nearest coast) for the seas our ships sail in, one
    /// per movement class and sea, made the first time a ship of it needs one.
    pub(crate) fn prepare_reaches(&mut self, sim: &Sim, ships: &[&Unit]) {
        for u in ships {
            let Some(spec) = self.info[u.type_id].spec else { continue };
            let region = map::region_at(&sim.world, &spec, u.pos);
            if region == 0 || self.reaches.iter().any(|r| r.spec == spec && r.region == region) {
                continue;
            }
            let mut r = map::Reach::new(&sim.world, &spec, region);
            r.coast = self
                .enemy_bases
                .iter()
                .map(|&b| (b, sim.world.nearest_in_component(&spec, region, World::pos_to_anchor(b, spec.footprint), 160).map(|c| sim.world.anchor_to_pos(c, spec.footprint))))
                .collect();
            self.reaches.push(r);
        }
    }

    /// Whether a ship can get within gun range of `to`.
    pub(crate) fn ship_reaches(&self, sim: &Sim, u: &Unit, to: FxVec3) -> bool {
        let i = &self.info[u.type_id];
        let slack = Self::gun_slack(i.range);
        match self.reach_of(sim, u) {
            Some(r) => r.cells_to(to) <= slack,
            None => i.spec.is_none_or(|sp| map::can_reach(&sim.world, &sp, u.pos, to, slack)),
        }
    }

    /// The water nearest an enemy start that a ship can get to, nearest `from` first, not
    /// where a fleet lately found nothing.
    fn enemy_coast(&self, sim: &Sim, probe: &Unit, from: FxVec3) -> Option<FxVec3> {
        let r = self.reach_of(sim, probe)?;
        let gone = |c: FxVec3| self.cleared.iter().chain(self.fleet_avoid.iter()).any(|(p, _)| p.within_xz(c, fx(400)));
        r.coast.iter().filter_map(|&(_, c)| c).filter(|&c| !gone(c)).min_by_key(|c| (dist(from, *c), c.x.0, c.z.0))
    }

    /// Looks for a shipyard site near home once the game is under way (again a minute later
    /// when none was found, a few times).
    pub(crate) fn plan_navy(&mut self, sim: &Sim, s: &Survey) {
        if self.naval.is_some() || self.naval_tries >= NAVAL_TRIES || s.minutes < 1 || self.naval_tries > 0 && s.now < self.naval_checked + MINUTE {
            return;
        }
        self.naval_checked = s.now;
        self.naval_tries += 1;
        let Some(b) = s.commander.or_else(|| s.builders.iter().copied().find(|b| self.info[b.type_id].builds_factory)) else { return };
        let Some(yard) = sim
            .unit_type(b)
            .ai_build_options
            .iter()
            .copied()
            .filter(|&o| self.info[o].role == Role::Factory && self.info[o].kind == Locomotion::Naval && !self.advanced_factory(sim, o) && sim.is_allowed(o))
            .min_by_key(|&o| (self.info[o].cost, o))
        else {
            log::debug!("P{} has no shipyard to build", self.player);
            self.naval_tries = NAVAL_TRIES;
            return;
        };
        let Some(site) = self.find_site(sim, yard, self.base, 56, &|p| self.builder_reaches(sim, b, p, yard)) else {
            log::debug!("P{} found no {} site near home", self.player, sim.types.get(yard).name);
            return;
        };
        let reaches = self.yard_reaches(sim, yard, site);
        let Some(post) = self.naval_post(sim, yard, site) else { return };
        log::debug!("P{} shipyard site ({}, {}), reaches the enemy: {reaches}", self.player, site.x.to_int(), site.z.to_int());
        self.naval = Some(Naval { yard, site, reaches, post });
    }

    /// A shipyard, when the sea reaches the enemy: soon when our land army cannot get at
    /// the enemy, else once the economy has grown (not on Easy).
    pub(crate) fn shipyard_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, plan: &Plan) -> Option<Task> {
        let n = self.naval.clone()?;
        let yards = s.kinds.iter().filter(|&&k| k == Locomotion::Naval).count();
        if !n.reaches || yards >= 3 || plan.pending[Role::Factory as usize] > 0 || !sim.unit_type(b).ai_build_options.contains(&n.yard) {
            return None;
        }
        let types = s.factory_types.clone();
        let land_reaches = types.iter().any(|&f| !matches!(self.info[f].kind, Locomotion::Air | Locomotion::Naval) && self.factory_reaches(sim, f));
        let ground_reaches = types.iter().any(|&f| self.info[f].kind == Locomotion::Ground && self.factory_reaches(sim, f));
        let factories = s.count[Role::Factory as usize];
        let ready = if yards >= 1 {
            // More yards where the war is at sea (the ground army can't get across): a second
            // once the economy can feed it, a third with metal piling up.
            let rich = self.metal_avg > Fx::from_ratio(6, 10);
            !ground_reaches && self.difficulty != Difficulty::Easy && s.minutes >= 8 && s.m_inc >= fx(if yards == 1 { 25 } else { 45 }) && (yards == 1 || rich) && factories < self.p.max_factories + 2
        } else if land_reaches {
            self.difficulty != Difficulty::Easy && s.minutes >= 9 && s.m_inc >= fx(14) && factories < self.p.max_factories
        } else if types.is_empty() {
            // No room for a factory on land: the sea it is.
            s.minutes >= 2 || self.place_failed.iter().any(|&(t, _)| self.info[t].role == Role::Factory)
        } else {
            s.minutes >= 3 && s.m_inc >= fx(5) && factories <= self.p.max_factories
        };
        if !ready {
            return None;
        }
        let r = fx(sim.types.get(n.yard).radius.to_int() + 16);
        let failed = |p: FxVec3| self.failed.iter().any(|(t, fp, _)| *t == n.yard && fp.within_xz(p, fx(32)));
        if sim.can_place(n.yard, n.site) && plan.free(n.site, r) && !failed(n.site) && self.builder_reaches(sim, b, n.site, n.yard) {
            return Some(Task::Build(n.yard, n.site));
        }
        let site = self.find_site(sim, n.yard, n.site, 24, &|p| plan.free(p, r) && self.builder_reaches(sim, b, p, n.yard))?;
        let post = self.naval_post(sim, n.yard, site)?;
        if let Some(n) = self.naval.as_mut() {
            n.site = site;
            n.post = post;
        }
        Some(Task::Build(n.yard, site))
    }

    /// The best target for ships at `from`: enemy warships near by, else the enemy building
    /// within gun range of the water worth the most for the least defence and distance
    /// (extractors under water, tidal plants, coastal factories...), else other enemy units
    /// there, else the enemy's coast (to find what stands there).
    fn naval_target(&self, sim: &Sim, s: &Survey, from: FxVec3, probe: &Unit) -> Option<FxVec3> {
        let avoided = |p: FxVec3| self.fleet_avoid.iter().any(|(q, _)| q.within_xz(p, fx(450)));
        let best = |v: Vec<(i64, u32, FxVec3, usize)>| v.into_iter().filter(|&(_, _, p, t)| !avoided(p) && self.ship_reaches(sim, probe, p) && self.ground_target_range(sim, probe, t, p).is_some()).min_by_key(|&(score, slot, _, _)| (score, slot)).map(|(_, _, p, _)| p);
        let ships = s.enemies.iter().filter(|e| e.is_complete() && self.info[e.type_id].kind == Locomotion::Naval && e.pos.within_xz(from, fx(2000))).map(|e| (dist(from, e.pos), e.id.slot, e.pos, e.type_id)).collect();
        best(ships)
            .or_else(|| best(self.enemy_buildings.iter().map(|&(id, p, t)| (dist(from, p) + self.defence_at(p) * 2 - self.target_value(t), id.slot, p, t)).collect()))
            .or_else(|| best(s.enemies.iter().filter(|e| self.info[e.type_id].kind != Locomotion::Air).map(|e| (dist(from, e.pos), e.id.slot, e.pos, e.type_id)).collect()))
            .or_else(|| self.enemy_coast(sim, probe, from))
    }

    /// The fleet: meets enemy warships in our waters, strikes once it is strong enough, keeps
    /// pushing along the coast (joined by new ships), falls back when outgunned, and waits
    /// off the yard.
    pub(crate) fn run_navy(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let now = s.now;
        let me = self.player;
        let repairing: FxHashSet<_> = self.repairing.iter().map(|r| r.0).collect();
        self.fleet.retain(|&id| !repairing.contains(&id) && sim.unit(id).is_some_and(|u| !u.dead && u.owner == me));
        self.fleet_avoid.retain(|&(_, until)| now < until);
        if (now / THINK_TICKS) % 4 != me as u64 % 4 {
            return;
        }
        let ships: Vec<&Unit> = s.army.iter().copied().filter(|u| self.info[u.type_id].kind == Locomotion::Naval && !repairing.contains(&u.id) && !self.defending.iter().any(|d| d.0 == u.id)).collect();
        if ships.is_empty() {
            return;
        }
        self.prepare_reaches(sim, &ships);
        let cost = |u: &Unit| self.info[u.type_id].cost;
        let post = self.naval.as_ref().map_or(self.rally, |n| n.post);
        let (sailing, home): (Vec<&Unit>, Vec<&Unit>) = ships.iter().partition(|u| self.fleet.contains(&u.id));

        // 1. Enemy warships in our waters: the ships at home meet them.
        let intruder = s
            .enemies
            .iter()
            .filter(|e| e.is_complete() && self.info[e.type_id].kind == Locomotion::Naval && e.pos.within_xz(post, fx(2000)))
            .filter(|e| home.iter().any(|u| self.ship_reaches(sim, u, e.pos) && self.ground_target_range(sim, u, e.type_id, e.pos).is_some()))
            .min_by_key(|e| (dist(e.pos, post), e.id.slot));
        if let Some(e) = intruder {
            let units: Vec<UnitId> = home.iter().filter(|u| self.ship_reaches(sim, u, e.pos) && self.ground_target_range(sim, u, e.type_id, e.pos).is_some()).map(|u| u.id).collect();
            if !units.is_empty() {
                log::debug!("{} min: P{me} fleet of {} meets ships at ({}, {})", s.minutes, units.len(), e.pos.x.to_int(), e.pos.z.to_int());
                self.defending.extend(units.iter().map(|&id| (id, now + 20 * SECOND, e.pos)));
                out.push(Command::Order { player: me, units, order: Order::Fight(e.pos), queue: false });
                return;
            }
        }

        // 2. A strike, once the ships at home are strong enough: for the enemy warships seen
        // lately, and for whatever drove the last strike back.
        let strength: i64 = home.iter().map(|u| cost(u)).sum();
        let enemy_navy: i64 = self.enemy_army.iter().filter(|a| a.3).map(|a| a.1).sum();
        let repelled = if now < self.fleet_repelled.1 + 4 * MINUTE { self.fleet_repelled.0 } else { 0 };
        let threshold = (self.p.wave_base / 3).max(400).max(enemy_navy * 13 / 10).max(repelled * 13 / 10);
        if home.len() >= 3 && !s.peace && strength >= threshold && now >= self.last_fleet_attack + self.p.wave_cooldown {
            let probe = home.iter().copied().max_by_key(|u| (cost(u), u.id.slot)).unwrap();
            if let Some(target) = self.naval_target(sim, s, post, probe) {
                let units: Vec<UnitId> = home.iter().filter(|u| self.ship_reaches(sim, u, target)).map(|u| u.id).collect();
                if units.len() >= 3 {
                    log::debug!("{} min: P{me} fleet strike of {} ships ({strength}) at ({}, {})", s.minutes, units.len(), target.x.to_int(), target.z.to_int());
                    self.last_fleet_attack = now;
                    self.fleet_anchor = None;
                    self.fleet.extend(&units);
                    out.push(Command::Order { player: me, units, order: Order::Fight(target), queue: false });
                    return;
                }
            }
        }

        // 3. The fleet at sea: back home when outgunned (to come again stronger), joined by
        // ships ready at home while the push goes well, else on to the next target.
        if !sailing.is_empty() && (now / THINK_TICKS) % 8 == me as u64 % 8 {
            let n = sailing.len() as i64;
            let sum = sailing.iter().fold(FxVec3::ZERO, |a, u| a + u.pos);
            let centre = FxVec3::new(sum.x / n, Fx::ZERO, sum.z / n);
            let strength: i64 = sailing.iter().map(|u| cost(u)).sum();
            let facing = self.opposition(s, centre);
            if facing * 10 > strength * 13 && !centre.within_xz(post, fx(900)) {
                log::debug!("{} min: P{me} fleet of {} ({strength}) falls back from {facing}", s.minutes, sailing.len());
                out.push(Command::Order { player: me, units: self.fleet.clone(), order: Order::Move(post), queue: false });
                self.fleet.clear();
                self.fleet_repelled = (facing, now);
                self.last_fleet_attack = now;
                self.fleet_anchor = None;
                return;
            }
            // Held in one place for long, shooting without wearing anything down (at what it
            // can't hit, up a bank or over a ridge): give the place up for a while, pull back
            // and go for the next.
            let around: i64 = s.enemies.iter().filter(|e| e.pos.within_xz(centre, fx(700))).map(|e| e.health.to_int()).sum();
            if self.fleet_anchor.is_none_or(|(p, _, h)| !p.within_xz(centre, fx(160)) || around * 10 < h * 9) {
                self.fleet_anchor = Some((centre, now, around));
            }
            let stalled = self.fleet_anchor.is_some_and(|(_, t, _)| now > t + 45 * SECOND) && sailing.iter().all(|u| !u.orders.is_empty());
            if stalled {
                let aim = sailing.iter().find_map(|u| match u.orders.front() {
                    Some(Order::Fight(p)) => Some(*p),
                    _ => None,
                });
                self.fleet_avoid.push((centre, now + 3 * MINUTE));
                self.fleet_avoid.extend(aim.map(|p| (p, now + 3 * MINUTE)));
                self.fleet_anchor = None;
                let back = centre + (post - centre).xz().normalize().scale(fx(500));
                let probe = sailing[0];
                let next = self.naval_target(sim, s, centre, probe);
                log::debug!("{} min: P{me} fleet of {} stalled at ({}, {}), moves on", s.minutes, sailing.len(), centre.x.to_int(), centre.z.to_int());
                let units: Vec<UnitId> = sailing.iter().map(|u| u.id).collect();
                out.push(Command::Order { player: me, units: units.clone(), order: Order::Move(back), queue: false });
                match next {
                    Some(t) => out.push(Command::Order { player: me, units, order: Order::Fight(t), queue: true }),
                    None => {
                        out.push(Command::Order { player: me, units, order: Order::Move(post), queue: true });
                        self.fleet.clear();
                    }
                }
                return;
            }
            let joiners: Vec<&Unit> = home.iter().copied().filter(|u| u.orders.is_empty() && self.ship_reaches(sim, u, centre)).collect();
            let joining: i64 = joiners.iter().map(|u| cost(u)).sum();
            if !joiners.is_empty() && facing * 3 < strength * 2 && (joiners.len() >= 2 || joining * 3 >= strength) {
                let units: Vec<UnitId> = joiners.iter().map(|u| u.id).collect();
                log::debug!("{} min: P{me} sends {} more ships to the fleet", s.minutes, units.len());
                self.fleet.extend(&units);
                out.push(Command::Order { player: me, units, order: Order::Fight(centre), queue: false });
            }
            let idle: Vec<&Unit> = sailing.iter().copied().filter(|u| u.orders.is_empty()).collect();
            // Arrived and nothing to shoot at: the place is cleared (for a while).
            if idle.len() * 2 >= sailing.len() && facing == 0 && !self.enemy_buildings.iter().any(|(_, p, _)| p.within_xz(centre, fx(500))) {
                self.cleared.push((centre, now + 3 * MINUTE));
            }
            if let Some(&probe) = idle.first() {
                match self.naval_target(sim, s, centre, probe) {
                    Some(target) => {
                        let units: Vec<UnitId> = idle.iter().filter(|u| self.ship_reaches(sim, u, target)).map(|u| u.id).collect();
                        if !units.is_empty() {
                            out.push(Command::Order { player: me, units, order: Order::Fight(target), queue: false });
                        }
                    }
                    None => {
                        let units: Vec<UnitId> = idle.iter().map(|u| u.id).collect();
                        self.fleet.retain(|id| !units.contains(id));
                        out.push(Command::Order { player: me, units, order: Order::Move(post), queue: false });
                    }
                }
            }
        }

        // 4. Idle ships at home wait off the yard.
        if (now / THINK_TICKS) % 16 != me as u64 % 16 {
            return;
        }
        let near = gathered_within(sim, home.len());
        let gather: Vec<UnitId> = home.iter().filter(|u| u.orders.is_empty() && !u.pos.within_xz(post, near) && self.ship_reaches(sim, u, post)).map(|u| u.id).collect();
        if !gather.is_empty() {
            out.push(Command::Order { player: me, units: gather, order: Order::Move(post), queue: false });
        }
    }
}
