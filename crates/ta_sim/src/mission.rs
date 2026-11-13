//! Mission objectives, evaluated deterministically inside the simulation so every player
//! of a (future) co-op game agrees on the outcome; placed units' scripts; and a scenario's
//! attack waves.

use std::collections::VecDeque;

use ta_data::mission::{Goal, MissionCommand, Toward};

use crate::defs::{Locomotion, UnitType, UnitTypes};
use crate::events::SimEvent;
use crate::fixed::{Fx, FxVec3, heading_of};
use crate::sim::Sim;
use crate::unit::{Order, Target, Unit, UnitId};
use crate::vision;

/// Ticks between objective checks.
const CHECK_TICKS: u64 = 15;

/// A line across the map, `x = at` or `z = at`, and the way a goal's units cross it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crossing {
    pub at: Fx,
    pub toward: Toward,
}

impl Crossing {
    pub fn new(line: &ta_data::mission::Line) -> Crossing {
        Crossing { at: Fx::from_f32(line.at), toward: line.toward }
    }

    /// Whether a point is past the line.
    pub fn passed(&self, p: FxVec3) -> bool {
        match self.toward {
            Toward::North => p.z < self.at,
            Toward::South => p.z > self.at,
            Toward::East => p.x > self.at,
            Toward::West => p.x < self.at,
        }
    }
}

/// A mission goal with unit names resolved to type ids.
#[derive(Clone, Debug, PartialEq)]
pub enum Objective {
    DestroyAllUnits,
    KillAllMobileUnits,
    KillEnemyCommander,
    KillUnitType { unit: usize, count: u32 },
    KillAllOfType(usize),
    /// Every unit of the type the enemy has, and every one a wave still to come brings (a
    /// scenario's Krogoths, walking in one after another).
    KillAllOfTypeToCome(usize),
    CaptureUnitType(usize),
    BuildUnitType(usize),
    MoveUnitToRadius { unit: Option<usize>, pos: FxVec3, radius: Fx },
    /// A unit of the type gets across the line.
    UnitTypePasses { unit: usize, line: Crossing },
    /// An enemy unit gets across the line.
    AnyUnitPasses(Crossing),
    AllUnitsKilled,
    CommanderKilled,
    AllUnitsKilledOfType(usize),
    UnitTypeKilled { unit: usize, count: u32 },
    /// Reached after this many ticks.
    Timer(u64),
    /// The human's side alone in the area (ground forces: aircraft passing over don't
    /// count) for `ticks` in all.
    HoldArea { pos: FxVec3, radius: Fx, ticks: u64 },
    /// The enemy alone in the area for `ticks` in all.
    EnemyHoldsArea { pos: FxVec3, radius: Fx, ticks: u64 },
    /// The human has made this much metal in all.
    MetalMade(Fx),
    /// An enemy has made this much metal in all.
    EnemyMetalMade(Fx),
    /// A unit the mission can't do without (`MissionCriticalUnit`) is destroyed, or no
    /// longer the human's.
    UnitLost(UnitId),
}

impl Objective {
    /// Resolves a data goal; goals naming unknown unit types are dropped.
    pub fn from_goal(goal: &Goal, types: &UnitTypes) -> Option<Objective> {
        let id = |name: &str| types.id(name);
        // Preserve fractional seconds and round the deadline up to a whole tick.
        let secs = |s: f32| {
            let raw = Fx::from_f32(s).max(Fx::ZERO).0 as u128;
            (raw * crate::cob::TICKS_PER_SECOND as u128).div_ceil(Fx::ONE.0 as u128) as u64
        };
        Some(match goal {
            Goal::DestroyAllUnits => Objective::DestroyAllUnits,
            Goal::KillAllMobileUnits => Objective::KillAllMobileUnits,
            Goal::KillEnemyCommander => Objective::KillEnemyCommander,
            Goal::KillUnitType { unit, count } => Objective::KillUnitType { unit: id(unit)?, count: (*count).max(1) },
            Goal::KillAllOfType(unit) => Objective::KillAllOfType(id(unit)?),
            Goal::CaptureUnitType(unit) => Objective::CaptureUnitType(id(unit)?),
            Goal::BuildUnitType(unit) => Objective::BuildUnitType(id(unit)?),
            Goal::MoveUnitToRadius { unit, x, z, radius } => Objective::MoveUnitToRadius {
                unit: match unit {
                    Some(u) => Some(id(u)?),
                    None => None,
                },
                pos: FxVec3::new(Fx::from_f32(*x), Fx::ZERO, Fx::from_f32(*z)),
                radius: Fx::from_f32(*radius),
            },
            Goal::UnitTypePasses { unit, line } => Objective::UnitTypePasses { unit: id(unit)?, line: Crossing::new(line) },
            Goal::VictoryTimer(s) | Goal::DeathTimer(s) => Objective::Timer(secs(*s)),
            Goal::AllUnitsKilled => Objective::AllUnitsKilled,
            Goal::CommanderKilled => Objective::CommanderKilled,
            Goal::AllUnitsKilledOfType(unit) => Objective::AllUnitsKilledOfType(id(unit)?),
            Goal::UnitTypeKilled { unit, count } => Objective::UnitTypeKilled { unit: id(unit)?, count: (*count).max(1) },
            Goal::AnyUnitPasses(line) => Objective::AnyUnitPasses(Crossing::new(line)),
        })
    }
}

/// The active mission: the human player, goals, and the result once decided.
#[derive(Clone, Debug, Default)]
pub struct MissionRules {
    pub human: u8,
    /// Met, they win the mission: any one of them, or all with `all_victory`.
    pub victory: Vec<Objective>,
    /// Any one met loses it.
    pub defeat: Vec<Objective>,
    /// Every victory goal must be met (TA's missions: "all the victory conditions must be
    /// met for a victory to occur", says its editor); otherwise any one wins (scenarios).
    pub all_victory: bool,
    /// `Some(true)` won, `Some(false)` lost.
    pub outcome: Option<bool>,
    /// Cumulative occupancy of each distinct objective area, in objective order.
    pub holds: Vec<AreaHold>,
    /// Attacks that arrive on schedule, soonest first (scenarios), and how many have come.
    pub waves: Vec<Wave>,
    pub waves_sent: u32,
    /// Units that came with the waves: with nothing left to do they go after the human's
    /// nearest forces (bombers after what their side sees: see `Sim::hunt`).
    pub hunters: Vec<Hunter>,
    /// The placed units' scripts still running, by unit.
    pub scripts: Vec<UnitScript>,
    /// Placed units nothing harms (`Immunity=1`), in order.
    pub immune: Vec<UnitId>,
    /// Placed units computer players act as though weren't there (`AiIgnore=1`), in order.
    pub ai_ignored: Vec<UnitId>,
    /// Placed units computer players go for first (`AiPriorityTarget=1`), in order.
    pub priority_targets: Vec<UnitId>,
    /// Unit types computer players build first (`BuildPriority`): (player, type).
    pub build_priority: Vec<(u8, usize)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AreaHold {
    pub pos: FxVec3,
    pub radius: Fx,
    pub held: u64,
    pub enemy_held: u64,
}

/// What ends the orders a script step puts in a unit's queue: a wait no script asks for (the
/// longest are hours; this is years). Its going means the unit was given orders of its own
/// (by the player, a computer player, a transport taking it aboard): its script ends there.
const END_MARK: u32 = u32::MAX;
/// Waits are kept well short of it.
const LONGEST_WAIT: u32 = END_MARK / 2;
/// Points on a factory's way out, as many as the player's.
const RALLY_MAX: usize = 16;

fn is_end_mark(o: &Order) -> bool {
    // (Waited down a little if it ever comes first in the queue.)
    matches!(o, Order::Wait(n) if *n > LONGEST_WAIT)
}

/// A placed unit's `InitialMission` script as it runs: the steps to come, and what the one
/// under way waits for.
#[derive(Clone, Debug)]
pub struct UnitScript {
    pub unit: UnitId,
    owner: u8,
    /// The tick it began.
    since: u64,
    steps: VecDeque<Step>,
    doing: Doing,
    /// The units a `wa` naming a group waits on (any one attacked).
    watch: Vec<UnitId>,
}

/// A step of a unit's script.
#[derive(Clone, Debug, PartialEq)]
enum Step {
    /// Orders the unit carries out itself, in turn: moves, patrols, waits, attacks, guarding,
    /// standing orders, a constructor's builds.
    Orders(Vec<Order>),
    /// A factory builds `count` units of a type.
    Produce { type_id: usize, count: u32 },
    /// A factory adds a point (a patrol point, a unit to guard) to the way its units leave by.
    Rally(Order),
    /// A transport sets one unit down there, or where it is.
    Unload(Option<FxVec3>),
    /// The unit blows up.
    SelfDestruct,
    /// Guard, or attack, the nearest of some units (a group) or of a unit type, as they
    /// stand when the step comes (`g TYPE`, `g group`, `a group`).
    Nearest { among: Among, attack: bool },
    /// Wait until any of these units (a group's) is attacked.
    WaitAttacked(Vec<UnitId>),
}

/// The units a script command names when it names more than one.
#[derive(Clone, Debug, PartialEq)]
enum Among {
    /// A group's (`InitialGroup`).
    Units(Vec<UnitId>),
    /// Every unit of a type.
    Type(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Doing {
    /// Riding in the transport it started in (its waits run all the same).
    Aboard,
    /// Nothing: the next step is due.
    Next,
    /// The step's orders, followed by the end mark.
    Orders,
    /// A factory building what the step asked for.
    Producing,
    /// A transport setting a unit down, with this many aboard before.
    Unloading { aboard: usize },
    /// Waiting (at the end mark) until one of the `watch` units is attacked.
    Watching,
}

/// A wave's unit on the hunt: where it was at the last look, and for how many looks it has
/// hardly moved though it had somewhere to go (wedged in a crowd).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hunter {
    pub id: UnitId,
    pub mark: FxVec3,
    pub still: u8,
}

/// A group of units that arrives on schedule: a scenario's attack.
#[derive(Clone, Debug, PartialEq)]
pub struct Wave {
    pub tick: u64,
    pub owner: u8,
    /// Unit types and how many of each.
    pub units: Vec<(usize, u16)>,
    /// Where it appears.
    pub from: FxVec3,
    /// Where it attack-moves to; `None`: the human's forces nearest to where it appears.
    pub target: Option<FxVec3>,
    /// Once there with nothing to do, its units go after the human's nearest forces
    /// (otherwise they stay: a hill's defenders).
    pub hunt: bool,
    /// On the human's side (reinforcements, an ally's): not one of the enemy's waves, nor
    /// counted with them; without a `target` its units wait where they appear.
    pub friendly: bool,
}

impl MissionRules {
    /// The enemy's next wave (reinforcements aside), if one is still to come.
    pub fn next_wave(&self) -> Option<&Wave> {
        self.waves.iter().find(|w| !w.friendly)
    }

    /// Progress for this goal's own area (the HUD and objective checks agree).
    pub fn hold_time(&self, objective: &Objective) -> u64 {
        let (pos, radius, enemy) = match *objective {
            Objective::HoldArea { pos, radius, .. } => (pos, radius, false),
            Objective::EnemyHoldsArea { pos, radius, .. } => (pos, radius, true),
            _ => return 0,
        };
        self.holds.iter().find(|a| a.pos == pos && a.radius == radius)
            .map_or(0, |a| if enemy { a.enemy_held } else { a.held })
    }
}

impl MissionRules {
    /// Scheduled work and objective progress are simulation state, before units appear
    /// or a goal ends the battle. Hash integer fields in their deterministic order.
    pub(crate) fn hash_state(&self, mix: &mut impl FnMut(i64), order: &impl Fn(&mut dyn FnMut(i64), &Order)) {
        fn pos(mix: &mut impl FnMut(i64), p: FxVec3) { mix(p.x.0); mix(p.y.0); mix(p.z.0); }
        fn id(mix: &mut impl FnMut(i64), id: UnitId) { mix(id.slot as i64); mix(id.generation as i64); }
        fn ids(mix: &mut impl FnMut(i64), units: &[UnitId]) {
            mix(units.len() as i64);
            for &unit in units { id(mix, unit); }
        }
        fn crossing(mix: &mut impl FnMut(i64), line: Crossing) {
            mix(line.at.0);
            mix(match line.toward { Toward::North => 0, Toward::South => 1, Toward::East => 2, Toward::West => 3 });
        }
        mix(self.human as i64); mix(self.all_victory as i64);
        mix(match self.outcome { None => 0, Some(false) => 1, Some(true) => 2 });
        for goals in [&self.victory, &self.defeat] {
            mix(goals.len() as i64);
            for goal in goals {
                match *goal {
                    Objective::DestroyAllUnits => mix(0),
                    Objective::KillAllMobileUnits => mix(1),
                    Objective::KillEnemyCommander => mix(2),
                    Objective::KillUnitType { unit, count } => { mix(3); mix(unit as i64); mix(count as i64); }
                    Objective::KillAllOfType(unit) => { mix(4); mix(unit as i64); }
                    Objective::KillAllOfTypeToCome(unit) => { mix(5); mix(unit as i64); }
                    Objective::CaptureUnitType(unit) => { mix(6); mix(unit as i64); }
                    Objective::BuildUnitType(unit) => { mix(7); mix(unit as i64); }
                    Objective::MoveUnitToRadius { unit, pos: p, radius } => { mix(8); mix(unit.map_or(-1, |t| t as i64)); pos(mix, p); mix(radius.0); }
                    Objective::UnitTypePasses { unit, line } => { mix(9); mix(unit as i64); crossing(mix, line); }
                    Objective::AnyUnitPasses(line) => { mix(10); crossing(mix, line); }
                    Objective::AllUnitsKilled => mix(11),
                    Objective::CommanderKilled => mix(12),
                    Objective::AllUnitsKilledOfType(unit) => { mix(13); mix(unit as i64); }
                    Objective::UnitTypeKilled { unit, count } => { mix(14); mix(unit as i64); mix(count as i64); }
                    Objective::Timer(ticks) => { mix(15); mix(ticks as i64); }
                    Objective::HoldArea { pos: p, radius, ticks } => { mix(16); pos(mix, p); mix(radius.0); mix(ticks as i64); }
                    Objective::EnemyHoldsArea { pos: p, radius, ticks } => { mix(17); pos(mix, p); mix(radius.0); mix(ticks as i64); }
                    Objective::MetalMade(metal) => { mix(18); mix(metal.0); }
                    Objective::EnemyMetalMade(metal) => { mix(19); mix(metal.0); }
                    Objective::UnitLost(unit) => { mix(20); id(mix, unit); }
                }
            }
        }
        mix(self.holds.len() as i64);
        for area in &self.holds { pos(mix, area.pos); mix(area.radius.0); mix(area.held as i64); mix(area.enemy_held as i64); }
        mix(self.waves_sent as i64); mix(self.waves.len() as i64);
        for wave in &self.waves {
            mix(wave.tick as i64); mix(wave.owner as i64); pos(mix, wave.from);
            mix(wave.target.is_some() as i64);
            if let Some(p) = wave.target { pos(mix, p); }
            mix(wave.hunt as i64); mix(wave.friendly as i64); mix(wave.units.len() as i64);
            for &(unit, count) in &wave.units { mix(unit as i64); mix(count as i64); }
        }
        mix(self.hunters.len() as i64);
        for hunter in &self.hunters { id(mix, hunter.id); pos(mix, hunter.mark); mix(hunter.still as i64); }
        mix(self.scripts.len() as i64);
        for script in &self.scripts {
            id(mix, script.unit); mix(script.owner as i64); mix(script.since as i64);
            match script.doing {
                Doing::Aboard => mix(0), Doing::Next => mix(1), Doing::Orders => mix(2), Doing::Producing => mix(3),
                Doing::Unloading { aboard } => { mix(4); mix(aboard as i64); } Doing::Watching => mix(5),
            }
            ids(mix, &script.watch); mix(script.steps.len() as i64);
            for step in &script.steps {
                match step {
                    Step::Orders(orders) => { mix(0); mix(orders.len() as i64); for o in orders { order(mix, o); } }
                    Step::Produce { type_id, count } => { mix(1); mix(*type_id as i64); mix(*count as i64); }
                    Step::Rally(o) => { mix(2); order(mix, o); }
                    Step::Unload(p) => { mix(3); mix(p.is_some() as i64); if let Some(p) = p { pos(mix, *p); } }
                    Step::SelfDestruct => mix(4),
                    Step::Nearest { among, attack } => {
                        mix(5); mix(*attack as i64);
                        match among { Among::Units(units) => { mix(0); ids(mix, units); } Among::Type(t) => { mix(1); mix(*t as i64); } }
                    }
                    Step::WaitAttacked(units) => { mix(6); ids(mix, units); }
                }
            }
        }
        for units in [&self.immune, &self.ai_ignored, &self.priority_targets] { ids(mix, units); }
        mix(self.build_priority.len() as i64);
        for &(owner, unit) in &self.build_priority { mix(owner as i64); mix(unit as i64); }
    }

    pub fn new(human: u8, goals: &[Goal], types: &UnitTypes) -> MissionRules {
        let mut rules = MissionRules { human, ..Default::default() };
        for g in goals {
            let Some(o) = Objective::from_goal(g, types) else { continue };
            if g.is_victory() {
                rules.victory.push(o);
            } else {
                rules.defeat.push(o);
            }
        }
        rules
    }
}

impl Sim {
    /// Moves the placed units' scripts on (every tick, the mission decided or not); evaluates
    /// the mission's objectives (every half second) and records the outcome; sends the waves
    /// due.
    pub(crate) fn check_mission(&mut self) {
        self.run_unit_scripts();
        let Some(rules) = &self.mission else { return };
        if rules.outcome.is_some() {
            return;
        }
        self.send_waves();
        if !self.tick.is_multiple_of(CHECK_TICKS) {
            return;
        }
        self.count_holding();
        if self.tick.is_multiple_of(CHECK_TICKS * 4) {
            self.hunt();
        }
        let Some(rules) = &self.mission else { return };
        let human = rules.human;
        let lost = rules.defeat.iter().any(|o| self.objective_met(o, human));
        let won = !lost
            && if rules.all_victory {
                !rules.victory.is_empty() && rules.victory.iter().all(|o| self.objective_met(o, human))
            } else {
                rules.victory.iter().any(|o| self.objective_met(o, human))
            };
        if lost || won {
            if let Some(m) = &mut self.mission {
                m.outcome = Some(won);
            }
            self.events.push(SimEvent::MissionEnded { victory: won });
        }
    }

    fn objective_met(&self, o: &Objective, human: u8) -> bool {
        let alive = || self.units().filter(|u| !u.dead);
        // (An ally's units are on the human's side.)
        let enemy = |owner: u8| !self.allied(owner, human);
        let deaths = |owner_filter: &dyn Fn(u8) -> bool, unit: usize| -> u64 {
            self.deaths_by.iter().filter(|((o, t), _)| *t == unit && owner_filter(*o)).map(|(_, n)| *n).sum()
        };
        // (Waves still to come are enemies too; reinforcements are not.)
        let no_more_waves = self.mission.as_ref().is_none_or(|m| m.next_wave().is_none());
        match *o {
            Objective::DestroyAllUnits => no_more_waves && !alive().any(|u| enemy(u.owner)),
            Objective::KillAllMobileUnits => no_more_waves && !alive().any(|u| enemy(u.owner) && self.unit_type(u).locomotion != Locomotion::Static),
            Objective::KillEnemyCommander => {
                // An enemy without a Commander has none to kill: met, where every goal must be
                // (the Core Contingency's last mission sets this goal against an Arm with only
                // decoys, and could never be won); where any one wins (scenarios), only once one
                // has fallen.
                let had = self.players.iter().any(|p| enemy(p.index) && p.had_commander);
                let all = self.mission.as_ref().is_some_and(|m| m.all_victory);
                let coming = self.mission.as_ref().is_some_and(|m| m.waves.iter().any(|w| !w.friendly
                    && enemy(w.owner) && w.units.iter().any(|&(t, n)| n > 0 && self.types.types.get(t).is_some_and(|t| t.def.is_commander))));
                (had || all) && !coming && !alive().any(|u| enemy(u.owner) && self.unit_type(u).def.is_commander)
            }
            Objective::KillUnitType { unit, count } => deaths(&|o| enemy(o), unit) >= count as u64,
            Objective::KillAllOfType(unit) => deaths(&|o| enemy(o), unit) > 0 && !alive().any(|u| enemy(u.owner) && u.type_id == unit),
            Objective::KillAllOfTypeToCome(unit) => {
                let to_come = self.mission.as_ref().is_some_and(|m| m.waves.iter().any(|w| !w.friendly && w.units.iter().any(|&(t, _)| t == unit)));
                !to_come && deaths(&|o| enemy(o), unit) > 0 && !alive().any(|u| enemy(u.owner) && u.type_id == unit)
            }
            Objective::CaptureUnitType(unit) => self.captured.contains(&(human, unit)),
            Objective::BuildUnitType(unit) => self.built.contains(&(human, unit)),
            Objective::MoveUnitToRadius { unit, pos, radius } => {
                // A finished unit of the type named, a building raised there too (Guardian
                // Hill's "build a Punisher on top of Guardian Hill", Waiting's extractor); for
                // any unit, one that went there (buildings at the spot, AC01's Gate, don't count).
                alive().any(|u| {
                    u.owner == human
                        && u.is_complete()
                        && unit.map_or(self.unit_type(u).locomotion != Locomotion::Static, |t| u.type_id == t)
                        && u.pos.within_xz(pos, radius)
                })
            }
            Objective::UnitTypePasses { unit, line } => alive().any(|u| u.owner == human && u.type_id == unit && line.passed(u.pos)),
            Objective::AnyUnitPasses(line) => alive().any(|u| enemy(u.owner) && line.passed(u.pos)),
            Objective::AllUnitsKilled => !alive().any(|u| u.owner == human),
            Objective::CommanderKilled => {
                let had = self.players.get(human as usize).is_some_and(|p| p.had_commander);
                had && !alive().any(|u| u.owner == human && self.unit_type(u).def.is_commander)
            }
            Objective::AllUnitsKilledOfType(unit) => deaths(&|_| true, unit) > 0 && !alive().any(|u| u.type_id == unit),
            Objective::UnitTypeKilled { unit, count } => deaths(&|o| o == human, unit) >= count as u64,
            Objective::Timer(ticks) => self.tick >= ticks,
            Objective::HoldArea { ticks, .. } | Objective::EnemyHoldsArea { ticks, .. } => self.mission.as_ref().is_some_and(|m| m.hold_time(o) >= ticks),
            Objective::MetalMade(metal) => self.players.get(human as usize).is_some_and(|p| p.stats.metal_produced >= metal),
            Objective::EnemyMetalMade(metal) => self.players.iter().any(|p| self.is_enemy(p.index, human) && p.stats.metal_produced >= metal),
            Objective::UnitLost(id) => self.unit(id).is_none_or(|u| u.dead || u.owner != human),
        }
    }

    /// Sends the waves whose time has come: their units appear (facing where they go, none
    /// inside another, aircraft already flying in) and attack-move there.
    fn send_waves(&mut self) {
        loop {
            let Some(rules) = &mut self.mission else { return };
            let Some(wave) = rules.waves.first().filter(|w| w.tick <= self.tick).cloned() else { return };
            rules.waves.remove(0);
            if !wave.friendly {
                rules.waves_sent += 1;
            }
            let human = rules.human;
            // (Reinforcements without a destination wait where they come in.)
            let target = wave.target.or_else(|| if wave.friendly { None } else { self.nearest_forces(human, wave.from) }).unwrap_or(wave.from);
            let heading = heading_of(target - wave.from);
            let mut arrived = Vec::new();
            for &(type_id, count) in &wave.units {
                if type_id < self.types.types.len() {
                    arrived.extend(self.place_units_bound(type_id, wave.owner, wave.from, count, heading, Some(target)));
                }
            }
            let types = self.types.clone();
            for &id in &arrived {
                // (Aircraft come over the map's edge in flight, not parked on whatever ground
                // or sea is there.)
                self.put_in_flight(id, None);
                if let Some(u) = self.unit_mut(id)
                    && types.get(u.type_id).locomotion != Locomotion::Static
                {
                    u.orders.push_back(Order::Fight(target));
                }
            }
            self.events.push(SimEvent::WaveArrived { owner: wave.owner, units: arrived.len() as u32, pos: wave.from });
            if let Some(rules) = &mut self.mission
                && wave.hunt
                && !wave.friendly
            {
                rules.hunters.extend(arrived.iter().map(|&id| Hunter { id, mark: FxVec3::ZERO, still: 0 }));
            }
        }
    }

    /// Where the human's side (its allies too) has a unit nearest to `from`.
    fn nearest_forces(&self, human: u8, from: FxVec3) -> Option<FxVec3> {
        self.units().filter(|u| !u.dead && u.carried_by.is_none() && self.allied(u.owner, human)).map(|u| u.pos).min_by_key(|p| (*p - from).xz().length_sq_raw())
    }

    /// The waves' units with nothing left to do attack-move to the human's nearest forces. One
    /// whose every weapon fires on order only (a bomber: TA's bombs are `commandfire`, never
    /// aimed of their own accord, and an attack-move flew it over the human's base without a
    /// bomb dropped) is sent at the nearest of the human's side its own side sees, in sight or
    /// on radar, that its weapons can hit: it keeps on that one while it stands and is seen, then
    /// takes the next. Seeing none, it attack-moves on as the others do.
    fn hunt(&mut self) {
        let Some(rules) = &mut self.mission else { return };
        if rules.hunters.is_empty() {
            return;
        }
        let human = rules.human;
        let mut hunters = std::mem::take(&mut rules.hunters);
        hunters.retain(|h| self.unit(h.id).is_some_and(|u| !u.dead && self.is_enemy(u.owner, human)));
        let prey: Vec<FxVec3> = self.units().filter(|u| !u.dead && u.carried_by.is_none() && self.allied(u.owner, human)).map(|u| u.pos).collect();
        let types = self.types.clone();
        // Hardly moving for three looks in a row (six seconds) with somewhere to go: wedged.
        const STILL_LOOKS: u8 = 3;
        let still_px = Fx::from_int(24);
        for h in &mut hunters {
            let Some(u) = self.unit(h.id).filter(|u| u.is_complete() && u.carried_by.is_none() && types.get(u.type_id).locomotion != Locomotion::Static) else { continue };
            let pos = u.pos;
            if self.fires_on_order_only(types.get(u.type_id)) {
                let on = match u.orders.front() {
                    Some(&Order::Attack(Target::Unit(t))) => Some(t),
                    _ => None,
                };
                if on.is_some_and(|t| self.sees_to_hit(u, t)) {
                    (h.still, h.mark) = (0, pos);
                    continue;
                }
                let next = self.units().filter(|o| self.allied(o.owner, human) && self.sees_to_hit(u, o.id)).min_by_key(|o| ((o.pos - pos).xz().length_sq_raw(), o.id.slot)).map(|o| (o.id, o.pos));
                if let Some(u) = self.unit_mut(h.id) {
                    if let Some((target, at)) = next {
                        u.orders.clear();
                        u.path.clear();
                        u.path_goal = None;
                        u.orders.push_back(Order::Attack(Target::Unit(target)));
                        // (Then on to where it stood, not idle while the next is chosen: an idle
                        // aircraft lands.)
                        u.orders.push_back(Order::Fight(at));
                        (h.still, h.mark) = (0, pos);
                        continue;
                    }
                    // Its target fallen or lost from sight, and nothing else seen: on as the
                    // others go.
                    if on.is_some() {
                        u.orders.pop_front();
                    }
                }
            }
            let Some(u) = self.unit(h.id) else { continue };
            let busy = !u.orders.is_empty();
            h.still = if busy && pos.within_xz(h.mark, still_px) { h.still.saturating_add(1) } else { 0 };
            h.mark = pos;
            if busy && h.still < STILL_LOOKS {
                continue;
            }
            let Some(&target) = prey.iter().min_by_key(|p| (**p - pos).xz().length_sq_raw()) else { break };
            // (Wedged units each take a point of their own near it: the same goal would give
            // them the same way, and the same jam.)
            let target = if busy {
                h.still = 0;
                let spread = Fx::from_int(96);
                let angle = (h.id.slot as i32).wrapping_mul(9_973) & 0xFFFF;
                target + FxVec3::new(crate::fixed::cos(angle) * spread, Fx::ZERO, crate::fixed::sin(angle) * spread)
            } else {
                target
            };
            if let Some(u) = self.unit_mut(h.id) {
                u.orders.clear();
                u.path.clear();
                u.path_goal = None;
                u.orders.push_back(Order::Fight(target));
            }
        }
        if let Some(rules) = &mut self.mission {
            rules.hunters = hunters;
        }
    }

    /// Whether every weapon of a unit type fires on order only (`commandfire`: TA's bombs, the
    /// D-gun), and it has one: left to an attack-move it never fires.
    fn fires_on_order_only(&self, t: &UnitType) -> bool {
        let mut weapons = t.weapons.iter().flatten().map(|&w| self.weapon_types.get(w)).peekable();
        weapons.peek().is_some() && weapons.all(|w| w.command_fire && !w.def.interceptor)
    }

    /// Whether `hunter`'s side sees `target` (in sight or on radar, never through the fog) and one
    /// of its weapons can hit it (bombs never an aircraft in flight, nor what is under the
    /// water), as a target it would choose (not one of its bad targets, nor what a mission keeps
    /// from the computer players' fire).
    fn sees_to_hit(&self, hunter: &Unit, target: UnitId) -> bool {
        let Some(o) = self.unit(target) else { return false };
        let t = self.types.get(hunter.type_id);
        !o.dead
            && o.carried_by.is_none()
            && self.is_enemy(hunter.owner, o.owner)
            && !self.no_auto_target.contains(&o.type_id)
            && !self.ai_ignores(o.id)
            && self.seen_by(o, hunter.owner) != vision::Seen::Hidden
            && (0..3).any(|s| t.weapons[s].is_some_and(|w| self.engages(hunter.type_id, s, self.weapon_types.get(w), o)))
    }

    /// Who holds the objective area: the human's side or the enemy, alone (finished ground
    /// units in it).
    fn count_holding(&mut self) {
        let Some(rules) = &self.mission else { return };
        let human = rules.human;
        let mut holds = Vec::new();
        for objective in rules.victory.iter().chain(&rules.defeat) {
            let (pos, radius) = match *objective {
                Objective::HoldArea { pos, radius, .. } | Objective::EnemyHoldsArea { pos, radius, .. } => (pos, radius),
                _ => continue,
            };
            if holds.iter().any(|a: &AreaHold| a.pos == pos && a.radius == radius) { continue; }
            let mut area = rules.holds.iter().find(|a| a.pos == pos && a.radius == radius).cloned()
                .unwrap_or(AreaHold { pos, radius, held: 0, enemy_held: 0 });
            let (mut ours, mut theirs) = (false, false);
            for u in self.units() {
                if u.dead || !u.is_complete() || u.carried_by.is_some() || self.unit_type(u).locomotion == Locomotion::Air || !u.pos.within_xz(pos, radius) { continue; }
                if self.allied(u.owner, human) { ours = true; } else { theirs = true; }
            }
            if ours && !theirs {
                area.held = area.held.saturating_add(CHECK_TICKS);
            } else if theirs && !ours {
                area.enemy_held = area.enemy_held.saturating_add(CHECK_TICKS);
            }
            holds.push(area);
        }
        if let Some(rules) = &mut self.mission {
            rules.holds = holds;
        }
    }

    /// Whether a unit type may be built (mission unit restrictions).
    pub fn is_allowed(&self, type_id: usize) -> bool {
        type_id < self.types.types.len() && self.allowed.as_ref().is_none_or(|a| a.get(type_id).copied().unwrap_or(false))
    }

    /// TA's "mapped" option: the whole map starts explored for this player (terrain shown,
    /// units still hidden outside sight).
    pub fn reveal_map_for(&mut self, player: u8) {
        let slot = self.vision_slot(player);
        self.vision.set_all(slot, vision::EXPLORED);
    }

    // ---- Placed units' scripts ----------------------------------------------------------

    /// Whether a unit still follows its mission script (computer players leave it to it).
    pub fn runs_mission_script(&self, id: UnitId) -> bool {
        self.mission.as_ref().is_some_and(|m| m.scripts.binary_search_by_key(&id, |s| s.unit).is_ok())
    }

    /// Whether the mission makes a unit immune to harm (but its own blast).
    pub fn is_immune(&self, id: UnitId) -> bool {
        self.mission.as_ref().is_some_and(|m| m.immune.binary_search(&id).is_ok())
    }

    /// Whether computer players act as though a unit weren't there (`AiIgnore=1`): their
    /// units never pick it as a target, and they never plan against it.
    pub fn ai_ignores(&self, id: UnitId) -> bool {
        self.mission.as_ref().is_some_and(|m| m.ai_ignored.binary_search(&id).is_ok())
    }

    /// Whether computer players' units go for a unit first (`AiPriorityTarget=1`).
    pub fn is_priority_target(&self, id: UnitId) -> bool {
        self.mission.as_ref().is_some_and(|m| m.priority_targets.binary_search(&id).is_ok())
    }

    /// The units computer players go for first (`AiPriorityTarget=1`), in order (gone ones
    /// too).
    pub fn priority_targets(&self) -> &[UnitId] {
        self.mission.as_ref().map_or(&[], |m| &m.priority_targets)
    }

    /// The unit types a computer player builds first, when it can (`BuildPriority`).
    pub fn build_priority(&self, player: u8) -> impl Iterator<Item = usize> + '_ {
        self.mission.iter().flat_map(|m| &m.build_priority).filter(move |p| p.0 == player).map(|p| p.1)
    }

    /// Takes every placed unit's script as far as it goes this tick.
    fn run_unit_scripts(&mut self) {
        let Some(rules) = &mut self.mission else { return };
        if rules.scripts.is_empty() {
            return;
        }
        let mut scripts = std::mem::take(&mut rules.scripts);
        scripts.retain_mut(|s| self.run_unit_script(s));
        if let Some(rules) = &mut self.mission {
            rules.scripts = scripts;
        }
    }

    /// Takes a unit's script as far as it goes this tick; false once it is over: done, or
    /// its unit gone, gone over to another side, or given orders of someone else's.
    fn run_unit_script(&mut self, s: &mut UnitScript) -> bool {
        loop {
            let Some(u) = self.unit(s.unit).filter(|u| !u.dead && u.owner == s.owner) else { return false };
            match s.doing {
                Doing::Aboard => {
                    if u.carried_by.is_some() {
                        return true;
                    }
                    // Set down: the waits it starts with have been running all along (TA's
                    // missions time a passenger's wait to end soon after its transport's flight).
                    skip_waited(&mut s.steps, self.tick - s.since);
                    s.doing = Doing::Next;
                }
                Doing::Orders => match u.orders.iter().position(is_end_mark) {
                    None => return false,
                    Some(0) => {
                        if let Some(u) = self.unit_mut(s.unit) {
                            u.orders.pop_front();
                        }
                        s.doing = Doing::Next;
                    }
                    Some(_) => return true,
                },
                Doing::Producing => {
                    if !u.factory_queue.is_empty() || u.build.is_some() {
                        return true;
                    }
                    s.doing = Doing::Next;
                }
                Doing::Unloading { aboard } => {
                    let Some(mark) = u.orders.iter().position(is_end_mark) else { return false };
                    if mark == 1 && u.cargo.len() < aboard && matches!(u.orders.front(), Some(Order::Unload(_))) {
                        // One unit is down: the rest stay aboard for the script's next `u`.
                        // (A ship's boom sets its cargo down one at a time, and would go on.)
                        if let Some(u) = self.unit_mut(s.unit) {
                            u.orders.pop_front();
                            u.transport_job = None;
                            u.hover_alt = None;
                        }
                        continue;
                    }
                    if mark > 0 {
                        return true;
                    }
                    if let Some(u) = self.unit_mut(s.unit) {
                        u.orders.pop_front();
                    }
                    s.doing = Doing::Next;
                }
                Doing::Watching => match u.orders.iter().position(is_end_mark) {
                    None => return false,
                    Some(0) => {
                        // (A unit gone counts as attacked, as with `wa` naming one.)
                        if !s.watch.iter().any(|&w| self.unit(w).is_none_or(|w| w.dead || w.last_attacker.is_some())) {
                            return true;
                        }
                        if let Some(u) = self.unit_mut(s.unit) {
                            u.orders.pop_front();
                        }
                        s.watch.clear();
                        s.doing = Doing::Next;
                    }
                    Some(_) => return true,
                },
                Doing::Next => {
                    let Some(step) = s.steps.pop_front() else { return false };
                    s.doing = self.start_step(s.unit, step, &mut s.watch);
                }
            }
        }
    }

    /// Starts a step of a unit's script; what the unit is then doing (and the units it
    /// watches, for a `wa` naming a group).
    fn start_step(&mut self, id: UnitId, step: Step, watch: &mut Vec<UnitId>) -> Doing {
        let tick = self.tick;
        // The nearest of the units named, as they stand now.
        let step = match step {
            Step::Nearest { among, attack } => match self.nearest_of(id, &among, attack) {
                Some(target) => Step::Orders(vec![if attack { Order::Attack(Target::Unit(target)) } else { Order::Guard(target) }]),
                None => return Doing::Next,
            },
            other => other,
        };
        let Some(u) = self.unit_mut(id) else { return Doing::Next };
        match step {
            Step::Nearest { .. } => Doing::Next,
            Step::WaitAttacked(units) => {
                u.orders.push_back(Order::Wait(END_MARK));
                *watch = units;
                Doing::Watching
            }
            Step::Orders(orders) => {
                u.orders.extend(orders);
                u.orders.push_back(Order::Wait(END_MARK));
                Doing::Orders
            }
            Step::Produce { type_id, count } => {
                // (As an order would: never past the queue's cap, whatever the script asks.)
                let count = (count as usize).min(crate::sim::FACTORY_QUEUE_MAX.saturating_sub(u.factory_queue.len()));
                u.factory_queue.extend(std::iter::repeat_n(type_id, count));
                if count > 0 { Doing::Producing } else { Doing::Next }
            }
            Step::Rally(order) => {
                if u.rally.len() < RALLY_MAX {
                    u.rally.push(order);
                }
                Doing::Next
            }
            Step::Unload(at) => {
                let aboard = u.cargo.len();
                let dest = at.unwrap_or(u.pos);
                u.orders.push_back(Order::Unload(dest));
                u.orders.push_back(Order::Wait(END_MARK));
                Doing::Unloading { aboard }
            }
            Step::SelfDestruct => {
                // At once: crawling bombs walk to their mark and go off; transports and their
                // escorts leave the battle this way (to a corner of the map, then `d`).
                u.self_destruct_manual = true;
                u.self_destruct_at = Some(tick);
                Doing::Next
            }
        }
    }

    /// The nearest to a unit of the units `among` names (alive, not itself): an enemy one
    /// to attack; to guard, one of its own side, else anyone's (Can Rush's jammer, its Cans
    /// gone, `g armfus`: the player's Fusion Reactor).
    fn nearest_of(&self, id: UnitId, among: &Among, attack: bool) -> Option<UnitId> {
        let me = self.unit(id)?;
        let (pos, owner) = (me.pos, me.owner);
        let units: Vec<&Unit> = match among {
            Among::Units(ids) => ids.iter().filter_map(|&u| self.unit(u)).collect(),
            Among::Type(t) => self.units().filter(|u| u.type_id == *t).collect(),
        };
        let nearest = |side: &dyn Fn(u8) -> bool| {
            units.iter().filter(|u| !u.dead && u.id != id && side(u.owner)).min_by_key(|u| ((u.pos - pos).xz().length_sq_raw(), u.id.slot)).map(|u| u.id)
        };
        if attack { nearest(&|o| self.is_enemy(owner, o)) } else { nearest(&|o| self.allied(owner, o)).or_else(|| nearest(&|_| true)) }
    }
}

/// A passenger is set down `elapsed` ticks into its script: the waits its script starts with
/// have been running since then, aboard or not.
fn skip_waited(steps: &mut VecDeque<Step>, mut elapsed: u64) {
    let Some(Step::Orders(orders)) = steps.front_mut() else { return };
    while elapsed > 0
        && let Some(Order::Wait(n)) = orders.first_mut()
    {
        let wait = u64::from(*n);
        if wait <= elapsed {
            elapsed -= wait;
            orders.remove(0);
        } else {
            *n -= elapsed as u32;
            elapsed = 0;
        }
    }
    if orders.is_empty() {
        steps.pop_front();
    }
}

/// Adds a patrol point as a player's orders do: the first pairs up with where the unit will
/// be by then (`from`), so the patrol loops back there; later ones slot in before that start.
fn patrol(orders: &mut Vec<Order>, to: FxVec3, from: FxVec3) {
    let n = orders.len();
    if n >= 2 && matches!((&orders[n - 2], &orders[n - 1]), (Order::Patrol(_), Order::Patrol(_))) {
        orders.insert(n - 1, Order::Patrol(to));
    } else {
        orders.push(Order::Patrol(to));
        orders.push(Order::Patrol(from));
    }
}

/// A placed unit's script as steps: runs of orders, and between them what a unit can't be
/// ordered to do (a factory's production and the way its units leave, a transport setting
/// one unit down, self-destruct, what is decided when the step comes). Commands naming what
/// isn't there (an unknown type, a unit no one is called) are left out, as are those the
/// unit can't carry out: buildings don't move, a factory builds only what it can.
///
/// A unit a command names is one called so (`Ident`), else a group's (`InitialGroup`: "works
/// much like setting a unique name, except that it allows a group of units to be
/// referenced", says TA's editor), else, for `a` and `g`, a unit type: `a ARMCOM` attacks
/// the nearest enemy one, `g CORTSHIP` guards the nearest (one of its own side first), as
/// TA's missions write it (Miniement Landing's escorts, Can Rush's jammer). `wa` waits on
/// the unit called so and the group of that name alike (Skirmish on the Sand's `wa patrol`).
fn compile_script(sim: &Sim, id: UnitId, script: &[MissionCommand], names: &MissionSetup) -> VecDeque<Step> {
    let mut steps = VecDeque::new();
    let Some(u) = sim.unit(id) else { return steps };
    let t = sim.types.get(u.type_id);
    let factory = t.is_factory && t.locomotion == Locomotion::Static;
    let mobile = t.locomotion != Locomotion::Static;
    let lookup = |name: &str| names.idents.get(&name.trim().to_ascii_uppercase()).copied();
    let group = |name: &str| names.groups.get(&name.trim().to_ascii_uppercase()).filter(|g| !g.is_empty());
    // A unit called so, else a group: the units a name stands for.
    let named = |name: &str| match (lookup(name), group(name)) {
        (Some(one), _) => Some(Among::Units(vec![one])),
        (None, Some(g)) => Some(Among::Units(g.clone())),
        (None, None) => None,
    };
    let point = |(x, z): (f32, f32)| FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z));
    let flush = |steps: &mut VecDeque<Step>, orders: &mut Vec<Order>| {
        if !orders.is_empty() {
            steps.push_back(Step::Orders(std::mem::take(orders)));
        }
    };
    let mut orders = Vec::new();
    // Where the unit will be when the next command comes (a patrol loops back there).
    let mut at = u.pos;
    for c in script {
        match c {
            MissionCommand::Move(x, z) if factory => {
                flush(&mut steps, &mut orders);
                steps.push_back(Step::Rally(Order::Move(point((*x, *z)))));
            }
            MissionCommand::Patrol(x, z) if factory => {
                flush(&mut steps, &mut orders);
                steps.push_back(Step::Rally(Order::Patrol(point((*x, *z)))));
            }
            MissionCommand::Move(x, z) if mobile => {
                at = point((*x, *z));
                orders.push(Order::Move(at));
            }
            MissionCommand::Patrol(x, z) if mobile => patrol(&mut orders, point((*x, *z)), at),
            MissionCommand::Move(..) | MissionCommand::Patrol(..) => {}
            // TA's waits are seconds of game time (see docs/FORMATS.md).
            MissionCommand::Wait(secs) if *secs > 0 => orders.push(Order::Wait(secs.saturating_mul(crate::cob::TICKS_PER_SECOND as u32).min(LONGEST_WAIT))),
            MissionCommand::Wait(_) => {}
            MissionCommand::WaitAttacked(name) => {
                // The unit called so and its group's, any one attacked; a name no one goes by:
                // this unit.
                let mut watched: Vec<UnitId> = name.as_deref().and_then(lookup).into_iter().collect();
                for &w in name.as_deref().and_then(group).into_iter().flatten() {
                    if !watched.contains(&w) {
                        watched.push(w);
                    }
                }
                match *watched.as_slice() {
                    [] => orders.push(Order::WaitAttacked(None)),
                    [one] => orders.push(Order::WaitAttacked(Some(one))),
                    _ => {
                        flush(&mut steps, &mut orders);
                        steps.push_back(Step::WaitAttacked(watched));
                    }
                }
            }
            MissionCommand::Attack(name) => {
                // Full idents/groups take precedence over types, as with guard orders.
                // Retain the first type for legacy commands with stray trailing words.
                let typed = || sim.types.id(name).or_else(|| name.split_whitespace().next().and_then(|n| sim.types.id(n)));
                match named(name) {
                    Some(Among::Units(one)) if one.len() == 1 => orders.push(Order::Attack(Target::Unit(one[0]))),
                    Some(among) => {
                        flush(&mut steps, &mut orders);
                        steps.push_back(Step::Nearest { among, attack: true });
                    }
                    None => orders.extend(typed().map(Order::AttackType)),
                }
            }
            MissionCommand::States { fire, movement } => orders.push(Order::SetStates { fire: *fire, movement: *movement }),
            MissionCommand::Guard(name) => match named(name).or_else(|| sim.types.id(name).map(Among::Type)) {
                // One unit: guarded until it is gone.
                Some(Among::Units(one)) if one.len() == 1 => {
                    if factory {
                        flush(&mut steps, &mut orders);
                        steps.push_back(Step::Rally(Order::Guard(one[0])));
                    } else {
                        orders.push(Order::Guard(one[0]));
                    }
                }
                // A group or a type: the nearest (a factory's way out: the nearest now).
                Some(among) if factory => {
                    flush(&mut steps, &mut orders);
                    steps.extend(sim.nearest_of(id, &among, false).map(|g| Step::Rally(Order::Guard(g))));
                }
                Some(among) => {
                    flush(&mut steps, &mut orders);
                    steps.push_back(Step::Nearest { among, attack: false });
                }
                None => {}
            },
            MissionCommand::Stop => orders.push(Order::Stop),
            MissionCommand::Build { unit, count, at: spot } => {
                let Some(b) = sim.types.id(unit).filter(|b| t.ai_build_options.contains(b)) else { continue };
                if factory {
                    flush(&mut steps, &mut orders);
                    steps.push_back(Step::Produce { type_id: b, count: *count });
                } else if mobile {
                    orders.push(Order::Build { type_id: b, pos: spot.map(point).unwrap_or(at) });
                }
            }
            MissionCommand::Unload(spot) if t.transport_capacity > 0 => {
                flush(&mut steps, &mut orders);
                steps.push_back(Step::Unload(spot.map(point)));
            }
            MissionCommand::Unload(_) => {}
            MissionCommand::SelfDestruct => {
                flush(&mut steps, &mut orders);
                steps.push_back(Step::SelfDestruct);
                break;
            }
            // Boarding is done as the units are placed; stockpiling weapons (`bw`) build
            // their missiles on their own.
            MissionCommand::Load(_) | MissionCommand::BuildWeapons(_) => {}
        }
    }
    flush(&mut steps, &mut orders);
    steps
}

/// Result of placing a mission: script idents resolved to units.
#[derive(Clone, Debug, Default)]
pub struct MissionSetup {
    /// The units called so (`Ident`), by name in upper case.
    pub idents: rustc_hash::FxHashMap<String, crate::unit::UnitId>,
    /// The units of each group (`InitialGroup`), by name in upper case, in placing order.
    pub groups: rustc_hash::FxHashMap<String, Vec<UnitId>>,
    /// The player's units a group's digit puts in that control group: (digit, unit), in
    /// placing order (Sink the Battleship's Titans in 1).
    pub control_groups: Vec<(u8, UnitId)>,
    /// Placed units whose type is missing from the game data.
    pub skipped: Vec<String>,
}

/// The transport a passenger's `i NAME` names: the unit called so, else the nearest of the
/// group or of that unit type that carries units, of the passenger's side (TA's editor: "i
/// ARMSHIP", "or if there is more than one transport give the transport a unique
/// identifier"; Mesa Run's Roach rides `i CORVALK`).
fn transport_named(sim: &Sim, id: UnitId, name: &str, setup: &MissionSetup) -> Option<UnitId> {
    let key = name.trim().to_ascii_uppercase();
    let me = sim.unit(id)?;
    if me.carried_by.is_some() { return None; }
    let carries = |u: &Unit| {
        let t = sim.unit_type(u);
        !u.dead && u.id != id && u.owner == me.owner && u.carried_by.is_none()
            && u.cargo.len() < t.transport_capacity as usize && t.can_carry(sim.unit_type(me))
    };
    if let Some(&t) = setup.idents.get(&key) {
        return sim.unit(t).filter(|u| carries(u)).map(|u| u.id);
    }
    let candidates: Vec<&Unit> = match setup.groups.get(&key) {
        Some(group) => group.iter().filter_map(|&u| sim.unit(u)).filter(|u| carries(u)).collect(),
        None => {
            let t = sim.types.id(name.trim())?;
            sim.units().filter(|u| u.type_id == t && carries(u)).collect()
        }
    };
    candidates.into_iter().min_by_key(|u| ((u.pos - me.pos).xz().length_sq_raw(), u.id.slot)).map(|u| u.id)
}

/// Places a mission's units, starts their scripts and installs its objectives and unit
/// restrictions. TA player `n` is sim player `n - 1`; player 1 is the human.
pub fn setup_mission(
    sim: &mut Sim,
    mission: &ta_data::mission::Mission,
    schema: &ta_data::mission::MissionSchema,
    allowed: Option<&[String]>,
) -> MissionSetup {
    let mut setup = MissionSetup::default();
    let mut placed = Vec::new();
    for pu in &schema.units {
        let Some(type_id) = sim.types.id(&pu.name) else {
            setup.skipped.push(pu.name.clone());
            continue;
        };
        let Some(owner) = pu.player.checked_sub(1).and_then(|n| u8::try_from(n).ok()) else { continue };
        if owner as usize >= sim.players.len() {
            continue;
        }
        let pos = FxVec3::new(Fx::from_f32(pu.x), Fx::ZERO, Fx::from_f32(pu.z));
        // TA angles are degrees clockwise from south, like our headings.
        let heading = (pu.angle.rem_euclid(360.0) / 360.0 * 65536.0) as i32;
        let Some(id) = sim.spawn_unit(type_id, owner, pos, heading, true) else { continue };
        let t = sim.types.get(type_id);
        let max = t.max_health;
        // TA sets its `NoAutoFire` guns to hold fire as they are built; the player's placed here
        // were never built, and keep their file's fire orders: Com Pattern Five's and A Shortage
        // of Water's Annihilators fire at will (`StandingFireOrder=2`), Go With the Floe's Big
        // Berthas hold their fire (0). A computer player's are its own to set going
        // (`SkirmishAi::arm_defenses`), or its script's. (Scripts' `o` come after.)
        let fire = (owner == 0).then(|| crate::sim::file_fire_state(&t.def));
        if let Some(u) = sim.unit_mut(id) {
            u.health = (max * Fx::from_f32(pu.health)).max(Fx::ONE);
            u.kills = pu.kills;
            u.xp = Fx::from_int(pu.kills as i64);
            if let Some(fire) = fire {
                u.fire_state = fire;
            }
        }
        if !pu.ident.is_empty() {
            setup.idents.insert(pu.ident.trim().to_ascii_uppercase(), id);
        }
        if !pu.group.is_empty() {
            setup.groups.entry(pu.group.trim().to_ascii_uppercase()).or_default().push(id);
            // A digit: the player's control group of that number.
            if let Ok(digit) = pu.group.trim().parse::<u8>()
                && digit <= 9
                && owner == 0
            {
                setup.control_groups.push((digit, id));
            }
        }
        placed.push((id, pu));
    }
    let mut scripts = Vec::new();
    // The units each flag of the placed units marks, in order.
    let marked = |flag: fn(&ta_data::mission::PlacedUnit) -> bool| {
        let mut ids: Vec<UnitId> = placed.iter().filter(|(_, pu)| flag(pu)).map(|(id, _)| *id).collect();
        ids.sort();
        ids
    };
    let (immune, ai_ignored, priority_targets) = (marked(|pu| pu.immune), marked(|pu| pu.ai_ignore), marked(|pu| pu.ai_priority));
    // The player's mission-critical units: the mission is lost with any one of them. (A
    // computer player's has no part in the goals: Sink the Battleship's battleship, the one
    // TA's missions mark, is the enemy's, and sinking it is how that mission is won.)
    let critical: Vec<UnitId> = marked(|pu| pu.critical).into_iter().filter(|&id| sim.unit(id).is_some_and(|u| u.owner == 0)).collect();
    // What computer players build first (Attack on Barrack's Dominators).
    let mut build_priority: Vec<(u8, usize)> = placed.iter().filter_map(|(id, pu)| Some((sim.unit(*id)?.owner, sim.types.id(pu.build_priority.trim())?))).collect();
    build_priority.sort();
    build_priority.dedup();
    for (id, pu) in placed {
        // `i TRANSPORT5`: starts stowed aboard that transport.
        let transport = pu.script.iter().find_map(|c| match c {
            MissionCommand::Load(name) => transport_named(sim, id, name, &setup),
            _ => None,
        });
        if let Some(t) = transport {
            // Reserve this space now: later passengers naming a type/group must
            // choose among transports that still have room.
            sim.attach(t, id, None);
        }
        let steps = compile_script(sim, id, &pu.script, &setup);
        if !steps.is_empty() {
            let owner = sim.unit(id).map_or(0, |u| u.owner);
            let doing = if transport.is_some() { Doing::Aboard } else { Doing::Next };
            scripts.push(UnitScript { unit: id, owner, since: sim.tick, steps, doing, watch: Vec::new() });
        }
    }
    scripts.sort_by_key(|s| s.unit);
    let goals: Vec<ta_data::mission::Goal> = mission.victory.iter().chain(&mission.defeat).cloned().collect();
    let mut rules = MissionRules::new(0, &goals, &sim.types);
    rules.all_victory = true;
    rules.defeat.extend(critical.into_iter().map(Objective::UnitLost));
    rules.scripts = scripts;
    rules.immune = immune;
    rules.ai_ignored = ai_ignored;
    rules.priority_targets = priority_targets;
    rules.build_priority = build_priority;
    // `maxunits`: each player's unit limit (the missions without one have none).
    if let Some(n) = mission.max_units {
        sim.unit_limit = n.max(1);
    }
    // Units the player must capture are never shot at automatically (explicit attack
    // orders still work), so a mission isn't lost to our own guns.
    for o in &rules.victory {
        if let Objective::CaptureUnitType(t) = o {
            sim.no_auto_target.insert(*t);
        }
    }
    sim.mission = Some(rules);
    // The scripts' first steps: the units start out on them.
    sim.run_unit_scripts();
    sim.commander_ends_game = false;
    sim.los_enabled = mission.line_of_sight;
    if mission.mapping {
        sim.reveal_map_for(0);
    }
    if let Some(names) = allowed {
        let mut a = vec![false; sim.types.types.len()];
        for n in names {
            if let Some(t) = sim.types.id(n) {
                a[t] = true;
            }
        }
        sim.allowed = Some(a);
    }
    setup
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u32 = crate::cob::TICKS_PER_SECOND as u32;

    /// A passenger's waits run while it rides: AC14's Roach (`i TRANSPORT4, w 2670, d`) rides a
    /// Valkyrie that sets off at 2,640 seconds; set down at 2,662 it has 8 seconds to go.
    #[test]
    fn a_passengers_waits_run_while_it_rides() {
        let mut steps = VecDeque::from([Step::Orders(vec![Order::Wait(2670 * SECOND)]), Step::SelfDestruct]);
        skip_waited(&mut steps, 2662 * SECOND as u64);
        assert_eq!(steps, VecDeque::from([Step::Orders(vec![Order::Wait(8 * SECOND)]), Step::SelfDestruct]));
        // Set down late: straight on to what comes next.
        skip_waited(&mut steps, 9 * SECOND as u64);
        assert_eq!(steps, VecDeque::from([Step::SelfDestruct]));
        // Only the waits it starts with: those after a move wait for the move.
        let to = FxVec3::from_ints(150, 0, 1000);
        let mut steps = VecDeque::from([Step::Orders(vec![Order::Wait(5 * SECOND), Order::Move(to), Order::Wait(10 * SECOND)])]);
        skip_waited(&mut steps, 60 * SECOND as u64);
        assert_eq!(steps, VecDeque::from([Step::Orders(vec![Order::Move(to), Order::Wait(10 * SECOND)])]));
    }

    /// Patrol points loop back to where the unit was when it set out, as a player's do; more
    /// points go in before that start.
    #[test]
    fn patrols_loop_back_to_their_start() {
        let (a, b, start) = (FxVec3::from_ints(100, 0, 0), FxVec3::from_ints(200, 0, 0), FxVec3::from_ints(0, 0, 50));
        let mut orders = vec![Order::Wait(SECOND)];
        patrol(&mut orders, a, start);
        patrol(&mut orders, b, start);
        assert_eq!(orders, vec![Order::Wait(SECOND), Order::Patrol(a), Order::Patrol(b), Order::Patrol(start)]);
    }

    #[test]
    fn crossings_and_end_marks() {
        let line = |toward| Crossing { at: Fx::from_int(100), toward };
        let (west, east) = (FxVec3::from_ints(99, 0, 500), FxVec3::from_ints(101, 0, 500));
        assert!(line(Toward::West).passed(west) && !line(Toward::West).passed(east));
        assert!(line(Toward::East).passed(east) && !line(Toward::East).passed(west));
        let (north, south) = (FxVec3::from_ints(500, 0, 99), FxVec3::from_ints(500, 0, 101));
        assert!(line(Toward::North).passed(north) && !line(Toward::North).passed(south));
        assert!(line(Toward::South).passed(south) && !line(Toward::South).passed(north));
        // The longest wait of TA's missions (Intimidator Ridge's 444,444 seconds) is no end mark.
        assert!(is_end_mark(&Order::Wait(END_MARK - 3)) && !is_end_mark(&Order::Wait(444_444 * SECOND)));
    }
}
