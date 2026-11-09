//! How the interface moves.
//!
//! Idle animations (a lamp's pulse, a gleam running across brass, a radar's sweep): drawn
//! smoothly while the window has the player's attention, and held still while it is in the
//! background or minimised, so that a window left open beside the game costs the game
//! nothing. The window says which once a frame ([`set_still`]); an animation asks for its
//! next frame with [`next_frame`] and draws its resting look when [`still`].
//!
//! Movements (a page coming in and going, a dialog opening and closing, a list building up, a
//! choice gliding to its place) share one set of curves and times, [`Pace`]: what comes in is
//! fast at first and settles ([`out_cubic`]), what goes gathers speed as it leaves
//! ([`in_cubic`]), and what moves from one place to another glides ([`in_out_cubic`]). Each is
//! a function of the time since it began, never a count of frames, so that a frame every 4 ms
//! and one every 33 ms draw the same pictures (and a capture on a fixed clock draws what the
//! game does). With reduced motion nothing travels: [`Pace::REDUCED`] moves nothing, and what
//! would have come in or gone is simply there or not.

use std::time::Duration;

use egui::{Context, Id};

// ---- Curves --------------------------------------------------------------------------------

pub fn clamp01(t: f32) -> f32 {
    if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Slow at both ends.
pub fn smooth(t: f32) -> f32 {
    let t = clamp01(t);
    t * t * (3.0 - 2.0 * t)
}

/// Fast at first, settling: what comes in.
pub fn out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - clamp01(t)).powi(3)
}

/// Slow at first, gathering: what goes out.
pub fn in_cubic(t: f32) -> f32 {
    clamp01(t).powi(3)
}

/// Slow away, fast across, slow to rest: what moves from one place to another.
pub fn in_out_cubic(t: f32) -> f32 {
    let t = clamp01(t);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// Comes in past its place and settles back (`s`: how far past: 1.70158 is the classic 10%).
pub fn out_back(t: f32, s: f32) -> f32 {
    let t = clamp01(t) - 1.0;
    1.0 + t * t * ((s + 1.0) * t + s)
}

/// How far on something is that began at `start` and takes `len` seconds, at `now` (0 before,
/// 1 after; at once when `len` is nothing).
pub fn progress(now: f64, start: f64, len: f32) -> f32 {
    if len <= 0.0 || !len.is_finite() {
        return if now >= start { 1.0 } else { 0.0 };
    }
    clamp01(((now - start) / len as f64) as f32)
}

// ---- Times ---------------------------------------------------------------------------------

/// How long the interface's movements take (seconds) and how far they travel: one set for every
/// page, dialog, list and control, so that they move alike.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pace {
    /// A page coming in (sliding from the side the player went), and the one it replaces going.
    pub page_in: f32,
    pub page_out: f32,
    /// A dialog opening (rising into place as the screen behind darkens), and closing.
    pub dialog_in: f32,
    pub dialog_out: f32,
    /// The wait between one entry of a list and the next as the list builds up.
    pub stagger: f32,
    /// A choice gliding to its place (a row of options' chosen one, a tab's marker).
    pub glide: f32,
    /// How far things travel, as a share of the full distance (0: they stand still).
    pub travel: f32,
}

impl Pace {
    pub const FULL: Pace = Pace { page_in: 0.26, page_out: 0.16, dialog_in: 0.2, dialog_out: 0.13, stagger: 0.03, glide: 0.18, travel: 1.0 };
    /// For those who asked for less motion: nothing travels and nothing waits; what comes in is
    /// there at once, and what goes is gone.
    pub const REDUCED: Pace = Pace { page_in: 0.0, page_out: 0.0, dialog_in: 0.0, dialog_out: 0.0, stagger: 0.0, glide: 0.0, travel: 0.0 };

    pub fn of(ctx: &Context) -> Pace {
        if reduced(ctx) { Pace::REDUCED } else { Pace::FULL }
    }
}

/// A value gliding to `target` over `len` seconds (on [`in_out_cubic`]) from wherever it was when
/// the target last changed; at the target at once the first time it is asked for, and with
/// reduced motion. Asks for frames while it moves.
pub fn glide(ctx: &Context, id: Id, target: f32, len: f32) -> f32 {
    #[derive(Clone, Copy)]
    struct Glide {
        from: f32,
        to: f32,
        since: f64,
    }
    let now = ctx.input(|i| i.time);
    let len = if reduced(ctx) { 0.0 } else { len };
    let key = id.with("ta_ui::motion::glide");
    let value = ctx.data_mut(|d| {
        let g = match d.get_temp::<Glide>(key) {
            None => Glide { from: target, to: target, since: now },
            Some(g) if (g.to - target).abs() > 1e-4 => {
                // (From where it is now, not from where it was going.)
                let at = lerp(g.from, g.to, in_out_cubic(progress(now, g.since, len)));
                Glide { from: at, to: target, since: now }
            }
            Some(g) => g,
        };
        d.insert_temp(key, g);
        lerp(g.from, g.to, in_out_cubic(progress(now, g.since, len)))
    });
    if (value - target).abs() > 1e-3 {
        ctx.request_repaint();
    }
    value
}

fn id() -> Id {
    Id::new("ta_ui::motion::still")
}

/// Keep the preference separate from focus: focusing a window must never restart
/// animations the player asked to hold still.
pub fn set_reduced(ctx: &Context, reduced: bool) {
    ctx.data_mut(|d| d.insert_temp(Id::new("ta_ui::motion::reduced"), reduced));
}

pub fn reduced(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp(Id::new("ta_ui::motion::reduced"))).unwrap_or(false)
}

/// The reduced-flashes preference: lights that flare, pulse or flash stay dim and steady.
pub fn set_reduced_flashes(ctx: &Context, reduced: bool) {
    ctx.data_mut(|d| d.insert_temp(Id::new("ta_ui::motion::reduced_flashes"), reduced));
}

pub fn reduced_flashes(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp(Id::new("ta_ui::motion::reduced_flashes"))).unwrap_or(false)
}

/// Holds the idle animations still (`true`), or lets them run.
pub fn set_still(ctx: &Context, still: bool) {
    ctx.data_mut(|d| d.insert_temp(id(), still));
}

/// Whether idle animations are held still.
pub fn still(ctx: &Context) -> bool {
    reduced(ctx) || ctx.data(|d| d.get_temp(id())).unwrap_or(false)
}

/// Asks for an idle animation's next frame `after` from now, unless they are held still.
pub fn next_frame(ctx: &Context, after: Duration) {
    if !still(ctx) {
        // (egui takes a frame's time off every delay, to land a frame on a deadline: 33 ms
        // asked for came every 16, twice the frames. An animation's frames are an interval
        // apart.)
        let frame = if after.is_zero() { Duration::ZERO } else { Duration::try_from_secs_f32(ctx.input(|i| i.predicted_dt)).unwrap_or_default() };
        ctx.request_repaint_after(after + frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every curve runs from 0 to 1, never leaves its range (the one that comes in past its place
    /// does, a little, and comes back) and never goes backwards.
    #[test]
    fn the_curves_run_from_nothing_to_all_without_going_back() {
        let curves: [(&str, fn(f32) -> f32); 4] = [("smooth", smooth), ("out", out_cubic), ("in", in_cubic), ("in-out", in_out_cubic)];
        for (name, f) in curves {
            assert_eq!(f(0.0), 0.0, "{name}");
            assert!((f(1.0) - 1.0).abs() < 1e-6, "{name}");
            assert_eq!(f(-3.0), 0.0, "{name} before");
            assert!((f(4.0) - 1.0).abs() < 1e-6, "{name} after");
            assert_eq!(f(f32::NAN), 0.0, "{name}: not a number is nothing");
            let mut last = 0.0;
            for k in 0..=100 {
                let v = f(k as f32 / 100.0);
                assert!(v >= last - 1e-6 && (0.0..=1.0 + 1e-6).contains(&v), "{name} at {k}");
                last = v;
            }
        }
        assert!(out_back(0.0, 1.1).abs() < 1e-6 && (out_back(1.0, 1.1) - 1.0).abs() < 1e-6);
        let top = (0..=100).map(|k| out_back(k as f32 / 100.0, 1.1)).fold(0.0, f32::max);
        assert!(top > 1.0 && top < 1.12, "it comes in a little past its place: {top}");
        // Fast then slow, slow then fast, and the glide half way at half time.
        assert!(out_cubic(0.3) > 0.3 && in_cubic(0.3) < 0.3);
        assert!((in_out_cubic(0.5) - 0.5).abs() < 1e-6 && in_out_cubic(0.2) < 0.2 && in_out_cubic(0.8) > 0.8);
    }

    #[test]
    fn progress_is_how_far_on_something_is() {
        assert_eq!(progress(1.0, 2.0, 0.5), 0.0);
        assert!((progress(2.25, 2.0, 0.5) - 0.5).abs() < 1e-6);
        assert_eq!(progress(9.0, 2.0, 0.5), 1.0);
        assert_eq!(progress(2.0, 2.0, 0.0), 1.0, "no time at all: done at once");
        assert_eq!(progress(1.0, 2.0, 0.0), 0.0);
        assert_eq!(progress(3.0, 2.0, f32::NAN), 1.0);
    }

    /// With reduced motion nothing travels or takes time; the full pace takes a moment for each,
    /// a page longer than a dialog, and a thing going sooner than one coming.
    #[test]
    fn reduced_motion_moves_nothing() {
        let ctx = Context::default();
        assert_eq!(Pace::of(&ctx), Pace::FULL);
        set_reduced(&ctx, true);
        let reduced = Pace::of(&ctx);
        assert_eq!(reduced, Pace::REDUCED);
        assert_eq!([reduced.page_in, reduced.page_out, reduced.dialog_in, reduced.dialog_out, reduced.stagger, reduced.glide, reduced.travel], [0.0; 7]);
        let full = Pace::FULL;
        assert!(full.page_in > full.dialog_in && full.page_out < full.page_in && full.dialog_out < full.dialog_in);
        assert!(full.page_in <= 0.3 && full.travel == 1.0, "quick: the player is waiting on it");
    }

    /// A glide starts where it is, goes to a new target on its curve from where it had got to, and
    /// is there at once with reduced motion.
    #[test]
    fn a_glide_goes_from_where_it_is_to_where_it_is_told() {
        let ctx = Context::default();
        let id = Id::new("chip");
        let at = |time: f64, target: f32| {
            let mut v = 0.0;
            ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| v = glide(ui.ctx(), id, target, 0.2)).drop_without_applying_deltas();
            v
        };
        assert_eq!(at(0.0, 10.0), 10.0, "the first time: where it is told");
        assert_eq!(at(0.1, 10.0), 10.0);
        assert_eq!(at(1.0, 30.0), 10.0, "a new target: from where it was");
        let half = at(1.1, 30.0);
        assert!((half - 20.0).abs() < 1e-3, "half way at half time: {half}");
        // Turned round half way: from where it had got to.
        let turned = at(1.15, 0.0);
        assert!(turned > 20.0 && turned < 30.0, "{turned}");
        assert!(at(1.25, 0.0) < turned && at(1.5, 0.0) == 0.0);
        set_reduced(&ctx, true);
        assert_eq!(at(2.0, 50.0), 50.0, "reduced motion: there at once");
    }

    #[test]
    fn reduced_motion_survives_focus_changes() {
        let ctx = Context::default();
        set_reduced(&ctx, true);
        for unfocused in [false, true, false] {
            set_still(&ctx, unfocused);
            assert!(still(&ctx));
        }
        set_reduced(&ctx, false);
        assert!(!still(&ctx));
        set_still(&ctx, true);
        assert!(still(&ctx));
    }

    #[test]
    fn reduced_flashes_are_their_own_preference() {
        let ctx = Context::default();
        assert!(!reduced_flashes(&ctx));
        set_reduced_flashes(&ctx, true);
        assert!(reduced_flashes(&ctx));
        // (It holds no idle animation still, and the other setting does not touch it.)
        assert!(!still(&ctx) && !reduced(&ctx));
        set_reduced(&ctx, true);
        set_reduced_flashes(&ctx, false);
        assert!(reduced(&ctx) && !reduced_flashes(&ctx));
    }

    #[test]
    fn an_animation_asks_for_its_frames_as_far_apart_as_it_says() {
        let ctx = Context::default();
        let delay = |after: u64| {
            let raw = egui::RawInput { predicted_dt: 1.0 / 60.0, ..Default::default() };
            let mut output = ctx.run_ui(raw, |ui| next_frame(ui.ctx(), Duration::from_millis(after)));
            output.textures_delta.clear();
            output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
        };
        for _ in 0..4 {
            delay(1000);
        }
        let d = delay(33);
        assert!(d >= Duration::from_millis(32) && d <= Duration::from_millis(34), "{d:?}");
    }

    #[test]
    fn a_still_window_asks_for_no_frames() {
        let ctx = Context::default();
        let run = |still: bool| {
            set_still(&ctx, still);
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| next_frame(ui.ctx(), Duration::from_millis(33)));
            output.textures_delta.clear();
            output.viewport_output[&egui::ViewportId::ROOT].repaint_delay < Duration::from_secs(1)
        };
        // (egui's first frames ask for more of their own: fonts, layout.)
        for _ in 0..4 {
            run(true);
        }
        assert_eq!([run(false), run(true), run(false)], [true, false, true]);
    }
}
