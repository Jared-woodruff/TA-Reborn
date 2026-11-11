//! Duties: standing work for a player's own constructors.
//!
//! A player puts a constructor on a [`Duty`] (the economy, repairs, reclaiming, defence) and a
//! [`DutyOfficer`] sends it where the work is, with the orders a player would give it: it
//! reuses the skirmish AI's builders (what to put up where, what to mend, what to reclaim),
//! asked about this one player's units on duty and nobody else's. The officer's orders are
//! ordinary [`Command`]s for the player's own units, so the game sends them the way it sends the
//! player's (lockstep, replays and saved games need nothing more), and any order the player gives
//! a unit takes it off duty again (the game's business: see `Game::issue`).
//!
//! The officer is a person's helper, not a computer player: it never runs the AI's factories,
//! army, scouts or metal makers' switches, never answers allies, goes only where the player has
//! been (the AI's own knowledge of the map is the player's, too: what is in the team's sight, and
//! what it remembers), and never blows up a building of the player's.
//!
//! A player has to be able to trust a constructor on duty, so it also keeps out of harm's way and
//! out of the others' way: it passes over a job whose way crosses the guns of something armed it
//! has seen (even when that is out of sight now), gets away from armed enemies that come close,
//! never works two to a hurt unit or a wreck, does not make for what it cannot reach again and
//! again, and hands the Commander back to the player when enemies are close.

use std::collections::{BTreeMap, BTreeSet};

use super::*;

/// What a constructor on duty does by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub enum Duty {
    /// Metal extractors on free deposits, power plants when power runs short (solar collectors,
    /// wind generators where the wind pays better, tidal generators at sea; an advanced
    /// constructor's big plant), metal makers when there is power to spare, and buildings of those
    /// kinds left unfinished near it; nothing else (kept plain, so that it can be counted on: no
    /// storage, geothermal plants, upgrades, reclaiming or help at the factories). Never
    /// factories, defences or strategic buildings.
    Economy,
    /// Mends the most damaged buildings and units near the base; otherwise finishes what is
    /// unfinished and helps the factories.
    Repair,
    /// Reclaims wrecks, then rocks and trees near the base, for their metal and energy.
    Reclaim,
    /// Towers where the base faces the enemy, anti-aircraft once planes show themselves and guard
    /// towers at outlying extractors; otherwise it finishes what is unfinished and helps the
    /// factories.
    Defence,
}

impl Duty {
    /// Every duty, in the order the interface lists them.
    pub const ALL: [Duty; 4] = [Duty::Economy, Duty::Repair, Duty::Reclaim, Duty::Defence];

    /// The duty after `current` when cycling through them: all four, then none, then the first.
    pub fn next(current: Option<Duty>) -> Option<Duty> {
        match current {
            None => Some(Duty::ALL[0]),
            Some(d) => Duty::ALL.get(Duty::ALL.iter().position(|&x| x == d).map_or(0, |i| i + 1)).copied(),
        }
    }

    /// Its name in English, lower case (for logs and tests; the interface translates its own).
    pub fn name(self) -> &'static str {
        match self {
            Duty::Economy => "economy",
            Duty::Repair => "repair",
            Duty::Reclaim => "reclaim",
            Duty::Defence => "defence",
        }
    }

    /// Its number in a saved game (fixed for good: the order of the enum may change, these may not).
    fn code(self) -> u8 {
        match self {
            Duty::Economy => 0,
            Duty::Repair => 1,
            Duty::Reclaim => 2,
            Duty::Defence => 3,
        }
    }

    fn from_code(code: u8) -> Option<Duty> {
        Some(match code {
            0 => Duty::Economy,
            1 => Duty::Repair,
            2 => Duty::Reclaim,
            3 => Duty::Defence,
            _ => return None,
        })
    }
}

/// The format byte of what [`DutyOfficer::export`] writes: bytes of another format are not read.
pub(crate) const SAVED_FORMAT: u8 = 1;
/// Bytes a saved game may hold of duties at most (nine for each constructor on duty).
pub const MAX_SAVED_BYTES: usize = 1 << 20;

/// How far from the middle of the base a unit may be sent to a job, and a builder's
/// working radius: the Commander stays close to home, the others go further.
pub(crate) const COMMANDER_REACH: i64 = 700;
const BUILDER_REACH: i64 = 1600;
/// Armed enemies within this many px of a constructor, worth this much together or more, are
/// danger to it: a builder gets away from them, the Commander is handed back to the player.
const DANGER_RANGE: i64 = 380;
const DANGER_WORTH: i64 = 100;
/// A builder that has got away holds back this long after the danger is gone.
const HOLD_BACK: u64 = 8 * SECOND;
/// How far from home an economy builder looks for free metal: the Commander's first extractors a little
/// further out when there are no deposits round the start.
const COMMANDER_FAR: i64 = 650;
const COMMANDER_FAR_EARLY: i64 = 1000;
const BUILDER_FAR: i64 = 2200;
/// Rocks and trees further from the base than this are left alone.
const SCRAP_RANGE: i64 = 1600;
/// A building or unit must be this much below full health (percent) to be worth mending.
const MEND_BELOW: i64 = 97;
/// A helper at a factory is looked at again every this many thinks (two seconds).
const HELPER_LOOK: u64 = 4;

/// A constructor that has had nothing to do for this long is told of (once, for each duty and reason).
const IDLE_NOTICE: u64 = 20 * SECOND;
/// The same notice (a duty and a reason) is not given again within this long, whichever unit it is for.
const IDLE_AGAIN: u64 = 2 * MINUTE;
/// What an advanced constructor on the economy duty waits for before it upgrades extractors or mines on free
/// deposits: a metal income this high (a second) or this much in store.
const ADVANCED_INCOME: i64 = 10;
const ADVANCED_STOCK: i64 = 1500;

/// Something the officer did on its own that the player should hear of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DutyEvent {
    /// The Commander had armed enemies close and was handed back to the player (it is the unit
    /// whose loss loses the battle, and what it should do about them is the player's to say).
    Danger(UnitId),
    /// A constructor has had nothing to do for a while, and why: it stays on its duty, and the
    /// player may want it elsewhere. Told once after a duty is given, and the same words (a duty and
    /// a reason) not again within two minutes, whichever unit it is for.
    Idle { unit: UnitId, duty: Duty, why: IdleReason },
}

/// Why a constructor on duty has nothing to do (the words are the game's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleReason {
    /// Economy: every free metal deposit in its reach is taken (or is where it will not go).
    NoDeposits,
    /// Economy: it has built what the economy wants.
    EconomyDone,
    /// Economy, an advanced constructor: it waits for this much metal income (a second) before it
    /// upgrades extractors or raises mines of its own.
    WaitingForIncome(i64),
    /// Repair: nothing near home is hurt (or what is, is under fire or on the move).
    NothingHurt,
    /// Reclaim: no wreck or feature near home is worth taking.
    NothingToReclaim,
    /// Reclaim: both stores are full, so there is nothing to gain.
    StoresFull,
    /// Defence: the towers wanted stand.
    DefencesDone,
    /// Defence: a tower is wanted but the economy cannot pay for it within a minute.
    SavingUp,
    /// Away from home with no way back (another island, walled in).
    Stranded,
}

/// What a constructor on duty is about, as far as the player is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DutyStatus {
    /// At a job, on its way to one or lending a hand: its orders say what.
    Working,
    /// Getting away from armed enemies (and holding back a while after).
    GettingAway,
    /// With nothing to do.
    Idle(IdleReason),
}

/// The player's helper: the constructors it has on duty and the brain that sends them to work.
#[derive(Clone)]
pub struct DutyOfficer {
    pub(crate) ai: SkirmishAi,
    duties: BTreeMap<UnitId, Duty>,
    /// The units just put on a duty (or moved to another) that have not been looked at since.
    pub(crate) fresh: BTreeSet<UnitId>,
    /// When the middle of the base was last worked out.
    homed: u64,
    /// What happened that the player is to be told, until taken (`take_events`).
    events: Vec<DutyEvent>,
    /// What each unit on duty was about at the last beat (`status`).
    status: BTreeMap<UnitId, DutyStatus>,
    /// Since when each unit with nothing to do has had nothing to do.
    idle_since: BTreeMap<UnitId, u64>,
    /// The units whose idleness the player has been told of since they were given their duty.
    told: BTreeSet<UnitId>,
    /// What was last told of, and when: each duty and reason at most once in `IDLE_AGAIN`.
    notified: Vec<(Duty, IdleReason, u64)>,
}

impl DutyOfficer {
    /// An officer for `player`, with nobody on duty yet.
    pub fn new(player: u8, sim: &Sim) -> DutyOfficer {
        // (Hard's economy: two power plants and two extractor upgrades at once; the rest of what
        // sets the levels apart is the computer players' alone.)
        let mut ai = SkirmishAi::new(player, sim, Difficulty::Hard);
        ai.helper = true;
        // (Metal makers, which only the player switches off again, in moderation.)
        ai.tweak("max_makers", 10);
        let mut officer = DutyOfficer { ai, duties: BTreeMap::new(), fresh: BTreeSet::new(), homed: sim.tick, events: Vec::new(), status: BTreeMap::new(), idle_since: BTreeMap::new(), told: BTreeSet::new(), notified: Vec::new() };
        officer.rehome(sim);
        officer
    }

    pub fn player(&self) -> u8 {
        self.ai.player
    }

    /// Whether a unit can take a duty: a constructor that moves (factories and nano towers
    /// cannot).
    pub fn can_serve(sim: &Sim, unit: &Unit) -> bool {
        let t = sim.unit_type(unit);
        t.is_builder && t.locomotion != Locomotion::Static
    }

    /// Puts a unit on a duty, or back under the player's command (`None`).
    pub fn set(&mut self, unit: UnitId, duty: Option<Duty>) {
        // An extractor it was upgrading for the economy is given up off that duty: nothing the
        // officer still has in hand orders a unit the player has taken back.
        if duty != Some(Duty::Economy) {
            self.ai.upgrades.retain(|u| u.builder != unit);
        }
        match duty {
            Some(d) => {
                if self.duties.insert(unit, d) != Some(d) {
                    // (A new duty: what the last said no longer holds, and nothing was told of this one.)
                    self.fresh.insert(unit);
                    self.status.remove(&unit);
                    self.idle_since.remove(&unit);
                    self.told.remove(&unit);
                }
            }
            None => {
                self.duties.remove(&unit);
                self.fresh.remove(&unit);
                self.status.remove(&unit);
                self.idle_since.remove(&unit);
                self.told.remove(&unit);
            }
        }
    }

    /// What a unit on duty was about at the last beat (none before the first beat after its duty began).
    pub fn status(&self, unit: UnitId) -> Option<DutyStatus> {
        self.status.get(&unit).copied()
    }

    /// The duty a unit is on, if it is on one.
    pub fn duty(&self, unit: UnitId) -> Option<Duty> {
        self.duties.get(&unit).copied()
    }

    /// The units on duty and their duties, in unit order.
    pub fn units(&self) -> impl Iterator<Item = (UnitId, Duty)> + '_ {
        self.duties.iter().map(|(&id, &d)| (id, d))
    }

    pub fn len(&self) -> usize {
        self.duties.len()
    }

    pub fn is_empty(&self) -> bool {
        self.duties.is_empty()
    }

    /// What the officer did on its own since the last call, for the game to tell the player.
    pub fn take_events(&mut self) -> Vec<DutyEvent> {
        std::mem::take(&mut self.events)
    }

    /// The constructors on duty, as the few bytes a saved game keeps of them: the duties are the
    /// player's machine's own (the orders they led to are in the battle's record, the duties are
    /// not), so a loaded battle would start with every constructor under command without them.
    /// Nothing on duty is nothing to keep.
    pub fn export(&self) -> Vec<u8> {
        if self.duties.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(5 + 9 * self.duties.len());
        out.push(SAVED_FORMAT);
        out.extend_from_slice(&(self.duties.len() as u32).to_le_bytes());
        for (id, duty) in &self.duties {
            out.extend_from_slice(&id.slot.to_le_bytes());
            out.extend_from_slice(&id.generation.to_le_bytes());
            out.push(duty.code());
        }
        out
    }

    /// Puts back the duties [`DutyOfficer::export`] wrote, on the units of ours that are still there,
    /// finished and able to serve; how many were put back. Bytes that make no sense put back none:
    /// a battle is never lost for want of its duties.
    pub fn import(&mut self, sim: &Sim, bytes: &[u8]) -> usize {
        let me = self.ai.player;
        let Some((&SAVED_FORMAT, rest)) = bytes.split_first() else { return 0 };
        let Some((count, mut rest)) = rest.split_first_chunk::<4>().map(|(count, rest)| (u32::from_le_bytes(*count) as usize, rest)) else { return 0 };
        let mut put = 0;
        for _ in 0..count.min(rest.len() / 9) {
            let (entry, more) = rest.split_at(9);
            rest = more;
            let id = UnitId { slot: u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]), generation: u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]) };
            let Some(duty) = Duty::from_code(entry[8]) else { continue };
            if sim.unit(id).is_some_and(|u| !u.dead && u.owner == me && u.is_complete() && Self::can_serve(sim, u)) {
                self.set(id, Some(duty));
                put += 1;
            }
        }
        put
    }

    /// This tick's orders for the units on duty (and for no one else). Like the computer players
    /// it thinks every `THINK_TICKS` ticks, on the simulation's state alone.
    pub fn think(&mut self, sim: &Sim) -> Vec<Command> {
        let mut out = Vec::new();
        let me = self.ai.player;
        let phase = me as u64 * THINK_TICKS / sim.players.len().max(1) as u64;
        if !(sim.tick + phase).is_multiple_of(THINK_TICKS) {
            return out;
        }
        // Units lost, given away (or taken) and no longer finished leave their duty.
        self.duties.retain(|&id, _| sim.unit(id).is_some_and(|u| !u.dead && u.owner == me && u.is_complete() && Self::can_serve(sim, u)));
        let duties = &self.duties;
        self.fresh.retain(|id| duties.contains_key(id));
        self.status.retain(|id, _| duties.contains_key(id));
        self.idle_since.retain(|id, _| duties.contains_key(id));
        self.told.retain(|id| duties.contains_key(id));
        if self.duties.is_empty() || sim.players.get(me as usize).is_none_or(|p| p.defeated) {
            return out;
        }
        if sim.tick >= self.homed + MINUTE {
            self.homed = sim.tick;
            self.rehome(sim);
        }
        let s = self.ai.survey(sim);
        if s.own.is_empty() {
            return out;
        }
        self.ai.remember(sim, &s);
        self.ai.forget_defeated(sim);
        if s.now % (10 * SECOND) < THINK_TICKS {
            self.ai.mark_far_deposits(sim, &s);
        }
        // The Commander with armed enemies in sight close by is handed back to the player.
        let hard_pressed: Vec<UnitId> = s
            .builders
            .iter()
            .filter(|b| self.duties.contains_key(&b.id) && self.ai.info[b.type_id].commander && self.ai.seen_enemies_near(&s, b.pos, DANGER_RANGE) >= DANGER_WORTH)
            .map(|b| b.id)
            .collect();
        for id in hard_pressed {
            self.set(id, None);
            self.ai.upgrades.retain(|u| u.builder != id);
            self.events.push(DutyEvent::Danger(id));
        }
        self.ai.run_duties(sim, &s, &self.duties, &mut self.fresh, &mut self.status, &mut out);
        self.note_idleness(sim.tick);
        out
    }

    /// Notes how long each constructor has had nothing to do, and tells of it (once after a duty is
    /// given): the first of those idle for `IDLE_NOTICE`, for each duty and reason.
    fn note_idleness(&mut self, now: u64) {
        let idle: Vec<(UnitId, IdleReason)> = self.status.iter().filter_map(|(&id, s)| if let DutyStatus::Idle(why) = s { Some((id, *why)) } else { None }).collect();
        self.idle_since.retain(|id, _| idle.iter().any(|(i, _)| i == id));
        let mut said: Vec<(Duty, IdleReason)> = Vec::new();
        for (id, why) in idle {
            let since = *self.idle_since.entry(id).or_insert(now);
            if now < since + IDLE_NOTICE || self.told.contains(&id) {
                continue;
            }
            let Some(&duty) = self.duties.get(&id) else { continue };
            self.told.insert(id);
            // (One notice for each duty and reason at a time: four reclaimers running out of work one after
            // the other are one piece of news.)
            let recent = self.notified.iter().any(|&(d, w, at)| d == duty && w == why && now < at + IDLE_AGAIN);
            if !said.contains(&(duty, why)) && !recent {
                said.push((duty, why));
                self.notified.retain(|&(d, w, _)| !(d == duty && w == why));
                self.notified.push((duty, why, now));
                self.events.push(DutyEvent::Idle { unit: id, duty, why });
            }
        }
    }

    /// The middle of the player's base: of the finished economy buildings within reach of
    /// where they started, once there are a few, else that start. (A Commander far afield, or
    /// dead, makes no difference to where "near the base" is.)
    fn rehome(&mut self, sim: &Sim) {
        let me = self.ai.player;
        let start = sim.players.get(me as usize).map(|p| p.start).filter(|&p| p != FxVec3::ZERO);
        let anchor = start.unwrap_or(self.ai.base);
        let (mut sum, mut n) = (FxVec3::ZERO, 0i64);
        for u in sim.units() {
            if u.owner != me || u.dead || !u.is_complete() || !u.pos.within_xz(anchor, fx(1400)) {
                continue;
            }
            if matches!(self.ai.info[u.type_id].role, Role::Factory | Role::Extractor | Role::Generator | Role::MetalMaker | Role::Storage) {
                sum += u.pos;
                n += 1;
            }
        }
        self.ai.base = if n >= 4 { FxVec3::new(sum.x / n, Fx::ZERO, sum.z / n) } else { anchor };
    }
}

impl SkirmishAi {
    /// The orders for every constructor on duty that is free for a new job: as a computer
    /// player's builders are sent out, but each by its own duty's rules.
    pub(crate) fn run_duties(&mut self, sim: &Sim, s: &Survey, duties: &BTreeMap<UnitId, Duty>, fresh: &mut BTreeSet<UnitId>, status: &mut BTreeMap<UnitId, DutyStatus>, out: &mut Vec<Command>) {
        let mine: Vec<(&Unit, Duty)> = s.builders.iter().copied().filter_map(|b| duties.get(&b.id).map(|&d| (b, d))).collect();
        if mine.is_empty() {
            return;
        }
        self.lanes = s.own.iter().filter(|u| self.info[u.type_id].role == Role::Factory).map(|u| Self::exit_lane(u.pos, sim.unit_type(u).footprint, u.heading)).collect();
        if s.now % (20 * SECOND) < THINK_TICKS {
            self.find_clutter(sim, s);
        }
        self.clutter.retain(|&f| sim.features.get(f).is_some_and(|f| f.alive));
        let (mut plan, upgrading) = self.start_builders(sim, s, out);
        // What every builder is at already (the player's own too): two are not sent at one job.
        for h in &s.builders {
            match h.orders.front() {
                Some(&Order::Reclaim(f)) => plan.claims.push((h.id, Claim::Reclaim(f))),
                Some(&Order::Repair(t)) if sim.unit(t).is_some_and(|t| t.is_complete()) => plan.claims.push((h.id, Claim::Mend(t))),
                _ => {}
            }
        }
        self.retreating.retain(|&(_, until)| s.now < until + MINUTE);
        let me = self.player;
        let power_out = self.energy_avg < Fx::from_ratio(6, 100) && s.e_inc <= s.e_exp;
        for (b, duty) in mine {
            // (In a transport, or busy upgrading an extractor, or just told to build the better
            // one: left as it is.)
            if b.carried_by.is_some() || self.upgrades.iter().any(|u| u.builder == b.id) || upgrading.contains(&b.id) {
                status.insert(b.id, DutyStatus::Working);
                continue;
            }
            // Armed enemies close: away from them first, and it holds back a while after.
            if self.keep_away(sim, s, b, out) {
                status.insert(b.id, DutyStatus::GettingAway);
                continue;
            }
            // (A helper at a factory, a Guard among its orders, is left to its factory but looked at
            // again every couple of seconds, and at once when it has just been given this duty: work
            // of the duty's own takes it off the factory. One on its way to a job, or at it, finishes
            // that first: a duty starts when the unit is free.)
            let helper = b.orders.iter().any(|o| matches!(o, Order::Guard(_)));
            let new = fresh.remove(&b.id);
            let look = helper && (new || (s.now / THINK_TICKS + b.id.slot as u64).is_multiple_of(HELPER_LOOK));
            if !look && self.leave_alone(s, b, power_out, out) {
                status.insert(b.id, DutyStatus::Working);
                continue;
            }
            // (A job whose way leads through the guns of something armed it has seen is passed over,
            // and another looked for: up to three tries.)
            let mut task = None;
            for _ in 0..3 {
                match self.duty_task(duty, sim, s, b, &mut plan) {
                    Some(t) if !self.route_safe(sim, b, &t) => self.shun(s, &t),
                    other => {
                        task = other;
                        break;
                    }
                }
            }
            if log::log_enabled!(log::Level::Debug) {
                let name = |t: usize| sim.types.get(t).name.clone();
                let what = match &task {
                    Some(Task::Build(t, p) | Task::ReclaimBuild(_, t, p)) => format!("build {} at ({}, {})", name(*t), p.x.to_int(), p.z.to_int()),
                    Some(t) => format!("{t:?}"),
                    None => "nothing".into(),
                };
                log::debug!("{} min {}: P{me} {} on {} duty at ({}, {}) -> {what}", s.minutes, s.now, name(b.type_id), duty.name(), b.pos.x.to_int(), b.pos.z.to_int());
            }
            let Some(task) = task else {
                // (Nothing to do, and why: looked at again every couple of seconds rather than every
                // beat, but at once for a unit that was at work till now.)
                let known = matches!(status.get(&b.id), Some(DutyStatus::Idle(_)));
                if !known || (s.now / THINK_TICKS + b.id.slot as u64).is_multiple_of(HELPER_LOOK) {
                    let why = self.idle_why(duty, sim, s, b, &plan);
                    status.insert(b.id, DutyStatus::Idle(why));
                }
                continue;
            };
            status.insert(b.id, DutyStatus::Working);
            // (A helper whose duty has only the same help for it stays as it is.)
            let same = match &task {
                Task::Guard(f) => b.orders.iter().any(|o| *o == Order::Guard(*f)),
                Task::Repair(id) => b.orders.front() == Some(&Order::Repair(*id)),
                _ => false,
            };
            if look && same {
                continue;
            }
            // (Only the defence duty puts up towers: an extractor out in the field is not
            // given a guard.)
            self.give_task(sim, s, b, task, duty == Duty::Defence, &mut plan, out);
        }
    }

    /// Why a builder on a duty found nothing to do (for the player to be told): looked at only then.
    fn idle_why(&mut self, duty: Duty, sim: &Sim, s: &Survey, b: &Unit, plan: &Plan) -> IdleReason {
        // (Away from home with no way back: another island, walled in, or told lately that it could not get there.
        // A ship is not stranded by the base standing on land: it never could get there.)
        let ship = self.info[b.type_id].kind == Locomotion::Naval;
        if !ship && !b.pos.within_xz(self.base, fx(400)) && (self.homeless.iter().any(|&(id, until)| id == b.id && s.now < until) || !self.unit_reaches(sim, b, self.base)) {
            return IdleReason::Stranded;
        }
        match duty {
            Duty::Economy => {
                if !self.deposit_in_reach(sim, s, b, plan) {
                    IdleReason::NoDeposits
                } else if self.info[b.type_id].tech >= 2 && !(s.m_inc >= fx(ADVANCED_INCOME) || s.metal >= fx(ADVANCED_STOCK)) {
                    IdleReason::WaitingForIncome(ADVANCED_INCOME)
                } else {
                    IdleReason::EconomyDone
                }
            }
            Duty::Repair => IdleReason::NothingHurt,
            Duty::Reclaim => {
                let eco = &sim.players[self.player as usize].economy;
                if eco.metal * 100 >= eco.metal_capacity * 97 && eco.energy * 100 >= eco.energy_capacity * 97 {
                    IdleReason::StoresFull
                } else {
                    IdleReason::NothingToReclaim
                }
            }
            Duty::Defence => {
                // (What it would put up if the economy could pay for it within a minute.)
                let (_, all) = self.duty_options(sim, b);
                let want = (2 + s.minutes as usize / 3).min(10);
                let wanted = self.aa_task(sim, s, b, &all, plan).is_some()
                    || self.p.towers && self.front_tower_task(sim, s, b, &all, plan, want).is_some()
                    || self.outpost_task(sim, s, b, &all, plan).is_some();
                if wanted { IdleReason::SavingUp } else { IdleReason::DefencesDone }
            }
        }
    }

    /// How far from home an economy builder looks for free metal: the Commander's first extractors
    /// a little further out when there are no deposits round the start.
    fn economy_far(&self, s: &Survey, b: &Unit, plan: &Plan) -> i64 {
        if !self.info[b.type_id].commander {
            BUILDER_FAR
        } else if s.minutes < 6 && s.count[Role::Extractor as usize] + plan.pending[Role::Extractor as usize] < 3 {
            COMMANDER_FAR_EARLY
        } else {
            COMMANDER_FAR
        }
    }

    /// Whether a free metal deposit is within an economy builder's reach: not taken, not lately lost,
    /// not deep in enemy land or under fire, explored, and where a builder of its kind can go.
    fn deposit_in_reach(&self, sim: &Sim, s: &Survey, b: &Unit, plan: &Plan) -> bool {
        let far = self.economy_far(s, b, plan);
        let walker = !self.expander(b.type_id) && !self.info[b.type_id].commander && self.info[b.type_id].kind == Locomotion::Ground;
        self.deposits.iter().any(|d| {
            d.owner.is_none()
                && d.avoid_until <= s.now
                && !(walker && d.far)
                && dist(d.pos, self.base) <= far
                && !self.enemy_ground(d.pos)
                && self.threat.at(d.pos) == 0
                && sim.explored(self.player, d.pos)
        })
    }

    /// The next job for a constructor on a duty.
    fn duty_task(&mut self, duty: Duty, sim: &Sim, s: &Survey, b: &Unit, plan: &mut Plan) -> Option<Task> {
        match duty {
            Duty::Economy => self.economy_duty(sim, s, b, plan),
            Duty::Repair => self.repair_duty(sim, s, b, plan),
            Duty::Reclaim => self.reclaim_duty(sim, s, b, plan),
            Duty::Defence => self.defence_duty(sim, s, b, plan),
        }
    }

    /// What a builder can put up, for where it works: all of it (extractors and power may stand
    /// on land or in water, whichever suits the site) and the part for its own element.
    fn duty_options(&self, sim: &Sim, b: &Unit) -> (Vec<usize>, Vec<usize>) {
        let naval = self.info[b.type_id].kind == Locomotion::Naval;
        let all: Vec<usize> = sim.unit_type(b).build_options.iter().copied().filter(|&o| sim.is_allowed(o)).collect();
        let own = all.iter().copied().filter(|&o| self.info[o].land != naval).collect();
        (all, own)
    }

    /// How far a builder works from where it stands: the Commander's near its post.
    fn duty_reach(&self, b: &Unit) -> i64 {
        if self.info[b.type_id].commander { COMMANDER_REACH } else { BUILDER_REACH }
    }

    /// The economy, kept plain so that a player can tell what it will do: its own kinds of
    /// building left unfinished near by finished first; power when it is all but gone; free metal
    /// (an advanced constructor's costly mines once the income can pay for them); more power when
    /// it runs short; metal makers with power to spare; else home.
    fn economy_duty(&mut self, sim: &Sim, s: &Survey, b: &Unit, plan: &mut Plan) -> Option<Task> {
        let (all_opts, opts) = self.duty_options(sim, b);
        let t2 = self.info[b.type_id].tech >= 2;
        let Ledger { e_low, e_crit, metal_starved, power_spare, .. } = self.ledger(s);
        let pend = |plan: &Plan, r: Role| plan.pending[r as usize];
        // (Builders only get this far down without an extractor job, so any of them may build
        // power when it runs short.)
        let gen_limit = 1 + s.builders_t1 + t2 as usize;
        // (A frame of its own kind left behind, when it had to get away or was put on this duty
        // in the middle of one, say: finished rather than left to come apart while another goes
        // up beside it.)
        let own_kind = |r: Role| matches!(r, Role::Extractor | Role::Generator | Role::MetalMaker);
        if let Some(t) = self.finish_task_of(sim, s, b, self.duty_reach(b), &own_kind) {
            return Some(t);
        }
        // Power all but gone, or running short with none going up (a lone constructor would
        // otherwise take every free deposit first): the first thing.
        if (e_crit && pend(plan, Role::Generator) < 2 || e_low && pend(plan, Role::Generator) == 0) && let Some(t) = self.plant_task(sim, s, b, &all_opts, plan) {
            return Some(t);
        }
        // Free metal. (An advanced constructor's mines cost much: once the income can pay, two
        // at a time.)
        let mines = !t2 || (s.m_inc >= fx(ADVANCED_INCOME) || s.metal >= fx(ADVANCED_STOCK)) && pend(plan, Role::Extractor) < 2;
        let far = Some(self.economy_far(s, b, plan));
        if mines && let Some(t) = self.extractor_task(sim, s, b, &all_opts, far, plan) {
            return Some(t);
        }
        if e_low && pend(plan, Role::Generator) < gen_limit && let Some(t) = self.plant_task(sim, s, b, &all_opts, plan) {
            return Some(t);
        }
        // Power to spare: metal makers (sooner when metal is what holds everything up).
        if let Some(t) = self.maker_task(sim, s, b, &opts, plan, metal_starved && power_spare) {
            return Some(t);
        }
        // Nothing of its own to do. A constructor that came to the duty helping at a factory, or
        // guarding something (the player's order, another duty's help), leaves it: the economy
        // duty never helps at the factories. Home, or where it stands when that is home.
        if b.orders.iter().any(|o| matches!(o, Order::Guard(_))) {
            let away = !b.pos.within_xz(self.base, fx(400)) && !self.homeless.iter().any(|&(id, until)| id == b.id && s.now < until) && self.unit_reaches(sim, b, self.base);
            return Some(Task::Move(if away { self.base } else { b.pos }));
        }
        self.go_home(sim, s, b)
    }

    /// Mends the most damaged of ours near the base; else a hand with what is unfinished.
    fn repair_duty(&mut self, sim: &Sim, s: &Survey, b: &Unit, plan: &mut Plan) -> Option<Task> {
        if let Some(t) = self.mend_task(sim, s, b, plan) {
            return Some(t);
        }
        self.lend_a_hand(sim, s, b, plan, false)
    }

    /// Reclaims wrecks, then rocks and trees; else home.
    fn reclaim_duty(&mut self, sim: &Sim, s: &Survey, b: &Unit, plan: &mut Plan) -> Option<Task> {
        if !sim.unit_type(b).def.can_reclaim {
            return self.lend_a_hand(sim, s, b, plan, false);
        }
        let eco = &sim.players[self.player as usize].economy;
        // (Nothing is worth reclaiming of a kind the store is full of.)
        let metal_full = eco.metal * 100 >= eco.metal_capacity * 97;
        let energy_full = eco.energy * 100 >= eco.energy_capacity * 97;
        if !metal_full && let Some(t) = self.wreck_task(sim, b, plan) {
            return Some(t);
        }
        if let Some(t) = self.scrap_task(sim, b, plan, metal_full, energy_full) {
            return Some(t);
        }
        self.go_home(sim, s, b)
    }

    /// Towers where the base faces the enemy, anti-aircraft once planes have shown themselves, guard
    /// towers at outlying extractors; else a hand with what is unfinished.
    fn defence_duty(&mut self, sim: &Sim, s: &Survey, b: &Unit, plan: &mut Plan) -> Option<Task> {
        let (_, opts) = self.duty_options(sim, b);
        // Only what the economy can pay for within a minute (the stores and what comes in): a
        // tower it cannot afford would hold up everything else that is being built, and the
        // player gave no thought to power for it.
        let (metal, energy) = (s.metal + s.m_inc * 60, s.energy + (s.e_inc - s.e_exp).max(Fx::ZERO) * 60);
        let opts: Vec<usize> = opts.into_iter().filter(|&o| sim.types.get(o).cost_metal <= metal && sim.types.get(o).cost_energy <= energy).collect();
        if let Some(t) = self.aa_task(sim, s, b, &opts, plan) {
            return Some(t);
        }
        // A few more towers than a computer player puts up: it was asked to.
        let want = (2 + s.minutes as usize / 3).min(10);
        if self.p.towers && let Some(t) = self.front_tower_task(sim, s, b, &opts, plan, want) {
            return Some(t);
        }
        if let Some(t) = self.outpost_task(sim, s, b, &opts, plan) {
            return Some(t);
        }
        self.lend_a_hand(sim, s, b, plan, false)
    }

    /// What a builder does with no work of its duty's own: finishes our unfinished buildings near
    /// it, helps a factory that is busy (with metal enough to feed it), clears the base's trees and
    /// rocks (`clear`: when metal is short), else heads home.
    fn lend_a_hand(&mut self, sim: &Sim, s: &Survey, b: &Unit, plan: &mut Plan, clear: bool) -> Option<Task> {
        let reach = self.duty_reach(b);
        if let Some(t) = self.finish_task(sim, s, b, reach) {
            return Some(t);
        }
        if (self.metal_avg > Fx::from_ratio(3, 10) || self.info[b.type_id].commander) && let Some(f) = self.factory_to_help(sim, s, b, reach) {
            return Some(Task::Guard(f));
        }
        if clear && let Some(t) = self.clear_task(sim, b, plan) {
            return Some(t);
        }
        self.go_home(sim, s, b)
    }

    /// Home, for a constructor with nothing to do away from it: if it can get there, and not
    /// again at once when it lately could not (walled in, or on another island: it would be told
    /// every beat for ever).
    fn go_home(&self, sim: &Sim, s: &Survey, b: &Unit) -> Option<Task> {
        if !b.orders.is_empty() || b.pos.within_xz(self.base, fx(400)) {
            return None;
        }
        if self.homeless.iter().any(|&(id, until)| id == b.id && s.now < until) {
            return None;
        }
        self.unit_reaches(sim, b, self.base).then_some(Task::Move(self.base))
    }

    /// A constructor with armed enemies close by gets away from them, and holds back a few
    /// seconds after: whether it is being kept out of harm's way this think (the Commander is
    /// handed back to the player instead: see `DutyOfficer::think`).
    fn keep_away(&mut self, sim: &Sim, s: &Survey, b: &Unit, out: &mut Vec<Command>) -> bool {
        let air = self.info[b.type_id].kind == Locomotion::Air;
        let threats: Vec<FxVec3> = s
            .enemies
            .iter()
            .filter(|e| {
                let i = &self.info[e.type_id];
                e.is_complete() && e.pos.within_xz(b.pos, fx(DANGER_RANGE)) && if air { i.air_dps > 0 } else { i.ground_dps > 0 && i.kind != Locomotion::Air }
            })
            .map(|e| e.pos)
            .chain(s.contacts.iter().copied().filter(|c| !air && c.within_xz(b.pos, fx(DANGER_RANGE))))
            .collect();
        let worth: i64 = if air {
            s.enemies.iter().filter(|e| e.is_complete() && self.info[e.type_id].air_dps > 0 && e.pos.within_xz(b.pos, fx(DANGER_RANGE))).map(|e| self.info[e.type_id].worth).sum()
        } else {
            self.enemies_near(s, b.pos, DANGER_RANGE)
        };
        let held = self.retreating.iter().position(|&(id, _)| id == b.id);
        if worth < DANGER_WORTH || threats.is_empty() {
            return match held {
                Some(k) if s.now < self.retreating[k].1 => true,
                Some(k) => {
                    self.retreating.remove(k);
                    false
                }
                None => false,
            };
        }
        let until = s.now + HOLD_BACK;
        match held {
            Some(k) => self.retreating[k].1 = until,
            None => {
                self.retreating.push((b.id, until));
                // What it was at is left alone for a while (as the computer players do for a job
                // under fire), or it would come back to it as soon as the hold is over, and go
                // again when the enemy has not gone: a dance at the edge of their guns.
                match b.orders.front() {
                    Some(Order::Build { type_id, pos }) => {
                        self.failed.push((*type_id, *pos, s.now));
                        if self.info[*type_id].role == Role::Extractor {
                            self.avoid_deposit(*pos, s.now + 90 * SECOND);
                        }
                    }
                    Some(Order::Reclaim(f)) => self.unreachable_features.push((*f, s.now + MINUTE)),
                    Some(Order::Repair(id)) => self.unreachable_units.push((*id, s.now + MINUTE)),
                    _ => {}
                }
                if let Some(job) = b.build.filter(|job| job.target != b.id) {
                    self.unreachable_units.push((job.target, s.now + MINUTE));
                }
            }
        }
        let n = threats.len() as i64;
        let sum = threats.iter().fold(FxVec3::ZERO, |a, &p| a + p);
        let centre = FxVec3::new(sum.x / n, Fx::ZERO, sum.z / n);
        let flee = (b.pos - centre).xz().normalize();
        let away = Self::clamp_to_map(sim, b.pos + flee.scale(fx(450)));
        // Away from them, or home if that is further from them, wherever the unit can get to.
        let spot = [away, self.base]
            .into_iter()
            .filter(|&p| (air || self.unit_reaches(sim, b, p)) && dist(p, centre) > dist(b.pos, centre) + 100)
            .max_by_key(|&p| dist(p, centre));
        let heading_into_it = matches!(b.orders.front(), Some(Order::Move(m)) if dist(*m, centre) <= DANGER_RANGE);
        if let Some(p) = spot && (heading_into_it || !matches!(b.orders.front(), Some(Order::Move(_)))) {
            out.push(Command::Order { player: self.player, units: vec![b.id], order: Order::Move(p), queue: false });
        }
        true
    }

    /// The places a task takes a builder to (none for a hand at a factory).
    fn task_places(sim: &Sim, ai: &SkirmishAi, t: &Task) -> Vec<FxVec3> {
        match t {
            Task::Build(_, p) => vec![*p],
            Task::ReclaimBuild(f, _, p) => sim.features.get(*f).map(|f| f.pos).into_iter().chain([*p]).collect(),
            Task::Reclaim(f) => sim.features.get(*f).map(|f| f.pos).into_iter().collect(),
            Task::Repair(id) => sim.unit(*id).map(|u| u.pos).into_iter().collect(),
            Task::Upgrade(deposit, _, _) => vec![ai.deposits[*deposit].pos],
            Task::Move(p) => vec![*p],
            Task::Guard(_) => Vec::new(),
        }
    }

    /// Whether the way to a task's places keeps clear of the guns of the armed enemy buildings and
    /// units known (a straight line, looked at every few cells): a builder already within their
    /// reach is let go whichever way it likes (its way out is `keep_away`'s business).
    fn route_safe(&self, sim: &Sim, b: &Unit, t: &Task) -> bool {
        if self.threat.at(b.pos) > 0 {
            return true;
        }
        Self::task_places(sim, self, t).into_iter().all(|to| {
            let steps = (dist(b.pos, to) / 64).max(1);
            (1..=steps).all(|k| self.threat.at(b.pos + (to - b.pos).scale(Fx::from_ratio(k, steps))) == 0)
        })
    }

    /// Passes a task over for a while (its way is dangerous), so that another is looked for.
    fn shun(&mut self, s: &Survey, t: &Task) {
        let until = s.now + MINUTE;
        match t {
            Task::Build(ty, p) => {
                self.failed.push((*ty, *p, s.now));
                if self.info[*ty].role == Role::Extractor {
                    self.avoid_deposit(*p, until);
                }
            }
            Task::ReclaimBuild(f, ty, p) => {
                self.failed.push((*ty, *p, s.now));
                self.avoid_deposit(*p, until);
                self.unreachable_features.push((*f, until));
            }
            Task::Reclaim(f) => self.unreachable_features.push((*f, until)),
            Task::Repair(id) => self.unreachable_units.push((*id, until)),
            Task::Upgrade(deposit, _, _) => {
                let d = &mut self.deposits[*deposit];
                d.avoid_until = d.avoid_until.max(until);
            }
            Task::Guard(_) | Task::Move(_) => {}
        }
        log::debug!("{} min: P{} passes over {t:?}: its way is not safe", s.minutes, self.player);
    }

    /// The most damaged building or unit of ours near the base and the builder, to repair:
    /// the lowest health first (a Commander or a costly building before a cheap one at about the
    /// same), not one that moves (it would be chased), is under fire or is already being mended.
    fn mend_task(&self, sim: &Sim, s: &Survey, b: &Unit, plan: &Plan) -> Option<Task> {
        let reach = self.duty_reach(b);
        let mut cands: Vec<(i64, UnitId, FxVec3)> = Vec::new();
        for u in &s.own {
            if u.id == b.id || u.dead || !u.is_complete() || u.carried_by.is_some() || u.factory.is_some() || plan.claimed(b.id, Claim::Mend(u.id)) {
                continue;
            }
            let t = sim.unit_type(u);
            if u.health * 100 >= t.max_health * MEND_BELOW || self.unreachable_units.iter().any(|&(id, _)| id == u.id) {
                continue;
            }
            // (A unit that moves is not chased, nor one that is on its way somewhere; one that
            // stands is.)
            if t.locomotion != Locomotion::Static && !u.orders.is_empty() {
                continue;
            }
            let d = dist(b.pos, u.pos);
            if d > reach || !u.pos.within_xz(self.base, fx(SCRAP_RANGE + 200)) || self.threat.at(u.pos) > 0 {
                continue;
            }
            let missing = ((t.max_health - u.health) * 1000 / t.max_health.max(Fx::ONE)).to_int();
            let i = &self.info[u.type_id];
            let weight = if i.commander { 400 } else { (i.cost / 20).min(120) };
            cands.push((missing + weight - d / 8, u.id, u.pos));
        }
        // (The best first; ties to the lower unit.)
        cands.sort_by_key(|&(score, id, _)| (Reverse(score), id));
        cands.into_iter().take(6).find(|&(_, _, pos)| self.unit_reaches(sim, b, pos)).map(|(_, id, _)| Task::Repair(id))
    }

    /// Rocks and trees (and any other reclaimable feature that is no wreck's metal, no deposit and no
    /// vent) worth taking apart near the base: metal counting four times energy, the nearer the
    /// better, the ones in the base's way first; none that another builder is already at, that has
    /// been found out of reach, that lies where an enemy covers it or has not been explored.
    fn scrap_task(&self, sim: &Sim, b: &Unit, plan: &mut Plan, metal_full: bool, energy_full: bool) -> Option<Task> {
        let me = self.player;
        // (The Commander stays closer to home than the others, and no one goes off into the enemy's ground.)
        let range = if self.info[b.type_id].commander { COMMANDER_REACH } else { SCRAP_RANGE };
        let mut cands: Vec<(i64, usize)> = Vec::new();
        for (i, f) in sim.features.iter().enumerate() {
            if !f.alive || !f.pos.within_xz(self.base, fx(range)) {
                continue;
            }
            let d = &sim.data.features[f.def];
            if !d.reclaimable || d.is_metal_deposit() || d.geothermal || plan.claimed(b.id, Claim::Reclaim(i)) || self.enemy_ground(f.pos) {
                continue;
            }
            let left = Fx::ONE - f.reclaimed;
            let metal = if metal_full { 0 } else { (Fx::from_f64(d.metal) * left).to_int() };
            let energy = if energy_full { 0 } else { (Fx::from_f64(d.energy) * left).to_int() };
            let worth = metal * 4 + energy;
            if worth < 8 || !plan.free(f.pos, fx(8)) || self.unreachable_feature(i) || self.threat.at(f.pos) > 0 || !sim.explored(me, f.pos) {
                continue;
            }
            // (Those in the base's way are worth taking even where they pay little.)
            let in_the_way = d.blocking && f.pos.within_xz(self.base, fx(320));
            cands.push((worth * 6 - dist(b.pos, f.pos) + if in_the_way { 300 } else { 0 }, i));
        }
        cands.sort_by_key(|&(score, i)| (Reverse(score), i));
        let (_, f) = cands.into_iter().take(6).find(|&(_, i)| self.unit_reaches(sim, b, sim.features[i].pos))?;
        plan.reserved.push((sim.features[f].pos, fx(8)));
        Some(Task::Reclaim(f))
    }
}
