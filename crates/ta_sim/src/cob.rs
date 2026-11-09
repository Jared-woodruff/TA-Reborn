//! COB script virtual machine.
//!
//! Every TA unit carries a compiled BOS script (`scripts/<object>.cob`) that animates its
//! pieces and answers engine queries (aim a weapon, where to fire from, where to build).
//! The engine calls named functions (`Create`, `StartMoving`, `AimPrimary(heading, pitch)`,
//! `QueryPrimary(piece)`, ...), each running as a cooperative thread that can sleep, wait for
//! animations, start other threads and kill them with signals.
//!
//! Semantics, verified against the shipped bytecode:
//! * `turn`/`move` pop the target (top of stack) then the speed; `spin` pops the speed
//!   (top) then the acceleration; speeds are per second. Turns go the short way round.
//! * Parameters are pushed by the caller; the callee's leading `create-local`s claim them
//!   instead of pushing zeros. Query functions return values through their parameters.
//! * `get` pops four arguments and then the value id.
//! * Linear values are 16.16 model units (1.0 = one map pixel); angles are 65536 per turn.
//! * Pieces keep the values as the script gives them. Our frame is the 3DO's with z
//!   negated; there, moves along x and turns about z apply negated and the rest as they are
//!   (`unit::piece_world_position` and the renderer's piece matrices; docs/FORMATS.md).

use std::sync::Arc;

use rustc_hash::FxHashMap;
use ta_formats::cob::{CobScript, op};

use crate::fixed::wrap_angle;

/// Instructions a single thread may execute per tick before being forcibly yielded.
const INSTRUCTION_BUDGET: u32 = 20_000;
// Bound both sleeping threads and same-tick START_SCRIPT chains. Dead slots count until
// the run ends, so a script cannot evade the limit by spawning a child then returning.
const MAX_THREADS: usize = 256;
const RUN_INSTRUCTION_BUDGET: u32 = INSTRUCTION_BUDGET * 5;
const MAX_STACK: usize = 1024;
const MAX_CALL_DEPTH: usize = 64;
pub const TICKS_PER_SECOND: i64 = 30;

/// A loaded script plus lookup tables, shared by all units of a type.
#[derive(Debug)]
pub struct CobProgram {
    pub script: CobScript,
    functions: FxHashMap<String, usize>,
    /// COB piece index → model piece index.
    pub piece_map: Vec<Option<usize>>,
}

impl CobProgram {
    pub fn new(script: CobScript, model_piece_names: &[String]) -> CobProgram {
        let functions = script.functions.iter().enumerate().map(|(i, f)| (f.name.to_ascii_lowercase(), i)).collect();
        let piece_map = script
            .pieces
            .iter()
            .map(|p| model_piece_names.iter().position(|m| m.eq_ignore_ascii_case(p)))
            .collect();
        CobProgram { script, functions, piece_map }
    }

    /// Case-insensitive lookup. Called several times per unit per tick (aiming, building),
    /// so short names are lowered on the stack instead of in a new `String`.
    pub fn function(&self, name: &str) -> Option<usize> {
        let bytes = name.as_bytes();
        let mut buf = [0u8; 64];
        if bytes.len() <= buf.len() {
            for (d, s) in buf.iter_mut().zip(bytes) {
                *d = s.to_ascii_lowercase();
            }
            let key = std::str::from_utf8(&buf[..bytes.len()]).ok()?;
            return self.functions.get(key).copied();
        }
        self.functions.get(&name.to_ascii_lowercase()).copied()
    }
}

/// Animation state of one axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Motion {
    #[default]
    Idle,
    /// Linear move towards `target` at `speed` units/second.
    Move { target: i32, speed: i32 },
    /// Rotation towards `target` (shortest way) at `speed` angle units/second.
    Turn { target: i32, speed: i32 },
    /// Continuous rotation; `speed` ramps towards `target_speed` by `accel` per second.
    Spin { speed: i32, target_speed: i32, accel: i32 },
    /// Spin slowing to a stop.
    StopSpin { speed: i32, decel: i32 },
}

/// Animated state of one COB piece.
#[derive(Clone, Debug)]
pub struct PieceAnim {
    /// Translation from the rest position, 16.16 model units (x, y, z).
    pub offset: [i32; 3],
    /// Rotation about x, y, z in angle units.
    pub rotation: [i32; 3],
    pub visible: bool,
    /// Whether the piece's animated textures (lamps, glows) play: TA's `dont-cache`, until a
    /// `cache` holds them on their first frame, where a piece starts (see FORMATS.md). A metal
    /// maker's `Go` lets its glow play and its `Stop` holds it.
    pub textures_play: bool,
    moves: [Motion; 3],
    turns: [Motion; 3],
}

impl Default for PieceAnim {
    fn default() -> Self {
        PieceAnim { offset: [0; 3], rotation: [0; 3], visible: true, textures_play: false, moves: [Motion::Idle; 3], turns: [Motion::Idle; 3] }
    }
}

impl PieceAnim {
    /// Whether anything on the piece is still moving or turning.
    fn active(&self) -> bool {
        self.moves.iter().chain(&self.turns).any(|m| !matches!(m, Motion::Idle))
    }

    fn advance(&mut self) {
        for axis in 0..3 {
            match self.moves[axis] {
                Motion::Move { target, speed } => {
                    let step = (speed as i64 / TICKS_PER_SECOND).max(1);
                    let cur = self.offset[axis] as i64;
                    let t = target as i64;
                    let next = if (t - cur).abs() <= step { t } else { cur + step * (t - cur).signum() };
                    self.offset[axis] = next as i32;
                    if next == t {
                        self.moves[axis] = Motion::Idle;
                    }
                }
                _ => self.moves[axis] = Motion::Idle,
            }
            match self.turns[axis] {
                Motion::Turn { target, speed } => {
                    let step = (speed as i64 / TICKS_PER_SECOND).max(1) as i32;
                    let delta = wrap_angle(target.wrapping_sub(self.rotation[axis]));
                    if delta.abs() <= step {
                        self.rotation[axis] = wrap_angle(target);
                        self.turns[axis] = Motion::Idle;
                    } else {
                        self.rotation[axis] = wrap_angle(self.rotation[axis] + step * delta.signum());
                    }
                }
                // Speeds are per second, but accelerations per frame (TA's 30 a second: an
                // extractor's arm reaches full speed in seconds, not minutes).
                Motion::Spin { speed, target_speed, accel } => {
                    let speed = if accel == 0 {
                        target_speed
                    } else {
                        let a = accel.saturating_abs();
                        if speed < target_speed { speed.saturating_add(a).min(target_speed) } else { speed.saturating_sub(a).max(target_speed) }
                    };
                    self.rotation[axis] = wrap_angle(self.rotation[axis].wrapping_add(speed / TICKS_PER_SECOND as i32));
                    self.turns[axis] = Motion::Spin { speed, target_speed, accel };
                }
                Motion::StopSpin { speed, decel } => {
                    let d = decel.saturating_abs();
                    let speed = if speed > 0 { (speed - d).max(0) } else { (speed + d).min(0) };
                    self.rotation[axis] = wrap_angle(self.rotation[axis].wrapping_add(speed / TICKS_PER_SECOND as i32));
                    self.turns[axis] = if speed == 0 { Motion::Idle } else { Motion::StopSpin { speed, decel } };
                }
                _ => {}
            }
        }
    }

    pub fn is_turning(&self, axis: usize) -> bool {
        !matches!(self.turns[axis], Motion::Idle)
    }

    pub fn is_moving(&self, axis: usize) -> bool {
        !matches!(self.moves[axis], Motion::Idle)
    }

    pub fn is_animating(&self) -> bool {
        (0..3).any(|a| self.is_turning(a) || self.is_moving(a))
    }
}

/// Engine services a running script needs.
pub trait CobHost {
    /// Current simulation time in milliseconds.
    fn now_ms(&self) -> i64;
    /// Uniform random integer in `lo..=hi`.
    fn random(&mut self, lo: i32, hi: i32) -> i32;
    /// `get VALUE(a, b, c, d)`; `instance` gives access to piece state.
    fn get_value(&mut self, instance: &CobInstance, id: i32, args: [i32; 4]) -> i32;
    fn set_value(&mut self, id: i32, value: i32);
    fn explode(&mut self, instance: &CobInstance, piece: usize, flags: i32);
    fn emit_sfx(&mut self, instance: &CobInstance, piece: usize, kind: i32);
    fn play_sound(&mut self, _sound: i32) {}
    fn attach_unit(&mut self, _unit: i32, _piece: i32) {}
    fn drop_unit(&mut self, _unit: i32) {}
}

/// Engine-assigned label identifying why a thread was started, so results can be collected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThreadTag {
    #[default]
    None,
    /// `AimPrimary/Secondary/Tertiary` for weapon slot n.
    Aim(u16),
    /// An aim for weapon slot n the engine no longer waits for (its target changed, or it went
    /// unanswered too long): it runs on as the script lets it (the script's own `signal`
    /// ends it when the next aim starts), but its answer is dropped.
    OldAim(u16),
    /// `StartBuilding`.
    Build,
    /// `TransportPickup` / `TransportDrop`.
    Transport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wait {
    Ready,
    SleepUntil(i64),
    /// Until a piece (any the script has) stops turning or moving about an axis.
    Turn(u32, u8),
    Move(u32, u8),
}

#[derive(Clone, Debug)]
struct Frame {
    function: usize,
    return_pc: usize,
    base: usize,
    argc: usize,
    params_unclaimed: usize,
}

#[derive(Clone, Debug)]
struct Thread {
    id: u32,
    pc: usize,
    stack: Vec<i32>,
    frames: Vec<Frame>,
    signal_mask: i32,
    wait: Wait,
    dead: bool,
    tag: ThreadTag,
}

/// Result of a finished engine-started thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallResult {
    pub value: i32,
    /// Final values of the function's parameters (for `Query*` functions).
    pub params: Vec<i32>,
}

/// Per-unit script state.
#[derive(Clone, Debug)]
pub struct CobInstance {
    pub program: Arc<CobProgram>,
    pub statics: Vec<i32>,
    /// Animation state indexed by COB piece.
    pub pieces: Vec<PieceAnim>,
    threads: Vec<Thread>,
    finished: Vec<(ThreadTag, CallResult)>,
    next_id: u32,
    /// Some piece may be moving or turning (set whenever a thread touches a piece; cleared
    /// by `animate` once everything has come to rest), so idle units skip the animation walk.
    animating: bool,
}

impl CobInstance {
    pub fn new(program: Arc<CobProgram>) -> CobInstance {
        CobInstance {
            statics: vec![0; program.script.static_var_count as usize],
            pieces: vec![PieceAnim::default(); program.script.pieces.len()],
            program,
            threads: Vec::new(),
            finished: Vec::new(),
            next_id: 1,
            animating: false,
        }
    }

    pub fn has_function(&self, name: &str) -> bool {
        self.program.function(name).is_some()
    }

    /// Starts `name(args)` as a new thread (runs on the next [`CobInstance::run`]).
    pub fn start(&mut self, name: &str, args: &[i32], tag: ThreadTag) -> bool {
        let Some(f) = self.program.function(name) else { return false };
        self.spawn(f, args.to_vec(), 0, tag).is_some()
    }

    fn spawn(&mut self, function: usize, args: Vec<i32>, signal_mask: i32, tag: ThreadTag) -> Option<u32> {
        if self.threads.len() >= MAX_THREADS || args.len() > MAX_STACK {
            return None;
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let argc = args.len();
        let entry = self.program.script.functions[function].offset as usize;
        self.threads.push(Thread {
            id,
            pc: entry,
            stack: args,
            frames: vec![Frame { function, return_pc: usize::MAX, base: 0, argc, params_unclaimed: argc }],
            signal_mask,
            wait: Wait::Ready,
            dead: false,
            tag,
        });
        Some(id)
    }

    /// Runs `name(args)` immediately to completion (for queries that never block).
    /// Returns `None` if the function does not exist or the thread limit is reached; if it blocks, the thread keeps running
    /// on later ticks and its parameters as of now are returned.
    pub fn call(&mut self, host: &mut dyn CobHost, name: &str, args: &[i32]) -> Option<CallResult> {
        let f = self.program.function(name)?;
        let id = self.spawn(f, args.to_vec(), 0, ThreadTag::None)?;
        let idx = self.threads.len() - 1;
        let mut budget = INSTRUCTION_BUDGET;
        self.run_thread(idx, host, &mut budget);
        let t = self.threads.iter().position(|t| t.id == id);
        match t {
            Some(i) if !self.threads[i].dead => {
                let th = &self.threads[i];
                let base = th.frames.first().map_or(0, |f| f.base);
                Some(CallResult { value: 0, params: th.stack[base..(base + args.len()).min(th.stack.len())].to_vec() })
            }
            _ => {
                // Finished: its result was stored with ThreadTag::None; take the last one.
                let pos = self.finished.iter().rposition(|(tag, _)| *tag == ThreadTag::None)?;
                Some(self.finished.remove(pos).1)
            }
        }
    }

    /// The engine stops waiting for weapon `slot`'s aims: those still running become
    /// [`ThreadTag::OldAim`] (they run on, their answers dropped) and answers not yet taken
    /// are forgotten. The engine never kills an aim itself: a script guards some steps from
    /// its own `signal` (an unpacking Hammer sets its signal mask to 0 while it unfolds and
    /// marks itself busy), and a thread cut off there leaves the unit busy for good.
    pub fn supersede_aims(&mut self, slot: u16) {
        for t in &mut self.threads {
            if t.tag == ThreadTag::Aim(slot) {
                t.tag = ThreadTag::OldAim(slot);
            }
        }
        self.finished.retain(|(t, _)| *t != ThreadTag::Aim(slot));
    }

    pub fn is_running(&self, tag: ThreadTag) -> bool {
        self.threads.iter().any(|t| !t.dead && t.tag == tag)
    }

    /// Removes and returns the result of the most recent finished thread with `tag`.
    pub fn take_result(&mut self, tag: ThreadTag) -> Option<CallResult> {
        let pos = self.finished.iter().rposition(|(t, _)| *t == tag)?;
        let r = self.finished.remove(pos).1;
        self.finished.retain(|(t, _)| *t != tag);
        Some(r)
    }

    /// Whether any thread is alive (dead ones are dropped after every run, so running the
    /// VM with none would do nothing).
    pub fn has_threads(&self) -> bool {
        !self.threads.is_empty()
    }

    pub fn thread_count(&self) -> usize {
        self.threads.iter().filter(|t| !t.dead).count()
    }

    /// Advances piece animations by one tick.
    pub fn animate(&mut self) {
        if !self.animating {
            return;
        }
        let mut active = false;
        for p in &mut self.pieces {
            p.advance();
            active |= p.active();
        }
        self.animating = active;
    }

    /// Whether a thread would run this tick (`now` in script milliseconds): the VM host need
    /// only be set up when one would.
    pub fn any_ready(&self, now: i64) -> bool {
        self.threads.iter().any(|t| self.thread_ready(t, now))
    }

    fn thread_ready(&self, t: &Thread, now: i64) -> bool {
        !t.dead
            && match t.wait {
                Wait::Ready => true,
                Wait::SleepUntil(ms) => now >= ms,
                Wait::Turn(p, a) => !self.pieces.get(p as usize).is_some_and(|pc| pc.is_turning(a as usize)),
                Wait::Move(p, a) => !self.pieces.get(p as usize).is_some_and(|pc| pc.is_moving(a as usize)),
            }
    }

    /// Runs all threads that are ready this tick.
    pub fn run(&mut self, host: &mut dyn CobHost) {
        let now = host.now_ms();
        let mut i = 0;
        let mut budget = RUN_INSTRUCTION_BUDGET;
        // Threads started during this loop are appended and also run this tick.
        while i < self.threads.len() && budget > 0 {
            let ready = self.thread_ready(&self.threads[i], now);
            if ready {
                self.threads[i].wait = Wait::Ready;
                self.run_thread(i, host, &mut budget);
            }
            i += 1;
        }
        self.threads.retain(|t| !t.dead);
        // Keep only recent results; the engine consumes them within a tick or two.
        if self.finished.len() > 32 {
            let excess = self.finished.len() - 32;
            self.finished.drain(..excess);
        }
    }

    fn piece_mut(&mut self, piece: i32) -> Option<&mut PieceAnim> {
        self.animating = true;
        self.pieces.get_mut(usize::try_from(piece).ok()?)
    }

    fn run_thread(&mut self, idx: usize, host: &mut dyn CobHost, run_budget: &mut u32) {
        let program = self.program.clone();
        let code = &program.script.code;
        let mut budget = INSTRUCTION_BUDGET;
        // Take the thread out so the instance (pieces, statics, other threads) stays usable.
        let mut t = std::mem::replace(
            &mut self.threads[idx],
            Thread { id: 0, pc: 0, stack: Vec::new(), frames: Vec::new(), signal_mask: 0, wait: Wait::Ready, dead: true, tag: ThreadTag::None },
        );

        macro_rules! pop {
            () => {
                t.stack.pop().unwrap_or(0)
            };
        }
        macro_rules! arg {
            ($n:expr) => {
                code.get(t.pc + $n).copied().unwrap_or(0) as i32
            };
        }

        loop {
            if t.dead {
                break;
            }
            if *run_budget == 0 {
                break;
            }
            if budget == 0 {
                log::warn!(
                    "COB thread in {} exceeded its instruction budget; yielding",
                    program.script.functions[t.frames[0].function].name
                );
                t.wait = Wait::SleepUntil(host.now_ms() + 1);
                break;
            }
            budget -= 1;
            *run_budget -= 1;
            let Some(&opcode) = code.get(t.pc) else {
                t.dead = true;
                break;
            };
            if t.stack.len() > MAX_STACK {
                log::warn!("COB stack overflow; killing thread");
                t.dead = true;
                break;
            }
            match opcode {
                op::MOVE | op::TURN => {
                    let (piece, axis) = (arg!(1), arg!(2));
                    let target = pop!();
                    let speed = pop!();
                    t.pc += 3;
                    if let (Some(p), Ok(a)) = (self.piece_mut(piece), usize::try_from(axis)) && a < 3 {
                        if opcode == op::MOVE {
                            if speed == 0 {
                                p.offset[a] = target;
                                p.moves[a] = Motion::Idle;
                            } else {
                                p.moves[a] = Motion::Move { target, speed: speed.saturating_abs() };
                            }
                        } else if speed == 0 {
                            p.rotation[a] = wrap_angle(target);
                            p.turns[a] = Motion::Idle;
                        } else {
                            p.turns[a] = Motion::Turn { target: wrap_angle(target), speed: speed.saturating_abs() };
                        }
                    }
                }
                op::MOVE_NOW | op::TURN_NOW => {
                    let (piece, axis) = (arg!(1), arg!(2));
                    let target = pop!();
                    t.pc += 3;
                    if let (Some(p), Ok(a)) = (self.piece_mut(piece), usize::try_from(axis)) && a < 3 {
                        if opcode == op::MOVE_NOW {
                            p.offset[a] = target;
                            p.moves[a] = Motion::Idle;
                        } else {
                            p.rotation[a] = wrap_angle(target);
                            p.turns[a] = Motion::Idle;
                        }
                    }
                }
                op::SPIN => {
                    let (piece, axis) = (arg!(1), arg!(2));
                    // `spin <piece> around <axis> speed <s> accelerate <a>` pushes the
                    // acceleration first: the speed is on top (radars push 0 then theirs).
                    let speed = pop!();
                    let accel = pop!();
                    // (A speed of nothing as written: `speed <0>`, pushed just before.)
                    let written_zero = speed == 0 && t.pc >= 2 && code[t.pc - 2] == op::PUSH_CONSTANT && code[t.pc - 1] == 0;
                    t.pc += 3;
                    if let (Some(p), Ok(a)) = (self.piece_mut(piece), usize::try_from(axis)) && a < 3 {
                        let cur = match p.turns[a] {
                            Motion::Spin { speed, .. } | Motion::StopSpin { speed, .. } => speed,
                            _ => 0,
                        };
                        // `speed <0>` about an axis the piece is not turning about would change
                        // nothing; while it spins about another, that spin is what its author
                        // meant to stop: TA's radar vehicles (the Seer, the Informer) stop their
                        // dish, spinning about y, with `spin dish around z-axis speed <0>` in
                        // `Deactivate` (see FORMATS.md). A speed reckoned to 0 changes nothing.
                        let spinning = |m: &Motion| matches!(*m, Motion::Spin { speed, target_speed, .. } if speed != 0 || target_speed != 0);
                        let still = matches!(p.turns[a], Motion::Idle | Motion::Spin { speed: 0, target_speed: 0, .. });
                        if written_zero && still && p.turns.iter().any(spinning) {
                            for m in &mut p.turns {
                                if let Motion::Spin { speed, .. } = *m {
                                    *m = Motion::Spin { speed, target_speed: 0, accel };
                                }
                            }
                        } else {
                            p.turns[a] = Motion::Spin { speed: cur, target_speed: speed, accel };
                        }
                    }
                }
                op::STOP_SPIN => {
                    let (piece, axis) = (arg!(1), arg!(2));
                    let decel = pop!();
                    t.pc += 3;
                    if let (Some(p), Ok(a)) = (self.piece_mut(piece), usize::try_from(axis)) && a < 3 && let Motion::Spin { speed, .. } = p.turns[a] {
                        p.turns[a] = if decel == 0 { Motion::Idle } else { Motion::StopSpin { speed, decel } };
                    }
                }
                op::SHOW | op::HIDE => {
                    let piece = arg!(1);
                    t.pc += 2;
                    if let Some(p) = self.piece_mut(piece) {
                        p.visible = opcode == op::SHOW;
                    }
                }
                op::CACHE | op::DONT_CACHE => {
                    let piece = arg!(1);
                    t.pc += 2;
                    // (Nothing moves: the animation walk is not woken for it.)
                    if let Some(p) = usize::try_from(piece).ok().and_then(|i| self.pieces.get_mut(i)) {
                        p.textures_play = opcode == op::DONT_CACHE;
                    }
                }
                // (TA drew a `dont-shade` piece unlit; Reborn's lights and glows shine by their
                // textures instead.)
                op::SHADE | op::DONT_SHADE => t.pc += 2,
                op::EMIT_SFX => {
                    let piece = arg!(1);
                    let kind = pop!();
                    t.pc += 2;
                    if piece >= 0 {
                        host.emit_sfx(self, piece as usize, kind);
                    }
                }
                op::EXPLODE => {
                    let piece = arg!(1);
                    let flags = pop!();
                    t.pc += 2;
                    if piece >= 0 {
                        host.explode(self, piece as usize, flags);
                    }
                }
                op::WAIT_TURN | op::WAIT_MOVE => {
                    let (piece, axis) = (arg!(1), arg!(2));
                    t.pc += 3;
                    let busy = u32::try_from(piece).ok().filter(|&p| self.pieces.get(p as usize).is_some_and(|p| {
                        let a = axis.clamp(0, 2) as usize;
                        if opcode == op::WAIT_TURN { p.is_turning(a) } else { p.is_moving(a) }
                    }));
                    if let Some(piece) = busy {
                        let axis = axis.clamp(0, 2) as u8;
                        t.wait = if opcode == op::WAIT_TURN { Wait::Turn(piece, axis) } else { Wait::Move(piece, axis) };
                        break;
                    }
                }
                op::SLEEP => {
                    let ms = pop!().max(0) as i64;
                    t.pc += 1;
                    t.wait = Wait::SleepUntil(host.now_ms() + ms.max(1));
                    break;
                }
                op::PUSH_CONSTANT => {
                    t.stack.push(arg!(1));
                    t.pc += 2;
                }
                op::PUSH_LOCAL => {
                    let base = t.frames.last().map_or(0, |f| f.base);
                    let v = t.stack.get(base + arg!(1).max(0) as usize).copied().unwrap_or(0);
                    t.stack.push(v);
                    t.pc += 2;
                }
                op::PUSH_STATIC => {
                    let v = self.statics.get(arg!(1).max(0) as usize).copied().unwrap_or(0);
                    t.stack.push(v);
                    t.pc += 2;
                }
                op::CREATE_LOCAL => {
                    if let Some(f) = t.frames.last_mut() {
                        if f.params_unclaimed > 0 {
                            f.params_unclaimed -= 1;
                        } else {
                            t.stack.push(0);
                        }
                    }
                    t.pc += 1;
                }
                op::POP_LOCAL => {
                    let v = pop!();
                    let base = t.frames.last().map_or(0, |f| f.base);
                    let slot = base + arg!(1).max(0) as usize;
                    if slot < t.stack.len() {
                        t.stack[slot] = v;
                    }
                    t.pc += 2;
                }
                op::POP_STATIC => {
                    let v = pop!();
                    if let Some(s) = self.statics.get_mut(arg!(1).max(0) as usize) {
                        *s = v;
                    }
                    t.pc += 2;
                }
                op::POP_STACK => {
                    pop!();
                    t.pc += 1;
                }
                op::ADD | op::SUB | op::MUL | op::DIV | op::BITWISE_AND | op::BITWISE_OR | op::BITWISE_XOR => {
                    let b = pop!();
                    let a = pop!();
                    t.stack.push(match opcode {
                        op::ADD => a.wrapping_add(b),
                        op::SUB => a.wrapping_sub(b),
                        op::MUL => a.wrapping_mul(b),
                        op::DIV => {
                            if b == 0 {
                                0
                            } else {
                                a.wrapping_div(b)
                            }
                        }
                        op::BITWISE_AND => a & b,
                        op::BITWISE_OR => a | b,
                        _ => a ^ b,
                    });
                    t.pc += 1;
                }
                op::BITWISE_NOT => {
                    let a = pop!();
                    t.stack.push(!a);
                    t.pc += 1;
                }
                op::RAND => {
                    let hi = pop!();
                    let lo = pop!();
                    let v = if hi < lo { lo } else { host.random(lo, hi) };
                    t.stack.push(v);
                    t.pc += 1;
                }
                op::GET_UNIT_VALUE => {
                    let id = pop!();
                    let v = host.get_value(self, id, [0; 4]);
                    t.stack.push(v);
                    t.pc += 1;
                }
                op::GET => {
                    let a4 = pop!();
                    let a3 = pop!();
                    let a2 = pop!();
                    let a1 = pop!();
                    let id = pop!();
                    let v = host.get_value(self, id, [a1, a2, a3, a4]);
                    t.stack.push(v);
                    t.pc += 1;
                }
                op::SET_LESS
                | op::SET_LESS_OR_EQUAL
                | op::SET_GREATER
                | op::SET_GREATER_OR_EQUAL
                | op::SET_EQUAL
                | op::SET_NOT_EQUAL
                | op::LOGICAL_AND
                | op::LOGICAL_OR
                | op::LOGICAL_XOR => {
                    let b = pop!();
                    let a = pop!();
                    let r = match opcode {
                        op::SET_LESS => a < b,
                        op::SET_LESS_OR_EQUAL => a <= b,
                        op::SET_GREATER => a > b,
                        op::SET_GREATER_OR_EQUAL => a >= b,
                        op::SET_EQUAL => a == b,
                        op::SET_NOT_EQUAL => a != b,
                        op::LOGICAL_AND => a != 0 && b != 0,
                        op::LOGICAL_OR => a != 0 || b != 0,
                        _ => (a != 0) != (b != 0),
                    };
                    t.stack.push(r as i32);
                    t.pc += 1;
                }
                op::LOGICAL_NOT => {
                    let a = pop!();
                    t.stack.push((a == 0) as i32);
                    t.pc += 1;
                }
                op::START_SCRIPT => {
                    let (func, argc) = (arg!(1), arg!(2).clamp(0, 64) as usize);
                    t.pc += 3;
                    let split = t.stack.len().saturating_sub(argc);
                    let args = t.stack.split_off(split);
                    if let Ok(f) = usize::try_from(func) && f < program.script.functions.len() {
                        self.spawn(f, args, t.signal_mask, ThreadTag::None);
                    }
                }
                op::CALL_SCRIPT => {
                    let (func, argc) = (arg!(1), arg!(2).clamp(0, 64) as usize);
                    let return_pc = t.pc + 3;
                    match usize::try_from(func).ok().filter(|&f| f < program.script.functions.len()) {
                        Some(f) if t.frames.len() < MAX_CALL_DEPTH => {
                            let base = t.stack.len().saturating_sub(argc);
                            t.frames.push(Frame { function: f, return_pc, base, argc, params_unclaimed: argc });
                            t.pc = program.script.functions[f].offset as usize;
                        }
                        _ => {
                            let keep = t.stack.len().saturating_sub(argc);
                            t.stack.truncate(keep);
                            t.pc = return_pc;
                        }
                    }
                }
                op::JUMP => t.pc = arg!(1).max(0) as usize,
                op::JUMP_IF_FALSE => {
                    let v = pop!();
                    t.pc = if v == 0 { arg!(1).max(0) as usize } else { t.pc + 2 };
                }
                op::RETURN => {
                    let value = pop!();
                    let frame = t.frames.pop().expect("thread always has a frame");
                    if t.frames.is_empty() {
                        // (No one waits for an aim the engine gave up on.)
                        if !matches!(t.tag, ThreadTag::OldAim(_)) {
                            let end = (frame.base + frame.argc).min(t.stack.len());
                            let params = t.stack[frame.base.min(end)..end].to_vec();
                            self.finished.push((t.tag, CallResult { value, params }));
                        }
                        t.dead = true;
                        break;
                    }
                    t.stack.truncate(frame.base);
                    t.pc = frame.return_pc;
                }
                op::SIGNAL => {
                    let mask = pop!();
                    t.pc += 1;
                    for other in &mut self.threads {
                        if other.signal_mask & mask != 0 {
                            other.dead = true;
                        }
                    }
                }
                op::SET_SIGNAL_MASK => {
                    t.signal_mask = pop!();
                    t.pc += 1;
                }
                op::PLAY_SOUND => {
                    let sound = arg!(1);
                    t.pc += 2;
                    host.play_sound(sound);
                }
                op::SET_VALUE => {
                    let value = pop!();
                    let id = pop!();
                    host.set_value(id, value);
                    t.pc += 1;
                }
                op::ATTACH_UNIT => {
                    let _c = pop!();
                    let piece = pop!();
                    let unit = pop!();
                    host.attach_unit(unit, piece);
                    t.pc += 1;
                }
                op::DROP_UNIT => {
                    let unit = pop!();
                    host.drop_unit(unit);
                    t.pc += 1;
                }
                other => {
                    log::warn!("unknown COB opcode {other:#010x} at {}; killing thread", t.pc);
                    t.dead = true;
                    break;
                }
            }
        }
        self.threads[idx] = t;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::cob::CobFunction;

    struct TestHost {
        now: i64,
        values: Vec<(i32, i32)>,
    }

    impl CobHost for TestHost {
        fn now_ms(&self) -> i64 {
            self.now
        }
        fn random(&mut self, lo: i32, _hi: i32) -> i32 {
            lo
        }
        fn get_value(&mut self, _i: &CobInstance, id: i32, args: [i32; 4]) -> i32 {
            id * 100 + args[0]
        }
        fn set_value(&mut self, id: i32, value: i32) {
            self.values.push((id, value));
        }
        fn explode(&mut self, _i: &CobInstance, _p: usize, _f: i32) {}
        fn emit_sfx(&mut self, _i: &CobInstance, _p: usize, _k: i32) {}
    }

    fn program(functions: &[(&str, Vec<u32>)]) -> Arc<CobProgram> {
        let mut code = Vec::new();
        let mut funcs = Vec::new();
        for (name, body) in functions {
            funcs.push(CobFunction { name: name.to_string(), offset: code.len() as u32 });
            code.extend_from_slice(body);
        }
        let script = CobScript { functions: funcs, pieces: vec!["base".into(), "turret".into()], code, static_var_count: 2 };
        Arc::new(CobProgram::new(script, &["base".into(), "turret".into()]))
    }

    #[test]
    fn a_spin_to_nothing_about_another_axis_stops_the_spin() {
        // The Seer's: `Activate` spins the dish about y, `Deactivate` "stops" it about z.
        let p = program(&[
            ("Activate", vec![op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 18204, op::SPIN, 1, 1, op::PUSH_CONSTANT, 0, op::RETURN]),
            ("Deactivate", vec![op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 0, op::SPIN, 1, 2, op::PUSH_CONSTANT, 0, op::RETURN]),
            // A spin about a second axis, with a speed, turns about both.
            ("Tumble", vec![op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 9102, op::SPIN, 1, 0, op::PUSH_CONSTANT, 0, op::RETURN]),
            // A speed reckoned to nothing (a static at 0) about the idle axis changes nothing.
            ("Reckoned", vec![op::PUSH_CONSTANT, 0, op::PUSH_STATIC, 0, op::SPIN, 1, 2, op::PUSH_CONSTANT, 0, op::RETURN]),
        ]);
        let mut inst = CobInstance::new(p);
        let mut host = TestHost { now: 0, values: vec![] };
        let turned = |inst: &mut CobInstance, host: &mut TestHost| {
            let before = inst.pieces[1].rotation;
            for _ in 0..10 {
                inst.run(host);
                inst.animate();
            }
            let after = inst.pieces[1].rotation;
            [0, 1, 2].map(|a| before[a] != after[a])
        };
        inst.start("Activate", &[], ThreadTag::None);
        assert_eq!(turned(&mut inst, &mut host), [false, true, false]);
        inst.start("Reckoned", &[], ThreadTag::None);
        assert_eq!(turned(&mut inst, &mut host), [false, true, false], "a reckoned 0 changes nothing");
        inst.start("Tumble", &[], ThreadTag::None);
        assert_eq!(turned(&mut inst, &mut host), [true, true, false], "spins about two axes at once stand");
        inst.start("Deactivate", &[], ThreadTag::None);
        assert_eq!(turned(&mut inst, &mut host), [false; 3], "the dish comes to rest");
        inst.start("Activate", &[], ThreadTag::None);
        assert_eq!(turned(&mut inst, &mut host), [false, true, false], "and spins again");
    }

    #[test]
    fn dont_cache_plays_a_pieces_textures_and_cache_holds_them() {
        // A metal maker's way: `Go` lets the glow play, `Stop` holds it; neither moves a piece.
        let p = program(&[
            ("Go", vec![op::DONT_CACHE, 1, op::PUSH_CONSTANT, 0, op::RETURN]),
            ("Stop", vec![op::CACHE, 1, op::PUSH_CONSTANT, 0, op::RETURN]),
            ("Stray", vec![op::DONT_CACHE, 7, op::CACHE, u32::MAX, op::PUSH_CONSTANT, 0, op::RETURN]),
        ]);
        let mut inst = CobInstance::new(p);
        let mut host = TestHost { now: 0, values: vec![] };
        assert!(inst.pieces.iter().all(|p| !p.textures_play), "TA held a piece's textures until told");
        inst.start("Go", &[], ThreadTag::None);
        inst.run(&mut host);
        assert!(inst.pieces[1].textures_play && !inst.pieces[0].textures_play);
        assert!(inst.pieces.iter().all(|p| !p.is_animating()), "nothing moves for it");
        inst.start("Stop", &[], ThreadTag::None);
        inst.run(&mut host);
        assert!(!inst.pieces[1].textures_play);
        // Pieces the script does not have are passed over.
        inst.start("Stray", &[], ThreadTag::None);
        inst.run(&mut host);
        assert_eq!(inst.thread_count(), 0);
        assert!(inst.pieces.iter().all(|p| !p.textures_play));
    }

    #[test]
    fn recursive_start_script_cannot_run_forever_in_one_tick() {
        let p = program(&[("Fork", vec![op::START_SCRIPT, 0, 0, op::PUSH_CONSTANT, 0, op::RETURN])]);
        let mut inst = CobInstance::new(p);
        let mut host = TestHost { now: 0, values: vec![] };
        assert!(inst.start("Fork", &[], ThreadTag::None));
        inst.run(&mut host);
        assert_eq!(inst.thread_count(), 0, "the spawn chain stops at the per-run thread limit");
        assert!(inst.start("Fork", &[], ThreadTag::None), "the next run can reuse finished slots");
    }

    #[test]
    fn sleeping_threads_and_synchronous_calls_share_the_thread_limit() {
        let p = program(&[("Sleep", vec![op::PUSH_CONSTANT, 100, op::SLEEP, op::PUSH_CONSTANT, 0, op::RETURN])]);
        let mut inst = CobInstance::new(p);
        let mut host = TestHost { now: 0, values: vec![] };
        for _ in 0..MAX_THREADS {
            assert!(inst.start("Sleep", &[], ThreadTag::None));
        }
        assert!(!inst.start("Sleep", &[], ThreadTag::None));
        assert!(inst.call(&mut host, "Sleep", &[]).is_none());
        inst.run(&mut host);
        assert_eq!(inst.thread_count(), MAX_THREADS);
        host.now = 100;
        inst.run(&mut host);
        assert_eq!(inst.thread_count(), 0);
        assert!(inst.call(&mut host, "Sleep", &[]).is_some());
    }

    #[test]
    fn instruction_budget_is_shared_across_threads() {
        let p = program(&[("Loop", vec![op::JUMP, 0])]);
        let mut inst = CobInstance::new(p);
        let mut host = TestHost { now: 0, values: vec![] };
        for _ in 0..10 {
            assert!(inst.start("Loop", &[], ThreadTag::None));
        }
        inst.run(&mut host);
        assert!(inst.threads.iter().any(|t| matches!(t.wait, Wait::Ready)), "the run stops before every thread spends a full budget");
        assert_eq!(inst.thread_count(), 10);
    }

    /// A piece's turn about the y axis over one more tick.
    fn spin_step(inst: &mut CobInstance) -> i32 {
        let before = inst.pieces[1].rotation[1];
        inst.animate();
        wrap_angle(inst.pieces[1].rotation[1].wrapping_sub(before))
    }

    #[test]
    fn spins_take_the_speed_from_the_top_and_speed_up_by_the_frame() {
        // Go() { spin turret around y-axis speed <60> accelerate <1>; }: the acceleration is
        // pushed first, the speed last.
        let p = program(&[("Go", vec![op::PUSH_CONSTANT, 182, op::PUSH_CONSTANT, 10920, op::SPIN, 1, 1, op::PUSH_CONSTANT, 0, op::RETURN])]);
        let mut inst = CobInstance::new(p);
        let mut host = TestHost { now: 0, values: vec![] };
        inst.start("Go", &[], ThreadTag::None);
        inst.run(&mut host);
        // 182 faster every frame: 10920 a second (364 a tick) after 60 frames.
        let first = spin_step(&mut inst);
        assert!((1..=10).contains(&first), "starts slow ({first})");
        for _ in 0..70 {
            spin_step(&mut inst);
        }
        assert_eq!(spin_step(&mut inst), 364);

        // Without an acceleration, at full speed at once (radar dishes).
        let p = program(&[("Go", vec![op::PUSH_CONSTANT, 0, op::PUSH_CONSTANT, 10920, op::SPIN, 1, 1, op::PUSH_CONSTANT, 0, op::RETURN])]);
        let mut inst = CobInstance::new(p);
        inst.start("Go", &[], ThreadTag::None);
        inst.run(&mut host);
        assert_eq!(spin_step(&mut inst), 364);
    }

    #[test]
    fn query_returns_through_parameters() {
        // QueryPrimary(piece) { piece = 1; }  → create-local; push 1; pop-local 0; push 0; return
        let p = program(&[("QueryPrimary", vec![op::CREATE_LOCAL, op::PUSH_CONSTANT, 1, op::POP_LOCAL, 0, op::PUSH_CONSTANT, 0, op::RETURN])]);
        let mut inst = CobInstance::new(p);
        let mut host = TestHost { now: 0, values: vec![] };
        let r = inst.call(&mut host, "QueryPrimary", &[0]).unwrap();
        assert_eq!(r.params, vec![1]);
    }

    #[test]
    fn turn_waits_and_completes() {
        // Aim(heading) { turn turret to y-axis heading speed <90>; wait-for-turn; return 1; }
        let p = program(&[(
            "AimPrimary",
            vec![
                op::CREATE_LOCAL,
                op::PUSH_CONSTANT,
                16384, // speed: 90°/s
                op::PUSH_LOCAL,
                0, // target
                op::TURN,
                1,
                1,
                op::WAIT_TURN,
                1,
                1,
                op::PUSH_CONSTANT,
                1,
                op::RETURN,
            ],
        )]);
        let mut inst = CobInstance::new(p);
        let mut host = TestHost { now: 0, values: vec![] };
        inst.start("AimPrimary", &[8192], ThreadTag::Aim(0)); // 45°
        let mut ticks = 0;
        loop {
            inst.animate();
            inst.run(&mut host);
            host.now += 33;
            ticks += 1;
            if let Some(r) = inst.take_result(ThreadTag::Aim(0)) {
                assert_eq!(r.value, 1);
                break;
            }
            assert!(ticks < 100, "aim never finished");
        }
        // 45° at 90°/s ≈ 0.5 s ≈ 15 ticks.
        assert!((14..=17).contains(&ticks), "took {ticks} ticks");
        assert_eq!(inst.pieces[1].rotation[1], 8192);
    }

    #[test]
    fn signals_kill_masked_threads_and_sleep_works() {
        // Loop() { set-signal-mask 2; while(1) { sleep 100; } }  Kill() { signal 2; }
        let p = program(&[
            (
                "Loop",
                vec![op::PUSH_CONSTANT, 2, op::SET_SIGNAL_MASK, op::PUSH_CONSTANT, 100, op::SLEEP, op::JUMP, 3],
            ),
            ("Kill", vec![op::PUSH_CONSTANT, 2, op::SIGNAL, op::PUSH_CONSTANT, 0, op::RETURN]),
        ]);
        let mut inst = CobInstance::new(p);
        let mut host = TestHost { now: 0, values: vec![] };
        inst.start("Loop", &[], ThreadTag::None);
        inst.run(&mut host);
        assert_eq!(inst.thread_count(), 1);
        host.now = 500;
        inst.run(&mut host);
        assert_eq!(inst.thread_count(), 1);
        inst.start("Kill", &[], ThreadTag::None);
        inst.run(&mut host);
        assert_eq!(inst.thread_count(), 0);
    }

    #[test]
    fn a_superseded_aim_runs_on_unanswered_until_the_scripts_own_signal_ends_it() {
        // AimPrimary() { signal 4; set-signal-mask 4; static0 = static0 + 1;
        //   set-signal-mask 0; sleep 100; set-signal-mask 4;   (a step guarded from signals)
        //   sleep 100; return static0; }
        let aim = vec![
            op::PUSH_CONSTANT,
            4,
            op::SIGNAL,
            op::PUSH_CONSTANT,
            4,
            op::SET_SIGNAL_MASK,
            op::PUSH_STATIC,
            0,
            op::PUSH_CONSTANT,
            1,
            op::ADD,
            op::POP_STATIC,
            0,
            op::PUSH_CONSTANT,
            0,
            op::SET_SIGNAL_MASK,
            op::PUSH_CONSTANT,
            100,
            op::SLEEP,
            op::PUSH_CONSTANT,
            4,
            op::SET_SIGNAL_MASK,
            op::PUSH_CONSTANT,
            100,
            op::SLEEP,
            op::PUSH_STATIC,
            0,
            op::RETURN,
        ];
        let mut inst = CobInstance::new(program(&[("AimPrimary", aim)]));
        let mut host = TestHost { now: 0, values: vec![] };
        inst.start("AimPrimary", &[], ThreadTag::Aim(0));
        inst.run(&mut host);
        // A newer aim while the first is in its guarded step: the engine kills nothing, the
        // new aim's signal passes the guarded thread by.
        inst.supersede_aims(0);
        assert!(inst.is_running(ThreadTag::OldAim(0)) && !inst.is_running(ThreadTag::Aim(0)));
        inst.start("AimPrimary", &[], ThreadTag::Aim(0));
        host.now = 50;
        inst.run(&mut host);
        assert_eq!(inst.thread_count(), 2, "the guarded step is never cut short");
        // Out of it, the old aim is ended by the next signal; only the newest answers.
        host.now = 150;
        inst.run(&mut host);
        inst.supersede_aims(0);
        inst.start("AimPrimary", &[], ThreadTag::Aim(0));
        for now in [200, 300, 400, 500, 600] {
            host.now = now;
            inst.run(&mut host);
        }
        assert_eq!(inst.thread_count(), 0);
        let answer = inst.take_result(ThreadTag::Aim(0)).expect("the newest aim answers");
        assert_eq!(answer.value, 3, "three aims started");
        assert!(inst.take_result(ThreadTag::Aim(0)).is_none() && inst.take_result(ThreadTag::OldAim(0)).is_none(), "older aims' answers are dropped");
    }

    #[test]
    fn get_pops_four_args_then_id() {
        // push 7; push 5; push 0; push 0; push 0; get; pop-static 0; push 0; return
        let p = program(&[(
            "F",
            vec![
                op::PUSH_CONSTANT,
                7,
                op::PUSH_CONSTANT,
                5,
                op::PUSH_CONSTANT,
                0,
                op::PUSH_CONSTANT,
                0,
                op::PUSH_CONSTANT,
                0,
                op::GET,
                op::POP_STATIC,
                0,
                op::PUSH_CONSTANT,
                0,
                op::RETURN,
            ],
        )]);
        let mut inst = CobInstance::new(p);
        let mut host = TestHost { now: 0, values: vec![] };
        inst.call(&mut host, "F", &[]);
        assert_eq!(inst.statics[0], 705);
    }

    #[test]
    fn nested_call_passes_arguments() {
        // Outer() { call-script Inner(3); set 1 to static0 } Inner(x) { static0 = x * 2; }
        let p = program(&[
            (
                "Outer",
                vec![op::PUSH_CONSTANT, 3, op::CALL_SCRIPT, 1, 1, op::PUSH_CONSTANT, 0, op::RETURN],
            ),
            (
                "Inner",
                vec![
                    op::CREATE_LOCAL,
                    op::PUSH_LOCAL,
                    0,
                    op::PUSH_CONSTANT,
                    2,
                    op::MUL,
                    op::POP_STATIC,
                    0,
                    op::PUSH_CONSTANT,
                    0,
                    op::RETURN,
                ],
            ),
        ]);
        let mut inst = CobInstance::new(p);
        let mut host = TestHost { now: 0, values: vec![] };
        inst.call(&mut host, "Outer", &[]);
        assert_eq!(inst.statics[0], 6);
    }
}
