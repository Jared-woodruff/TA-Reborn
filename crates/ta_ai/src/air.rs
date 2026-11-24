//! Defence against the air: anti-aircraft over what the enemy's aircraft would strike, in
//! proportion to how many of them it fields, and built in time. Its bombers go for our economy
//! (extractors, power, metal makers, factories: `pick_air_target`, the outskirts first), so cover
//! goes where that is and has least of it (`air_exposed`): anti-aircraft buildings there (and
//! beside an extractor put up where none reaches), mobile anti-aircraft posted there, fighters on
//! patrol over it. Under a heavy threat (`air_heavy`: many armed aircraft seen at once, or a raid
//! from the air lately) the anti-aircraft comes before more of the economy it covers. All of it
//! from what the enemy fields and what is seen of it, whatever the side.

use super::*;

/// What a part of our economy must be worth (to a bomber, as `exposed_to_air` weighs it) for
/// anti-aircraft to be put up for it: two or three small extractors, a power plant or two, a
/// factory.
const EXPOSED_WORTH: i64 = 300;

impl SkirmishAi {
    /// How many armed enemy aircraft threaten us: the most seen at once lately, one fewer for every
    /// `AIR_FADE` since.
    pub(crate) fn air_pressure(&self, now: u64) -> usize {
        let (peak, at) = self.air_peak;
        peak.saturating_sub((now.saturating_sub(at) / AIR_FADE) as usize)
    }

    /// Whether the air is a heavy threat: `HEAVY_AIR` armed enemy aircraft or more seen at once
    /// lately, or a raid from the air on us in the last two minutes. (Never on Easy: its slower hand
    /// is the point, and its anti-aircraft stays as few as a light threat's.)
    pub(crate) fn air_heavy(&self, now: u64) -> bool {
        self.difficulty != Difficulty::Easy && self.air_threat(now) && (self.air_pressure(now) >= HEAVY_AIR || self.air_raids.iter().any(|&(_, t)| now < t + 2 * MINUTE))
    }

    /// What an anti-aircraft building or unit counts for in our cover (`own_aa`): one for every
    /// hundred of its cost, from one to four.
    pub(crate) fn aa_strength(&self, t: usize) -> i64 {
        (self.info[t].cost / 100).clamp(1, 4)
    }

    /// How far round an anti-aircraft building our cover counts: three quarters of its reach at
    /// aircraft, 200 to 700 px.
    pub(crate) fn aa_reach(&self, t: usize) -> i64 {
        (self.info[t].air_range * 3 / 4).clamp(200, 700)
    }

    /// The cover wanted over each part of our economy (in `aa_strength`): one for a few aircraft,
    /// one more for every eight seen at once lately.
    pub(crate) fn air_cover_wanted(&self, now: u64) -> i64 {
        1 + (self.air_pressure(now) / 8) as i64
    }

    /// Each think, our cover from the air (`own_aa`: our anti-aircraft buildings, those going up
    /// too), and while enemy aircraft threaten, the parts of our economy short of it, worth most
    /// first (`air_exposed`), and the posts of our mobile anti-aircraft (`aa_posts`: the rally
    /// point, then the three most exposed of them).
    pub(crate) fn plan_air_cover(&mut self, sim: &Sim, s: &Survey) {
        self.own_aa.clear();
        for u in &s.own {
            if self.info[u.type_id].role == Role::AntiAir {
                self.own_aa.add_disc(u.pos, self.aa_reach(u.type_id), self.aa_strength(u.type_id));
            }
        }
        self.air_exposed.clear();
        self.aa_posts.clear();
        if !self.air_threat(s.now) {
            return;
        }
        self.air_exposed = self.exposed_to_air(sim, s);
        self.aa_posts.push(self.rally);
        let posts: Vec<FxVec3> = self.air_exposed.iter().take(3).map(|&(p, _)| p).filter(|p| !p.within_xz(self.rally, fx(RAID_NEAR))).collect();
        self.aa_posts.extend(posts);
    }

    /// The parts of our economy short of cover from the air (`air_cover_wanted`), worth most first:
    /// each building as an enemy bomber's strike weighs it (extractors, power plants and metal
    /// makers three times their cost, factories twice, storage and silos once), twice where it was
    /// raided from the air lately; those within `RAID_NEAR` of each other one part, at its worthiest
    /// building; parts worth less than `EXPOSED_WORTH` left out.
    pub(crate) fn exposed_to_air(&self, sim: &Sim, s: &Survey) -> Vec<(FxVec3, i64)> {
        let need = self.air_cover_wanted(s.now);
        // (Where, what it is all worth, its worthiest building's worth.)
        let mut parts: Vec<(FxVec3, i64, i64)> = Vec::new();
        for u in &s.own {
            let i = &self.info[u.type_id];
            if sim.unit_type(u).locomotion != Locomotion::Static || self.own_aa.at(u.pos) >= need {
                continue;
            }
            let worth = match i.role {
                Role::Extractor | Role::Generator | Role::MetalMaker => i.cost * 3,
                Role::Factory => i.cost * 2,
                Role::Storage | Role::Silo | Role::AntiNuke => i.cost,
                _ => continue,
            };
            let worth = if self.air_raids.iter().any(|(p, _)| p.within_xz(u.pos, fx(RAID_NEAR))) { worth * 2 } else { worth };
            match parts.iter_mut().find(|(p, ..)| p.within_xz(u.pos, fx(RAID_NEAR))) {
                Some(part) => {
                    part.1 += worth;
                    if worth > part.2 {
                        part.0 = u.pos;
                        part.2 = worth;
                    }
                }
                None => parts.push((u.pos, worth, worth)),
            }
        }
        parts.retain(|&(_, w, _)| w >= EXPOSED_WORTH);
        parts.sort_by_key(|&(p, w, _)| (Reverse(w), p.x.0, p.z.0));
        parts.into_iter().map(|(p, w, _)| (p, w)).collect()
    }

    /// Anti-aircraft buildings wanted while enemy aircraft threaten: a few more the more of them
    /// there were (one, and one for every three seen at once lately, six at most); under a heavy
    /// threat two, and two for every three, or what we have and one for each part of our economy
    /// short of cover (four at most at once), whichever is more, within what the economy pays for
    /// (three, and one for every two metal a second of income).
    pub(crate) fn aa_towers_wanted(&self, s: &Survey) -> usize {
        if !self.air_threat(s.now) {
            return 0;
        }
        let p = self.air_pressure(s.now);
        if !self.air_heavy(s.now) {
            return (1 + p / 3).min(6);
        }
        let by_count = 2 + p * 2 / 3;
        let by_cover = s.count[Role::AntiAir as usize] + self.air_exposed.len().min(4);
        let affordable = 3 + (s.m_inc.to_int().max(0) / 2) as usize;
        by_count.max(by_cover).min(affordable)
    }

    /// Mobile anti-aircraft wanted while enemy aircraft threaten: one, and one for every two seen at
    /// once lately (eight at most); under a heavy threat two, and one for each seen, up to eight and
    /// one more a minute of the game, and a fifth of the army's limit (eight at least). Of them
    /// fighters, where an air plant of ours turns them out: a third under a heavy threat.
    pub(crate) fn aa_units_wanted(&self, s: &Survey) -> usize {
        if !self.air_threat(s.now) {
            return 0;
        }
        let p = self.air_pressure(s.now);
        if !self.air_heavy(s.now) {
            return (1 + p / 2).min(8);
        }
        (2 + p).min(8 + s.minutes as usize).min((self.p.army_cap / 5).max(8))
    }

    /// The best anti-aircraft building of `opts` within a budget (800, or forty seconds of our
    /// metal income).
    pub(crate) fn pick_aa_tower(&self, s: &Survey, opts: &[usize]) -> Option<usize> {
        let budget = (s.m_inc.to_int() * 40).max(800);
        let cands: Vec<usize> = opts.iter().copied().filter(|&o| self.info[o].role == Role::AntiAir && self.info[o].cost <= budget).collect();
        self.pick_best(&cands, Role::AntiAir)
    }

    /// A site for an anti-aircraft building of type `a`: beside the part of our economy most
    /// exposed from the air (`air_exposed`: the first three with none planned near them, within
    /// `reach` of home), towards the enemy, clear of the enemy's guns, where the builder gets to.
    pub(crate) fn aa_site(&self, sim: &Sim, b: &Unit, a: usize, reach: Option<i64>, plan: &Plan) -> Option<FxVec3> {
        let r = fx(sim.types.get(a).radius.to_int() + 16);
        let ok = |p: FxVec3| plan.free(p, r) && self.threat.at(p) == 0 && self.builder_reaches(sim, b, p, a);
        let parts = self
            .air_exposed
            .iter()
            .filter(|&&(p, _)| !plan.aa.iter().any(|q| q.within_xz(p, fx(RAID_NEAR))) && reach.is_none_or(|m| dist(p, self.base) <= m))
            .take(3);
        for &(at, _) in parts {
            let toward = self.nearest_enemy_base(at).map_or(FxVec3::ZERO, |e| (e - at).xz().normalize());
            if let Some(site) = self.find_site(sim, a, at + toward.scale(fx(72)), 8, &ok) {
                return Some(site);
            }
        }
        None
    }

    /// Under a heavy threat from the air, an anti-aircraft building beside an extractor about to
    /// go up where our cover does not reach (none planned near it either, and fewer than
    /// `aa_towers_wanted` of ours): put up after it.
    pub(crate) fn aa_beside(&self, sim: &Sim, s: &Survey, b: &Unit, mex: FxVec3, plan: &Plan) -> Option<(usize, FxVec3)> {
        if !self.air_heavy(s.now) || self.own_aa.at(mex) > 0 || plan.aa.iter().any(|q| q.within_xz(mex, fx(RAID_NEAR))) {
            return None;
        }
        if s.count[Role::AntiAir as usize] + plan.pending[Role::AntiAir as usize] >= self.aa_towers_wanted(s) {
            return None;
        }
        let opts: Vec<usize> = b.ai_build_options(sim.unit_type(b)).iter().copied().filter(|&o| self.info[o].land && sim.is_allowed(o)).collect();
        let a = self.pick_aa_tower(s, &opts)?;
        let r = fx(sim.types.get(a).radius.to_int() + 16);
        let toward = self.nearest_enemy_base(mex).map_or(FxVec3::ZERO, |e| (e - mex).xz().normalize());
        let site = self.find_site(sim, a, mex + toward.scale(fx(64)), 6, &|p| plan.free(p, r) && self.threat.at(p) == 0)?;
        self.builder_reaches(sim, b, site, a).then_some((a, site))
    }

    /// Idle anti-aircraft: on the ground it waits at its post (`aa_posts`, by unit: the rally point
    /// with no aircraft about), the rally point where it cannot get there; in the air, fighters
    /// patrol between home and the parts of our economy most exposed (once on patrol they keep to
    /// it).
    pub(crate) fn post_anti_air(&self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let me = self.player;
        let rally = self.rally;
        let posts = if self.aa_posts.is_empty() { vec![rally] } else { self.aa_posts.clone() };
        let mut at_post = vec![0usize; posts.len()];
        for u in &s.aa_army {
            at_post[u.id.slot as usize % posts.len()] += 1;
        }
        let mut gather: Vec<Vec<UnitId>> = vec![Vec::new(); posts.len()];
        let mut fighters: Vec<UnitId> = Vec::new();
        for u in s.aa_army.iter().filter(|u| u.orders.is_empty()) {
            if self.info[u.type_id].kind == Locomotion::Air {
                fighters.push(u.id);
                continue;
            }
            let k = u.id.slot as usize % posts.len();
            let (k, near) = if self.unit_reaches(sim, u, posts[k]) { (k, gathered_within(sim, at_post[k])) } else { (0, gathered_within(sim, s.aa_army.len())) };
            if !u.pos.within_xz(posts[k], near) && self.unit_reaches(sim, u, posts[k]) {
                gather[k].push(u.id);
            }
        }
        for (k, units) in gather.into_iter().enumerate() {
            if !units.is_empty() {
                out.push(Command::Order { player: me, units, order: Order::Move(posts[k]), queue: false });
            }
        }
        if fighters.is_empty() {
            return;
        }
        // (Home, then the most exposed parts; with none, home and the rally point.)
        let mut route: Vec<FxVec3> = vec![self.base];
        route.extend(self.air_exposed.iter().take(2).map(|&(p, _)| p).filter(|p| !p.within_xz(self.base, fx(RAID_NEAR))));
        if route.len() == 1 {
            route.push(rally);
        }
        for (k, &p) in route.iter().enumerate() {
            out.push(Command::Order { player: me, units: fighters.clone(), order: Order::Patrol(p), queue: k > 0 });
        }
        log::debug!("{} min: P{me} sends {} fighters on patrol over {} places", s.minutes, fighters.len(), route.len());
    }
}
