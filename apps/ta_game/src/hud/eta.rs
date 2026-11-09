//! The time left on what is being built, over it: small and dim, fading in as the work starts and
//! out as it ends, for our own and our allies' construction (drawn by `Hud::overlays`, with the
//! health and build bars).
//!
//! The pace comes from the builders (every one working on it: the factory, the constructors
//! assisting) at full speed, scaled by how much of that the economy lets through, which is
//! learned from how the work has actually gone, a second or two back and smoothed: a stall (a
//! store run dry, everything building slower) shows as a longer time, and a builder joining as a
//! shorter one at once.

use std::collections::HashMap;

use egui::{Color32, Painter, Pos2, Stroke};
use ta_sim::{Sim, UnitId};

/// Ticks over which the economy's share is learned (a second and a half).
const FOLLOW: f32 = 45.0;
/// Seconds an estimate fades in over, and the last seconds it fades out in.
const FADE_IN: f32 = 0.6;
const FADE_OUT: f32 = 1.5;
/// A build slower than this many whole builds a second (one in half an hour) is a stall.
const STALLED: f32 = 1.0 / 1800.0;

/// How one thing being built has gone lately.
#[derive(Clone, Copy, Debug)]
struct Track {
    /// The tick, and how much of the build was done (0 to 1), when last looked at.
    tick: u64,
    done: f32,
    /// How much of the builders' full pace the economy lets through, learned.
    share: f32,
    /// When it was first drawn (the interface clock), and last.
    first: f32,
    seen: f32,
}

/// What to draw over a thing being built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Eta {
    /// Seconds left at the present pace, and how visible it is (0 to 1).
    Left(f32, f32),
    /// Builders are at it and nothing comes of it: a store has run dry. How visible (0 to 1).
    Stalled(f32),
}

#[derive(Default)]
pub(super) struct Etas {
    tracks: HashMap<UnitId, Track>,
}

/// Whole builds a second each thing under construction gets from its builders at full pace:
/// the sum, over the units working on it, of their work a second out of the work it takes.
pub(super) fn builder_paces(sim: &Sim) -> HashMap<UnitId, f32> {
    let mut paces: HashMap<UnitId, f32> = HashMap::new();
    for builder in sim.units() {
        let Some(job) = builder.build else { continue };
        let Some(target) = sim.unit(job.target).filter(|t| !t.is_complete()) else { continue };
        let work = sim.unit_type(target).build_work.max(1) as f32;
        *paces.entry(job.target).or_default() += sim.unit_type(builder).work_rate as f32 * ta_sim::cob::TICKS_PER_SECOND as f32 / work;
    }
    paces
}

impl Etas {
    /// Forgets what has not been drawn for a while (call once a frame).
    pub(super) fn forget_old(&mut self, clock: f32) {
        self.tracks.retain(|_, t| clock - t.seen < 2.0);
    }

    /// The time left for `id`, `done` of it built at `tick`, with `pace` whole builds a second
    /// from its builders at full speed (none: nobody is building it, nothing to say).
    pub(super) fn of(&mut self, id: UnitId, tick: u64, done: f32, pace: f32, clock: f32) -> Option<Eta> {
        if pace <= 0.0 {
            self.tracks.remove(&id);
            return None;
        }
        let track = self.tracks.entry(id).or_insert(Track { tick, done, share: 1.0, first: clock, seen: clock });
        track.seen = clock;
        if tick > track.tick {
            // What the builders did these ticks against what they could: the economy's share.
            let ticks = (tick - track.tick) as f32;
            let observed = ((done - track.done) / (ticks / ta_sim::cob::TICKS_PER_SECOND as f32) / pace).clamp(0.0, 1.2);
            track.share += (observed - track.share) * (1.0 - (-ticks / FOLLOW).exp());
            (track.tick, track.done) = (tick, done);
        } else if tick < track.tick {
            // (Time went back: a rewound replay, a loaded game.)
            *track = Track { tick, done, share: 1.0, first: clock, seen: clock };
        }
        let fade_in = ((clock - track.first) / FADE_IN).clamp(0.0, 1.0);
        let speed = pace * track.share;
        if speed < STALLED {
            return Some(Eta::Stalled(fade_in));
        }
        let left = ((1.0 - done) / speed).max(0.0);
        Some(Eta::Left(left, fade_in * (left / FADE_OUT).clamp(0.0, 1.0)))
    }
}

/// "0:42", "12:05", "1:02:09": minutes and seconds (hours when there are some).
pub(super) fn clock_text(seconds: f32) -> String {
    let s = seconds.max(0.0).ceil() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// Draws the time left with its bottom middle at `at`: a tiny clock and the digits, dim and
/// shadowed so that they read on pale ground; `strength` (0 to 1) fades the lot (the zoom's).
pub(super) fn paint(painter: &Painter, at: Pos2, eta: Eta, strength: f32) {
    let (text, colour, shown) = match eta {
        Eta::Left(seconds, shown) => (clock_text(seconds), crate::theme::TEXT, shown),
        Eta::Stalled(shown) => ("--:--".to_string(), crate::theme::AMBER, shown),
    };
    // (Dim: the bars say the rest, and it must not shout over the battle.)
    let strength = shown * strength * 0.8;
    if strength < 0.02 {
        return;
    }
    let galley = painter.layout_no_wrap(text, crate::theme::strong(11.0), colour.gamma_multiply(strength));
    let (width, height) = (galley.size().x, galley.size().y);
    let left = at.x - (width + 13.0) / 2.0;
    let top = at.y - height;
    let dark = Color32::from_black_alpha((150.0 * strength) as u8);
    // The clock: a ring and two hands, shadowed.
    let centre = Pos2::new(left + 4.5, top + height * 0.55);
    for (offset, tone) in [(egui::vec2(1.0, 1.0), dark), (egui::Vec2::ZERO, colour.gamma_multiply(strength))] {
        let c = centre + offset;
        painter.circle_stroke(c, 4.2, Stroke::new(1.2, tone));
        painter.line_segment([c, c + egui::vec2(0.0, -2.7)], Stroke::new(1.1, tone));
        painter.line_segment([c, c + egui::vec2(2.0, 0.6)], Stroke::new(1.1, tone));
    }
    let digits = Pos2::new(left + 13.0, top);
    painter.galley_with_override_text_color(digits + egui::vec2(1.0, 1.0), galley.clone(), dark);
    painter.galley(digits, galley, colour);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> UnitId {
        UnitId { slot: 4, generation: 1 }
    }

    #[test]
    fn the_time_is_shown_as_a_clock() {
        assert_eq!(clock_text(0.0), "0:00");
        assert_eq!(clock_text(0.2), "0:01", "a started second counts");
        assert_eq!(clock_text(42.0), "0:42");
        assert_eq!(clock_text(65.0), "1:05");
        assert_eq!(clock_text(3725.0), "1:02:05");
    }

    /// A build followed tick by tick: its builders' pace, and the share of it the economy lets
    /// through.
    struct Run {
        etas: Etas,
        done: f32,
        now: u64,
    }

    impl Run {
        fn step(&mut self, pace: f32, share: f32, ticks: u64) -> Option<Eta> {
            let mut last = None;
            for _ in 0..ticks {
                self.now += 1;
                self.done += pace * share / 30.0;
                last = self.etas.of(id(), self.now, self.done, pace, self.now as f32 / 30.0);
            }
            last
        }
    }

    /// Builders at a pace of a tenth of the build a second, the economy letting all through: ten
    /// seconds in all. Joined by another, the time left halves at once; a stall lengthens it,
    /// learned over a second or two; a dry store says so; the last seconds fade it out.
    #[test]
    fn the_time_left_follows_the_builders_and_the_economy() {
        let mut run = Run { etas: Etas::default(), done: 0.0, now: 0 };
        // The first look: all the builders can do.
        let Some(Eta::Left(left, fade)) = run.step(0.1, 1.0, 1) else { panic!("no time") };
        assert!((left - 10.0).abs() < 0.2 && fade < 0.1, "{left} {fade}");
        // A second later it is visible, and about right.
        let Some(Eta::Left(left, fade)) = run.step(0.1, 1.0, 30) else { panic!() };
        assert!((left - 9.0).abs() < 0.2, "{left}");
        assert!(fade > 0.9, "{fade}");
        // A second builder joins: twice the pace, and the time halves (nothing to learn).
        let Some(Eta::Left(joined, _)) = run.step(0.2, 1.0, 1) else { panic!() };
        assert!((joined - left / 2.0).abs() < 0.3, "{left} then {joined}");
        // The economy lets half through: slower, learned over a second or two.
        let Some(Eta::Left(early, _)) = run.step(0.2, 0.5, 5) else { panic!() };
        let (alone, stalled) = ((1.0 - run.done) / 0.2, (1.0 - run.done) / (0.2 * 0.5));
        assert!(early < (alone + stalled) / 2.0, "learned over a second or two, not at once: {early} between {alone} and {stalled}");
        let Some(Eta::Left(late, _)) = run.step(0.2, 0.5, 120) else { panic!() };
        let stalled = (1.0 - run.done) / (0.2 * 0.5);
        assert!((late - stalled).abs() < stalled * 0.1, "{late} against {stalled}");
        // A dry store: nothing comes of it.
        assert!(matches!(run.step(0.2, 0.0, 300), Some(Eta::Stalled(_))));
        // The last seconds fade it out.
        let mut etas = Etas::default();
        let Some(Eta::Left(left, fade)) = etas.of(id(), 1, 0.97, 0.1, 10.0) else { panic!() };
        assert!(left < 0.5 && fade < 0.1, "{left} {fade}");
    }

    #[test]
    fn nobody_building_it_says_nothing_and_a_rewound_battle_starts_over() {
        let mut etas = Etas::default();
        assert_eq!(etas.of(id(), 10, 0.3, 0.0, 1.0), None);
        assert!(etas.of(id(), 10, 0.3, 0.1, 1.0).is_some());
        // Time stands still (paused): no change, no problem.
        assert!(etas.of(id(), 10, 0.3, 0.1, 5.0).is_some());
        // Time went back: it begins again, its fade too.
        let Some(Eta::Left(_, fade)) = etas.of(id(), 5, 0.2, 0.1, 6.0) else { panic!() };
        assert_eq!(fade, 0.0);
        etas.forget_old(100.0);
        assert!(etas.tracks.is_empty());
    }
}
