//! The battle's units heard as they go about it (TA Reborn's sound keys, `ta_audio::SoundEvent`):
//! setting off and coming to a stop, aircraft leaving the ground and setting down, the loops of
//! their moves, of their work (building, reclaiming, capturing) and of their buildings at work,
//! opening up and closing, their weapons' loops (a beam's hum, a gatling's barrels), and the unit
//! the player sent somewhere arriving. Each frame the units are looked at as the simulation has
//! them (nothing of it changes): a unit whose sound category gives none of these keys (TA's own)
//! costs a look at its type. Loops play for the nearest units alone, at a low priority, so that a
//! battle of many units leaves the voices to its guns and blasts; each stops as its unit dies,
//! goes out of sight or out of earshot, and with the scene.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ta_audio::{PlayParams, Priority, SoundEvent, SoundHandle, SoundKind};
use ta_sim::{Fx, FxVec3, Order, Sim, UnitId};

use super::GameAudio;

/// Loops of each kind heard at once: the nearest units' (the others' silent).
const MOVE_LOOPS: usize = 8;
const WORK_LOOPS: usize = 6;
const ACTIVE_LOOPS: usize = 6;
/// A unit's loop keeps its place against another's until the other is this much nearer (a share
/// of the distance): units passing each other do not swap their loops every frame.
const KEEP: f32 = 0.8;
/// Seconds a loop fades out over as it stops, where its sound set says none.
const LOOP_FADE: f32 = 0.25;
/// Ticks past a weapon's reload and burst its loop waits for the next shot before it stops.
const SHOT_MARGIN: u64 = 10;
/// Ticks the unit ordered is waited for to set off on its move (an order's way through the
/// network), and how near its goal it is heard to arrive.
const ORDER_WAIT: u64 = 90;
const ARRIVED_WITHIN: i64 = 96;

/// Which of the keys followed here a unit type's sound category gives.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Kinds {
    moves: bool,
    takeoff: bool,
    work: bool,
    active: bool,
    /// Which loops it has (a unit with only the transitions, `startmove` and no `moveloop`,
    /// takes no loop's place).
    move_loop: bool,
    build_loop: bool,
    reclaim_loop: bool,
    capture_loop: bool,
    active_loop: bool,
    /// How much farther than an ordinary sound its farthest-reaching of these is heard (its sound
    /// sets' `radius`, at least 1): how far round the listener its unit is listened to.
    reach: f32,
}

impl Kinds {
    fn none(self) -> bool {
        !(self.moves || self.takeoff || self.work || self.active)
    }

    /// Whether it has the loop of `work`.
    fn work_loop(self, work: Work) -> bool {
        match work {
            Work::Build => self.build_loop,
            Work::Reclaim => self.reclaim_loop,
            Work::Capture => self.capture_loop,
        }
    }
}

/// What a builder is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Work {
    Build,
    Reclaim,
    Capture,
}

impl Work {
    fn looped(self) -> SoundEvent {
        match self {
            Work::Build => SoundEvent::BuildLoop,
            Work::Reclaim => SoundEvent::ReclaimLoop,
            Work::Capture => SoundEvent::CaptureLoop,
        }
    }

    fn stop(self) -> SoundEvent {
        match self {
            Work::Build => SoundEvent::BuildStop,
            Work::Reclaim => SoundEvent::ReclaimStop,
            Work::Capture => SoundEvent::CaptureStop,
        }
    }
}

/// A unit as last looked at (by its slot).
#[derive(Clone, Copy, Debug)]
struct Was {
    id: UnitId,
    moving: bool,
    airborne: bool,
    work: Option<Work>,
    active: bool,
}

/// A loop playing: its handle, its key, the level and rate its sound set gave it (what the game
/// lays on as its unit goes is on top), and how long it fades out.
#[derive(Clone, Copy, Debug)]
pub(super) struct Loop {
    handle: SoundHandle,
    event: SoundEvent,
    volume: f32,
    pitch: f32,
    fade: f32,
}

/// A weapon's loop: the last tick it fired and how long it waits for the next.
#[derive(Clone, Copy, Debug)]
pub(super) struct Firing {
    handle: SoundHandle,
    fade: f32,
    last: u64,
    wait: u64,
}

/// The unit the player last gave an order (its "ok" said), heard to arrive where it was sent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Arrival {
    /// Its order on its way (since that tick).
    Ordered { unit: UnitId, since: u64 },
    Moving { unit: UnitId, to: FxVec3 },
}

/// A unit heard this frame, for a loop of one kind.
struct Candidate {
    distance: f32,
    unit: UnitId,
    type_id: usize,
    pos: [f32; 3],
    event: SoundEvent,
    level: f32,
    rate: f32,
}

/// The loops of one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Which {
    Moves,
    Work,
    Active,
}

#[derive(Default)]
pub(super) struct UnitSounds {
    /// The unit types these were worked out for (the simulation's, by where they are), and for
    /// each type which keys its category gives.
    types: usize,
    kinds: Vec<Option<Kinds>>,
    was: Vec<Option<Was>>,
    moves: BTreeMap<UnitId, Loop>,
    work: BTreeMap<UnitId, Loop>,
    active: BTreeMap<UnitId, Loop>,
    pub(super) weapons: BTreeMap<(UnitId, usize), Firing>,
    pub(super) arrival: Option<Arrival>,
    /// The tick of the last strategic launch announced.
    pub(super) announced: Option<u64>,
}

impl UnitSounds {
    /// Everything followed forgotten (its sounds stopped with the scene's).
    pub(super) fn forget(&mut self) {
        *self = UnitSounds::default();
    }

    fn loops(&mut self, which: Which) -> &mut BTreeMap<UnitId, Loop> {
        match which {
            Which::Moves => &mut self.moves,
            Which::Work => &mut self.work,
            Which::Active => &mut self.active,
        }
    }

    /// How many loops of each kind play now (moves, work, active, weapons).
    #[cfg(test)]
    pub(super) fn playing(&self) -> [usize; 4] {
        [self.moves.len(), self.work.len(), self.active.len(), self.weapons.len()]
    }
}

impl GameAudio {
    /// Which of the keys followed here unit type `type_id`'s sound category gives.
    fn kinds(&mut self, sim: &Sim, type_id: usize) -> Kinds {
        if let Some(Some(k)) = self.units.kinds.get(type_id) {
            return *k;
        }
        let kinds = match self.categories.try_category(&sim.types.get(type_id).def.sound_category) {
            Some(c) => {
                let any = |events: &[SoundEvent]| events.iter().any(|&e| c.get(e).is_some());
                let has = |event: SoundEvent| c.get(event).is_some();
                const FOLLOWED: [SoundEvent; 14] = [
                    SoundEvent::StartMove, SoundEvent::StopMove, SoundEvent::MoveLoop, SoundEvent::TakeOff, SoundEvent::Landing, SoundEvent::BuildLoop, SoundEvent::BuildStop,
                    SoundEvent::ReclaimLoop, SoundEvent::ReclaimStop, SoundEvent::CaptureLoop, SoundEvent::CaptureStop, SoundEvent::ActiveLoop, SoundEvent::Open, SoundEvent::Close,
                ];
                // (The farthest any of them carries: a sound set's `radius`, a file's 1.)
                let sets = self.engine.sound_sets();
                let reach = FOLLOWED.iter().flat_map(|&e| c.variants(e)).map(|name| sets.get(name).map_or(1.0, |set| set.radius)).fold(1.0f32, f32::max);
                Kinds {
                    moves: any(&[SoundEvent::StartMove, SoundEvent::StopMove, SoundEvent::MoveLoop]),
                    takeoff: any(&[SoundEvent::TakeOff, SoundEvent::Landing]),
                    work: any(&[SoundEvent::BuildLoop, SoundEvent::BuildStop, SoundEvent::ReclaimLoop, SoundEvent::ReclaimStop, SoundEvent::CaptureLoop, SoundEvent::CaptureStop]),
                    active: any(&[SoundEvent::ActiveLoop, SoundEvent::Open, SoundEvent::Close]),
                    move_loop: has(SoundEvent::MoveLoop),
                    build_loop: has(SoundEvent::BuildLoop),
                    reclaim_loop: has(SoundEvent::ReclaimLoop),
                    capture_loop: has(SoundEvent::CaptureLoop),
                    active_loop: has(SoundEvent::ActiveLoop),
                    reach,
                }
            }
            None => Kinds::default(),
        };
        if self.units.kinds.len() <= type_id {
            self.units.kinds.resize(type_id + 1, None);
        }
        self.units.kinds[type_id] = Some(kinds);
        kinds
    }

    /// Unit type `type_id`'s sound for the first of `events` its category gives, played once at
    /// `pos` (none where it gives none of them).
    pub(super) fn type_sound(&mut self, sim: &Sim, type_id: usize, events: &[SoundEvent], pos: [f32; 3], volume: f32) -> SoundHandle {
        let Some(category) = self.categories.try_category(&sim.types.get(type_id).def.sound_category) else { return SoundHandle::NONE };
        let Some(&event) = events.iter().find(|&&e| category.get(e).is_some()) else { return SoundHandle::NONE };
        let category = category.clone();
        let Some(name) = self.engine.pick(&category, event) else { return SoundHandle::NONE };
        self.engine.play(&name, PlayParams::at(pos, volume))
    }

    /// Follows the battle's units for the sounds they make as they go about it (see the module).
    /// `viewer`: whose eyes the view uses (none: everything is seen; what they cannot see is not
    /// heard); `local`: the player at the keys (their unit sent somewhere heard arriving);
    /// `paused`: the battle stands still (its loops silent, back as it goes on).
    pub fn update_units(&mut self, sim: &Sim, viewer: Option<u8>, local: u8, paused: bool) {
        let types = Arc::as_ptr(&sim.types) as usize;
        if self.units.types != types {
            self.stop_unit_loops();
            self.units.forget();
            self.units.types = types;
        }
        if paused {
            self.stop_unit_loops();
            return;
        }
        let listener = *self.engine.listener();
        // (The hearing curve reaches two half widths of the view, and a little more.)
        let reach = listener.view_width * 1.1;
        let seen = |pos: FxVec3| viewer.is_none_or(|player| sim.in_sight(player, pos));
        let slots = sim.unit_slots();
        if self.units.was.len() < slots.len() {
            self.units.was.resize(slots.len(), None);
        }
        let mut candidates: [Vec<Candidate>; 3] = Default::default();
        for (slot, u) in slots.iter().enumerate() {
            let Some(u) = u else {
                self.units.was[slot] = None;
                continue;
            };
            let kinds = self.kinds(sim, u.type_id);
            if kinds.none() {
                self.units.was[slot] = None;
                continue;
            }
            let t = sim.unit_type(u);
            let plane = t.is_plane();
            let airborne = plane && !u.landed && u.carried_by.is_none() && u.is_complete();
            let moving = if plane { airborne } else { u.moving && u.is_complete() };
            let work = u.build.as_ref().filter(|j| j.started).map(|_| match u.orders.front() {
                Some(Order::Reclaim(_) | Order::ReclaimUnit(_)) => Work::Reclaim,
                Some(Order::Capture(_)) => Work::Capture,
                _ => Work::Build,
            });
            let active = u.activated && u.is_complete() && u.upkeep_paid;
            let pos = u.pos.to_f32();
            let l = listener.position;
            let distance = ((pos[0] - l[0]).powi(2) + (pos[1] - l[1]).powi(2) + (pos[2] - l[2]).powi(2)).sqrt();
            // (Within earshot of its farthest-reaching sound: a sound set's `radius` carries it.)
            let heard = !u.dead && u.carried_by.is_none() && distance < reach * kinds.reach && seen(u.pos);
            let was = self.units.was[slot].filter(|w| w.id == u.id);
            self.units.was[slot] = Some(Was { id: u.id, moving, airborne, work, active });
            // What changed since the last look (nothing the first time it is looked at).
            if let Some(was) = was.filter(|_| heard) {
                let one = |events: &[SoundEvent]| events.to_vec();
                let mut due: Vec<Vec<SoundEvent>> = Vec::new();
                if plane && (kinds.takeoff || kinds.moves) {
                    if !was.airborne && airborne {
                        due.push(one(&[SoundEvent::TakeOff, SoundEvent::StartMove]));
                    } else if was.airborne && !airborne {
                        due.push(one(&[SoundEvent::Landing, SoundEvent::StopMove]));
                    }
                } else if kinds.moves && was.moving != moving {
                    due.push(one(if moving { &[SoundEvent::StartMove] } else { &[SoundEvent::StopMove] }));
                }
                if kinds.work && was.work != work && let Some(w) = was.work {
                    due.push(one(&[w.stop()]));
                }
                if kinds.active && was.active != active {
                    due.push(one(if active { &[SoundEvent::Open] } else { &[SoundEvent::Close] }));
                }
                for events in due {
                    self.type_sound(sim, u.type_id, &events, pos, 1.0);
                }
            }
            if !heard {
                continue;
            }
            if kinds.move_loop && moving {
                // (Louder and higher the faster it goes.)
                let speed = (u.speed.to_f32() / t.max_speed.to_f32().max(0.01)).clamp(0.0, 1.0);
                candidates[0].push(Candidate { distance, unit: u.id, type_id: u.type_id, pos, event: SoundEvent::MoveLoop, level: 0.45 + 0.55 * speed, rate: 0.88 + 0.2 * speed });
            }
            if let Some(w) = work.filter(|w| kinds.work_loop(*w)) {
                candidates[1].push(Candidate { distance, unit: u.id, type_id: u.type_id, pos, event: w.looped(), level: 1.0, rate: 1.0 });
            }
            if kinds.active_loop && active {
                candidates[2].push(Candidate { distance, unit: u.id, type_id: u.type_id, pos, event: SoundEvent::ActiveLoop, level: 1.0, rate: 1.0 });
            }
        }
        let [moves, work, active] = candidates;
        self.keep_loops(sim, Which::Moves, moves, MOVE_LOOPS);
        self.keep_loops(sim, Which::Work, work, WORK_LOOPS);
        self.keep_loops(sim, Which::Active, active, ACTIVE_LOOPS);
        self.follow_weapons(sim, &seen);
        self.follow_arrival(sim, local);
    }

    /// The loops of one kind for the nearest `most` of `candidates` (those whose category gives
    /// the key): started, moved with their units and changed as they go; the others stopped.
    fn keep_loops(&mut self, sim: &Sim, which: Which, mut candidates: Vec<Candidate>, most: usize) {
        let mut loops = std::mem::take(self.units.loops(which));
        for c in &mut candidates {
            if loops.get(&c.unit).is_some_and(|l| l.event == c.event) {
                c.distance *= KEEP;
            }
        }
        candidates.sort_by(|a, b| a.distance.total_cmp(&b.distance).then(a.unit.cmp(&b.unit)));
        candidates.truncate(most);
        let kept: BTreeSet<(UnitId, SoundEvent)> = candidates.iter().map(|c| (c.unit, c.event)).collect();
        loops.retain(|unit, l| {
            let keep = kept.contains(&(*unit, l.event)) && self.engine.is_playing(l.handle);
            if !keep {
                self.engine.stop_with_fade(l.handle, l.fade);
            }
            keep
        });
        for c in candidates {
            match loops.get(&c.unit) {
                Some(l) => {
                    self.engine.set_position(l.handle, c.pos);
                    self.engine.set_sound_volume(l.handle, l.volume * c.level);
                    self.engine.set_pitch(l.handle, l.pitch * c.rate);
                }
                None => {
                    let Some(category) = self.categories.try_category(&sim.types.get(c.type_id).def.sound_category).cloned() else { continue };
                    let Some(name) = self.engine.pick(&category, c.event) else { continue };
                    let played = self.engine.play_detailed(&name, PlayParams::at(c.pos, c.level).pitch(c.rate).looping(true).priority(Priority::Low).kind(SoundKind::World));
                    if !played.handle.is_none() {
                        let fade = if played.fade_out > 0.0 { played.fade_out } else { LOOP_FADE };
                        loops.insert(c.unit, Loop { handle: played.handle, event: c.event, volume: played.volume / c.level.max(1e-3), pitch: played.pitch / c.rate.max(1e-3), fade });
                    }
                }
            }
        }
        *self.units.loops(which) = loops;
    }

    /// A weapon of `unit` (its type `weapon`) fired at `pos`: its loop (`soundloop`) on from its
    /// first shot, kept on while the shots keep coming.
    pub(super) fn weapon_loop(&mut self, sim: &Sim, unit: UnitId, weapon: usize, pos: [f32; 3]) {
        let w = sim.weapon_types.get(weapon);
        if w.def.sound_loop.trim().is_empty() {
            return;
        }
        let tick = sim.tick;
        if let Some(f) = self.units.weapons.get_mut(&(unit, weapon))
            && self.engine.is_playing(f.handle)
        {
            f.last = tick;
            return;
        }
        let played = self.engine.play_detailed(&w.def.sound_loop, PlayParams::at(pos, 0.8).looping(true).kind(SoundKind::World));
        if !played.handle.is_none() {
            let wait = w.reload_ticks as u64 + w.burst as u64 * w.burst_ticks as u64 + SHOT_MARGIN;
            let fade = if played.fade_out > 0.0 { played.fade_out } else { LOOP_FADE };
            self.units.weapons.insert((unit, weapon), Firing { handle: played.handle, fade, last: tick, wait });
        }
    }

    /// The weapons' loops: moved with their units; stopped as the shots stop coming (its
    /// `soundstop` then), as the unit dies or goes out of sight.
    fn follow_weapons(&mut self, sim: &Sim, seen: &dyn Fn(FxVec3) -> bool) {
        let tick = sim.tick;
        // (Each: its unit and weapon, and where its unit is, if it is still there.)
        type Stopped = ((UnitId, usize), Option<[f32; 3]>);
        let mut stopped: Vec<Stopped> = Vec::new();
        for (&(unit, weapon), f) in &self.units.weapons {
            match sim.unit(unit).filter(|u| !u.dead && seen(u.pos)) {
                Some(u) if tick <= f.last + f.wait && self.engine.is_playing(f.handle) => self.engine.set_position(f.handle, u.pos.to_f32()),
                Some(u) => stopped.push(((unit, weapon), Some(u.pos.to_f32()))),
                None => stopped.push(((unit, weapon), None)),
            }
        }
        for (key, pos) in stopped {
            let Some(f) = self.units.weapons.remove(&key) else { continue };
            self.engine.stop_with_fade(f.handle, f.fade);
            let stop = &sim.weapon_types.get(key.1).def.sound_stop;
            if let Some(pos) = pos
                && !stop.trim().is_empty()
            {
                let stop = stop.clone();
                self.engine.play_at(&stop, pos, 0.8);
            }
        }
    }

    /// The unit the player last ordered heard arriving where its move took it (`arrived`): its
    /// order seen through to its end, near its goal (not another order in its place).
    fn follow_arrival(&mut self, sim: &Sim, local: u8) {
        let next = match self.units.arrival {
            None => return,
            Some(Arrival::Ordered { unit, since }) => match sim.unit(unit).map(|u| (u.owner, u.orders.front())) {
                Some((owner, _)) if owner != local => None,
                Some((_, Some(Order::Move(to)))) => Some(Arrival::Moving { unit, to: *to }),
                Some(_) if sim.tick <= since + ORDER_WAIT => self.units.arrival,
                _ => None,
            },
            Some(Arrival::Moving { unit, to }) => match sim.unit(unit) {
                None => None,
                Some(u) => match u.orders.front() {
                    Some(Order::Move(next)) if *next == to => self.units.arrival,
                    // (The next of a queue of moves: heard at the last.)
                    Some(Order::Move(next)) => Some(Arrival::Moving { unit, to: *next }),
                    _ => {
                        if u.pos.within_xz(to, Fx::from_int(ARRIVED_WITHIN)) {
                            self.unit_voice(sim, unit, SoundEvent::Arrived);
                        }
                        None
                    }
                },
            },
        };
        self.units.arrival = next;
    }

    /// The unit the player gave an order (its "ok" said): heard arriving where a move takes it.
    pub(super) fn ordered(&mut self, sim: &Sim, unit: UnitId) {
        self.units.arrival = Some(Arrival::Ordered { unit, since: sim.tick });
    }

    /// Every unit loop stopped (the scene goes on: the next look starts them again).
    pub(super) fn stop_unit_loops(&mut self) {
        let loops: Vec<(SoundHandle, f32)> = [&self.units.moves, &self.units.work, &self.units.active]
            .into_iter()
            .flat_map(|m| m.values().map(|l| (l.handle, l.fade)))
            .chain(self.units.weapons.values().map(|f| (f.handle, f.fade)))
            .collect();
        for (handle, fade) in loops {
            self.engine.stop_with_fade(handle, fade);
        }
        self.units.moves.clear();
        self.units.work.clear();
        self.units.active.clear();
        self.units.weapons.clear();
    }

    /// A unit's shield came up (`on`) or went down (`SimEvent::ShieldChanged`, `sim/shield.rs`): its
    /// `shieldon` or `shieldoff` where it is.
    pub fn shield_changed(&mut self, sim: &Sim, unit: UnitId, on: bool) {
        if let Some(u) = sim.unit(unit) {
            let (type_id, pos) = (u.type_id, u.pos.to_f32());
            self.type_sound(sim, type_id, &[if on { SoundEvent::ShieldOn } else { SoundEvent::ShieldOff }], pos, 1.0);
        }
    }
}
