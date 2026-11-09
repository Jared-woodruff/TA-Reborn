//! The radial context menu's drawing, and the pointer's part in it (see `crate::radial` for what
//! it offers and `Game::radial_pick` for what a choice does): a ring of bevelled plates of
//! gunmetal round the point it was opened at, each with its glyph or picture, its word and its
//! key, the one under the pointer risen in brass, the unit on a seat in the middle with its health
//! round it and what the lit choice does under the ring. Its shapes are `radial/shape.rs`, how
//! they are painted `radial/paint.rs`, and how it moves `crate::radial_motion`: the ring unfolds
//! plate by plate, the light follows the pointer on springs, a choice is pressed in, a ring opened
//! from another unfolds from the plate that opened it while the one before folds into the middle,
//! and the menu closes by folding into the pointer, or by the plate that made a choice flaring as
//! the rest let go. The menu takes the whole screen's pointer while it is up, and lets go of it
//! the moment a choice closes it (what it showed finishes on its own, with no say over the game).

use egui::{Order, Pos2, Vec2 as EVec2, pos2, vec2};
use ta_ui::sound::UiSound;
use ta_vfs::Vfs;

use super::{Hud, UnitPic};
use crate::game::Game;
use crate::radial::{self, Entry};
use crate::radial_motion::{self as motion, Anim, Change, RingKey, Timing};

mod paint;
mod shape;

use paint::{Hub, Look, Space};

/// What the interface keeps of the menu from one frame to the next: how it moves, what it last
/// drew (the ring that a change leaves behind, the menu that is closed), and those two on their
/// way out.
#[derive(Default)]
pub(super) struct View {
    anim: Anim,
    last: Option<Scene>,
    ghost: Option<Ghost>,
    fade: Option<Fade>,
    line: Line,
}

/// What a frame showed, to show again as it goes.
#[derive(Clone)]
struct Scene {
    centre: Pos2,
    scale: f32,
    depth: usize,
    entries: Vec<Entry>,
    pics: Vec<Option<UnitPic>>,
    /// How lit each plate was.
    lit: Vec<f32>,
    hub: HubData,
}

/// The middle of the menu, in what it needs to be drawn again.
#[derive(Clone)]
struct HubData {
    picture: Option<UnitPic>,
    name: String,
    health: Option<f32>,
    back: Option<(String, usize, Option<usize>)>,
    page: Option<(usize, usize)>,
}

impl HubData {
    fn view(&self, hot: f32, page_from: Option<usize>, page_mix: f32) -> Hub<'_> {
        Hub { picture: self.picture, name: &self.name, health: self.health, back: self.back.as_ref().map(|(n, p, f)| (n.as_str(), *p, *f)), page: self.page, page_from, page_mix, hot }
    }
}

/// The ring a change has left, going.
struct Ghost {
    scene: Scene,
    since: f64,
    change: Change,
    picked: Option<usize>,
}

/// A menu that has been closed, folding away (drifting `pull` points towards the pointer as it
/// goes, when no choice was made).
struct Fade {
    scene: Scene,
    since: f64,
    chosen: Option<usize>,
    pull: [f32; 2],
}

/// The words under the ring, the words before them and when they changed.
#[derive(Default)]
struct Line {
    cur: String,
    prev: String,
    at: f64,
}

impl Hud {
    /// The radial menu, while one is open (and a moment after): its ring, its pointer, its choices.
    pub(super) fn radial_menu(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &mut Game) {
        let mut view = std::mem::take(&mut self.radial_view);
        self.radial_frame(&mut view, ctx, vfs, game);
        self.radial_view = view;
    }

    fn radial_frame(&mut self, view: &mut View, ctx: &egui::Context, vfs: &Vfs, game: &mut Game) {
        let now = ctx.input(|i| i.time);
        let tm = Timing::of(ta_ui::motion::still(ctx));
        let calm = ta_ui::motion::reduced_flashes(ctx);
        let Some(mut radial) = game.radial.take() else {
            // No menu: one that has just been closed goes on folding away.
            view.last = None;
            view.ghost = None;
            if view.fade.is_some() {
                self.radial_fade(view, ctx, now, &tm, calm);
            }
            return;
        };
        if game.sim.unit(radial.unit).is_none() {
            *view = View::default();
            return;
        }
        let screen = ctx.content_rect();
        let ppp = ctx.pixels_per_point();
        let scale = (screen.height() / 720.0).clamp(1.0, 1.5);
        let first = radial.born.is_none();
        radial.born.get_or_insert(now);
        if first {
            *view = View::default();
        }
        // A menu told to close (by a key, before this frame) is let go of at once.
        if let Some(leaving) = radial.leaving {
            self.radial_let_go(view, ctx, now, &tm, calm, leaving.chosen, radial.from_pad);
            return;
        }
        // Kept inside the battlefield the interface leaves free (under the top bar, over the deck),
        // room under it for the line of words: where it has to move, the pointer goes with it (the
        // app carries the cursor).
        let reach = (radial::OUTER + radial::MARGIN) * scale;
        let free = self.free_view(ctx).intersect(screen);
        let (low, high) = (free.min + vec2(reach, reach), free.max - vec2(reach, reach + 52.0 * scale));
        let at = pos2(radial.centre.x / ppp, radial.centre.y / ppp);
        let centre = pos2(at.x.clamp(low.x, high.x.max(low.x)), at.y.clamp(low.y, high.y.max(low.y)));
        if first && centre != at {
            radial.centre = glam::Vec2::new(centre.x, centre.y) * ppp;
            radial.warp = Some(radial.centre);
        }
        let mut entries = radial::entries(game, &radial);
        if entries.is_empty() {
            return;
        }
        // The pointer, in points from the middle of the ring: it lights what it is over (a stick
        // or the d-pad keep what they lit until the pointer moves).
        let pointer = ctx.pointer_latest_pos();
        let moved = ctx.input(|i| i.pointer.delta() != EVec2::ZERO);
        if let Some(p) = pointer && (moved || !radial.from_pad) {
            radial.from_pad = false;
            radial.hover = radial::pick(glam::Vec2::new(p.x - centre.x, p.y - centre.y), entries.len(), scale);
        }
        if radial.hover.is_some_and(|h| h >= entries.len()) {
            radial.hover = None;
        }
        // Clicks (not on the frame it opened on: the click that opened it is still being let go). A
        // middle click (not a drag: that pans the view) is the right button's in a menu not held
        // open: back, or a queued unit fewer.
        let (primary, secondary, middle_click, shift) = ctx.input(|i| {
            (i.pointer.button_released(egui::PointerButton::Primary), i.pointer.button_released(egui::PointerButton::Secondary), i.pointer.button_clicked(egui::PointerButton::Middle), i.modifiers.shift)
        });
        let offset = pointer.map(|p| vec2(p.x - centre.x, p.y - centre.y));
        let far = offset.is_none_or(|o| o.length() > (radial::OUTER + radial::REACH) * scale);
        let middle = offset.is_some_and(|o| o.length() < radial::INNER * scale * 0.7);
        if !first {
            let lit = radial.hover.filter(|&h| h < entries.len());
            let button = if primary {
                Some((radial::Button::Left, radial.held))
            } else if secondary {
                Some((radial::Button::Right, radial.held))
            } else if middle_click {
                Some((radial::Button::Right, false))
            } else {
                None
            };
            let fewer = lit.is_some_and(|h| matches!(entries[h].id, radial::Id::Produce(_)));
            if let Some((button, held)) = button {
                match radial::released(button, lit, held, radial.from_pad, middle, far, fewer) {
                    radial::Release::Pick(h) => {
                        // (The button has been let go: a choice that opens a ring leaves it for clicks.)
                        radial.held = false;
                        game.radial_pick(&mut radial, &entries, h, shift, false);
                    }
                    radial::Release::Fewer(h) => game.radial_pick(&mut radial, &entries, h, shift, true),
                    radial::Release::Back => {
                        if !radial.back() {
                            radial.close(None);
                        }
                    }
                    radial::Release::Close => radial.close(None),
                    radial::Release::Settle => radial.held = false,
                    radial::Release::Nothing => {}
                }
            }
        }
        // What was picked or refused (by the pointer just now, or by a key before the frame) is
        // pressed or shaken, with its sound.
        if let Some(i) = radial.picked.take() {
            view.anim.press(i, now);
            ta_ui::sound::play(ctx, UiSound::Button);
        }
        if let Some(i) = radial.refused.take() {
            view.anim.refuse(i, now);
            ta_ui::sound::play(ctx, UiSound::Refuse);
        }
        // A menu that a choice or a click has told to close is let go of at once, so that the game has
        // its pointer back (a click on the ground that follows a choice is the ground's).
        if let Some(leaving) = radial.leaving {
            self.radial_let_go(view, ctx, now, &tm, calm, leaving.chosen, radial.from_pad);
            return;
        }
        // (A choice may have opened another ring, and a ring with nothing to choose from, a purpose
        // the mission has left no building of, is no ring: the menu goes, as it always did.)
        entries = radial::entries(game, &radial);
        if entries.is_empty() {
            return;
        }
        if radial.hover.is_some_and(|h| h >= entries.len()) {
            radial.hover = None;
        }
        let count = entries.len();
        let depth = radial.rings.len() - 1;

        // The motion: which ring is on top and how it came (the one before goes on, folding).
        let dt = view.anim.tick(now);
        if let Some(change) = view.anim.ring(RingKey { depth: radial.rings.len(), menu: radial.ring() }, &radial.places, count, now) {
            if let Some(scene) = view.last.take() {
                let picked = if let Change::Into { from, .. } = change { from } else { None };
                view.ghost = Some(Ghost { scene, since: now, change, picked });
            }
            if let Change::Back { to, .. } = change {
                // (The way back keeps your place: the stick is on the plate it came from.)
                if radial.from_pad {
                    radial.hover = to.filter(|&t| t < entries.len());
                }
                ta_ui::sound::play(ctx, UiSound::Back);
            }
        }
        let hovered = radial.hover.filter(|&h| h < entries.len());
        let health = radial::health(game, &radial);
        view.anim.follow(dt, &tm, count, hovered, None, middle, health);

        // Pictures for the choices that have one, and for the middle.
        let pic_size = 34.0 * scale;
        let pics: Vec<Option<UnitPic>> = entries.iter().map(|e| e.picture.and_then(|ty| self.unit_pic(ctx, vfs, game, ty, pic_size))).collect();
        let owner = game.sim.unit(radial.unit).map_or(game.local_player, |u| u.owner);
        let centre_pic = game.sim.unit(radial.unit).and_then(|u| self.unit_pic_of(ctx, vfs, game, u.type_id, 40.0 * scale, owner));
        let title = radial::title(game, &radial);
        let confirming = radial.ring() == radial::Menu::Confirm;
        // The middle: the unit's name, and in a ring opened from another the ring's own and where it
        // came from. (The question of a self-destruct is the unit's: it is the one to be blown up.)
        let unit_name = if confirming { game.sim.unit(radial.unit).map(|u| crate::chat::unit_label(&game.sim, u.type_id)).unwrap_or_default() } else { title.clone() };
        let ring_name = if confirming { Some(unit_name.clone()) } else { radial::ring_name(&radial) };
        let came_from = depth.checked_sub(1).and_then(|k| radial.places.get(k)).copied();
        let hub = HubData {
            picture: centre_pic,
            name: unit_name,
            health: health.map(|_| view.anim.health()),
            back: ring_name.map(|name| (name, came_from.map_or(0, |p| p.of), came_from.map(|p| p.index))),
            page: radial::paging(game, &radial),
        };
        // What the lit choice does, under the ring (the question of a self-destruct when none is lit).
        let line = match hovered {
            Some(h) => entries[h].hint.clone(),
            None if confirming => title,
            None => String::new(),
        };
        if line != view.line.cur {
            view.line.prev = std::mem::replace(&mut view.line.cur, line);
            view.line.at = now;
        }
        let lit: Vec<f32> = (0..count).map(|i| view.anim.lit(i)).collect();
        let scene = Scene { centre, scale, depth, entries, pics, lit, hub };

        // Drawn: the shade, the rim and the hub as the menu comes up, the ring before (going) under
        // the ring now (coming in), the line of words.
        let since_open = view.anim.since_open(now);
        let rim = motion::clamp01(since_open / tm.rim.max(1e-4));
        let rim_alpha = motion::smooth(rim * 1.4);
        let swell = motion::lerp(1.0 - 0.14 * tm.travel, 1.0, motion::out_cubic(rim));
        let rim_space = Space { centre, scale, r0: radial::INNER * scale * swell, r1: radial::OUTER * scale * swell, alpha: rim_alpha, calm };
        let plate_space = Space { alpha: 1.0, r0: radial::INNER * scale, r1: radial::OUTER * scale, ..rim_space };
        // (The shade under it comes as the plates do, not before them.)
        let shade_space = Space { alpha: rim_alpha * motion::smooth(since_open / tm.open_total(count).max(1e-3)), ..rim_space };
        let since_ring = view.anim.since_ring(now);
        let change = view.anim.change;
        let looks = plate_looks(&scene.entries, &view.anim, now, change, since_ring, &tm);
        let ghost_t = view.ghost.as_ref().map(|g| (now - g.since) as f32);
        let ghost_looks = view.ghost.as_ref().zip(ghost_t).map(|(g, t)| going_looks(&g.scene, g.change, g.picked, t, &tm));
        if ghost_t.is_some_and(|t| t > tm.turn_out + 0.03) {
            view.ghost = None;
        }
        // The hub's face changes with the ring (the unit, the way back): the one before fades as the new comes.
        let face_in = if matches!(change, Change::Open) { 1.0 } else { motion::smooth(since_ring / (tm.turn_in * 0.65).max(1e-3)) };
        let page_mix = motion::smooth(since_ring / tm.turn_in.max(1e-3));
        let page_from = match (change, view.ghost.as_ref()) {
            (Change::Page { .. }, Some(g)) => g.scene.hub.page.map(|(at, _)| at),
            _ => None,
        };
        // (The words cross over in a short fade, shorter still with reduced motion.)
        let mix = motion::smooth(motion::progress(now, view.line.at, tm.plate));
        let busy = view.anim.busy(now, &tm, count) || view.ghost.is_some() || mix < 1.0 || page_mix < 1.0;
        if busy {
            ctx.request_repaint();
        }
        let screen_width = screen.width();
        let hub_hot = view.anim.hub();
        let bead = view.anim.bead();
        let line = (view.line.cur.clone(), view.line.prev.clone());
        egui::Area::new(egui::Id::new("radial_menu")).order(Order::Foreground).fixed_pos(screen.min).constrain(false).show(ctx, |ui| {
            // The whole screen's pointer is the menu's while it is up.
            ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
            ui.set_clip_rect(screen);
            let painter = ui.painter().clone();
            paint::pool(&painter, &shade_space);
            paint::bezel(&painter, &rim_space, count, &lamps_of(&scene.lit, count), bead, depth);
            if let (Some(g), Some(ls)) = (&view.ghost, &ghost_looks) {
                draw_plates(&painter, ctx, &plate_space, &g.scene, ls);
            }
            draw_plates(&painter, ctx, &plate_space, &scene, &looks);
            paint::hub_seat(&painter, &rim_space, hub_hot);
            if let (Some(g), Some(t)) = (&view.ghost, ghost_t) {
                // (The face that is going.)
                if !matches!(g.change, Change::Page { .. }) {
                    paint::hub_face(&painter, &rim_space, &g.scene.hub.view(hub_hot, None, 1.0), 1.0 - motion::smooth(t / 0.1));
                }
            }
            paint::hub_face(&painter, &rim_space, &scene.hub.view(hub_hot, page_from, page_mix), face_in);
            paint::plate(&painter, &rim_space, &line.0, &line.1, mix, (330.0 * scale).min(screen_width - 24.0), 5.0 * tm.travel);
        });
        ctx.set_cursor_icon(egui::CursorIcon::Default);
        view.last = Some(scene);
        game.radial = Some(radial);
    }

    /// The menu has been told to close: it is let go of (the game has its pointer back at once) and
    /// what it showed last goes on folding away by itself. With no choice made it drifts a little
    /// towards the pointer as it folds (not when a stick or the keys drove it: the pointer is
    /// wherever it was left).
    fn radial_let_go(&mut self, view: &mut View, ctx: &egui::Context, now: f64, tm: &Timing, calm: bool, chosen: Option<usize>, from_pad: bool) {
        if let Some(scene) = view.last.take() {
            let pull = match ctx.pointer_latest_pos() {
                Some(p) if chosen.is_none() && !from_pad => motion::closing_pull([p.x - scene.centre.x, p.y - scene.centre.y], 0.4, 70.0 * scene.scale),
                _ => [0.0, 0.0],
            };
            view.fade = Some(Fade { scene, since: now, chosen, pull });
        }
        view.ghost = None;
        self.radial_fade(view, ctx, now, tm, calm);
    }

    /// A menu that has been closed, folding away: towards the pointer, or, when a choice made it, the
    /// plate that made it flaring (a ripple of light, unless the player asked for no flashes) as the
    /// rest let go. It draws only: the game has its pointer back.
    fn radial_fade(&mut self, view: &mut View, ctx: &egui::Context, now: f64, tm: &Timing, calm: bool) {
        let Some(fade) = view.fade.as_ref() else { return };
        let t = (now - fade.since) as f32;
        let total = motion::closing_total(fade.chosen.is_some(), tm);
        if t > total + 0.02 {
            view.fade = None;
            return;
        }
        ctx.request_repaint();
        let scene = &fade.scene;
        let go = motion::clamp01(t / total.max(1e-4));
        // (Drifting towards the pointer as it folds: nowhere with reduced motion.)
        let drift = motion::closing_drift(fade.pull, go, tm);
        let (centre, scale) = (scene.centre + vec2(drift[0], drift[1]), scene.scale);
        let count = scene.entries.len().max(1);
        let rim_alpha = 1.0 - motion::smooth(go * 1.1);
        let swell = 1.0 - 0.1 * motion::in_cubic(go) * tm.travel;
        let rim_space = Space { centre, scale, r0: radial::INNER * scale * swell, r1: radial::OUTER * scale * swell, alpha: rim_alpha, calm };
        let plate_space = Space { alpha: 1.0, r0: radial::INNER * scale, r1: radial::OUTER * scale, ..rim_space };
        // (The shade under it goes with the first of the plates, not with the last: a shade with nothing on it
        // reads as a grey disc.)
        let shade_space = Space { alpha: 1.0 - motion::smooth(go * 2.2), ..rim_space };
        let looks: Vec<Look> = (0..scene.entries.len())
            .map(|i| {
                let pose = motion::closing(i, count, fade.chosen, t, tm);
                let made = fade.chosen == Some(i);
                let press = if made { motion::press(t) } else { 0.0 };
                Look { hot: if made { 1.0 } else { scene.lit.get(i).copied().unwrap_or(0.0) * (1.0 - motion::smooth(t / 0.1)) }, press, sink: press * tm.travel, alpha: pose.alpha, grow: pose.grow, swing: pose.swing, ..Look::REST }
            })
            .collect();
        let lamps = fade.chosen.map(|c| lamps_of(&one_lit(c, count), count)).unwrap_or_default();
        let ripple = fade.chosen.filter(|_| !calm && tm.travel > 0.0).map(|c| {
            let a = radial::angle(c, count);
            (shape::polar(centre, a, (radial::INNER + radial::OUTER) * 0.5 * scale), motion::clamp01((t - 0.02) / 0.28))
        });
        // (The menu's own area, that goes on from the frames before: a new area is not shown for its first frame.)
        egui::Area::new(egui::Id::new("radial_menu")).order(Order::Foreground).fixed_pos(ctx.content_rect().min).constrain(false).interactable(false).show(ctx, |ui| {
            ui.set_clip_rect(ctx.content_rect());
            let painter = ui.painter().clone();
            paint::pool(&painter, &shade_space);
            paint::bezel(&painter, &rim_space, count, &lamps, None, scene.depth);
            draw_plates(&painter, ctx, &plate_space, scene, &looks);
            paint::hub(&painter, &rim_space, &scene.hub.view(0.0, None, 1.0), 1.0);
            if let Some((at, p)) = ripple {
                paint::ripple(&painter, &plate_space, at, p);
            }
        });
    }
}

/// How each plate of the ring on top looks at `now`: lit as far as the pointer's light has come,
/// pressed or shaken, and as far into its place as the ring's coming in has got.
fn plate_looks(entries: &[Entry], anim: &Anim, now: f64, change: Change, since_ring: f32, tm: &Timing) -> Vec<Look> {
    let count = entries.len();
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let pose = motion::coming_in(change, i, count, since_ring, tm);
            let pressed = anim.pressed(i, now);
            Look {
                hot: anim.lit(i),
                // (The flash of a press is a flash, left out with reduced flashes; what sinks, shakes
                // or swells moves, and is left out with reduced motion: a refusal is then a tint of
                // the plate, a queued unit its count changing.)
                press: pressed,
                sink: pressed * tm.travel,
                warn: anim.refusal(i, now),
                alpha: pose.alpha,
                grow: pose.grow,
                swing: pose.swing,
                lift: 0.0,
                nudge: anim.shaken(i, now) * 3.5 * tm.travel,
                // (A unit just queued: the count on its plate pops.)
                pop: if matches!(e.id, radial::Id::Produce(_)) { pressed * tm.travel } else { 0.0 },
            }
        })
        .collect()
}

/// How each plate of the ring that is being left looks `t` seconds into its going: the plate that
/// opened the ring after it pressed, the others folding.
fn going_looks(scene: &Scene, change: Change, picked: Option<usize>, t: f32, tm: &Timing) -> Vec<Look> {
    let count = scene.entries.len();
    (0..count)
        .map(|i| {
            let pose = motion::going_out(change, i, count, picked, t, tm);
            let lit = scene.lit.get(i).copied().unwrap_or(0.0) * (1.0 - motion::smooth(t / 0.1));
            let press = if picked == Some(i) { motion::press(t) } else { 0.0 };
            Look { hot: lit, press, sink: press * tm.travel, alpha: pose.alpha, grow: pose.grow, swing: pose.swing, ..Look::REST }
        })
        .collect()
}

/// The plates of a scene, those that are lit last (they have risen over their neighbours).
fn draw_plates(painter: &egui::Painter, ctx: &egui::Context, sp: &Space, scene: &Scene, looks: &[Look]) {
    let count = scene.entries.len();
    let hot = |i: usize| looks.get(i).is_some_and(|l| l.hot > 0.02);
    for i in (0..count).filter(|&i| !hot(i)).chain((0..count).filter(|&i| hot(i))) {
        let look = looks.get(i).copied().unwrap_or(Look::REST);
        paint::wedge(painter, ctx, sp, i, count, &scene.entries[i], scene.pics.get(i).copied().flatten(), &look, scene.depth);
    }
}

/// The lamps of the rim over the plates that are lit: where each reaches round the ring and how
/// strongly.
fn lamps_of(lit: &[f32], count: usize) -> Vec<(f32, f32, f32)> {
    let wedge = std::f32::consts::TAU / count.max(1) as f32;
    lit.iter()
        .enumerate()
        .filter(|&(_, &k)| k > 0.01)
        .map(|(i, &k)| {
            let a = radial::angle(i, count);
            if count == 1 { (0.0, std::f32::consts::TAU, k.clamp(0.0, 1.0)) } else { (a - wedge / 2.0 + 0.02, a + wedge / 2.0 - 0.02, k.clamp(0.0, 1.0)) }
        })
        .collect()
}

/// How lit `count` plates are with only `i` lit all the way.
fn one_lit(i: usize, count: usize) -> Vec<f32> {
    (0..count).map(|k| if k == i { 1.0 } else { 0.0 }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn produce(i: usize) -> Entry {
        Entry::new(radial::Id::Produce(i), crate::hud_icons::Glyph::Build, format!("unit {i}"), "")
    }

    fn scene_of(entries: Vec<Entry>) -> Scene {
        let n = entries.len();
        Scene { centre: pos2(0.0, 0.0), scale: 1.0, depth: 0, entries, pics: vec![None; n], lit: vec![0.0; n], hub: HubData { picture: None, name: String::new(), health: None, back: None, page: None } }
    }

    /// No plates, no looks (a ring that has nothing to choose from must not index into nothing), and one
    /// look to each plate otherwise.
    #[test]
    fn every_plate_has_a_look_and_a_ring_of_none_has_none() {
        let mut anim = Anim::default();
        anim.tick(0.0);
        anim.ring(RingKey { depth: 1, menu: radial::Menu::Main }, &[], 0, 0.0);
        assert!(plate_looks(&[], &anim, 0.1, Change::Open, 0.1, &Timing::FULL).is_empty());
        let entries: Vec<Entry> = (0..5).map(produce).collect();
        let mut anim = Anim::default();
        anim.tick(0.0);
        anim.ring(RingKey { depth: 1, menu: radial::Menu::Main }, &[], entries.len(), 0.0);
        assert_eq!(plate_looks(&entries, &anim, 0.1, Change::Open, 0.1, &Timing::FULL).len(), 5);
        assert!(going_looks(&scene_of(Vec::new()), Change::Swap, None, 0.05, &Timing::FULL).is_empty());
    }

    /// The press sinks its plate, shakes a refused one and pops a queued unit's count: movement, and none
    /// of it with reduced motion, which leaves the flash of the press (a flash, which reduced flashes
    /// has its own say over) and gives a refusal a tint of the plate in the place of the shake.
    #[test]
    fn the_press_the_shake_and_the_pop_are_movement_and_reduced_motion_leaves_them_out() {
        let entries = vec![produce(0), produce(1), Entry::new(radial::Id::Info, crate::hud_icons::Glyph::Info, "info", "")];
        let mut anim = Anim::default();
        anim.tick(0.0);
        anim.ring(RingKey { depth: 1, menu: radial::Menu::Main }, &[], entries.len(), 0.0);
        anim.press(0, 1.0);
        anim.refuse(2, 1.0);
        let at = 1.03;
        let full = plate_looks(&entries, &anim, at, Change::Open, 2.0, &Timing::FULL);
        assert!(full[0].press > 0.5 && full[0].sink > 0.5 && full[0].pop > 0.5, "pressed, sunk and popped: {:?}", full[0]);
        assert!(full[1].press == 0.0 && full[1].sink == 0.0 && full[1].pop == 0.0, "its neighbour is not");
        assert!(full[2].nudge.abs() > 0.1 && full[2].warn > 0.5, "a refused plate is shaken and tinted: {:?}", full[2]);
        assert!(full[2].pop == 0.0, "(a count pops only on a unit plate)");
        let reduced = plate_looks(&entries, &anim, at, Change::Open, 2.0, &Timing::REDUCED);
        assert!(reduced[0].press > 0.5, "the flash is still there for reduced flashes to leave out: {:?}", reduced[0]);
        assert!(reduced[0].sink == 0.0 && reduced[0].pop == 0.0, "but nothing sinks or swells: {:?}", reduced[0]);
        assert!(reduced[2].nudge == 0.0 && reduced[2].warn > 0.5, "nothing shakes, and the refusal is a tint: {:?}", reduced[2]);
        // Later, all of it is gone.
        let later = plate_looks(&entries, &anim, at + 1.0, Change::Open, 3.0, &Timing::FULL);
        assert!(later.iter().all(|l| l.press < 1e-3 && l.warn == 0.0 && l.nudge == 0.0 && l.pop < 1e-3), "{later:?}");
        // The ring that is going: the plate that opened the ring after it, pressed, sinks only with motion.
        let scene = scene_of(entries);
        let going = going_looks(&scene, Change::Into { from: Some(1), of: 3 }, Some(1), 0.04, &Timing::FULL);
        assert!(going[1].sink > 0.5 && going[0].sink == 0.0);
        let going = going_looks(&scene, Change::Into { from: Some(1), of: 3 }, Some(1), 0.04, &Timing::REDUCED);
        assert!(going[1].press > 0.5 && going[1].sink == 0.0, "{:?}", going[1]);
    }

    /// The rim's lamp covers the plate that is lit, as wide as that plate, and only that plate.
    #[test]
    fn the_rim_is_lit_over_the_plate_that_is_lit() {
        for count in [1usize, 2, 5, 9] {
            for i in 0..count {
                let lamps = lamps_of(&one_lit(i, count), count);
                assert_eq!(lamps.len(), 1, "{i} of {count}");
                let (lo, hi, k) = lamps[0];
                assert!((k - 1.0).abs() < 1e-6);
                if count == 1 {
                    assert!((hi - lo - std::f32::consts::TAU).abs() < 1e-5);
                } else {
                    let a = radial::angle(i, count);
                    assert!(lo < a && a < hi && hi - lo < std::f32::consts::TAU / count as f32 + 1e-5, "{i} of {count}");
                    assert!(((lo + hi) / 2.0 - a).abs() < 1e-5, "centred on the plate");
                }
            }
        }
        // Nothing lit: no lamp; two half lit: two lamps of those strengths, a plate not quite dark too.
        assert!(lamps_of(&[0.0, 0.0, 0.0], 3).is_empty());
        let two = lamps_of(&[0.4, 0.0, 1.3, 0.005], 4);
        assert_eq!(two.len(), 2);
        assert!((two[0].2 - 0.4).abs() < 1e-6 && (two[1].2 - 1.0).abs() < 1e-6, "strength is held to full: {two:?}");
    }
}
