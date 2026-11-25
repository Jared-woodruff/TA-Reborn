//! Projects: the big buildings advanced constructors put up one at a time, the others helping to
//! finish them: anti-nuclear cover once missiles may be coming (and cover against tactical
//! missiles once an enemy launcher of them is seen), strategic silos (built by the constructors
//! whose options hold one: Forged Alliance's tech 3 engineers), tactical launchers in reach of
//! the enemy's base, heavy defences on the approach, long-range guns in reach of it,
//! experimentals (or a factory of them); each once the economy can feed it, as the difficulty and
//! the persona have it (Easy builds none but anti-missile cover against a launcher it has seen).
//! The experimentals and strategic silos are saved for: they go ahead on a metal income that
//! feeds them however short the store is (the factories then leave them a share of the metal,
//! `economy.rs`), and constructors lend them a hand, those in a factory's yard too. And the
//! long-range guns fired at what we know of the enemy's base: a building's guns fire of
//! themselves only at what their side sees.

use super::*;

/// A defence costing this much (metal and energy / 60) or more is a heavy one (TA's Annihilator,
/// Doomsday Machine, Guardian, Punisher).
pub(crate) const HEAVY_COST: i64 = 1500;
/// How long (minutes) of our metal income a project may cost: a strategic silo, a long-range gun,
/// anything else (an experimental: `SkirmishAi::experimental_budget`); and a factory of
/// experimentals with the cheapest it makes.
const SILO_INCOME_MINUTES: i64 = 7;
const LONG_GUN_INCOME_MINUTES: i64 = 5;
const EXPERIMENTAL_FACTORY_INCOME_MINUTES: i64 = 10;
/// The metal income (a second) on which the projects saved for (experimentals, strategic silos)
/// go ahead however short the store is: an economy that spends what it makes, as ours does, is
/// short of metal in store most of the time.
const SAVED_FOR_INCOME: i64 = 30;
/// Build time per extra hand a project in a factory's yard or in the field wants beyond the
/// level's (TA's Krogoth: 382,193, six more, its Gantry's own work a lone half hour; its silo,
/// three), and those extra hands at most.
const HAND_PER_BUILD_TIME: i64 = 60_000;
const EXTRA_HANDS: i64 = 6;
/// How often (ticks) the long-range guns are looked at, and how long one keeps its target
/// before it turns to the next (what it shells may be gone unseen).
const GUNS_LOOK: u64 = 10 * SECOND;
const GUN_TARGET_TICKS: u64 = 60 * SECOND;

/// A kind of project taking turns with the others (anti-missile cover comes first; an
/// experimental goes up beside them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Project {
    /// A strategic silo.
    Silo,
    /// A tactical launcher.
    Tactical,
    HeavyTower,
    LongGun,
}

impl SkirmishAi {
    /// Whether a unit type is a project (see the module's notes).
    pub(crate) fn is_project(&self, t: usize) -> bool {
        let i = &self.info[t];
        matches!(i.role, Role::Silo | Role::AntiNuke) || i.long_range || self.is_experimental_project(t) || i.role == Role::Defense && i.cost >= HEAVY_COST
    }

    /// What an experimental may cost: so many minutes of our metal income (eight; ten for a
    /// persona that races up the tiers, six for one that rushes). Built in the field or in a
    /// factory's yard (TA's Krogoth: 31,000, on 66 metal a second), one beyond it would starve the
    /// army for longer than the army can wait.
    pub(crate) fn experimental_budget(&self, s: &Survey) -> i64 {
        let minutes = match self.persona {
            Persona::Tech => 10,
            Persona::Rush => 6,
            _ => 8,
        };
        s.m_inc.to_int() * 60 * minutes
    }

    /// Whether a unit type is an experimental, or a factory of them: a project of its own, put
    /// up beside the others (by the constructors that build them, often few).
    pub(crate) fn is_experimental_project(&self, t: usize) -> bool {
        self.info[t].experimental || self.info[t].experimental_factory
    }

    /// An advanced constructor's project, if one is due (none while another is under way, but
    /// anti-missile cover against a launcher seen; an experimental goes up beside another).
    pub(crate) fn project_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        let (m, e) = (s.m_inc.to_int(), s.e_inc.to_int());
        let frames = |experimental: bool| s.nanoframes.iter().any(|n| self.is_project(n.type_id) && self.is_experimental_project(n.type_id) == experimental);
        // (A strategic silo's first missile, when it costs more than two minutes of our income, is
        // a project under way: the next waits for it, an experimental too.)
        let first_missile = self.first_missile_due(s);
        let under_way = plan.project || frames(false) || first_missile;
        // (Not on an economy short of power or metal: what it has goes to that first. Cover
        // against a launcher seen excepted; and the projects saved for, short of metal in store,
        // go ahead on an income that feeds them, the factories leaving them a share of it.)
        let ledger = self.ledger(s);
        let short = ledger.e_low || ledger.metal_starved;
        let saved_for = !ledger.e_low && m >= SAVED_FOR_INCOME;
        // Experimentals, one at a time beside the other projects: one the constructor builds
        // itself (Forged Alliance's), or a factory of them (TA's Gantry).
        if !plan.experimental && !frames(true) && !first_missile && saved_for && s.minutes >= self.p.experimental_minute && e >= 1000 && s.experimentals < self.p.experimentals
            && let Some(t) = self.experimental_task(sim, s, b, opts, plan)
        {
            return Some(t);
        }
        // Anti-nuclear cover: against a silo seen, as many as the level keeps; else one once
        // missiles may be coming (late in the game, on an economy that could build them too).
        let antis = s.count[Role::AntiNuke as usize] + plan.pending[Role::AntiNuke as usize];
        let wanted = if self.enemy_silo {
            self.p.anti_nukes.max(1)
        } else if s.minutes >= self.p.anti_minute && e >= 600 && m >= 15 {
            self.p.anti_nukes.min(1)
        } else {
            0
        };
        // (On the economy the projects saved for go ahead on, too: cover before our own silo.)
        let anti_frame = s.nanoframes.iter().any(|n| self.info[n.type_id].role == Role::AntiNuke) || plan.pending[Role::AntiNuke as usize] > 0;
        if antis < wanted && (!under_way && (!short || saved_for) || self.enemy_silo && !anti_frame) {
            let anti = opts.iter().copied().filter(|&o| self.info[o].role == Role::AntiNuke && self.info[o].land).min_by_key(|&o| (self.info[o].cost, o));
            if let Some(a) = anti && let Some(site) = self.place(sim, b, a, Zone::Ring, 60, plan) {
                return Some(Task::Build(a, site));
            }
        }
        // Cover against tactical missiles once a launcher of them of the enemy's is seen: a
        // tactical defence (an interceptor of their class: never an anti-nuclear one, nor one
        // for these an anti-nuclear defence), one going up at a time, one more every eight
        // minutes since, as many as the level keeps.
        if let Some(seen) = self.enemy_tactical {
            let wanted = (1 + (s.now.saturating_sub(seen) / (8 * MINUTE)) as usize).min(self.p.tactical_covers);
            let going_up = plan.covers > 0 || s.nanoframes.iter().any(|n| self.info[n.type_id].tactical_cover());
            if s.tactical_covers + plan.covers < wanted && !going_up && !ledger.e_crit {
                let cover = opts.iter().copied().filter(|&o| self.info[o].tactical_cover() && self.info[o].land).min_by_key(|&o| (self.info[o].cost, o));
                if let Some(c) = cover && let Some(site) = self.place(sim, b, c, Zone::Ring, 100 + 80 * s.tactical_covers as i64, plan) {
                    log::debug!("{} min: P{} puts up cover against tactical missiles, {}", s.minutes, self.player, sim.types.get(c).name);
                    return Some(Task::Build(c, site));
                }
            }
        }
        if under_way {
            return None;
        }
        // The projects due, each kind in its turn: the kind with the fewest of its own built
        // first (one of each before a second of any), in the persona's order of liking. A
        // strategic silo only for a constructor whose options hold one (Forged Alliance's are
        // its tech 3 engineers' and support commanders': its tech 2 engineers put up the
        // tactical launchers), within so many minutes of our metal income.
        let silos = if s.minutes >= self.p.nuke_minute.saturating_add(16) { self.p.silos } else { self.p.silos.min(1) };
        let (strategic_built, tactical_built) = (s.strategic_silos + plan.silos[0], s.tactical_silos + plan.silos[1]);
        let budget = m * 60 * SILO_INCOME_MINUTES;
        let strategic = opts.iter().copied().filter(|&o| self.info[o].strategic_silo() && self.info[o].land && self.info[o].cost <= budget).min_by_key(|&o| (self.info[o].cost, o));
        let mut due: Vec<(usize, usize, Project)> = Vec::new();
        if (saved_for || !short && m >= 18) && s.minutes >= self.p.nuke_minute && e >= 900 && strategic_built < silos && strategic.is_some() {
            due.push((strategic_built, 0, Project::Silo));
        }
        if !short {
            if s.minutes >= self.p.tactical_minute && m >= 12 && tactical_built < self.p.tactical {
                due.push((tactical_built, 0, Project::Tactical));
            }
            if s.minutes >= self.p.heavy_minute && m >= 20 && s.heavy_towers < self.p.heavy_towers.min(1 + s.minutes as usize / 12) {
                due.push((s.heavy_towers, 0, Project::HeavyTower));
            }
            if s.minutes >= self.p.long_gun_minute && m >= 25 && e >= 700 && s.long_guns < self.p.long_guns {
                due.push((s.long_guns, 0, Project::LongGun));
            }
        }
        let liking: [Project; 4] = match self.persona {
            Persona::Turtle => [Project::HeavyTower, Project::LongGun, Project::Tactical, Project::Silo],
            Persona::Tech => [Project::Silo, Project::Tactical, Project::LongGun, Project::HeavyTower],
            Persona::Rush => [Project::HeavyTower, Project::Tactical, Project::Silo, Project::LongGun],
            _ => [Project::Silo, Project::HeavyTower, Project::Tactical, Project::LongGun],
        };
        for d in &mut due {
            d.1 = liking.iter().position(|&k| k == d.2).unwrap_or(0);
        }
        due.sort_by_key(|&(built, order, _)| (built, order));
        for (_, _, kind) in due {
            let task = match kind {
                Project::Silo => strategic.and_then(|n| {
                    let site = self.place(sim, b, n, Zone::Back, 300, plan)?;
                    log::debug!("{} min: P{} puts up a strategic silo, {}", s.minutes, self.player, sim.types.get(n).name);
                    Some(Task::Build(n, site))
                }),
                Project::Tactical => self.tactical_task(sim, s, b, opts, plan),
                Project::HeavyTower => {
                    let tower = opts
                        .iter()
                        .copied()
                        .filter(|&o| {
                            // (One whose guns draw power only on an income that keeps them firing.)
                            let i = &self.info[o];
                            i.role == Role::Defense && i.land && !i.long_range && i.cost >= HEAVY_COST && i.cost <= m * 60 * LONG_GUN_INCOME_MINUTES && i.fire_energy * 4 <= e
                        })
                        .max_by_key(|&o| (self.info[o].value, Reverse(o)));
                    tower.and_then(|t| self.place(sim, b, t, Zone::Front, 300, plan).map(|site| Task::Build(t, site)))
                }
                Project::LongGun => self.long_gun_task(sim, s, b, opts, plan),
            };
            if task.is_some() {
                return task;
            }
        }
        None
    }

    /// An experimental: one the constructor builds itself where it can (Forged Alliance's), else
    /// a factory of them (TA's Gantry) when we have none: the best it can, within so many minutes
    /// of our metal income (`experimental_budget`); the factory once it and the cheapest it makes
    /// would cost no more than ten minutes of it (one that stands idle, its experimental beyond
    /// our means, is metal lost to the army).
    fn experimental_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        let budget = self.experimental_budget(s);
        let unit = opts
            .iter()
            .copied()
            .filter(|&o| self.info[o].experimental && self.info[o].kind != Locomotion::Naval && self.info[o].kind != Locomotion::Air && self.info[o].cost <= budget)
            .max_by_key(|&o| (self.info[o].value.max(self.info[o].efficiency), Reverse(o)));
        if let Some(x) = unit && let Some(site) = self.place(sim, b, x, Zone::Front, 220, plan) {
            log::debug!("{} min: P{} starts an experimental, {}", s.minutes, self.player, sim.types.get(x).name);
            return Some(Task::Build(x, site));
        }
        let have_factory = s.factory_types.iter().any(|&f| self.info[f].experimental_factory);
        let with_its_own = |f: usize| self.info[f].cost + sim.types.get(f).ai_build_options.iter().filter(|&&o| self.info[o].experimental).map(|&o| self.info[o].cost).min().unwrap_or(0);
        let factory_budget = s.m_inc.to_int() * 60 * EXPERIMENTAL_FACTORY_INCOME_MINUTES;
        let factory = opts.iter().copied().filter(|&o| self.info[o].experimental_factory && self.info[o].land && with_its_own(o) <= factory_budget).min_by_key(|&o| (self.info[o].cost, o));
        // (Away from the enemy: what it builds is long in its yard.)
        if !have_factory && let Some(f) = factory && let Some(site) = self.place(sim, b, f, Zone::Back, 260, plan) {
            log::debug!("{} min: P{} puts up a factory of experimentals, {}", s.minutes, self.player, sim.types.get(f).name);
            return Some(Task::Build(f, site));
        }
        None
    }

    /// A long-range gun on a site of ours from which the nearest enemy base we have found lies
    /// within its reach (towards the enemy, in our half of the ground between), the cheapest
    /// that reaches; none with no enemy base found, or none in reach of any site we can put
    /// one on.
    pub(crate) fn long_gun_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        let budget = s.m_inc.to_int() * 60 * LONG_GUN_INCOME_MINUTES;
        // The cheapest that has a site from which it reaches (one too short for the distance
        // has none: a pricier one with the reach is taken).
        let mut guns: Vec<usize> = opts.iter().copied().filter(|&o| self.info[o].long_range && self.info[o].land && self.info[o].cost <= budget).collect();
        guns.sort_by_key(|&o| (self.info[o].cost, o));
        for gun in guns {
            if let Some(site) = self.site_in_reach(sim, b, gun, self.info[gun].range, plan) {
                log::debug!("{} min: P{} puts up a long-range gun, {}", s.minutes, self.player, sim.types.get(gun).name);
                return Some(Task::Build(gun, site));
            }
        }
        None
    }

    /// A tactical launcher (`TypeInfo::tactical_silo`: a mod's, Forged Alliance's) on a site of
    /// ours from which the nearest enemy base we have found lies within its missiles' reach, as a
    /// long-range gun is sited: the cheapest that reaches.
    pub(crate) fn tactical_task(&mut self, sim: &Sim, s: &Survey, b: &Unit, opts: &[usize], plan: &Plan) -> Option<Task> {
        let budget = s.m_inc.to_int() * 60 * LONG_GUN_INCOME_MINUTES;
        let mut launchers: Vec<usize> = opts.iter().copied().filter(|&o| self.info[o].tactical_silo() && self.info[o].land && self.info[o].cost <= budget).collect();
        launchers.sort_by_key(|&o| (self.info[o].cost, o));
        for launcher in launchers {
            if let Some(site) = self.site_in_reach(sim, b, launcher, self.info[launcher].missile_range, plan) {
                log::debug!("{} min: P{} puts up a tactical launcher, {}", s.minutes, self.player, sim.types.get(launcher).name);
                return Some(Task::Build(launcher, site));
            }
        }
        None
    }

    /// A site for a building of type `t` whose weapon reaches `range` from which the nearest enemy
    /// base we have found lies within that reach: towards the enemy, as far back as it can stand
    /// and still reach (never much past halfway), out of the threats we know and where the builder
    /// gets to. None with no base found, or none in reach: the type then rests a while (looked
    /// for again later, not by every constructor at every turn).
    fn site_in_reach(&mut self, sim: &Sim, b: &Unit, t: usize, range: i64, plan: &Plan) -> Option<FxVec3> {
        let seen = self.enemy_bases.iter().zip(&self.enemy_base_seen).filter(|&(_, &seen)| seen).map(|(&p, _)| p).min_by_key(|p| (dist(*p, self.base), p.x.0, p.z.0))?;
        if self.place_failed.iter().any(|&(f, until)| f == t && sim.tick < until) {
            return None;
        }
        let d = dist(seen, self.base);
        let toward = (seen - self.base).xz().normalize();
        let reach = range * 92 / 100;
        let out = (d - reach).max(200).min(d * 55 / 100);
        let near = Self::clamp_to_map(sim, self.base + toward.scale(fx(out)));
        let r = fx(sim.types.get(t).radius.to_int() + 16);
        // (Past its guns' minimum range of it, where they have one.)
        let least = self.info[t].min_range;
        let ok = |p: FxVec3| p.within_xz(seen, fx(reach)) && (least <= 0 || !p.within_xz(seen, fx(least + 64))) && plan.free(p, r) && self.threat.at(p) == 0 && self.builder_reaches(sim, b, p, t);
        let site = self.find_site(sim, t, near, 24, &ok);
        if site.is_none() {
            self.place_failed.push((t, sim.tick + 30 * SECOND));
        }
        site
    }

    /// Our long-range guns shell what we know of the enemy's buildings in their reach, worth the
    /// most first (its economy and factories, its strategic buildings), away from our own and our
    /// allies' units; each turns to the next after a minute (what it shells may be gone out of
    /// our sight), and back to fire at will with nothing known in reach.
    pub(crate) fn run_long_guns(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        if s.peace || s.now % GUNS_LOOK >= THINK_TICKS {
            return;
        }
        let me = self.player;
        let friends: Vec<FxVec3> = s.own.iter().chain(s.allies.iter()).map(|u| u.pos).collect();
        self.gun_targets.retain(|&(g, _, _)| sim.unit(g).is_some_and(|u| !u.dead && u.owner == me));
        let guns: Vec<&Unit> = s.own.iter().copied().filter(|u| self.info[u.type_id].long_range && Self::can_command(sim, u)).collect();
        for gun in guns {
            let (reach, near) = (self.info[gun.type_id].range, self.info[gun.type_id].min_range);
            let shelling = match gun.orders.front() {
                Some(&Order::Attack(Target::Ground(p))) => Some(p),
                _ => None,
            };
            let kept = self.gun_targets.iter().position(|&(g, _, _)| g == gun.id);
            // On a building we still know of, for less than its time: on with it.
            if let (Some(p), Some(k)) = (shelling, kept) {
                let (_, at, since) = self.gun_targets[k];
                if at == p && s.now < since + GUN_TARGET_TICKS && self.enemy_buildings.iter().any(|b| b.1.within_xz(p, fx(48))) {
                    continue;
                }
            }
            let last = kept.map(|k| self.gun_targets[k].1);
            let best = self
                .enemy_buildings
                .iter()
                // (Past its minimum range, where it has one: Forged Alliance's heavy artillery's.)
                .filter(|(_, p, _)| p.within_xz(gun.pos, fx(reach - 32)) && (near <= 0 || !p.within_xz(gun.pos, fx(near + 32))) && !friends.iter().any(|f| f.within_xz(*p, fx(220))))
                // (Not the one it had just now, while another is known.)
                .map(|&(id, p, t)| (self.target_value(t) * 1000 / (1000 + dist(p, gun.pos) / 4) - if last.is_some_and(|l| l.within_xz(p, fx(48))) { 1_000_000 } else { 0 }, id.slot, p))
                .max_by_key(|&(score, slot, _)| (score, Reverse(slot)));
            if let Some(k) = kept {
                self.gun_targets.remove(k);
            }
            match best {
                Some((_, _, p)) => {
                    log::debug!("{} min: P{me} shells ({}, {}) with its {}", s.minutes, p.x.to_int(), p.z.to_int(), sim.unit_type(gun).name);
                    self.gun_targets.push((gun.id, p, s.now));
                    out.push(Command::Order { player: me, units: vec![gun.id], order: Order::Attack(Target::Ground(p)), queue: false });
                }
                None if shelling.is_some() => out.push(Command::Order { player: me, units: vec![gun.id], order: Order::Stop, queue: false }),
                None => {}
            }
        }
    }
}

impl SkirmishAi {
    /// A hand for a project under way: a constructor near enough helps finish it while fewer than
    /// it wants are at it (alone, one takes many minutes over a silo or an anti-nuclear defence,
    /// the Commander only near home): the level's, and more the longer it takes to build (TA's
    /// Krogoth, half an hour of its Gantry's work alone). A frame is mended; an experimental in a
    /// factory's yard is helped at the factory (a guard, whom the simulation sets to work on what
    /// the factory builds), and one at it already is kept at it.
    pub(crate) fn help_project_task(&self, sim: &Sim, s: &Survey, b: &Unit, plan: &Plan) -> Option<Task> {
        let reach = if self.info[b.type_id].commander { 900 } else { 2500 };
        let unreachable = |id: UnitId| self.unreachable_units.iter().any(|&(u, _)| u == id);
        let frames = s.nanoframes.iter().filter(|n| self.is_project(n.type_id) && !unreachable(n.id)).map(|&n| (n, None));
        let yards = s.yard_projects.iter().filter_map(|&(u, f)| sim.unit(f).filter(|f| !f.dead && !unreachable(f.id)).map(|f| (u, Some(f))));
        let (frame, factory) = frames.chain(yards).min_by_key(|(n, f)| (dist(f.map_or(n.pos, |f| f.pos), b.pos), n.id.slot))?;
        let at = factory.map_or(frame.pos, |f| f.pos);
        if dist(at, b.pos) > reach {
            return None;
        }
        let at_it = |h: &Unit| {
            h.build.is_some_and(|j| j.target == frame.id)
                || matches!(h.orders.front(), Some(&Order::Repair(t)) if t == frame.id)
                || factory.is_some_and(|f| matches!(h.orders.front(), Some(&Order::Guard(g)) if g == f.id))
        };
        let hands = s.builders.iter().filter(|h| h.id != b.id && at_it(h)).count() + plan.hands.iter().filter(|&&f| f == frame.id || factory.is_some_and(|fu| fu.id == f)).count();
        let extra = (sim.types.get(frame.type_id).build_time.to_int() / HAND_PER_BUILD_TIME).clamp(0, EXTRA_HANDS) as usize;
        let wanted = match self.difficulty {
            Difficulty::Easy => 1,
            Difficulty::Normal => 3 + extra,
            Difficulty::Hard => 4 + extra,
        } + (self.persona == Persona::Tech) as usize;
        let here = at_it(b);
        if hands >= wanted && !here || !self.unit_reaches(sim, b, at) {
            return None;
        }
        match factory {
            Some(f) => Some(Task::Guard(f.id)),
            None if here => None,
            None => Some(Task::Repair(frame.id)),
        }
    }
}
