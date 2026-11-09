//! The military: home defence against raids, attack waves, the air wing, the commander
//! and missile silos.

use super::*;

use rustc_hash::{FxHashMap, FxHashSet};

/// Squares (as a shift of px: 256) concentrated fire looks for targets in.
const FOCUS_SQUARE_SHIFT: i64 = 8;

/// A wave gathering before it goes in: idle units wait at several posts (the rally point,
/// outlying extractor fields), and sent from there straight at the target they would arrive
/// one post at a time, to be beaten one group after another. Its units, where they gather,
/// and since when.
#[derive(Clone)]
pub(crate) struct Muster {
    pub(crate) units: Vec<UnitId>,
    pub(crate) at: FxVec3,
    pub(crate) since: u64,
}

/// A wave whose units stand further apart than this (px) gathers first.
const MUSTER_SPREAD: i64 = 450;

/// An air strike under way: its targets and planes with their worth, and when it began.
#[derive(Clone)]
pub(crate) struct AirStrike {
    pub(crate) targets: Vec<(UnitId, i64)>,
    pub(crate) planes: Vec<(UnitId, i64)>,
    pub(crate) started: u64,
}

impl SkirmishAi {
    // ---- Army ---------------------------------------------------------------------------------

    /// Enemy raiders near our buildings or base, grouped: (position, strength).
    pub(crate) fn threats(&self, sim: &Sim, s: &Survey) -> Vec<(FxVec3, i64)> {
        let mut clusters: Vec<(FxVec3, i64)> = Vec::new();
        for e in &s.enemies {
            let i = &self.info[e.type_id];
            if !e.is_complete() || i.kind == Locomotion::Air || sim.unit_type(e).locomotion == Locomotion::Static {
                continue;
            }
            let armed = i.ground_dps > 0;
            if !armed && i.role != Role::Builder {
                continue;
            }
            let ours = e.pos.within_xz(self.base, fx(1000)) || self.presence.around(e.pos, 300) > 0;
            if !ours && self.ally_presence.around(e.pos, 300) == 0 {
                continue;
            }
            let w = if armed { i.worth } else { i.worth / 4 };
            let w = if ours { w } else { w / 2 };
            match clusters.iter_mut().find(|(p, _)| p.within_xz(e.pos, fx(500))) {
                Some(c) => c.1 += w,
                None => clusters.push((e.pos, w)),
            }
        }
        clusters.sort_by_key(|&(p, w)| (Reverse(w), p.x.0, p.z.0));
        clusters
    }

    pub(crate) fn run_army(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let now = s.now;
        let me = self.player;
        self.attacking.retain(|&id| sim.unit(id).is_some_and(|u| !u.dead && u.owner == me));
        self.defending.retain(|&(id, until, _)| now < until && sim.unit(id).is_some_and(|u| !u.dead && u.owner == me));
        if self.p.micro {
            self.run_repairs(sim, s, out);
        }
        // (Who is doing what, looked up by unit: an army of thousands is asked about each.)
        let repairing: FxHashSet<UnitId> = self.repairing.iter().map(|r| r.0).collect();
        let mending = |id: UnitId| repairing.contains(&id);
        // Units waiting for a transport or set down across the water are the lift's.
        let carried: FxHashSet<UnitId> = self.landed.iter().copied().chain(self.riders.iter().map(|r| r.unit)).collect();
        let lifted = |id: UnitId| carried.contains(&id);
        let fleet: FxHashSet<UnitId> = self.fleet.iter().copied().collect();
        let mut attacking: FxHashSet<UnitId> = self.attacking.iter().copied().collect();
        let mut defending: FxHashSet<UnitId> = self.defending.iter().map(|d| d.0).collect();

        // 1. Meet raids with the nearest home units (enough of them to win); an ally's call to
        // defend a spot first, with what we see of the enemy there (or enough to hold it).
        let mut threats = self.threats(sim, s);
        let asked = self.requests.defend.filter(|r| now < r.until);
        if let Some(r) = asked {
            let seen: i64 = s.enemies.iter().filter(|e| e.is_complete() && e.pos.within_xz(r.at, fx(700))).map(|e| self.info[e.type_id].worth).sum();
            threats.retain(|(p, _)| !p.within_xz(r.at, fx(500)));
            threats.insert(0, (r.at, seen.max(400)));
        }
        // Raiders at our base too many for what we have at home: our allies are called.
        if let Some(&(pos, strength)) = threats.iter().find(|(p, _)| asked.is_none_or(|r| r.at != *p)) {
            let home: i64 = s.army.iter().filter(|u| !attacking.contains(&u.id) && dist(u.pos, pos) < 2600).map(|u| self.info[u.type_id].cost).sum();
            log::debug!("{} min: P{me} raided ({strength}) at ({}, {}), {home} at home", s.minutes, pos.x.to_int(), pos.z.to_int());
            self.call_for_help(sim, now, pos, strength, home, out);
        }
        for (pos, strength) in threats.into_iter().take(3) {
            let ally_call = asked.is_some_and(|r| r.at == pos);
            let need = strength * 3 / 2 + 50;
            // Units already on their way count; the quickest to arrive go next.
            let mut have: i64 = self.defending.iter().filter(|d| d.2.within_xz(pos, fx(600))).filter_map(|d| sim.unit(d.0)).map(|u| self.info[u.type_id].cost).sum();
            if have >= need {
                continue;
            }
            let eta = |u: &Unit| dist(u.pos, pos) * 1000 / self.info[u.type_id].speed.0.max(1) * 65536 / 1000;
            let mut cands: Vec<(i64, UnitId, i64)> = s
                .army
                .iter()
                .filter(|u| Some(u.id) != self.ground_scout && !attacking.contains(&u.id) && !fleet.contains(&u.id) && !defending.contains(&u.id) && !mending(u.id) && !lifted(u.id) && dist(u.pos, pos) < if ally_call { 9000 } else { 2600 })
                .map(|u| (eta(u), u.id, self.info[u.type_id].cost))
                .collect();
            if pos.within_xz(self.base, fx(900)) {
                // The base itself: call back attackers that are still close.
                cands.extend(s.army.iter().filter(|u| attacking.contains(&u.id) && dist(u.pos, pos) < 1800).map(|u| (eta(u), u.id, self.info[u.type_id].cost)));
            }
            cands.sort_unstable();
            // (Whether we had any at all: none that can get there is another answer.)
            let had = !cands.is_empty();
            let mut sent = Vec::new();
            for (_, id, cost) in cands {
                if have >= need {
                    break;
                }
                let Some(u) = sim.unit(id) else { continue };
                if !self.unit_reaches(sim, u, pos) {
                    continue;
                }
                have += cost;
                sent.push(id);
            }
            // Sending a few at a time only feeds them to the raiders one by one: away from
            // the base, go with enough or not at all (an ally who asked gets half of that).
            if sent.is_empty() || have < need && !pos.within_xz(self.base, fx(900)) && !(ally_call && have * 2 >= need) {
                if ally_call && let Some(r) = self.requests.defend.as_mut().filter(|r| !r.answered) {
                    r.answered = true;
                    let reached = sent.len();
                    self.answers.push(if had && reached == 0 { ally::Answer::CannotReach { to: r.from } } else { ally::Answer::CannotDefend { to: r.from } });
                }
                continue;
            }
            if ally_call && let Some(r) = self.requests.defend.as_mut().filter(|r| !r.answered) {
                r.answered = true;
                self.answers.push(ally::Answer::Defending { to: r.from });
            }
            let going: FxHashSet<UnitId> = sent.iter().copied().collect();
            self.attacking.retain(|id| !going.contains(id));
            attacking.retain(|id| !going.contains(id));
            self.defending.extend(sent.iter().map(|&id| (id, now + if ally_call { 45 } else { 25 } * SECOND, pos)));
            defending.extend(sent.iter().copied());
            log::debug!("{} min: P{me} answers a raid of {strength} at ({}, {}) with {} units ({have})", s.minutes, pos.x.to_int(), pos.z.to_int(), sent.len());
            out.push(Command::Order { player: me, units: sent, order: Order::Fight(pos), queue: false });
        }

        // 2. Attack waves, with the units that can get at the enemy (on an island map the
        // ground units stay home as its guard).
        // (Warships are the fleet's: `run_navy`.)
        let dominant = self.dominant(s);
        // A wave gathering goes in once most of it is there (or it has waited long enough),
        // at a target picked afresh. (Units sent off to meet a raid meanwhile are no longer
        // its.)
        if let Some(m) = self.muster.as_mut() {
            m.units.retain(|&id| !defending.contains(&id) && !repairing.contains(&id) && sim.unit(id).is_some_and(|u| !u.dead && u.owner == me));
        }
        let gathered = self.muster.as_ref().is_some_and(|m| {
            let near = gathered_within(sim, m.units.len());
            let there = m.units.iter().filter_map(|&id| sim.unit(id)).filter(|u| u.pos.within_xz(m.at, near)).count();
            there * 4 >= m.units.len() * 3 || now > m.since + 40 * SECOND
        });
        if gathered && let Some(m) = self.muster.take() {
            let units: Vec<&Unit> = m.units.iter().filter_map(|&id| sim.unit(id)).collect();
            let finish = if dominant { self.finishing_target(sim, &units) } else { None };
            if let Some(target) = finish.or_else(|| self.asked_target(now)).or_else(|| self.pick_target(sim, &units)) {
                let sent: Vec<UnitId> = units.iter().filter(|u| self.unit_reaches(sim, u, target)).map(|u| u.id).collect();
                // (Too few left: they wait at home for the next wave.)
                if sent.len() < 3 && finish.is_none() && self.asked_target(now) == Some(target) {
                    self.cannot_reach_asked();
                }
                if sent.len() >= 3 {
                    log::debug!("{} min: P{me} wave {} of {} units gathered, goes in at ({}, {})", s.minutes, self.wave, sent.len(), target.x.to_int(), target.z.to_int());
                    self.last_attack_tick = now;
                    self.attacking.extend(&sent);
                    attacking.extend(sent.iter().copied());
                    let worth: i64 = sent.iter().filter_map(|&id| sim.unit(id)).map(|u| self.info[u.type_id].cost).sum();
                    self.answer_attack(sim, now, target, worth, out);
                    out.push(Command::Order { player: me, units: sent, order: Order::Fight(target), queue: false });
                }
            }
        }
        let mustering: FxHashSet<UnitId> = self.muster.iter().flat_map(|m| m.units.iter().copied()).collect();
        let home: Vec<&Unit> = s.army.iter().copied().filter(|u| Some(u.id) != self.ground_scout && self.info[u.type_id].kind != Locomotion::Naval && !attacking.contains(&u.id) && !defending.contains(&u.id) && !mustering.contains(&u.id) && !mending(u.id) && !lifted(u.id)).collect();
        // Each wave wants more than the last: growing waves, not a trickle.
        let schedule = (self.p.wave_base + self.p.wave_growth * self.wave as i64).min(self.p.wave_base * 4);
        let mut threshold = schedule;
        if self.asked_target(now).is_some() {
            threshold = threshold * 2 / 3;
        }
        if self.difficulty != Difficulty::Easy {
            // Enough to beat the army the enemy has shown lately: a weak enemy is pressed
            // hard, a strong one met in force; but never less than half the wave due.
            threshold = threshold.min((self.enemy_army_value() * 13 / 10).max(schedule / 2));
        }
        let total: i64 = home.iter().map(|u| self.info[u.type_id].cost).sum();
        let enemy = self.nearest_enemy_base(self.base);
        let mobile: Vec<&Unit> = if self.muster.is_none() && !s.peace && total * 2 >= threshold && now >= self.last_attack_tick + self.p.wave_cooldown {
            home.iter().copied().filter(|u| enemy.is_none_or(|e| self.unit_reaches(sim, u, e))).collect()
        } else {
            Vec::new()
        };
        let strength: i64 = mobile.iter().map(|u| self.info[u.type_id].cost).sum();
        // On an island map only the units that can cross (hovercraft, gunships) attack:
        // don't wait for a full-size wave of them.
        let threshold = if strength * 2 < total { threshold / 2 } else { threshold };
        if strength >= threshold && total >= threshold {
            // Keep a home guard when raiders have been around lately.
            let raided = self.alerts.iter().any(|(p, t)| now < t + 60 * SECOND && !p.within_xz(self.base, fx(200)));
            let keep = if raided && !dominant { strength / 5 } else { 0 };
            // Far stronger: go for what ends the game (factories, the Commander).
            let target = if dominant { self.finishing_target(sim, &mobile) } else { None };
            let target_was_finishing = target;
            if let Some(target) = target.or_else(|| self.asked_target(now)).or_else(|| self.pick_target(sim, &mobile)) {
                let mut by_home: Vec<(i64, &Unit)> = mobile.iter().map(|u| (dist(u.pos, self.base), *u)).collect();
                by_home.sort_by_key(|(d, u)| (*d, u.id.slot));
                let mut kept = 0;
                let mut sent = Vec::new();
                for (_, u) in by_home {
                    if kept < keep {
                        kept += self.info[u.type_id].cost;
                        continue;
                    }
                    if self.unit_reaches(sim, u, target) {
                        sent.push(u.id);
                    }
                }
                // (Too few reach where an ally asked: they hear so, and the waves go where they
                // can; the next wave picks its own target.)
                if sent.len() < 3 && target_was_finishing.is_none() && self.asked_target(now) == Some(target) {
                    self.cannot_reach_asked();
                }
                if sent.len() >= 3 {
                    log::debug!("{} min: P{me} wave {} of {} units (strength {strength}, kept {kept}) at ({}, {})", s.minutes, self.wave + 1, sent.len(), target.x.to_int(), target.z.to_int());
                    self.wave += 1;
                    self.last_attack_tick = now;
                    // Spread over the guard posts: they gather at the rally point first (not on
                    // Easy, whose waves are there to be beaten one by one).
                    let group: Vec<FxVec3> = sent.iter().filter_map(|&id| sim.unit(id)).map(|u| u.pos).collect();
                    let n = group.len().max(1) as i64;
                    let sum = group.iter().fold(FxVec3::ZERO, |a, &p| a + p);
                    let centre = FxVec3::new(sum.x / n, Fx::ZERO, sum.z / n);
                    let rally = self.rally;
                    let spread = group.iter().any(|p| !p.within_xz(centre, fx(MUSTER_SPREAD)));
                    if spread && self.difficulty != Difficulty::Easy && !rally.within_xz(target, fx(600)) && sent.iter().filter_map(|&id| sim.unit(id)).all(|u| self.unit_reaches(sim, u, rally)) {
                        self.muster = Some(Muster { units: sent.clone(), at: rally, since: now });
                        out.push(Command::Order { player: me, units: sent, order: Order::Fight(rally), queue: false });
                    } else {
                        self.attacking.extend(&sent);
                        self.answer_attack(sim, now, target, strength, out);
                        out.push(Command::Order { player: me, units: sent, order: Order::Fight(target), queue: false });
                    }
                }
            }
        }

        // (Those just sent, to the attack or to gather first, are home no longer: what follows
        // in this think must not take their orders back, nor count them twice.)
        let home: Vec<&Unit> = home.into_iter().filter(|u| !self.attacking.contains(&u.id) && !self.muster.as_ref().is_some_and(|m| m.units.contains(&u.id))).collect();

        // 3. Keep the attack pushing: idle attackers go for the next target near them.
        if (now / THINK_TICKS) % 20 == self.player as u64 % 20 && !self.attacking.is_empty() {
            let group: Vec<&Unit> = self.attacking.iter().filter_map(|&id| sim.unit(id)).collect();
            let n = group.len().max(1) as i64;
            let sum = group.iter().fold(FxVec3::ZERO, |a, u| a + u.pos);
            let centre = FxVec3::new(sum.x / n, Fx::ZERO, sum.z / n);
            let strength: i64 = group.iter().map(|u| self.info[u.type_id].cost).sum();
            // Outmatched: fall back and gather again (a dominant army only when badly so).
            let facing = self.opposition(s, centre);
            let outmatched = if dominant { facing > strength * 2 } else { facing * 10 > strength * 13 };
            if outmatched && !centre.within_xz(self.base, fx(900)) {
                log::debug!("{} min: P{me} attack of {} units ({strength}) falls back", s.minutes, group.len());
                let rally = self.rally;
                out.push(Command::Order { player: me, units: self.attacking.clone(), order: Order::Move(rally), queue: false });
                self.attacking.clear();
                self.last_attack_tick = now;
            }
            // Units ready at home join a push that is going well instead of waiting for the next
            // wave (all but a guard when the enemy is beaten).
            let guard = if dominant { 0 } else { home.len() / 3 };
            let joiners: Vec<&Unit> = home.iter().skip(guard).copied().filter(|u| u.orders.is_empty() && self.unit_reaches(sim, u, centre)).collect();
            // In a body, not one by one (units arriving alone are picked off on the way).
            let body = joiners.len() >= 4 || joiners.iter().map(|u| self.info[u.type_id].cost).sum::<i64>() * 3 >= strength;
            let joiners: Vec<UnitId> = joiners.iter().map(|u| u.id).collect();
            if body && facing * 3 < strength * 2 {
                log::debug!("{} min: P{me} sends {} more units to the attack", s.minutes, joiners.len());
                self.attacking.extend(&joiners);
                out.push(Command::Order { player: me, units: joiners, order: Order::Fight(centre), queue: false });
            }
            let idle: Vec<&Unit> = self.attacking.iter().filter_map(|&id| sim.unit(id)).filter(|u| u.orders.is_empty()).collect();
            // Arrived and nothing to fight: the place is cleared (for a while).
            if idle.len() * 2 >= group.len() && facing == 0 && !self.enemy_buildings.iter().any(|(_, p, _)| p.within_xz(centre, fx(400))) {
                self.cleared.push((centre, now + 3 * MINUTE));
            }
            if !idle.is_empty() {
                let finish = if dominant { self.finishing_target(sim, &idle) } else { None };
                let target = finish
                    .or_else(|| self.priority_target(sim, centre, idle[0]))
                    .or_else(|| self.weak_target(sim, centre, idle[0], strength, false))
                    .or_else(|| self.nearest_target(sim, centre, idle[0]));
                if let Some(target) = target {
                    let units: Vec<UnitId> = idle.iter().filter(|u| self.unit_reaches(sim, u, target)).map(|u| u.id).collect();
                    if !units.is_empty() {
                        out.push(Command::Order { player: me, units, order: Order::Fight(target), queue: false });
                    }
                }
            }
        }

        // 4. Idle home units wait at the guard posts (spread by unit), anti-aircraft at the
        // rally point.
        if (now / THINK_TICKS).is_multiple_of(8) {
            let rally = self.rally;
            let posts = if self.posts.is_empty() { vec![rally] } else { self.posts.clone() };
            let mut gather: Vec<(FxVec3, Vec<UnitId>)> = posts.iter().map(|&p| (p, Vec::new())).collect();
            // (How near counts as at a post: a formation of those waiting there stands wider.)
            let mut at_post = vec![0usize; posts.len()];
            for u in home.iter() {
                at_post[u.id.slot as usize % posts.len()] += 1;
            }
            for u in home.iter() {
                let k = u.id.slot as usize % posts.len();
                let post = posts[k];
                if u.orders.is_empty() && !u.pos.within_xz(post, gathered_within(sim, at_post[k])) && !defending.contains(&u.id) && self.unit_reaches(sim, u, post) {
                    gather[k].1.push(u.id);
                }
            }
            let near = gathered_within(sim, s.aa_army.len());
            let aa: Vec<UnitId> = s.aa_army.iter().filter(|u| u.orders.is_empty() && !u.pos.within_xz(rally, near) && self.unit_reaches(sim, u, rally)).map(|u| u.id).collect();
            gather.push((rally, aa));
            for (post, units) in gather {
                if !units.is_empty() {
                    out.push(Command::Order { player: me, units, order: Order::Move(post), queue: false });
                }
            }
        }
        if self.p.micro {
            self.focus_fire(sim, s, out);
        }
    }

    // ---- Battle handling ------------------------------------------------------------------------

    /// Badly hurt units leave the fight for the rally point, where a builder mends them (TA
    /// repairs cost nothing, and a unit saved is one not rebuilt); back in service once
    /// nearly whole again.
    pub(crate) fn run_repairs(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let now = s.now;
        let me = self.player;
        let rally = self.rally;
        self.repairing.retain(|&(id, since)| now < since + 3 * MINUTE && sim.unit(id).is_some_and(|u| !u.dead && u.owner == me && u.health * 10 < sim.unit_type(u).max_health * 9));
        // Ships go back to the naval post (off the shipyard), the rest to the rally point.
        let post = self.naval.as_ref().map(|n| n.post);
        let home_of = |u: &Unit| if self.info[u.type_id].kind == Locomotion::Naval { post } else { Some(rally) };
        let mut leaving: Vec<(UnitId, FxVec3)> = Vec::new();
        let carried: FxHashSet<UnitId> = self.landed.iter().copied().chain(self.riders.iter().map(|r| r.unit)).collect();
        let repairing: FxHashSet<UnitId> = self.repairing.iter().map(|r| r.0).collect();
        for u in &s.army {
            let Some(home) = home_of(u) else { continue };
            if self.info[u.type_id].kind == Locomotion::Air || u.health * 100 >= sim.unit_type(u).max_health * 35 || u.pos.within_xz(home, fx(500)) {
                continue;
            }
            if carried.contains(&u.id) {
                continue;
            }
            let reaches = if self.info[u.type_id].kind == Locomotion::Naval { self.ship_reaches(sim, u, home) } else { self.unit_reaches(sim, u, home) };
            if !repairing.contains(&u.id) && reaches {
                leaving.push((u.id, home));
            }
        }
        if !leaving.is_empty() {
            let ids: Vec<UnitId> = leaving.iter().map(|l| l.0).collect();
            let going: FxHashSet<UnitId> = ids.iter().copied().collect();
            self.attacking.retain(|id| !going.contains(id));
            self.defending.retain(|d| !going.contains(&d.0));
            self.fleet.retain(|id| !going.contains(id));
            if self.ground_scout.is_some_and(|id| going.contains(&id)) { self.ground_scout = None; }
            self.repairing.extend(ids.iter().map(|&id| (id, now)));
            for home in [Some(rally), post].into_iter().flatten() {
                let units: Vec<UnitId> = leaving.iter().filter(|l| l.1 == home).map(|l| l.0).collect();
                if !units.is_empty() {
                    out.push(Command::Order { player: me, units, order: Order::Move(home), queue: false });
                }
            }
        }
        // A builder for each hurt unit back home: one off factory duty, or an idle one, that
        // can reach it with its movement class and working range (aircraft can mend ships too).
        let mut taken: Vec<UnitId> = Vec::new();
        let ordered = ordered_units(out);
        let mended: FxHashSet<UnitId> = s
            .builders
            .iter()
            .filter_map(|b| match b.orders.front() {
                Some(&Order::Repair(id)) => Some(id),
                _ => None,
            })
            .collect();
        for &(id, _) in &self.repairing {
            let Some(u) = sim.unit(id) else { continue };
            let Some(home) = home_of(u) else { continue };
            if !u.pos.within_xz(home, fx(450)) || mended.contains(&id) {
                continue;
            }
            let helper = s
                .builders
                .iter()
                .filter(|b| !ordered.contains(&b.id) && !self.info[b.type_id].commander && !taken.contains(&b.id) && matches!(b.orders.front(), None | Some(Order::Guard(_))) && b.pos.within_xz(u.pos, fx(1200)))
                .filter(|b| self.builder_reaches(sim, b, u.pos, u.type_id))
                .min_by_key(|b| (dist(b.pos, u.pos), b.id.slot));
            if let Some(b) = helper {
                taken.push(b.id);
                out.push(Command::Order { player: me, units: vec![b.id], order: Order::Repair(id), queue: false });
            }
        }
    }

    /// Concentrated fire: every engaged unit shoots the enemy in its reach whose loss costs
    /// the enemy the most firepower per point of damage, until enough guns are on it for a
    /// quick kill, then the next one. (Left to themselves units shoot whatever is nearest,
    /// spreading their fire, and every enemy keeps shooting to the last moment.)
    pub(crate) fn focus_fire(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let me = self.player;
        let targets: Vec<(&Unit, i64)> = s
            .enemies
            .iter()
            .filter(|e| {
                let i = &self.info[e.type_id];
                e.is_complete() && i.ground_dps > 0 && i.kind != Locomotion::Air && sim.seen_by(e, me) == Seen::Visible
            })
            .map(|e| (*e, self.info[e.type_id].ground_dps * 1000 / e.health.to_int().max(1)))
            .collect();
        if targets.is_empty() {
            return;
        }
        let reach = |u: &Unit, e: &Unit, extra: i64| self.ground_target_range(sim, u, e.type_id, e.pos)
            .is_some_and(|range| e.pos.within_xz(u.pos, range + fx(extra) + sim.unit_type(e).radius));
        // Enough for a kill in about two seconds (firepower is per ten seconds).
        let enough = |e: &Unit| e.health.to_int() * 5;
        let mut given: Vec<i64> = vec![0; targets.len()];
        // The targets by square (a big battle has thousands on each side), and by unit.
        let index: FxHashMap<UnitId, usize> = targets.iter().enumerate().map(|(k, (e, _))| (e.id, k)).collect();
        let widest = targets.iter().map(|(e, _)| sim.unit_type(e).radius).max().unwrap_or(Fx::ZERO);
        let square = |p: FxVec3| ((p.x.to_int() >> FOCUS_SQUARE_SHIFT) as i32, (p.z.to_int() >> FOCUS_SQUARE_SHIFT) as i32);
        let mut squares: FxHashMap<(i32, i32), Vec<usize>> = FxHashMap::default();
        for (k, (e, _)) in targets.iter().enumerate() {
            squares.entry(square(e.pos)).or_default().push(k);
        }
        let mut shooters: Vec<(&Unit, FxVec3)> = Vec::new();
        let repairing: FxHashSet<UnitId> = self.repairing.iter().map(|r| r.0).collect();
        for u in &s.army {
            let i = &self.info[u.type_id];
            if i.kind == Locomotion::Air || i.ground_dps == 0 || repairing.contains(&u.id) || Some(u.id) == self.ground_scout {
                continue;
            }
            // Units fighting their way somewhere, idle, or already on a target.
            let dest = match u.orders.front() {
                None => u.pos,
                Some(Order::Fight(p)) => *p,
                Some(Order::Attack(Target::Unit(_))) => match u.orders.get(1) {
                    Some(Order::Fight(p)) => *p,
                    _ => u.pos,
                },
                _ => continue,
            };
            // A target still in reach is kept (switching wastes the aim and the reload).
            if let Some(Order::Attack(Target::Unit(t))) = u.orders.front() && let Some(&k) = index.get(t).filter(|&&k| reach(u, targets[k].0, 24)) {
                given[k] += i.ground_dps;
                continue;
            }
            shooters.push((u, dest));
        }
        let mut orders: Vec<(UnitId, FxVec3, Vec<UnitId>)> = Vec::new();
        let mut order_of: FxHashMap<(UnitId, FxVec3), usize> = FxHashMap::default();
        let mut released: Vec<(FxVec3, Vec<UnitId>)> = Vec::new();
        let mut released_to: FxHashMap<FxVec3, usize> = FxHashMap::default();
        for (u, dest) in shooters {
            // (Only the squares within its reach hold targets it can have.)
            let r = fx(self.info[u.type_id].range + 24) + widest;
            let (lo, hi) = (square(u.pos - FxVec3::new(r, Fx::ZERO, r)), square(u.pos + FxVec3::new(r, Fx::ZERO, r)));
            let best = (lo.1..=hi.1)
                .flat_map(|z| (lo.0..=hi.0).map(move |x| (x, z)))
                .filter_map(|q| squares.get(&q))
                .flatten()
                .map(|&k| (k, &targets[k]))
                .filter(|(k, (e, _))| given[*k] < enough(e) && reach(u, e, 24))
                // (A mission's priority targets in reach first.)
                .max_by_key(|(_, (e, priority))| (sim.is_priority_target(e.id), *priority, Reverse(e.id)))
                .map(|(k, (e, _))| (k, e.id));
            match best {
                Some((k, target)) => {
                    given[k] += self.info[u.type_id].ground_dps;
                    match order_of.get(&(target, dest)) {
                        Some(&o) => orders[o].2.push(u.id),
                        None => {
                            order_of.insert((target, dest), orders.len());
                            orders.push((target, dest, vec![u.id]));
                        }
                    }
                }
                // Chasing a target out of reach: back to the fight order.
                None if matches!(u.orders.front(), Some(Order::Attack(_))) => match released_to.get(&dest) {
                    Some(&r) => released[r].1.push(u.id),
                    None => {
                        released_to.insert(dest, released.len());
                        released.push((dest, vec![u.id]));
                    }
                },
                None => {}
            }
        }
        for (target, dest, units) in orders {
            out.push(Command::Order { player: me, units: units.clone(), order: Order::Attack(Target::Unit(target)), queue: false });
            out.push(Command::Order { player: me, units, order: Order::Fight(dest), queue: true });
        }
        for (dest, units) in released {
            out.push(Command::Order { player: me, units, order: Order::Fight(dest), queue: false });
        }
    }

    /// Where the next wave goes: a mission's priority target, else the known enemy building
    /// nearest our base that the wave can reach, else a deposit the enemy mines, else their
    /// start position.
    pub(crate) fn pick_target(&self, sim: &Sim, army: &[&Unit]) -> Option<FxVec3> {
        let probe = army.iter().max_by_key(|u| (self.info[u.type_id].cost, u.id.slot))?;
        let strength: i64 = army.iter().map(|u| self.info[u.type_id].cost).sum();
        let forced = army.len() >= self.p.army_cap * 3 / 4;
        self.priority_target(sim, self.base, probe)
            .or_else(|| self.weak_target(sim, self.base, probe, strength, forced))
            .or_else(|| if forced || self.enemy_buildings.is_empty() { self.nearest_target(sim, self.base, probe) } else { None })
    }

    /// The nearest to `from` of a mission's priority targets (`AiPriorityTarget`) we see, or
    /// buildings of them we have seen, that the army can get at: "free units looking for
    /// something to attack head for that unit first", says TA's editor (Miniement Landing's
    /// Shipyard).
    pub(crate) fn priority_target(&self, sim: &Sim, from: FxVec3, probe: &Unit) -> Option<FxVec3> {
        let me = self.player;
        // (In sight, where it is; else where we saw it, if we did: one destroyed out of our
        // sight is still a target until we look.)
        let mut known: Vec<(i64, u32, FxVec3)> = sim
            .priority_targets()
            .iter()
            .filter_map(|&id| {
                let seen = sim.unit(id).filter(|u| !u.dead && sim.seen_by(u, me) == Seen::Visible);
                let pos = match seen {
                    Some(u) if !sim.is_enemy(me, u.owner) || sim.ai_ignores(u.id) => return None,
                    Some(u) => u.pos,
                    None => self.enemy_buildings.iter().find(|b| b.0 == id)?.1,
                };
                Some((dist(from, pos), id.slot, pos))
            })
            .collect();
        known.sort_unstable_by_key(|&(d, slot, _)| (d, slot));
        known.into_iter().map(|(_, _, p)| p).find(|&p| self.unit_reaches(sim, probe, p))
    }

    /// What ends the game, nearest first: a mission's priority targets, the enemy's
    /// factories we know of, a Commander still alive, the deposits they mine, any building of
    /// theirs we have seen, their starts; not places our army just found empty.
    pub(crate) fn finishing_target(&self, sim: &Sim, army: &[&Unit]) -> Option<FxVec3> {
        let probe = army.iter().max_by_key(|u| (self.info[u.type_id].cost, u.id.slot))?;
        let from = army.iter().fold(FxVec3::ZERO, |a, u| a + u.pos);
        let from = FxVec3::new(from.x / army.len() as i64, Fx::ZERO, from.z / army.len() as i64);
        if let Some(first) = self.priority_target(sim, from, probe) {
            return Some(first);
        }
        let nearest = |mut v: Vec<FxVec3>| {
            v.sort_unstable_by_key(|p| (dist(from, *p), p.x.0, p.z.0));
            v
        };
        let factories = nearest(self.enemy_buildings.iter().filter(|(_, _, t)| self.info[*t].role == Role::Factory).map(|(_, p, _)| *p).collect());
        let mined = nearest(self.deposits.iter().filter(|d| d.owner.is_some_and(|o| sim.is_enemy(self.player, o))).map(|d| d.pos).collect());
        let buildings = nearest(self.enemy_buildings.iter().map(|(_, p, _)| *p).collect());
        factories
            .into_iter()
            .chain(self.enemy_commander.map(|(_, p, _)| p))
            .chain(mined)
            .chain(buildings)
            .chain(self.enemy_bases.iter().copied())
            .filter(|p| !self.cleared.iter().any(|(c, _)| c.within_xz(*p, fx(300))))
            .find(|&p| self.unit_reaches(sim, probe, p))
    }

    /// Enemy firepower (cost) an army at `pos` faces: the armed units in sight within 500 px
    /// and the towers we know of that cover the spot, each counted once (the threat grid
    /// stamps a unit into every cell its guns reach, so sums over it count units many times).
    pub(crate) fn opposition(&self, s: &Survey, pos: FxVec3) -> i64 {
        let units: i64 = s
            .enemies
            .iter()
            .filter(|e| {
                let i = &self.info[e.type_id];
                e.is_complete() && i.ground_dps > 0 && !matches!(i.kind, Locomotion::Air | Locomotion::Static) && e.pos.within_xz(pos, fx(500))
            })
            .map(|e| self.info[e.type_id].worth)
            .sum();
        let towers: i64 = self
            .enemy_buildings
            .iter()
            .filter(|(_, p, t)| self.info[*t].role == Role::Defense && p.within_xz(pos, fx(self.info[*t].range.max(200) + 96)))
            .map(|(_, _, t)| self.info[*t].cost)
            .sum();
        units + towers
    }

    /// Enemy firepower (cost) guarding a point: towers we have seen, and enemy units in
    /// sight now.
    pub(crate) fn defence_at(&self, pos: FxVec3) -> i64 {
        let towers: i64 = self
            .enemy_buildings
            .iter()
            .filter(|(_, p, t)| self.info[*t].role == Role::Defense && p.within_xz(pos, fx(self.info[*t].range.max(200) + 96)))
            .map(|(_, _, t)| self.info[*t].cost)
            .sum();
        towers + self.threat.at(pos)
    }

    /// What destroying an enemy building of type `t` is worth: the economy and production
    /// double, strategic weapons triple.
    pub(crate) fn target_value(&self, t: usize) -> i64 {
        let i = &self.info[t];
        match i.role {
            Role::Extractor | Role::Generator | Role::MetalMaker | Role::Factory => i.cost * 2,
            Role::Silo | Role::AntiNuke => i.cost * 3,
            _ => i.cost,
        }
        .min(4000)
    }

    /// The best enemy building for a force of `strength` to hit: worth the most, closest,
    /// and guarded by less than two thirds of that strength (unless `forced`).
    pub(crate) fn weak_target(&self, sim: &Sim, from: FxVec3, probe: &Unit, strength: i64, forced: bool) -> Option<FxVec3> {
        let mut cands: Vec<(i64, u32, FxVec3)> = Vec::new();
        for &(id, p, t) in &self.enemy_buildings {
            let value = self.target_value(t);
            let defence = self.defence_at(p);
            if !forced && defence * 3 > strength * 2 {
                continue;
            }
            cands.push((dist(from, p) + defence * 2 - value, id.slot, p));
        }
        cands.sort_unstable_by_key(|&(score, slot, _)| (score, slot));
        cands.into_iter().take(8).map(|(_, _, p)| p).find(|&p| self.unit_reaches(sim, probe, p))
    }

    pub(crate) fn nearest_target(&self, sim: &Sim, from: FxVec3, probe: &Unit) -> Option<FxVec3> {
        let mut known: Vec<(i128, u32, FxVec3)> = self.enemy_buildings.iter().map(|(id, p, _)| ((*p - from).xz().length_sq_raw(), id.slot, *p)).collect();
        known.sort_unstable_by_key(|&(d, slot, _)| (d, slot));
        for &(_, _, p) in known.iter().take(10) {
            if self.unit_reaches(sim, probe, p) {
                return Some(p);
            }
        }
        let mut mined: Vec<(i128, FxVec3)> = self.deposits.iter().filter(|d| d.owner.is_some_and(|o| sim.is_enemy(self.player, o))).map(|d| ((d.pos - from).xz().length_sq_raw(), d.pos)).collect();
        mined.sort_unstable_by_key(|&(d, p)| (d, p.x.0, p.z.0));
        for &(_, p) in mined.iter().take(6) {
            if self.unit_reaches(sim, probe, p) {
                return Some(p);
            }
        }
        self.nearest_enemy_base(from).filter(|&p| self.unit_reaches(sim, probe, p))
    }

    // ---- Air wing ---------------------------------------------------------------------------


    /// Planes strike together at the enemy's economy away from its anti-aircraft cover: the
    /// best building and the valuable ones around it in one sortie, then home. Each strike
    /// is accounted for (worth destroyed against planes lost); when strikes stop paying,
    /// no planes are built for a while.
    pub(crate) fn run_air(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let me = self.player;
        self.account_air_strike(sim, s);
        // (No strikes in a ceasefire: the planes wait at home.)
        if s.peace {
            return;
        }
        // Island wars: find the enemy Commander, then strike it with bombers massed for it.
        let doctrine = self.p.air_doctrine && self.island;
        if doctrine {
            self.run_scout(sim, s, out);
        }
        let mut ready = Vec::new();
        for p in &s.planes {
            if !p.orders.is_empty() || Some(p.id) == self.scout || Some(p.id) == self.ground_scout {
                continue;
            }
            if p.pos.within_xz(self.base, fx(900)) {
                ready.push(p.id);
            } else {
                let home = self.zone_point(sim, Zone::Back, 200);
                out.push(Command::Order { player: me, units: vec![p.id], order: Order::Move(home), queue: false });
            }
        }
        // The enemy Commander seen lately: enough bombers go for it at once; until there are,
        // they stay home for it (for four minutes at most, then they strike buildings again).
        let quarry = self.enemy_commander.filter(|&(_, _, seen)| doctrine && s.now <= seen + 90 * SECOND);
        match quarry {
            Some((commander, at, _)) if self.air_strike.is_none() => {
                let bomber = |id: &UnitId| sim.unit(*id).is_some_and(|u| self.info[u.type_id].bomber);
                let bombers = ready.iter().filter(|id| bomber(id)).count();
                if bombers >= self.commander_strike_size(sim, commander, at) {
                    log::debug!("{} min: P{me} sends {} planes ({bombers} bombers) at the enemy Commander", s.minutes, ready.len());
                    self.air_attack(sim, &ready, commander, at, false, out);
                    let cost = self.info[self.enemy_commander_type].cost;
                    let planes = ready.iter().filter_map(|&id| sim.unit(id)).map(|u| (u.id, self.info[u.type_id].cost)).collect();
                    self.air_strike = Some(AirStrike { targets: vec![(commander, cost)], planes, started: s.now });
                    self.last_air_strike = s.now;
                    self.air_strikes += 1;
                    self.commander_strikes += 1;
                    self.massing_since = None;
                    return;
                }
                let since = *self.massing_since.get_or_insert(s.now);
                if s.now < since + 4 * MINUTE {
                    ready.retain(|id| !bomber(id));
                }
            }
            Some(_) => {}
            None => self.massing_since = None,
        }
        // Three planes will do against a target with no anti-aircraft cover seen; when the
        // enemy is all but gone, any plane goes after the last buildings, straight away.
        let mop_up = self.hunting(s);
        let (min_group, gap) = if mop_up { (1, 5 * SECOND) } else { (3, 20 * SECOND) };
        let target = (ready.len() >= min_group && s.now >= self.last_air_strike + gap && self.air_strike.is_none()).then(|| self.pick_air_target(sim)).flatten();
        // Nothing of theirs left that we know of: a plane (or an idle air transport) sweeps
        // the map for the rest.
        if mop_up && target.is_none() && self.air_strike.is_none() && s.now >= self.last_hunt + MINUTE {
            let spare_lift = s
                .transports
                .iter()
                .find(|t| self.info[t.type_id].kind == Locomotion::Air && t.orders.is_empty() && t.cargo.is_empty() && !self.riders.iter().any(|r| r.transport == t.id))
                .map(|t| t.id);
            if let Some(scout) = ready.first().copied().or(spare_lift) {
                log::debug!("{} min: P{me} sweeps the map for what is left of the enemy", s.minutes);
                let from = sim.unit(scout).map_or(self.base, |u| u.pos);
                for (k, p) in self.hunt_points(sim, from).into_iter().enumerate() {
                    out.push(Command::Order { player: me, units: vec![scout], order: Order::Move(p), queue: k > 0 });
                }
                self.last_hunt = s.now;
            }
        }
        let need = |t: UnitId| if mop_up || self.enemy_buildings.iter().find(|b| b.0 == t).is_some_and(|b| self.enemy_aa.at(b.1) == 0) { min_group } else { self.p.strike_group };
        if let Some(target) = target.filter(|&t| ready.len() >= need(t)) {
            {
                self.last_air_strike = s.now;
                let targets = self.air_cluster(sim, target);
                for (k, &(t, _)) in targets.iter().enumerate() {
                    let at = self.enemy_buildings.iter().find(|b| b.0 == t).unwrap().1;
                    self.air_attack(sim, &ready, t, at, k > 0, out);
                }
                let planes = ready.iter().filter_map(|&id| sim.unit(id)).map(|u| (u.id, self.info[u.type_id].cost)).collect();
                self.air_strike = Some(AirStrike { targets, planes, started: s.now });
                self.air_strikes += 1;
            }
        }
    }

    /// Approach a remembered position before attacking an unseen unit. The simulation
    /// drops unit-target orders without sensor contact, so the approach must stand alone.
    fn air_attack(&self, sim: &Sim, planes: &[UnitId], target: UnitId, at: FxVec3, queue: bool, out: &mut Vec<Command>) {
        let hidden = sim.unit(target).is_none_or(|u| u.dead || sim.seen_by(u, self.player) == Seen::Hidden);
        if hidden {
            out.push(Command::Order { player: self.player, units: planes.to_vec(), order: Order::Move(at), queue });
        }
        out.push(Command::Order { player: self.player, units: planes.to_vec(), order: Order::Attack(Target::Unit(target)), queue: queue || hidden });
    }

    /// How many bombers a strike on the enemy Commander at `at` takes: six, and one more for
    /// each 1,500 worth of anti-aircraft cover seen round it (not counting the Commander's own
    /// weapons, which reach up too), twelve at most.
    pub(crate) fn commander_strike_size(&self, _sim: &Sim, _commander: UnitId, at: FxVec3) -> usize {
        let own = self.info[self.enemy_commander_type].worth;
        let cover = (self.enemy_aa.at(at) - own).max(0);
        (6 + cover / 1500).min(12) as usize
    }

    /// The scout plane of an island war: once there is one (built by an air plant, see
    /// `run_factories`), while the enemy Commander is unseen for two minutes it flies a loop
    /// round the enemy starts, nearest first, far enough out to keep clear of their towers.
    fn run_scout(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        if self.scout.is_some_and(|id| sim.unit(id).is_none_or(|u| u.dead || u.owner != self.player)) {
            self.scout = None;
        }
        if self.scout.is_none() {
            self.scout = s.own.iter().find(|u| Self::can_command(sim, u) && u.factory.is_none() && Some(u.id) != self.ground_scout && self.is_scout(u.type_id)).map(|u| u.id);
        }
        let Some(scout) = self.scout.and_then(|id| sim.unit(id)) else { return };
        let known = self.enemy_commander.is_some_and(|(_, _, seen)| s.now <= seen + 2 * MINUTE);
        if known || !scout.orders.is_empty() {
            return;
        }
        let mut bases = self.enemy_bases.clone();
        bases.sort_by_key(|&b| (dist(b, self.base), b.x.0, b.z.0));
        let (w, h) = sim.world.size_px();
        let mut k = 0;
        for b in bases.into_iter().take(3) {
            for i in 0..6 {
                let a = i * 65536 / 6;
                let p = FxVec3::new((b.x + ta_sim::fixed::cos(a) * 520).clamp(fx(64), w - fx(64)), Fx::ZERO, (b.z + ta_sim::fixed::sin(a) * 520).clamp(fx(64), h - fx(64)));
                out.push(Command::Order { player: self.player, units: vec![scout.id], order: Order::Move(p), queue: k > 0 });
                k += 1;
            }
        }
        log::debug!("{} min: P{} sends its scout round the enemy starts", s.minutes, self.player);
    }

    /// A unit looks at the starts where the enemy may be (`enemy_bases` not yet seen to be
    /// bases), nearest first from where it is, until none is left to look at: a scout plane of
    /// ours, else the fastest of our army at home that gets to one of them by land (the
    /// factories make one for it: see `run_factories`), else a plane at home (an island war).
    /// One lost, another is sent a minute and a half later.
    pub(crate) fn run_scouting(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let me = self.player;
        let unknown: Vec<FxVec3> = self.enemy_bases.iter().zip(&self.enemy_base_seen).filter(|&(_, &seen)| !seen).map(|(&p, _)| p).collect();
        if let Some(id) = self.ground_scout
            && !s.own.iter().any(|u| u.id == id && Self::can_command(sim, u) && !self.repairing.iter().any(|r| r.0 == id))
        {
            self.ground_scout = None;
            if !unknown.is_empty() {
                self.scout_lost = Some(s.now);
            }
        }
        if unknown.is_empty() {
            // (Nothing left to look for: it is one of the army again.)
            self.ground_scout = None;
            return;
        }
        if self.ground_scout.is_none() && self.scout_lost.is_none_or(|t| s.now >= t + 90 * SECOND) {
            let home = |u: &&&Unit| Self::can_command(sim, u) && u.factory.is_none() && u.pos.within_xz(self.base, fx(1500)) && Some(u.id) != self.scout
                && !self.attacking.contains(&u.id) && !self.defending.iter().any(|d| d.0 == u.id) && !self.landed.contains(&u.id)
                && !self.riders.iter().any(|r| r.unit == u.id) && !self.repairing.iter().any(|r| r.0 == u.id)
                && !self.muster.as_ref().is_some_and(|m| m.units.contains(&u.id));
            let fastest = |u: &&&Unit| (self.info[u.type_id].speed, Reverse(self.info[u.type_id].cost), Reverse(u.id.slot));
            let free_plane = |u: &&&Unit| u.orders.is_empty() && self.air_strike.as_ref().is_none_or(|a| !a.planes.iter().any(|p| p.0 == u.id));
            self.ground_scout = s
                .own
                .iter()
                .filter(home)
                .filter(|u| self.is_scout(u.type_id))
                .max_by_key(fastest)
                .or_else(|| {
                    s.army
                        .iter()
                        .filter(home)
                        .filter(|u| matches!(self.info[u.type_id].kind, Locomotion::Ground | Locomotion::Hover) && unknown.iter().any(|&p| self.unit_reaches(sim, u, p)))
                        .max_by_key(fastest)
                })
                .or_else(|| s.planes.iter().filter(home).filter(free_plane).max_by_key(fastest))
                .map(|u| u.id);
            if self.ground_scout.is_some() {
                log::debug!("{} min: P{me} sends a scout to look for the enemy at {} places", s.minutes, unknown.len());
            }
        }
        let Some(u) = self.ground_scout.and_then(|id| s.own.iter().find(|u| u.id == id)) else { return };
        // Its way: the places still to look at that it gets to, nearest first; given again
        // when the one it is making for has been seen to.
        let heading = match u.orders.front() {
            Some(Order::Move(p)) => Some(*p),
            _ => None,
        };
        if heading.is_some_and(|h| unknown.iter().any(|p| p.within_xz(h, fx(64)))) {
            return;
        }
        let mut tour: Vec<FxVec3> = unknown.into_iter().filter(|&p| self.unit_reaches(sim, u, p)).collect();
        if tour.is_empty() {
            // (None it gets to: another, another time.)
            self.ground_scout = None;
            return;
        }
        tour.sort_by_key(|&p| (dist(p, u.pos), p.x.0, p.z.0));
        for (k, p) in tour.into_iter().enumerate() {
            out.push(Command::Order { player: me, units: vec![u.id], order: Order::Move(p), queue: k > 0 });
        }
    }

    /// An unarmed plane that is neither a constructor nor a transport: a scout.
    pub(crate) fn is_scout(&self, t: usize) -> bool {
        let i = &self.info[t];
        i.kind == Locomotion::Air && i.role == Role::Other && i.transport == 0 && i.worker == Fx::ZERO && i.ground_dps == 0 && i.air_dps == 0
    }

    /// A sweep over the map's parts out of our sight now: the centres of a 6×6 grid not in
    /// view, nearest first from `from` (eight at most).
    fn hunt_points(&self, sim: &Sim, from: FxVec3) -> Vec<FxVec3> {
        let slot = sim.vision_slot(self.player);
        let (w, h) = sim.world.size_px();
        let mut left: Vec<FxVec3> = (0..36)
            .map(|k| FxVec3::new(w * (2 * (k % 6) + 1) / 12, Fx::ZERO, h * (2 * (k / 6) + 1) / 12))
            .filter(|&p| sim.vision.flags_at(slot, p) & ta_sim::vision::LOS == 0)
            .collect();
        let mut tour = Vec::new();
        let mut at = from;
        while let Some(i) = (0..left.len()).min_by_key(|&i| (dist(left[i], at), i)) {
            at = left.swap_remove(i);
            tour.push(at);
            if tour.len() == 8 {
                break;
            }
        }
        tour
    }

    /// The target and up to four more valuable buildings within 350 px of it that have no
    /// more anti-aircraft cover than it (the sortie takes them in turn), with their cost.
    fn air_cluster(&self, _sim: &Sim, first: UnitId) -> Vec<(UnitId, i64)> {
        let Some(&(_, at, first_type)) = self.enemy_buildings.iter().find(|b| b.0 == first) else { return Vec::new() };
        let cover = self.enemy_aa.at(at);
        let mut near: Vec<(i64, UnitId, i64)> = self
            .enemy_buildings
            .iter()
            .filter(|&&(id, p, t)| id != first && p.within_xz(at, fx(350)) && !self.enemy_frames.contains(&id) && self.info[t].role != Role::AntiAir && self.enemy_aa.at(p) <= cover)
            .map(|&(id, p, t)| (dist(p, at), id, self.info[t].cost))
            .collect();
        near.sort_by_key(|&(d, id, _)| (d, id.slot));
        let first_cost = self.info[first_type].cost;
        std::iter::once((first, first_cost)).chain(near.into_iter().take(4).map(|(_, id, c)| (id, c))).collect()
    }

    /// Ends the strike under way once its planes are home or gone (or after three minutes):
    /// the worth of the targets destroyed and of the planes lost go into the ledger (older
    /// strikes count half), and planes are not built for five minutes when the ledger shows
    /// them losing more than twice what they destroy.
    fn account_air_strike(&mut self, sim: &Sim, s: &Survey) {
        let Some(strike) = &self.air_strike else { return };
        let alive: Vec<&Unit> = strike.planes.iter().filter_map(|&(id, _)| sim.unit(id)).filter(|u| !u.dead && u.owner == self.player).collect();
        let done = alive.iter().all(|u| u.orders.is_empty() || u.pos.within_xz(self.base, fx(900)) && !u.orders.iter().any(|o| matches!(o, Order::Attack(_))));
        if !(done || s.now > strike.started + 3 * MINUTE) {
            return;
        }
        // (Destroyed: seen gone, as far as we know; the planes lost are ours to count.)
        let known = |id: UnitId| self.enemy_buildings.iter().any(|b| b.0 == id) || self.enemy_commander.is_some_and(|c| c.0 == id);
        let destroyed: i64 = strike.targets.iter().filter(|&&(id, _)| !known(id)).map(|&(_, c)| c).sum();
        let lost: i64 = strike.planes.iter().filter(|&&(id, _)| sim.unit(id).is_none_or(|u| u.dead || u.owner != self.player)).map(|&(_, c)| c).sum();
        let (d, l) = self.air_ledger;
        self.air_ledger = (d / 2 + destroyed, l / 2 + lost);
        if self.air_ledger.1 > 600 && self.air_ledger.1 > 2 * self.air_ledger.0 {
            self.air_pause_until = s.now + 5 * MINUTE;
        }
        log::debug!("P{} air strike: destroyed {destroyed}, lost {lost}; ledger {:?}", self.player, self.air_ledger);
        self.air_strike = None;
    }

    pub(crate) fn pick_air_target(&self, sim: &Sim) -> Option<UnitId> {
        // A mission's priority targets first, the nearest of those we have seen.
        let first = sim
            .priority_targets()
            .iter()
            .filter_map(|&id| self.enemy_buildings.iter().find(|b| b.0 == id))
            .filter(|b| !self.enemy_frames.contains(&b.0))
            .min_by_key(|b| (dist(b.1, self.base), b.0.slot))
            .map(|b| b.0);
        if first.is_some() {
            return first;
        }
        let mut best: Option<(i64, UnitId)> = None;
        for &(id, pos, t) in &self.enemy_buildings {
            if self.enemy_frames.contains(&id) {
                continue;
            }
            let i = &self.info[t];
            let value = match i.role {
                Role::Extractor | Role::Generator | Role::MetalMaker | Role::Silo | Role::AntiNuke => i.cost * 3,
                Role::Factory => i.cost * 2,
                Role::AntiAir => 0,
                _ => i.cost,
            };
            let aa = self.enemy_aa.at(pos);
            // The outskirts first: a base's heart hides anti-aircraft cover not yet seen.
            let outskirts = self.nearest_enemy_base(pos).map_or(0, |b| dist(pos, b).min(2000));
            let score = value * 1000 / (200 + aa * 2) * 3000 / (3000 + dist(pos, self.base)) * (1000 + outskirts) / 1000;
            if score > 0 && best.is_none_or(|(bs, bid)| (score, Reverse(id)) > (bs, Reverse(bid))) {
                best = Some((score, id));
            }
        }
        best.map(|(_, id)| id)
    }

    // ---- Commander -------------------------------------------------------------------------

    pub(crate) fn run_commander(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let Some(c) = s.commander else { return };
        let me = self.player;
        let ct = sim.unit_type(c);
        // Fall back when badly hurt with enemies close.
        if c.health * 2 < ct.max_health && s.now >= self.retreat_until && self.enemies_near(s, c.pos, 450) > 0 && !c.pos.within_xz(self.base, fx(150)) {
            self.retreat_until = s.now + 20 * SECOND;
            out.push(Command::Order { player: me, units: vec![c.id], order: Order::Move(self.base), queue: false });
            return;
        }
        // D-gun anything that gets too close.
        let eco = &sim.players[me as usize].economy;
        if eco.energy <= fx(700) || matches!(c.orders.front(), Some(Order::ManualFire(_))) {
            return;
        }
        // Mobile attackers only (buildings don't threaten the commander), and never with one
        // of our own units in the line of fire: the D-gun ball destroys everything it passes
        // through.
        let reach = sim.unit_type(c).weapons.iter().flatten()
            .find_map(|&w| { let wt = sim.weapon_types.get(w); wt.def.command_fire.then_some(wt.range) })
            .unwrap_or(fx(250)) * Fx::from_ratio(16, 10);
        let flight = |to: FxVec3| {
            let along = (to - c.pos).xz();
            let len = along.length_xz();
            if len > Fx::ZERO { c.pos + along.scale(reach / len) } else { to }
        };
        let clear_shot = |to: FxVec3| !s.own.iter().chain(s.allies.iter()).any(|f| f.id != c.id && segment_near(c.pos, flight(to), f.pos, sim.unit_type(f).radius + fx(12)));
        let threat = s
            .enemies
            .iter()
            .filter(|e| matches!(sim.unit_type(e).locomotion, Locomotion::Ground | Locomotion::Hover | Locomotion::Naval))
            .filter(|e| sim.seen_by(e, me) == Seen::Visible && e.pos.within_xz(c.pos, fx(120)))
            .filter(|e| clear_shot(e.pos))
            .min_by_key(|e| ((e.pos - c.pos).xz().length_sq_raw(), e.id.slot))
            .map(|e| e.id);
        if let Some(e) = threat {
            out.push(Command::Order { player: me, units: vec![c.id], order: Order::ManualFire(Target::Unit(e)), queue: false });
        }
    }

    // ---- Strategic missiles -------------------------------------------------------------------

    /// Salvos big enough to get through the anti-missile cover over the most valuable target.
    pub(crate) fn run_silos(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        if s.peace {
            return;
        }
        for silo in s.own.iter().filter(|u| Self::can_command(sim, u) && u.stockpile > 0 && self.info[u.type_id].role == Role::Silo) {
            if silo.orders.iter().any(|o| matches!(o, Order::ManualFire(_))) {
                continue;
            }
            let t = sim.unit_type(silo);
            let Some(weapon) = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w)).find(|w| w.def.stockpile && w.def.command_fire && w.def.targetable) else { continue };
            let aoe = weapon.aoe_radius.max(fx(64));
            let have = silo.stockpile;
            let Some((target, need)) = self.pick_nuke_target(sim, silo.pos, weapon.range, aoe, have) else { continue };
            if need <= have || silo.stockpile >= ta_sim::combat::MAX_STOCKPILE {
                for i in 0..need.min(have) {
                    out.push(Command::Order { player: self.player, units: vec![silo.id], order: Order::ManualFire(Target::Ground(target)), queue: i > 0 });
                }
            }
        }
    }

    /// Where to drop a nuke, and how many missiles it takes to get through: the enemy
    /// building cluster worth the most (minus what of ours stands near it) among those our
    /// `have` missiles can reach through anti-missile cover; else the least defended one.
    pub(crate) fn pick_nuke_target(&self, sim: &Sim, from: FxVec3, range: Fx, aoe: Fx, have: u32) -> Option<(FxVec3, u32)> {
        let me = self.player;
        let value = |u: &Unit| self.info[u.type_id].cost;
        // What we know of the enemy: the buildings we have seen (and not seen gone since), and
        // the units in sight now.
        let enemies: Vec<(FxVec3, i64)> = self
            .enemy_buildings
            .iter()
            .map(|&(_, p, t)| (p, self.info[t].cost))
            .chain(
                sim.units()
                    .filter(|u| !u.dead && sim.is_enemy(me, u.owner) && !sim.ai_ignores(u.id) && sim.unit_type(u).locomotion != Locomotion::Static && sim.seen_by(u, me) == Seen::Visible)
                    .map(|u| (u.pos, value(u))),
            )
            .collect();
        // Allies count as our own: never drop a missile on them.
        let ours: Vec<&Unit> = sim.units().filter(|u| !sim.is_enemy(me, u.owner) && !u.dead).collect();
        // Their anti-missile cover: the defences of it we have seen finished, each taken as
        // stocked (what one holds is not to be seen).
        let anti: Vec<(FxVec3, Fx)> = self
            .enemy_buildings
            .iter()
            .filter(|(id, _, _)| !self.enemy_frames.contains(id))
            .filter_map(|&(_, p, t)| self.info[t].coverage.map(|r| (p, r)))
            // Mobile defences only contribute where we can actually see them now;
            // remembered buildings stay put, but vehicles must not be tracked through fog.
            .chain(sim.units().filter(|u| !u.dead && u.is_complete() && u.carried_by.is_none()
                && sim.is_enemy(me, u.owner) && sim.unit_type(u).locomotion != Locomotion::Static
                && sim.seen_by(u, me) == Seen::Visible)
                .filter_map(|u| self.info[u.type_id].coverage.map(|r| (u.pos, r))))
            .collect();
        let candidates: Vec<FxVec3> = self.enemy_buildings.iter().map(|b| b.1).filter(|p| p.within_xz(from, range)).collect();
        let step = (candidates.len() / 120).max(1);
        let hit = aoe * 6 / 10;
        let mut best_ok: Option<(i64, FxVec3, u32)> = None;
        let mut best_any: Option<(u32, i64, FxVec3)> = None;
        for &c in candidates.iter().step_by(step) {
            let mut score: i64 = enemies.iter().filter(|e| e.0.within_xz(c, hit)).map(|e| e.1).sum();
            score -= 2 * ours.iter().filter(|o| o.pos.within_xz(c, aoe * 13 / 10)).map(|o| value(o)).sum::<i64>();
            if score <= 0 || ours.iter().any(|o| self.info[o.type_id].commander && o.pos.within_xz(c, aoe * 3 / 2)) {
                continue;
            }
            let need = 1 + anti.iter().filter(|(p, r)| p.within_xz(c, *r)).count() as u32 * ANTI_STOCK;
            if need <= have && best_ok.is_none_or(|(s, _, _)| score > s) {
                best_ok = Some((score, c, need));
            }
            if best_any.is_none_or(|(n, s, _)| (need, -score) < (n, -s)) {
                best_any = Some((need, score, c));
            }
        }
        best_ok.map(|(_, c, n)| (c, n)).or(best_any.map(|(n, _, c)| (c, n)))
    }
}
