//! Walkers' legs kept in step with the ground they cover (for the eye alone).
//!
//! TA's walk scripts were made for its 640×480 view: their strides cover a fraction of the
//! ground the units move over (the Peewee's planted foot slides back with three quarters of the
//! body's travel), and their poses hold from keyframe to keyframe while the body glides on. Up
//! close, in 3D, the units seem to glide rather than walk. So the scene poses walkers' legs by
//! the ground they cover. Each walking type's gait is read from its own script the first time
//! one is drawn, a mod's units' too:
//!
//! * The script runs in a sandbox: `Create`, then `StartMoving` two seconds on, with a host
//!   that answers as a finished unit's would and time going on a millisecond at a time, so
//!   keyframes keep the times the script gives them (the game wakes a sleeping script at the
//!   next tick). Every change it makes to a piece is recorded as made: set at once, or turned or
//!   moved towards a target at a speed (both read back from a copy of the script run on), and
//!   reckoned exactly in time from there. A piece's turns and moves end when they arrive, and a
//!   thread waiting for them goes on then.
//! * The walk's cycle is where the changes repeat: the same changes at the same times one
//!   period on (its first cycle may differ, setting off). A script whose changes never repeat
//!   (random numbers, a crawl that is not a cycle) has no gait and keeps the game's animation.
//! * The gait's channels are those the cycle animates (not those animated standing still, a
//!   radar dish), less those any `Aim*`, `Fire*` or `Query*` function, or what they call or
//!   start, moves: turrets, torsos and arms that aim stay the game's. (Channel by channel: some
//!   units unfold as they aim, setting their legs' places, not the turns they walk with.) A
//!   script that walks otherwise while it aims (`walklegs`: the legs alone, sometimes in other
//!   steps) gets a second gait, recorded aiming, which the unit blends to while its script
//!   aims.
//! * Its stride is measured on the model as drawn: at each moment the planted foot is, of the
//!   pieces whose lowest point goes back and is down at the ground, the one going back fastest,
//!   and how far its lowest point goes back over the cycle is how far the body goes in a cycle
//!   while the feet stay put. A gait that plants no foot (no stance, a tiny stride) is no gait.
//! * The cycle is laid out by that distance, each stretch as long as the ground the planted
//!   foot covers in it but no shorter than [`FLOOR`] of its pace (so that nothing is skipped as
//!   one foot takes over from the other), in a table of poses: keyframes set at once are joined
//!   by straight lines, turns at a speed are as the script turns them (a pose it then sets at
//!   once, the next cycle's first, is reached over the time since its last change).
//!
//! A walking unit is posed from the table by how far it has come, between its poses at the
//! display's rate. Its legs blend in as it sets off and back into the game's pose as it stops;
//! it steps at most [`CAP`] times as fast as its script would, or [`BRISK`] cycles a second
//! (faster it would scurry: its feet slide a little instead). Units turning on the spot step round
//! as their feet are carried round ([`PIVOT`] of their radius from their middle), where TA slid
//! them round standing. Units too small on screen for their legs to show, and every unit with
//! `TA_WALK_SYNC=0`, keep the game's pose at no cost. Nothing here feeds back into the game: the
//! simulation, its scripts and its pieces' positions are untouched.

use std::sync::OnceLock;

use glam::{Mat4, Vec3};
use ta_formats::cob::{CobScript, op, operand_count};
use ta_formats::model3do::Model3do;
use ta_render::{Camera, CameraMode};
use ta_sim::cob::{CobHost, CobInstance, CobProgram, PieceAnim, ThreadTag};
use ta_sim::{Locomotion, Unit, UnitType};

use super::{PieceState, piece_matrices_between};

/// Poses in a gait's table, evenly over its cycle.
const POSES: usize = 64;
/// Fastest a gait plays: this many times its script's own pace, or [`BRISK`] cycles a second if
/// that is faster.
const CAP: f32 = 2.0;
/// Cycles a second that still read as walking (six steps a second for two legs).
const BRISK: f32 = 3.0;
/// Seconds the legs take to blend into the gait as a unit sets off, and back as it stops.
const BLEND_IN: f32 = 0.2;
const BLEND_OUT: f32 = 0.3;
/// Fewest pixels across a unit's footprint for its legs to be posed by the ground.
const CLOSE_PIXELS: f32 = 20.0;
/// A unit turning on the spot steps as if it walked as far as its feet are carried round: this
/// share of its radius from its middle, by the angle it turns. Turning slower than [`PIVOTING`]
/// (radians a second, a body's sway as it aims) it stands.
const PIVOT: f32 = 0.5;
const PIVOTING: f32 = 0.6;
/// The least ground a stretch of the cycle takes, as a share of the planted foot's mean pace:
/// where no foot goes back (as one takes over from the other) the cycle still takes some ground,
/// or it would be skipped at once (the feet slide there instead).
const FLOOR: f64 = 0.25;
/// The sandbox: `StartMoving` this long after `Create`, then this long recorded (milliseconds).
const SETTLE_MS: i64 = 2000;
const RECORD_MS: i64 = 10_000;
/// The shortest cycle (milliseconds).
const MIN_PERIOD: i64 = 120;
/// Longest a turn or move is followed to find where it goes (ticks).
const PROBE_TICKS: usize = 900;
/// Samples over a cycle for its stride.
const STRIDE_SAMPLES: usize = 256;
/// The shortest stride (world units a cycle) that plants a foot.
const MIN_STRIDE: f64 = 2.0;

/// `TA_WALK_SYNC=0` leaves every unit the game's pose (for comparisons).
fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("TA_WALK_SYNC").map_or(true, |v| v != "0"))
}

/// An angle difference (angle units, 65536 a turn) brought into -32768..32768.
fn wrap(a: f64) -> f64 {
    a - 65536.0 * (a / 65536.0).round()
}

fn wrap_f32(a: f32) -> f32 {
    a - 65536.0 * (a / 65536.0).round()
}

/// A piece's channel `ch` (0..3 moves along x, y, z; 3..6 turns about them) in its state.
fn channel(state: &PieceState, ch: usize) -> i32 {
    if ch < 3 { state.0[ch] } else { state.1[ch - 3] }
}

fn set_channel(state: &mut PieceState, ch: usize, v: i32) {
    if ch < 3 {
        state.0[ch] = v;
    } else {
        state.1[ch - 3] = v;
    }
}

/// A model's drawn vertices, piece by piece (model space, z negated as the renderer loads it).
pub(super) struct Shape {
    pieces: Vec<Vec<Vec3>>,
}

impl Shape {
    pub(super) fn of(m: &Model3do) -> Shape {
        let one = ta_formats::model3do::FIXED_ONE;
        let pieces = m
            .pieces
            .iter()
            .map(|p| {
                let mut used = vec![false; p.vertices.len()];
                for (k, prim) in p.primitives.iter().enumerate() {
                    if Some(k) == p.selection_primitive || prim.indices.len() < 3 {
                        continue;
                    }
                    for &i in &prim.indices {
                        if let Some(u) = used.get_mut(i as usize) {
                            *u = true;
                        }
                    }
                }
                p.vertices.iter().zip(used).filter(|(_, u)| *u).map(|(v, _)| Vec3::new(v[0] as f32 / one, v[1] as f32 / one, -(v[2] as f32) / one)).collect()
            })
            .collect();
        Shape { pieces }
    }
}

/// A gait: the legs' poses over a cycle, laid out by the ground covered.
pub(super) struct Gait {
    /// The channels it sets: the model piece and the channel (0..3 moves along x, y, z; 3..6
    /// turns about them).
    channels: Vec<(usize, usize)>,
    /// [`POSES`] poses, one after another, each the channels' values in script units (turns
    /// unwrapped along the cycle).
    poses: Vec<f32>,
    /// How far the body goes in a cycle while a planted foot stays put (world units).
    distance: f32,
    /// The cycle's length as the script times it (seconds).
    period: f32,
}

impl Gait {
    /// The value of channel `c` `phase` (0..1) of the way through the cycle.
    fn value(&self, c: usize, phase: f32) -> f32 {
        let n = self.channels.len();
        let x = phase.rem_euclid(1.0) * POSES as f32;
        let i = (x as usize).min(POSES - 1);
        let f = x - i as f32;
        let (a, b) = (self.poses[i * n + c], self.poses[(i + 1) % POSES * n + c]);
        if self.channels[c].1 >= 3 { a + wrap_f32(b - a) * f } else { a + (b - a) * f }
    }

    /// Whether two gaits set the same channels to the same values.
    fn same(&self, other: &Gait) -> bool {
        self.channels == other.channels && self.poses.iter().zip(&other.poses).all(|(a, b)| (a - b).abs() < 8.0)
    }
}

/// A walking type's gaits: walking, and walking as it aims when its script walks otherwise
/// then (`walklegs`: the legs alone, sometimes in other steps).
pub(super) struct Walk {
    walk: Gait,
    aiming: Option<Gait>,
    /// Every channel either sets, and its place in each.
    channels: Vec<(usize, usize)>,
    in_walk: Vec<Option<usize>>,
    in_aiming: Vec<Option<usize>>,
}

impl Walk {
    fn new(walk: Gait, aiming: Option<Gait>) -> Walk {
        let aiming = aiming.filter(|a| !a.same(&walk));
        let mut channels = walk.channels.clone();
        for c in aiming.iter().flat_map(|a| &a.channels) {
            if !channels.contains(c) {
                channels.push(*c);
            }
        }
        let place = |g: Option<&Gait>| channels.iter().map(|c| g.and_then(|g| g.channels.iter().position(|d| d == c))).collect();
        Walk { in_walk: place(Some(&walk)), in_aiming: place(aiming.as_ref()), channels, walk, aiming }
    }
}

/// Every walking type's gaits, read from its script the first time one is drawn.
#[derive(Default)]
pub(super) struct Gaits {
    /// By unit type: the model's vertices (ground units with a script).
    shapes: Vec<Option<Shape>>,
    walks: Vec<OnceLock<Option<Walk>>>,
}

impl Gaits {
    /// Keeps a ground unit's model for its stride.
    pub(super) fn add_model(&mut self, t: &UnitType, m: &Model3do) {
        if t.locomotion != Locomotion::Ground || t.script.is_none() {
            return;
        }
        if self.shapes.len() <= t.id {
            self.shapes.resize_with(t.id + 1, || None);
            self.walks.resize_with(t.id + 1, OnceLock::new);
        }
        self.shapes[t.id] = Some(Shape::of(m));
    }

    /// A type's gaits, if it walks (worked out on the first call).
    pub(super) fn get(&self, t: &UnitType) -> Option<&Walk> {
        if !enabled() {
            return None;
        }
        let shape = self.shapes.get(t.id)?.as_ref()?;
        self.walks
            .get(t.id)?
            .get_or_init(|| {
                let started = std::time::Instant::now();
                let walk = extract(t, shape);
                let took = started.elapsed().as_secs_f32() * 1000.0;
                match walk {
                    Ok(w) => {
                        let g = &w.walk;
                        log::debug!(
                            "{}: walks in step: {} channels, a {:.2} s cycle covering {:.1}{} (read in {took:.1} ms)",
                            t.name,
                            g.channels.len(),
                            g.period,
                            g.distance,
                            if w.aiming.is_some() { ", another as it aims" } else { "" }
                        );
                        Some(w)
                    }
                    Err(why) => {
                        log::debug!("{}: keeps its script's walk: {why} (read in {took:.1} ms)", t.name);
                        None
                    }
                }
            })
            .as_ref()
    }
}

/// Whether a unit's script walks as it does aiming (`walklegs`): it was asked to aim lately (its
/// `RestoreAfterDelay` stops aiming twice its longest reload after the last aim).
fn walks_aiming(sim: &ta_sim::Sim, u: &Unit, t: &UnitType) -> bool {
    let last = u.weapons.iter().map(|w| w.last_aim_tick).max().unwrap_or(0);
    let reload = t.weapons.iter().flatten().map(|&w| sim.weapon_types.get(w).reload_ticks as u64).max().unwrap_or(0);
    last > 0 && sim.tick.saturating_sub(last) <= 2 * reload + 15
}

/// How big the units look, to leave alone those too small for their legs to show.
#[derive(Clone, Copy)]
pub(super) struct View {
    classic: bool,
    zoom: f32,
    eye: Vec3,
    forward: Vec3,
    /// Pixels a world unit spans one unit in front of the eye.
    scale: f32,
}

impl View {
    pub(super) fn of(camera: &Camera) -> View {
        View {
            classic: camera.mode == CameraMode::Classic,
            zoom: camera.zoom,
            eye: camera.eye(),
            forward: -camera.back_dir(),
            scale: camera.viewport.y / (camera.fov_y * 0.5).tan().max(1e-3),
        }
    }

    /// Whether a unit `radius` in size at `centre` is big enough on screen to pose its legs.
    pub(super) fn close(&self, centre: Vec3, radius: f32) -> bool {
        let pixels = if self.classic {
            2.0 * radius * self.zoom
        } else {
            let distance = (centre - self.eye).dot(self.forward);
            if distance + radius <= 0.0 { 0.0 } else { radius * self.scale / distance.max(1.0) }
        };
        pixels >= CLOSE_PIXELS
    }
}

/// A unit's legs: where they are in their gait, and how far they follow it.
#[derive(Default)]
pub(super) struct Walker {
    /// 0..1 through the cycle.
    phase: f32,
    /// 0: the game's pose; 1: the gait's.
    weight: f32,
    /// 0: walking; 1: walking as it aims.
    aim: f32,
    /// When it was last posed (game seconds), and how far it had come and turned then.
    last: Option<(f64, f32, f32)>,
    /// The script states it was last posed with.
    before: Vec<PieceState>,
    now: Vec<PieceState>,
}

/// What posing walkers takes this frame: the gaits, the game, the moment drawn and the camera.
pub(super) struct Moment<'a> {
    gaits: &'a Gaits,
    sim: &'a ta_sim::Sim,
    alpha: f32,
    time: f64,
    view: Option<View>,
}

impl<'a> Moment<'a> {
    /// `alpha` of the way through the game's tick, `time` game seconds in.
    pub(super) fn new(gaits: &'a Gaits, sim: &'a ta_sim::Sim, alpha: f32, time: f64, camera: Option<&Camera>) -> Moment<'a> {
        Moment { gaits, sim, alpha, time, view: camera.map(View::of) }
    }

    /// Whether something at `at`, `radius` across, is big enough on screen for its pieces'
    /// motion to show (with no camera, as in the showroom, always).
    pub(super) fn close(&self, at: Vec3, radius: f32) -> bool {
        self.view.as_ref().is_none_or(|v| v.close(at, radius))
    }
}

impl Walker {
    /// The script states to pose unit `u` (at `at`, `travel` world units come and `turned`
    /// radians turned all told) with this frame, given its script's states at the tick before and
    /// this one: the game's, or with its legs in step with the ground.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn states<'a>(&'a mut self, m: &Moment, u: &Unit, t: &UnitType, at: Vec3, travel: f32, turned: f32, prev: &'a [PieceState], cur: &'a [PieceState]) -> (&'a [PieceState], &'a [PieceState]) {
        // (Only for walkers big enough on screen for their legs to show.)
        let walk = m.view.is_none_or(|v| v.close(at, t.radius.to_f32())).then(|| m.gaits.get(t)).flatten();
        let Some(walk) = walk else {
            self.weight = 0.0;
            self.last = None;
            return (prev, cur);
        };
        let (alpha, time) = (m.alpha, m.time);
        let aiming = walk.aiming.is_some() && walks_aiming(m.sim, u, t);
        // Since the last frame (nothing when posed twice in one, as the eye's unit is).
        let since = self.last.filter(|&(t, _, _)| time >= t && time - t < 1.0).map(|(t, d, r)| ((time - t) as f32, travel - d, turned - r));
        self.last = Some((time, travel, turned));
        // (Turning on the spot, it steps round: as far as its feet are carried round.)
        let pivot = since.map_or(0.0, |(_, _, turn)| turn.abs() * t.radius.to_f32() * PIVOT);
        let pivoting = since.is_some_and(|(dt, _, turn)| dt > 0.0 && turn.abs() >= PIVOTING * dt);
        let walking = (u.moving || pivoting) && u.is_complete() && u.carried_by.is_none() && !u.dead;
        let wanted = if walking { 1.0 } else { 0.0 };
        let aim_wanted = if aiming { 1.0 } else { 0.0 };
        match since {
            // Seen again after a while (or the game went back): as it is now.
            None => (self.weight, self.aim) = (wanted, aim_wanted),
            Some((dt, moved, _)) => {
                if self.weight == 0.0 && wanted > 0.0 {
                    // Setting off: from the start of the cycle, as its script starts it.
                    self.phase = 0.0;
                }
                let approach = |x: f32, to: f32| if to > x { (x + dt / BLEND_IN).min(to) } else { (x - dt / BLEND_OUT).max(to) };
                self.weight = approach(self.weight, wanted);
                self.aim = approach(self.aim, aim_wanted);
                // A step for the ground covered, but no faster than reads as walking.
                let (distance, period) = match &walk.aiming {
                    Some(a) => (walk.walk.distance + (a.distance - walk.walk.distance) * self.aim, walk.walk.period + (a.period - walk.walk.period) * self.aim),
                    None => (walk.walk.distance, walk.walk.period),
                };
                let most = (CAP / period.max(1e-3)).max(BRISK) * dt;
                let stepped = if moved.abs() >= pivot { moved } else { pivot };
                self.phase = (self.phase + (stepped / distance).clamp(-most, most)).rem_euclid(1.0);
            }
        }
        if self.weight <= 0.0 || prev.len() != cur.len() {
            return (prev, cur);
        }
        self.before.clear();
        self.before.extend_from_slice(prev);
        self.now.clear();
        self.now.extend_from_slice(cur);
        // (Eased: no jolt as the legs set off or come to rest.)
        let ease = |x: f32| x * x * (3.0 - 2.0 * x);
        let (w, aim, phase) = (ease(self.weight), ease(self.aim), self.phase);
        for (c, &(piece, ch)) in walk.channels.iter().enumerate() {
            let (Some(a), Some(b)) = (prev.get(piece), cur.get(piece)) else { continue };
            let (a, b) = (channel(a, ch), channel(b, ch));
            let turn = ch >= 3;
            // The game's value between its ticks (as `piece_matrices_between` has it)...
            let s = if turn { a as f32 + (b.wrapping_sub(a) as i16) as f32 * alpha } else { a as f32 + b.wrapping_sub(a) as f32 * alpha };
            let toward = |from: f32, to: f32, f: f32| if turn { from + wrap_f32(to - from) * f } else { from + (to - from) * f };
            // ... towards the gait's (as it walks, or as it walks aiming; the game's for what
            // the one it is in leaves alone).
            let mut g = walk.in_walk[c].map_or(s, |i| walk.walk.value(i, phase));
            if aim > 0.0
                && let Some(a) = &walk.aiming
            {
                g = toward(g, walk.in_aiming[c].map_or(s, |i| a.value(i, phase)), aim);
            }
            let v = toward(s, g, w).round() as i32;
            let v = if turn { v as i16 as i32 } else { v };
            set_channel(&mut self.before[piece], ch, v);
            set_channel(&mut self.now[piece], ch, v);
        }
        (&self.before, &self.now)
    }
}

// ---- Reading a gait from its script --------------------------------------------------------

/// A change a script makes to one of a piece's channels (times in milliseconds).
#[derive(Clone, Copy, Debug)]
enum Change {
    /// Set at once (`turn-now`, `move-now`, or a turn or move at speed 0).
    Now { at: f64, value: f64 },
    /// Turned or moved towards `to` at `speed` (units a second): the shortest way round for a
    /// turn, `to` unwrapped from where it starts.
    Toward { at: f64, to: f64, speed: f64 },
}

impl Change {
    fn at(&self) -> f64 {
        match *self {
            Change::Now { at, .. } | Change::Toward { at, .. } => at,
        }
    }
}

/// A turn or move under way: from `from` at `at` towards `to` at `speed` units a second.
#[derive(Clone, Copy, Debug)]
struct Motion {
    at: f64,
    from: f64,
    to: f64,
    speed: f64,
}

impl Motion {
    fn value(&self, t: f64) -> f64 {
        let (d, reach) = (self.to - self.from, self.speed * (t - self.at).max(0.0) / 1000.0);
        if reach >= d.abs() { self.to } else { self.from + reach * d.signum() }
    }

    fn done(&self, t: f64) -> bool {
        self.speed * (t - self.at) / 1000.0 >= (self.to - self.from).abs()
    }
}

/// One channel of a script piece over the sandbox's run: every change made to it.
#[derive(Clone, Debug, Default)]
struct Track {
    turn: bool,
    changes: Vec<Change>,
    /// Spun (or turned too slowly to see where to): no gait's.
    spun: bool,
    /// Its value at its last change and the motion it has been on since.
    value: f64,
    motion: Option<Motion>,
}

impl Track {
    /// Its value at `t`, from its last change on.
    fn current(&self, t: f64) -> f64 {
        self.motion.map_or(self.value, |m| m.value(t))
    }

    /// The script sets it to `value` (wrapped, for a turn) at `at`.
    fn set(&mut self, at: f64, value: f64) {
        let v = if self.turn {
            let c = self.current(at);
            c + wrap(value - c)
        } else {
            value
        };
        self.changes.push(Change::Now { at, value: v });
        self.value = v;
        self.motion = None;
    }

    /// The script sends it towards `target` (wrapped, for a turn) at `speed` at `at`.
    fn toward(&mut self, at: f64, target: f64, speed: f64) {
        let c = self.current(at);
        let to = if self.turn { c + wrap(target - c) } else { target };
        self.changes.push(Change::Toward { at, to, speed });
        self.value = c;
        self.motion = Some(Motion { at, from: c, to, speed });
    }

    /// Whether it is already on its way to `target` at `speed`.
    fn heading_for(&self, target: f64, speed: f64) -> bool {
        matches!(self.changes.last(), Some(&Change::Toward { to, speed: s, .. }) if self.motion.is_some() && s == speed && signature(to, self.turn) == signature(target, self.turn))
    }

    /// Its value at `t` (from the start of the run).
    fn at(&self, t: f64) -> f64 {
        let (mut v, mut motion): (f64, Option<Motion>) = (0.0, None);
        for c in &self.changes {
            if c.at() > t {
                break;
            }
            if let Some(m) = motion {
                v = m.value(c.at());
            }
            match *c {
                Change::Now { value, .. } => {
                    v = value;
                    motion = None;
                }
                Change::Toward { at, to, speed } => motion = Some(Motion { at, from: v, to, speed }),
            }
        }
        motion.map_or(v, |m| m.value(t))
    }
}

/// A value as compared between cycles: a turn's whatever way round it was reached.
fn signature(v: f64, turn: bool) -> i64 {
    (if turn { v.rem_euclid(65536.0) } else { v }).round() as i64
}

/// What the sandbox's host tells the script: a finished unit's answers, at the sandbox's time.
/// (A unit's standing pose is read in one too, see `still.rs`.)
pub(super) struct Sandbox {
    pub(super) now: i64,
    pub(super) seed: u32,
    pub(super) height: i32,
    pub(super) active: bool,
}

impl CobHost for Sandbox {
    fn now_ms(&self) -> i64 {
        self.now
    }

    fn random(&mut self, lo: i32, hi: i32) -> i32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        if hi <= lo { lo } else { (lo as i64 + (self.seed as i64).rem_euclid(hi as i64 - lo as i64 + 1)) as i32 }
    }

    fn get_value(&mut self, _: &CobInstance, id: i32, args: [i32; 4]) -> i32 {
        use ta_sim::host::value::*;
        match id {
            ACTIVATION => self.active as i32,
            STANDING_MOVE_ORDERS => 1,
            STANDING_FIRE_ORDERS => 2,
            HEALTH => 100,
            UNIT_HEIGHT => self.height,
            HYPOT => (args[0] as f64).hypot(args[1] as f64).min(i32::MAX as f64) as i32,
            // (0..65535, as the game's: `ta_sim::host::script_angle`.)
            ATAN => ta_sim::host::script_angle(((args[0] as f64).atan2(args[1] as f64) * 32768.0 / std::f64::consts::PI) as i32),
            // Built, not building, not busy, nothing in the way...
            _ => 0,
        }
    }

    fn set_value(&mut self, _: i32, _: i32) {}
    fn explode(&mut self, _: &CobInstance, _: usize, _: i32) {}
    fn emit_sfx(&mut self, _: &CobInstance, _: usize, _: i32) {}
}

fn anim_value(p: &PieceAnim, ch: usize) -> i32 {
    if ch < 3 { p.offset[ch] } else { p.rotation[ch - 3] }
}

fn set_anim_value(p: &mut PieceAnim, ch: usize, v: i32) {
    if ch < 3 {
        p.offset[ch] = v;
    } else {
        p.rotation[ch - 3] = v;
    }
}

fn anim_busy(p: &PieceAnim, ch: usize) -> bool {
    if ch < 3 { p.is_moving(ch) } else { p.is_turning(ch - 3) }
}

/// The script's pieces' channels over the sandbox's run.
struct Recording {
    /// By COB piece, its six channels.
    tracks: Vec<[Track; 6]>,
    /// Which pieces are shown at the end.
    visible: Vec<bool>,
}

/// Runs a script as the game runs a new unit's (`Create`, then `StartMoving` once settled) and
/// records every change it makes to its pieces, in exact time. `height`: the model's (`get
/// UNIT_HEIGHT`); `active`: switched on when built; `aim`: the function the game asks it to aim
/// with, to record it walking as it aims (asked from before it sets off and every so often
/// after, with a long reload: it never stops aiming).
fn record(program: &std::sync::Arc<CobProgram>, height: i32, active: bool, aim: Option<&str>) -> Recording {
    let mut inst = CobInstance::new(program.clone());
    let mut tracks: Vec<[Track; 6]> = (0..inst.pieces.len()).map(|_| std::array::from_fn(|ch| Track { turn: ch >= 3, ..Track::default() })).collect();
    let mut host = Sandbox { now: 0, seed: 0x2545_F491, height, active };
    for ms in 0..=SETTLE_MS + RECORD_MS {
        host.now = ms;
        let at = ms as f64;
        // Pieces whose turns and moves have all arrived come to rest in the script too (a
        // thread waiting for one goes on).
        for (piece, track) in inst.pieces.iter_mut().zip(&tracks) {
            if piece.is_animating() && track.iter().all(|tr| !tr.spun && tr.motion.is_none_or(|m| m.done(at))) {
                let (offset, rotation, visible) = (piece.offset, piece.rotation, piece.visible);
                *piece = PieceAnim::default();
                (piece.offset, piece.rotation, piece.visible) = (offset, rotation, visible);
            }
        }
        if ms == 0 {
            inst.start("Create", &[], ThreadTag::None);
        } else if ms == SETTLE_MS {
            inst.start("StartMoving", &[], ThreadTag::None);
        }
        if let Some(aim) = aim
            && ms >= SETTLE_MS - 1500
            && (ms - (SETTLE_MS - 1500)) % 1500 == 0
        {
            inst.start(aim, &[0, 0], ThreadTag::Aim(0));
        }
        if !inst.any_ready(ms) {
            continue;
        }
        // The pieces where their changes have them (the script reads them back, and a
        // change sets them from there).
        for (piece, track) in inst.pieces.iter_mut().zip(&tracks) {
            for (ch, tr) in track.iter().enumerate() {
                let v = tr.current(at).round();
                set_anim_value(piece, ch, if tr.turn { (v as i64) as i16 as i32 } else { v.clamp(i32::MIN as f64, i32::MAX as f64) as i32 });
            }
        }
        let before: Vec<([i32; 3], [i32; 3])> = inst.pieces.iter().map(|p| (p.offset, p.rotation)).collect();
        inst.run(&mut host);
        if ms == 0 {
            // (As the game tells a new unit: its longest reload, its being switched on.)
            let _ = inst.call(&mut host, "SetMaxReloadTime", &[if aim.is_some() { 30_000 } else { 1000 }]);
            if active {
                inst.start("Activate", &[], ThreadTag::None);
                inst.run(&mut host);
            }
        }
        // Set at once.
        for ((piece, track), (offset, rotation)) in inst.pieces.iter().zip(&mut tracks).zip(&before) {
            for (ch, tr) in track.iter_mut().enumerate() {
                let old = if ch < 3 { offset[ch] } else { rotation[ch - 3] };
                let new = anim_value(piece, ch);
                if new != old {
                    tr.set(at, new as f64);
                }
            }
        }
        probe(&inst, &mut tracks, at);
    }
    Recording { tracks, visible: inst.pieces.iter().map(|p| p.visible).collect() }
}

/// After a run of the script: what each channel it has turning or moving is on its way to, and
/// how fast (from a copy run on), and the turns and moves it stopped.
fn probe(inst: &CobInstance, tracks: &mut [[Track; 6]], at: f64) {
    let mut busy = Vec::new();
    for (p, (piece, track)) in inst.pieces.iter().zip(tracks.iter_mut()).enumerate() {
        for (ch, tr) in track.iter_mut().enumerate() {
            if anim_busy(piece, ch) {
                if !tr.spun {
                    busy.push((p, ch));
                }
            } else if tr.motion.is_some_and(|m| !m.done(at)) {
                // Stopped where it is (set at once to where it was).
                let v = anim_value(piece, ch) as f64;
                tr.set(at, v);
            }
        }
    }
    if busy.is_empty() {
        return;
    }
    let mut copy = inst.clone();
    let start: Vec<i32> = busy.iter().map(|&(p, ch)| anim_value(&inst.pieces[p], ch)).collect();
    let mut first = vec![0i32; busy.len()];
    let mut end: Vec<Option<(i32, bool)>> = vec![None; busy.len()];
    for k in 0..PROBE_TICKS {
        copy.animate();
        let mut all = true;
        for (i, &(p, ch)) in busy.iter().enumerate() {
            if end[i].is_some() {
                continue;
            }
            let piece = &copy.pieces[p];
            let v = anim_value(piece, ch);
            if k == 0 {
                first[i] = v;
            }
            if anim_busy(piece, ch) {
                all = false;
            } else {
                end[i] = Some((v, k == 0));
            }
        }
        if all {
            break;
        }
    }
    for (i, &(p, ch)) in busy.iter().enumerate() {
        let Some((target, at_once)) = end[i] else {
            tracks[p][ch].spun = true;
            continue;
        };
        // The game's speed: its step a tick, 30 a second (from far off when it arrived in one).
        let step = if at_once { far_step(inst, p, ch, target) } else { step(first[i], start[i], ch) };
        let speed = step as f64 * 30.0;
        let tr = &mut tracks[p][ch];
        if !tr.heading_for(target as f64, speed) {
            tr.toward(at, target as f64, speed);
        }
    }
}

/// How far a channel went in a tick.
fn step(after: i32, before: i32, ch: usize) -> i64 {
    if ch < 3 { (after as i64 - before as i64).abs() } else { (after.wrapping_sub(before) as i16 as i64).abs() }
}

/// A channel's step a tick from far from its target.
fn far_step(inst: &CobInstance, p: usize, ch: usize, target: i32) -> i64 {
    let mut copy = inst.clone();
    let far = if ch < 3 { target.saturating_sub(1 << 24) } else { target.wrapping_add(32767) as i16 as i32 };
    set_anim_value(&mut copy.pieces[p], ch, far);
    copy.animate();
    step(anim_value(&copy.pieces[p], ch), far, ch)
}

/// One change, as compared between cycles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Event {
    at: i64,
    channel: usize,
    kind: u8,
    a: i64,
    b: i64,
}

impl Recording {
    /// Every change from `from` on to what doesn't aim, in time order.
    fn events(&self, from: f64, aimed: &[[bool; 6]]) -> Vec<Event> {
        let mut out = Vec::new();
        for (p, track) in self.tracks.iter().enumerate() {
            for (ch, tr) in track.iter().enumerate().filter(|&(ch, tr)| !tr.spun && !aims(aimed, p, ch)) {
                for c in tr.changes.iter().filter(|c| c.at() >= from) {
                    let channel = p * 6 + ch;
                    out.push(match *c {
                        Change::Now { at, value } => Event { at: at as i64, channel, kind: 0, a: signature(value, tr.turn), b: 0 },
                        Change::Toward { at, to, speed } => Event { at: at as i64, channel, kind: 1, a: signature(to, tr.turn), b: speed.round() as i64 },
                    });
                }
            }
        }
        out.sort_by_key(|e| (e.at, e.channel, e.kind));
        out
    }
}

/// Where the changes repeat: the cycle's start and period (milliseconds), the earliest and
/// then the shortest that the run shows twice. (A walk's first cycle may differ from the
/// rest: it sets off from standing.)
fn find_cycle(events: &[Event], end: i64) -> Option<(i64, i64)> {
    let same = |a: &Event, b: &Event| (a.channel, a.kind, a.a, a.b) == (b.channel, b.kind, b.a, b.b);
    let window = |from: i64, to: i64| events.iter().filter(move |e| e.at >= from && e.at < to);
    let first = events.first()?.at;
    for (i0, e0) in events.iter().enumerate().take_while(|(_, e)| e.at < first + (end - first) / 3) {
        if i0 > 0 && events[i0 - 1].at == e0.at {
            continue;
        }
        for e1 in &events[i0 + 1..] {
            let period = e1.at - e0.at;
            if e0.at + 2 * period > end {
                break;
            }
            if period < MIN_PERIOD || !same(e0, e1) {
                continue;
            }
            let (a, b) = (window(e0.at, e0.at + period), window(e0.at + period, e0.at + 2 * period));
            if a.clone().count() == b.clone().count() && a.zip(b).all(|(x, y)| y.at - x.at == period && same(x, y)) {
                return Some((e0.at, period));
            }
        }
    }
    None
}

/// Whether channel `ch` of script piece `p` aims.
fn aims(aimed: &[[bool; 6]], p: usize, ch: usize) -> bool {
    aimed.get(p).is_some_and(|a| a[ch])
}

/// The channels of the script's pieces that aiming and firing move: those any `Aim*`, `Fire*`
/// or `Query*` function (or `TargetCleared`, `RockUnit`, `HitByWeapon`) moves, turns or spins, or
/// a function it calls or starts does. (Channel by channel: some units unfold as they aim,
/// setting their legs' places, not the turns they walk with.)
fn aimed_channels(script: &CobScript) -> Vec<[bool; 6]> {
    let mut out = vec![[false; 6]; script.pieces.len()];
    let mut seen = vec![false; script.functions.len()];
    for (i, f) in script.functions.iter().enumerate() {
        let name = f.name.to_ascii_lowercase();
        if ["aim", "fire", "query"].iter().any(|p| name.starts_with(p)) || ["targetcleared", "rockunit", "hitbyweapon"].contains(&name.as_str()) {
            channels_moved(script, i, &mut out, &mut seen);
        }
    }
    out
}

/// Marks the channels function `f` moves (following its jumps, and the functions it calls and
/// starts): by COB piece, moves along x, y, z, then turns about them.
pub(super) fn channels_moved(script: &CobScript, f: usize, out: &mut [[bool; 6]], seen: &mut [bool]) {
    if seen.get(f).is_none_or(|&s| s) {
        return;
    }
    seen[f] = true;
    let code = &script.code;
    let mut visited = vec![false; code.len()];
    let mut todo = vec![script.functions[f].offset as usize];
    while let Some(mut pc) = todo.pop() {
        while pc < code.len() && !visited[pc] {
            visited[pc] = true;
            let opcode = code[pc];
            let Some(n) = operand_count(opcode) else { break };
            let arg = |k: usize| code.get(pc + k).copied().unwrap_or(0) as usize;
            match opcode {
                op::MOVE | op::TURN | op::MOVE_NOW | op::TURN_NOW | op::SPIN | op::STOP_SPIN => {
                    let turn = !matches!(opcode, op::MOVE | op::MOVE_NOW);
                    if let Some(o) = out.get_mut(arg(1))
                        && arg(2) < 3
                    {
                        o[arg(2) + if turn { 3 } else { 0 }] = true;
                    }
                }
                op::CALL_SCRIPT | op::START_SCRIPT => channels_moved(script, arg(1), out, seen),
                op::JUMP => {
                    pc = arg(1);
                    continue;
                }
                op::JUMP_IF_FALSE => todo.push(arg(1)),
                op::RETURN => break,
                _ => {}
            }
            pc += 1 + n;
        }
    }
}

/// The cycle as recorded: each channel's value at any time in it.
struct Cycle<'a> {
    rec: &'a Recording,
    start: f64,
    period: f64,
    /// When the keyframes set at once fall (from the start, in order).
    keys: Vec<f64>,
    /// By COB piece and channel: whether the cycle animates it, and whether only by setting it
    /// at once (joined by straight lines between the keyframes).
    animated: Vec<[bool; 6]>,
    stepped: Vec<[bool; 6]>,
    /// By COB piece and channel turned or moved at a speed: where it is set at once in the
    /// cycle (from its start), by how much it jumps, and since when it last changed before.
    jumps: Vec<[Vec<(f64, f64, f64)>; 6]>,
}

impl Cycle<'_> {
    /// A channel's value `t` milliseconds into the cycle.
    fn value(&self, p: usize, ch: usize, t: f64) -> f64 {
        let tr = &self.rec.tracks[p][ch];
        if !self.animated[p][ch] {
            return tr.at(self.start);
        }
        if !self.stepped[p][ch] || self.keys.is_empty() {
            // As turned and moved, its jumps (a walk setting its first pose at once, where the
            // cycle before left off near it) spread over the time since its last change.
            let t = t.clamp(0.0, self.period);
            let mut v = tr.at(self.start + t);
            for &(at, jump, since) in &self.jumps[p][ch] {
                for t in [t, t - self.period] {
                    if t < at && t >= at - since {
                        v += jump * (1.0 - (at - t) / since);
                    }
                }
            }
            return v;
        }
        // Between the keyframes either side, the last before the cycle's first one being its
        // last a cycle before.
        let n = self.keys.len();
        let k = self.keys.partition_point(|&key| key <= t);
        let (k0, k1, t0, t1) = if k == 0 {
            (n - 1, 0, self.keys[n - 1] - self.period, self.keys[0])
        } else if k == n {
            (n - 1, 0, self.keys[n - 1], self.keys[0] + self.period)
        } else {
            (k - 1, k, self.keys[k - 1], self.keys[k])
        };
        let (a, b) = (tr.at(self.start + self.keys[k0]), tr.at(self.start + self.keys[k1]));
        let f = if t1 > t0 { ((t - t0) / (t1 - t0)).clamp(0.0, 1.0) } else { 0.0 };
        if tr.turn { a + wrap(b - a) * f } else { a + (b - a) * f }
    }

    /// Every model piece's script state `t` milliseconds into the cycle.
    fn states(&self, t: &UnitType, at: f64, out: &mut Vec<PieceState>) {
        out.clear();
        out.extend((0..t.model.pieces.len()).map(|mi| match t.model_to_cob.get(mi).copied().flatten().filter(|&c| c < self.rec.tracks.len()) {
            Some(c) => {
                let mut s: PieceState = ([0; 3], [0; 3], self.rec.visible[c]);
                for ch in 0..6 {
                    let v = self.value(c, ch, at).round();
                    set_channel(&mut s, ch, if ch >= 3 { (v as i64) as i16 as i32 } else { v.clamp(i32::MIN as f64, i32::MAX as f64) as i32 });
                }
                s
            }
            None => ([0; 3], [0; 3], true),
        }));
    }
}

/// Reads a type's gaits from its script and its model (see the module's notes): walking, and
/// walking as it aims.
fn extract(t: &UnitType, shape: &Shape) -> Result<Walk, String> {
    let program = t.script.as_ref().ok_or("no script")?;
    if program.function("StartMoving").is_none() {
        return Err("no StartMoving".into());
    }
    let aimed = aimed_channels(&program.script);
    let walk = gait(t, shape, program, &aimed, None)?;
    let aim = ["AimPrimary", "AimSecondary", "AimTertiary"].into_iter().find(|f| program.function(f).is_some());
    let aiming = aim.and_then(|f| gait(t, shape, program, &aimed, Some(f)).ok());
    Ok(Walk::new(walk, aiming))
}

/// A gait from the script run in the sandbox (`aim`: aiming with that function all along).
fn gait(t: &UnitType, shape: &Shape, program: &std::sync::Arc<CobProgram>, aimed: &[[bool; 6]], aim: Option<&str>) -> Result<Gait, String> {
    let rec = record(program, t.model.height.to_cob(), t.def.activate_when_built, aim);
    let cycle = cycle_of(&rec, aimed)?;
    let (n, period) = (rec.tracks.len(), cycle.period);
    // The gait: what the cycle animates.
    let (mut channels, mut cob_of) = (Vec::new(), Vec::new());
    for c in 0..n {
        let Some(mi) = program.piece_map.get(c).copied().flatten().filter(|&mi| mi < t.model.pieces.len()) else { continue };
        for ch in (0..6).filter(|&ch| cycle.animated[c][ch]) {
            channels.push((mi, ch));
            cob_of.push(c);
        }
    }
    if channels.is_empty() {
        return Err("its walk moves only what aims".into());
    }
    let Stride { covered, planted, .. } = stride(t, shape, &cycle);
    let samples = STRIDE_SAMPLES;
    let stride = covered[samples];
    if stride < MIN_STRIDE || planted * 4 < samples {
        return Err(format!("plants no foot (stride {stride:.1})"));
    }
    // The table: the cycle by the ground the planted foot covers, each stretch at least `FLOOR`
    // of the planted foot's mean pace.
    let least = FLOOR * stride / planted as f64;
    let mut progress = vec![0.0f64; samples + 1];
    for k in 0..samples {
        progress[k + 1] = progress[k] + (covered[k + 1] - covered[k]).max(least);
    }
    let distance = progress[samples];
    for p in &mut progress {
        *p /= distance;
    }
    let mut poses = Vec::with_capacity(POSES * channels.len());
    for j in 0..POSES {
        let p = j as f64 / POSES as f64;
        let k = progress.partition_point(|&x| x <= p).clamp(1, samples) - 1;
        let f = ((p - progress[k]) / (progress[k + 1] - progress[k]).max(1e-12)).clamp(0.0, 1.0);
        let at = period * (k as f64 + f) / samples as f64;
        for (c, &(_, ch)) in channels.iter().enumerate() {
            poses.push(cycle.value(cob_of[c], ch, at) as f32);
        }
    }
    // (Turns unwrapped from pose to pose.)
    let width = channels.len();
    for (c, &(_, ch)) in channels.iter().enumerate() {
        if ch >= 3 {
            for j in 1..POSES {
                let (a, b) = (poses[(j - 1) * width + c], poses[j * width + c]);
                poses[j * width + c] = a + wrap_f32(b - a);
            }
        }
    }
    Ok(Gait { channels, poses, distance: distance as f32, period: (period / 1000.0) as f32 })
}

/// The walk's cycle in a recording: where the changes to the pieces that don't aim repeat, and
/// which of them it animates (not what moves standing still, before the unit sets off).
fn cycle_of<'a>(rec: &'a Recording, aimed: &[[bool; 6]]) -> Result<Cycle<'a>, String> {
    let events = rec.events(SETTLE_MS as f64, aimed);
    if events.is_empty() {
        return Err("nothing moves as it walks".into());
    }
    let (start, period) = find_cycle(&events, SETTLE_MS + RECORD_MS).ok_or("its walk never repeats")?;
    let (start, period) = (start as f64, period as f64);
    let n = rec.tracks.len();
    let mut animated = vec![[false; 6]; n];
    let mut stepped = vec![[true; 6]; n];
    let mut jumps: Vec<[Vec<(f64, f64, f64)>; 6]> = (0..n).map(|_| Default::default()).collect();
    let mut keys: Vec<f64> = Vec::new();
    for (p, track) in rec.tracks.iter().enumerate() {
        for (ch, tr) in track.iter().enumerate() {
            if tr.spun || aims(aimed, p, ch) {
                continue;
            }
            let span = |from: f64, to: f64, k: usize| {
                let (lo, hi) = (0..=k).map(|i| tr.at(from + (to - from) * i as f64 / k as f64)).fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
                hi - lo
            };
            let standing = span(SETTLE_MS as f64 - 1000.0, SETTLE_MS as f64 - 1.0, 32) > 0.5;
            animated[p][ch] = !standing && span(start, start + period, 128) > 0.5;
            if !animated[p][ch] {
                continue;
            }
            let changes: Vec<&Change> = tr.changes.iter().filter(|c| c.at() >= start && c.at() < start + period).collect();
            for (i, c) in changes.iter().enumerate() {
                match **c {
                    Change::Now { at, value } => {
                        keys.push(at - start);
                        // (The change before it, the cycle's last for its first.)
                        let before = if i > 0 { changes[i - 1].at() } else { changes.last().map_or(at, |c| c.at()) - period };
                        let jump = value - tr.at(at - 1e-6);
                        let jump = if tr.turn { wrap(jump) } else { jump };
                        if jump.abs() > 0.5 && at > before {
                            jumps[p][ch].push((at - start, jump, at - before));
                        }
                    }
                    Change::Toward { .. } => stepped[p][ch] = false,
                }
            }
        }
    }
    keys.sort_by(f64::total_cmp);
    keys.dedup();
    Ok(Cycle { rec, start, period, keys, animated, stepped, jumps })
}

/// How the planted foot goes back over a cycle.
struct Stride {
    /// How far it has gone back by each of the samples (world units).
    covered: Vec<f64>,
    /// Samples at which a foot is planted (its lowest point going back).
    planted: usize,
    /// At each sample: the planted foot (none between strides), how low the model goes, and
    /// how far the foot goes back by the next (to look at).
    #[cfg_attr(not(test), allow(dead_code))]
    lows: Vec<(Option<usize>, f32, f64)>,
}

/// The stride, on the model as drawn. At each moment the feet on the ground are the pieces the
/// cycle moves whose lowest point goes back and is down at the ground (as low as the model
/// usually reaches: a foot swinging forwards can dip lower, some scripts putting a toe through
/// the ground), the planted one the fastest of them; the ground the body covers is how far its
/// lowest point goes back. (Some walks never lift a foot: their feet slide forwards as they
/// swing.)
fn stride(t: &UnitType, shape: &Shape, cycle: &Cycle) -> Stride {
    let samples = STRIDE_SAMPLES;
    let mut states = Vec::new();
    let mut matrices: Vec<Vec<Mat4>> = Vec::with_capacity(samples + 1);
    for k in 0..=samples {
        cycle.states(t, cycle.period * k as f64 / samples as f64, &mut states);
        let mut m = Vec::new();
        piece_matrices_between(t, &states, &states, 1.0, Mat4::IDENTITY, &mut m);
        matrices.push(m);
    }
    let pieces = t.model.pieces.len().min(shape.pieces.len());
    // Pieces the cycle moves (themselves or what carries them).
    let moves: Vec<bool> = (0..pieces).map(|i| matrices.iter().any(|m| m[i] != Mat4::ZERO && !m[i].abs_diff_eq(matrices[0][i], 1e-4))).collect();
    // Each piece's lowest point at each moment: how low, and how far it goes back by the next
    // (towards -z: the units face +z).
    let contact: Vec<Vec<Option<(f32, f64)>>> = (0..samples)
        .map(|k| {
            let (m, next) = (&matrices[k], &matrices[k + 1]);
            (0..pieces)
                .map(|i| {
                    if m[i] == Mat4::ZERO || next[i] == Mat4::ZERO {
                        return None;
                    }
                    let v = shape.pieces[i].iter().copied().min_by(|a, b| m[i].transform_point3(*a).y.total_cmp(&m[i].transform_point3(*b).y))?;
                    let (now, then) = (m[i].transform_point3(v), next[i].transform_point3(v));
                    Some((now.y, (now.z - then.z) as f64))
                })
                .collect()
        })
        .collect();
    // The ground: as low as the model usually reaches (the middle of its lowest points), and a
    // little higher (a heel or a toe down).
    let mut lowest: Vec<f32> = contact.iter().map(|c| c.iter().flatten().map(|l| l.0).fold(f32::MAX, f32::min)).collect();
    lowest.sort_by(f32::total_cmp);
    let ground = lowest[samples / 2] + 1.0 + 0.02 * t.model.height.to_f32();
    let down = |k: usize, i: usize| moves[i] && contact[k][i].is_some_and(|(y, back)| y <= ground && back > 0.0);
    let mut covered = vec![0.0f64; samples + 1];
    let mut planted = 0;
    let mut lows = Vec::with_capacity(samples);
    for k in 0..samples {
        // Of the feet down, the one going back fastest.
        let back = |i: usize| contact[k][i].map_or(0.0, |c| c.1);
        let foot = (0..pieces).filter(|&i| down(k, i)).max_by(|&a, &b| back(a).total_cmp(&back(b)));
        let back = foot.map_or(0.0, back);
        if foot.is_some() {
            planted += 1;
        }
        lows.push((foot, lowest[0], back));
        covered[k + 1] = covered[k] + back;
    }
    Stride { covered, planted, lows }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3Swizzles;

    /// A script of two pieces (`base`, `leg`): `Create` starts `walk`, which loops, doing
    /// `moving` while static 0 is set (by `StartMoving`), and sleeping a tenth of a second at a
    /// time otherwise.
    fn walker(moving: &[u32]) -> std::sync::Arc<CobProgram> {
        use ta_formats::cob::CobFunction;
        let mut code: Vec<u32> = Vec::new();
        let mut functions = Vec::new();
        functions.push(CobFunction { name: "Create".into(), offset: code.len() as u32 });
        code.extend([op::PUSH_CONSTANT, 0, op::POP_STATIC, 0, op::START_SCRIPT, 2, 0, op::PUSH_CONSTANT, 0, op::RETURN]);
        functions.push(CobFunction { name: "StartMoving".into(), offset: code.len() as u32 });
        code.extend([op::PUSH_CONSTANT, 1, op::POP_STATIC, 0, op::PUSH_CONSTANT, 0, op::RETURN]);
        functions.push(CobFunction { name: "walk".into(), offset: code.len() as u32 });
        let top = code.len() as u32;
        code.extend([op::PUSH_STATIC, 0, op::JUMP_IF_FALSE, 0]);
        let idle = code.len() - 1;
        code.extend_from_slice(moving);
        code.extend([op::JUMP, top]);
        code[idle] = code.len() as u32;
        code.extend([op::PUSH_CONSTANT, 100, op::SLEEP, op::JUMP, top]);
        let script = CobScript { functions, pieces: vec!["base".into(), "leg".into()], code, static_var_count: 1 };
        std::sync::Arc::new(CobProgram::new(script, &["base".into(), "leg".into()]))
    }

    /// A leg turned 45° out and back at 90° a second, each turn waited for: a cycle of two
    /// seconds (at the game's speed, 546 a tick), turning steadily from one end to the other.
    #[test]
    fn turns_waited_for_make_a_smooth_cycle() {
        let (speed, out, back) = (16384, 8192u32, (-8192i32) as u32);
        let program = walker(&[op::PUSH_CONSTANT, speed, op::PUSH_CONSTANT, out, op::TURN, 1, 0, op::WAIT_TURN, 1, 0, op::PUSH_CONSTANT, speed, op::PUSH_CONSTANT, back, op::TURN, 1, 0, op::WAIT_TURN, 1, 0]);
        let rec = record(&program, 0, false, None);
        let cycle = cycle_of(&rec, &[[false; 6]; 2]).unwrap();
        assert!((cycle.period - 2000.0).abs() <= 4.0, "a cycle of {} ms", cycle.period);
        assert!(cycle.animated[1][3] && !cycle.stepped[1][3], "the leg turns at a speed");
        assert!(!cycle.animated[0].iter().any(|&a| a) && !cycle.animated[1][4], "nothing else moves");
        let at = |t: f64| cycle.value(1, 3, t);
        let p = cycle.period;
        assert!((at(0.0).abs() - 8192.0).abs() < 40.0, "from one end ({})", at(0.0));
        assert!(at(p * 0.25).abs() < 60.0, "through the middle ({})", at(p * 0.25));
        assert!((at(p * 0.5) + at(0.0)).abs() < 60.0, "to the other end ({})", at(p * 0.5));
        assert!((at(p * 0.125) - at(0.0) * 0.5).abs() < 60.0, "steadily ({})", at(p * 0.125));
    }

    /// Keyframes set at once and held for 90, 70 and 50 ms keep those times (the game wakes a
    /// sleeping script on the next tick: 100, 100 and 66 ms), joined by straight lines.
    #[test]
    fn keyframes_keep_their_times() {
        let program = walker(&[
            op::PUSH_CONSTANT,
            4096,
            op::TURN_NOW,
            1,
            0,
            op::PUSH_CONSTANT,
            90,
            op::SLEEP,
            op::PUSH_CONSTANT,
            (-4096i32) as u32,
            op::TURN_NOW,
            1,
            0,
            op::PUSH_CONSTANT,
            70,
            op::SLEEP,
            op::PUSH_CONSTANT,
            0,
            op::TURN_NOW,
            1,
            0,
            op::PUSH_CONSTANT,
            50,
            op::SLEEP,
        ]);
        let rec = record(&program, 0, false, None);
        let cycle = cycle_of(&rec, &[[false; 6]; 2]).unwrap();
        assert_eq!(cycle.period, 210.0);
        assert!(cycle.stepped[1][3]);
        let gaps: Vec<f64> = (0..3).map(|k| if k < 2 { cycle.keys[k + 1] - cycle.keys[k] } else { cycle.keys[0] + 210.0 - cycle.keys[2] }).collect();
        assert!([[90.0, 70.0, 50.0], [70.0, 50.0, 90.0], [50.0, 90.0, 70.0]].iter().any(|g| g[..] == gaps[..]), "keyframes {:?}", cycle.keys);
        // Halfway between +4096 and -4096 (from the keyframe held 90 ms to the next).
        let k = cycle.keys.iter().position(|&key| cycle.value(1, 3, key) == 4096.0).unwrap();
        assert_eq!(cycle.value(1, 3, cycle.keys[k] + 45.0), 0.0);
    }

    /// The install's units and models (none without the install).
    fn install() -> Option<(ta_vfs::Vfs, std::sync::Arc<ta_sim::UnitTypes>)> {
        let install = ta_vfs::install::discover(None).ok()?;
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").ok()?;
        let data = std::sync::Arc::new(ta_data::GameData::load(&vfs));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
        Some((vfs, types))
    }

    fn gaits(vfs: &ta_vfs::Vfs, types: &ta_sim::UnitTypes) -> Gaits {
        let mut gaits = Gaits::default();
        for t in &types.types {
            let model = vfs.read_opt(&format!("objects3d/{}.3do", t.def.object.to_ascii_lowercase())).and_then(|b| Model3do::parse(&b).ok());
            if let Some(m) = model {
                gaits.add_model(t, &m);
            }
        }
        gaits
    }

    /// Whether `walklegs` (walking while aiming) moves the pieces that don't aim as `walk` does,
    /// keyframe by keyframe, in every script of the install that has both.
    #[test]
    #[ignore]
    fn walklegs_against_walk() {
        let Some((_, types)) = install() else { return };
        // A function's moves and turns of pieces that don't aim, and its sleeps, in code order.
        let steps = |s: &CobScript, f: usize, aimed: &[[bool; 6]]| {
            let code = &s.code;
            let mut pc = s.functions[f].offset as usize;
            let mut out = Vec::new();
            let mut last = 0u32;
            while let Some(&opcode) = code.get(pc) {
                let Some(n) = operand_count(opcode) else { break };
                match opcode {
                    op::PUSH_CONSTANT => last = code[pc + 1],
                    op::MOVE | op::TURN | op::MOVE_NOW | op::TURN_NOW if !aims(aimed, code[pc + 1] as usize, code[pc + 2] as usize % 3 + if matches!(opcode, op::TURN | op::TURN_NOW) { 3 } else { 0 }) => out.push((opcode, code[pc + 1], code[pc + 2], last)),
                    op::SLEEP => out.push((opcode, 0, 0, last)),
                    op::RETURN => break,
                    _ => {}
                }
                pc += 1 + n;
            }
            out
        };
        let mut seen = std::collections::BTreeSet::new();
        for t in &types.types {
            let Some(p) = &t.script else { continue };
            let s = &p.script;
            let (Some(walk), Some(legs)) = (s.function("walk"), s.function("walklegs")) else { continue };
            if !seen.insert(t.def.object.to_ascii_lowercase()) {
                continue;
            }
            let aimed = aimed_channels(s);
            let (a, b) = (steps(s, walk, &aimed), steps(s, legs, &aimed));
            // The pieces walklegs moves, as walk moves them; and those walk alone moves.
            let legs_pieces: std::collections::BTreeSet<u32> = b.iter().filter(|x| x.0 != op::SLEEP).map(|x| x.1).collect();
            let a_legs: Vec<_> = a.iter().filter(|x| x.0 == op::SLEEP || legs_pieces.contains(&x.1)).copied().collect();
            let only_walk: std::collections::BTreeSet<&str> = a.iter().filter(|x| x.0 != op::SLEEP && !legs_pieces.contains(&x.1)).map(|x| s.pieces[x.1 as usize].as_str()).collect();
            println!(
                "{:9} legs {}; walk alone moves {:?}",
                t.name,
                if a_legs == b { "the same".to_string() } else { format!("differ ({} against {} steps)", a_legs.len(), b.len()) },
                only_walk
            );
        }
    }

    /// `TA_UNITS=a,b`: what the sandbox makes of their walks, sample by sample.
    #[test]
    #[ignore]
    fn dump_walks() {
        let Some((vfs, types)) = install() else { return };
        let gaits = gaits(&vfs, &types);
        for name in std::env::var("TA_UNITS").unwrap_or("CORCAN".into()).split(',') {
            let t = types.by_name(name).unwrap();
            let program = t.script.as_ref().unwrap();
            let aimed = aimed_channels(&program.script);
            let aim = std::env::var("TA_AIM").ok();
            let rec = record(program, t.model.height.to_cob(), t.def.activate_when_built, aim.as_deref());
            let events = rec.events(SETTLE_MS as f64, &aimed);
            println!("{name}: {} events after StartMoving; first at {:?}", events.len(), events.first().map(|e| e.at));
            for e in events.iter().take(40) {
                let (p, ch) = (e.channel / 6, e.channel % 6);
                println!("  {:6} {:>10} ch{ch} {} {} {}", e.at, program.script.pieces[p], if e.kind == 0 { "now" } else { "toward" }, e.a, e.b);
            }
            let cycle = find_cycle(&events, SETTLE_MS + RECORD_MS);
            println!("  cycle {cycle:?}");
            let mut times: Vec<(i64, usize)> = Vec::new();
            for e in &events {
                match times.last_mut() {
                    Some((at, n)) if *at == e.at => *n += 1,
                    _ => times.push((e.at, 1)),
                }
            }
            println!("  event times: {}", times.iter().take(60).map(|(t, n)| format!("{t}({n})")).collect::<Vec<_>>().join(" "));
            println!("  aimed: {}", program.script.pieces.iter().zip(&aimed).filter(|(_, a)| a.iter().any(|&x| x)).map(|(p, a)| format!("{p}{:?}", (0..6).filter(|&c| a[c]).collect::<Vec<_>>())).collect::<Vec<_>>().join(", "));
            for (p, track) in rec.tracks.iter().enumerate() {
                let spun: Vec<usize> = (0..6).filter(|&ch| track[ch].spun).collect();
                if !spun.is_empty() {
                    println!("  {} spun {spun:?}", program.script.pieces[p]);
                }
            }
            if let Ok(cycle) = cycle_of(&rec, &aimed) {
                println!("  keys {:?}", cycle.keys);
                for (p, track) in rec.tracks.iter().enumerate() {
                    for (ch, tr) in track.iter().enumerate() {
                        if !tr.changes.is_empty() {
                            let model = program.piece_map[p];
                            println!(
                                "  {:>10}.{ch} -> {:?} animated {} stepped {} at start {:.0} mid {:.0} | {} changes",
                                program.script.pieces[p],
                                model,
                                cycle.animated[p][ch],
                                cycle.stepped[p][ch],
                                cycle.value(p, ch, 0.0),
                                cycle.value(p, ch, cycle.period / 2.0),
                                tr.changes.len()
                            );
                        }
                    }
                }
                let s = stride(t, gaits.shapes[t.id].as_ref().unwrap(), &cycle);
                println!("  stride {:.1}, planted {} of {STRIDE_SAMPLES}", s.covered[STRIDE_SAMPLES], s.planted);
                let feet: Vec<usize> = std::env::var("TA_FEET").unwrap_or("lfoot,rfoot".into()).split(',').filter_map(|f| t.model.piece_index(f)).collect();
                let mut states = Vec::new();
                let mut m = Vec::new();
                let step: usize = std::env::var("TA_STEP").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
                let shape = gaits.shapes[t.id].as_ref().unwrap();
                for (k, (piece, y, back)) in s.lows.iter().enumerate().step_by(step) {
                    cycle.states(t, cycle.period * k as f64 / STRIDE_SAMPLES as f64, &mut states);
                    piece_matrices_between(t, &states, &states, 1.0, Mat4::IDENTITY, &mut m);
                    let at: Vec<String> = feet
                        .iter()
                        .map(|&f| {
                            let low = shape.pieces[f].iter().map(|&v| m[f].transform_point3(v)).min_by(|a, b| a.y.total_cmp(&b.y)).unwrap_or(Vec3::ZERO);
                            format!("[origin z {:6.2}, lowest point z {:6.2} y {:5.2}]", m[f].transform_point3(Vec3::ZERO).z, low.z, low.y)
                        })
                        .collect();
                    println!("    {k:3}: lowest {:>10} at {y:6.2}, back {back:5.2} | {}", piece.map_or("-", |i| t.model.pieces[i].name.as_str()), at.join(" "));
                }
            }
            match extract(t, gaits.shapes[t.id].as_ref().unwrap()) {
                Ok(w) => {
                    println!("  gait: {:?}", w.walk.channels.iter().map(|&(mi, ch)| format!("{}.{ch}", t.model.pieces[mi].name)).collect::<Vec<_>>());
                    if let Some(a) = &w.aiming {
                        println!("  aiming: {:?}, covers {:.1} in {:.3} s", a.channels.iter().map(|&(mi, ch)| format!("{}.{ch}", t.model.pieces[mi].name)).collect::<Vec<_>>(), a.distance, a.period);
                    }
                }
                Err(e) => println!("  no gait: {e}"),
            }
        }
    }

    /// How a walker's feet move as the scene draws it: it walks east across a map at full speed,
    /// drawn at four frames a tick, with its script's walk (0) and in step (1).
    struct Steps {
        /// How far its planted foot slides over the ground, along the way it walks and across it,
        /// as shares of the ground its body covers (0: the foot stays where it was put down; 1:
        /// the unit glides). The planted foot: of its feet on the ground (solid pieces nothing
        /// that aims carries, reaching about as low as they do), the one whose lowest point
        /// slides least.
        along: [f64; 2],
        across: [f64; 2],
        /// The same measured as the scratch test that found the gliding did: the lowest solid
        /// piece's origin, along and across.
        origin: [f64; 2],
        /// How smoothly it moves: the most any point of it moves from one frame to the next
        /// against the body (the 99th percentile over the frames).
        jumps: [f32; 2],
    }

    fn steps(vfs: &ta_vfs::Vfs, types: &std::sync::Arc<ta_sim::UnitTypes>, data: &std::sync::Arc<ta_data::GameData>, map: (&ta_formats::tnt::TntMap, &ta_data::MapInfo), name: &str) -> Option<Steps> {
        use glam::Vec2;
        use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Sim};
        let t = types.by_name(name)?;
        let model = Model3do::parse(&vfs.read(&format!("objects3d/{}.3do", t.def.object.to_ascii_lowercase())).ok()?).ok()?;
        let shape = Shape::of(&model);
        let players: Vec<PlayerSetup> = (0..2).map(|i| PlayerSetup { side: ["ARM", "CORE"][i].into(), color: i as u8, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
        let mut sim = Sim::new(types.clone(), data.clone(), map.0, map.1, 0, &players, 1);
        let (w, h) = sim.world.size_px();
        let z = h.to_int() / 2;
        let id = sim.spawn_unit(t.id, 0, FxVec3::from_ints(300, 0, z), 16384, true)?;
        sim.step(&[Command::Order { player: 0, units: vec![id], order: Order::Move(FxVec3::from_ints(w.to_int() - 300, 0, z)), queue: false }]);
        // The game's walk, and in step.
        let mut scenes = [super::super::Scene::default(), super::super::Scene::default()];
        for (k, s) in scenes.iter_mut().enumerate() {
            s.models.insert(t.id, Some(0));
            if k == 1 {
                s.walk.add_model(t, &model);
            }
        }
        // Feet are what nothing that aims carries (arms and guns hang low on some): pieces that
        // aim and that the gait leaves alone.
        let aimed = aimed_channels(&t.script.as_ref()?.script);
        let driven: Vec<usize> = scenes[1].walk.get(t).map(|w| w.channels.iter().map(|c| c.0).collect()).unwrap_or_default();
        let aims = |mi: usize| !driven.contains(&mi) && t.model_to_cob.get(mi).copied().flatten().is_some_and(|c| aimed.get(c).is_some_and(|a| a.iter().any(|&x| x)));
        let foot: Vec<bool> = (0..t.model.pieces.len())
            .map(|mut mi| loop {
                if aims(mi) {
                    break false;
                }
                match t.model.pieces[mi].parent {
                    Some(p) => mi = p,
                    None => break true,
                }
            })
            .collect();
        let ground = 1.0 + 0.02 * t.model.height.to_f32();
        let mut last: [Option<(Vec3, Vec<Mat4>)>; 2] = [None, None];
        let (mut body, mut along, mut across, mut origin) = ([0.0f64; 2], [0.0f64; 2], [0.0f64; 2], [0.0f64; 2]);
        let mut jumps: [Vec<f32>; 2] = Default::default();
        for tick in 0..30 * 12 {
            sim.step(&[]);
            for s in &mut scenes {
                s.after_tick(&sim);
            }
            for f in 0..4 {
                let alpha = f as f32 / 4.0;
                for (k, s) in scenes.iter_mut().enumerate() {
                    let pieces = s.instances(&sim, alpha, &|_| false, &|_| true, (Vec2::splat(-1e6), Vec2::splat(1e6)), 0.0).first()?.pieces.to_vec();
                    let at = s.unit_position(id, alpha)?;
                    // (From the third second on: up to speed.)
                    if tick >= 90
                        && let Some((at0, pieces0)) = &last[k]
                    {
                        let solid = |i: usize| i < shape.pieces.len() && !shape.pieces[i].is_empty() && pieces[i] != Mat4::ZERO && pieces0[i] != Mat4::ZERO;
                        let moved = |i: usize, v: Vec3| pieces[i].transform_point3(v) - pieces0[i].transform_point3(v);
                        let lowest = |i: usize| shape.pieces[i].iter().copied().min_by(|a, b| pieces[i].transform_point3(*a).y.total_cmp(&pieces[i].transform_point3(*b).y)).unwrap_or(Vec3::ZERO);
                        let feet: Vec<usize> = (0..pieces.len()).filter(|&i| solid(i) && foot[i]).collect();
                        let low = feet.iter().map(|&i| pieces[i].transform_point3(lowest(i)).y).fold(f32::MAX, f32::min);
                        let planted = feet
                            .iter()
                            .copied()
                            .filter(|&i| pieces[i].transform_point3(lowest(i)).y <= low + ground)
                            .map(|i| (i, moved(i, lowest(i))))
                            .min_by(|a, b| a.1.xz().length().total_cmp(&b.1.xz().length()));
                        let step = (at - *at0).xz().length() as f64;
                        let trace = std::env::var("TA_TRACE").unwrap_or_default();
                        if trace == "feet" && tick < 100 {
                            let on: Vec<String> = feet
                                .iter()
                                .map(|&i| {
                                    let (y, slid) = (pieces[i].transform_point3(lowest(i)).y - low, moved(i, lowest(i)));
                                    format!("{} {y:.2} {:+.2},{:+.2}", t.model.pieces[i].name, slid.x, slid.z)
                                })
                                .collect();
                            println!("{k} {tick:3}.{f} body {:+.2} | {}", at.x - at0.x, on.join(" | "));
                        }
                        if let Some((i, slid)) = planted {
                            body[k] += step;
                            along[k] += slid.x.abs() as f64;
                            across[k] += slid.z.abs() as f64;
                            if k == 1 && (trace == "all" && slid.xz().length() > 0.05 || trace == "1" && tick < 100) {
                                let w = &s.visuals[id.slot as usize].as_ref().unwrap().walk;
                                println!("{tick:3}.{f} phase {:.3} weight {:.2} planted {:>8} slides {:+6.3} {:+6.3} body {:+6.3}", w.phase, w.weight, t.model.pieces[i].name, slid.x, slid.z, at.x - at0.x);
                            }
                        }
                        if let Some(i) = (0..pieces.len()).filter(|&i| solid(i)).min_by(|&a, &b| pieces[a].w_axis.y.total_cmp(&pieces[b].w_axis.y)) {
                            origin[k] += moved(i, Vec3::ZERO).xz().length() as f64;
                        }
                        let mut most = 0.0f32;
                        for i in (0..pieces.len()).filter(|&i| solid(i)) {
                            for &v in &shape.pieces[i] {
                                most = most.max((moved(i, v) - (at - *at0)).length());
                            }
                        }
                        jumps[k].push(most);
                    }
                    last[k] = Some((at, pieces));
                }
            }
        }
        let share = |x: [f64; 2]| [x[0] / body[0].max(1e-6), x[1] / body[1].max(1e-6)];
        let p99 = |v: &mut Vec<f32>| {
            v.sort_by(f32::total_cmp);
            v.get(v.len() * 99 / 100).copied().unwrap_or(0.0)
        };
        Some(Steps { along: share(along), across: share(across), origin: share(origin), jumps: [p99(&mut jumps[0]), p99(&mut jumps[1])] })
    }

    /// Walkers drawn in step keep their planted feet put (the Peewee's slid back with three
    /// quarters of its body's travel). `TA_UNITS=a,b` measures others; `TA_TRACE=1` (or `all`)
    /// follows the planted foot frame by frame, `TA_TRACE=feet` every foot (how high, how far it
    /// slides) both ways.
    #[test]
    fn planted_feet_stay_put() {
        use ta_formats::tdf;
        let Some((vfs, types)) = install() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let data = std::sync::Arc::new(ta_data::GameData::load(&vfs));
        let name = "Painted Desert";
        let map = ta_formats::tnt::TntMap::parse(&vfs.read(&format!("maps/{name}.tnt")).unwrap()).unwrap();
        let info = ta_data::MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{name}.ota")).unwrap()));
        let units = std::env::var("TA_UNITS").unwrap_or("ARMPW,CORAK,ARMHAM,CORTHUD,ARMCOM,CORCOM,ARMZEUS,CORPYRO,ARMJETH,CORSTORM".into());
        for unit in units.split(',') {
            let Some(s) = steps(&vfs, &types, &data, (&map, &info), unit) else { continue };
            let pc = |x: f64| x * 100.0;
            println!(
                "{unit:9} planted foot slides {:5.1}% of the body's travel along the way in the script's walk, {:5.1}% in step ({:4.1}% and {:4.1}% across; lowest piece's origin {:5.1}% and {:5.1}%); jumps a frame up to {:.2} and {:.2}",
                pc(s.along[0]),
                pc(s.along[1]),
                pc(s.across[0]),
                pc(s.across[1]),
                pc(s.origin[0]),
                pc(s.origin[1]),
                s.jumps[0],
                s.jumps[1]
            );
            if unit == "ARMPW" {
                assert!(s.along[0] > 0.5, "the Peewee's script walk slides {:?}", s.along);
                assert!(s.along[1] < 0.05, "the Peewee in step slides {:?}", s.along);
            }
        }
    }

    /// Every walker of the install has a gait from its script; vehicles have none.
    #[test]
    fn the_installs_walkers_walk_in_step() {
        let Some((vfs, types)) = install() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let gaits = gaits(&vfs, &types);
        let mut walking = 0;
        let mut kept = Vec::new();
        for t in types.types.iter().filter(|t| t.locomotion == Locomotion::Ground) {
            let shape = gaits.shapes.get(t.id).and_then(|s| s.as_ref());
            let Some(shape) = shape else { continue };
            match extract(t, shape) {
                Ok(w) => {
                    let g = &w.walk;
                    walking += 1;
                    let speed = t.max_speed.to_f64() * 30.0;
                    println!(
                        "{:9} {:2} channels, cycle {:.3} s, covers {:5.1}: at full speed {:.2} cycles a second, {:.2} times its script's pace{}",
                        t.name,
                        g.channels.len(),
                        g.period,
                        g.distance,
                        speed / g.distance as f64,
                        speed / g.distance as f64 * g.period as f64,
                        w.aiming.as_ref().map_or(String::new(), |a| format!("; aiming {} channels, covers {:.1} in {:.3} s", a.channels.len(), a.distance, a.period))
                    );
                }
                Err(why) => kept.push(format!("{} ({why})", t.name)),
            }
        }
        println!("{walking} walk in step; kept their scripts' walk: {}", kept.join(", "));
        for name in ["ARMPW", "CORAK", "ARMHAM", "CORTHUD", "ARMCOM", "CORCOM", "CORCAN", "ARMFIDO"] {
            let t = types.by_name(name).unwrap();
            assert!(gaits.get(t).is_some(), "{name} has no gait");
        }
        for name in ["ARMSTUMP", "CORRAID", "ARMBULL"] {
            let t = types.by_name(name).unwrap();
            assert!(gaits.get(t).is_none(), "{name} is no walker");
        }
    }
}
