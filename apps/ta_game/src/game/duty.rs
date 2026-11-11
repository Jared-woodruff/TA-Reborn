//! Duties: the player's constructors working by themselves.
//!
//! A constructor put on a duty (the economy, repairs, reclaiming, defence) is sent to work by the
//! player's own `ta_ai::DutyOfficer`, which thinks as the computer players do and gives the
//! orders a player would: they go out the way the player's do (in single player into the tick's
//! commands, in multiplayer through the session), so they are recorded and played back with the
//! battle, and nothing else needs to know of them. Any order the player gives a unit takes it off
//! duty again ([`Game::issue`] calls [`Game::release_from_duty`]); so does an attack on the
//! Commander, and armed enemies in sight close to it (the officer's `DutyEvent::Danger`). Saved
//! games keep the orders the officer gave in their commands and the duties themselves beside them
//! (`Record::duties`, from version 44: [`Game::duties_saved`]); a loaded battle puts the
//! constructors that are still there back on them once it has caught up ([`Game::restore_duties`]).

use ta_ai::{Duty, DutyEvent, DutyOfficer, DutyStatus, IdleReason};
use ta_sim::{Command, UnitId};
use ta_ui::i18n::{n, t, tf};

use super::Game;
use crate::hud_icons::Glyph;

/// What the selected constructors are doing on duty.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionDuty {
    /// How many of the selection can take a duty at all.
    pub eligible: usize,
    /// How many of those are on one.
    pub on_duty: usize,
    /// How many of those on one have nothing to do just now.
    pub idle: usize,
    /// The duty they are all on (none: not all on the same, or none on one).
    pub shared: Option<Duty>,
}

/// What the interface says of a constructor with nothing to do, or that cannot get on.
pub fn idle_words(why: IdleReason) -> String {
    let nothing = |why: &str| tf("Nothing to do: {why}", &[("why", &t(why))]);
    match why {
        IdleReason::NoDeposits => nothing(n("all metal deposits in reach are taken")),
        IdleReason::EconomyDone => nothing(n("the economy has all it needs")),
        IdleReason::NothingHurt => nothing(n("everything near home is in good repair")),
        IdleReason::NothingToReclaim => nothing(n("no wrecks or rocks to reclaim near home")),
        IdleReason::StoresFull => nothing(n("the stores are full")),
        IdleReason::DefencesDone => nothing(n("the defences are as wanted")),
        IdleReason::WaitingForIncome(income) => tf("Waiting for {n} metal a second before advanced work", &[("n", &income)]),
        IdleReason::SavingUp => t("Saving up: a tower must be paid for within a minute").to_string(),
        IdleReason::Stranded => t("Stranded: it cannot get home from here").to_string(),
    }
}

/// A duty's name, as the interface letters it.
pub fn label(duty: Duty) -> &'static str {
    match duty {
        Duty::Economy => n("ECONOMY"),
        Duty::Repair => n("REPAIR"),
        Duty::Reclaim => n("RECLAIM"),
        Duty::Defence => n("DEFENCE"),
    }
}

/// What a duty does, in a line.
pub fn blurb(duty: Duty) -> &'static str {
    match duty {
        Duty::Economy => n("Builds metal extractors on free metal, power plants when power runs short (wind where it pays), and metal makers when there is power to spare."),
        Duty::Repair => n("Mends the most damaged buildings and units near the base, and finishes unfinished buildings."),
        Duty::Reclaim => n("Reclaims wrecks, then rocks and trees near the base, for their metal and energy."),
        Duty::Defence => n("Builds towers where the base faces the enemy, and anti-aircraft when planes show up."),
    }
}

/// The units a command of the player's gives orders to (those that leave their duty for it): the
/// orders, the formations and the edits of a constructor's queue; not the standing orders (hold
/// fire, on and off), which leave a unit's work as it is.
fn commanded(c: &Command) -> &[UnitId] {
    match c {
        Command::Order { units, .. } | Command::Formation { units, .. } => units,
        Command::EditQueue { unit, .. } => std::slice::from_ref(unit),
        _ => &[],
    }
}

/// What a duty menu offers, in order (the command plate's menu and the radial menu's ring both stand
/// them so, that they never differ): each duty with its glyph, and last "under my command" (none).
pub const CHOICES: [(Option<Duty>, Glyph); 5] = [
    (Some(Duty::Economy), Glyph::AreaMex),
    (Some(Duty::Repair), Glyph::Repair),
    (Some(Duty::Reclaim), Glyph::Reclaim),
    (Some(Duty::Defence), Glyph::Guard),
    (None, Glyph::Cancel),
];

/// A choice's name: its duty's, or "under my command".
pub fn choice_label(choice: Option<Duty>) -> &'static str {
    choice.map_or(n("UNDER MY COMMAND"), label)
}

/// What a choice does, in a line.
pub fn choice_blurb(choice: Option<Duty>) -> &'static str {
    choice.map_or(n("Waits for your orders and does nothing by itself."), blurb)
}

/// The DUTY control as the interface words it, for the command plate's button and the radial menu's
/// choice: what it says, what it does (on the pointer) and whether any of the selection is on duty.
pub struct Control {
    pub label: String,
    pub tip: String,
    pub lit: bool,
}

impl Game {
    /// The DUTY control for the selection, when it holds constructors that can take a duty: it
    /// names the duty they are all on, else DUTIES (on different ones) or DUTY (none).
    pub fn duty_control(&self) -> Option<Control> {
        let duty = self.selection_duty();
        if duty.eligible == 0 {
            return None;
        }
        let key = self.keys.text("duty");
        let (label, tip) = match duty.shared {
            Some(d) => (t(label(d)).to_string(), format!("{}\n{}", tf("{duty} duty ({key})", &[("duty", &t(label(d))), ("key", &key)]), t(blurb(d)))),
            None if duty.on_duty > 0 => (t("DUTIES").to_string(), tf("Duties ({key}): the selected constructors are on different duties", &[("key", &key)])),
            None => (t("DUTY").to_string(), tf("Duty ({key}): send the constructors to work by themselves: the economy, repairs, reclaiming or defence. Any order you give takes them off duty.", &[("key", &key)])),
        };
        Some(Control { label, tip, lit: duty.on_duty > 0 })
    }

    /// Whether this battle lets the player put constructors on duty: one they play, not a replay,
    /// a seat for watching or a lesson.
    pub fn duties_allowed(&self) -> bool {
        !self.spectator && self.playback.is_none() && !self.tutorial
    }

    /// The duty a unit of ours is on.
    pub fn duty_of(&self, id: UnitId) -> Option<Duty> {
        self.duty.as_ref().and_then(|o| o.duty(id))
    }

    /// Whether a unit of ours is on a duty.
    pub fn on_duty(&self, id: UnitId) -> bool {
        self.duty_of(id).is_some()
    }

    /// The units on duty and their duties.
    pub fn units_on_duty(&self) -> impl Iterator<Item = (UnitId, Duty)> + '_ {
        self.duty.iter().flat_map(|o| o.units())
    }

    /// What a unit on duty is about, as the officer last looked (none before it has).
    pub fn duty_status(&self, id: UnitId) -> Option<DutyStatus> {
        self.duty.as_ref().and_then(|o| o.status(id))
    }

    /// What a unit on duty is doing that its orders do not say: that it is getting away from enemies, or
    /// has nothing to do and why. (A unit at work is told by its orders.)
    pub fn duty_line(&self, id: UnitId) -> Option<String> {
        match self.duty_status(id)? {
            DutyStatus::Working => None,
            DutyStatus::GettingAway => Some(t("Getting away from enemies").to_string()),
            DutyStatus::Idle(why) => Some(idle_words(why)),
        }
    }

    /// Whether a unit of ours is a constructor that can take a duty.
    fn can_take_duty(&self, id: UnitId) -> bool {
        self.sim.unit(id).is_some_and(|u| u.owner == self.local_player && !u.dead && u.is_complete() && DutyOfficer::can_serve(&self.sim, u))
    }

    /// The selected units that can take a duty (constructors that move: the Commander too).
    pub fn duty_candidates(&self) -> Vec<UnitId> {
        if !self.duties_allowed() {
            return Vec::new();
        }
        self.selection.iter().copied().filter(|&id| self.can_take_duty(id)).collect()
    }

    /// What the selection's constructors are on.
    pub fn selection_duty(&self) -> SelectionDuty {
        let candidates = self.duty_candidates();
        let duties: Vec<Option<Duty>> = candidates.iter().map(|&id| self.duty_of(id)).collect();
        let shared = duties.first().copied().flatten().filter(|d| duties.iter().all(|x| *x == Some(*d)));
        let idle = candidates.iter().filter(|&&id| matches!(self.duty_status(id), Some(DutyStatus::Idle(_)))).count();
        SelectionDuty { eligible: candidates.len(), on_duty: duties.iter().flatten().count(), idle, shared }
    }

    /// The constructors on duty, as the bytes a saved game keeps of them (`Record::duties`).
    pub fn duties_saved(&self) -> Vec<u8> {
        self.duty.as_ref().map(DutyOfficer::export).unwrap_or_default()
    }

    /// Puts the duties of a saved game (`Record::duties`) back on the constructors, as it goes live
    /// again: the orders the officer gave were in the record, so the units are where they were, and the
    /// officer takes up from there. What no longer makes sense is left under command.
    pub(super) fn restore_duties(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || !self.duties_allowed() {
            return;
        }
        let mut officer = DutyOfficer::new(self.local_player, &self.sim);
        if officer.import(&self.sim, bytes) > 0 {
            self.duty = Some(officer);
        }
    }

    /// Puts the selected constructors on a duty, or back under the player's command (`None`).
    pub fn set_duty(&mut self, duty: Option<Duty>) {
        let units = self.duty_candidates();
        self.set_duty_of(&units, duty);
    }

    /// The duty key: the selected constructors' next duty (none after the last, then the first).
    pub fn cycle_duty(&mut self) {
        let now = self.selection_duty();
        if now.eligible == 0 {
            return;
        }
        let next = Duty::next(now.shared);
        self.set_duty(next);
    }

    /// Puts these units (those of them that can) on a duty, or back under the player's command.
    pub fn set_duty_of(&mut self, units: &[UnitId], duty: Option<Duty>) {
        if !self.duties_allowed() {
            return;
        }
        let units: Vec<UnitId> = units.iter().copied().filter(|&id| self.can_take_duty(id)).collect();
        let Some(&first) = units.first() else { return };
        if duty.is_some() && self.duty.is_none() {
            self.duty = Some(DutyOfficer::new(self.local_player, &self.sim));
        }
        let Some(officer) = self.duty.as_mut() else { return };
        let changed = units.iter().filter(|&&id| officer.duty(id) != duty).count();
        for &id in &units {
            officer.set(id, duty);
        }
        if changed == 0 {
            return;
        }
        self.voices.push((first, ta_audio::SoundEvent::Ok));
        match duty {
            Some(d) => self.done(tf("On duty: {duty}", &[("duty", &t(label(d)))])),
            None => self.done(t("Back under your command")),
        }
    }

    /// An order of the player's takes the units it is for off duty (the officer's own never come
    /// through [`Game::issue`]: see [`Game::duty_orders`]).
    pub(super) fn release_from_duty(&mut self, c: &Command) {
        let Some(officer) = self.duty.as_mut() else { return };
        let freed: Vec<UnitId> = commanded(c).iter().copied().filter(|&id| officer.duty(id).is_some()).collect();
        if freed.is_empty() {
            return;
        }
        for id in freed {
            officer.set(id, None);
        }
        self.done(t("Back under your command"));
    }

    /// The officer's orders for the tick about to be played, given the way the player's are: in
    /// single player they join the tick's commands, in multiplayer they go through the session (and
    /// come back in the host's bundle). Only in a live battle of the player's own, and not while a
    /// client is catching up on the others' ticks.
    pub(super) fn duty_orders(&mut self, commands: &mut Vec<Command>) {
        if self.spectator || self.playback.is_some() || self.outcome.is_some() || self.net.as_ref().is_some_and(|net| net.backlog() > 3) {
            return;
        }
        let Some(officer) = self.duty.as_mut().filter(|o| !o.is_empty()) else { return };
        let orders = officer.think(&self.sim);
        // (What the officer did on its own, that the player is told: the Commander handed back.)
        for event in officer.take_events() {
            log::debug!("duty officer: {event:?}");
            match event {
                DutyEvent::Danger(_) => self.trouble(t("The Commander has enemies close by: back under your command")),
                // (A constructor with nothing to do for a while, and why: a click looks at it.)
                DutyEvent::Idle { unit, duty, why } => {
                    let text = tf("{duty} · {line}", &[("duty", &t(label(duty))), ("line", &idle_words(why))]);
                    let at = self.sim.unit(unit).map(|u| super::Link::Place(glam::Vec3::from_array(u.pos.to_f32())));
                    super::post_to(&mut self.messages, &mut self.event_history, self.sim.tick, text, 6.0, super::Tone::Note, at);
                }
            }
        }
        if orders.is_empty() {
            return;
        }
        match &mut self.net {
            Some(net) => orders.into_iter().for_each(|c| net.submit(c)),
            None => commands.extend(orders),
        }
    }

    /// A Commander on duty that has been hit lately by an enemy leaves its duty: losing it loses
    /// the battle, and keeping it safe is the player's to look to.
    pub(super) fn guard_commander_on_duty(&mut self) {
        let Some(officer) = self.duty.as_ref().filter(|o| !o.is_empty()) else { return };
        let (tick, me) = (self.sim.tick, self.local_player);
        let hit: Vec<UnitId> = officer
            .units()
            .map(|(id, _)| id)
            .filter(|&id| {
                self.sim.unit(id).is_some_and(|u| {
                    let enemy = u.last_attacker.is_none_or(|a| self.sim.unit(a).is_none_or(|a| self.sim.is_enemy(a.owner, me)));
                    self.sim.unit_type(u).def.is_commander && u.last_hit_tick > 0 && tick <= u.last_hit_tick + 30 && enemy
                })
            })
            .collect();
        if hit.is_empty() {
            return;
        }
        if let Some(officer) = self.duty.as_mut() {
            for id in hit {
                officer.set(id, None);
            }
        }
        self.trouble(t("The Commander is under attack: back under your command"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_sim::{FxVec3, Order};

    #[test]
    fn orders_take_units_off_duty_and_standing_orders_do_not() {
        let (a, b) = (UnitId { slot: 1, generation: 1 }, UnitId { slot: 2, generation: 1 });
        let go = Order::Move(FxVec3::from_ints(100, 0, 100));
        assert_eq!(commanded(&Command::Order { player: 0, units: vec![a, b], order: go.clone(), queue: true }), [a, b]);
        assert_eq!(commanded(&Command::Order { player: 0, units: vec![a], order: Order::Stop, queue: false }), [a], "Stop too");
        let shape = ta_sim::formation::Shape::Rows { facing: None };
        assert_eq!(commanded(&Command::Formation { player: 0, units: vec![b], order: go, queue: false, shape }), [b]);
        assert_eq!(commanded(&Command::EditQueue { player: 0, unit: a, index: 0, type_id: 3, pos: None, to_front: true }), [a]);
        assert!(commanded(&Command::SetStates { player: 0, units: vec![a], fire: Some(0), movement: None }).is_empty());
        assert!(commanded(&Command::SetActive { player: 0, units: vec![a], on: false }).is_empty());
        assert!(commanded(&Command::Resign { player: 0 }).is_empty());
    }

    #[test]
    fn every_reason_for_having_nothing_to_do_has_its_own_words() {
        let all = [
            IdleReason::NoDeposits,
            IdleReason::EconomyDone,
            IdleReason::WaitingForIncome(10),
            IdleReason::NothingHurt,
            IdleReason::NothingToReclaim,
            IdleReason::StoresFull,
            IdleReason::DefencesDone,
            IdleReason::SavingUp,
            IdleReason::Stranded,
        ];
        let words: Vec<String> = all.iter().map(|&why| idle_words(why)).collect();
        for (why, line) in all.iter().zip(&words) {
            assert!(!line.is_empty() && !line.contains('{'), "{why:?}: {line}");
        }
        for (k, line) in words.iter().enumerate() {
            assert!(!words[..k].contains(line), "two reasons say {line}");
        }
        assert!(words[2].contains("10"), "the income it waits for: {}", words[2]);
        // (A unit that is not at work is told so, whatever its orders say.)
        assert!(words[0].starts_with("Nothing to do: "), "{}", words[0]);
    }

    #[test]
    fn every_duty_has_its_words() {
        for d in Duty::ALL {
            assert!(!label(d).is_empty() && !blurb(d).is_empty());
            assert_eq!(label(d), label(d).to_uppercase(), "lettered in capitals: {}", label(d));
        }
    }
}
