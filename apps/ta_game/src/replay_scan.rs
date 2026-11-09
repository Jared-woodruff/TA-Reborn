//! A replay's big moments, found ahead of the viewer. As a replay opens, a copy of the battle
//! at its start (the simulation, and its computer players for a record of only the humans'
//! orders) runs through the whole record on a thread of its own: the simulation is
//! deterministic, so it plays out as the replay will. Its losses are tallied as the highlights
//! tally them (`highlights::Tally`), and its costliest fights, nuclear strikes and fallen
//! Commanders mark the replay's timeline, a click away (`hud.rs`, `timeline`). It keeps its
//! marks as the replay starts over (a seek backwards), and stops when the replay is left. It
//! also keeps what went on second by second and where (`clips::ActionLog`), from which the
//! replay's social media clips are chosen and filmed (`clips.rs`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ta_ai::SkirmishAi;
use ta_sim::{Command, Sim};

use crate::highlights::{Moment, Tally};
use crate::replay::Record;

/// The most marks on a timeline, and how often (in ticks) the scan shares what it has found.
const MARKS: usize = 24;
const SHARE_EVERY: u64 = 30 * 30;

/// What the scan has found so far.
#[derive(Clone, Default)]
pub struct ScanState {
    /// How far it has got (ticks).
    pub done: u64,
    /// The battle's big moments found so far, in the order they happened.
    pub marks: Vec<Moment>,
    pub finished: bool,
    /// What went on, second by second (once the scan is finished).
    pub action: Option<Arc<crate::clips::ActionLog>>,
}

pub type SharedScan = Arc<Mutex<ScanState>>;

/// A replay's scan running (or done).
pub struct ReplayScan {
    record: Arc<Record>,
    state: SharedScan,
    stop: Arc<AtomicBool>,
}

impl ReplayScan {
    /// Starts scanning `record` from `sim` (the battle at its first tick) and `ais` (its computer
    /// players, which think along with it: a record of only the humans' orders needs their
    /// orders, and any record their state; see `game.rs`, `commands_for_tick`).
    pub fn start(record: Arc<Record>, sim: Sim, ais: Vec<SkirmishAi>) -> ReplayScan {
        let state: SharedScan = Arc::new(Mutex::new(ScanState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_record, thread_state, thread_stop) = (record.clone(), state.clone(), stop.clone());
        let spawned = std::thread::Builder::new().name("replay scan".into()).spawn(move || scan(&thread_record, sim, ais, &thread_state, &thread_stop));
        if let Err(e) = spawned {
            log::warn!("replay scan: {e}");
            if let Ok(mut s) = state.lock() {
                s.finished = true;
            }
        }
        ReplayScan { record, state, stop }
    }

    /// Whether this is `record`'s scan.
    pub fn is_for(&self, record: &Arc<Record>) -> bool {
        Arc::ptr_eq(&self.record, record)
    }

    /// What it has found, shared (the game's timeline reads it).
    pub fn state(&self) -> SharedScan {
        self.state.clone()
    }
}

impl Drop for ReplayScan {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn scan(record: &Record, mut sim: Sim, mut ais: Vec<SkirmishAi>, state: &SharedScan, stop: &AtomicBool) {
    let full = record.metadata.as_ref().is_some_and(|m| m.full_commands);
    let mut tally = Tally::default();
    let mut action = crate::clips::ActionRecorder::default();
    let mut next = record.ticks.iter().peekable();
    let started = std::time::Instant::now();
    while sim.tick < record.end_tick {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let tick = sim.tick + 1;
        // (The computer players think every tick, as in the replay, whether their orders are
        // the record's or theirs.)
        let ai_orders: Vec<Command> = ais.iter_mut().flat_map(|ai| ai.think(&sim)).collect();
        let mut commands = match next.peek() {
            Some((t, c)) if *t == tick => {
                let c = c.clone();
                next.next();
                c
            }
            _ => Vec::new(),
        };
        if !full {
            commands.extend(ai_orders);
        }
        sim.step(&commands);
        for e in sim.take_events() {
            tally.note(&e, &sim);
            action.note(&e, &sim);
        }
        // (The marks worked out before the lock is taken: the frame reads them every frame, and
        // in a long battle they take a while.)
        if sim.tick.is_multiple_of(SHARE_EVERY) {
            let marks = tally.marks(MARKS);
            if let Ok(mut s) = state.lock() {
                s.done = sim.tick;
                s.marks = marks;
            }
        }
    }
    let marks = tally.marks(MARKS);
    let action = Arc::new(action.finish(record.end_tick));
    if let Ok(mut s) = state.lock() {
        s.done = record.end_tick;
        s.marks = marks;
        s.action = Some(action);
        s.finished = true;
    }
    log::info!("replay scan: {} ticks in {:.1} s, {} marks", record.end_tick, started.elapsed().as_secs_f32(), tally.marks(MARKS).len());
}
