//! The galaxy map: deep space; each charted system's territory (the ground nearer it than any
//! other, in its owner's colour, the front between ours and theirs drawn in brass); the lanes
//! (ours in our colour, those we can attack along in brass, flowing towards the enemy, the rest
//! dashed); the route to the enemy capital when it is wanted; an enemy counter-attack, a red
//! arrow along its lane; the planets (the worlds of their maps, ringed in their owner's colour,
//! those within reach pulsing, one under attack pulsing red, the capital crowned, our home
//! marked, defeats counted, each one's reward badged; uncharted ones dark, a question mark on
//! them); and the systems' names, each beside its own planet clear of the rest. The systems are
//! buttons: clicked or focused with Tab, moved between with the arrow keys, Enter (or a double
//! click) on one within reach, or one of ours under attack, briefs its battle.

use std::f32::consts::TAU;

use egui::{Color32, Id, Key, Mesh, Modifiers, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2};
use ta_ui::i18n::{t, tf};
use ta_ui::widgets as w;

use super::ConquestPage;
use crate::conquest::{Conquest, Owner, Reward};
use crate::palette::team_color;
use crate::theme::*;

/// How far round a system's planet its selection reticle reaches, and how far out its name
/// stays clear of that (both at a galaxy's normal size).
pub(super) const SELECT_RING: f32 = 10.0;
const LABEL_CLEAR: f32 = SELECT_RING + 3.0;

/// How the galaxy is shown.
#[derive(Clone, Copy)]
pub(super) struct View {
    /// Where the systems' places (0 to 1 across and down) fall.
    pub area: Rect,
    /// What the map may paint in (names reach past `area`).
    pub bounds: Rect,
    /// How large planets, lanes and names are (1 for a galaxy about 1,000 points across).
    pub k: f32,
    pub time: f32,
}

impl View {
    pub fn at(&self, c: &Conquest, i: usize) -> Pos2 {
        let p = c.systems[i].pos;
        pos2(self.area.min.x + p[0] * self.area.width(), self.area.min.y + p[1] * self.area.height())
    }

    pub fn radius(&self, c: &Conquest, i: usize) -> f32 {
        (if i == c.capital { 15.0 } else { 11.0 }) * self.k
    }
}

/// The scale for a galaxy laid over `area`: 1 up to about 1,000 x 600 points, larger beyond, so
/// a big screen shows bigger worlds and names rather than the same small ones further apart.
pub(super) fn scale_for(area: Rect) -> f32 {
    (area.width() / 1000.0).min(area.height() / 580.0).clamp(1.0, 1.7)
}

/// What the player did on the map this frame.
#[derive(Default)]
pub(super) struct MapInput {
    pub hovered: Option<usize>,
    /// A system chosen, or the empty space (`Some(None)`: back to the empire).
    pub select: Option<Option<usize>>,
    /// A system within reach to brief (a double click, or Enter on it).
    pub brief: Option<usize>,
}

/// A world's colour on the galaxy map, by the planet its map names (its stand-in while its
/// world is made, and its glow).
pub(crate) fn planet_color(planet: &str) -> Color32 {
    let p = planet.to_ascii_lowercase();
    let table: [(&str, [u8; 3]); 17] = [
        ("earth", [70, 128, 168]),
        ("asteroid", [128, 116, 104]),
        ("wet desert", [176, 150, 104]),
        ("desert", [214, 178, 112]),
        ("green", [92, 152, 72]),
        ("lush", [62, 140, 86]),
        ("red", [176, 82, 52]),
        ("lava", [228, 96, 34]),
        ("metal", [146, 156, 166]),
        ("ice", [204, 228, 246]),
        ("archipelago", [64, 146, 196]),
        ("water", [44, 104, 196]),
        ("slate", [98, 104, 118]),
        ("lunar", [178, 178, 178]),
        ("crystal", [150, 214, 238]),
        ("acid", [172, 204, 64]),
        ("urban", [132, 142, 162]),
    ];
    // (A map that names no planet is a green world, as the terrain takes it.)
    let p = if p.trim().is_empty() { "green".to_string() } else { p };
    table.iter().find(|(k, _)| p.contains(k)).map_or(Color32::from_rgb(150, 140, 170), |(_, [r, g, b])| Color32::from_rgb(*r, *g, *b))
}

/// A planet `r` across round `p` in `base`, lit as a ball from the upper left (in place of its
/// world, `planet::Planets`, until that is drawn).
pub(crate) fn paint_planet(painter: &egui::Painter, p: Pos2, r: f32, base: Color32) {
    const RINGS: [f32; 7] = [0.0, 0.35, 0.6, 0.78, 0.9, 0.97, 1.0];
    const SEGMENTS: u32 = 32;
    let light = {
        let l = [-0.45f32, -0.55, 0.7];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / n, l[1] / n, l[2] / n]
    };
    let [br, bg, bb, _] = base.to_array();
    let air = base.lerp_to_gamma(Color32::from_rgb(190, 215, 255), 0.6);
    let shade = |x: f32, y: f32| {
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        let d = (x * light[0] + y * light[1] + z * light[2]).max(0.0);
        let lit = 0.22 + 0.9 * d;
        let glint = d.powi(24) * 0.35;
        let rim = (1.0 - z).powf(2.6) * 0.55 * (d * 2.0).min(1.0);
        let mix = |c: u8, a: u8| ((c as f32 * lit + 255.0 * glint) * (1.0 - rim) + a as f32 * rim).clamp(0.0, 255.0) as u8;
        Color32::from_rgb(mix(br, air.r()), mix(bg, air.g()), mix(bb, air.b()))
    };
    let radius = r + 0.5;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(p, shade(0.0, 0.0));
    for &s in &RINGS[1..] {
        for k in 0..SEGMENTS {
            let a = k as f32 / SEGMENTS as f32 * TAU;
            let (x, y) = (a.cos() * s, a.sin() * s);
            mesh.colored_vertex(p + vec2(x, y) * radius, shade(x, y));
        }
    }
    let at = |ring: u32, k: u32| 1 + ring * SEGMENTS + k % SEGMENTS;
    for k in 0..SEGMENTS {
        mesh.add_triangle(0, at(0, k), at(0, k + 1));
    }
    for ring in 0..RINGS.len() as u32 - 2 {
        for k in 0..SEGMENTS {
            mesh.add_triangle(at(ring, k), at(ring + 1, k), at(ring + 1, k + 1));
            mesh.add_triangle(at(ring, k), at(ring + 1, k + 1), at(ring, k + 1));
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// The enemy capital's crown over a planet `r` across round `p` (`scale` 1 on a normal galaxy):
/// three tines with balls on a band, in brass, edged dark.
pub(super) fn paint_crown(painter: &egui::Painter, p: Pos2, r: f32, scale: f32) {
    let foot = p.y - r - 4.0 * scale;
    let s = |x: f32, y: f32| pos2(p.x + x * scale, foot + y * scale);
    let tines = [(-7.0, -9.0), (0.0, -12.0), (7.0, -9.0)];
    for (edge, color) in [(1.2 * scale, Color32::from_black_alpha(190)), (0.0, AMBER)] {
        let grow = |q: Pos2, toward: Pos2| q + (q - toward).normalized() * edge;
        let centre = s(0.0, -3.0);
        for (x, y) in tines {
            painter.add(Shape::convex_polygon(vec![grow(s(x - 3.2, -3.0), centre), grow(s(x, y), centre), grow(s(x + 3.2, -3.0), centre)], color, Stroke::NONE));
            painter.circle_filled(s(x, y - 0.6), 1.5 * scale + edge, if edge > 0.0 { color } else { AMBER_BRIGHT });
        }
        let band = Rect::from_min_max(s(-9.0, -4.0), s(9.0, 0.0)).expand(edge);
        if edge > 0.0 {
            painter.rect_filled(band, 1.0, color);
        } else {
            painter.add(ta_ui::metal::vgrad(band, AMBER_BRIGHT, AMBER_DIM));
        }
    }
}

/// Our home's mark over its planet: a small diamond in our colour with a lit edge.
fn paint_home(painter: &egui::Painter, p: Pos2, r: f32, k: f32) {
    let c = p + vec2(0.0, -r - 9.0 * k);
    let d = 4.5 * k;
    let points = vec![c + vec2(0.0, -d), c + vec2(d, 0.0), c + vec2(0.0, d), c + vec2(-d, 0.0)];
    painter.add(Shape::convex_polygon(points, team_color(0), Stroke::new(1.2, Color32::from_rgb(200, 220, 255))));
}

/// The reticle round the chosen system: four brackets turning slowly.
fn paint_reticle(painter: &egui::Painter, p: Pos2, r: f32, k: f32, time: f32) {
    let reach = r + SELECT_RING * k;
    let turn = time * 0.5;
    for q in 0..4 {
        let a = turn + q as f32 * TAU / 4.0;
        let points: Vec<Pos2> = (0..=6).map(|s| {
            let b = a - 0.32 + 0.64 * s as f32 / 6.0;
            p + vec2(b.cos(), b.sin()) * reach
        }).collect();
        painter.add(Shape::line(points, Stroke::new(2.2 * k.sqrt(), TEXT_BRIGHT)));
    }
}

/// "◆◆◇": filled pips for an enemy's strength, `size` points high.
pub(super) fn paint_threat(painter: &egui::Painter, left_mid: Pos2, threat: u8, size: f32) {
    let half = size * 0.45;
    for k in 0..3 {
        let c = pos2(left_mid.x + half + k as f32 * size * 1.25, left_mid.y);
        let on = k < threat as usize;
        let points = vec![c + vec2(0.0, -half), c + vec2(half, 0.0), c + vec2(0.0, half), c + vec2(-half, 0.0)];
        painter.add(Shape::convex_polygon(points, if on { RED } else { INSET }, Stroke::new(1.0, if on { RED } else { LINE_HI })));
    }
}

/// A reward's colour: metal's steel, energy's yellow, the units' blue, the college's brass,
/// the sensors' cyan, the depot's green.
pub(super) fn reward_color(reward: Reward) -> Color32 {
    match reward {
        Reward::None => STEEL,
        Reward::Mines => Color32::from_rgb(196, 204, 214),
        Reward::Reactors => Color32::from_rgb(246, 206, 72),
        Reward::Foundry | Reward::Airfield => Color32::from_rgb(140, 186, 242),
        Reward::Academy => AMBER_BRIGHT,
        Reward::Sensors => Color32::from_rgb(110, 214, 232),
        Reward::Depot => Color32::from_rgb(128, 206, 124),
    }
}

/// A reward's glyph round `c`, `size` points across: a hexagon of metal, a bolt of energy, a
/// tank, an aircraft, a college's chevrons, a sensor's waves, a depot's crate.
pub(super) fn paint_reward(painter: &egui::Painter, c: Pos2, size: f32, reward: Reward) {
    let color = reward_color(reward);
    let u = size / 12.0;
    let at = |x: f32, y: f32| c + vec2(x * u, y * u);
    let line = Stroke::new(1.5 * u, color);
    match reward {
        Reward::None => {}
        Reward::Mines => {
            let hex: Vec<Pos2> = (0..6).map(|k| { let a = TAU * (k as f32 + 0.5) / 6.0; at(5.5 * a.cos(), 5.5 * a.sin()) }).collect();
            painter.add(Shape::convex_polygon(hex.clone(), color.gamma_multiply(0.35), line));
            painter.add(Shape::convex_polygon((0..6).map(|k| { let a = TAU * (k as f32 + 0.5) / 6.0; at(2.4 * a.cos(), 2.4 * a.sin()) }).collect(), color, Stroke::NONE));
        }
        Reward::Reactors => {
            painter.add(Shape::convex_polygon(vec![at(1.5, -6.0), at(-3.5, 1.0), at(0.5, 1.0)], color, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![at(-0.5, -1.0), at(3.5, -1.0), at(-1.5, 6.0)], color, Stroke::NONE));
        }
        Reward::Foundry => {
            painter.rect_filled(Rect::from_min_max(at(-5.5, 0.5), at(5.5, 4.5)), 1.5 * u, color);
            painter.rect_filled(Rect::from_min_max(at(-2.5, -2.5), at(2.0, 0.5)), 1.0 * u, color);
            painter.line_segment([at(2.0, -1.2), at(6.0, -2.8)], line);
        }
        Reward::Airfield => {
            painter.add(Shape::convex_polygon(vec![at(0.0, -6.0), at(1.4, -1.0), at(-1.4, -1.0)], color, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![at(0.0, -2.5), at(6.0, 2.0), at(0.0, 1.0), at(-6.0, 2.0)], color, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![at(-1.2, 0.5), at(1.2, 0.5), at(0.8, 5.0), at(-0.8, 5.0)], color, Stroke::NONE));
            painter.line_segment([at(-2.8, 5.5), at(2.8, 5.5)], line);
        }
        Reward::Academy => {
            for y in [-3.5, 0.5] {
                painter.add(Shape::line(vec![at(-5.0, y + 2.5), at(0.0, y - 1.5), at(5.0, y + 2.5)], Stroke::new(2.0 * u, color)));
            }
            painter.circle_filled(at(0.0, 5.0), 1.2 * u, color);
        }
        Reward::Sensors => {
            painter.circle_filled(at(-4.0, 4.0), 1.6 * u, color);
            for r in [4.5f32, 8.0] {
                let arc: Vec<Pos2> = (0..=8).map(|k| { let a = -TAU / 4.0 + TAU / 4.0 * k as f32 / 8.0; at(-4.0 + r * a.cos(), 4.0 + r * a.sin()) }).collect();
                painter.add(Shape::line(arc, line));
            }
        }
        Reward::Depot => {
            let crate_ = Rect::from_min_max(at(-5.0, -5.0), at(5.0, 5.0));
            painter.rect(crate_, 1.0 * u, color.gamma_multiply(0.3), line, egui::StrokeKind::Inside);
            painter.line_segment([at(0.0, -3.0), at(0.0, 3.0)], Stroke::new(2.0 * u, color));
            painter.line_segment([at(-3.0, 0.0), at(3.0, 0.0)], Stroke::new(2.0 * u, color));
        }
    }
}

/// A perk's glyph round `c`, `size` points across, in the Commander's brass: a gear, a shield, a
/// sight, a star, a capacitor's plates, a quartermaster's crates.
pub(crate) fn paint_perk(painter: &egui::Painter, c: Pos2, size: f32, perk: crate::conquest::Perk) {
    use crate::conquest::Perk;
    let color = AMBER_BRIGHT;
    let u = size / 12.0;
    let at = |x: f32, y: f32| c + vec2(x * u, y * u);
    let line = Stroke::new(1.6 * u, color);
    match perk {
        Perk::Engineer => {
            for k in 0..8 {
                let a = TAU * k as f32 / 8.0;
                painter.line_segment([at(3.5 * a.cos(), 3.5 * a.sin()), at(6.0 * a.cos(), 6.0 * a.sin())], Stroke::new(2.2 * u, color));
            }
            painter.circle_stroke(c, 4.0 * u, line);
            painter.circle_filled(c, 1.4 * u, color);
        }
        Perk::Armour => {
            painter.add(Shape::convex_polygon(vec![at(-5.0, -5.5), at(5.0, -5.5), at(5.0, 0.5), at(0.0, 6.0), at(-5.0, 0.5)], color.gamma_multiply(0.3), line));
            painter.line_segment([at(0.0, -3.5), at(0.0, 3.5)], line);
        }
        Perk::Marksman => {
            painter.circle_stroke(c, 4.5 * u, line);
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                painter.line_segment([at(2.0 * dx, 2.0 * dy), at(6.5 * dx, 6.5 * dy)], line);
            }
        }
        Perk::Instructor => {
            let point = |k: usize, r: f32| {
                let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
                at(r * a.cos(), r * a.sin())
            };
            painter.add(Shape::convex_polygon((0..5).map(|j| point(2 * j + 1, 2.6)).collect(), color, Stroke::NONE));
            for j in 0..5 {
                painter.add(Shape::convex_polygon(vec![point((2 * j + 9) % 10, 2.6), point(2 * j, 6.2), point(2 * j + 1, 2.6)], color, Stroke::NONE));
            }
        }
        Perk::Capacitors => {
            painter.line_segment([at(-1.6, -5.0), at(-1.6, 5.0)], Stroke::new(2.2 * u, color));
            painter.line_segment([at(1.6, -5.0), at(1.6, 5.0)], Stroke::new(2.2 * u, color));
            painter.line_segment([at(-6.0, 0.0), at(-1.6, 0.0)], line);
            painter.line_segment([at(1.6, 0.0), at(6.0, 0.0)], line);
        }
        Perk::Quartermaster => {
            for (x, y) in [(-5.5, 0.5), (0.5, 0.5), (-2.5, -5.0)] {
                painter.rect(Rect::from_min_size(at(x, y), vec2(5.0 * u, 5.0 * u)), 0.5 * u, color.gamma_multiply(0.3), Stroke::new(1.3 * u, color), egui::StrokeKind::Inside);
            }
        }
    }
}

/// A reward's badge on the map: its glyph in a dark disc rimmed in its colour.
fn paint_reward_badge(painter: &egui::Painter, c: Pos2, k: f32, reward: Reward) {
    let r = 7.5 * k.sqrt();
    painter.circle(c, r, Color32::from_rgba_unmultiplied(8, 10, 14, 225), Stroke::new(1.0, reward_color(reward).gamma_multiply(0.7)));
    paint_reward(painter, c, r * 1.25, reward);
}

/// An uncharted system: a dark world under a question mark.
fn paint_uncharted(painter: &egui::Painter, p: Pos2, r: f32, k: f32) {
    paint_planet(painter, p, r, Color32::from_rgb(52, 56, 66));
    painter.circle_stroke(p, r + 1.5, Stroke::new(1.4 * k.sqrt(), Color32::from_rgb(92, 98, 112)));
    painter.text(p + vec2(0.0, 0.5 * k), egui::Align2::CENTER_CENTER, "?", strong(r * 1.2), Color32::from_rgb(170, 178, 192));
}

/// The enemy's counter-attack: a red arrow from the system it comes from to ours, its dashes
/// running towards ours.
fn paint_counter(painter: &egui::Painter, from: (Pos2, f32), to: (Pos2, f32), k: f32, time: f32) {
    let dir = (to.0 - from.0).normalized();
    let (a, b) = (from.0 + dir * (from.1 + 6.0 * k), to.0 - dir * (to.1 + 10.0 * k));
    if (b - a).dot(dir) <= 0.0 {
        return;
    }
    painter.line_segment([a, b], Stroke::new(8.0 * k, RED.gamma_multiply(0.14)));
    let (dash, gap) = (8.0 * k, 5.0 * k);
    let offset = (dash + gap) - (time * 30.0 * k) % (dash + gap);
    painter.extend(Shape::dashed_line_with_offset(&[a, b], Stroke::new(2.4 * k.sqrt(), RED), &[dash], &[gap], offset));
    let side = vec2(-dir.y, dir.x);
    let head = b + dir * 6.0 * k;
    painter.add(Shape::convex_polygon(vec![head, b - side * 5.0 * k, b + side * 5.0 * k], RED, Stroke::NONE));
}

impl ConquestPage {
    /// Draws the galaxy as `view` places it; with `interactive`, its systems are buttons and
    /// what the player did with them comes back.
    pub(super) fn draw_galaxy(&mut self, ui: &mut Ui, c: &Conquest, view: View, interactive: bool) -> MapInput {
        let mut input = MapInput::default();
        let ctx = ui.ctx().clone();
        let painter = ui.painter_at(view.bounds);
        let k = view.k;
        let n = c.systems.len();
        let points: Vec<Pos2> = (0..n).map(|i| view.at(c, i)).collect();
        let ours = team_color(0);
        let theirs = team_color(1);
        // (What is known of each: an uncharted one keeps its name to itself, but the enemy
        // capital is the war's aim.)
        let charted = c.charted();
        let named = |i: usize| charted[i] || i == c.capital;

        // What the pointer and the keyboard are on (laid out before anything is painted, so
        // that what they are on can be drawn as such).
        let names: Vec<std::sync::Arc<egui::Galley>> = c.systems.iter().enumerate().map(|(i, s)| painter.layout_no_wrap(if named(i) { s.name.to_uppercase() } else { String::new() }, strong(11.0 * k.sqrt()), STEEL)).collect();
        let planets: Vec<(Pos2, f32, f32)> = (0..n).map(|i| (points[i], view.radius(c, i), if i == c.capital { 6.0 * k } else { 0.0 })).collect();
        let segments: Vec<(Pos2, Pos2)> = c.lanes.iter().map(|&(a, b)| (points[a], points[b])).collect();
        let labels = place_labels(&planets, &names.iter().map(|g| g.size()).collect::<Vec<_>>(), &segments, view.bounds, k);
        let mut focused = None;
        if interactive {
            // (Empty space first: the systems over it take their own clicks.)
            let space = ui.interact(view.bounds, ui.id().with("conquest_space"), Sense::click());
            if space.clicked() {
                input.select = Some(None);
            }
            for i in 0..n {
                let id = system_id(i);
                let planet = Rect::from_center_size(points[i], Vec2::splat(2.0 * planets[i].1 + 12.0 * k));
                let rect = if named(i) { planet.union(labels[i].expand(2.0)) } else { planet };
                let response = ui.interact(rect, id, Sense::click());
                let label = system_label(c, i);
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
                if response.hovered() {
                    input.hovered = Some(i);
                }
                if response.clicked() {
                    input.select = Some(Some(i));
                    ta_ui::sound::play(&ctx, ta_ui::sound::UiSound::Toggle);
                }
                if response.double_clicked() && (c.attackable(i) || c.defendable(i)) {
                    input.brief = Some(i);
                }
                if response.has_focus() {
                    focused = Some(i);
                }
            }
        }
        // The arrow keys move between the systems, as they lie on the map; Enter briefs the
        // focused one when it is within reach (and chooses it in any case).
        if let Some(i) = focused {
            for (key, dir) in [(Key::ArrowRight, vec2(1.0, 0.0)), (Key::ArrowLeft, vec2(-1.0, 0.0)), (Key::ArrowDown, vec2(0.0, 1.0)), (Key::ArrowUp, vec2(0.0, -1.0))] {
                if ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, key))
                    && let Some(j) = nearest_towards(&points, i, dir)
                {
                    ui.memory_mut(|m| m.request_focus(system_id(j)));
                    input.select = Some(Some(j));
                }
            }
            if ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::Enter)) {
                input.select = Some(Some(i));
                if c.attackable(i) || c.defendable(i) {
                    input.brief = Some(i);
                }
            }
        }
        let hot = |i: usize| input.hovered == Some(i) || self.selected == Some(i) || focused == Some(i);

        // Territory: each system's ground, brighter towards its world; the rim of what is held
        // in the owner's colour, the front between ours and theirs in brass.
        let reach: Vec<f32> = (0..n).map(|i| territory_reach(c, &points, i, k)).collect();
        let cells = territories(&points, &reach);
        for (i, cell) in cells.iter().enumerate() {
            if cell.len() < 3 || !charted[i] {
                continue;
            }
            let owner = if c.systems[i].owner == Owner::Player { ours } else { theirs };
            let (centre_alpha, edge_alpha) = if c.systems[i].owner == Owner::Player { (56, 20) } else { (22, 4) };
            let shade = |a: u8| Color32::from_rgba_unmultiplied(owner.r(), owner.g(), owner.b(), a);
            let mut mesh = Mesh::default();
            mesh.colored_vertex(points[i], shade(centre_alpha));
            for &(p, _) in cell {
                mesh.colored_vertex(p, shade(edge_alpha));
            }
            let m = cell.len() as u32;
            for v in 0..m {
                mesh.add_triangle(0, 1 + v, 1 + (v + 1) % m);
            }
            painter.add(Shape::mesh(mesh));
        }
        for (i, cell) in cells.iter().enumerate() {
            let mine = c.systems[i].owner == Owner::Player;
            for v in 0..cell.len() {
                let (a, across) = cell[v];
                let b = cell[(v + 1) % cell.len()].0;
                match across {
                    // The front, drawn once from our side: a glow, then the line.
                    Some(j) if mine && c.systems[j].owner != Owner::Player => {
                        painter.line_segment([a, b], Stroke::new(6.0 * k, AMBER.gamma_multiply(0.10)));
                        painter.line_segment([a, b], Stroke::new(1.6 * k, AMBER.gamma_multiply(0.62)));
                    }
                    // (Our territory's rim, faint; the enemy's own melts into space.)
                    None if mine => {
                        painter.line_segment([a, b], Stroke::new(1.0, ours.gamma_multiply(0.36)));
                    }
                    _ => {}
                }
            }
        }

        // Lanes: held ones in our colour, those we can attack along in brass with their dashes
        // flowing from ours towards theirs, the rest dashed; brighter by a system in hand.
        let holes: Vec<Rect> = labels.iter().map(|r| r.expand(4.0)).collect();
        for &(a, b) in &c.lanes {
            let (oa, ob) = (c.systems[a].owner == Owner::Player, c.systems[b].owner == Owner::Player);
            // (The front's lanes run from our end.)
            let (a, b) = if ob && !oa { (b, a) } else { (a, b) };
            let (pa, pb) = (points[a], points[b]);
            let lit = if hot(a) || hot(b) { 1.0 } else { 0.0 };
            let length = pa.distance(pb).max(1.0);
            for (t0, t1) in lane_pieces(pa, pb, &holes) {
                let (qa, qb) = (pa.lerp(pb, t0), pa.lerp(pb, t1));
                if oa && ob {
                    painter.line_segment([qa, qb], Stroke::new(5.0 * k, ours.gamma_multiply(0.12 + 0.08 * lit)));
                    painter.line_segment([qa, qb], Stroke::new(2.0 * k, ours.gamma_multiply(0.72 + 0.28 * lit)));
                } else if oa != ob {
                    let (dash, gap) = (7.0 * k, 5.0 * k);
                    let flow = (view.time * 18.0 * k) % (dash + gap);
                    let offset = ((dash + gap) - (t0 * length - flow).rem_euclid(dash + gap)) % (dash + gap);
                    let color = AMBER.gamma_multiply(0.66 + 0.34 * lit);
                    painter.extend(Shape::dashed_line_with_offset(&[qa, qb], Stroke::new(2.2 * k.sqrt(), color), &[dash], &[gap], offset));
                } else {
                    let offset = (11.0 - (t0 * length) % 11.0) % 11.0;
                    let known = charted[a] && charted[b];
                    let color = Color32::from_rgba_unmultiplied(130, 140, 160, if lit > 0.0 { 130 } else if known { 70 } else { 34 });
                    painter.extend(Shape::dashed_line_with_offset(&[qa, qb], Stroke::new(1.2 * k.sqrt(), color), &[6.0], &[5.0], offset));
                }
            }
        }

        // The route to the enemy capital, when it is wanted (the capital in hand, or asked for).
        let route_wanted = self.show_route || [input.hovered, self.selected, focused].contains(&Some(c.capital));
        if route_wanted && !c.won {
            let route = c.route_to_capital();
            for pair in route.windows(2) {
                let (pa, pb) = (points[pair[0]], points[pair[1]]);
                painter.line_segment([pa, pb], Stroke::new(9.0 * k, AMBER_BRIGHT.gamma_multiply(0.10)));
                let length = pa.distance(pb).max(1.0);
                let steps = (length / (14.0 * k)).floor().max(1.0) as usize;
                for s in 1..steps {
                    let f = s as f32 / steps as f32;
                    let pulse = 0.5 + 0.5 * (view.time * 4.0 - f * 6.0).sin();
                    painter.circle_filled(pa.lerp(pb, f), 1.6 * k.sqrt() + pulse * 0.8, AMBER_BRIGHT.gamma_multiply(0.55 + 0.45 * pulse));
                }
            }
        }

        // The enemy's counter-attack under way.
        if let Some(counter) = c.counter {
            paint_counter(&painter, (points[counter.from], view.radius(c, counter.from)), (points[counter.system], view.radius(c, counter.system)), k, view.time);
        }

        // The planets.
        let defeats = c.defeats();
        for (i, s) in c.systems.iter().enumerate() {
            let p = points[i];
            let r = view.radius(c, i) * if input.hovered == Some(i) { 1.08 } else { 1.0 };
            let owner = if s.owner == Owner::Player { ours } else { theirs };
            if charted[i] {
                w::soft_glow(&painter, p, r, r + 12.0 * k, Color32::from_rgba_unmultiplied(owner.r(), owner.g(), owner.b(), 95));
                self.planets.paint(&painter, p, r, &s.planet, crate::planet::seed_of(&s.map), || paint_planet(&painter, p, r, planet_color(&s.planet)));
                painter.circle_stroke(p, r + 1.5, Stroke::new(2.0 * k.sqrt(), owner));
            } else {
                paint_uncharted(&painter, p, r, k);
            }
            if c.attackable(i) {
                let pulse = (view.time * 3.0 + i as f32).sin() * 0.5 + 0.5;
                painter.circle_stroke(p, r + (6.0 + pulse * 3.0) * k, Stroke::new(1.6 * k.sqrt(), AMBER.gamma_multiply(0.5 + 0.5 * pulse)));
            }
            // Under attack: a quicker red pulse.
            if c.defendable(i) {
                let pulse = (view.time * 5.0).sin() * 0.5 + 0.5;
                painter.circle_stroke(p, r + (5.0 + pulse * 4.0) * k, Stroke::new(2.2 * k.sqrt(), RED.gamma_multiply(0.55 + 0.45 * pulse)));
            }
            if i == c.capital {
                paint_crown(&painter, p, r, k);
            }
            if i == c.home {
                paint_home(&painter, p, r, k);
            }
            // What it brings, badged by its shoulder.
            if charted[i] && c.reward(i) != Reward::None {
                paint_reward_badge(&painter, p + vec2(r * 0.95, -r * 0.95), k, c.reward(i));
            }
            // Defeats here, while it holds out: a small red count by its foot.
            if charted[i] && s.owner == Owner::Enemy && defeats[i] > 0 {
                let at = p + vec2(r * 0.85, r * 0.85);
                let text = painter.layout_no_wrap(format!("×{}", defeats[i]), strong(9.5 * k.sqrt()), Color32::WHITE);
                let pill = Rect::from_center_size(at + vec2(text.size().x * 0.5, 0.0), text.size() + vec2(8.0, 2.0));
                painter.rect_filled(pill, pill.height() / 2.0, Color32::from_rgb(150, 36, 28));
                painter.galley(pill.center() - text.size() / 2.0, text, Color32::WHITE);
            }
            if self.selected == Some(i) || focused == Some(i) {
                paint_reticle(&painter, p, r, k, view.time);
            }
            // Just taken: rings of our colour going out from it.
            if let Some((taken, since)) = self.capture
                && taken == i
            {
                let age = (ctx.input(|inp| inp.time) - since) as f32;
                for ring in 0..2 {
                    let f = ((age - ring as f32 * 0.35) / 1.4).clamp(0.0, 1.0);
                    if f > 0.0 && f < 1.0 {
                        painter.circle_stroke(p, r + f * 70.0 * k, Stroke::new(3.0 * (1.0 - f) * k, ours.gamma_multiply(1.0 - f)));
                    }
                }
            }
        }
        if let Some((_, since)) = self.capture
            && ctx.input(|inp| inp.time) - since > 2.2
        {
            self.capture = None;
        }

        // The names last, over everything, outlined dark so they read over glows and lanes.
        for (i, galley) in names.into_iter().enumerate() {
            let color = if hot(i) { TEXT_BRIGHT } else if c.systems[i].owner == Owner::Player { Color32::from_rgb(178, 196, 222) } else { STEEL };
            let at = labels[i].min.round();
            for (dx, dy) in [(-1.0, -1.0), (0.0, -1.0), (1.0, -1.0), (-1.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (0.0, 1.0), (1.0, 1.0)] {
                painter.galley_with_override_text_color(at + vec2(dx, dy), galley.clone(), Color32::from_black_alpha(170));
            }
            painter.galley_with_override_text_color(at + vec2(0.0, 1.5), galley.clone(), Color32::from_black_alpha(120));
            painter.galley_with_override_text_color(at, galley, color);
        }
        if interactive && let Some(i) = focused.filter(|&j| ui.memory(|m| m.focused().is_some_and(|f| f == system_id(j)))) {
            let rect = Rect::from_center_size(points[i], Vec2::splat(2.0 * planets[i].1 + 26.0 * k));
            painter.rect_stroke(rect, 4.0, Stroke::new(1.5, AMBER_BRIGHT.gamma_multiply(0.7)), egui::StrokeKind::Outside);
        }
        if let Some(i) = input.hovered.or(focused) {
            self.system_card(&ctx, c, i, points[i], planets[i].1, view);
        }
        input
    }

    /// The card beside a system the pointer is on: its name, its world and map, who holds it and
    /// how strongly, and what can be done with it.
    fn system_card(&self, ctx: &egui::Context, c: &Conquest, i: usize, p: Pos2, r: f32, view: View) {
        let s = &c.systems[i];
        let charted = c.charted()[i];
        let width = 250.0;
        // Beside the planet, on the side with room.
        let right = p.x + r + 18.0 + width < view.bounds.max.x;
        let x = if right { p.x + r + 18.0 } else { p.x - r - 18.0 - width };
        let y = (p.y - 34.0).clamp(view.bounds.min.y, (view.bounds.max.y - 120.0).max(view.bounds.min.y));
        egui::Area::new(Id::new(("conquest_card", i))).fixed_pos(pos2(x, y)).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(Color32::from_rgba_unmultiplied(14, 16, 20, 238)).stroke(Stroke::new(1.0, LINE_HI)).show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 3.0;
                if !charted {
                    let name = if i == c.capital { s.name.to_uppercase() } else { t("UNCHARTED SYSTEM").to_string() };
                    ui.label(RichText::new(name).font(strong(13.5)).color(TEXT_BRIGHT));
                    if i == c.capital {
                        ui.label(RichText::new(t("The enemy capital: take it to win")).color(AMBER_BRIGHT));
                    }
                    ui.label(RichText::new(t("Uncharted: take a system beside it to learn more")).font(body(12.0)).color(STEEL));
                    return;
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(s.name.to_uppercase()).font(strong(13.5)).color(TEXT_BRIGHT));
                    if s.owner == Owner::Enemy {
                        let (rect, _) = ui.allocate_exact_size(vec2(42.0, 12.0), Sense::hover());
                        paint_threat(ui.painter(), pos2(rect.min.x + 2.0, rect.center().y), s.threat, 9.0);
                    }
                });
                ui.label(RichText::new(s.planet.to_uppercase()).font(strong(10.5)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
                if c.reward(i) != Reward::None {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        paint_reward(ui.painter(), rect.center(), 13.0, c.reward(i));
                        ui.label(RichText::new(t(c.reward(i).label())).font(strong(11.0)).color(reward_color(c.reward(i))).extra_letter_spacing(ta_ui::i18n::tracking(0.8)));
                    });
                }
                if let Some(counter) = c.counter.filter(|counter| counter.system == i && c.defendable(i)) {
                    ui.label(w::caption("UNDER ATTACK").color(RED));
                    ui.label(RichText::new(super::panels::counter_when(c, counter)).color(TEXT_BRIGHT));
                }
                let (line, color) = match s.owner {
                    Owner::Player if i == c.home => (t("Your home system").to_string(), Color32::from_rgb(150, 190, 240)),
                    Owner::Player => (t("Held by your forces").to_string(), Color32::from_rgb(150, 190, 240)),
                    Owner::Enemy if i == c.capital => (t("The enemy capital: take it to win").to_string(), AMBER_BRIGHT),
                    Owner::Enemy => (super::enemy_line(c, i, None), TEXT),
                };
                ui.label(RichText::new(line).color(color));
                let hint = if c.defendable(i) { t("Click to plan its defence") } else if c.attackable(i) { t("Within reach: click to plan the attack") } else if s.owner == Owner::Enemy && !c.won { t("Out of reach: take a neighbouring system first") } else { "" };
                if !hint.is_empty() {
                    ui.label(RichText::new(hint).font(body(12.0)).color(if c.defendable(i) { RED } else if c.attackable(i) { AMBER } else { STEEL_DIM }));
                }
            });
        });
    }
}

/// A system's button's id on the map.
fn system_id(i: usize) -> Id {
    Id::new(("conquest_system", i))
}

/// What a screen reader says of a system's button.
fn system_label(c: &Conquest, i: usize) -> String {
    let s = &c.systems[i];
    if !c.charted()[i] {
        return if i == c.capital { tf("{name}, the enemy capital, uncharted", &[("name", &s.name)]) } else { t("Uncharted system").to_string() };
    }
    if c.defendable(i) {
        return tf("{name}, held, under attack", &[("name", &s.name)]);
    }
    let whose = match s.owner {
        Owner::Player => t("held"),
        Owner::Enemy if i == c.capital => t("the enemy capital"),
        Owner::Enemy => t("enemy"),
    };
    let reach = if c.attackable(i) { t("within reach") } else { "" };
    tf("{name}, {whose} {reach}", &[("name", &s.name), ("whose", &whose), ("reach", &reach)]).trim().trim_end_matches(',').to_string()
}

/// The system nearest `from` in direction `dir` (within 60 degrees of it), if any: its distance
/// counted double sideways.
fn nearest_towards(points: &[Pos2], from: usize, dir: Vec2) -> Option<usize> {
    let p = points[from];
    (0..points.len())
        .filter(|&j| j != from)
        .filter_map(|j| {
            let d = points[j] - p;
            let along = d.dot(dir);
            let across = (d - dir * along).length();
            (along > 0.0 && across <= along * 1.75).then_some((j, along + 2.0 * across))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(j, _)| j)
}

/// How far a system's territory reaches: past the middle of its lanes to its neighbours (so
/// neighbours' territories meet), a little round one without near neighbours.
fn territory_reach(c: &Conquest, points: &[Pos2], i: usize, k: f32) -> f32 {
    let longest = c.lanes.iter().filter_map(|&(a, b)| if a == i { Some(b) } else if b == i { Some(a) } else { None }).map(|j| points[i].distance(points[j])).fold(0.0f32, f32::max);
    (longest * 0.62).clamp(48.0 * k, 150.0 * k)
}

/// Each point's territory: the ground nearer it than any other point, as far as its `reach` (a
/// convex polygon, its corners in order, each with what lies across the edge it begins: another
/// point's territory, or `None` for the rim of its reach).
pub(super) fn territories(points: &[Pos2], reach: &[f32]) -> Vec<Vec<(Pos2, Option<usize>)>> {
    const ROUND: usize = 64;
    points
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            let mut cell: Vec<(Pos2, Option<usize>)> = (0..ROUND).map(|s| {
                let a = s as f32 / ROUND as f32 * TAU;
                (p + vec2(a.cos(), a.sin()) * reach[i], None)
            }).collect();
            for (j, &q) in points.iter().enumerate() {
                if j == i || p.distance(q) > reach[i] * 2.0 {
                    continue;
                }
                // Keep the side nearer p of the line half way between them.
                let mid = p.lerp(q, 0.5);
                let normal = q - p;
                let inside = |x: Pos2| (x - mid).dot(normal) <= 0.0;
                let mut out = Vec::with_capacity(cell.len() + 2);
                for v in 0..cell.len() {
                    let (a, edge) = cell[v];
                    let b = cell[(v + 1) % cell.len()].0;
                    let (ia, ib) = (inside(a), inside(b));
                    if ia {
                        out.push((a, edge));
                    }
                    if ia != ib {
                        let f = (mid - a).dot(normal) / (b - a).dot(normal);
                        // (Leaving, the edge from here runs along the cut; entering, along a-b.)
                        out.push((a + (b - a) * f, if ia { Some(j) } else { edge }));
                    }
                }
                cell = out;
                if cell.len() < 3 {
                    break;
                }
            }
            cell
        })
        .collect()
}

/// Where each system's name goes (`sizes`): beside its planet (`planets`: where, its radius,
/// how far up its marks reach), on whichever side keeps clear of the other names, the
/// planets, the lanes (`lanes`) and the edges of `bounds`, and nearer its own planet than any
/// other, below by choice. The same galaxy always places its names the same way.
pub(super) fn place_labels(planets: &[(Pos2, f32, f32)], sizes: &[Vec2], lanes: &[(Pos2, Pos2)], bounds: Rect, k: f32) -> Vec<Rect> {
    let n = planets.len();
    let clear = LABEL_CLEAR * k;
    // The sides, each with how much less it is liked than below.
    let candidate = |i: usize, side: usize| -> (Rect, f32) {
        let (p, r, lift) = planets[i];
        let size = sizes[i];
        let d = r + clear;
        let dd = d * std::f32::consts::FRAC_1_SQRT_2;
        let (min, cost) = match side {
            0 => (pos2(p.x - size.x / 2.0, p.y + d), 0.0),
            1 => (pos2(p.x - size.x / 2.0, p.y - d - lift - size.y), 0.5),
            2 => (pos2(p.x + d, p.y - size.y / 2.0), 0.8),
            3 => (pos2(p.x - d - size.x, p.y - size.y / 2.0), 0.8),
            4 => (pos2(p.x + dd, p.y + dd), 1.0),
            5 => (pos2(p.x - dd - size.x, p.y + dd), 1.0),
            6 => (pos2(p.x + dd, p.y - dd - lift - size.y), 1.2),
            _ => (pos2(p.x - dd - size.x, p.y - dd - lift - size.y), 1.2),
        };
        (Rect::from_min_size(min, size), cost)
    };
    let cost = |i: usize, rect: Rect, placed: &[Option<Rect>]| -> f32 {
        let mut cost = 0.0;
        // (Names keep a line's room above and below each other: stacked close, two read as one.)
        let room = rect.expand2(vec2(5.0, 8.0) * k);
        for (j, other) in placed.iter().enumerate() {
            if let Some(other) = other
                && j != i
                && room.intersects(*other)
            {
                cost += 20.0 + room.intersect(*other).area() / 100.0;
            }
        }
        let own = rect.distance_to_pos(planets[i].0);
        for (j, &(q, r, _)) in planets.iter().enumerate() {
            if j == i {
                continue;
            }
            let d = rect.distance_to_pos(q);
            if d < r + 12.0 * k {
                cost += 20.0;
            }
            // (A name nearer another planet than its own reads as that one's.)
            if d < own {
                cost += 6.0;
            }
        }
        let near = rect.expand(2.0);
        cost += 3.0 * lanes.iter().filter(|&&(a, b)| segment_hits(a, b, near).is_some()).count() as f32;
        if !bounds.contains_rect(rect) {
            cost += 30.0;
        }
        cost
    };
    // Widest first; then over again, each name moved to its best side with the others set.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| sizes[b].x.total_cmp(&sizes[a].x).then(a.cmp(&b)));
    let mut placed: Vec<Option<Rect>> = vec![None; n];
    for _ in 0..3 {
        for &i in &order {
            let best = (0..8).map(|side| candidate(i, side)).map(|(rect, prefer)| (rect, prefer + cost(i, rect, &placed))).min_by(|a, b| a.1.total_cmp(&b.1));
            placed[i] = best.map(|b| b.0);
        }
    }
    placed.into_iter().map(|r| r.unwrap_or(Rect::NOTHING)).collect()
}

/// Where the segment `a`-`b` runs inside `rect`: the stretch of it, as fractions of the way
/// from `a` (Liang–Barsky), if any.
pub(super) fn segment_hits(a: Pos2, b: Pos2, rect: Rect) -> Option<(f32, f32)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-d.x, a.x - rect.min.x), (d.x, rect.max.x - a.x), (-d.y, a.y - rect.min.y), (d.y, rect.max.y - a.y)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    (t0 < t1).then_some((t0, t1))
}

/// The stretches of the segment `a`-`b` (as fractions of the way from `a`) outside every one
/// of `holes`, leaving out any shorter than a point.
pub(super) fn lane_pieces(a: Pos2, b: Pos2, holes: &[Rect]) -> Vec<(f32, f32)> {
    let mut cut: Vec<(f32, f32)> = holes.iter().filter_map(|&h| segment_hits(a, b, h)).collect();
    cut.sort_by(|x, y| x.0.total_cmp(&y.0));
    let length = a.distance(b).max(1e-3);
    let mut pieces = Vec::new();
    let mut from = 0.0f32;
    for (t0, t1) in cut {
        if t0 > from {
            pieces.push((from, t0));
        }
        from = from.max(t1);
    }
    if from < 1.0 {
        pieces.push((from, 1.0));
    }
    pieces.retain(|(t0, t1)| (t1 - t0) * length >= 1.0);
    pieces
}

/// Deep space: a dark gradient, faint nebulae and twinkling stars (the same every frame), as
/// many as fit the size of `rect`.
pub(crate) fn paint_space(painter: &egui::Painter, rect: Rect, time: f32) {
    w::gradient_v(painter, rect, Color32::from_rgb(8, 11, 20), Color32::from_rgb(3, 4, 9));
    let hash = |k: u32| {
        let mut x = k.wrapping_mul(0x9E37_79B9) ^ 0x5BD1_E995;
        x ^= x >> 15;
        x = x.wrapping_mul(0x2C1B_3C6D);
        x ^= x >> 12;
        (x & 0xFFFF) as f32 / 65535.0
    };
    for (k, color) in [(0u32, [60, 40, 110]), (1, [20, 70, 110]), (2, [110, 40, 60])] {
        let c = pos2(rect.min.x + rect.width() * (0.2 + 0.3 * k as f32 + 0.1 * hash(k * 7 + 1)), rect.min.y + rect.height() * (0.3 + 0.4 * hash(k * 7 + 2)));
        w::soft_glow(painter, c, 0.0, rect.height() * 0.5, Color32::from_rgba_unmultiplied(color[0], color[1], color[2], 60));
    }
    let stars = ((rect.area() / 2200.0) as u32).clamp(60, 520);
    for k in 0..stars {
        let p = pos2(rect.min.x + hash(k * 3) * rect.width(), rect.min.y + hash(k * 3 + 1) * rect.height());
        let size = 0.5 + hash(k * 3 + 2).powi(3) * 1.6;
        let twinkle = 0.55 + 0.45 * (time * (0.6 + hash(k) * 1.5) + k as f32).sin();
        let a = (70.0 + 150.0 * hash(k * 5 + 7) * twinkle) as u8;
        painter.circle_filled(p, size, Color32::from_rgba_unmultiplied(220, 228, 255, a));
    }
}

/// The text of an enemy's strength beside its pips: one or two computer players and how good.
pub(crate) fn strength_text(count: usize, difficulty: ta_ai::Difficulty) -> String {
    let who = match difficulty {
        ta_ai::Difficulty::Easy => t("Easy"),
        ta_ai::Difficulty::Normal => t("Normal"),
        ta_ai::Difficulty::Hard => t("Hard"),
    };
    if count > 1 { tf("{n} {level} computer players", &[("n", &count), ("level", &who)]) } else { tf("One {level} computer player", &[("level", &who)]) }
}

/// The bezel a galaxy's picture is set in.
pub(super) fn frame(painter: &egui::Painter, rect: Rect) {
    painter.add(ta_ui::metal::bezel(rect, 4.0, true));
}
