//! Playing with allies: hearing their signals (attack here, defend here, I need metal or
//! energy) and answering them, in deeds (`run_army` defends the spot, waves go for it,
//! resources are shared) and in a word for the game to put in its own language
//! ([`Answer`]); leaving allies the metal by their bases.

use super::*;

/// What an ally asked of us and when it lapses, and whether we have answered yet.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Request {
    pub at: FxVec3,
    pub until: u64,
    pub from: u8,
    pub answered: bool,
}

/// How an ally's signal was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Units are on their way to defend the spot `to` asked about.
    Defending { to: u8 },
    /// Nothing to send to defend it (no army at hand, or not enough to hold it).
    CannotDefend { to: u8 },
    /// Our army cannot get there (across water, up a cliff).
    CannotReach { to: u8 },
    /// Our next attack goes where `to` asked; and now it does.
    WillAttack { to: u8 },
    Attacking { to: u8 },
    /// Sent `to` so much metal and energy.
    Sent { to: u8, metal: i32, energy: i32 },
    /// Nothing of what `to` needs to spare.
    NothingToSpare { to: u8 },
}

/// The worth of a wave we call our allies to, and of raiders we call them against.
const CALL_ATTACK_WORTH: i64 = 1500;
const CALL_HELP_WORTH: i64 = 300;
/// How long an ally's call to defend a spot, and to attack one, stands.
const DEFEND_FOR: u64 = 60 * SECOND;
const ATTACK_FOR: u64 = 2 * MINUTE;
/// What we keep of our own stock (a share of our storage) before we give, and the most we
/// give at once.
const KEEP_SHARE: i64 = 3;
const GIFT_METAL: i64 = 600;
const GIFT_ENERGY: i64 = 3000;

impl SkirmishAi {
    /// Our allies' signals given since we last thought: what they ask stands for a while
    /// (`run_army` acts on it); resources asked for are sent at once, from what we can spare.
    pub(crate) fn hear_allies(&mut self, sim: &Sim, s: &Survey, out: &mut Vec<Command>) {
        let me = self.player;
        let from = self.signals_from;
        self.signals_from = sim.signals.iter().map(|g| g.tick + 1).max().unwrap_or(from).max(from);
        let fresh: Vec<ta_sim::Signal> = sim
            .signals
            .iter()
            .filter(|g| g.tick >= from && g.player != me && sim.allied(g.player, me) && sim.players.get(g.player as usize).is_some_and(|p| !p.defeated))
            .copied()
            .collect();
        let mut given = [0i64; 2];
        let mut incoming = vec![[0i64; 2]; sim.players.len()];
        for g in fresh {
            match g.kind {
                SignalKind::Defend => self.requests.defend = Some(Request { at: g.at, until: s.now + DEFEND_FOR, from: g.player, answered: false }),
                SignalKind::Attack => {
                    self.requests.attack = Some(Request { at: g.at, until: s.now + ATTACK_FOR, from: g.player, answered: false });
                    self.answers.push(Answer::WillAttack { to: g.player });
                }
                SignalKind::NeedMetal | SignalKind::NeedEnergy => {
                    let eco = &sim.players[me as usize].economy;
                    let metal = g.kind == SignalKind::NeedMetal;
                    let (have, cap, most, least) = if metal {
                        (eco.metal, eco.metal_capacity, GIFT_METAL, 50)
                    } else {
                        (eco.energy, eco.energy_capacity, GIFT_ENERGY, 250)
                    };
                    let k = usize::from(!metal);
                    let theirs = &sim.players[g.player as usize].economy;
                    let room = (if metal { theirs.metal_capacity - theirs.metal } else { theirs.energy_capacity - theirs.energy }.to_int() - incoming[g.player as usize][k]).max(0);
                    let spare = (have.to_int() - given[k] - cap.to_int() / KEEP_SHARE).clamp(0, most).min(room);
                    if spare >= least {
                        given[k] += spare;
                        incoming[g.player as usize][k] += spare;
                        let (m, e) = if metal { (spare, 0) } else { (0, spare) };
                        out.push(Command::Share { player: me, to: g.player, metal: fx(m), energy: fx(e) });
                        self.answers.push(Answer::Sent { to: g.player, metal: m as i32, energy: e as i32 });
                    } else {
                        self.answers.push(Answer::NothingToSpare { to: g.player });
                    }
                }
                SignalKind::Look | SignalKind::OnMyWay => {}
            }
        }
        let valid = |r: Request| s.now < r.until && sim.allied(r.from, me) && sim.players.get(r.from as usize).is_some_and(|p| !p.defeated);
        if self.requests.defend.is_some_and(|r| !valid(r)) {
            self.requests.defend = None;
        }
        if self.requests.attack.is_some_and(|r| !valid(r)) {
            self.requests.attack = None;
        }
    }

    /// Our army cannot reach where an ally asked us to attack: they hear so, and the ask goes.
    pub(crate) fn cannot_reach_asked(&mut self) {
        if let Some(r) = self.requests.attack.take() {
            self.answers.push(Answer::CannotReach { to: r.from });
        }
    }

    /// Where an ally asked us to attack, while that stands.
    pub(crate) fn asked_target(&self, now: u64) -> Option<FxVec3> {
        self.requests.attack.filter(|r| now < r.until).map(|r| r.at)
    }

    /// A wave goes in at `target`: if that is where an ally asked, they hear it now; else, a
    /// wave worth it, our allies are called to it (at most every two minutes).
    pub(crate) fn answer_attack(&mut self, sim: &Sim, now: u64, target: FxVec3, strength: i64, out: &mut Vec<Command>) {
        if let Some(r) = self.requests.attack.as_mut().filter(|r| r.at == target) {
            // (Where an ally asked: told once; never asked of them again, which renewed their
            // ask of us, and theirs ours, for the rest of the game.)
            if !r.answered {
                r.answered = true;
                self.answers.push(Answer::Attacking { to: r.from });
            }
            return;
        }
        if strength >= CALL_ATTACK_WORTH && self.has_allies(sim) && self.attack_called.is_none_or(|t| now >= t + 2 * MINUTE) {
            self.attack_called = Some(now);
            out.push(Command::Signal { player: self.player, kind: SignalKind::Attack, at: target });
        }
    }

    /// Raiders at our base, or among our buildings, too strong for what we have at home: our
    /// allies are called to defend it (at most once a minute).
    pub(crate) fn call_for_help(&mut self, sim: &Sim, now: u64, at: FxVec3, strength: i64, home: i64, out: &mut Vec<Command>) {
        let ours = at.within_xz(self.base, fx(1100)) || self.presence.around(at, 300) > 0;
        if strength * 5 > home * 4 && strength >= CALL_HELP_WORTH && ours && self.has_allies(sim) && self.help_called.is_none_or(|t| now >= t + MINUTE) {
            self.help_called = Some(now);
            out.push(Command::Signal { player: self.player, kind: SignalKind::Defend, at });
        }
    }

    /// Whether an ally is still in the game.
    pub(crate) fn has_allies(&self, sim: &Sim) -> bool {
        sim.players.iter().any(|p| p.index != self.player && !p.defeated && sim.allied(p.index, self.player))
    }

    /// Whether a spot is an ally's to take: by an ally's start, or among its buildings.
    pub(crate) fn allies_claim(&self, pos: FxVec3) -> bool {
        self.ally_bases.iter().any(|b| pos.within_xz(*b, fx(900))) || self.ally_presence.around(pos, 400) > 0
    }

    /// What we told our allies since the game last asked.
    pub fn take_answers(&mut self) -> Vec<Answer> {
        std::mem::take(&mut self.answers)
    }
}

/// An ally's standing requests.
#[derive(Clone, Default)]
pub(crate) struct Requests {
    /// Defend this spot.
    pub defend: Option<Request>,
    /// Our next waves go for this spot.
    pub attack: Option<Request>,
}
