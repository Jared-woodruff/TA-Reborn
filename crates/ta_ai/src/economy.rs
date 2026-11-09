//! The economy: what constructors build (extractors, power, factories, tech, defences,
//! strategic buildings, extractor upgrades), metal makers and factory production.

use super::*;

impl SkirmishAi {
    // ---- Economy -------------------------------------------------------------------------------

    /// Energy income we want: enough to spend our metal (TA costs run about ten energy per
    /// metal), to run the metal makers, and later to feed strategic weapons.
    pub(crate) fn energy_want(&self, s: &Survey) -> Fx {
        let spend = s.m_inc * 10 + s.maker_upkeep + fx(40);
        // Strategic weapons (tens of thousands of energy each) only on an economy that can
        // afford them: a metal-poor map needs its metal for the army.
        let strategic = if !s.adv_factory {
            0
        } else if s.minutes + 8 >= self.p.nuke_minute && s.m_inc >= fx(20) {
            1800
        } else if s.minutes >= 16 && s.m_inc >= fx(14) {
            600
        } else {
            0
        };
        spend.max(fx(strategic))
    }

    /// Metal makers follow the power: switched off when the energy store runs low
    /// (construction comes first), back on when it holds enough again (a big store need not
    /// fill up first: a healthy bank will do). By what the store holds now: on a windy map
    /// it fills and empties within seconds, and a gust missed is power thrown away.
    pub(crate) fn run_makers(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        if s.makers == 0 {
            return;
        }
        let makers = s.own.iter().filter(|u| self.info[u.type_id].role == Role::MetalMaker && Self::can_command(sim, u));
        if s.energy < (s.e_cap * 12 / 100).min(fx(800)) {
            // Keep only what the income minus other spending can pay for.
            let spare = s.e_inc - (s.e_exp - s.maker_upkeep);
            let mut used = Fx::ZERO;
            let mut off = Vec::new();
            for u in makers.filter(|u| u.activated) {
                let up = self.info[u.type_id].maker_upkeep;
                if used + up <= spare * 9 / 10 {
                    used += up;
                } else {
                    off.push(u.id);
                }
            }
            if !off.is_empty() {
                out.push(Command::SetActive { player: self.player, units: off, on: false });
            }
        } else if s.energy > (s.e_cap * 3 / 10).min(fx(1500)) {
            let on: Vec<UnitId> = makers.filter(|u| !u.activated).map(|u| u.id).collect();
            if !on.is_empty() {
                out.push(Command::SetActive { player: self.player, units: on, on: true });
            }
        }
    }

    pub(crate) fn run_builders(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        self.lanes = s.own.iter().filter(|u| self.info[u.type_id].role == Role::Factory).map(|u| Self::exit_lane(u.pos, sim.unit_type(u).footprint, u.heading)).collect();
        self.reserve_tech_site(sim, s);
        if s.now % (20 * SECOND) < THINK_TICKS {
            self.find_clutter(sim, s);
        }
        self.clutter.retain(|&f| sim.features.get(f).is_some_and(|f| f.alive));
        if let Some((p, t)) = self.tech_site {
            let fp = sim.types.get(t).footprint;
            let (x, z, hx, hz) = (p.x.to_int(), p.z.to_int(), fp.0 as i64 * 8 + 48, fp.1 as i64 * 8 + 48);
            self.lanes.push((x - hx, z - hz, x + hx, z + hz));
            self.lanes.push(Self::exit_lane(p, fp, 0));
        }
        let (mut plan, upgrading) = self.start_builders(sim, s, out);
        let ordered = ordered_units(out);
        let me = self.player;
        let power_out = self.energy_avg < Fx::from_ratio(6, 100) && s.e_inc <= s.e_exp;
        for b in &s.builders {
            // (Busy upgrading, or just told to build the better extractor: not given another
            // job in the same think, which took that order back.)
            if ordered.contains(&b.id) || self.upgrades.iter().any(|u| u.builder == b.id) || upgrading.contains(&b.id) {
                continue;
            }
            if s.commander.is_some_and(|c| c.id == b.id) && s.now < self.retreat_until {
                continue;
            }
            if self.leave_alone(s, b, power_out, out) {
                continue;
            }
            let task = self.builder_task(sim, s, b, &mut plan);
            if log::log_enabled!(log::Level::Debug) {
                let name = |t: usize| sim.types.get(t).name.clone();
                let what = match &task {
                    Some(Task::Build(t, p) | Task::ReclaimBuild(_, t, p)) => format!("build {} at ({}, {})", name(*t), p.x.to_int(), p.z.to_int()),
                    Some(t) => format!("{t:?}"),
                    None => "nothing".into(),
                };
                log::debug!("{} min {}: P{me} {} at ({}, {}) -> {what}", s.minutes, s.now, name(b.type_id), b.pos.x.to_int(), b.pos.z.to_int());
            }
            let Some(task) = task else { continue };
            self.give_task(sim, s, b, task, true, &mut plan, out);
        }
    }

    /// What the builders' jobs this think start from: the sites and jobs already in hand (from
    /// the orders the builders carry), the failures and refusals noted, and the extractor
    /// upgrades under way. The builders just told to build the better extractor come back
    /// (theirs is the job now).
    pub(crate) fn start_builders(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) -> (Plan, Vec<UnitId>) {
        self.place_failed.retain(|&(_, until)| s.now < until);
        let mut plan = Plan::default();
        for b in &s.builders {
            for o in &b.orders {
                if let Order::Build { type_id, pos } = o {
                    plan.reserved.push((*pos, fx(sim.types.get(*type_id).radius.to_int() + 16)));
                    plan.pending[self.info[*type_id].role as usize] += 1;
                    plan.tech |= self.advanced_factory(sim, *type_id);
                }
            }
        }
        for n in &s.nanoframes {
            plan.tech |= self.advanced_factory(sim, n.type_id);
        }
        self.stuck.retain(|e| sim.unit(e.0).is_some());
        self.note_refused_builds(sim, s);
        let upgrading = self.run_upgrades(sim, s, out);
        (plan, upgrading)
    }

    /// Whether a builder is better left to the job it has (or to nothing, boxed in) than given a
    /// new one this think: out in the field with enemies close it falls back home, whatever the
    /// job; a job it cannot start is dropped after a while; building under fire only feeds
    /// nanoframes to the enemy's guns; and one assisting a factory looks again now and then.
    pub(crate) fn leave_alone(&mut self, s: &Survey, b: &Unit, power_out: bool, out: &mut Vec<Command>) -> bool {
        let me = self.player;
        let commander = self.info[b.type_id].commander;
        // Out in the field with enemies close: fall back home, whatever the job.
        let fleeing = matches!(b.orders.front(), Some(Order::Move(p)) if p.within_xz(self.base, fx(64)));
        if !commander && !fleeing && !b.pos.within_xz(self.base, fx(600)) && self.enemies_near(s, b.pos, 380) >= 100 {
            if let Some(Order::Build { type_id, pos }) = b.orders.front() {
                self.failed.push((*type_id, *pos, s.now));
                self.avoid_deposit(*pos, s.now + 90 * SECOND);
            }
            out.push(Command::Order { player: me, units: vec![b.id], order: Order::Move(self.base), queue: false });
            return true;
        }
        if let Some(front) = b.orders.front() {
            if self.check_stuck(s, b, out) {
                return true;
            }
            // Building under fire only feeds nanoframes to the enemy's guns: a destroyed
            // nanoframe is replaced at once while the order stands.
            if let Order::Build { type_id, pos } = front {
                let lost_here = self.lost_spots.iter().any(|(p, t)| s.now < t + 20 * SECOND && p.within_xz(*pos, fx(64)));
                let contested = (b.build.is_none() || !pos.within_xz(self.base, fx(500))) && self.enemies_near(s, *pos, 400) > 0;
                if lost_here || contested {
                    self.failed.push((*type_id, *pos, s.now));
                    self.avoid_deposit(*pos, s.now + 90 * SECOND);
                    out.push(Command::Order { player: me, units: vec![b.id], order: Order::Stop, queue: false });
                    return true;
                }
            }
            // Assisting a factory is what builders do when there is nothing better:
            // look again now and then (soon when power has run out: the factories stall
            // without it, and hands are wanted at the power plants).
            let guarding = matches!(front, Order::Guard(_));
            let every = if power_out { 4 } else { 40 };
            if !guarding || !(s.now / THINK_TICKS + b.id.slot as u64).is_multiple_of(every) {
                return true;
            }
        }
        // Boxed in: left alone a while (any job would be refused too).
        self.boxed_in(b.id, b.pos)
    }

    /// Gives a builder its job: the orders for it, and the notes the next think reads them by
    /// (`towers`: an extractor put up far out also gets a guard tower, once it has been raided).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn give_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, task: Task, towers: bool, plan: &mut Plan, out: &mut Vec<Command>) {
        let me = self.player;
        let order = |order: Order, queue: bool| Command::Order { player: me, units: vec![b.id], order, queue };
        match task {
            Task::Build(type_id, pos) => {
                plan.reserved.push((pos, fx(sim.types.get(type_id).radius.to_int() + 16)));
                plan.pending[self.info[type_id].role as usize] += 1;
                out.push(order(Order::Build { type_id, pos }, false));
                self.issued.push((b.id, type_id, pos, s.now));
                // Outlying extractors get a guard tower once they have been raided.
                if towers && self.info[type_id].role == Role::Extractor && let Some((tower, site)) = self.outpost_tower(sim, s, b, pos, plan) {
                    plan.reserved.push((site, fx(sim.types.get(tower).radius.to_int() + 16)));
                    plan.pending[Role::Defense as usize] += 1;
                    out.push(order(Order::Build { type_id: tower, pos: site }, true));
                }
            }
            Task::ReclaimBuild(feature, type_id, pos) => {
                plan.reserved.push((pos, fx(sim.types.get(type_id).radius.to_int() + 16)));
                plan.pending[self.info[type_id].role as usize] += 1;
                out.push(order(Order::Reclaim(feature), false));
                out.push(order(Order::Build { type_id, pos }, true));
                self.issued.push((b.id, type_id, pos, s.now));
                self.issued_reclaims.push((b.id, feature, s.now));
                if self.helper {
                    plan.claims.push((b.id, Claim::Reclaim(feature)));
                }
            }
            Task::Repair(id) => {
                out.push(order(Order::Repair(id), false));
                self.issued_repairs.push((b.id, id, s.now));
                // (A hurt unit has a mender, a frame any number of hands.)
                if self.helper && sim.unit(id).is_some_and(|t| t.is_complete()) {
                    plan.claims.push((b.id, Claim::Mend(id)));
                }
            }
            Task::Guard(id) => {
                if b.orders.front() != Some(&Order::Guard(id)) {
                    out.push(order(Order::Guard(id), false));
                }
            }
            Task::Move(p) => {
                out.push(order(Order::Move(p), false));
                if self.helper && p == self.base {
                    self.issued_homes.push((b.id, s.now));
                }
            }
            Task::Reclaim(f) => {
                out.push(order(Order::Reclaim(f), false));
                self.issued_reclaims.push((b.id, f, s.now));
                if self.helper {
                    plan.claims.push((b.id, Claim::Reclaim(f)));
                }
            }
            Task::Upgrade(deposit, old, new_type) => {
                let at = self.deposits[deposit].pos;
                let back = (self.base - at).xz().normalize();
                out.push(order(Order::Move(at + back.scale(fx(110))), false));
                plan.pending[Role::Extractor as usize] += 1;
                self.upgrades.push(Upgrade { deposit, builder: b.id, old, new_type, since: s.now, reclaims: 0 });
            }
        }
    }

    /// Builders idle again within a few thinks of a build order had it refused (a site the
    /// simulation finds unreachable though our map said otherwise): remember the site as
    /// failed so it is not ordered again and again.
    fn note_refused_builds(&mut self, sim: &Sim, s: &Survey) {
        const WINDOW: u64 = 3 * THINK_TICKS;
        let me = self.player;
        self.refusals.retain(|&(_, t, _)| s.now < t + 90 * SECOND);
        let mut refused = Vec::new();
        self.issued.retain(|&(id, type_id, pos, at)| {
            let Some(b) = sim.unit(id).filter(|b| !b.dead && b.owner == me) else { return false };
            if s.now > at + WINDOW {
                return false;
            }
            if s.now > at && b.orders.is_empty() && b.build.is_none() {
                refused.push((id, b.pos, type_id, pos));
                return false;
            }
            true
        });
        for (id, at, type_id, pos) in refused {
            log::debug!("P{} build of {} at ({}, {}) refused", self.player, sim.types.get(type_id).name, pos.x.to_int(), pos.z.to_int());
            self.refusals.push((id, s.now, at));
            // (A builder boxed in is refused everything: not the site's fault.)
            if self.boxed_in(id, at) {
                continue;
            }
            self.failed.push((type_id, pos, s.now));
            if self.info[type_id].role == Role::Extractor {
                self.avoid_deposit(pos, s.now + 2 * MINUTE);
            }
        }
        // A reclaim given up at once (the builder can't get there) would be given again at
        // every turn: the feature is left alone for a while, and so is a deposit under it.
        // (The player's helper watches an order for as long as it stands, not only the first
        // moments: a builder that walks a long way and finds the feature out of reach, where
        // the simulation drops the order, has told us the same.)
        let helper = self.helper;
        let mut given_up = Vec::new();
        self.issued_reclaims.retain(|&(id, feature, at)| {
            let Some(b) = sim.unit(id).filter(|b| !b.dead && b.owner == me) else { return false };
            if s.now > at + if helper { 6 * MINUTE } else { WINDOW } {
                return false;
            }
            // (Sent on to something else since: nothing to learn from it.)
            if helper && !b.orders.is_empty() && !b.orders.iter().any(|o| *o == Order::Reclaim(feature)) {
                return false;
            }
            if s.now > at && b.orders.is_empty() && b.build.is_none() {
                if sim.features.get(feature).is_some_and(|f| f.alive) {
                    given_up.push((id, b.pos, feature));
                }
                return false;
            }
            true
        });
        // Likewise a constructor sent home that is idle again and still away: it cannot get
        // there (walled in, on another island), and is not told again for a while.
        let mut homeless = Vec::new();
        let base = self.base;
        self.issued_homes.retain(|&(id, at)| {
            let Some(b) = sim.unit(id).filter(|b| !b.dead && b.owner == me) else { return false };
            if s.now > at + 6 * THINK_TICKS {
                return false;
            }
            if s.now > at && b.orders.is_empty() && b.build.is_none() {
                if !b.pos.within_xz(base, fx(400)) {
                    homeless.push(id);
                }
                return false;
            }
            true
        });
        self.homeless.retain(|&(_, until)| s.now < until);
        for id in homeless {
            self.homeless.retain(|&(h, _)| h != id);
            self.homeless.push((id, s.now + 2 * MINUTE));
        }
        self.unreachable_features.retain(|&(_, until)| s.now < until);
        // Likewise help for a unit that can't be got to (a nanoframe walled in, say).
        let mut out_of_reach = Vec::new();
        self.issued_repairs.retain(|&(id, target, at)| {
            let Some(b) = sim.unit(id).filter(|b| !b.dead && b.owner == me) else { return false };
            if s.now > at + WINDOW {
                return false;
            }
            if s.now > at && b.orders.is_empty() && b.build.is_none() {
                // (One being taken apart has nothing more for it once mended: no sign of a
                // place it cannot get to.)
                if sim.unit(target).is_some_and(|t| !t.dead && !sim.being_taken_apart(t) && (!t.is_complete() || t.health < sim.unit_type(t).max_health)) {
                    out_of_reach.push((id, b.pos, target));
                }
                return false;
            }
            true
        });
        self.unreachable_units.retain(|&(_, until)| s.now < until);
        for (id, at, target) in out_of_reach {
            self.refusals.push((id, s.now, at));
            if !self.boxed_in(id, at) {
                self.unreachable_units.push((target, s.now + 2 * MINUTE));
            }
        }
        for (id, at, feature) in given_up {
            self.refusals.push((id, s.now, at));
            if self.boxed_in(id, at) {
                continue;
            }
            log::debug!("P{} reclaim of feature {feature} given up", self.player);
            // (The helper remembers longer each time: three minutes, twelve, forty-eight.)
            let memory = if self.helper {
                let k = match self.feature_strikes.iter().position(|&(f, _)| f == feature) {
                    Some(k) => k,
                    None => {
                        self.feature_strikes.push((feature, 0));
                        self.feature_strikes.len() - 1
                    }
                };
                let strikes = self.feature_strikes[k].1;
                self.feature_strikes[k].1 = strikes.saturating_add(1);
                3 * MINUTE << (2 * strikes.min(4))
            } else {
                3 * MINUTE
            };
            self.unreachable_features.push((feature, s.now + memory));
            for d in &mut self.deposits {
                if d.wreck == Some(feature) {
                    d.avoid_until = d.avoid_until.max(s.now + memory);
                }
            }
        }
    }

    /// A feature a builder lately could not get to.
    pub(crate) fn unreachable_feature(&self, feature: usize) -> bool {
        self.unreachable_features.iter().any(|&(f, _)| f == feature)
    }

    /// A builder (standing at `at`) whose orders have been refused three times in the last
    /// minute and a half without its moving from the spot: it can't get anywhere (walled in),
    /// and new jobs would only be refused too.
    pub(crate) fn boxed_in(&self, id: UnitId, at: FxVec3) -> bool {
        self.refusals.iter().filter(|&&(b, _, p)| b == id && p.within_xz(at, fx(24))).count() >= 3
    }

    pub(crate) fn avoid_deposit(&mut self, pos: FxVec3, until: u64) {
        for d in &mut self.deposits {
            if d.pos.within_xz(pos, fx(48)) {
                d.avoid_until = d.avoid_until.max(until);
            }
        }
    }

    /// Stops builders that have been trying to start the same job for a long time without
    /// getting anywhere (blocked or unreachable site), and avoids the site for a while.
    pub(crate) fn check_stuck(&mut self, s: &Survey, b: &Unit, out: &mut Vec<Command>) -> bool {
        let Some(front) = b.orders.front() else { return false };
        let watched = matches!(front, Order::Build { .. } | Order::Reclaim(_) | Order::Repair(_));
        if !watched || b.build.is_some() || b.in_build_stance {
            self.stuck.retain(|e| e.0 != b.id);
            return false;
        }
        let Some(e) = self.stuck.iter_mut().find(|e| e.0 == b.id) else {
            self.stuck.push((b.id, front.clone(), b.pos, s.now));
            return false;
        };
        if e.1 != *front || !b.pos.within_xz(e.2, fx(40)) {
            *e = (b.id, front.clone(), b.pos, s.now);
            return false;
        }
        if s.now < e.3 + STUCK_TICKS {
            return false;
        }
        if let Order::Build { type_id, pos } = front {
            self.failed.push((*type_id, *pos, s.now));
            self.avoid_deposit(*pos, s.now + 2 * MINUTE);
        }
        self.stuck.retain(|e| e.0 != b.id);
        out.push(Command::Order { player: self.player, units: vec![b.id], order: Order::Stop, queue: false });
        true
    }

    /// How the economy stands this think, as the builders' choices read it.
    pub(crate) fn ledger(&self, s: &Survey) -> Ledger {
        let want_e = self.energy_want(s);
        // Keep the energy store between a fifth and most of the way full: below, more
        // power; above, metal makers turn the surplus into metal. (A big store a fifth full
        // still holds plenty: power is short only when little is banked; and a store that
        // keeps filling up in the gusts wants room, not more plants.)
        let banked = self.energy_banked(s);
        let overflowing = self.brimming > Fx::from_ratio(1, 10);
        let e_low = self.energy_avg < Fx::from_ratio(2, 10) && banked < fx(2500) || s.e_inc < want_e && self.energy_avg < Fx::from_ratio(5, 10) && banked < fx(1500) && !overflowing;
        let e_crit = self.energy_avg < Fx::from_ratio(6, 100) && s.e_inc <= s.e_exp;
        // Metal holds everything up while power piles up: that power should become metal
        // (makers, wrecks), and more hands at the factories would change nothing.
        let metal_starved = s.minutes >= 4 && self.metal_avg < Fx::from_ratio(8, 100);
        let power_spare = self.power_spare(s);
        Ledger { e_low, e_crit, overflowing, metal_starved, power_spare }
    }

    /// The next job for an idle builder. Economy first: extractors, power and factories,
    /// then tech, defences, metal makers and strategic buildings; otherwise help.
    pub(crate) fn builder_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, plan: &mut Plan) -> Option<Task> {
        let bi = self.info[b.type_id].clone();
        // Buildings for where the builder works (land, or water for ships); extractors and
        // power may be either, whichever suits the site (underwater deposits, tides).
        let naval = bi.kind == Locomotion::Naval;
        let all_opts: Vec<usize> = sim.unit_type(b).build_options.iter().copied().filter(|&o| sim.is_allowed(o)).collect();
        let opts: Vec<usize> = all_opts.iter().copied().filter(|&o| self.info[o].land != naval).collect();
        let commander = bi.commander;
        let t2 = bi.tech >= 2;
        let pending = |r: Role| plan.pending[r as usize];
        let have = |r: Role| s.count[r as usize] + plan.pending[r as usize];
        let Ledger { e_low, e_crit, overflowing, metal_starved, power_spare } = self.ledger(s);
        // The Commander builds near home; its first few extractors a little further out when
        // there are no deposits round the start (it would wait for constructors otherwise,
        // with nothing coming in).
        let home = if !commander {
            None
        } else if s.minutes < 6 && have(Role::Extractor) < 3 {
            Some(1000)
        } else {
            Some(650)
        };
        // Builders only get this far down without an extractor job, so any of them may
        // build power when it runs short.
        let gen_limit = 1 + s.builders_t1 + t2 as usize;

        if commander && s.minutes < 5 {
            // The opening: an extractor, power, the first factory, then more of both.
            if have(Role::Extractor) < 1 && let Some(t) = self.extractor_task(sim, s, b, &all_opts, home, plan) {
                return Some(t);
            }
            if have(Role::Generator) < 1 && pending(Role::Generator) == 0 && let Some(t) = self.energy_task(sim, s, b, &all_opts, plan) {
                return Some(t);
            }
            if have(Role::Factory) == 0 {
                if let Some(t) = self.factory_task(sim, s, b, &opts, plan) {
                    return Some(t);
                }
                // No room on land for it (a start at sea): the shipyard comes first.
                if let Some(t) = self.shipyard_task(sim, s, b, plan) {
                    return Some(t);
                }
            }
            if have(Role::Extractor) < 3 && let Some(t) = self.extractor_task(sim, s, b, &all_opts, home, plan) {
                return Some(t);
            }
            if have(Role::Generator) < 3 && pending(Role::Generator) == 0 && let Some(t) = self.energy_task(sim, s, b, &all_opts, plan) {
                return Some(t);
            }
        }
        if e_crit && pending(Role::Generator) < 2 && let Some(t) = self.energy_task(sim, s, b, &all_opts, plan) {
            return Some(t);
        }
        if !self.paused.is_empty() && let Some(t) = self.yard_tower_task(sim, s, b, &opts, plan) {
            return Some(t);
        }
        // Constructors that fly or hover: first the metal no other constructor gets to.
        if self.expander(b.type_id) && self.far_deposits(s) > 0 && let Some(t) = self.extractor_task(sim, s, b, &all_opts, None, plan) {
            return Some(t);
        }
        if have(Role::Factory) == 0 && let Some(t) = self.factory_task(sim, s, b, &opts, plan) {
            return Some(t);
        }
        if let Some(t) = self.shipyard_task(sim, s, b, plan) {
            return Some(t);
        }
        // An island start: a plant for units that get over by themselves (hovercraft,
        // amphibians); failing that, planes (and transports), early when not even a fleet
        // could get at the enemy.
        if pending(Role::Factory) == 0 && !s.factory_types.is_empty() && s.count[Role::Factory as usize] < self.p.max_factories && !commander {
            let types = s.factory_types.clone();
            let crossing = types.iter().any(|&f| !matches!(self.info[f].kind, Locomotion::Air | Locomotion::Naval) && self.factory_reaches(sim, f));
            let by_sea = self.naval.as_ref().is_some_and(|n| n.reaches);
            let (minute, income) = if by_sea { (7, 10) } else { (4, 5) };
            if !crossing && s.minutes >= minute && s.m_inc >= fx(income) && let Some(t) = self.crossing_factory_task(sim, s, b, &opts, plan) {
                return Some(t);
            }
            let stranded = !crossing && !types.iter().any(|&f| self.info[f].kind == Locomotion::Air);
            if stranded && s.minutes >= minute && s.m_inc >= fx(income) && let Some(t) = self.factory_task(sim, s, b, &opts, plan) {
                return Some(t);
            }
        }
        // Metal on other islands and nothing that makes constructors able to get there: a
        // plant for them (planes, usually).
        if s.minutes >= 4 && s.m_inc >= fx(6) && pending(Role::Factory) == 0 && !commander && s.count[Role::Factory as usize] < self.p.max_factories && self.p.expanders > 0 {
            let makes = s.factory_types.iter().any(|&f| sim.types.get(f).build_options.iter().any(|&o| self.expander(o)));
            if !makes && self.far_deposits(s) >= 4 && let Some(t) = self.cheapest_factory_task(sim, s, b, &opts, plan, |ai, list| list.iter().any(|&o| ai.expander(o))) {
                return Some(t);
            }
        }
        // Land units stranded and nothing to carry them across: a factory that makes
        // transports.
        if s.minutes >= 6 && s.m_inc >= fx(8) && pending(Role::Factory) == 0 && !commander && s.count[Role::Factory as usize] <= self.p.max_factories && self.want_transports(sim, s) > 0 {
            let makes = s.factory_types.iter().any(|&f| self.pick_transport(sim, s, &sim.types.get(f).build_options).is_some());
            if !makes && let Some(t) = self.transport_factory_task(sim, s, b, &opts, plan) {
                return Some(t);
            }
        }
        // Tech up.
        if !t2 && !s.adv_factory && !plan.tech && s.minutes >= self.p.tech_minute && (s.m_inc >= fx(self.p.tech_income) || s.metal >= fx(1200)) && let Some(t) = self.adv_factory_task(sim, s, b, &opts, plan) {
            plan.tech = true;
            return Some(t);
        }
        // Geothermal vents are the best power there is (when power is what we need).
        if (e_low || s.minutes >= 4 && self.energy_avg < Fx::from_ratio(6, 10) && !s.own.iter().any(|u| sim.types.get(u.type_id).needs_geo)) && pending(Role::Generator) < gen_limit && let Some(t) = self.geo_task(sim, s, b, &opts, plan) {
            return Some(t);
        }
        // An army too small for what it guards: help the factories rather than start
        // optional projects (towers, storage, upgrades, missiles).
        let army_first = s.minutes >= 6 && s.army_value * 100 < s.building_value * self.p.army_ratio && s.army.len() < self.p.army_cap;
        // More factories as income grows: the army is what keeps the economy alive.
        let per = match self.difficulty {
            Difficulty::Easy => 14,
            Difficulty::Normal => 8,
            Difficulty::Hard => 8,
        };
        // Metal piling up: the factories can't keep up with the income. More of them than
        // usual to spend it.
        let metal_banked = s.minutes >= 4 && self.metal_avg > Fx::from_ratio(6, 10);
        let spare = if self.difficulty != Difficulty::Easy && metal_banked && s.m_inc >= fx(15) { (s.m_inc.to_int() / 12).min(6) as usize } else { 0 };
        let want_factories = (1 + (s.m_inc.to_int() / per) as usize + army_first as usize).min(self.p.max_factories + spare);
        let factory_room = s.minutes >= 4 && !commander && s.count[Role::Factory as usize] + pending(Role::Factory) < want_factories && s.army.len() < self.p.army_cap;
        // Then a factory comes before more extractors (one going up at a time): more income
        // would only pile up too.
        if metal_banked && factory_room && pending(Role::Factory) == 0 && let Some(t) = self.factory_task(sim, s, b, &opts, plan) {
            return Some(t);
        }
        // Out of metal with power to spare: a metal maker first (it costs no metal and pays
        // at once; an extractor far off waits for the very metal that is missing), and room
        // in store for the gusts.
        if metal_starved && power_spare {
            if let Some(t) = self.maker_task(sim, s, b, &opts, plan, true) {
                return Some(t);
            }
            if overflowing && let Some(t) = self.storage_task(sim, s, b, &opts, plan) {
                return Some(t);
            }
        }
        // Metal piling up in storage: more extractors won't help (on a metal world the spots
        // never run out); spend it instead.
        let metal_rich = self.metal_avg > Fx::from_ratio(8, 10) && s.m_inc >= fx(20);
        if !t2 && !metal_rich {
            if let Some(t) = self.extractor_task(sim, s, b, &all_opts, home, plan) {
                return Some(t);
            }
        } else if t2 && !metal_rich && (s.m_inc >= fx(10) || s.metal >= fx(1500)) && pending(Role::Extractor) < 2 {
            // Moho mines on free deposits, then in place of our small extractors.
            if let Some(t) = self.extractor_task(sim, s, b, &all_opts, home, plan) {
                return Some(t);
            }
            if let Some(t) = self.upgrade_task(sim, s, b, &opts) {
                return Some(t);
            }
        }
        if metal_starved && let Some(t) = self.wreck_task(sim, b, plan) {
            return Some(t);
        }
        // With advanced constructors around, power comes from big plants (one building
        // instead of fifty); small ones only when it is running out.
        let big_power = s.builders_t2 > 0 && s.minutes >= 15;
        if e_low && pending(Role::Generator) < gen_limit && (t2 || !big_power || self.energy_avg < Fx::from_ratio(1, 10)) && let Some(t) = self.energy_task(sim, s, b, &all_opts, plan) {
            return Some(t);
        }
        if t2 && !army_first {
            if let Some(t) = self.strategic_task(sim, s, b, &opts, plan) {
                return Some(t);
            }
            if let Some(t) = self.upgrade_task(sim, s, b, &opts) {
                return Some(t);
            }
        } else if t2 {
            // Power still comes first for advanced constructors.
            if let Some(t) = self.fusion_task(sim, s, b, &opts, plan) {
                return Some(t);
            }
        }
        if let Some(t) = self.aa_task(sim, s, b, &opts, plan) {
            return Some(t);
        }
        if s.minutes >= 3 && s.radars < 1 + (s.minutes >= 10) as usize {
            // A radar tower: raiders seen coming can be met.
            let radar = opts.iter().copied().filter(|&o| self.info[o].role == Role::Other && !sim.types.get(o).radar.is_zero()).min_by_key(|&o| (self.info[o].cost, o));
            if let Some(r) = radar && let Some(p) = self.place(sim, b, r, Zone::Front, 220, plan) {
                return Some(Task::Build(r, p));
            }
        }
        if factory_room && let Some(t) = self.factory_task(sim, s, b, &opts, plan) {
            return Some(t);
        }
        if let Some(t) = self.outpost_task(sim, s, b, &opts, plan) {
            return Some(t);
        }
        // An army too small for its base, metal too short for new projects (every project
        // takes a share of what there is, and it is the army's), or piling up faster than the
        // factories spend it: help the factories.
        if (army_first || metal_starved || metal_banked) && !commander && let Some(f) = self.factory_to_help(sim, s, b, i64::MAX) {
            return Some(Task::Guard(f));
        }
        if let Some(t) = self.defense_task(sim, s, b, &opts, plan) {
            return Some(t);
        }

        if let Some(t) = self.maker_task(sim, s, b, &opts, plan, false) {
            return Some(t);
        }
        // Short of metal with every deposit taken: turn power into metal, building the
        // power first where the surplus can't run another maker yet.
        if self.metal_avg < Fx::from_ratio(25, 100) && s.minutes >= 4 && s.makers < self.p.max_makers && (t2 || !big_power) {
            if let Some(t) = self.maker_task(sim, s, b, &opts, plan, true) {
                return Some(t);
            }
            if pending(Role::Generator) < gen_limit && self.energy_avg < Fx::from_ratio(8, 10) && !overflowing && let Some(t) = self.energy_task(sim, s, b, &all_opts, plan) {
                return Some(t);
            }
        }
        if let Some(t) = self.storage_task(sim, s, b, &opts, plan) {
            return Some(t);
        }
        // The Commander's hands (and anyone's, with metal piling up) are worth more at the
        // factories than tidying the base; but not with metal short (metal goes to the work
        // by its builders' hands: the extractors would get less of what little there is).
        let reach = if commander { 700 } else { 1600 };
        if commander && self.metal_avg >= Fx::from_ratio(1, 10) || metal_banked {
            if let Some(t) = self.finish_task(sim, s, b, reach) {
                return Some(t);
            }
            if let Some(f) = self.factory_to_help(sim, s, b, reach) {
                return Some(Task::Guard(f));
            }
        }
        if let Some(t) = self.clear_task(sim, b, plan) {
            return Some(t);
        }
        self.assist_task(sim, s, b, reach)
    }

    /// The factory a builder should help: one with work queued that it can get to, near it,
    /// with few helpers already (they spread over the factories instead of crowding the
    /// nearest one).
    pub(crate) fn factory_to_help(&self, sim: &Sim, s: &Survey, b: &Unit, max_dist: i64) -> Option<UnitId> {
        s.factories
            .iter()
            .filter(|f| !f.factory_queue.is_empty() && dist(f.pos, b.pos) < max_dist && self.unit_reaches(sim, b, f.pos))
            .min_by_key(|f| {
                let helpers = s.builders.iter().filter(|h| h.id != b.id && h.orders.front() == Some(&Order::Guard(f.id))).count() as i64;
                (dist(f.pos, b.pos) + helpers * 400, f.id.slot)
            })
            .map(|f| f.id)
    }

    /// Energy storage once the economy is big, or sooner when the store keeps brimming: wind
    /// and tides come in gusts, and the store holds what can't be used at once.
    pub(crate) fn storage_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        let wanted = if s.minutes >= 4 && self.brimming > Fx::from_ratio(2, 10) {
            (1 + s.minutes as usize / 12).min(3)
        } else if s.minutes >= 8 && s.e_inc >= fx(250) && self.energy_avg > Fx::from_ratio(7, 10) {
            1 + s.minutes as usize / 20
        } else {
            0
        };
        if s.count[Role::Storage as usize] + plan.pending[Role::Storage as usize] >= wanted {
            return None;
        }
        let st = opts.iter().copied().filter(|&o| self.info[o].role == Role::Storage && !sim.types.get(o).energy_storage.is_zero()).min_by_key(|&o| (self.info[o].cost, o))?;
        let p = self.place(sim, b, st, Zone::Back, 200, plan)?;
        Some(Task::Build(st, p))
    }

    /// Blocking trees and rocks around our factories and the base centre: vehicles need
    /// room to get out and about (and reclaiming them pays a little).
    pub(crate) fn find_clutter(&mut self, sim: &Sim, s: &Survey) {
        let factories: Vec<FxVec3> = s.own.iter().filter(|u| self.info[u.type_id].role == Role::Factory).map(|u| u.pos).collect();
        self.clutter.clear();
        for (i, f) in sim.features.iter().enumerate() {
            if !f.alive {
                continue;
            }
            let def = &sim.data.features[f.def];
            if !def.blocking || !def.reclaimable || def.is_metal_deposit() || def.geothermal {
                continue;
            }
            if f.pos.within_xz(self.base, fx(320)) || factories.iter().any(|p| p.within_xz(f.pos, fx(220))) {
                self.clutter.push(i);
            }
        }
    }

    pub(crate) fn clear_task(&mut self, sim: &Sim, b: &Unit, plan: &mut Plan) -> Option<Task> {
        if !sim.unit_type(b).def.can_reclaim {
            return None;
        }
        let (k, f) = self
            .clutter
            .iter()
            .enumerate()
            .filter(|&(_, &f)| plan.free(sim.features[f].pos, fx(8)) && !self.unreachable_feature(f))
            .min_by_key(|&(_, &f)| (dist(sim.features[f].pos, b.pos), f))
            .map(|(k, &f)| (k, f))?;
        if dist(sim.features[f].pos, b.pos) > 900 || !self.unit_reaches(sim, b, sim.features[f].pos) {
            return None;
        }
        self.clutter.remove(k);
        plan.reserved.push((sim.features[f].pos, fx(8)));
        Some(Task::Reclaim(f))
    }

    /// Metal lying about while we are starved of it: the richest wreck near home that no one
    /// is reclaiming yet and no enemy covers.
    pub(crate) fn wreck_task(&self, sim: &Sim, b: &Unit, plan: &Plan) -> Option<Task> {
        if !sim.unit_type(b).def.can_reclaim {
            return None;
        }
        // (The player's helper keeps to its own reach of home, the Commander's short, off the
        // enemy's ground, and to a wreck no other builder is at.)
        let reach = if self.helper && self.info[b.type_id].commander { crate::duty::COMMANDER_REACH } else { 1400 };
        let (_, f, pos) = sim
            .features
            .iter()
            .enumerate()
            .filter(|&(i, f)| f.alive && f.pos.within_xz(self.base, fx(reach)) && plan.free(f.pos, fx(8)) && !self.unreachable_feature(i))
            .filter(|&(i, f)| !self.helper || sim.explored(self.player, f.pos) && !self.enemy_ground(f.pos) && !plan.claimed(b.id, Claim::Reclaim(i)))
            .filter_map(|(i, f)| {
                let d = &sim.data.features[f.def];
                let metal = (Fx::from_f64(d.metal) * (Fx::ONE - f.reclaimed)).to_int();
                (d.reclaimable && metal >= 40 && self.threat.at(f.pos) == 0).then(|| (metal * 8 - dist(b.pos, f.pos), i, f.pos))
            })
            .max_by_key(|&(score, i, _)| (score, Reverse(i)))?;
        // (Two builders on one wreck only reclaim it sooner.)
        self.unit_reaches(sim, b, pos).then_some(Task::Reclaim(f))
    }

    /// The cheapest option of a role (by cost, then id).
    pub(crate) fn pick_cheapest(&self, opts: &[usize], role: Role) -> Option<usize> {
        opts.iter().copied().filter(|&o| self.info[o].role == role).min_by_key(|&o| (self.info[o].cost, o))
    }

    /// The option of a role with the best combat value.
    pub(crate) fn pick_best(&self, opts: &[usize], role: Role) -> Option<usize> {
        opts.iter().copied().filter(|&o| self.info[o].role == role).max_by_key(|&o| (self.info[o].value.max(self.info[o].air_dps * self.info[o].hp / self.info[o].cost), Reverse(o)))
    }

    pub(crate) fn extractor_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], max_dist: Option<i64>, plan: &Plan) -> Option<Task> {
        // Extraction per metal value is what matters; cheapest among the best. Deposits on
        // land take a land extractor, those under water an underwater one.
        let best = |water: bool| {
            opts.iter()
                .copied()
                .filter(|&o| self.info[o].role == Role::Extractor && self.info[o].land != water && !sim.types.get(o).needs_geo)
                .max_by_key(|&o| (sim.types.get(o).extracts_metal.0, Reverse(self.info[o].cost), Reverse(o)))
        };
        let exts = [best(false), best(true)];
        if exts == [None, None] {
            return None;
        }
        let (_, site, wreck, ext) = self.pick_deposit(sim, s, b, exts, max_dist, plan)?;
        Some(match wreck {
            Some(f) => Task::ReclaimBuild(f, ext, site),
            None => Task::Build(ext, site),
        })
    }

    /// Constructors that fly or hover (they get to other islands).
    pub(crate) fn expander(&self, t: usize) -> bool {
        let i = &self.info[t];
        i.role == Role::Builder && i.builds_mex && !i.commander && matches!(i.kind, Locomotion::Air | Locomotion::Hover)
    }

    /// Deep in enemy land: farther from our base than from the enemy's (by the level's
    /// reach), and not right by ours.
    pub(crate) fn enemy_ground(&self, pos: FxVec3) -> bool {
        let db = dist(pos, self.base);
        self.nearest_enemy_base(pos).is_some_and(|e| db > 700 && db * 100 > dist(pos, e) * self.p.reach_pct)
    }

    /// Marks the deposits on dry land that none of our ground constructors can get to (on
    /// another island): work for constructors that fly or hover. (Nothing is far before
    /// there are ground constructors to go by.)
    pub(crate) fn mark_far_deposits(&mut self, sim: &Sim, s: &Survey) {
        let mut walkers: Vec<(ta_sim::world::MoveSpec, ta_sim::world::Region, i32)> = Vec::new();
        for b in &s.builders {
            let i = &self.info[b.type_id];
            if i.commander || !i.builds_mex || i.kind != Locomotion::Ground {
                continue;
            }
            let Some(sp) = i.spec else { continue };
            let ext = sim.unit_type(b).build_options.iter().copied().find(|&o| self.info[o].role == Role::Extractor && self.info[o].land && !sim.types.get(o).needs_geo);
            let Some(ext) = ext else { continue };
            let w = (sp, map::region_at(&sim.world, &sp, b.pos), Self::build_slack(sim, b, ext));
            if !walkers.contains(&w) {
                walkers.push(w);
            }
        }
        let sea = sim.world.sea_level;
        for d in &mut self.deposits {
            let dry = sim.world.height_at(d.pos.x, d.pos.z) >= sea;
            d.far = dry && !walkers.is_empty() && !walkers.iter().any(|&(sp, r, slack)| d.reachable(&sim.world, &sp, r, slack));
        }
    }

    /// Free far deposits worth going for: not lost there lately, not under fire, not deep
    /// in enemy land.
    pub(crate) fn far_deposits(&self, s: &Survey) -> usize {
        self.deposits.iter().filter(|d| d.far && d.owner.is_none() && d.avoid_until <= s.now && self.threat.at(d.pos) == 0 && !self.enemy_ground(d.pos)).count()
    }

    /// The best free deposit for builder `b`: close to it and to our base, richer first,
    /// not taken, not recently lost, not under fire, not deep in enemy land, reachable. `exts`
    /// are the extractors for deposits on land and under water; the one used comes back.
    pub(crate) fn pick_deposit(&mut self, sim: &Sim, s: &Survey, b: &Unit, exts: [Option<usize>; 2], max_dist: Option<i64>, plan: &Plan) -> Option<(usize, FxVec3, Option<usize>, usize)> {
        let can_reclaim = sim.unit_type(b).def.can_reclaim;
        let spec = self.info[b.type_id].spec;
        // Ground constructors can't get to the far deposits; those that fly or hover go
        // there first.
        let expander = self.expander(b.type_id);
        let walker = !expander && !self.info[b.type_id].commander && self.info[b.type_id].kind == Locomotion::Ground;
        let region = spec.map(|sp| map::region_at(&sim.world, &sp, b.pos));
        let sea = sim.world.sea_level;
        let ext_for = |d: &Deposit| exts[(sim.world.height_at(d.pos.x, d.pos.z) < sea) as usize];
        let mut cands: Vec<(i64, usize)> = Vec::new();
        for (i, d) in self.deposits.iter().enumerate() {
            let Some(ext) = ext_for(d) else { continue };
            let ext_r = fx(sim.types.get(ext).radius.to_int() + 8);
            if d.owner.is_some() || d.avoid_until > s.now || !plan.free(d.pos, ext_r) || walker && d.far || self.helper && !sim.explored(self.player, d.pos) {
                continue;
            }
            if let Some(w) = d.wreck && (!can_reclaim || !sim.features.get(w).is_some_and(|f| f.alive && sim.data.features[f.def].reclaimable) || self.unreachable_feature(w)) {
                continue;
            }
            let db = dist(d.pos, self.base);
            if max_dist.is_some_and(|m| db > m) {
                continue;
            }
            if self.enemy_ground(d.pos) || self.threat.at(d.pos) > 0 {
                continue;
            }
            // (An ally's metal is theirs, unless nothing else is left.)
            let score = dist(b.pos, d.pos) + db / 2 - d.metal / 8 + if expander && !d.far { 20_000 } else { 0 } + if self.allies_claim(d.pos) { 30_000 } else { 0 };
            cands.push((score, i));
        }
        cands.sort_unstable();
        for (_, i) in cands.into_iter().take(8) {
            let Some(ext) = ext_for(&self.deposits[i]) else { continue };
            if let (Some(sp), Some(r)) = (spec, region) {
                let slack = Self::build_slack(sim, b, ext);
                if !self.deposits[i].reachable(&sim.world, &sp, r, slack) {
                    continue;
                }
            }
            let d = &self.deposits[i];
            if let Some(w) = d.wreck {
                return Some((i, sim.snap_to_grid(ext, d.pos), Some(w), ext));
            }
            if let Some(site) = self.extractor_site(sim, ext, d.pos) && !self.failed.iter().any(|(t, p, _)| *t == ext && p.within_xz(site, fx(24))) {
                return Some((i, site, None, ext));
            }
        }
        None
    }

    /// The legal extractor site on a deposit covering the most metal.
    pub(crate) fn extractor_site(&self, sim: &Sim, ext: usize, pos: FxVec3) -> Option<FxVec3> {
        let fp = sim.types.get(ext).footprint;
        let centre = World::pos_to_anchor(pos, fp);
        let mut best: Option<(i64, i32, FxVec3)> = None;
        for dz in -1..=1i32 {
            for dx in -1..=1i32 {
                let a = (centre.0 + dx, centre.1 + dz);
                let p = sim.world.anchor_to_pos(a, fp);
                if !sim.can_place(ext, p) {
                    continue;
                }
                let m = sim.world.metal_under(World::footprint_origin(a, fp), fp);
                let near = -(dx.abs() + dz.abs());
                if best.is_none_or(|(bm, bn, _)| (m, near) > (bm, bn)) {
                    best = Some((m, near, p));
                }
            }
        }
        best.map(|(_, _, p)| p)
    }

    /// A guard tower beside an outlying extractor that has been raided before (or sits
    /// closer to the enemy than to us), unless one already stands nearby.
    pub(crate) fn outpost_tower(&mut self, sim: &Sim, s: &Survey, b: &Unit, mex: FxVec3, plan: &Plan) -> Option<(usize, FxVec3)> {
        if !self.p.towers || s.minutes < 4 || s.m_inc < fx(5) || self.info[b.type_id].commander {
            return None;
        }
        let d = self.deposits.iter().find(|d| d.pos.within_xz(mex, fx(48)))?;
        let db = dist(d.pos, self.base);
        let exposed = db > 600 && (d.losses > 0 || self.nearest_enemy_base(d.pos).is_some_and(|e| dist(d.pos, e) * 13 < db * 10 + 6000));
        if !exposed || s.defenses.iter().any(|t| t.within_xz(mex, fx(320))) || plan.reserved.iter().any(|(p, _)| p.within_xz(mex, fx(200)) && !p.within_xz(mex, fx(24))) {
            return None;
        }
        if s.count[Role::Defense as usize] + plan.pending[Role::Defense as usize] >= 2 + s.minutes as usize / 2 {
            return None;
        }
        let opts: Vec<usize> = sim.unit_type(b).build_options.iter().copied().filter(|&o| self.info[o].land && sim.is_allowed(o)).collect();
        let tower = opts.iter().copied().filter(|&o| self.info[o].role == Role::Defense && self.info[o].cost <= 800).max_by_key(|&o| (self.info[o].value, Reverse(o)))?;
        let toward = self.nearest_enemy_base(mex).map_or(FxVec3::ZERO, |e| (e - mex).xz().normalize());
        let site = self.find_site(sim, tower, mex + toward.scale(fx(56)), 5, &|_| true)?;
        self.builder_reaches(sim, b, site, tower).then_some((tower, site))
    }

    pub(crate) fn energy_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        if let Some(t) = self.geo_task(sim, s, b, opts, plan) {
            return Some(t);
        }
        let g = self.best_generator(sim, s, opts)?;
        let big = self.info[g].energy >= fx(300);
        let first = self.place(sim, b, g, Zone::Back, if big { 260 } else { 150 }, plan);
        if let Some(site) = first.filter(|&p| dist(p, b.pos) <= 600) {
            return Some(Task::Build(g, site));
        }
        // No room for it near by (no water close enough for a tidal plant, or no dry land
        // around a start at sea): the best plant of the other kind, if that goes up nearer.
        let other: Vec<usize> = opts.iter().copied().filter(|&o| self.info[o].land != self.info[g].land).collect();
        let alternative = self.best_generator(sim, s, &other).and_then(|g| {
            let big = self.info[g].energy >= fx(300);
            self.place(sim, b, g, Zone::Back, if big { 260 } else { 150 }, plan).map(|p| (g, p))
        });
        match (first, alternative) {
            (Some(p), Some((g2, p2))) => Some(if dist(p2, b.pos) < dist(p, b.pos) { Task::Build(g2, p2) } else { Task::Build(g, p) }),
            (Some(p), None) => Some(Task::Build(g, p)),
            (None, Some((g2, p2))) => Some(Task::Build(g2, p2)),
            (None, None) => None,
        }
    }

    /// Energy per second a generator is expected to make (wind at its recent average, the
    /// map's tides).
    pub(crate) fn expected_energy(&self, sim: &Sim, o: usize) -> Fx {
        let t = sim.types.get(o);
        let mut e = self.info[o].energy;
        if t.wind_generator > Fx::ZERO {
            e += self.wind_avg.min(t.wind_generator);
        }
        if t.tidal_generator > Fx::ZERO {
            e += sim.tidal_strength() * t.tidal_generator;
        }
        e
    }

    pub(crate) fn best_generator(&self, sim: &Sim, s: &Survey, opts: &[usize]) -> Option<usize> {
        let usable = |o: usize| {
            let t = sim.types.get(o);
            self.info[o].role == Role::Generator && !t.needs_geo
        };
        // A big plant (fusion) once we can pay for it: one building instead of fifty.
        let big = opts
            .iter()
            .copied()
            .filter(|&o| usable(o) && self.info[o].energy >= fx(300) && (s.metal * 3 >= sim.types.get(o).cost_metal || s.m_inc >= fx(12)))
            .max_by_key(|&o| ((self.info[o].energy * 1000 / fx(self.info[o].cost)).0, Reverse(o)));
        if big.is_some() {
            return big;
        }
        opts.iter()
            .copied()
            .filter(|&o| usable(o) && self.info[o].energy < fx(300))
            .max_by_key(|&o| ((self.expected_energy(sim, o) * 1000 / fx(self.info[o].cost)).0, Reverse(o)))
            .filter(|&o| self.expected_energy(sim, o) > Fx::ZERO)
    }

    pub(crate) fn geo_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        let plant = opts.iter().copied().filter(|&o| sim.types.get(o).needs_geo && self.info[o].role == Role::Generator).max_by_key(|&o| (self.info[o].energy.0, Reverse(o)))?;
        if self.vents.is_empty() {
            return None;
        }
        let spec = self.info[b.type_id].spec;
        let region = spec.map(|sp| map::region_at(&sim.world, &sp, b.pos));
        let slack = Self::build_slack(sim, b, plant);
        let reach = if self.info[b.type_id].commander { 650 } else { 2200 };
        let mut best: Option<(i64, FxVec3)> = None;
        for i in 0..self.vents.len() {
            let v = &self.vents[i];
            let (pos, avoid) = (v.pos, v.avoid_until);
            let db = dist(pos, self.base);
            if avoid > s.now || db > reach || !plan.free(pos, fx(48)) || self.threat.at(pos) > 0 || self.helper && !sim.explored(self.player, pos) {
                continue;
            }
            if self.nearest_enemy_base(pos).is_some_and(|e| db > 700 && db * 100 > dist(pos, e) * self.p.reach_pct) {
                continue;
            }
            let score = dist(b.pos, pos) + db / 2;
            if best.is_some_and(|(bs, _)| score >= bs) {
                continue;
            }
            if let (Some(sp), Some(r)) = (spec, region) && !self.vents[i].reachable(&sim.world, &sp, r, slack) {
                continue;
            }
            if let Some(site) = self.find_site(sim, plant, pos, 2, &|_| true) && !self.failed.iter().any(|(t, p, _)| *t == plant && p.within_xz(site, fx(24))) {
                best = Some((score, site));
            }
        }
        best.map(|(_, site)| Task::Build(plant, site))
    }

    /// Whether a factory type's combat units can get from our base to the enemy's.
    pub(crate) fn factory_reaches(&mut self, sim: &Sim, f: usize) -> bool {
        if self.info[f].kind == Locomotion::Naval {
            // Ships start from the shipyard, not from our base on land.
            return self.naval.as_ref().is_some_and(|n| n.reaches);
        }
        if let Some(r) = self.reach[f] {
            return r;
        }
        let r = match self.nearest_enemy_base(self.base) {
            None => true,
            Some(enemy) => sim.types.get(f).build_options.iter().any(|&o| {
                let i = &self.info[o];
                i.role == Role::Combat
                    && match i.spec {
                        None => true,
                        Some(sp) => map::can_reach(&sim.world, &sp, self.base, enemy, 8),
                    }
            }),
        };
        self.reach[f] = Some(r);
        r
    }

    pub(crate) fn factory_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        let first = s.count[Role::Factory as usize] == 0;
        let air_ok = s.minutes >= 5 && s.m_inc >= fx(7);
        let cands: Vec<usize> = opts.iter().copied().filter(|&o| self.info[o].role == Role::Factory && !self.advanced_factory(sim, o) && self.info[o].kind != Locomotion::Naval).collect();
        let mut keyed = Vec::new();
        for o in cands {
            let i = self.info[o].clone();
            // The first factory must turn out constructors, and cheaply.
            let makes_builders = sim.types.get(o).build_options.iter().any(|&u| self.info[u].role == Role::Builder && self.info[u].builds_mex);
            if first && !makes_builders {
                continue;
            }
            let reaches = i.kind == Locomotion::Air || self.factory_reaches(sim, o);
            let key = if first {
                (false, false, false, false, i.cost)
            } else {
                (s.factory_types.contains(&o), !reaches, i.kind == Locomotion::Air && (!air_ok || s.kinds.contains(&Locomotion::Air)), i.kind == Locomotion::Air, i.cost)
            };
            keyed.push((key, o));
        }
        keyed.sort();
        let f = keyed.first()?.1;
        let site = self.place(sim, b, f, Zone::Ring, 170 + 40 * s.count[Role::Factory as usize] as i64, plan)?;
        Some(Task::Build(f, site))
    }

    /// The cheapest plant we don't have yet whose units get from home to the enemy by
    /// themselves (hovercraft, amphibians), for an island start.
    pub(crate) fn crossing_factory_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        let cands: Vec<usize> = opts
            .iter()
            .copied()
            .filter(|&o| self.info[o].role == Role::Factory && !self.advanced_factory(sim, o) && !matches!(self.info[o].kind, Locomotion::Air | Locomotion::Naval) && !s.factory_types.contains(&o))
            .collect();
        let reaching: Vec<usize> = cands.into_iter().filter(|&o| self.factory_reaches(sim, o)).collect();
        let f = reaching.into_iter().min_by_key(|&o| (self.info[o].cost, o))?;
        let site = self.place(sim, b, f, Zone::Ring, 170 + 40 * s.count[Role::Factory as usize] as i64, plan)?;
        Some(Task::Build(f, site))
    }

    /// The cheapest factory that makes a transport for our stranded units.
    pub(crate) fn transport_factory_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        self.cheapest_factory_task(sim, s, b, opts, plan, |ai, list| ai.pick_transport(sim, s, list).is_some())
    }

    /// The cheapest basic factory whose build list passes `makes`.
    pub(crate) fn cheapest_factory_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan, makes: impl Fn(&Self, &[usize]) -> bool) -> Option<Task> {
        let f = opts
            .iter()
            .copied()
            .filter(|&o| self.info[o].role == Role::Factory && !self.advanced_factory(sim, o) && makes(self, &sim.types.get(o).build_options))
            .min_by_key(|&o| (self.info[o].cost, o))?;
        let site = self.place(sim, b, f, Zone::Ring, 170 + 40 * s.count[Role::Factory as usize] as i64, plan)?;
        Some(Task::Build(f, site))
    }

    pub(crate) fn adv_factory_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        let mut keyed = Vec::new();
        let cands: Vec<usize> = opts.iter().copied().filter(|&o| self.advanced_factory(sim, o)).collect();
        // Across water: a plant for units that get over by themselves (amphibians) first.
        let types = s.factory_types.clone();
        let stranded = !types.iter().any(|&f| matches!(self.info[f].kind, Locomotion::Ground | Locomotion::Hover) && self.factory_reaches(sim, f));
        for o in cands {
            let i = self.info[o].clone();
            let reaches = i.kind == Locomotion::Air || self.factory_reaches(sim, o);
            let crossing = i.kind != Locomotion::Air && reaches;
            keyed.push(((stranded && !crossing, !s.kinds.contains(&i.kind), !reaches, i.kind == Locomotion::Air, i.cost), o));
        }
        keyed.sort();
        let f = keyed.first()?.1;
        // The room kept for it, if still usable.
        if let Some((p, _)) = self.tech_site {
            let site = sim.snap_to_grid(f, p);
            if sim.can_place(f, site) && self.builder_reaches(sim, b, site, f) && plan.free(site, fx(sim.types.get(f).radius.to_int())) {
                return Some(Task::Build(f, site));
            }
        }
        let site = self.place(sim, b, f, Zone::Ring, 220, plan)?;
        Some(Task::Build(f, site))
    }

    /// Early on, keep room for the advanced factory (sized for the biggest one our
    /// constructors could build) before small buildings fill the base.
    pub(crate) fn reserve_tech_site(&mut self, sim: &Sim, s: &Survey) {
        if s.adv_factory {
            self.tech_site = None;
            return;
        }
        if self.tech_site.is_some() || s.factory_types.is_empty() || s.now % (10 * SECOND) >= THINK_TICKS {
            return;
        }
        let mut best: Option<(u32, usize)> = None;
        for &f in &s.factory_types {
            for &b in &sim.types.get(f).build_options {
                if self.info[b].role != Role::Builder {
                    continue;
                }
                for &o in &sim.types.get(b).build_options {
                    if self.info[o].kind != Locomotion::Air && self.info[o].land && self.advanced_factory(sim, o) {
                        let fp = sim.types.get(o).footprint;
                        let size = fp.0.max(fp.1);
                        if best.is_none_or(|(bs, bt)| (size, Reverse(o)) > (bs, Reverse(bt))) {
                            best = Some((size, o));
                        }
                    }
                }
            }
        }
        let Some((_, t)) = best else { return };
        let p = self.zone_point(sim, Zone::Ring, 200);
        let site = self.find_site(sim, t, p, 30, &|site| self.threat.at(site) == 0).or_else(|| self.find_site(sim, t, self.base, 64, &|_| true));
        self.tech_site = site.map(|p| (p, t));
    }

    pub(crate) fn defense_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        if !self.p.towers || s.minutes < 3 {
            return None;
        }
        // A couple of towers on the approach, more as the game goes on.
        let want = 1 + s.minutes as usize / 5 + self.p.extra_towers;
        self.front_tower_task(sim, s, b, opts, plan, want.min(8))
    }

    /// Anti-aircraft once planes of the enemy's have been seen lately, a few more the more of
    /// them there were.
    pub(crate) fn aa_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        if !self.air_threat(s.now) || s.count[Role::AntiAir as usize] + plan.pending[Role::AntiAir as usize] >= (1 + self.enemy_air / 3).min(6) {
            return None;
        }
        let a = self.pick_best(opts, Role::AntiAir)?;
        let p = self.place(sim, b, a, Zone::Ring, 120, plan)?;
        Some(Task::Build(a, p))
    }

    /// A tower on the approach, towards the enemy, while fewer than `want` of ours stand or
    /// are going up and there is metal coming in to pay for them.
    pub(crate) fn front_tower_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan, want: usize) -> Option<Task> {
        let have = s.count[Role::Defense as usize] + plan.pending[Role::Defense as usize];
        if have >= want || s.m_inc < fx(4) {
            return None;
        }
        let budget = if s.minutes >= 12 { 2500 } else { 800 };
        let tower = opts.iter().copied().filter(|&o| self.info[o].role == Role::Defense && self.info[o].cost <= budget).max_by_key(|&o| (self.info[o].value, Reverse(o)))?;
        let site = self.place(sim, b, tower, Zone::Front, 260, plan)?;
        Some(Task::Build(tower, site))
    }

    /// An extractor of ours worth replacing with a better one we can build (richest safe
    /// deposits first), when metal and power allow.
    pub(crate) fn upgrade_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize]) -> Option<Task> {
        if self.upgrades.len() >= self.p.upgrades || s.m_inc < fx(12) || self.energy_avg < Fx::from_ratio(3, 10) {
            return None;
        }
        let best_ext = opts.iter().copied().filter(|&o| self.info[o].role == Role::Extractor && !sim.types.get(o).needs_geo).max_by_key(|&o| (sim.types.get(o).extracts_metal.0, Reverse(o)))?;
        let rate = sim.types.get(best_ext).extracts_metal;
        // Broke is normal for an economy that spends everything; an upgrade pays for itself
        // within minutes, so only a small income holds it back.
        if s.metal * 2 < sim.types.get(best_ext).cost_metal && self.metal_avg < Fx::from_ratio(3, 10) && s.m_inc < fx(20) {
            return None;
        }
        let spec = self.info[b.type_id].spec;
        let region = spec.map(|sp| map::region_at(&sim.world, &sp, b.pos));
        let slack = Self::build_slack(sim, b, best_ext);
        let mut cands: Vec<(i64, usize, UnitId)> = Vec::new();
        for (i, d) in self.deposits.iter().enumerate() {
            let Some((old, t)) = d.ours else { continue };
            if sim.types.get(t).extracts_metal * 2 > rate || d.avoid_until > s.now || d.losses > 2 || self.threat.at(d.pos) > 0 {
                continue;
            }
            if self.upgrades.iter().any(|u| u.deposit == i) || sim.unit(old).is_none_or(|u| !u.is_complete() || u.self_destruct_at.is_some()) {
                continue;
            }
            let db = dist(d.pos, self.base);
            if self.nearest_enemy_base(d.pos).is_some_and(|e| db * 10 > dist(d.pos, e) * 9) {
                continue;
            }
            cands.push((db / 4 - d.metal, i, old));
        }
        cands.sort_unstable();
        for (_, i, old) in cands.into_iter().take(4) {
            if let (Some(sp), Some(r)) = (spec, region) && !self.deposits[i].reachable(&sim.world, &sp, r, slack) {
                continue;
            }
            return Some(Task::Upgrade(i, old, best_ext));
        }
        None
    }

    /// Advances extractor upgrades: once the builder is close it reclaims the old extractor
    /// (getting its metal back; builders that cannot reclaim blow it up instead), then
    /// builds the new one (clearing any wreck first).
    /// Returns the builders just told to build the new extractor (theirs is the job now).
    pub(crate) fn run_upgrades(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) -> Vec<UnitId> {
        let me = self.player;
        let ordered = ordered_units(out);
        let mut done = Vec::new();
        let mut building = Vec::new();
        for k in 0..self.upgrades.len() {
            let up = self.upgrades[k];
            let Some(b) = sim.unit(up.builder).filter(|b| !b.dead && b.owner == me) else {
                done.push(k);
                continue;
            };
            let d = &self.deposits[up.deposit];
            if s.now > up.since + 90 * SECOND || self.threat.at(d.pos) > 0 {
                done.push(k);
                continue;
            }
            if ordered.contains(&b.id) || !Self::can_command(sim, b) { continue; }
            match sim.unit(up.old).filter(|u| !u.dead) {
                Some(old) if old.owner != me => done.push(k),
                Some(old) => {
                    let close = b.pos.within_xz(d.pos, fx(260)) && b.orders.is_empty();
                    if close && old.self_destruct_at.is_none() {
                        if sim.unit_type(b).def.can_reclaim && up.reclaims < 2 {
                            self.upgrades[k].reclaims += 1;
                            out.push(Command::Order { player: me, units: vec![b.id], order: Order::ReclaimUnit(old.id), queue: false });
                        } else if self.helper {
                            // (A player's extractor is never blown up for the sake of a better one:
                            // the upgrade is given up.)
                            done.push(k);
                            continue;
                        } else {
                            out.push(Command::SelfDestruct { player: me, units: vec![old.id] });
                        }
                    }
                }
                None => {
                    let Some(site) = self.extractor_site(sim, up.new_type, d.pos).or_else(|| d.wreck.map(|_| sim.snap_to_grid(up.new_type, d.pos))) else {
                        done.push(k);
                        continue;
                    };
                    match d.wreck {
                        Some(w) if sim.features.get(w).is_some_and(|f| f.alive && sim.data.features[f.def].reclaimable) => {
                            out.push(Command::Order { player: me, units: vec![b.id], order: Order::Reclaim(w), queue: false });
                            out.push(Command::Order { player: me, units: vec![b.id], order: Order::Build { type_id: up.new_type, pos: site }, queue: true });
                        }
                        _ => out.push(Command::Order { player: me, units: vec![b.id], order: Order::Build { type_id: up.new_type, pos: site }, queue: false }),
                    }
                    building.push(b.id);
                    done.push(k);
                }
            }
        }
        for k in done.into_iter().rev() {
            self.upgrades.remove(k);
        }
        building
    }

    /// A tower by a factory stopped because its yard is under fire, between it and what
    /// shoots at it (ships off the shore, say, that no land unit of ours can get at).
    pub(crate) fn yard_tower_task(&self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        // (Not the Commander: it would walk into the fire.)
        if !self.p.towers || plan.pending[Role::Defense as usize] > 0 || self.info[b.type_id].commander {
            return None;
        }
        for &(id, _) in &self.paused {
            let Some(f) = sim.unit(id).filter(|f| !f.dead && f.owner == self.player) else { continue };
            if s.defenses.iter().any(|d| d.within_xz(f.pos, fx(300))) || !self.unit_reaches(sim, b, f.pos) {
                continue;
            }
            let Some(attacker) = s.enemies.iter().filter(|e| e.is_complete() && self.info[e.type_id].ground_dps > 0 && e.pos.within_xz(f.pos, fx(800))).min_by_key(|e| (dist(e.pos, f.pos), e.id.slot)) else {
                continue;
            };
            let budget = if s.minutes >= 12 { 1500 } else { 800 };
            let tower = opts.iter().copied().filter(|&o| self.info[o].role == Role::Defense && self.info[o].land && self.info[o].cost <= budget).max_by_key(|&o| (self.info[o].value, Reverse(o)))?;
            let toward = (attacker.pos - f.pos).xz().normalize();
            let r = fx(sim.types.get(tower).radius.to_int() + 16);
            let near = f.pos + toward.scale(fx(sim.unit_type(f).footprint.0.max(sim.unit_type(f).footprint.1) as i64 * 8 + 64));
            if let Some(site) = self.find_site(sim, tower, near, 8, &|p| plan.free(p, r) && self.builder_reaches(sim, b, p, tower)) {
                log::debug!("{} min: P{} puts a tower by its {} under fire", s.minutes, self.player, sim.unit_type(f).name);
                return Some(Task::Build(tower, site));
            }
        }
        None
    }

    /// Guard towers at outlying extractor fields: raiders pick off undefended ones.
    pub(crate) fn outpost_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        if !self.p.towers || s.minutes < 4 || s.m_inc < fx(6) || self.info[b.type_id].commander {
            return None;
        }
        let outposts = s.defenses.iter().filter(|p| !p.within_xz(self.base, fx(600))).count() + plan.pending[Role::Defense as usize];
        if outposts > (s.m_inc.to_int() / 4) as usize {
            return None;
        }
        let budget = if s.minutes >= 14 { 1200 } else { 700 };
        let tower = opts.iter().copied().filter(|&o| self.info[o].role == Role::Defense && self.info[o].cost <= budget).max_by_key(|&o| (self.info[o].value, Reverse(o)))?;
        let mut best: Option<(i64, FxVec3)> = None;
        for d in self.deposits.iter().filter(|d| d.ours.is_some()) {
            let db = dist(d.pos, self.base);
            if db < 650 || s.defenses.iter().any(|t| t.within_xz(d.pos, fx(400))) || self.threat.at(d.pos) > 0 {
                continue;
            }
            if plan.reserved.iter().any(|(p, _)| p.within_xz(d.pos, fx(300)) && !p.within_xz(d.pos, fx(24))) {
                continue;
            }
            // Worth guarding: a rich field, or one that has been raided before.
            let field: i64 = self.deposits.iter().filter(|e| e.ours.is_some() && e.pos.within_xz(d.pos, fx(450))).map(|e| e.metal).sum();
            let losses: i64 = self.deposits.iter().filter(|e| e.pos.within_xz(d.pos, fx(450))).map(|e| e.losses as i64).sum();
            if field < 1800 && losses == 0 {
                continue;
            }
            let exposed = self.nearest_enemy_base(d.pos).map_or(0, |e| (db - dist(d.pos, e)).max(-2000));
            let score = field + losses * 800 + exposed / 2;
            if best.is_none_or(|(bs, _)| score > bs) {
                best = Some((score, d.pos));
            }
        }
        let (_, at) = best?;
        let toward = self.nearest_enemy_base(at).map_or(FxVec3::ZERO, |e| (e - at).xz().normalize());
        let site = self.find_site(sim, tower, at + toward.scale(fx(72)), 6, &|p| self.threat.at(p) == 0 && self.builder_reaches(sim, b, p, tower))?;
        Some(Task::Build(tower, site))
    }

    /// Power to spare: the store mostly full, or (with big storage the fraction says little)
    /// a healthy bank that keeps growing.
    pub(crate) fn power_spare(&self, s: &Survey) -> bool {
        self.energy_avg > Fx::from_ratio(55, 100) || s.energy >= fx(1500) && s.e_inc - s.e_exp > fx(100)
    }

    /// Energy in the store (smoothed): what the store holds on average lately, not the
    /// moment's figure (wind and construction make that jump about).
    pub(crate) fn energy_banked(&self, s: &Survey) -> Fx {
        self.energy_avg * s.e_cap
    }

    /// Energy per second more metal makers could have: what is left over for makers on
    /// average lately (income minus everything else's spending, what is lost to a full store
    /// included) beyond what the makers we have can use, and what the store holds beyond a
    /// reserve (spent over a minute: power piling up while metal runs short is what makers
    /// are for).
    pub(crate) fn maker_headroom(&self, s: &Survey) -> Fx {
        let reserve = (s.e_cap / 4).max(fx(600));
        let bank = (self.energy_banked(s) - reserve).max(Fx::ZERO) / 60;
        self.maker_room + bank - s.maker_capacity
    }

    pub(crate) fn maker_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan, starved: bool) -> Option<Task> {
        let full = if starved { Fx::from_ratio(4, 10) } else { Fx::from_ratio(7, 10) };
        let spare = self.energy_avg >= full || starved && self.power_spare(s);
        if !spare || self.metal_avg > Fx::from_ratio(6, 10) || s.makers + plan.pending[Role::MetalMaker as usize] >= self.p.max_makers {
            return None;
        }
        if plan.pending[Role::MetalMaker as usize] >= if starved { 4 } else { 2 } {
            return None;
        }
        // Advanced constructors build the big makers; the small ones stop at a score.
        let tech = self.info[b.type_id].tech;
        if tech < 2 && s.builders_t2 > 0 && s.makers >= 20 {
            return None;
        }
        let room = self.maker_headroom(s);
        // The most metal per energy among makers the room can run most of the time, with
        // those already on order. With the store overflowing most of the time power is being
        // thrown away whatever the figures say: one more at a time (of those using a quarter
        // of the income at most).
        let ordered = plan.pending[Role::MetalMaker as usize] as i64 + 1;
        let overflowing = self.brimming > Fx::HALF && ordered == 1;
        let m = opts
            .iter()
            .copied()
            .filter(|&o| {
                let up = self.info[o].maker_upkeep;
                self.info[o].role == Role::MetalMaker && up > Fx::ZERO && (up * ordered * 4 <= room * 5 || overflowing && up * 4 <= s.e_inc)
            })
            .max_by_key(|&o| ((self.info[o].makes_metal * 10000 / self.info[o].maker_upkeep).0, Reverse(self.info[o].cost), Reverse(o)))?;
        let site = self.place(sim, b, m, Zone::Back, 200, plan)?;
        Some(Task::Build(m, site))
    }

    /// Advanced constructors: fusion when power is short, anti-missile cover, missile
    /// silos once the economy can feed them.
    pub(crate) fn strategic_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        let pending = |r: Role| plan.pending[r as usize];
        if let Some(t) = self.fusion_task(sim, s, b, opts, plan) {
            return Some(t);
        }
        // One strategic building at a time, and only on an economy that can pay for it
        // (tens of thousands of energy each).
        let strategic_building = s.nanoframes.iter().any(|n| matches!(self.info[n.type_id].role, Role::Silo | Role::AntiNuke));
        if strategic_building || s.big_plants == 0 {
            return None;
        }
        // One anti-missile defence once missiles could be coming; more only against a
        // silo we have actually seen.
        let anti_wanted = if self.enemy_silo {
            self.p.anti_nukes.max(1)
        } else if s.minutes >= self.p.nuke_minute.saturating_sub(4) && s.e_inc >= fx(1000) {
            1.min(self.p.anti_nukes)
        } else {
            0
        };
        if s.count[Role::AntiNuke as usize] + pending(Role::AntiNuke) < anti_wanted && let Some(a) = self.pick_cheapest(opts, Role::AntiNuke) && let Some(site) = self.place(sim, b, a, Zone::Ring, 60, plan) {
            return Some(Task::Build(a, site));
        }
        // A second silo only deep into the game.
        let silos = if s.minutes >= self.p.nuke_minute.saturating_add(16) { self.p.silos } else { self.p.silos.min(1) };
        if s.minutes >= self.p.nuke_minute && s.e_inc >= fx(1500) && s.m_inc >= fx(20) && s.count[Role::Silo as usize] + pending(Role::Silo) < silos && let Some(n) = self.pick_cheapest(opts, Role::Silo) && let Some(site) = self.place(sim, b, n, Zone::Back, 300, plan) {
            return Some(Task::Build(n, site));
        }
        None
    }

    /// A big power plant (fusion) when power is short and none is going up yet.
    pub(crate) fn fusion_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        let big_building = s.nanoframes.iter().any(|n| self.info[n.type_id].role == Role::Generator && self.info[n.type_id].energy >= fx(300));
        // Storage brimming, or power to spare already: more would only be thrown away.
        let full = self.energy_avg > Fx::from_ratio(8, 10) || self.power_spare(s);
        if full || s.e_inc >= self.energy_want(s) || big_building || plan.pending[Role::Generator as usize] >= self.p.power_plants {
            return None;
        }
        let g = self.best_generator(sim, s, opts).filter(|&g| self.info[g].energy >= fx(300))?;
        let site = self.place(sim, b, g, Zone::Back, 280, plan)?;
        Some(Task::Build(g, site))
    }

    /// Help: finish our most valuable unfinished building nearby, or (with metal to spare)
    /// speed up a factory; otherwise head home.
    pub(crate) fn assist_task(&self, sim: &Sim, s: &Survey, b: &Unit, max_dist: i64) -> Option<Task> {
        if let Some(t) = self.finish_task(sim, s, b, max_dist) {
            return Some(t);
        }
        if (self.metal_avg > Fx::from_ratio(3, 10) || self.info[b.type_id].commander) && let Some(f) = self.factory_to_help(sim, s, b, max_dist) {
            return Some(Task::Guard(f));
        }
        (!b.pos.within_xz(self.base, fx(400)) && b.orders.is_empty()).then_some(Task::Move(self.base))
    }

    /// Our most valuable unfinished building near a builder, to finish.
    pub(crate) fn finish_task(&self, sim: &Sim, s: &Survey, b: &Unit, max_dist: i64) -> Option<Task> {
        let mut best: Option<(i64, UnitId)> = None;
        for n in &s.nanoframes {
            let d = dist(b.pos, n.pos);
            if d > max_dist || self.unreachable_units.iter().any(|&(u, _)| u == n.id) {
                continue;
            }
            let i = &self.info[n.type_id];
            let weight = match i.role {
                Role::Factory | Role::Silo | Role::AntiNuke => 6,
                Role::Generator if i.energy >= fx(300) => 6,
                Role::Extractor if i.tech >= 2 => 5,
                Role::MetalMaker | Role::AntiAir | Role::Defense => 3,
                Role::Generator => 2,
                _ => 1,
            };
            let score = weight * 1000 - d;
            if best.is_some_and(|(bs, _)| score <= bs) {
                continue;
            }
            if !self.unit_reaches(sim, b, n.pos) {
                continue;
            }
            best = Some((score, n.id));
        }
        best.map(|(_, id)| Task::Repair(id))
    }

    // ---- Factories -------------------------------------------------------------------------

    pub(crate) fn run_factories(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let want_t1 = (self.p.builders_base + (s.minutes / self.p.builders_div) as usize).min(self.p.builders_cap);
        let want_t2 = if s.adv_factory { self.p.adv_builders.min(1 + s.minutes as usize / 4) } else { 0 };
        let want_aa = if self.air_threat(s.now) { (1 + self.enemy_air / 2).min(8) } else { 0 };
        let (mut t1, mut t2, mut aa) = (s.builders_t1, s.builders_t2, s.aa_units);
        let has_ground_factory = s.kinds.iter().any(|k| *k != Locomotion::Air);
        // (None while strikes have not been paying for the planes they lose, unless the enemy
        // is all but gone and planes can finish it.)
        let mut plane_cap = if s.now < self.air_pause_until && !self.mopping_up(s) { 0 } else { 4 + s.minutes as usize / 2 };
        // An island war: room for the bombers a strike on the enemy Commander takes, and a
        // scout to find it.
        let doctrine = self.p.air_doctrine && self.island;
        if doctrine && plane_cap > 0 {
            plane_cap = plane_cap.max(14);
        }
        // (And on any difficulty a scout plane to look for the enemy over the water, there being
        // no way there by land.)
        let unknown = self.enemy_base_seen.iter().any(|&seen| !seen);
        let mut want_scout = (doctrine && self.scout.is_none() || self.island && unknown && self.ground_scout.is_none()) && !s.own.iter().any(|u| self.is_scout(u.type_id));
        // A fast unit to look for the enemy (`run_scouting`), while there is a start of theirs
        // not looked at and no scout out (one lost: after a while).
        let mut want_ground_scout = self.ground_scout.is_none()
            && self.enemy_base_seen.iter().any(|&seen| !seen)
            && self.scout_lost.is_none_or(|t| s.now >= t + 90 * SECOND)
            && !s.army.iter().any(|u| u.pos.within_xz(self.base, fx(1500)) && matches!(self.info[u.type_id].kind, Locomotion::Ground | Locomotion::Hover));
        let mut planes = s.planes.len() + s.own.iter().filter(|u| !u.is_complete() && u.factory.is_some() && self.info[u.type_id].kind == Locomotion::Air).count();
        // Warships, finished or in production.
        let fleet_cap = if self.difficulty == Difficulty::Easy { 4 } else { (6 + s.minutes as usize / 2).min(24) };
        let mut fleet = s.own.iter().filter(|u| self.info[u.type_id].kind == Locomotion::Naval && self.info[u.type_id].role == Role::Combat).count();
        // Naval constructors for the free deposits under water.
        let sea = sim.world.sea_level;
        let wet = self.deposits.iter().filter(|d| d.owner.is_none() && sim.world.height_at(d.pos.x, d.pos.z) < sea).count();
        let want_boats = if wet >= 3 { 1 + (wet >= 8 && self.difficulty != Difficulty::Easy) as usize + (wet >= 20 && s.minutes >= 6) as usize } else { 0 };
        // Battle handling keeps one at sea to mend the fleet.
        let warships = s.army.iter().filter(|u| self.info[u.type_id].kind == Locomotion::Naval).count();
        let want_boats = want_boats.max((self.p.micro && warships >= 3) as usize);
        let mut boats = s.own.iter().filter(|u| self.info[u.type_id].kind == Locomotion::Naval && self.info[u.type_id].role == Role::Builder).count();
        let rally = self.rally;
        let me = self.player;
        // Combat units, finished or in production.
        let mut army = s.own.iter().filter(|u| matches!(self.info[u.type_id].role, Role::Combat | Role::AntiAirUnit)).count();
        // Transport room (units carried at once), finished or in production, and how much
        // the stranded army needs.
        let want_lift = self.want_transports(sim, s);
        // Constructors that fly or hover (finished or in production), for the metal on
        // other islands.
        let far = self.far_deposits(s);
        let want_far = if s.minutes >= 3 && far >= 3 { (1 + far / 6).min(self.p.expanders) } else { 0 };
        let mut expanders = s.own.iter().filter(|u| self.expander(u.type_id)).count();
        let mut lifts: usize = s.own.iter().filter(|u| self.info[u.type_id].transport > 0).map(|u| self.room(u.type_id)).sum();
        let enemies_left = sim.players.iter().any(|p| sim.is_enemy(me, p.index) && !p.defeated);
        self.produced.retain(|&(id, t)| s.now < t + self.p.produce_gap && sim.unit(id).is_some());
        // A factory whose units keep dying in its yard unfinished is under fire: every unit
        // started there would be fed to the guns (a factory starts its next unit by itself).
        // It stops for a while, its queue cancelled, until the threat is dealt with or gone.
        self.paused.retain(|&(f, until)| s.now < until && sim.unit(f).is_some_and(|f| !f.dead && f.owner == me));
        for f in &s.factories {
            let lost = self.yard_losses.iter().filter(|&&(id, _)| id == f.id).count();
            if lost >= 2 && !self.paused.iter().any(|&(id, _)| id == f.id) {
                log::debug!("{} min: P{me} stops {} at ({}, {}): {lost} units lost in its yard", s.minutes, sim.unit_type(f).name, f.pos.x.to_int(), f.pos.z.to_int());
                self.paused.push((f.id, s.now + 90 * SECOND));
                for &t in &f.factory_queue {
                    out.push(Command::Produce { player: me, factory: f.id, type_id: t, count: -1 });
                }
            }
        }
        // Units queued and not started yet count as well as those under way.
        for f in &s.factories {
            for &t in f.factory_queue.iter().skip(f.build.is_some() as usize) {
                let i = &self.info[t];
                match i.role {
                    Role::Builder if i.kind == Locomotion::Naval => boats += 1,
                    Role::Builder if i.builds_mex || i.builds_energy => {
                        if i.tech >= 2 {
                            t2 += 1;
                        } else {
                            t1 += 1;
                        }
                        expanders += self.expander(t) as usize;
                    }
                    Role::AntiAirUnit => {
                        aa += 1;
                        army += 1;
                    }
                    Role::Combat => {
                        army += 1;
                        planes += (i.kind == Locomotion::Air) as usize;
                        fleet += (i.kind == Locomotion::Naval) as usize;
                    }
                    _ => lifts += if i.transport > 0 { self.room(t) } else { 0 },
                }
            }
        }
        for f in &s.factories {
            // A second unit waits behind the one under way: the factory goes straight on to
            // it instead of closing up and opening again, and its helpers never stand idle.
            // (Not on Easy: its slower hand is the point.)
            let depth = if self.p.produce_gap == 0 { 2 } else { 1 };
            if f.factory_queue.len() >= depth || self.produced.iter().any(|&(id, _)| id == f.id) || self.paused.iter().any(|&(id, _)| id == f.id) {
                continue;
            }
            let fi = self.info[f.type_id].clone();
            let opts: Vec<usize> = sim.unit_type(f).build_options.iter().copied().filter(|&o| sim.is_allowed(o)).collect();
            // Constructors: the fastest worker that builds extractors and power.
            let builder = |tech: u8| {
                opts.iter()
                    .copied()
                    .filter(|&o| {
                        let i = &self.info[o];
                        i.role == Role::Builder && (i.builds_mex || i.builds_energy) && (i.tech >= 2) == (tech >= 2)
                    })
                    .max_by_key(|&o| ((self.info[o].worker * self.info[o].speed).0, Reverse(o)))
            };
            let air = fi.kind == Locomotion::Air;
            if fi.kind == Locomotion::Naval {
                if boats < want_boats && (boats == 0 || fleet >= 2) {
                    let boat = opts.iter().copied().filter(|&o| self.info[o].role == Role::Builder && self.info[o].kind == Locomotion::Naval).max_by_key(|&o| ((self.info[o].worker * self.info[o].speed).0, Reverse(o)));
                    if let Some(t) = boat {
                        boats += 1;
                        out.push(Command::Produce { player: me, factory: f.id, type_id: t, count: 1 });
                        continue;
                    }
                }
                // Shipyards turn out warships (as many as the fleet may have).
                if fleet >= fleet_cap || army >= self.p.army_cap || !enemies_left {
                    continue;
                }
                let Some(t) = self.priority_build(sim, &opts).or_else(|| self.pick_combat(sim, s, f.type_id, &opts)) else { continue };
                fleet += 1;
                army += 1;
                if self.p.produce_gap > 0 {
                    self.produced.push((f.id, s.now));
                }
                out.push(Command::Produce { player: me, factory: f.id, type_id: t, count: 1 });
                continue;
            }
            let mut choice = None;
            if expanders < want_far && matches!(fi.kind, Locomotion::Air | Locomotion::Hover) {
                choice = opts.iter().copied().filter(|&o| self.expander(o)).max_by_key(|&o| ((self.info[o].worker * self.info[o].speed).0, Reverse(o)));
                if let Some(t) = choice {
                    expanders += 1;
                    log::debug!("{} min: P{me} makes {} for {far} deposits out of reach", s.minutes, sim.types.get(t).name);
                }
            }
            // Constructors and army grow together: two constructors first, then one more
            // for every two combat units (an army-less economy is a gift to raiders).
            if choice.is_none() && t1 < want_t1 && (t1 < 2 || t1 < 2 + army / 2) && (!air || !has_ground_factory) {
                choice = builder(1);
                t1 += choice.is_some() as usize;
            }
            if choice.is_none() && t2 < want_t2 && (t2 < 1 || t2 < 1 + army / 6) {
                choice = builder(2);
                t2 += choice.is_some() as usize;
            }
            // The scout, after the first two constructors.
            if choice.is_none() && want_ground_scout && t1 >= 2 && !air {
                choice = opts
                    .iter()
                    .copied()
                    .filter(|&o| self.info[o].role == Role::Combat && matches!(self.info[o].kind, Locomotion::Ground | Locomotion::Hover))
                    .max_by_key(|&o| (self.info[o].speed, Reverse(self.info[o].cost), Reverse(o)));
                want_ground_scout = choice.is_none();
            }
            // The island war's scout, first thing from an air plant.
            if choice.is_none() && air && want_scout {
                choice = opts.iter().copied().filter(|&o| self.is_scout(o)).min_by_key(|&o| (self.info[o].cost, o));
                want_scout = choice.is_none();
            }
            // Mopping up: bombers finish the last buildings sooner than more lifts would.
            if choice.is_none() && air && planes < 3 && self.mopping_up(s) {
                choice = self.pick_combat(sim, s, f.type_id, &opts);
                planes += choice.is_some() as usize;
            }
            if choice.is_none() && lifts < want_lift {
                choice = self.pick_transport(sim, s, &opts);
                lifts += choice.map_or(0, |t| self.room(t));
            }
            if choice.is_none() && aa < want_aa {
                choice = self.pick_best(&opts, Role::AntiAirUnit);
                aa += choice.is_some() as usize;
            }
            if choice.is_none() {
                if air && planes >= plane_cap || army >= self.p.army_cap || !enemies_left {
                    continue;
                }
                choice = self.priority_build(sim, &opts).or_else(|| self.pick_combat(sim, s, f.type_id, &opts));
                planes += (air && choice.is_some()) as usize;
                army += choice.is_some() as usize;
            }
            if let Some(t) = choice {
                if self.p.produce_gap > 0 {
                    self.produced.push((f.id, s.now));
                }
                out.push(Command::Produce { player: me, factory: f.id, type_id: t, count: 1 });
                if f.rally != [Order::Move(rally)] {
                    out.push(Command::SetRally { player: me, factory: f.id, pos: rally });
                }
            }
        }
    }

    /// Of what a factory offers, what a mission tells us to build first (`BuildPriority`: "of
    /// all the units specified this particular unit should be built first", says TA's
    /// editor; Attack on Barrack's Dominators). It goes where a combat unit would.
    pub(crate) fn priority_build(&self, sim: &Sim, opts: &[usize]) -> Option<usize> {
        sim.build_priority(self.player).find(|t| opts.contains(t))
    }

    /// Pick among the best-value combat units a factory offers (a little variety).
    pub(crate) fn pick_combat(&mut self, sim: &Sim, s: &Survey, factory: usize, opts: &[usize]) -> Option<usize> {
        let air = self.info[factory].kind == Locomotion::Air;
        let naval = self.info[factory].kind == Locomotion::Naval;
        if !air && !self.factory_reaches(sim, factory) && s.army.len() >= 12 {
            // An island: a home guard is all these units can do, unless transports carry
            // them across (as many as they can keep up with).
            let stranded = s.army.iter().filter(|u| self.stranded(u)).count();
            let room: usize = s.transports.iter().map(|t| self.room(t.type_id)).sum();
            if naval || room == 0 || stranded >= 8 + 3 * room {
                return None;
            }
        }
        // Across water: the units that get over by themselves (hovercraft, amphibians).
        let crossing_only = !air && !naval && opts.iter().any(|&o| self.info[o].role == Role::Combat && self.info[o].kind != Locomotion::Air && self.crosses[o]);
        let mut c: Vec<(i64, usize)> = opts
            .iter()
            .copied()
            .filter(|&o| {
                let i = &self.info[o];
                i.role == Role::Combat && (i.kind == Locomotion::Air) == air && !(air && i.fighter) && (!crossing_only || self.crosses[o])
            })
            // Bombers are what an air plant is for (strikes on the enemy economy).
            // (Budgets of cheap units win land battles; at sea and in the air the big hulls
            // and bombers do the work, so there by the unit.)
            .map(|o| {
                let i = &self.info[o];
                // (In an island war, bombers above all: the Commander is their target.)
                let bomber_weight = if i.bomber { if self.p.air_doctrine && self.island { 4 } else { 2 } } else { 1 };
                (if self.p.efficient && matches!(i.kind, Locomotion::Ground | Locomotion::Hover) { i.efficiency } else { i.value * bomber_weight }, o)
            })
            .collect();
        if c.is_empty() {
            return None;
        }
        c.sort_by_key(|&(v, o)| (Reverse(v), o));
        if !self.p.choose_best {
            let i = self.rng.range_i32(0, c.len() as i32 - 1) as usize;
            return Some(c[i].1);
        }
        let top = c.len().min(3);
        // Weights 3:2:1 for the best three.
        let roll = self.rng.range_i32(0, [0, 2, 4, 5][top]);
        let i = match roll {
            0..=2 => 0,
            3..=4 => 1,
            _ => 2,
        }
        .min(top - 1);
        Some(c[i].1)
    }
}
