//! What every crew's glass shares, over the world in the picture's own pixels (a unit is a pixel
//! of a 1080-line picture): the weapons' readiness on arcs hugging the reticle, the target
//! bracketed where it stands (its name and range in one line, its hull along the bracket's foot,
//! the bracket closing up on it when it is destroyed), the enemies out of sight as chevrons round
//! the window's edge pointing the way to them, and one line of messages, low.
//!
//! The glass carries only what the eyes need while looking out: the unit's own state is on the
//! console's screens (or, where none shows, on panels along the picture's edge).

use egui::{Align2, Color32, Mesh, Pos2, Rect, Shape, Vec2, pos2, vec2};
use glam::Vec3;
use ta_sim::{Fx, Seen, Target};
use ta_ui::i18n::t;

use super::kit::{Pen, clock};
use super::{PHOSPHOR, PHOSPHOR_DIM, View};
use crate::game::Game;
use crate::palette::health_color;
use crate::perspective::{Marked, Perspective, WeaponState};
use crate::theme::{AMBER, AMBER_BRIGHT, AMBER_DIM, GREEN, RED};

/// The window the crew looks out through, as the glass has it this frame: what is written over
/// the world keeps inside it.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub rect: Rect,
    /// Round (a periscope's field, the magnified sight): the threats' ring is a circle in it.
    pub round: bool,
}

impl Window {
    /// The ellipse the threats' chevrons stand on: inside the window, clear of its edge.
    fn ring(&self, pen: &Pen) -> (Pos2, Vec2) {
        let r = self.rect;
        let inset = pen.s(30.0);
        let radii = vec2((r.width() * 0.5 - inset).max(pen.s(60.0)), (r.height() * 0.5 - inset).max(pen.s(60.0)));
        let radii = if self.round { Vec2::splat(radii.x.min(radii.y)) } else { radii };
        (r.center(), radii)
    }
}

// ---- The readiness ring --------------------------------------------------------------------

/// An arc of the readiness ring: its middle (clock degrees, 0 up, clockwise), and which way it
/// fills (from the end nearer the foot of the reticle, or from the left).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Slot {
    middle: f32,
    /// Degrees from the end it fills from to the other: negative fills anticlockwise.
    sweep: f32,
}

/// Half an arc's span (degrees).
const HALF_ARC: f32 = 27.0;

/// Where each weapon's arc is round the reticle, by how many there are (up to three): the first
/// at the lower left, the second at the lower right, the third over the top; a lone weapon has
/// both lower arcs, mirrored. Clear of the reticle's own lines (they run to the four points).
fn slots(count: usize) -> Vec<Vec<Slot>> {
    let left = Slot { middle: 225.0, sweep: 2.0 * HALF_ARC };
    let right = Slot { middle: 135.0, sweep: -2.0 * HALF_ARC };
    let top = Slot { middle: 0.0, sweep: 2.0 * HALF_ARC };
    match count {
        0 => Vec::new(),
        1 => vec![vec![left, right]],
        2 => vec![vec![left], vec![right]],
        _ => vec![vec![left], vec![right], vec![top]],
    }
}

/// Where an arc starts filling (clock radians).
fn start(slot: Slot) -> f32 {
    (slot.middle - slot.sweep * 0.5).to_radians()
}

/// How a weapon's arc looks: lit (ready), filling (loading: amber) or starved (the D-gun without
/// the energy for a shot: red); a flash as it fires, a pulse as it comes ready.
fn state_colour(w: &WeaponState, starved: bool) -> Color32 {
    if starved {
        RED
    } else if w.ready {
        PHOSPHOR
    } else {
        AMBER
    }
}

/// The weapons' readiness on arcs round the reticle's middle `c`, `r` units out: one arc a weapon
/// (a lone weapon's on both lower arcs), filling as it reloads, lit when it is ready, flashing as
/// it fires. `numbered`: each arc has its weapon's number beside it (a mech's weapon groups).
#[allow(clippy::too_many_arguments)]
pub(super) fn readiness(pen: &Pen, c: Pos2, r: f32, p: &Perspective, game: &Game, now: f64, numbered: bool) {
    let weapons = p.weapons(game);
    let Some(u) = game.sim.unit(p.unit) else { return };
    let ty = game.sim.unit_type(u);
    let energy = game.sim.players.get(u.owner as usize).map_or(0.0, |pl| pl.economy.energy.to_f32());
    let layout = slots(weapons.len());
    let numbered = numbered || weapons.len() > 1;
    for (i, (w, arcs)) in weapons.iter().zip(&layout).enumerate() {
        let wt = ty.weapons.get(w.slot).copied().flatten().map(|id| game.sim.weapon_types.get(id));
        let cost = wt.filter(|wt| wt.command_fire && w.manual).map_or(0.0, |wt| game.sim.shot_energy(u.id, wt).to_f32());
        let starved = w.manual && cost > energy;
        let col = state_colour(w, starved);
        let fired = (now - p.hud.last_shot[w.slot.min(2)]) as f32;
        let came = (now - p.hud.ready_since[w.slot.min(2)]) as f32;
        // (A pulse as it comes ready, a flash as it fires.)
        let pulse = if w.ready && (0.0..0.35).contains(&came) { 1.0 - came / 0.35 } else { 0.0 };
        let flash = if (0.0..0.15).contains(&fired) { 1.0 - fired / 0.15 } else { 0.0 };
        let frac = if w.ready || starved { 1.0 } else { (1.0 - w.left).clamp(0.0, 1.0) };
        let width = 3.2 + 1.6 * pulse;
        for &slot in arcs {
            let a0 = start(slot);
            let a1 = a0 + slot.sweep.to_radians();
            // The track, dim; the fill over it.
            pen.arc(c, r, a0, a1, 3.2, PHOSPHOR_DIM.gamma_multiply(0.55));
            if frac > 0.0 {
                let lit = if flash > 0.0 { lerp(col, Color32::WHITE, flash * 0.8) } else { col };
                let lit = if starved { lit.gamma_multiply(0.6) } else { lit };
                pen.arc(c, r, a0, a0 + (a1 - a0) * frac, width, lit);
            }
            // A notch at the arc's ends, outwards: it reads as a gauge, not a ring.
            for a in [a0, a1] {
                pen.line(c + clock(a) * pen.s(r - 3.0), c + clock(a) * pen.s(r + 4.5), 1.2, PHOSPHOR.gamma_multiply(0.55));
            }
        }
        // Its number (and a stockpile's count) outside the arc's middle.
        if numbered && let Some(slot) = arcs.first() {
            let at = c + clock(slot.middle.to_radians()) * pen.s(r + 14.0);
            pen.text(at, Align2::CENTER_CENTER, format!("{}", i + 1), 13.0, if w.ready || starved { col } else { AMBER.gamma_multiply(0.85) });
        }
        if wt.is_some_and(|wt| wt.def.stockpile)
            && let Some(slot) = arcs.last()
        {
            let at = c + clock(slot.middle.to_radians()) * pen.s(r + 14.0) + vec2(0.0, pen.s(15.0));
            pen.text(at, Align2::CENTER_CENTER, format!("×{}", u.stockpile), 12.0, AMBER_BRIGHT);
        }
    }
}

// ---- The target and the other units in view -----------------------------------------------------

/// A unit's box on the screen (points): its foot's middle `at`, `radius` across and `height`
/// tall, as the eye sees it; none when any of it is behind the eye.
fn bounds(view: &View, at: Vec3, radius: f32, height: f32) -> Option<Rect> {
    let mut r = Rect::NOTHING;
    for (x, y, z) in [(-1.0, 0.0, -1.0), (1.0, 0.0, -1.0), (-1.0, 0.0, 1.0), (1.0, 0.0, 1.0), (-1.0, 1.0, -1.0), (1.0, 1.0, -1.0), (-1.0, 1.0, 1.0), (1.0, 1.0, 1.0)] {
        r.extend_with(view.project(at + Vec3::new(x * radius * 0.8, y * height, z * radius * 0.8))?);
    }
    Some(r)
}

/// Points in the world on the screen: the box round them (points); none when any is behind the
/// eye, or there are none.
fn projected_box(view: &View, points: &[Vec3]) -> Option<Rect> {
    let mut r = Rect::NOTHING;
    for &q in points {
        r.extend_with(view.project(q)?);
    }
    r.is_positive().then_some(r)
}

/// A unit's box on the screen (points), fitted to its model as it is drawn standing at `at` (its
/// pieces as its script has them, see `perspective::drawn_corners`); a box of its radius and
/// height for a model not loaded yet. None when any of it is behind the eye.
fn unit_box(view: &View, game: &Game, u: &ta_sim::Unit, at: Vec3, scratch: &mut Vec<Vec3>) -> Option<Rect> {
    crate::perspective::drawn_corners(game, u, at, scratch);
    if scratch.is_empty() {
        let ty = game.sim.unit_type(u);
        return bounds(view, at, ty.radius.to_f32().max(6.0), ty.model.height.to_f32().max(8.0));
    }
    projected_box(view, scratch)
}

/// How near (world units) the other units' brackets are fitted to their models as drawn.
const NEAR: f32 = 700.0;

/// A bracket's four corners round `r`, each `len` points long.
fn corners(pen: &Pen, r: Rect, len: f32, w: f32, col: Color32) {
    let l = len.min(r.width() * 0.45).min(r.height() * 0.45);
    for (corner, sx, sy) in [(r.left_top(), 1.0, 1.0), (r.right_top(), -1.0, 1.0), (r.right_bottom(), -1.0, -1.0), (r.left_bottom(), 1.0, -1.0)] {
        pen.path(vec![corner + vec2(sx * l, 0.0), corner, corner + vec2(0.0, sy * l)], w, col);
    }
}

/// The other units in view: the enemy's in small red brackets (those firing at this unit
/// brighter), friends near by as green carets over them; what radar alone sees, dim amber.
pub(super) fn marks(pen: &Pen, view: &View, window: &Window, p: &Perspective, game: &Game, now: f64) {
    let me = game.sim.unit(p.unit).map_or(game.local_player, |u| u.owner);
    let alpha = game.alpha();
    let target = p.target(game);
    // (`TA_PERSPECTIVE_ALLIES=0` marks enemies only, for captures inside a crowd of one's own.)
    let allies = std::env::var("TA_PERSPECTIVE_ALLIES").map_or(true, |v| v != "0");
    let blink = p.reduced_flashes || (now * 3.0).fract() < 0.6;
    let mut scratch = Vec::new();
    for u in game.sim.units() {
        if u.id == p.unit || u.dead || u.carried_by.is_some() || Some(u.id) == target {
            continue;
        }
        let seen = game.seen(u);
        if seen == Seen::Hidden {
            continue;
        }
        let ty = game.sim.unit_type(u);
        let pos = game.scene.unit_position(u.id, alpha).unwrap_or(Vec3::from_array(u.pos.to_f32()));
        let dist = pos.distance(p.eye);
        if dist > 1800.0 {
            continue;
        }
        let enemy = game.sim.is_enemy(me, u.owner);
        if !enemy && (!allies || dist > 700.0 || ty.locomotion == ta_sim::Locomotion::Static) {
            continue;
        }
        // (Far out of the window, not worth fitting a box to.)
        if view.project(pos).is_none_or(|q| !window.rect.expand(window.rect.height() * 0.3).contains(q)) {
            continue;
        }
        // (Fitted to the model near by; far off, where it is small, its radius and height do: a
        // battle puts hundreds in view.)
        let b = if dist < NEAR { unit_box(view, game, u, pos, &mut scratch) } else { bounds(view, pos, ty.radius.to_f32().max(6.0) * 0.8, ty.model.height.to_f32().max(8.0)) };
        let Some(b) = b else { continue };
        if !window.rect.expand(20.0).contains(b.center()) {
            continue;
        }
        let a = (1.0 - dist / 1900.0).clamp(0.35, 0.85);
        if enemy {
            let on_us = firing_at(u, p);
            let col = if seen == Seen::Radar { AMBER_DIM } else { RED };
            let b = b.expand(pen.s(3.0)).union(Rect::from_center_size(b.center(), Vec2::splat(pen.s(14.0))));
            corners(pen, b, pen.s(7.0), if on_us { 1.8 } else { 1.3 }, col.gamma_multiply(if on_us && blink { 1.0 } else { a }));
        } else {
            let top = pos2(b.center().x, b.top() - pen.s(5.0));
            pen.fill(vec![top, top + vec2(pen.s(6.0), -pen.s(9.0)), top + vec2(-pen.s(6.0), -pen.s(9.0))], GREEN.gamma_multiply(a));
        }
    }
}

/// Whether a unit's weapons are on this unit.
fn firing_at(u: &ta_sim::Unit, p: &Perspective) -> bool {
    u.weapons.iter().any(|w| w.target == Some(Target::Unit(p.unit)))
}

/// The target, bracketed where it stands: corners round it (closing in from wider as it is taken,
/// amber), its hull along the bracket's foot, and one line by it, its name and its range: under
/// it, else over it, beside it, wherever keeps it off the sight's own marks (`avoid`) and inside
/// the window. Out of the window, a chevron on the threats' ring points the way to it, with the
/// same line. Returns where the line was written.
#[allow(clippy::too_many_arguments)]
pub(super) fn bracket(pen: &Pen, view: &View, window: &Window, p: &Perspective, game: &Game, now: f64, avoid: &[Rect]) -> Option<Rect> {
    let id = p.target(game)?;
    let u = game.sim.unit(id)?;
    let ty = game.sim.unit_type(u);
    let at = game.scene.unit_position(id, game.alpha())?;
    let range = at.distance(p.eye);
    let enemy = game.sim.is_enemy(game.sim.unit(p.unit).map_or(game.local_player, |me| me.owner), u.owner);
    let name_col = if enemy { RED } else { GREEN };
    let frac = (u.health / ty.max_health.max(Fx::ONE)).to_f32().clamp(0.0, 1.0);
    let since = p.hud.target.filter(|t| t.0 == id).map_or(now, |t| t.1);
    let taken = crate::perspective::ease(((now - since) as f32 / 0.3).clamp(0.0, 1.0));
    // NAME · 0000: the name in the side's colour, the range in amber.
    let size = 16.0;
    let name = pen.fitted(&ty.def.display_name.to_uppercase(), size, pen.s(220.0));
    let figure = format!(" · {:04}", range.round().min(9999.0) as i32);
    let (nw, fw) = (pen.width(&name, size), pen.width(&figure, size));
    let line_size = vec2(nw + fw, pen.s(size));
    let line = |r: Rect| -> Rect {
        pen.text(r.left_center(), Align2::LEFT_CENTER, name.clone(), size, name_col);
        pen.text(pos2(r.left() + nw, r.center().y), Align2::LEFT_CENTER, figure.clone(), size, AMBER_BRIGHT);
        r
    };
    let inside = unit_box(view, game, u, at, &mut Vec::new()).filter(|b| window.rect.contains(b.center()));
    match inside {
        Some(b) => {
            // At least so big; while being taken, wider.
            let b = b.expand(pen.s(5.0)).union(Rect::from_center_size(b.center(), Vec2::splat(pen.s(26.0))));
            let b = Rect::from_center_size(b.center(), b.size() * (1.0 + (1.0 - taken) * 1.4));
            let col = AMBER_BRIGHT.gamma_multiply(0.55 + 0.45 * taken);
            corners(pen, b, pen.s(12.0), 2.2, col);
            // The hull along its foot.
            let bar = Rect::from_min_size(pos2(b.left(), b.bottom() + pen.s(4.0)), vec2(b.width(), pen.s(3.0)));
            pen.painter.rect_filled(bar.expand(pen.s(1.0)), 0.0, pen.c(Color32::from_black_alpha(110)));
            pen.painter.rect_filled(bar, 0.0, pen.c(PHOSPHOR_DIM.gamma_multiply(0.6)));
            pen.painter.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac, bar.height())), 0.0, pen.c(health_color(frac)));
            // The line: the first of under, over, right and left that is clear (else under).
            let gap = pen.s(6.0);
            let places = [
                Align2::CENTER_TOP.anchor_size(pos2(b.center().x, bar.bottom() + gap), line_size),
                Align2::CENTER_BOTTOM.anchor_size(pos2(b.center().x, b.top() - gap), line_size),
                Align2::LEFT_CENTER.anchor_size(pos2(b.right() + gap, b.center().y), line_size),
                Align2::RIGHT_CENTER.anchor_size(pos2(b.left() - gap, b.center().y), line_size),
            ];
            let clear = |r: &Rect| window.rect.contains_rect(*r) && !avoid.iter().any(|a| a.intersects(*r));
            Some(line(places.iter().copied().find(clear).unwrap_or(places[0])))
        }
        None => {
            // The way to it, on the threats' ring.
            let (c, radii) = window.ring(pen);
            let dir = view.toward(at + Vec3::Y * ty.model.height.to_f32() * 0.5);
            let q = on_ring(c, radii, dir);
            let mark = chevron(pen, q, dir, 13.0, AMBER_BRIGHT, 1.0);
            // Its line inside the chevron, else beside it, clear of the sight's own marks.
            let inward = (c - q).normalized();
            let anchor = q + inward * pen.s(26.0);
            let align = Align2([if inward.x > 0.3 { egui::Align::Min } else if inward.x < -0.3 { egui::Align::Max } else { egui::Align::Center }, if inward.y > 0.3 { egui::Align::Min } else if inward.y < -0.3 { egui::Align::Max } else { egui::Align::Center }]);
            let gap = pen.s(20.0);
            let mut places = vec![
                align.anchor_size(anchor, line_size),
                Align2::RIGHT_CENTER.anchor_size(q - vec2(gap, 0.0), line_size),
                Align2::LEFT_CENTER.anchor_size(q + vec2(gap, 0.0), line_size),
                Align2::CENTER_BOTTOM.anchor_size(q - vec2(0.0, gap), line_size),
            ];
            // (Else just beside the mark in the way, at the chevron's height: an aircraft's bank
            // scale stands where things under the nose point.)
            let first = places[0];
            for a in avoid.iter().filter(|a| a.intersects(first)) {
                places.push(Align2::LEFT_CENTER.anchor_size(pos2(a.right() + pen.s(8.0), q.y), line_size));
                places.push(Align2::RIGHT_CENTER.anchor_size(pos2(a.left() - pen.s(8.0), q.y), line_size));
            }
            let clear = |r: &Rect| window.rect.contains_rect(*r) && !avoid.iter().any(|a| a.intersects(*r));
            Some(line(places.iter().copied().find(clear).unwrap_or(places[0])).union(mark))
        }
    }
}

/// The last target, destroyed: its bracket closing up on where it was, DESTROYED over it, for a
/// little over half a second.
pub(super) fn destroyed(pen: &Pen, view: &View, mark: &Marked, since: f64, now: f64) {
    let age = (now - since) as f32;
    if !(0.0..0.75).contains(&age) {
        return;
    }
    let Some(b) = projected_box(view, &mark.corners) else { return };
    let b = b.expand(pen.s(5.0)).union(Rect::from_center_size(b.center(), Vec2::splat(pen.s(26.0))));
    let close = crate::perspective::ease((age / 0.35).clamp(0.0, 1.0));
    let fade = 1.0 - ((age - 0.5) / 0.25).clamp(0.0, 1.0);
    let r = Rect::from_center_size(b.center(), b.size() * (1.0 - close * 0.8));
    corners(pen, r, pen.s(12.0), 2.2, AMBER_BRIGHT.gamma_multiply(fade));
    pen.text(pos2(b.center().x, b.center().y - pen.s(2.0)), Align2::CENTER_CENTER, t("DESTROYED"), 16.0, RED.gamma_multiply(fade * close.max(0.4)));
}

// ---- Threats out of sight -------------------------------------------------------------------

/// Where a ray from `c` the way `dir` (screen points, y down) meets the ellipse of `radii` round it.
fn on_ring(c: Pos2, radii: Vec2, dir: Vec2) -> Pos2 {
    let d = if dir.length() < 1e-4 { vec2(0.0, 1.0) } else { dir.normalized() };
    let k = 1.0 / ((d.x / radii.x).powi(2) + (d.y / radii.y).powi(2)).sqrt();
    c + d * k
}

/// A chevron at `at` pointing the way `dir`, `size` units long; the room it takes.
fn chevron(pen: &Pen, at: Pos2, dir: Vec2, size: f32, col: Color32, weight: f32) -> Rect {
    let d = if dir.length() < 1e-4 { vec2(0.0, 1.0) } else { dir.normalized() };
    let side = vec2(-d.y, d.x);
    let s = pen.s(size);
    let tip = at + d * s * 0.5;
    let points = vec![tip - d * s + side * s * 0.75, tip, tip - d * s - side * s * 0.75];
    let room = Rect::from_points(&points).expand(pen.s(2.0 * weight));
    pen.path(points, 2.4 * weight, col);
    room
}

/// The way round the window's edge (clock degrees) of a screen direction.
fn clock_of(dir: Vec2) -> f32 {
    dir.x.atan2(-dir.y).to_degrees().rem_euclid(360.0)
}

/// How many ways round the threats are told apart (a chevron each at most).
const BEARINGS: usize = 24;

/// Enemies near by out of the window, and those firing at the unit from out of it, as chevrons
/// on an ellipse inside the window's edge pointing the way to them (one a bearing, larger the
/// nearer, pulsing while they fire at the unit); a hit flashes the chevron the way it came from,
/// and the window's edge on that side. Returns the room the chevrons take.
pub(super) fn threats(pen: &Pen, view: &View, window: &Window, p: &Perspective, game: &Game, now: f64) -> Vec<Rect> {
    let mut room = Vec::new();
    let Some(me) = game.sim.unit(p.unit) else { return room };
    let (c, radii) = window.ring(pen);
    // Per bearing: the nearest's distance, and whether any of them is firing at the unit.
    let mut ways: [(f32, bool, Vec2); BEARINGS] = [(f32::MAX, false, Vec2::ZERO); BEARINGS];
    for u in game.sim.units() {
        if u.dead || u.carried_by.is_some() || !game.sim.is_enemy(me.owner, u.owner) || game.seen(u) == Seen::Hidden {
            continue;
        }
        let pos = game.scene.unit_position(u.id, game.alpha()).unwrap_or(Vec3::from_array(u.pos.to_f32()));
        let dist = pos.distance(p.eye);
        let on_us = firing_at(u, p);
        if dist > 1400.0 && !on_us {
            continue;
        }
        // (In the window, its bracket marks it.)
        if view.project(pos).is_some_and(|q| window.rect.contains(q)) {
            continue;
        }
        let dir = view.toward(pos);
        let k = ((clock_of(dir) / 360.0 * BEARINGS as f32).round() as usize) % BEARINGS;
        let w = &mut ways[k];
        if dist < w.0 {
            w.0 = dist;
            w.2 = dir;
        }
        w.1 |= on_us;
    }
    let pulse = if p.reduced_flashes { 1.0 } else { 0.65 + 0.35 * ((now * 6.0) as f32).sin() };
    for (dist, on_us, dir) in ways {
        if dist == f32::MAX {
            continue;
        }
        let near = (1.0 - dist / 1400.0).clamp(0.0, 1.0);
        let size = 9.0 + 6.0 * near;
        let a = if on_us { pulse } else { 0.45 + 0.4 * near };
        room.push(chevron(pen, on_ring(c, radii, dir), dir, size, RED.gamma_multiply(a), if on_us { 1.2 } else { 1.0 }));
    }
    // Hits from out of the window: the chevron the way each came from, bright, and the window's
    // edge glowing red on that side. (An attacker in the window is bracketed, blinking.)
    for &(at, from, blow) in &p.hud.hits {
        let age = (now - at) as f32;
        let Some(from) = from else { continue };
        if !(0.0..1.4).contains(&age) || view.project(from).is_some_and(|q| window.rect.contains(q)) {
            continue;
        }
        let dir = view.toward(from);
        let fade = (1.0 - age / 1.4) * blow.max(0.5);
        room.push(chevron(pen, on_ring(c, radii, dir), dir, 20.0, Color32::from_rgb(255, 96, 70).gamma_multiply(fade), 1.5));
        if !p.reduced_flashes {
            edge_glow(pen, window, dir, fade * (1.0 - (age / 0.5).clamp(0.0, 1.0)) * blow);
        }
    }
    room
}

/// How long a hit flashes the window's edge all round (seconds).
const HIT_FLASH: f32 = 0.3;

/// How strongly a hit flashes the window's edge all round, `age` seconds after it, by how hard
/// it was (0..1): at once, then gone over [`HIT_FLASH`].
fn flash_strength(age: f32, blow: f32) -> f32 {
    if !(0.0..HIT_FLASH).contains(&age) {
        return 0.0;
    }
    (1.0 - age / HIT_FLASH) * (0.35 + 0.65 * blow.clamp(0.0, 1.0))
}

/// Every hit, wherever it came from: the window's edge flashes red all round for a moment (the way
/// a hit from out of the window came, `threats` shows). Not with flashes reduced.
pub(super) fn hit_flash(pen: &Pen, window: &Window, p: &Perspective, now: f64) {
    if p.reduced_flashes {
        return;
    }
    let k = p.hud.hits.iter().map(|&(at, _, blow)| flash_strength((now - at) as f32, blow)).fold(0.0f32, f32::max);
    glow(pen, window, None, 0.86, 0.13, k * 0.8);
}

/// The window's edge glowing red on one side (the way `dir` points), as strong as `k`.
fn edge_glow(pen: &Pen, window: &Window, dir: Vec2, k: f32) {
    glow(pen, window, Some(dir), 0.8, 0.22, k);
}

/// How far round from the way a hit came its glow reaches (radians, seen from the window's
/// middle).
const GLOW_REACH: f32 = 1.0;

/// How strongly a part of the edge `v` away from the window's middle glows for a hit from the way
/// `dir` (both on the screen): fully where they agree, nothing [`GLOW_REACH`] round from it.
fn glow_weight(v: Vec2, dir: Vec2) -> f32 {
    if v.length() < 1e-4 || dir.length() < 1e-4 {
        return 0.0;
    }
    let cos = v.normalized().dot(dir.normalized()).clamp(-1.0, 1.0);
    (1.0 - cos.acos() / GLOW_REACH).max(0.0)
}

/// A band round the inside of `r`'s edge, `depth` points deep (mitred at the corners), red at the
/// edge and fading inwards, as strong as `k` times what `weight` gives each part of the edge (by
/// the way it lies from the middle).
fn edge_band(pen: &Pen, r: Rect, depth: f32, k: f32, weight: impl Fn(Vec2) -> f32) {
    let c = r.center();
    let inner = r.shrink(depth.min(r.width() * 0.45).min(r.height() * 0.45));
    let outer = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()];
    let inside = [inner.left_top(), inner.right_top(), inner.right_bottom(), inner.left_bottom()];
    const STEPS: u32 = 16;
    let mut mesh = Mesh::default();
    for e in 0..4 {
        let base = mesh.vertices.len() as u32;
        for i in 0..=STEPS {
            let f = i as f32 / STEPS as f32;
            let o = outer[e].lerp(outer[(e + 1) % 4], f);
            let w = weight(o - c).clamp(0.0, 1.0);
            mesh.colored_vertex(o, Color32::from_rgba_unmultiplied(200, 24, 12, (150.0 * k * w * pen.on) as u8));
            mesh.colored_vertex(inside[e].lerp(inside[(e + 1) % 4], f), Color32::TRANSPARENT);
        }
        for i in 0..STEPS {
            let j = base + i * 2;
            mesh.add_triangle(j, j + 1, j + 2);
            mesh.add_triangle(j + 1, j + 3, j + 2);
        }
    }
    pen.painter.add(Shape::mesh(mesh));
}

/// The edges glowing red, as strong as `k`: round the way `dir` points (strongest there, gone
/// [`GLOW_REACH`] round each side), or, with no way, all round alike. A window's: from the
/// picture's edges, the cabin red as with a lamp, fading into the glass (`depth`, a share of the
/// picture's height), down to the window's foot (the console's screens stay as they are). A round
/// field's: round its rim, in to `inner` of the way to its middle.
fn glow(pen: &Pen, window: &Window, dir: Option<Vec2>, inner: f32, depth: f32, k: f32) {
    if k <= 0.01 {
        return;
    }
    let r = window.rect;
    if !window.round {
        let picture = pen.painter.ctx().content_rect();
        let band = Rect::from_min_max(picture.min, pos2(picture.right(), r.bottom().min(picture.bottom())));
        edge_band(pen, band, picture.height() * depth, k, |v| dir.map_or(1.0, |d| glow_weight(v, d)));
        return;
    }
    let (c, radii) = (r.center(), vec2(r.width() * 0.5, r.height() * 0.5));
    let (middle, reach, steps) = match dir {
        Some(d) => (d.x.atan2(-d.y), 0.9f32, 24),
        None => (0.0, std::f32::consts::PI, 72),
    };
    let mut mesh = Mesh::default();
    for i in 0..=steps {
        let a = middle - reach + 2.0 * reach * i as f32 / steps as f32;
        let w = if dir.is_some() { 1.0 - ((a - middle) / reach).abs() } else { 1.0 };
        let d = clock(a);
        let outer = on_ring(c, radii, d);
        let inside = c + (outer - c) * inner;
        mesh.colored_vertex(outer, Color32::from_rgba_unmultiplied(200, 24, 12, (150.0 * k * w * pen.on) as u8));
        mesh.colored_vertex(inside, Color32::TRANSPARENT);
    }
    for i in 0..steps as u32 {
        let j = i * 2;
        mesh.add_triangle(j, j + 1, j + 2);
        mesh.add_triangle(j + 1, j + 3, j + 2);
    }
    pen.painter.add(Shape::mesh(mesh));
}

fn lerp(a: Color32, b: Color32, k: f32) -> Color32 {
    let k = k.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k) as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

// ---- Messages -----------------------------------------------------------------------------------

/// The newest message, one line low in the window (over the sight's own marks there, `avoid`;
/// while the view's keys are shown there, `keys`, high in the window instead), typed out and
/// fading: no plate.
pub(super) fn message(pen: &Pen, window: &Window, p: &Perspective, now: f64, avoid: &[Rect], keys: Option<Rect>) {
    let Some((text, since, urgent)) = p.hud.messages.iter().rev().find(|m| now >= m.1) else { return };
    let age = (now - since) as f32;
    let a = 1.0 - ((age - 2.2) / 0.8).clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let shown: String = text.chars().take((age * 40.0) as usize + 1).collect();
    let low = pos2(window.rect.center().x, window.rect.bottom() - pen.s(if window.round { 120.0 } else { 34.0 }));
    // (Its whole line's room, typed out or not.)
    let size = vec2(pen.painter.layout_no_wrap(text.clone(), crate::theme::strong(pen.s(17.0)), Color32::WHITE).size().x, pen.s(20.0));
    // Low in the middle; else, still low, beside the mark in the way; else over it. While the
    // view's keys are shown low, high in the window.
    let mut places = vec![Rect::from_center_size(low, size)];
    for a in avoid.iter().filter(|a| a.intersects(places[0])).copied().collect::<Vec<_>>() {
        places.push(Align2::RIGHT_CENTER.anchor_size(pos2(a.left() - pen.s(12.0), low.y), size));
        places.push(Align2::LEFT_CENTER.anchor_size(pos2(a.right() + pen.s(12.0), low.y), size));
        places.push(Align2::CENTER_BOTTOM.anchor_size(pos2(low.x, a.top()), size));
    }
    let clear = |r: &Rect| window.rect.contains_rect(*r) && !avoid.iter().any(|a| a.intersects(*r)) && !keys.is_some_and(|k| k.expand(pen.s(6.0)).intersects(*r));
    let place = places.iter().copied().find(clear).unwrap_or_else(|| Rect::from_center_size(pos2(low.x, window.rect.top() + pen.s(if window.round { 120.0 } else { 46.0 })), size));
    pen.name(place.center(), Align2::CENTER_CENTER, shown, 17.0, (if *urgent == 1 { RED } else { AMBER_BRIGHT }).gamma_multiply(a));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_weapon_has_its_arc_and_a_lone_one_both_lower_arcs() {
        assert!(slots(0).is_empty());
        assert_eq!(slots(1).len(), 1);
        assert_eq!(slots(1)[0].len(), 2);
        assert_eq!(slots(2).iter().map(Vec::len).collect::<Vec<_>>(), [1, 1]);
        assert_eq!(slots(3).len(), 3);
        // (Never more than three: a unit has three weapons.)
        assert_eq!(slots(5).len(), 3);
        // The arcs keep off the reticle's lines, which run to the four points of the compass.
        for count in 1..=3 {
            for slot in slots(count).iter().flatten() {
                let (a, b) = (slot.middle - HALF_ARC, slot.middle + HALF_ARC);
                for line in [0.0f32, 90.0, 180.0, 270.0, 360.0] {
                    assert!(!(a + 4.0..b - 4.0).contains(&line) || slot.middle == 0.0 && line.rem_euclid(360.0) == 0.0, "{slot:?} crosses {line}");
                }
            }
        }
        // Both lower arcs fill from the foot.
        let one = &slots(1)[0];
        assert!((start(one[0]).to_degrees() - 198.0).abs() < 0.01 && (start(one[1]).to_degrees() - 162.0).abs() < 0.01);
    }

    #[test]
    fn a_hit_glows_on_the_edge_it_came_from() {
        let left = vec2(-1.0, 0.0);
        // The left edge's middle, fully; its top corner of a wide window, much of it; the right
        // edge, none.
        assert!((glow_weight(vec2(-600.0, 0.0), left) - 1.0).abs() < 1e-5);
        assert!(glow_weight(vec2(-600.0, -300.0), left) > 0.4);
        assert_eq!(glow_weight(vec2(600.0, 0.0), left), 0.0);
        // Straight up from the middle is a quarter turn round: nothing.
        assert_eq!(glow_weight(vec2(0.0, -300.0), left), 0.0);
        assert_eq!(glow_weight(Vec2::ZERO, left), 0.0);
    }

    #[test]
    fn a_hit_flashes_at_once_and_harder_the_harder_it_was() {
        assert!(flash_strength(0.0, 1.0) > 0.99);
        assert!(flash_strength(0.0, 0.2) < flash_strength(0.0, 0.8));
        // (A graze still shows.)
        assert!(flash_strength(0.0, 0.0) > 0.3);
        assert!(flash_strength(0.1, 1.0) < flash_strength(0.05, 1.0));
        // Over in a moment; nothing before the hit.
        assert_eq!(flash_strength(HIT_FLASH, 1.0), 0.0);
        assert_eq!(flash_strength(2.0, 1.0), 0.0);
        assert_eq!(flash_strength(-0.1, 1.0), 0.0);
    }

    #[test]
    fn a_chevron_stands_on_the_ring_the_way_it_points() {
        let c = pos2(500.0, 300.0);
        let radii = vec2(400.0, 200.0);
        assert!((on_ring(c, radii, vec2(1.0, 0.0)) - pos2(900.0, 300.0)).length() < 1e-3);
        assert!((on_ring(c, radii, vec2(0.0, -3.0)) - pos2(500.0, 100.0)).length() < 1e-3);
        let q = on_ring(c, radii, vec2(1.0, 1.0));
        assert!((((q.x - c.x) / radii.x).powi(2) + ((q.y - c.y) / radii.y).powi(2) - 1.0).abs() < 1e-4);
        // (Straight behind, with no way at all, it is at the foot.)
        assert!((on_ring(c, radii, Vec2::ZERO) - pos2(500.0, 500.0)).length() < 1e-3);
        assert_eq!(clock_of(vec2(0.0, -1.0)), 0.0);
        assert_eq!(clock_of(vec2(1.0, 0.0)), 90.0);
        assert_eq!(clock_of(vec2(-1.0, 0.0)), 270.0);
    }
}
