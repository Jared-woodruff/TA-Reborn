//! The radial menu's motion: how long each of its animations takes, the easing they run on and the
//! springs that follow the pointer. Everything here is a function of time since something began, or
//! a spring moved on by the exact solution of its equation: never a count of frames, so that a
//! frame every 4 ms and one every 33 ms draw the same pictures (and a capture on a fixed clock
//! draws what the game does). Nothing here knows of egui or of what is drawn (that is
//! `hud/radial.rs` and `hud/radial/paint.rs`), so it is tested for what it should be: bounded,
//! ordered, and at rest in the end.
//!
//! The animations: the ring unfolds, its plates coming in one after another from the top; a plate
//! under the pointer (or the stick) rises on a spring and its rim lights, a bead of light riding the
//! rim to wherever the choice has moved; a choice made is pressed in; a ring opened from another
//! unfolds from the plate that opened it while the one before folds into the middle (and the reverse
//! going back, a page of a long ring turning like a dial); the menu closes by folding into the
//! pointer, or, when a choice was made, the plate that made it flares and the rest let go.
//! With reduced motion nothing moves: plates fade (in a twelfth of a second), lights follow at once.
//! With reduced flashes the flare and the pulses of light are left out (`paint::Space::calm`).

use std::f32::consts::{PI, TAU};

use crate::radial::{Menu, Place};

// ---- Easing ------------------------------------------------------------------------------

// (The interface's own curves, which every page and dialog moves on too.)
pub use ta_ui::motion::{clamp01, in_cubic, lerp, out_back, out_cubic, progress, smooth};

/// An angle taken into -pi to pi: the way round the short side.
pub fn wrap(a: f32) -> f32 {
    (a + PI).rem_euclid(TAU) - PI
}

/// The angle of the middle of plate `i` of `count`, clockwise from straight up (the same as
/// `radial::angle`, which draws and picks by it).
pub fn plate_angle(i: usize, count: usize) -> f32 {
    crate::radial::angle(i, count)
}

// ---- Springs -----------------------------------------------------------------------------

/// A value that follows another as a spring does: it takes time to get there, and a spring that is
/// not damped to the full swings past. Moved on by the exact solution of the equation, so that any
/// number of steps that add up to the same time end in the same place.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spring {
    pub x: f32,
    pub v: f32,
}

impl Spring {
    pub fn at(x: f32) -> Spring {
        Spring { x, v: 0.0 }
    }

    /// Moves `dt` seconds on towards `target`: a spring of `freq` hertz with `damping` (1: just enough
    /// to reach it with no swing; less: swings past).
    pub fn step(&mut self, target: f32, dt: f32, freq: f32, damping: f32) {
        if dt <= 0.0 || !dt.is_finite() {
            return;
        }
        let w = TAU * freq.max(0.01);
        let z = damping.clamp(0.05, 1.0);
        let (y0, v0) = (self.x - target, self.v);
        let (y, v);
        if z >= 0.9999 {
            let e = (-w * dt).exp();
            y = e * (y0 + (v0 + w * y0) * dt);
            v = e * (v0 - w * (v0 + w * y0) * dt);
        } else {
            let a = z * w;
            let wd = w * (1.0 - z * z).sqrt();
            let e = (-a * dt).exp();
            let (s, c) = (wd * dt).sin_cos();
            y = e * (y0 * c + (v0 + a * y0) / wd * s);
            v = e * (v0 * c - (y0 * w * w + a * v0) / wd * s);
        }
        self.x = target + y;
        self.v = v;
    }

    /// Whether it has come to rest at `target` (near enough that nothing on the screen moves).
    pub fn settled(&self, target: f32) -> bool {
        (self.x - target).abs() < 0.002 && self.v.abs() < 0.02
    }

    /// Puts it at `x` at rest.
    pub fn jump(&mut self, x: f32) {
        *self = Spring::at(x);
    }
}

// ---- Timing ------------------------------------------------------------------------------

/// How long things take and how far they go. With reduced motion nothing goes anywhere: it fades.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    /// A plate's own time to come in (seconds), and the wait between one and the next as the
    /// ring opens.
    pub plate: f32,
    pub stagger: f32,
    /// The longest wait of a plate of a ring opened from another (the one furthest round from the
    /// plate that opened it).
    pub spread: f32,
    /// The rim and the hub: coming up, and going.
    pub rim: f32,
    /// A ring that is replaced: how long the new one takes to settle, and the old to go.
    pub turn_in: f32,
    pub turn_out: f32,
    /// Closing with no choice, and with one (the plate that made it flares for a little longer).
    pub leave: f32,
    pub confirm: f32,
    /// How far things move, as a share of the full journey (0: they fade where they stand).
    pub travel: f32,
    /// The springs of a plate coming alight and going dark: hertz and damping.
    pub lit_up: (f32, f32),
    pub lit_down: (f32, f32),
    /// The bead of light on the rim.
    pub bead: (f32, f32),
}

impl Timing {
    pub const FULL: Timing = Timing { plate: 0.15, stagger: 0.011, spread: 0.07, rim: 0.14, turn_in: 0.18, turn_out: 0.12, leave: 0.14, confirm: 0.26, travel: 1.0, lit_up: (5.5, 0.6), lit_down: (7.0, 1.0), bead: (4.6, 0.85) };
    /// For those who asked for less motion: everything fades, quickly, and lights follow at once.
    pub const REDUCED: Timing = Timing { plate: 0.08, stagger: 0.0, spread: 0.0, rim: 0.08, turn_in: 0.08, turn_out: 0.08, leave: 0.08, confirm: 0.12, travel: 0.0, lit_up: (40.0, 1.0), lit_down: (40.0, 1.0), bead: (40.0, 1.0) };

    pub fn of(reduced: bool) -> Timing {
        if reduced { Timing::REDUCED } else { Timing::FULL }
    }

    /// How long a menu of `count` plates takes to open, rim and all.
    pub fn open_total(&self, count: usize) -> f32 {
        self.stagger * count.saturating_sub(1) as f32 + self.plate.max(self.rim)
    }

    /// How long a ring takes to come in after another.
    pub fn turn_total(&self) -> f32 {
        self.spread + self.plate + 0.03 + 0.01 * 9.0
    }
}

// ---- How a ring changes -----------------------------------------------------------------

/// How the ring on top came to be.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Change {
    /// The menu coming up.
    #[default]
    Open,
    /// Opened from plate `from` of a ring of `of` plates.
    Into { from: Option<usize>, of: usize },
    /// Back from a ring that plate `to` of the ring now on top (`of` plates) opened.
    Back { to: Option<usize>, of: usize },
    /// The next page of a long ring (or the one before).
    Page { forward: bool },
    /// Replaced by another, some other way.
    Swap,
}

/// Which ring is on top (for noticing that it has changed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingKey {
    pub depth: usize,
    pub menu: Menu,
}

/// The page a ring is on (0 for a ring that has none).
fn page_of(menu: Menu) -> usize {
    match menu {
        Menu::More(k) | Menu::Units(k) | Menu::Buildings(_, k) => k,
        _ => 0,
    }
}

/// The pages of one list of choices are one family: the unit's own ring and its MORE pages, a
/// factory's units, the buildings of a purpose.
fn family(menu: Menu) -> (u8, usize) {
    match menu {
        Menu::Main | Menu::More(_) => (0, 0),
        Menu::Units(_) => (1, 0),
        Menu::Buildings(p, _) => (2, p),
        Menu::Fire => (3, 0),
        Menu::Movement => (4, 0),
        Menu::Purposes => (5, 0),
        Menu::Confirm => (6, 0),
        Menu::Duties => (7, 0),
    }
}

impl Change {
    /// How `new` came after `old`, given the places the rings were opened from as they stood before
    /// (`old_places`) and as they stand (`places`).
    pub fn between(old: RingKey, new: RingKey, old_places: &[Place], places: &[Place]) -> Change {
        if new.depth > old.depth {
            let place = places.get(new.depth.wrapping_sub(2));
            Change::Into { from: place.map(|p| p.index), of: place.map_or(0, |p| p.of) }
        } else if new.depth < old.depth {
            let place = old_places.get(old.depth.wrapping_sub(2));
            Change::Back { to: place.map(|p| p.index), of: place.map_or(0, |p| p.of) }
        } else if family(new.menu) == family(old.menu) && page_of(new.menu) != page_of(old.menu) {
            Change::Page { forward: page_of(new.menu) > page_of(old.menu) }
        } else {
            Change::Swap
        }
    }
}

/// How a plate stands at a moment: the three things that carry it from where it rests to where it
/// is coming from or going to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub alpha: f32,
    /// Its distance from the middle, as a share of what it is at rest.
    pub grow: f32,
    /// How far it has turned round the ring from its place (radians, clockwise).
    pub swing: f32,
}

impl Pose {
    pub const REST: Pose = Pose { alpha: 1.0, grow: 1.0, swing: 0.0 };
}

/// Plate `i` of `count` coming in, `t` seconds after its ring began.
pub fn coming_in(change: Change, i: usize, count: usize, t: f32, tm: &Timing) -> Pose {
    let a = plate_angle(i, count);
    let fly = tm.travel;
    match change {
        Change::Open => {
            // One after another from the top, round the ring, each swinging in from before its place.
            let p = clamp01((t - tm.stagger * i as f32) / tm.plate.max(1e-4));
            Pose { alpha: smooth(p * 2.2), grow: 1.0 - (1.0 - out_back(p, 1.1)) * 0.45 * fly, swing: -(1.0 - out_cubic(p)) * 0.85 * fly }
        }
        Change::Into { from, of } => {
            // From the plate that opened it, the plates nearest it first.
            let origin = from.map_or(0.0, |f| plate_angle(f, of.max(1)));
            let d = wrap(a - origin).abs() / PI;
            let p = clamp01((t - d * tm.spread) / (tm.plate + 0.03));
            let e = out_cubic(p);
            Pose { alpha: smooth(p * 2.0), grow: 1.0 - (1.0 - e) * 0.4 * fly, swing: wrap(origin - a) * (1.0 - e) * 0.65 * fly }
        }
        Change::Back { .. } => {
            // The ring before settles back: a little large, then in its place.
            let p = clamp01(t / tm.turn_in.max(1e-4));
            Pose { alpha: smooth(p * 1.6), grow: 1.0 + (1.0 - out_cubic(p)) * 0.08 * fly, swing: 0.0 }
        }
        Change::Page { forward } => {
            let p = clamp01((t - 0.008 * i as f32 * fly.min(1.0)) / (tm.plate + 0.04));
            let e = out_cubic(p);
            Pose { alpha: smooth(p * 2.0), grow: 1.0, swing: if forward { -1.0 } else { 1.0 } * (1.0 - e) * 0.75 * fly }
        }
        Change::Swap => Pose { alpha: smooth(t / tm.turn_in.max(1e-4)), ..Pose::REST },
    }
}

/// Plate `i` of `count` of the ring that is being left, `t` seconds after it began to go
/// (`picked`: the plate that made the choice, which is the last to go).
pub fn going_out(change: Change, i: usize, count: usize, picked: Option<usize>, t: f32, tm: &Timing) -> Pose {
    let a = plate_angle(i, count);
    let fly = tm.travel;
    let p = clamp01(t / tm.turn_out.max(1e-4));
    match change {
        Change::Into { .. } => {
            let last = picked == Some(i);
            Pose { alpha: 1.0 - smooth(p * if last { 0.9 } else { 1.3 }), grow: 1.0 - 0.42 * in_cubic(p) * fly, swing: 0.0 }
        }
        Change::Back { to, of } => {
            // Folding into the plate that opened it.
            let origin = to.map_or(a, |f| plate_angle(f, of.max(1)));
            Pose { alpha: 1.0 - smooth(p * 1.4), grow: 1.0 - 0.35 * in_cubic(p) * fly, swing: wrap(origin - a) * in_cubic(p) * 0.7 * fly }
        }
        Change::Page { forward } => Pose { alpha: 1.0 - smooth(p * 1.5), grow: 1.0, swing: if forward { 1.0 } else { -1.0 } * in_cubic(p) * 0.75 * fly },
        Change::Open | Change::Swap => Pose { alpha: 1.0 - smooth(p), ..Pose::REST },
    }
}

/// Plate `i` of `count` as the menu closes, `t` seconds after it began: folding into the middle of the
/// ring (which drifts towards the pointer, `closing_drift`), or, when `chosen` is the plate that made a
/// choice, flaring (and the rest letting go).
pub fn closing(i: usize, count: usize, chosen: Option<usize>, t: f32, tm: &Timing) -> Pose {
    let fly = tm.travel;
    match chosen {
        Some(c) if c == i => {
            // It swells a little as it takes the choice, holds, and then lets go.
            let swell = out_cubic(t / 0.08);
            let go = smooth((t - 0.10) / (tm.confirm - 0.10).max(1e-4));
            Pose { alpha: 1.0 - go, grow: 1.0 + 0.06 * swell * fly + 0.05 * go * fly, swing: 0.0 }
        }
        Some(_) => {
            let p = clamp01(t / (tm.leave * 0.85).max(1e-4));
            Pose { alpha: 1.0 - smooth(p * 1.2), grow: 1.0 - 0.2 * in_cubic(p) * fly, swing: 0.0 }
        }
        None => {
            // The last plate folds first.
            let delay = 0.006 * count.saturating_sub(1 + i) as f32 * fly.min(1.0);
            let p = clamp01((t - delay) / tm.leave.max(1e-4));
            Pose { alpha: 1.0 - smooth(p * 1.15), grow: 1.0 - 0.5 * in_cubic(p) * fly, swing: 0.35 * in_cubic(p) * fly }
        }
    }
}

/// How long a menu takes to close (the rim and hub as long as its slowest part).
pub fn closing_total(chosen: bool, tm: &Timing) -> f32 {
    if chosen { tm.confirm } else { tm.leave + 0.006 * 8.0 }
}

/// How far a menu that closes with no choice drifts towards the pointer as it folds: `share` of the
/// way to it, but no more than `max` points (`offset`: from the middle of the menu to the pointer).
/// A pointer right at the middle, or a figure that is not a number, pulls nowhere.
pub fn closing_pull(offset: [f32; 2], share: f32, max: f32) -> [f32; 2] {
    let len = (offset[0] * offset[0] + offset[1] * offset[1]).sqrt();
    if !len.is_finite() || len < 1e-3 || !share.is_finite() || !max.is_finite() {
        return [0.0, 0.0];
    }
    let reach = (len * share).clamp(0.0, max.max(0.0));
    [offset[0] / len * reach, offset[1] / len * reach]
}

/// How far a menu that is closing has drifted when the closing has come `go` of the way (0 to 1): its
/// `pull` (`closing_pull`) as far as that, and none at all with reduced motion, when nothing travels.
pub fn closing_drift(pull: [f32; 2], go: f32, tm: &Timing) -> [f32; 2] {
    let k = smooth(go) * tm.travel;
    [pull[0] * k, pull[1] * k]
}

/// The press of a plate, `t` seconds after it was pressed: in quickly and out again (0 to 1 to 0).
pub fn press(t: f32) -> f32 {
    if t < 0.0 {
        0.0
    } else if t < 0.04 {
        t / 0.04
    } else {
        (-(t - 0.04) * 13.0).exp()
    }
}

/// The shake of a choice that is refused, `t` seconds after it was: how far to the side (1 is the
/// full shake; the interface scales it).
pub fn shake(t: f32) -> f32 {
    if !(0.0..0.3).contains(&t) {
        return 0.0;
    }
    (t * 62.0).sin() * (-t * 11.0).exp()
}

// ---- The state of a menu's motion ----------------------------------------------------------

/// What follows the pointer: how lit each plate is, the bead of light on the rim, the hub and the
/// ring of health, and what the ring on top has been through.
#[derive(Clone, Debug, Default)]
pub struct Anim {
    clock: Option<f64>,
    /// When the menu came up.
    born: Option<f64>,
    ring: Option<RingKey>,
    places: Vec<Place>,
    /// When the ring on top began, and how it came.
    pub ring_at: f64,
    pub change: Change,
    /// How lit each plate is, and when it was last pressed.
    lit: Vec<Spring>,
    pressed: Vec<Option<f64>>,
    refused: Option<(usize, f64)>,
    /// The bead on the rim: where it is (an angle that does not wrap, so that it goes the short way
    /// round) and how bright, and where it is going.
    bead: Spring,
    bead_on: Spring,
    bead_to: f32,
    /// The pointer over the hub, and the health the ring shows (and the health it is going to).
    hub: Spring,
    health: Spring,
    health_to: f32,
    health_set: bool,
}

impl Anim {
    /// Starts a frame at `now`: how long since the one before (at most a tenth of a second, so a
    /// pause in the game does not fling the springs).
    pub fn tick(&mut self, now: f64) -> f32 {
        let dt = self.clock.map_or(0.0, |c| (now - c).clamp(0.0, 0.1) as f32);
        self.clock = Some(now);
        dt
    }

    /// Notes which ring is on top at `now`: when it has changed, how, and from what. `count` is how
    /// many plates it has. Returns the change when there was one (the ring before is the caller's to
    /// keep as long as it takes to go).
    pub fn ring(&mut self, key: RingKey, places: &[Place], count: usize, now: f64) -> Option<Change> {
        let changed = match self.ring {
            None => {
                self.change = Change::Open;
                self.born = Some(now);
                None
            }
            Some(old) if old != key => {
                let change = Change::between(old, key, &self.places, places);
                self.change = change;
                Some(change)
            }
            Some(_) => None,
        };
        if self.ring != Some(key) {
            self.ring = Some(key);
            self.ring_at = now;
            self.lit = vec![Spring::default(); count];
            self.pressed = vec![None; count];
            self.refused = None;
            self.bead_on.jump(0.0);
        }
        self.places = places.to_vec();
        if self.lit.len() != count {
            self.lit.resize(count, Spring::default());
            self.pressed.resize(count, None);
        }
        changed
    }

    /// Seconds since the ring on top began.
    pub fn since_ring(&self, now: f64) -> f32 {
        (now - self.ring_at).max(0.0) as f32
    }

    /// Seconds since the menu came up.
    pub fn since_open(&self, now: f64) -> f32 {
        self.born.map_or(0.0, |b| (now - b).max(0.0) as f32)
    }

    /// The plate `i` was pressed at `now`.
    pub fn press(&mut self, i: usize, now: f64) {
        if let Some(p) = self.pressed.get_mut(i) {
            *p = Some(now);
        }
    }

    /// The plate `i` was refused at `now`.
    pub fn refuse(&mut self, i: usize, now: f64) {
        self.refused = Some((i, now));
    }

    /// How pressed plate `i` is at `now` (0 to 1).
    pub fn pressed(&self, i: usize, now: f64) -> f32 {
        self.pressed.get(i).copied().flatten().map_or(0.0, |at| press((now - at) as f32))
    }

    /// How refused plate `i` is at `now` (1 as it is refused, then fading): the plate is tinted for it, with
    /// or without the shake.
    pub fn refusal(&self, i: usize, now: f64) -> f32 {
        match self.refused {
            Some((r, at)) if r == i => {
                let t = (now - at) as f32;
                if (0.0..0.3).contains(&t) { (1.0 - t / 0.3).powi(2) } else { 0.0 }
            }
            _ => 0.0,
        }
    }

    /// How far plate `i` is shaken sideways at `now` (-1 to 1).
    pub fn shaken(&self, i: usize, now: f64) -> f32 {
        match self.refused {
            Some((r, at)) if r == i => shake((now - at) as f32),
            _ => 0.0,
        }
    }

    /// Moves the springs on `dt` seconds towards where the pointer (or the stick) has put things:
    /// `hover` the plate it is on, `chosen` the one a choice was made on, `hub` the pointer over the
    /// middle, `health` the share of health the ring should show.
    pub fn follow(&mut self, dt: f32, tm: &Timing, count: usize, hover: Option<usize>, chosen: Option<usize>, hub: bool, health: Option<f32>) {
        let lit = hover.or(chosen);
        for (i, s) in self.lit.iter_mut().enumerate() {
            let on = lit == Some(i) || chosen == Some(i);
            let (freq, damping) = if on { tm.lit_up } else { tm.lit_down };
            s.step(if on { 1.0 } else { 0.0 }, dt, freq, damping);
        }
        // The bead: to the middle of the lit plate by the short way; with nothing lit it fades where it is.
        match lit.filter(|&i| i < count) {
            Some(i) => {
                let target = plate_angle(i, count);
                if self.bead_on.x < 0.05 {
                    self.bead.jump(target);
                }
                self.bead_to = self.bead.x + wrap(target - self.bead.x);
                self.bead.step(self.bead_to, dt, tm.bead.0, tm.bead.1);
                self.bead_on.step(1.0, dt, 6.0, 1.0);
            }
            None => self.bead_on.step(0.0, dt, 6.0, 1.0),
        }
        self.hub.step(if hub { 1.0 } else { 0.0 }, dt, tm.lit_down.0, tm.lit_down.1);
        if let Some(h) = health {
            // (The ring starts at the health the unit has, and runs to a new one.)
            if !self.health_set {
                self.health.jump(h);
                self.health_set = true;
            }
            self.health_to = h;
            // (A figure running to another is movement: with reduced motion it is the new figure at once.)
            self.health.step(h, dt, if tm.travel > 0.0 { 3.0 } else { 40.0 }, 1.0);
        }
    }

    /// How lit plate `i` is (0 to 1, a little over while the spring swings past on the way up).
    pub fn lit(&self, i: usize) -> f32 {
        self.lit.get(i).map_or(0.0, |s| s.x)
    }

    /// How bright the bead is, and where (the angle, clockwise from up).
    pub fn bead(&self) -> Option<(f32, f32)> {
        let k = clamp01(self.bead_on.x);
        (k > 0.01).then_some((self.bead.x, k))
    }

    /// How much the pointer is over the hub.
    pub fn hub(&self) -> f32 {
        clamp01(self.hub.x)
    }

    /// The share of health the ring shows now (it runs to the real one over a moment).
    pub fn health(&self) -> f32 {
        clamp01(self.health.x)
    }

    /// Whether anything is still moving at `now` (the interface asks for frames while it is).
    pub fn busy(&self, now: f64, tm: &Timing, count: usize) -> bool {
        if self.since_ring(now) < tm.open_total(count).max(tm.turn_total()) + 0.02 {
            return true;
        }
        if self.pressed.iter().flatten().any(|&at| now - at < 0.5) || self.refused.is_some_and(|(_, at)| now - at < 0.3) {
            return true;
        }
        let settled = |s: &Spring| s.settled(0.0) || s.settled(1.0);
        let bead_moving = self.bead_on.x > 0.01 && !self.bead.settled(self.bead_to);
        !(self.lit.iter().all(settled) && settled(&self.bead_on) && settled(&self.hub) && !bead_moving && self.health.settled(self.health_to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TM: Timing = Timing::FULL;

    /// Every easing runs from 0 to 1, never leaves its range (the one that comes in past its place
    /// does, a little, and comes back) and never goes backwards.
    #[test]
    fn the_easings_run_from_nothing_to_all_without_going_back() {
        let ease: [(&str, fn(f32) -> f32); 3] = [("smooth", smooth), ("out", out_cubic), ("in", in_cubic)];
        for (name, f) in ease {
            assert_eq!(f(0.0), 0.0, "{name}");
            assert_eq!(f(1.0), 1.0, "{name}");
            assert_eq!(f(-3.0), 0.0, "{name} before");
            assert_eq!(f(4.0), 1.0, "{name} after");
            let mut last = 0.0;
            for k in 0..=100 {
                let v = f(k as f32 / 100.0);
                assert!(v >= last - 1e-6 && (0.0..=1.0).contains(&v), "{name} at {k}");
                last = v;
            }
        }
        assert!((out_back(0.0, 1.1)).abs() < 1e-6 && (out_back(1.0, 1.1) - 1.0).abs() < 1e-6);
        let top = (0..=100).map(|k| out_back(k as f32 / 100.0, 1.1)).fold(0.0, f32::max);
        assert!(top > 1.0 && top < 1.12, "it comes in a little past its place: {top}");
        // Fast then slow, slow then fast.
        assert!(out_cubic(0.3) > 0.3 && in_cubic(0.3) < 0.3);
    }

    #[test]
    fn progress_is_how_far_on_something_is() {
        assert_eq!(progress(1.0, 2.0, 0.5), 0.0);
        assert!((progress(2.25, 2.0, 0.5) - 0.5).abs() < 1e-6);
        assert_eq!(progress(9.0, 2.0, 0.5), 1.0);
        assert_eq!(progress(2.0, 2.0, 0.0), 1.0, "no time at all: done at once");
        assert_eq!(progress(1.0, 2.0, 0.0), 0.0);
    }

    #[test]
    fn angles_go_the_short_way_round() {
        assert!((wrap(TAU - 0.1) + 0.1).abs() < 1e-5);
        assert!((wrap(0.1 - TAU) - 0.1).abs() < 1e-5);
        assert!((wrap(PI + 0.2) + PI - 0.2).abs() < 1e-5);
        for k in -20..20 {
            let w = wrap(k as f32 * 0.7);
            assert!((-PI..=PI).contains(&w));
        }
    }

    /// A spring comes to rest at its target, whatever it starts from; one that is not damped to the
    /// full swings past once or twice on the way and one that is does not.
    #[test]
    fn a_spring_comes_to_rest_at_its_target() {
        for damping in [0.3, 0.6, 1.0] {
            let mut s = Spring::at(0.0);
            let mut peak = 0.0f32;
            for _ in 0..300 {
                s.step(1.0, 1.0 / 60.0, 5.5, damping);
                peak = peak.max(s.x);
            }
            assert!(s.settled(1.0), "damping {damping}: {s:?}");
            if damping >= 1.0 {
                assert!(peak <= 1.0 + 1e-4, "no swing at full damping: {peak}");
            } else {
                assert!(peak > 1.0, "damping {damping} swings past");
            }
        }
        // From a push, too.
        let mut s = Spring { x: 0.4, v: -3.0 };
        for _ in 0..600 {
            s.step(0.0, 1.0 / 120.0, 4.0, 0.5);
        }
        assert!(s.settled(0.0));
    }

    /// The same time in any number of steps ends in the same place: a frame every 4 ms, every 16 or
    /// every 33 give one picture (the exact solution, not a step that drifts with its size).
    #[test]
    fn a_spring_is_the_same_at_any_frame_rate() {
        for damping in [0.5, 0.85, 1.0] {
            let at = |frames: u32| {
                let mut s = Spring { x: 0.2, v: 1.5 };
                for _ in 0..frames {
                    s.step(1.0, 0.24 / frames as f32, 5.0, damping);
                }
                s
            };
            let base = at(1);
            for frames in [2, 4, 8, 15, 60, 240] {
                let s = at(frames);
                assert!((s.x - base.x).abs() < 2e-4 && (s.v - base.v).abs() < 2e-3, "damping {damping}, {frames} steps: {s:?} against {base:?}");
            }
        }
    }

    /// A long pause (a spring stepped a whole second, or ten) and bad times do no harm.
    #[test]
    fn a_spring_survives_long_steps_and_bad_ones() {
        let mut s = Spring::at(0.0);
        s.step(1.0, 10.0, 6.0, 0.4);
        assert!(s.settled(1.0), "{s:?}");
        let before = s;
        for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            s.step(5.0, dt, 6.0, 0.4);
            assert_eq!(s, before, "dt {dt}");
        }
        let mut s = Spring::at(0.3);
        s.step(0.7, 0.05, 0.0, 0.0);
        assert!(s.x.is_finite() && s.v.is_finite());
    }

    fn key(depth: usize, menu: Menu) -> RingKey {
        RingKey { depth, menu }
    }

    /// Going into a ring, back from it, turning a page and swapping are told apart, and a ring
    /// that was opened by a plate says which.
    #[test]
    fn a_change_of_ring_is_told_into_back_page_or_swap() {
        let places = [Place { index: 4, of: 9 }];
        let main = key(1, Menu::Main);
        let fire = key(2, Menu::Fire);
        assert_eq!(Change::between(main, fire, &[], &places), Change::Into { from: Some(4), of: 9 });
        assert_eq!(Change::between(fire, main, &places, &[]), Change::Back { to: Some(4), of: 9 });
        // (A ring opened with no place kept: still into, from nowhere.)
        assert_eq!(Change::between(main, fire, &[], &[]), Change::Into { from: None, of: 0 });
        // Pages: the next and the one before, of a factory's units and of the first ring's.
        assert_eq!(Change::between(key(2, Menu::Units(0)), key(2, Menu::Units(1)), &places, &places), Change::Page { forward: true });
        assert_eq!(Change::between(key(2, Menu::Units(2)), key(2, Menu::Units(1)), &places, &places), Change::Page { forward: false });
        assert_eq!(Change::between(main, key(1, Menu::More(1)), &[], &[]), Change::Page { forward: true });
        assert_eq!(Change::between(key(1, Menu::More(1)), main, &[], &[]), Change::Page { forward: false });
        // Two different rings at one depth: a swap.
        assert_eq!(Change::between(key(2, Menu::Fire), key(2, Menu::Movement), &places, &places), Change::Swap);
    }

    /// The state notices a change once, keeps its places and starts the new ring's clock.
    #[test]
    fn the_state_notices_a_change_once() {
        let mut a = Anim::default();
        a.tick(0.0);
        assert_eq!(a.ring(key(1, Menu::Main), &[], 6, 0.0), None, "the first ring is the opening, not a change");
        assert_eq!(a.change, Change::Open);
        assert_eq!(a.ring(key(1, Menu::Main), &[], 6, 0.1), None);
        let places = [Place { index: 2, of: 6 }];
        assert_eq!(a.ring(key(2, Menu::Fire), &places, 3, 0.5), Some(Change::Into { from: Some(2), of: 6 }));
        assert_eq!(a.ring(key(2, Menu::Fire), &places, 3, 0.6), None, "once");
        assert!((a.since_ring(0.75) - 0.25).abs() < 1e-6);
        assert_eq!(a.ring(key(1, Menu::Main), &[], 6, 1.0), Some(Change::Back { to: Some(2), of: 6 }));
        assert_eq!(a.change, Change::Back { to: Some(2), of: 6 });
        assert_eq!(a.lit(5), 0.0);
    }

    /// Plates come in one after another round the ring from the top, each from nothing to its place,
    /// bounded all the way, and the last is in its place when the ring says it is open.
    #[test]
    fn the_ring_unfolds_plate_by_plate() {
        let count = 8;
        let pose = |i: usize, t: f32| coming_in(Change::Open, i, count, t, &TM);
        for i in 0..count {
            let first = pose(i, 0.0);
            assert_eq!(first.alpha, 0.0, "{i} starts unseen");
            let mut last_alpha = 0.0;
            for k in 0..=60 {
                let p = pose(i, k as f32 * 0.005);
                assert!((0.0..=1.0).contains(&p.alpha) && p.alpha >= last_alpha - 1e-6, "{i} at {k}: alpha {}", p.alpha);
                assert!((0.5..=1.06).contains(&p.grow) && p.swing.abs() <= 0.9, "{i} at {k}: {p:?}");
                last_alpha = p.alpha;
            }
            let end = pose(i, TM.open_total(count) + 0.01);
            assert!((end.alpha - 1.0).abs() < 1e-5 && (end.grow - 1.0).abs() < 1e-3 && end.swing.abs() < 1e-3, "{i} ends in its place: {end:?}");
        }
        // The first is up before the last has begun.
        assert!(pose(0, 0.1).alpha > 0.99 && pose(count - 1, 0.1).alpha < 0.5);
        for i in 1..count {
            assert!(pose(i - 1, 0.07).alpha >= pose(i, 0.07).alpha, "in order from the top");
        }
    }

    /// A ring opened from a plate unfolds from that plate: the plates nearest it in its direction
    /// come first, those furthest round last, and each starts nearer the plate than its place.
    #[test]
    fn a_sub_ring_unfolds_from_the_plate_that_opened_it() {
        let count = 6;
        let from = Some(2);
        let change = Change::Into { from, of: 8 };
        let origin = plate_angle(2, 8);
        let near = (0..count).min_by(|&a, &b| wrap(plate_angle(a, count) - origin).abs().total_cmp(&wrap(plate_angle(b, count) - origin).abs())).unwrap();
        let far = (0..count).max_by(|&a, &b| wrap(plate_angle(a, count) - origin).abs().total_cmp(&wrap(plate_angle(b, count) - origin).abs())).unwrap();
        let t = 0.05;
        assert!(coming_in(change, near, count, t, &TM).alpha > coming_in(change, far, count, t, &TM).alpha, "the nearest first");
        for i in 0..count {
            let p = coming_in(change, i, count, 0.0, &TM);
            let a = plate_angle(i, count);
            // Its swing is towards the origin: the plate begins nearer it.
            assert!(wrap(a + p.swing - origin).abs() <= wrap(a - origin).abs() + 1e-5, "{i} begins towards the plate that opened it");
            let end = coming_in(change, i, count, TM.turn_total(), &TM);
            assert!((end.alpha - 1.0).abs() < 1e-5 && end.swing.abs() < 1e-3 && (end.grow - 1.0).abs() < 1e-3);
        }
    }

    /// Whatever the change, a ring going out starts as it was, ends unseen, and is bounded; and the
    /// plate that made the choice is the last to go.
    #[test]
    fn a_ring_that_is_left_goes_out_whole() {
        let count = 7;
        let changes = [Change::Into { from: Some(1), of: 7 }, Change::Back { to: Some(3), of: 9 }, Change::Page { forward: true }, Change::Page { forward: false }, Change::Swap];
        for change in changes {
            for i in 0..count {
                let start = going_out(change, i, count, Some(1), 0.0, &TM);
                assert!((start.alpha - 1.0).abs() < 1e-6 && (start.grow - 1.0).abs() < 1e-6 && start.swing.abs() < 1e-6, "{change:?} {i}: {start:?}");
                let end = going_out(change, i, count, Some(1), TM.turn_out + 0.001, &TM);
                assert!(end.alpha.abs() < 1e-5 || matches!(change, Change::Into { .. }) && i == 1 && end.alpha < 0.2, "{change:?} {i}: gone by the end: {end:?}");
                let mut last = 1.0;
                for k in 0..=40 {
                    let p = going_out(change, i, count, Some(1), k as f32 * TM.turn_out / 40.0, &TM);
                    assert!((0.0..=1.0).contains(&p.alpha) && p.alpha <= last + 1e-6, "{change:?} {i} at {k}");
                    assert!((0.5..=1.0 + 1e-6).contains(&p.grow) && p.swing.abs() <= PI, "{change:?} {i} at {k}: {p:?}");
                    last = p.alpha;
                }
            }
        }
        let mid = TM.turn_out * 0.6;
        let picked = going_out(Change::Into { from: Some(1), of: 7 }, 1, count, Some(1), mid, &TM);
        let other = going_out(Change::Into { from: Some(1), of: 7 }, 2, count, Some(1), mid, &TM);
        assert!(picked.alpha > other.alpha, "the choice made is the last to go");
    }

    /// A page turns like a dial: the one before goes the way the pages go, the new one comes in
    /// from the other side; going back, the opposite.
    #[test]
    fn a_page_turns_like_a_dial() {
        let count = 8;
        let fwd = Change::Page { forward: true };
        let back = Change::Page { forward: false };
        assert!(going_out(fwd, 3, count, None, 0.1, &TM).swing > 0.0 && going_out(back, 3, count, None, 0.1, &TM).swing < 0.0);
        assert!(coming_in(fwd, 3, count, 0.02, &TM).swing < 0.0 && coming_in(back, 3, count, 0.02, &TM).swing > 0.0);
    }

    /// The menu closes by folding into the pointer; with a choice, the plate that made it swells and goes
    /// last while the others let go; both are gone by the total they give.
    #[test]
    fn the_menu_closes_into_the_pointer_or_flares_on_its_choice() {
        let count = 8;
        for chosen in [None, Some(3)] {
            let total = closing_total(chosen.is_some(), &TM);
            for i in 0..count {
                let start = closing(i, count, chosen, 0.0, &TM);
                assert!((start.alpha - 1.0).abs() < 1e-6 && (start.grow - 1.0).abs() < 1e-6 && start.swing.abs() < 1e-6, "{chosen:?} {i}: {start:?}");
                let end = closing(i, count, chosen, total + 0.001, &TM);
                assert!(end.alpha < 1e-4, "{chosen:?} {i}: gone: {end:?}");
                let mut last = 1.01;
                for k in 0..=50 {
                    let p = closing(i, count, chosen, total * k as f32 / 50.0, &TM);
                    assert!((0.0..=1.0).contains(&p.alpha) && p.alpha <= last + 1e-5 && (0.4..=1.12).contains(&p.grow), "{chosen:?} {i} at {k}: {p:?}");
                    last = p.alpha;
                }
            }
        }
        let t = 0.06;
        let (made, other) = (closing(3, count, Some(3), t, &TM), closing(2, count, Some(3), t, &TM));
        assert!(made.alpha > other.alpha && made.grow > 1.0 && other.grow < 1.0, "the chosen plate swells and the rest let go");
        assert!(closing(3, count, Some(3), 0.2, &TM).alpha > 0.0, "it holds a moment");
        // Folding in: it shrinks towards the middle.
        assert!(closing(count - 1, count, None, 0.12, &TM).grow < 0.8);
    }

    /// A menu closed with no choice drifts a share of the way to the pointer, never further than the
    /// most it is given, in the pointer's direction; and not at all towards nothing.
    #[test]
    fn a_menu_that_closes_drifts_towards_the_pointer_a_little() {
        let near = closing_pull([30.0, 40.0], 0.4, 70.0);
        assert!((near[0] - 12.0).abs() < 1e-4 && (near[1] - 16.0).abs() < 1e-4, "{near:?}: 0.4 of the way");
        let far = closing_pull([-300.0, 400.0], 0.4, 70.0);
        let len = (far[0] * far[0] + far[1] * far[1]).sqrt();
        assert!((len - 70.0).abs() < 1e-3, "held to the most: {len}");
        assert!(far[0] < 0.0 && far[1] > 0.0 && (far[0] / far[1] + 0.75).abs() < 1e-4, "the same direction: {far:?}");
        assert_eq!(closing_pull([0.0, 0.0], 0.4, 70.0), [0.0, 0.0], "at the middle: nowhere");
        assert_eq!(closing_pull([f32::NAN, 3.0], 0.4, 70.0), [0.0, 0.0]);
        assert_eq!(closing_pull([f32::INFINITY, 3.0], 0.4, 70.0), [0.0, 0.0]);
        assert_eq!(closing_pull([10.0, 0.0], 0.4, -5.0), [0.0, 0.0], "a negative most is nothing");
    }

    /// The drift of a closing menu grows from nothing to its whole pull as the closing comes, and is
    /// nothing at all with reduced motion.
    #[test]
    fn the_closing_drift_grows_with_the_closing_and_is_none_with_reduced_motion() {
        let pull = [30.0, -20.0];
        assert_eq!(closing_drift(pull, 0.0, &TM), [0.0, 0.0]);
        let half = closing_drift(pull, 0.5, &TM);
        assert!((half[0] - 15.0).abs() < 1e-4 && (half[1] + 10.0).abs() < 1e-4, "{half:?}");
        assert_eq!(closing_drift(pull, 1.0, &TM), pull);
        let mut last = 0.0;
        for k in 0..=20 {
            let d = closing_drift(pull, k as f32 / 20.0, &TM)[0];
            assert!(d >= last - 1e-5, "it does not turn back");
            last = d;
        }
        for go in [0.0, 0.3, 0.7, 1.0, 2.0] {
            assert_eq!(closing_drift(pull, go, &Timing::REDUCED), [0.0, 0.0], "reduced motion, {go}");
        }
    }

    /// With reduced motion nothing travels: every pose is in its place, only the opacity moves, and
    /// everything is done sooner.
    #[test]
    fn reduced_motion_fades_and_nothing_flies() {
        let tm = Timing::of(true);
        assert_eq!(Timing::of(false), Timing::FULL);
        assert!(tm.open_total(9) < 0.1 && closing_total(true, &tm) < Timing::FULL.confirm);
        let changes = [Change::Open, Change::Into { from: Some(1), of: 6 }, Change::Back { to: Some(1), of: 6 }, Change::Page { forward: true }, Change::Swap];
        for change in changes {
            for i in 0..6 {
                for k in 0..=20 {
                    let t = k as f32 * 0.01;
                    for pose in [coming_in(change, i, 6, t, &tm), going_out(change, i, 6, Some(1), t, &tm)] {
                        assert!((pose.grow - 1.0).abs() < 1e-6 && pose.swing.abs() < 1e-6, "{change:?}: {pose:?}");
                    }
                }
            }
        }
        for chosen in [None, Some(2)] {
            for i in 0..6 {
                for k in 0..=20 {
                    let p = closing(i, 6, chosen, k as f32 * 0.01, &tm);
                    assert!((p.grow - 1.0).abs() < 1e-6 && p.swing.abs() < 1e-6, "{chosen:?} {i}: {p:?}");
                }
            }
        }
        // Lights follow at once: a plate is lit within a frame or two.
        let mut a = Anim::default();
        a.tick(0.0);
        a.ring(key(1, Menu::Main), &[], 6, 0.0);
        for _ in 0..3 {
            a.follow(1.0 / 30.0, &tm, 6, Some(2), None, false, None);
        }
        assert!(a.lit(2) > 0.97, "{}", a.lit(2));
    }

    /// The press goes in fast and comes out slower, once; the shake is a short ring-down.
    #[test]
    fn a_press_is_a_pulse_and_a_refusal_a_shake() {
        assert_eq!(press(-1.0), 0.0);
        assert_eq!(press(0.0), 0.0);
        assert!((press(0.04) - 1.0).abs() < 1e-6);
        assert!(press(0.15) < 0.4 && press(0.15) > 0.0 && press(2.0) < 0.01);
        assert!((0..100).all(|k| (0.0..=1.0).contains(&press(k as f32 * 0.01))));
        assert_eq!(shake(-0.1), 0.0);
        assert_eq!(shake(0.5), 0.0);
        let swing: Vec<f32> = (0..30).map(|k| shake(k as f32 * 0.01)).collect();
        assert!(swing.iter().any(|&v| v > 0.3) && swing.iter().any(|&v| v < -0.3), "it goes both ways");
        assert!(swing.iter().all(|v| v.abs() <= 1.0));
    }

    /// The plate under the pointer comes alight on a spring and the one it leaves goes dark; the
    /// bead of light rides the rim the short way round to the new plate; with none lit the bead goes out.
    #[test]
    fn the_light_follows_the_pointer_round_the_ring() {
        let mut a = Anim::default();
        a.tick(0.0);
        a.ring(key(1, Menu::Main), &[], 8, 0.0);
        let dt = 1.0 / 60.0;
        for _ in 0..60 {
            a.follow(dt, &TM, 8, Some(0), None, false, None);
        }
        assert!(a.lit(0) > 0.99 && a.lit(1) < 0.01);
        let (angle, k) = a.bead().expect("the bead is lit over the plate");
        assert!(angle.abs() < 0.01 && k > 0.9);
        // To the last plate: the short way, past the top, not all the way round.
        let mut least = angle;
        for _ in 0..90 {
            a.follow(dt, &TM, 8, Some(7), None, false, None);
            least = least.min(a.bead().unwrap().0);
        }
        assert!(least < 0.0 && least > -1.2, "it went backwards past the top, not forwards round: {least}");
        assert!(a.lit(7) > 0.99 && a.lit(0) < 0.01);
        assert!((wrap(a.bead().unwrap().0 - plate_angle(7, 8))).abs() < 0.05);
        // The light on a plate swings a little past full as it rises, and never below nothing.
        let mut b = Anim::default();
        b.tick(0.0);
        b.ring(key(1, Menu::Main), &[], 4, 0.0);
        let peak = (0..40).map(|_| { b.follow(dt, &TM, 4, Some(1), None, false, None); b.lit(1) }).fold(0.0, f32::max);
        assert!(peak > 1.0 && peak < 1.25, "{peak}");
        for _ in 0..120 {
            b.follow(dt, &TM, 4, None, None, false, None);
            assert!(b.lit(1) > -0.01);
        }
        assert!(b.lit(1) < 0.002 && b.bead().is_none(), "dark again, the bead out");
    }

    /// A menu with nothing moving asks for no frames; one that has just opened, been pressed or lit does.
    #[test]
    fn it_asks_for_frames_only_while_something_moves() {
        let mut a = Anim::default();
        a.tick(0.0);
        a.ring(key(1, Menu::Main), &[], 6, 0.0);
        assert!(a.busy(0.05, &TM, 6), "opening");
        for k in 0..120 {
            a.follow(1.0 / 60.0, &TM, 6, None, None, false, None);
            a.tick(k as f64 / 60.0 + 1.0);
        }
        assert!(!a.busy(5.0, &TM, 6), "at rest");
        a.press(2, 5.0);
        assert!(a.busy(5.1, &TM, 6) && !a.busy(6.0, &TM, 6));
        a.refuse(1, 6.0);
        assert!(a.busy(6.1, &TM, 6) && !a.busy(7.0, &TM, 6));
        a.follow(1.0 / 60.0, &TM, 6, Some(3), None, false, None);
        assert!(a.busy(7.0, &TM, 6), "a plate coming alight");
    }

    /// With reduced motion the health ring does not run to a new figure: it is there within a frame or two.
    #[test]
    fn the_health_ring_does_not_run_with_reduced_motion() {
        let tm = Timing::of(true);
        let mut a = Anim::default();
        a.tick(0.0);
        a.ring(key(1, Menu::Main), &[], 4, 0.0);
        a.follow(0.0, &tm, 4, None, None, false, Some(0.8));
        for _ in 0..3 {
            a.follow(1.0 / 30.0, &tm, 4, None, None, false, Some(0.3));
        }
        assert!((a.health() - 0.3).abs() < 0.02, "{}", a.health());
    }

    /// A refusal tints its plate for a moment and then lets go, and only that plate.
    #[test]
    fn a_refusal_tints_its_plate_for_a_moment() {
        let mut a = Anim::default();
        a.tick(0.0);
        a.ring(key(1, Menu::Main), &[], 5, 0.0);
        a.refuse(3, 2.0);
        assert!((a.refusal(3, 2.0) - 1.0).abs() < 1e-6 && a.refusal(2, 2.0) == 0.0);
        assert!(a.refusal(3, 2.1) < a.refusal(3, 2.05) && a.refusal(3, 2.1) > 0.0, "fading");
        assert_eq!(a.refusal(3, 2.4), 0.0);
        assert_eq!(a.refusal(3, 1.9), 0.0, "(not before it was)");
    }

    /// The health ring starts at the health the unit has (the first frame shows the real figure) and
    /// runs to a new one, over a moment, without leaving 0 to 1.
    #[test]
    fn the_health_ring_starts_where_the_unit_is_and_runs_to_what_it_becomes() {
        let mut a = Anim::default();
        a.tick(0.0);
        a.ring(key(1, Menu::Main), &[], 4, 0.0);
        a.follow(0.0, &TM, 4, None, None, false, Some(0.8));
        assert!((a.health() - 0.8).abs() < 1e-6, "{}", a.health());
        a.follow(1.0 / 60.0, &TM, 4, None, None, false, Some(0.3));
        assert!(a.health() < 0.8 && a.health() > 0.3, "on its way: {}", a.health());
        for _ in 0..120 {
            a.follow(1.0 / 60.0, &TM, 4, None, None, false, Some(0.3));
            assert!((0.0..=1.0).contains(&a.health()));
        }
        assert!((a.health() - 0.3).abs() < 0.005);
        assert!(!a.busy(50.0, &TM, 4), "and still");
    }

    /// The bead of light is motion until it has arrived: the plates' lights settle sooner than the
    /// bead, and the menu goes on asking for frames while it travels.
    #[test]
    fn the_bead_is_motion_until_it_arrives() {
        let mut a = Anim::default();
        a.tick(0.0);
        a.ring(key(1, Menu::Main), &[], 8, 0.0);
        let dt = 1.0 / 60.0;
        for _ in 0..120 {
            a.follow(dt, &TM, 8, Some(0), None, false, None);
        }
        assert!(!a.busy(50.0, &TM, 8), "at rest on the first plate");
        a.follow(dt, &TM, 8, Some(4), None, false, None);
        assert!(a.busy(50.0, &TM, 8), "off to the plate opposite");
        let mut frames = 0;
        while a.busy(50.0, &TM, 8) && frames < 600 {
            a.follow(dt, &TM, 8, Some(4), None, false, None);
            frames += 1;
        }
        assert!((10..600).contains(&frames), "it arrives after {frames} frames");
        let (angle, k) = a.bead().expect("still lit");
        assert!(wrap(angle - plate_angle(4, 8)).abs() < 0.02 && k > 0.99, "{angle}");
    }

    /// A press and a shake are read back at the time they are asked, for the plate they were made on.
    #[test]
    fn a_press_and_a_refusal_are_the_plates_own() {
        let mut a = Anim::default();
        a.tick(0.0);
        a.ring(key(1, Menu::Main), &[], 5, 0.0);
        a.press(2, 1.0);
        assert!(a.pressed(2, 1.04) > 0.99 && a.pressed(1, 1.04) == 0.0 && a.pressed(2, 0.5) == 0.0);
        a.refuse(3, 2.0);
        assert!(a.shaken(3, 2.02) != 0.0 && a.shaken(2, 2.02) == 0.0 && a.shaken(3, 3.0) == 0.0);
        // A frame's time is at most a tenth of a second.
        assert_eq!(a.tick(0.0), 0.0);
        assert!((a.tick(0.5) - 0.1).abs() < 1e-6);
        assert!((a.tick(0.52) - 0.02).abs() < 1e-6);
    }
}
