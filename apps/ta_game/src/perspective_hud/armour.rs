//! A ground vehicle's, ship's, gun house's or submarine's sight, after a main battle tank's: a
//! reticle with the weapons' readiness on arcs round it and a ring on the picture where the gun
//! points; beside it, small, the hull seen from above under its turret (a ship's: the gun
//! director, its guns' train and bearing); and, while nothing is bracketed, the range along the
//! line of sight as a laser range finder reads it.
//!
//! Over the world in the picture's own pixels (a unit is a pixel of a 1080-line picture), in the
//! green of the cockpit's screens with a dark halo. The unit's state (its hull, speed and weapons in
//! full) is on the console's screens.

use egui::{Align2, Pos2, Rect, Vec2, pos2, vec2};
use ta_ui::i18n::t;

use super::data::{Readouts, ground_range};
use super::kit::{Pen, clock};
use super::sight::Window;
use super::{PHOSPHOR, View, sight};
use crate::game::Game;
use crate::perspective::{Cockpit, Perspective};
use crate::theme::{AMBER, AMBER_BRIGHT};

/// How far the range finder reaches (world units).
const REACH: f32 = 2600.0;

/// The sight over the world. Returns the room its marks take (what else is written over the world
/// keeps off it).
pub(super) fn sight(pen: &Pen, view: &View, window: &Window, p: &Perspective, game: &Game, rd: &Readouts, now: f64) -> Vec<Rect> {
    let c = view.rect.center();
    // (A low window, a gun house's slit or a ship's bridge, gets a smaller sight.)
    let pen = pen.scaled((window.rect.height() / pen.s(560.0)).clamp(0.75, 1.0));
    let periscope = p.cockpit == Cockpit::Periscope;
    // (Through the magnified sight, or the periscope, their own cross-hairs are the reticle.)
    let own = p.zoom > 0 || periscope;
    if !own {
        reticle(&pen, c);
    }
    gun_cue(&pen, view, p);
    let ring = if own { 58.0 } else { 70.0 };
    sight::readiness(&pen, c, ring, p, game, now, false);
    let mut room = vec![Rect::from_min_max(c - vec2(pen.s(100.0), pen.s(ring + 22.0)), c + vec2(pen.s(100.0), pen.s(if own { ring + 22.0 } else { 134.0 })))];
    // Beside it: the hull under the turret, or a ship's gun director, its figures and all inside
    // the window (a bridge's sill stands close under the middle of the view).
    let beside = c + vec2(pen.s(170.0), pen.s(82.0));
    let beside = pos2(beside.x, beside.y.min(window.rect.bottom() - pen.s(1.25 * 60.0)).max(c.y));
    match p.cockpit {
        Cockpit::Bridge => room.push(director(&pen.scaled(1.25), beside, rd)),
        Cockpit::Periscope => {}
        _ => room.push(hull(&pen.scaled(1.25), beside, rd, !rd.mobile)),
    }
    // The range finder, while nothing is bracketed (the bracket gives the target's range).
    if p.target(game).is_none() {
        let range = ground_range(game, p.eye, p.forward, REACH);
        let at = c + vec2(pen.s(if own { 70.0 } else { 104.0 }), -pen.s(if own { 46.0 } else { 9.0 }));
        let label = pen.text(at, Align2::LEFT_BOTTOM, t("RNG"), 13.0, PHOSPHOR.gamma_multiply(0.8));
        let figure = match range {
            Some(r) => format!("{:04}", r.round().min(9999.0) as i32),
            None => "----".to_string(),
        };
        let figure = pen.text(pos2(label.right() + pen.s(5.0), at.y + pen.s(1.5)), Align2::LEFT_BOTTOM, figure, 18.0, if range.is_some() { AMBER_BRIGHT } else { PHOSPHOR.gamma_multiply(0.6) });
        room.push(label.union(figure));
    }
    room
}

/// The sight's reticle: an open ring, a cross with mil marks along it and a stadia ladder under it.
fn reticle(pen: &Pen, c: Pos2) {
    let col = PHOSPHOR;
    let ring = 56.0;
    for k in 0..4 {
        let mid = (k as f32 * 90.0 + 45.0).to_radians();
        pen.arc(c, ring, mid - 25f32.to_radians(), mid + 25f32.to_radians(), 1.6, col.gamma_multiply(0.55));
    }
    for sd in [-1.0f32, 1.0] {
        pen.line(c + vec2(sd * pen.s(14.0), 0.0), c + vec2(sd * pen.s(92.0), 0.0), 2.0, col);
        pen.line(c + vec2(sd * pen.s(92.0), -pen.s(8.0)), c + vec2(sd * pen.s(92.0), pen.s(8.0)), 2.0, col);
        for x in [34.0, 54.0, 74.0] {
            pen.line(c + vec2(sd * pen.s(x), 0.0), c + vec2(sd * pen.s(x), pen.s(6.0)), 1.8, col);
        }
    }
    pen.line(c + vec2(0.0, -pen.s(14.0)), c + vec2(0.0, -pen.s(42.0)), 2.0, col);
    pen.line(c + vec2(0.0, pen.s(14.0)), c + vec2(0.0, pen.s(130.0)), 2.0, col);
    for (y, half) in [(36.0, 10.0), (62.0, 7.0), (92.0, 10.0), (122.0, 7.0)] {
        pen.line(c + vec2(-pen.s(half), pen.s(y)), c + vec2(pen.s(half), pen.s(y)), 1.8, col);
    }
    pen.dot(c, 2.8, AMBER_BRIGHT);
}

/// Where the gun points, as a ring on the picture.
fn gun_cue(pen: &Pen, view: &View, p: &Perspective) {
    let Some(dir) = p.gun_dir else { return };
    if let Some(m) = view.project(p.eye + dir * 800.0)
        && view.rect.contains(m)
    {
        pen.circle(m, 8.0, 1.6, AMBER);
        pen.line(m + vec2(-pen.s(15.0), 0.0), m + vec2(-pen.s(8.0), 0.0), 1.6, AMBER);
        pen.line(m + vec2(pen.s(15.0), 0.0), m + vec2(pen.s(8.0), 0.0), 1.6, AMBER);
    }
}

/// `v` (screen points, y down) turned `deg` degrees clockwise.
fn turned(v: Vec2, deg: f32) -> Vec2 {
    let (s, c) = deg.to_radians().sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// The hull (a building's base: `fixed`) seen from above with its turret, the view's way up: the
/// hull turned as far as the eye looks off its heading, the gun as it points. Small, beside the
/// reticle: which way the vehicle faces, and its gun, at a glance. Returns the room it takes.
fn hull(pen: &Pen, c: Pos2, rd: &Readouts, fixed: bool) -> Rect {
    let hull = -rd.twist;
    let at = |x: f32, y: f32| c + turned(vec2(pen.s(x), pen.s(y)), hull);
    // The view's way, a caret over it.
    pen.fill(vec![c + vec2(0.0, -pen.s(30.0)), c + vec2(-pen.s(5.0), -pen.s(37.0)), c + vec2(pen.s(5.0), -pen.s(37.0))], PHOSPHOR.gamma_multiply(0.8));
    let body = PHOSPHOR.gamma_multiply(0.75);
    if fixed {
        pen.path(vec![at(-14.0, -14.0), at(14.0, -14.0), at(14.0, 14.0), at(-14.0, 14.0), at(-14.0, -14.0)], 1.6, body);
    } else {
        // Tracks either side, the deck between, a chevron at the bow.
        for sx in [-1.0f32, 1.0] {
            pen.path(vec![at(sx * 9.0, -17.0), at(sx * 14.0, -17.0), at(sx * 14.0, 17.0), at(sx * 9.0, 17.0), at(sx * 9.0, -17.0)], 1.5, body);
        }
        pen.path(vec![at(-8.0, -17.0), at(0.0, -24.0), at(8.0, -17.0)], 1.5, PHOSPHOR);
    }
    // The turret, and the gun as it points (else along the view).
    let gun = rd.turret.map_or(0.0, |az| az + hull);
    pen.circle(c, 6.0, 1.6, AMBER);
    pen.line(c + turned(vec2(0.0, -pen.s(6.0)), gun), c + turned(vec2(0.0, -pen.s(26.0)), gun), 3.0, AMBER_BRIGHT);
    Rect::from_center_size(c, Vec2::splat(pen.s(64.0))).union(Rect::from_center_size(c - vec2(0.0, pen.s(34.0)), Vec2::splat(pen.s(12.0))))
}

/// A ship's gun director: the ship seen from above, the view's way up, its guns' train on it, and
/// their bearing off the bow in figures. Returns the room it takes.
fn director(pen: &Pen, c: Pos2, rd: &Readouts) -> Rect {
    let hull = -rd.twist;
    let r = 30.0;
    pen.circle(c, r, 1.2, PHOSPHOR.gamma_multiply(0.6));
    for k in 0..12 {
        let d = clock((k as f32 * 30.0).to_radians());
        pen.line(c + d * pen.s(r), c + d * pen.s(r - if k % 3 == 0 { 6.0 } else { 3.5 }), 1.2, PHOSPHOR.gamma_multiply(0.75));
    }
    // The ship: long and pointed at the bow.
    let at = |x: f32, y: f32| c + turned(vec2(pen.s(x), pen.s(y)), hull);
    pen.path(vec![at(0.0, -22.0), at(6.0, -10.0), at(6.0, 16.0), at(3.0, 20.0), at(-3.0, 20.0), at(-6.0, 16.0), at(-6.0, -10.0), at(0.0, -22.0)], 1.5, PHOSPHOR.gamma_multiply(0.85));
    let train = rd.turret.unwrap_or(0.0);
    pen.line(c, c + turned(vec2(0.0, -pen.s(r - 3.0)), train + hull), 2.6, AMBER_BRIGHT);
    pen.dot(c, 3.0, AMBER);
    // Its bearing off the bow, as a ship gives it: degrees round from the bow to starboard.
    let below = pos2(c.x, c.y + pen.s(r + 13.0));
    let label = pen.text(below - vec2(pen.s(2.0), 0.0), Align2::RIGHT_CENTER, t("BRG"), 11.0, PHOSPHOR.gamma_multiply(0.8));
    let figure = pen.text(pos2(label.right() + pen.s(6.0), below.y), Align2::LEFT_CENTER, format!("{:03.0}°", train.round().rem_euclid(360.0)), 14.0, AMBER_BRIGHT);
    Rect::from_center_size(c, Vec2::splat(pen.s(2.0 * r + 4.0))).union(label).union(figure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_goes_clockwise_on_the_screen() {
        let close = |a: Vec2, b: Vec2| (a - b).length() < 1e-4;
        // Up turned a quarter clockwise is to the right; down, half a turn.
        assert!(close(turned(vec2(0.0, -1.0), 90.0), vec2(1.0, 0.0)));
        assert!(close(turned(vec2(0.0, -1.0), 180.0), vec2(0.0, 1.0)));
        assert!(close(turned(vec2(0.0, -1.0), -90.0), vec2(-1.0, 0.0)));
    }
}
