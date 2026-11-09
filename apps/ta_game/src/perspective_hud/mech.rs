//! A kbot's glass, after a battle mech's: the reticle with the weapon groups' readiness on arcs
//! round it (each numbered), and the torso twist as a short arc under it: how far the eye looks
//! off the legs' way. Over the world in the picture's own pixels (a unit is a pixel of a 1080-line
//! picture). The walker's state (its speed, its armour by section, its weapons in full, the
//! radar) is on the console's screens.

use egui::{Pos2, Rect, Vec2, vec2};

use super::data::Readouts;
use super::kit::{Pen, clock};
use super::{PHOSPHOR, View, sight};
use crate::game::Game;
use crate::perspective::Perspective;
use crate::theme::{AMBER, AMBER_BRIGHT};

/// The glass over the world. Returns the room its marks take (what else is written over the world
/// keeps off it).
pub(super) fn hud(pen: &Pen, view: &View, p: &Perspective, game: &Game, rd: &Readouts, now: f64) -> Vec<Rect> {
    // (A mech's marks are a little larger than the other crews'.)
    let pen = pen.scaled(1.1);
    let c = view.rect.center();
    // (Through the magnified sight, its own cross-hairs are the reticle.)
    if p.zoom > 0 {
        sight::readiness(&pen, c, 60.0, p, game, now, true);
        return vec![Rect::from_center_size(c, Vec2::splat(pen.s(160.0)))];
    }
    reticle(&pen, c);
    sight::readiness(&pen, c, 40.0, p, game, now, true);
    twist(&pen, rd, c);
    vec![Rect::from_min_max(c - vec2(pen.s(72.0), pen.s(62.0)), c + vec2(pen.s(72.0), pen.s(TWIST_RADIUS + 14.0)))]
}

/// The reticle: a ring with a pip, ticks outside it and brackets either side.
fn reticle(pen: &Pen, c: Pos2) {
    let col = PHOSPHOR;
    pen.circle(c, 22.0, 1.6, col);
    for k in 0..4 {
        let d = clock(k as f32 * std::f32::consts::FRAC_PI_2);
        pen.line(c + d * pen.s(22.0), c + d * pen.s(31.0), 1.6, col);
    }
    pen.dot(c, 2.2, AMBER_BRIGHT);
    for sd in [-1.0f32, 1.0] {
        let a = pen.s(60.0);
        pen.path(vec![c + vec2(sd * a, -pen.s(14.0)), c + vec2(sd * (a + pen.s(8.0)), -pen.s(14.0)), c + vec2(sd * (a + pen.s(8.0)), pen.s(14.0)), c + vec2(sd * a, pen.s(14.0))], 1.6, col.gamma_multiply(0.7));
    }
}

/// How far the twist arc reaches either side of its middle (clock degrees), the twist that takes
/// its marks there, and how far it is from the reticle's middle (units).
const ARC: f32 = 34.0;
const ARC_TWIST: f32 = 120.0;
const TWIST_RADIUS: f32 = 70.0;

/// Where a twist (degrees, the eye to the right of the legs positive) puts the legs on the arc
/// (clock degrees round the reticle; the arc's middle, straight down, is where the eye looks): the
/// legs off to the other side, as far as the arc reaches.
fn legs_on_arc(twist: f32) -> f32 {
    180.0 + (twist / ARC_TWIST * ARC).clamp(-ARC, ARC)
}

/// The torso twist under the reticle: a short arc whose middle is where the eye looks, the legs'
/// way marked on it (they stay where they point while the torso turns), and the torso's gun, where
/// it has turned elsewhere.
fn twist(pen: &Pen, rd: &Readouts, c: Pos2) {
    let r = TWIST_RADIUS;
    let (from, to) = ((180.0 - ARC).to_radians(), (180.0 + ARC).to_radians());
    pen.arc(c, r, from, to, 1.4, PHOSPHOR.gamma_multiply(0.7));
    // Ticks every 30 degrees of twist, the middle's longer.
    for deg in [-120.0f32, -90.0, -60.0, -30.0, 0.0, 30.0, 60.0, 90.0, 120.0] {
        let a = (180.0 + deg / ARC_TWIST * ARC).to_radians();
        let long = if deg == 0.0 { 9.0 } else if deg.abs() == 90.0 { 6.0 } else { 4.0 };
        pen.line(c + clock(a) * pen.s(r), c + clock(a) * pen.s(r + long), 1.3, PHOSPHOR.gamma_multiply(0.8));
    }
    // The legs: two short bars across the arc, pointing the way they walk.
    let at = legs_on_arc(rd.twist).to_radians();
    let d = clock(at);
    let side = vec2(-d.y, d.x);
    let base = c + d * pen.s(r);
    for k in [-1.0f32, 1.0] {
        let q = base + side * pen.s(3.2 * k);
        pen.line(q - d * pen.s(5.0), q + d * pen.s(6.0), 2.4, AMBER_BRIGHT);
    }
    // Past the arc's end, an arrow says the legs are further round.
    if rd.twist.abs() > ARC_TWIST {
        let way = if rd.twist > 0.0 { 1.0 } else { -1.0 };
        let tip = base + side * pen.s(-way * 12.0);
        pen.fill(vec![tip, tip + side * pen.s(way * 6.0) + d * pen.s(4.0), tip + side * pen.s(way * 6.0) - d * pen.s(4.0)], AMBER_BRIGHT);
    }
    // The torso's gun, if it is not where the eye looks.
    if let Some(az) = rd.turret.filter(|az| (az - rd.twist).abs() > 6.0) {
        let g = clock((180.0 + ((rd.twist - az) / ARC_TWIST * ARC).clamp(-ARC, ARC)).to_radians());
        pen.line(c + g * pen.s(r - 7.0), c + g * pen.s(r + 1.0), 2.0, AMBER);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legs_go_the_other_way_round_the_arc_from_the_twist() {
        // Looking straight along the legs, they are in the arc's middle, under the reticle.
        assert_eq!(legs_on_arc(0.0), 180.0);
        // The eye turned right: the legs point off to its left, round the arc towards the left
        // (clock angles past 180 are on the left).
        assert!(legs_on_arc(30.0) > 180.0);
        assert!(legs_on_arc(-30.0) < 180.0);
        // No further than the arc's ends, however far round.
        assert_eq!(legs_on_arc(170.0), 180.0 + ARC);
        assert_eq!(legs_on_arc(-170.0), 180.0 - ARC);
    }
}
