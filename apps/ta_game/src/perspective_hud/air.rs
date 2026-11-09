//! An aircraft's displays, after a modern fighter's: the flight symbology projected on the
//! helmet's visor (a heading tape, a pitch ladder, the flight path marker, the gun cross with the
//! weapons' readiness round it, speed and height on tapes with their figures boxed, a bank scale),
//! and the glass panoramic display on the panel under it (`glass_pcd` in the parent module draws its
//! three panes: the aircraft's page, the radar, the weapons in full). The flight's figures are on
//! the helmet only; the airframe's state, on the panel only.
//!
//! The symbology is drawn over the world in the picture's own pixels (a unit is a pixel of a
//! 1080-line picture), in the green of the cockpit's screens, with a dark halo to be read over sky
//! and ground.

use egui::{Align2, Color32, Painter, Pos2, Rect, pos2, vec2};
use glam::Vec3;
use ta_ui::i18n::t;

use super::data::{Readouts, bearing_of};
use super::kit::{Pen, clock};
use super::{PHOSPHOR, View};
use crate::game::Game;
use crate::perspective::Perspective;
use crate::theme::{AMBER_BRIGHT, RED};

/// How a tape is scaled: units of the picture to a value's unit, and the values a tick and a
/// labelled tick are apart.
struct TapeScale {
    per_value: f32,
    step: f32,
    major: f32,
}

/// The heading tape across the top of the view: bearings sliding past a fixed caret, the figure
/// boxed over it, the nose's heading and the target's bearing marked. Returns where it ends (points
/// down the screen).
pub(super) fn heading_tape(painter: &Painter, view: &View, p: &Perspective, game: &Game, rd: &Readouts, on: f32, top: f32) -> f32 {
    let rect = view.rect;
    let pen = Pen::new(painter, rect.height() / 1080.0, true, on);
    let cx = rect.center().x;
    let width = rect.width() * p.cockpit.tape_width();
    let span = 70.0 * p.cockpit.tape_width() / crate::perspective::TAPE_WIDTH;
    let look = rd.look;
    // The figure, boxed, over the tape's middle.
    let b = Rect::from_center_size(pos2(cx, top + pen.s(14.0)), vec2(pen.s(70.0), pen.s(26.0)));
    pen.painter.rect_filled(b, 0.0, Color32::from_black_alpha((110.0 * pen.on) as u8));
    pen.frame(b, 1.5, PHOSPHOR);
    pen.text(b.center(), Align2::CENTER_CENTER, format!("{:03.0}", look.round().rem_euclid(360.0)), 21.0, AMBER_BRIGHT);
    let base = b.bottom() + pen.s(8.0);
    pen.line(pos2(cx - width * 0.5, base), pos2(cx + width * 0.5, base), 1.2, PHOSPHOR.gamma_multiply(0.7));
    let mut deg = ((look - span * 0.5) / 5.0).ceil() as i32 * 5;
    while (deg as f32) <= look + span * 0.5 {
        let x = cx + (deg as f32 - look) / span * width;
        let d = deg.rem_euclid(360);
        let major = d % 10 == 0;
        let edge = (1.0 - ((x - cx).abs() / (width * 0.5)).powi(3)).clamp(0.0, 1.0);
        pen.line(pos2(x, base), pos2(x, base + pen.s(if major { 12.0 } else { 7.0 })), 1.7, PHOSPHOR.gamma_multiply(edge));
        if major {
            let label = match d {
                0 => t("N").to_string(),
                90 => t("E").to_string(),
                180 => t("S").to_string(),
                270 => t("W").to_string(),
                _ => format!("{:02}", d / 10),
            };
            pen.text(pos2(x, base + pen.s(26.0)), Align2::CENTER_CENTER, label, 15.0, PHOSPHOR.gamma_multiply(edge));
        }
        deg += 5;
    }
    // The caret the tape reads against, and where the nose points and the guns' target lies.
    pen.fill(vec![pos2(cx, base + pen.s(1.0)), pos2(cx - pen.s(6.0), base - pen.s(9.0)), pos2(cx + pen.s(6.0), base - pen.s(9.0))], AMBER_BRIGHT);
    let at = |bearing: f32| cx + (super::data::bearing_delta(look, bearing) / span * width).clamp(-width * 0.5, width * 0.5);
    let nose = bearing_of(p.body.transform_vector3(Vec3::Z).x, p.body.transform_vector3(Vec3::Z).z);
    let x = at(nose);
    pen.fill(vec![pos2(x, base + pen.s(40.0)), pos2(x - pen.s(6.0), base + pen.s(48.0)), pos2(x + pen.s(6.0), base + pen.s(48.0))], PHOSPHOR);
    if let Some(target) = p.target(game).and_then(|id| game.scene.unit_position(id, game.alpha())) {
        let d = target - p.eye;
        let x = at(bearing_of(d.x, d.z));
        pen.frame(Rect::from_center_size(pos2(x, base + pen.s(7.0)), vec2(pen.s(10.0), pen.s(14.0))), 1.4, RED);
    }
    base + pen.s(50.0)
}

/// The symbology over the world. `top`: where the heading tape ended. Returns the room its tapes,
/// gun cross and bank scale take (what else is written over the world keeps off it).
#[allow(clippy::too_many_arguments)]
pub(super) fn symbology(painter: &Painter, view: &View, p: &Perspective, game: &Game, rd: &Readouts, on: f32, top: f32, now: f64) -> Vec<Rect> {
    let rect = view.rect;
    let pen = Pen::new(painter, rect.height() / 1080.0, true, on);
    let sill = p.sill_y.map_or(rect.bottom(), |y| y / view.ppp);
    let (top, bottom) = (top + pen.s(8.0), (sill - pen.s(14.0)).min(rect.bottom() - pen.s(10.0)));
    let centre = rect.center();
    // (The tapes stand in the middle of the band over the bank scale.)
    let mid = (top + (bottom - pen.s(100.0))) * 0.5 + pen.s(6.0);
    let clipped = painter.with_clip_rect(Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), bottom)));
    let mut room = Vec::new();
    // (From behind the aircraft the eye is not in it: no ladder, no flight path, no gun cross.)
    if !p.chase {
        ladder(&Pen::new(&clipped, pen.u, true, on), view, p);
        if let Some(nose) = path_marker(&pen, view, p) {
            // The weapons' readiness round the gun cross: the guns fire where the nose points.
            super::sight::readiness(&pen, nose, 30.0, p, game, now, false);
            room.push(Rect::from_center_size(nose, egui::Vec2::splat(pen.s(96.0))));
        }
    }
    // How far from the middle the tapes stand: as far as the window allows.
    let reach = (rect.width() * 0.5 - pen.s(60.0)).min(pen.s(560.0));
    let x = reach.min(pen.s(384.0));
    tape(&pen, pos2(centre.x - x, mid), -1.0, rd.speed.max(0.0).round(), TapeScale { per_value: 1.35, step: 10.0, major: 50.0 }, t("KM/H").to_string());
    tape(&pen, pos2(centre.x + x, mid), 1.0, rd.altitude.max(0.0).round(), TapeScale { per_value: 3.0, step: 5.0, major: 20.0 }, t("ALT").to_string());
    for side in [-1.0f32, 1.0] {
        let line = centre.x + side * x;
        room.push(Rect::from_min_max(pos2(line.min(line + side * pen.s(104.0)), mid - pen.s(96.0)), pos2(line.max(line + side * pen.s(104.0)), mid + pen.s(110.0))));
    }
    // The bank scale low in the middle; from behind, or through the magnified sight, the painted
    // instruments stand along the foot.
    if !p.chase && p.zoom == 0 {
        // (Clear of the window's foot, where the threats' chevrons under the nose stand.)
        let c = pos2(centre.x, bottom - pen.s(66.0));
        bank_scale(&pen, c, rd);
        room.push(Rect::from_min_max(c - egui::vec2(pen.s(150.0), pen.s(70.0)), c + egui::vec2(pen.s(150.0), pen.s(24.0))));
    }
    room
}

/// The pitch ladder: the horizon and a rung every ten degrees, drawn on the world (its rungs'
/// middles in the view's way, whatever the aircraft's bank).
fn ladder(pen: &Pen, view: &View, p: &Perspective) {
    let az = p.forward.x.atan2(p.forward.z);
    let right = Vec3::new(az.cos(), 0.0, -az.sin());
    for k in -6..=6i32 {
        let pitch = (k as f32 * 10.0).to_radians();
        let dir = Vec3::new(az.sin() * pitch.cos(), pitch.sin(), az.cos() * pitch.cos());
        let (Some(a), Some(b)) = (view.project(p.eye + dir * 1000.0 - right * 120.0), view.project(p.eye + dir * 1000.0 + right * 120.0)) else { continue };
        if !view.rect.contains(a.lerp(b, 0.5)) {
            continue;
        }
        let along = (b - a).normalized();
        if k == 0 {
            // The horizon, broken over the middle.
            pen.line(a - along * pen.s(240.0), a.lerp(b, 0.34), 1.8, PHOSPHOR);
            pen.line(a.lerp(b, 0.66), b + along * pen.s(240.0), 1.8, PHOSPHOR);
            continue;
        }
        // (Hooks at a rung's ends point at the horizon.)
        let down = vec2(-along.y, along.x) * if k > 0 { 1.0 } else { -1.0 };
        for (from, to, end) in [(a, a.lerp(b, 0.4), a), (a.lerp(b, 0.6), b, b)] {
            if k > 0 {
                pen.line(from, to, 1.5, PHOSPHOR);
            } else {
                for q in 0..3 {
                    pen.line(from.lerp(to, q as f32 / 3.0), from.lerp(to, (q as f32 + 0.62) / 3.0), 1.5, PHOSPHOR);
                }
            }
            pen.line(end, end + down * pen.s(9.0), 1.5, PHOSPHOR);
        }
        for (side, at) in [(-1.0f32, a), (1.0, b)] {
            pen.text(at + along * side * pen.s(24.0), Align2::CENTER_CENTER, format!("{}", (k * 10).abs()), 13.0, PHOSPHOR);
        }
    }
}

/// The flight path marker where the aircraft is going (a ring with wings and a fin), and the
/// boresight cross where its nose points; returns where that is, in the picture.
fn path_marker(pen: &Pen, view: &View, p: &Perspective) -> Option<Pos2> {
    let v = p.velocity();
    if v.length() > 6.0
        && let Some(m) = view.project(p.eye + v.normalize() * 800.0)
    {
        pen.circle(m, 10.0, 2.0, PHOSPHOR);
        pen.line(m + vec2(-pen.s(10.0), 0.0), m + vec2(-pen.s(26.0), 0.0), 2.0, PHOSPHOR);
        pen.line(m + vec2(pen.s(10.0), 0.0), m + vec2(pen.s(26.0), 0.0), 2.0, PHOSPHOR);
        pen.line(m + vec2(0.0, -pen.s(10.0)), m + vec2(0.0, -pen.s(21.0)), 2.0, PHOSPHOR);
    }
    let nose = p.body.transform_vector3(Vec3::Z).normalize_or(Vec3::Z);
    let m = view.project(p.eye + nose * 800.0).filter(|m| view.rect.shrink(pen.s(40.0)).contains(*m))?;
    let s = pen.s(7.0);
    pen.line(m + vec2(-s, 0.0), m + vec2(s, 0.0), 1.4, AMBER_BRIGHT);
    pen.line(m + vec2(0.0, -s), m + vec2(0.0, s), 1.4, AMBER_BRIGHT);
    Some(m)
}

/// The ticks of a tape showing `value` with `half` units of the picture's height either side of the
/// middle: each tick's height over the middle (units, up positive), whether it is labelled and
/// the value it stands for; none below zero.
fn tape_marks(value: f32, scale: &TapeScale, half: f32) -> Vec<(f32, bool, i32)> {
    let first = ((value - half / scale.per_value) / scale.step).ceil() as i32;
    let last = ((value + half / scale.per_value) / scale.step).floor() as i32;
    (first..=last)
        .map(|i| i as f32 * scale.step)
        .filter(|v| *v >= 0.0)
        .map(|v| ((v - value) * scale.per_value, (v / scale.major - (v / scale.major).round()).abs() < 1e-3, v.round() as i32))
        .collect()
}

/// A vertical tape with the figure it reads boxed at its middle. `side`: -1 for a tape on the
/// left of the view, 1 on the right; its ticks point to the middle of the view.
fn tape(pen: &Pen, c: Pos2, side: f32, value: f32, scale: TapeScale, caption: String) {
    let half = 92.0;
    pen.line(c + vec2(0.0, -pen.s(half)), c + vec2(0.0, pen.s(half)), 1.4, PHOSPHOR.gamma_multiply(0.8));
    for (up, major, v) in tape_marks(value, &scale, half) {
        let y = c.y - up * pen.u;
        let fade = (1.0 - up.abs() / half).clamp(0.0, 1.0).sqrt();
        let col = PHOSPHOR.gamma_multiply(fade);
        pen.line(pos2(c.x, y), pos2(c.x - side * pen.s(if major { 14.0 } else { 8.0 }), y), 1.4, col);
        // (The label nearest the figure is under its box.)
        if major && up.abs() > 17.0 {
            let align = if side < 0.0 { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER };
            pen.text(pos2(c.x + side * pen.s(8.0), y), align, format!("{v}"), 13.0, col);
        }
    }
    // The boxed figure, with a pointer at the tape.
    let h = pen.s(30.0);
    let (inner, outer) = (c.x + side * pen.s(20.0), c.x + side * pen.s(98.0));
    let b = Rect::from_min_max(pos2(inner.min(outer), c.y - h / 2.0), pos2(inner.max(outer), c.y + h / 2.0));
    pen.painter.rect_filled(b, 0.0, Color32::from_black_alpha((120.0 * pen.on) as u8));
    pen.frame(b, 1.6, PHOSPHOR);
    pen.text(b.center(), Align2::CENTER_CENTER, format!("{}", value as i32), 24.0, AMBER_BRIGHT);
    pen.fill(vec![pos2(c.x + side * pen.s(9.0), c.y), pos2(inner, c.y - pen.s(6.0)), pos2(inner, c.y + pen.s(6.0))], PHOSPHOR);
    pen.text(pos2(b.center().x, b.bottom() + pen.s(11.0)), Align2::CENTER_CENTER, caption, 12.0, PHOSPHOR.gamma_multiply(0.8));
}

/// The bank scale: an arc over the middle of a small cluster, a tick every ten degrees, the bank
/// pointed at, with the pitch and the bank in figures under it.
fn bank_scale(pen: &Pen, c: Pos2, rd: &Readouts) {
    let r = 66.0;
    let to = 50f32.to_radians();
    pen.arc(c, r, -to, to, 1.3, PHOSPHOR.gamma_multiply(0.75));
    for deg in [-45, -30, -20, -10, 0, 10, 20, 30, 45] {
        let d = clock((deg as f32).to_radians());
        pen.line(c + d * pen.s(r), c + d * pen.s(r - if deg % 30 == 0 { 10.0 } else { 6.0 }), 1.4, PHOSPHOR.gamma_multiply(0.9));
    }
    // The pointer: a triangle inside the arc, at the bank (the right side down is to the right).
    let a = rd.roll.clamp(-55.0, 55.0).to_radians();
    let (d, side) = (clock(a), vec2(-clock(a).y, clock(a).x));
    let tip = c + d * pen.s(r - 12.0);
    pen.fill(vec![tip, tip - d * pen.s(10.0) + side * pen.s(6.0), tip - d * pen.s(10.0) - side * pen.s(6.0)], AMBER_BRIGHT);
    let level = |v: f32| if v.abs() < 0.5 { 0.0 } else { v };
    pen.text(c + vec2(0.0, pen.s(14.0)), Align2::CENTER_CENTER, format!("{} {:+03.0}°  {} {:+03.0}°", t("PITCH"), level(rd.pitch), t("ROLL"), level(rd.roll)), 13.0, PHOSPHOR);
}
