//! The cockpit round a Perspective view (see `perspective.rs`): the frame the unit's crew
//! looks out through (a tank's hatch, a walker's visor, the Commander's helmet, a ship's
//! bridge, a periscope, a canopy), what is written on its glass over the world, and its
//! instruments. Everything is painted over the picture each frame; nothing here is input (the
//! unit goes on with its orders).
//!
//! Every fact is shown in one place. The glass, the window onto the battle, carries only what the
//! eyes need while looking out and what is anchored in the world (`sight`: the weapons' readiness
//! round the reticle, the target bracketed where it stands, threats out of the window, one line of
//! messages; each kind's own reticle and motion cue in `air`, `armour` and `mech`; damage on the
//! glass itself). The unit's state is on the console's screens (`draw_instruments`), or, where no
//! console is in view, on panels along the picture's foot (`Layout`): never both.
//!
//! The look is TA's own: gunmetal, bolts, amber lamps and green phosphor, not a hologram.
//!
//! What the displays read (`data`: the unit's hull, rank, speed, heading and attitude, the weather
//! and the ground), the kit they are drawn with (`kit`) and the pages they show (`pages`, the
//! environment in `environment`) are in the folder beside this file.

mod air;
mod armour;
mod data;
mod environment;
mod kit;
mod mech;
mod pages;
mod sight;

use self::data::{Readouts, metres};
use self::kit::{Pen, clock};
use self::sight::Window;

use egui::{Align2, Color32, FontId, Id, LayerId, Mesh, Order, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};
use glam::{Vec3, Vec4Swizzles};
use ta_render::Camera;
use ta_sim::{Fx, Locomotion, Seen};
use ta_ui::i18n::{n, t, tf};

use crate::game::Game;
use crate::perspective::{Cockpit, Perspective, WeaponState, ZOOMS};
use crate::theme::{AMBER, AMBER_BRIGHT, AMBER_DIM, GREEN, RED, STEEL, STEEL_DIM, TEXT, mono};

/// The phosphor of the cockpit's screens.
pub(crate) const PHOSPHOR: Color32 = Color32::from_rgb(150, 236, 118);
pub(crate) const PHOSPHOR_DIM: Color32 = Color32::from_rgb(58, 104, 46);
const SCREEN: Color32 = Color32::from_rgb(8, 16, 9);
/// The cockpit's metal: shadow, body, lit edge.
const METAL_DEEP: Color32 = Color32::from_rgb(12, 12, 11);
const METAL: Color32 = Color32::from_rgb(40, 39, 36);
const METAL_LIT: Color32 = Color32::from_rgb(92, 88, 79);
const METAL_EDGE: Color32 = Color32::from_rgb(140, 134, 118);

/// Where things are on the screen: the picture in points, and world points projected there.
struct View {
    rect: Rect,
    vp: glam::Mat4,
    ppp: f32,
    viewport: glam::Vec2,
}

impl View {
    /// A world point on the screen (points), unless it is behind the eye.
    fn project(&self, p: Vec3) -> Option<Pos2> {
        let c = self.vp * p.extend(1.0);
        if c.w <= 0.05 {
            return None;
        }
        let ndc = c.xy() / c.w;
        let px = glam::Vec2::new((ndc.x + 1.0) * 0.5 * self.viewport.x, (1.0 - ndc.y) * 0.5 * self.viewport.y);
        Some(pos2(px.x / self.ppp, px.y / self.ppp))
    }

    /// The way a world point lies from the picture's middle on the screen (y down; its length
    /// means nothing), even off the picture or behind the eye: there, the way to turn to face it
    /// (straight behind, down).
    fn toward(&self, p: Vec3) -> Vec2 {
        let c = self.vp * p.extend(1.0);
        // (The clip coordinates before the divide keep their way on both sides of the eye.)
        let d = vec2(c.x * self.viewport.x, -c.y * self.viewport.y);
        if d.length() < 1e-3 { vec2(0.0, 1.0) } else { d }
    }
}

/// The Minimalist First-Person View: the unit's hull, the sight (not from behind the unit: the eye
/// is not in it there) and a radar, coming on and going as the cockpit's glass does (`on`).
fn draw_minimalist(ctx: &egui::Context, health: f32, p: &Perspective, game: &Game, camera: &Camera, now: f64, on: f32) {
    let mut painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("minimalist_first_person")));
    painter.set_opacity(on);
    let rect = ctx.content_rect();
    // (Lettered in proportion to the picture, as the sight is: a pixel of a 1080-line picture,
    // not much smaller than a point.)
    let u = lettering(rect);
    let bar = minimalist_health_rect(rect, u);
    let health_pen = Pen::new(&painter, u, true, 1.0);
    let color = crate::palette::health_color(health);
    health_pen.text(bar.left_top() - vec2(0.0, 3.0 * u), Align2::LEFT_BOTTOM, t("HULL"), 11.0, PHOSPHOR);
    health_pen.text(bar.right_top() - vec2(0.0, 3.0 * u), Align2::RIGHT_BOTTOM, format!("{}%", (health * 100.0).round() as i32), 14.0, color);
    health_pen.bar(bar, 10, health.clamp(0.0, 1.0), color, PHOSPHOR_DIM.gamma_multiply(0.45));
    let view = View { rect, vp: camera.view_proj(), ppp: ctx.pixels_per_point(), viewport: camera.viewport };
    let pen = Pen::new(&painter, rect.height() / 1080.0, true, 1.0);
    let mut sight = Vec::new();
    if let Some(rd) = Readouts::of(p, game) {
        health_pen.text(bar.center_bottom() + vec2(0.0, 4.0 * u), Align2::CENTER_TOP, format!("{} / {}", rd.hp, rd.hp_max), 11.0, PHOSPHOR);
        if !p.chase {
            sight = armour::sight(&pen, &view, &Window { rect, round: false }, p, game, &rd, now);
        }
    }
    if p.controlling {
        sight.push(control_tag(&painter, bar.center_bottom() + vec2(0.0, 22.0 * u), u, 1.0));
    }
    let radar = minimalist_radar_rect(rect);
    painter.circle_filled(radar.center(), radar.width() * 0.5, Color32::from_black_alpha(150));
    glass_radar(&painter, radar, p, game, now);
    // (Over the radar, clear of the sight and the unit's picture beside it; else under the hull's
    // bar and the tag that says the unit is driven.)
    hints(&painter, rect, p, (now - p.hud.entered) as f32, game, now, KeysRoom { above: Some(radar.top() - 12.0 * u), high: bar.bottom() + 60.0 * u, avoid: &sight, across: rect, alpha: 1.0 });
}

/// The unit of what is lettered over the picture, outside the sight (the keys, the heading, the
/// link's and the lost signal's words): a pixel of a 1080-line picture, as the sight's, but not
/// less than three quarters of a point (smaller, they could not be read).
fn lettering(rect: Rect) -> f32 {
    (rect.height() / 1080.0).max(0.75)
}

/// The tag that says the player is driving the unit (E): on a plate like the keys', its top
/// centred on `at`, `scale` times its size at 1080 lines. Returns the room it takes.
fn control_tag(painter: &Painter, at: Pos2, scale: f32, alpha: f32) -> Rect {
    let galley = painter.layout_no_wrap(t("IN CONTROL").to_string(), mono(14.0 * scale), AMBER_BRIGHT);
    let pad = vec2(9.0, 4.0) * scale;
    let chip = Rect::from_min_size(at - vec2(galley.size().x / 2.0 + pad.x, 0.0), galley.size() + 2.0 * pad);
    painter.rect_filled(chip, 3.0 * scale, Color32::from_black_alpha((165.0 * alpha) as u8));
    painter.rect_stroke(chip, 3.0 * scale, egui::Stroke::new(scale.max(1.0), fade(AMBER, 0.55 * alpha)), egui::StrokeKind::Inside);
    painter.galley(chip.min + pad, galley, fade(AMBER_BRIGHT, alpha));
    chip
}

fn minimalist_health_rect(rect: Rect, u: f32) -> Rect {
    let width = (rect.width() - 32.0).clamp(0.0, 480.0 * u);
    Rect::from_center_size(pos2(rect.center().x, rect.top() + 35.0 * u), vec2(width, 10.0 * u))
}

fn minimalist_radar_rect(rect: Rect) -> Rect {
    let size = (rect.height() * 0.25).clamp(80.0, 220.0).min((rect.width() - 32.0).max(0.0));
    Rect::from_center_size(pos2(rect.center().x, rect.bottom() - size * 0.5 - 12.0), Vec2::splat(size))
}

/// The side's colour on the cockpit's trim.
fn accent(side: &str) -> Color32 {
    if side.eq_ignore_ascii_case("ARM") {
        Color32::from_rgb(92, 146, 214)
    } else if side.eq_ignore_ascii_case("CORE") {
        Color32::from_rgb(206, 72, 52)
    } else {
        AMBER
    }
}

fn fade(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

/// A colour the HUD says what a unit is doing in (`hud::activity`), as the cockpit's screens
/// show it: the plain ones in phosphor.
fn on_screen(c: Color32) -> Color32 {
    if c == TEXT {
        PHOSPHOR
    } else if c == STEEL || c == STEEL_DIM {
        fade(PHOSPHOR, 0.55)
    } else {
        c
    }
}

/// The cockpit round the view, painted over the picture. `on_glass`: a modelled cockpit's
/// instruments are pictures on its glass (`draw_instruments`), not painted here. `portrait`: the
/// unit drawn from its model, so many points across, once it is drawn (the HUD's portraits).
#[allow(clippy::too_many_arguments)]
pub fn draw(p: &Perspective, ctx: &egui::Context, game: &Game, camera: &Camera, pics: &mut rustc_hash::FxHashMap<usize, Option<egui::TextureHandle>>, vfs: &ta_vfs::Vfs, now: f64, on_glass: bool, portrait: &mut dyn FnMut(f32) -> Option<egui::TextureId>) {
    let rect = ctx.content_rect();
    let top = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("perspective_top")));
    // How far the cockpit has come on (the dive in), and gone as the view leaves (Escape: in a
    // moment, as the camera starts its climb out).
    let since = (now - p.hud.entered) as f32;
    let out = leaving_for(p, now);
    let staying = |over: f32| out.map_or(1.0, |s| 1.0 - (s / over).clamp(0.0, 1.0));
    let on = ((since - 0.55) / 0.5).clamp(0.0, 1.0) * staying(HUD_OUT);
    // Lost, the picture's signal goes (in the Minimalist First-Person View too), fading as the
    // camera climbs back out.
    if let Some(lost) = p.lost {
        lost_signal(&top, rect, (now - lost) as f32, if p.reduced_flashes || p.reduced_motion { 0.0 } else { now }, staying(CLIMB_OUT));
        return;
    }
    if on <= 0.0 {
        return;
    }
    if p.minimalist {
        if let Some(unit) = game.sim.unit(p.unit) {
            // (The unit's own hull: an enhancement raises it.)
            let maximum = unit.max_health(game.sim.unit_type(unit)).max(Fx::ONE);
            draw_minimalist(ctx, (unit.health / maximum).to_f32().clamp(0.0, 1.0), p, game, camera, now, on);
        }
        return;
    }
    let view = View { rect, vp: camera.view_proj(), ppp: ctx.pixels_per_point(), viewport: camera.viewport };
    let back = ctx.layer_painter(LayerId::new(Order::Background, Id::new("perspective_back")));
    let front = ctx.layer_painter(LayerId::new(Order::Middle, Id::new("perspective_front")));
    let Some(u) = game.sim.unit(p.unit) else { return };
    let side = game.sim.players.get(u.owner as usize).map_or("", |pl| pl.side.as_str()).to_string();
    let trim = accent(&side);
    let zoomed = p.zoom > 0 && !p.chase;
    // A modelled cockpit (rendered with the scene) replaces the painted frame, which is never
    // drawn for it (painted while the model was on its way, it showed a second cockpit, see-
    // through, for half a second of every entry); the instruments are drawn on its screens.
    let has_model = p.model.is_some();
    let modelled = has_model && p.seat > 0.02;
    // In it, rather than behind it or looking through the magnified sight.
    let seated_view = !p.chase && !zoomed;
    // Where the console's screens are in view (seated in a modelled cockpit), the unit's state
    // is on them; where none are (the periscope, behind the unit, the magnified sight), on
    // panels along the picture's foot. Never both.
    let console = has_model && seated_view && (on_glass || !modelled);
    // The window the crew looks out through: what is written over the world keeps inside it.
    let window = window(&view, p, modelled);
    // The glass: the night sight, the vignette, cracks; and in the rain, drops running down it.
    glass(&back, rect, &window, p, on, now);
    // TODO(lead): rain_on_glass
    let rain = game.effects.rain(crate::scene::game_seconds(game.sim.tick, game.alpha()) as f32);
    if rain > 0.02 && !p.chase && p.cockpit != Cockpit::Periscope && !modelled {
        raindrops(&back, rect, p, rain * on, now);
    }
    // (A painted frame, the periscope's, stays in place once it is in, through the magnified
    // field too: the seat goes back out as the sight magnifies, and the periscope's field and
    // its cross-wires went with it. Got in magnified, it comes in as the dive ends, as the seat
    // would have.)
    let dived = if p.reduced_motion { 1.0 } else { ((since - FRAME_IN.0) / FRAME_IN.1).clamp(0.0, 1.0) };
    let seat = if zoomed { p.seat.max(dived) } else { p.seat };
    let framed = !has_model && !p.chase && !p.leaving && (p.hud.framed.get() || seat >= 1.0);
    p.hud.framed.set(framed);
    let seat = if framed { 1.0 } else { seat };
    if !p.chase {
        if zoomed && p.cockpit != Cockpit::Periscope {
            magnified_sight(&front, rect, on);
        } else if !has_model {
            frame(&front, rect, p, seat, trim, now);
        }
    }
    // What the displays read, once this frame.
    let readouts = Readouts::of(p, game);
    // Over the world, inside the window (under a modelled cockpit's window sill they would land
    // on the instruments).
    let world = match p.sill_y.filter(|_| modelled && seated_view) {
        Some(y) => back.with_clip_rect(Rect::from_min_max(rect.min, pos2(rect.right(), (y / view.ppp).clamp(rect.top() + rect.height() * 0.4, rect.bottom())))),
        None => back.clone(),
    };
    let pen = Pen::new(&world, rect.height() / 1080.0, true, on);
    // The heading, once: an aircraft's on its helmet, a cockpit's with a heading screen of its
    // own on that, the others' (and from behind or through the magnified sight) a tape over the
    // view; where what is written along the top of the view ends.
    // (The model's own heading screen, whether or not it is on the picture this frame: as the view
    // leaves, a tape came up over it as it went.)
    let heading_screen = seated_view && on_glass && p.model.as_ref().is_some_and(|m| m.screens.iter().any(|s| s.name == "compass"));
    // (From behind the unit or through the magnified sight, what stands in the console's place
    // comes in as the modelled cockpit goes back out past the eye, its heading screen with it.)
    let behind_on = if has_model && !seated_view { on * (1.0 - crate::perspective::ease(p.seat)) } else { on };
    let mut heading_end = None;
    if let Some(rd) = readouts.as_ref().filter(|_| p.cockpit.flies()) {
        heading_end = Some(air::heading_tape(&front, &view, p, game, rd, behind_on, compass_top(&view, p)));
    } else if !heading_screen {
        heading_end = compass(&front, &view, p, game, behind_on);
    }
    // The world as the crew's eyes need it: the units in view marked, the crew's own sight (each
    // kind's reticle, the weapons' readiness round it and its motion cue), the target bracketed,
    // threats out of the window, one line of messages.
    sight::marks(&pen, &view, &window, p, game, now);
    // (The room each crew's own marks take: the target's line keeps off it.)
    let mut room = Vec::new();
    if let (true, Some(rd), Some(end)) = (p.cockpit.flies(), &readouts, heading_end) {
        room.extend(air::symbology(&world, &view, p, rd, on, end, now));
    }
    // A vehicle's, ship's, gun house's or periscope's sight; from behind the unit the eye is not
    // in it.
    if let Some(rd) = readouts.as_ref().filter(|_| !p.chase && !p.cockpit.flies() && !walks(p.cockpit)) {
        room.extend(armour::sight(&pen, &view, &window, p, game, rd, now));
    }
    // A kbot's: a mech's.
    if let Some(rd) = readouts.as_ref().filter(|_| !p.chase && walks(p.cockpit)) {
        room.extend(mech::hud(&pen, &view, p, rd, now));
    }
    // The magnification, at the upper left of the sight (the range finder stands at its right).
    if p.zoom > 0 && !p.chase {
        let at = view.rect.center() + vec2(-pen.s(118.0), -pen.s(64.0));
        room.push(projected(&world, at, Align2::RIGHT_CENTER, format!("×{:.1}", ZOOMS[p.zoom]), mono(pen.s(15.0)), fade(if p.cockpit.flies() { PHOSPHOR } else { AMBER_BRIGHT }, on * 0.85)));
    }
    let target = sight::bracket(&pen, &view, &window, p, game, now, &room);
    if let Some((mark, since)) = &p.hud.destroyed {
        sight::destroyed(&pen, &view, mark, *since, now);
    }
    let edge = Pen::new(&top, rect.height() / 1080.0, true, on);
    sight::hit_flash(&edge, &window, p, now);
    let chevrons = sight::threats(&edge, &view, &window, p, game, now);
    sparks(&top.with_clip_rect(window.rect), &window, p, on, now);
    boot(&top, rect, p, since, &side, game);
    // The tag that says the unit is driven, at the top of the window under the heading.
    let tag = p.controlling.then(|| {
        let below = heading_end.map_or(f32::MIN, |end| end + pen.s(8.0)).max(window.rect.top() + pen.s(12.0));
        control_tag(&top, pos2(window.rect.center().x, below), pen.s(1.0), on)
    });
    // (The keys keep off the sight; else they stand under the top of the window and what is
    // written along it.)
    let high = heading_end.map_or(f32::MIN, |end| end + pen.s(14.0)).max(tag.map_or(f32::MIN, |t| t.bottom() + pen.s(10.0))).max(window.rect.top() + pen.s(14.0));
    // (Off the target's line too.)
    room.extend(target);
    let keys = hints(&top, rect, p, since, game, now, KeysRoom { above: None, high, avoid: &room, across: window.rect, alpha: on });
    // (The message keeps off everything else written low in the window.)
    room.extend(chevrons);
    room.extend(tag);
    sight::message(&edge, &window, p, now, &room, keys);
    // The unit's state: on the console's screens, else on panels along the foot of the picture,
    // with the environment in a strip at its top left.
    let Some(rd) = readouts.as_ref().filter(|_| !console) else { return };
    // Seated in a modelled cockpit whose instruments are not pictures on its glass, the panels
    // lie on its screens as they are this frame. Else along the foot of the picture: from behind
    // the unit or through the magnified sight, coming in as the model goes back out past the eye
    // (none laid on a screen sliding by: it was of no size, or thousands of points across); a
    // painted dashboard's, with its frame.
    let on_screens = seated_view && modelled && !on_glass;
    let on = if has_model && !seated_view {
        behind_on
    } else if seated_view && !has_model {
        on.min(crate::perspective::ease((seat - 0.4) / 0.6))
    } else {
        on
    };
    if on <= 0.0 {
        return;
    }
    let mut layout = Layout::for_cockpit(p.cockpit, rect, !seated_view);
    if !on_screens {
        environment::corner(&Pen::new(&front, rect.height() / 1080.0, true, on), foot_band(rect).left_top() + vec2(pen.s(16.0), pen.s(14.0)), rd);
    }
    // (A screen not in the picture whole, or of no size, has nothing on it this frame.)
    let quad = |name: &str| {
        p.screens.iter().find(|s| s.0 == name).map(|(_, c)| snapped(Rect::from_points(&c.map(|q| pos2(q.x / view.ppp, q.y / view.ppp))))).filter(|r| on_picture(*r, rect)).unwrap_or(Rect::NOTHING)
    };
    if on_screens {
        layout = Layout { speed: quad("speed"), radar: quad("radar"), hull: quad("hull"), status: quad("weapons") };
    }
    // Painted panels are lettered in proportion to the hull's slot, and brought to one height
    // (the taller of what each needs), standing on the slots' foot beside the dials; the weapons'
    // rows may stand taller still, as many as a mod's unit carries (up to three tenths of the
    // picture).
    let (need, tall, scale) = panel_heights(layout.hull.height(), rd.weapons.len(), rect.height());
    if !on_screens {
        layout.hull = Rect::from_min_max(pos2(layout.hull.left(), layout.hull.bottom() - need), layout.hull.max);
        layout.status = Rect::from_min_max(pos2(layout.status.left(), layout.status.bottom() - tall), layout.status.max);
    }
    // Aircraft: an attitude ball where the speed would be, while the helmet shows no pitch ladder
    // (from behind the aircraft); the Commander's economy.
    if p.cockpit.flies() && p.chase {
        attitude(&front, layout.speed, p, on, on_screens);
    }
    if on_screens {
        economy(&front, quad("economy"), game, u.owner, on);
    }
    speed_gauge(&front, layout.speed, p, rd, on, on_screens);
    status_panel(&front, layout.status, p, game, rd, on, now, on_screens, scale);
    radar(&front, layout.radar, p, game, on, on_screens);
    integrity(&front, layout.hull, p, game, rd, on, now, portrait, pics, vfs, ctx, on_screens);
}

/// How long what is written over the view takes to go once the view is leaving, and how long the
/// camera's climb back out takes (as `perspective`'s own): a lost signal's static fades over it.
const HUD_OUT: f32 = 0.25;
const CLIMB_OUT: f32 = 0.85;
/// When the cockpit comes in round the eye after getting in, and how long it takes (seconds, as
/// `perspective`'s seat does: from its share of the dive on).
const FRAME_IN: (f32, f32) = (0.66, 0.5);

/// How long the view has been leaving (seconds), if it is: from the first frame it was seen to.
fn leaving_for(p: &Perspective, now: f64) -> Option<f32> {
    if !p.leaving {
        p.hud.left_at.set(None);
        return None;
    }
    let since = p.hud.left_at.get().unwrap_or(now);
    p.hud.left_at.set(Some(since));
    Some((now - since).max(0.0) as f32)
}

/// Whether a screen of a modelled cockpit, as it is on the picture this frame, can carry a panel:
/// all of it a number, of some size, and no larger than the picture.
fn on_picture(r: Rect, picture: Rect) -> bool {
    r.is_finite() && r.width() >= 8.0 && r.height() >= 8.0 && r.width() <= picture.width() && r.height() <= picture.height()
}

/// The least and the most (of a point) a panel's or a page's unit is: smaller it could not be
/// read, nor is there room for it (a screen sliding past the eye), and it is not drawn; larger,
/// it is held there.
const LEAST: f32 = 0.3;

/// The unit a page laid out `size` units across takes to fill `room` points, held to `most`; none
/// where it would be under [`LEAST`], or where the room is none at all (of no size, turned inside
/// out, or not a number).
fn fill_scale(room: Vec2, size: Vec2, most: f32) -> Option<f32> {
    // (Each way a number: `min` would pass over one that is none.)
    let (across, down) = (room.x / size.x, room.y / size.y);
    let k = across.min(down);
    (across.is_finite() && down.is_finite() && k >= LEAST).then(|| k.min(most.max(LEAST)))
}

/// Where the drive page stands on a screen `r` (`bare`: a modelled cockpit's, else a painted
/// dial's) and how large: its middle, its face's radius and its unit; none on a screen of no size,
/// or not a number.
fn drive_on(r: Rect, bare: bool, most: f32) -> Option<(Pos2, f32, f32)> {
    if !r.is_finite() {
        return None;
    }
    let (c, rad) = if bare { (r.center(), r.width().min(r.height()) * 0.48) } else { dial(r) };
    fill_scale(Vec2::splat(rad), Vec2::splat(pages::DRIVE_RADIUS), most).map(|k| (c, rad, k))
}

/// Where the unit's page stands on a screen `r` (`bare`: a modelled cockpit's, else a painted
/// panel's inside its bezel) and how large: its box and its unit, as big as the screen allows;
/// none on a screen of no size, or not a number.
fn unit_page_on(r: Rect, bare: bool, most: f32) -> Option<(Rect, f32)> {
    if !r.is_finite() {
        return None;
    }
    let (w, h) = pages::UNIT_SIZE;
    let glass = if bare { r.shrink(r.height() * 0.04) } else { r.shrink(4.0) };
    fill_scale(glass.size() - Vec2::splat(4.0), vec2(w, h), most).map(|k| (Rect::from_center_size(glass.center(), vec2(w * k, h * k)), k))
}

/// The most a panel painted over the picture is scaled: four times its size at 1080 lines, in
/// proportion on a taller picture.
fn most_over(painter: &Painter) -> f32 {
    4.0 * (painter.ctx().content_rect().height() / 1080.0).max(1.0)
}

/// A modelled cockpit's instruments, each drawn into its screen's region of the atlas
/// (`instruments.rs`), which its glass shows lit. Each runs a short self test as the cockpit
/// powers up.
#[allow(clippy::too_many_arguments)]
pub fn draw_instruments(p: &Perspective, ctx: &egui::Context, game: &Game, regions: &[(String, Rect)], pics: &mut rustc_hash::FxHashMap<usize, Option<egui::TextureHandle>>, portraits: &mut crate::instruments::Portraits, vfs: &ta_vfs::Vfs, now: f64) {
    let painter = ctx.layer_painter(LayerId::background());
    let since = (now - p.hud.entered) as f32;
    let owner = game.sim.unit(p.unit).map_or(game.local_player, |u| u.owner);
    // What the displays read, once this frame.
    let readouts = Readouts::of(p, game);
    // (Once its lamps are on, one after another.)
    let test_from = crate::perspective::LAMPS_AT;
    for (i, (name, r)) in regions.iter().enumerate() {
        let painter = painter.with_clip_rect(*r);
        let ready_at = test_from + 0.3 + 0.15 * i as f32;
        if since < ready_at {
            glass_self_test(&painter, *r, screen_name(name, p.cockpit.flies()), ((since - test_from) / (ready_at - test_from)).clamp(0.0, 1.0), p.reduced_flashes);
            continue;
        }
        match (name.as_str(), readouts.as_ref()) {
            ("pcd", Some(rd)) => glass_pcd(&painter, *r, p, game, rd, now, pics, portraits, vfs, ctx),
            ("speed", _) if p.cockpit.flies() => attitude(&painter, *r, p, 1.0, true),
            ("speed", Some(rd)) => glass_drive(&painter, *r, rd),
            ("radar", _) => glass_radar(&painter, *r, p, game, now),
            ("hull", Some(rd)) => glass_unit(&painter, *r, p, game, rd, now, pics, portraits, vfs, ctx),
            ("weapons", Some(rd)) => glass_status(&painter, *r, p, game, rd, now),
            ("economy", _) => glass_economy(&painter, *r, game, owner),
            ("compass", _) => glass_compass(&painter, *r, p, game),
            _ => {}
        }
    }
}

/// What a modelled cockpit's screen is called as it powers up, by its region's name (none for
/// one not known).
fn screen_name(region: &str, flies: bool) -> Option<&'static str> {
    Some(match region {
        // (An aircraft's shows its attitude.)
        "speed" if flies => n("ATTITUDE"),
        "speed" => n("SPEED"),
        "radar" => n("RADAR"),
        "hull" => n("HULL"),
        "weapons" => n("WEAPONS"),
        "economy" => n("ECONOMY"),
        "compass" => n("HEADING"),
        "pcd" => n("DISPLAY"),
        _ => return None,
    })
}

// ---- The instruments on a modelled cockpit's glass ----------------------------------------------
//
// Sizes are millimetres of glass (`mm`): the atlas has a fixed number of pixels to a metre, so a
// letter is as big on every screen, as on real displays. The bezels cover the glass's outer 6 mm.

/// Millimetres of glass in atlas pixels.
fn mm(v: f32) -> f32 {
    v * ta_render::cockpit::ATLAS_DENSITY / 1000.0
}

/// A screen's glass inside its bezel (whose bars, standing off the glass, hide more of it seen
/// from the side).
fn glass_inside(r: Rect) -> Rect {
    r.shrink2(vec2(mm(12.0), mm(8.0)))
}

/// A bar of `segments` lit up to `frac`, LED style.
fn segment_bar(painter: &Painter, r: Rect, segments: usize, frac: f32, lit: Color32, unlit: Color32) {
    let gap = (r.width() / segments as f32 * 0.18).max(1.0);
    let w = (r.width() - gap * (segments - 1) as f32) / segments as f32;
    for k in 0..segments {
        let x = r.left() + k as f32 * (w + gap);
        let on = frac * segments as f32 > k as f32 + 0.02;
        painter.rect_filled(Rect::from_min_size(pos2(x, r.top()), vec2(w, r.height())), 0.0, if on { lit } else { unlit });
    }
}

/// Powering up: the screen's name, SELF TEST and a bar filling (flickering, unless flashes are
/// reduced: then steady).
fn glass_self_test(painter: &Painter, r: Rect, name: Option<&str>, progress: f32, steady: bool) {
    if progress <= 0.0 {
        return;
    }
    let c = r.center();
    let flicker = if !steady && (progress * 23.0).fract() < 0.15 { 0.4 } else { 1.0 };
    let col = fade(PHOSPHOR, flicker);
    let k = (r.height() / mm(60.0)).min(1.0);
    if let Some(name) = name {
        let font = mono(mm(9.0) * k);
        painter.text(c - vec2(0.0, mm(8.0) * k), Align2::CENTER_CENTER, kit::fit(painter, t(name), &font, r.width() - mm(4.0)), font, col);
    }
    painter.text(c + vec2(0.0, mm(3.0) * k), Align2::CENTER_CENTER, t("SELF TEST"), mono(mm(6.0) * k), fade(PHOSPHOR, flicker * 0.6));
    let bar = Rect::from_center_size(c + vec2(0.0, mm(12.0) * k), vec2(r.width().min(r.height() * 2.0) * 0.55, mm(3.0) * k));
    segment_bar(painter, bar, 12, progress, col, fade(PHOSPHOR_DIM, 0.4));
}

/// What the unit is doing, as its status page says it: the command deck's words, without naming
/// again what its guns are on (the sight brackets it, named, in the window).
fn doing(game: &Game, u: &ta_sim::Unit, p: &Perspective) -> (String, Color32) {
    let (text, colour) = crate::hud::activity(game, u);
    let Some(target) = p.target(game).and_then(|id| game.sim.unit(id)) else { return (text, colour) };
    // (Unless something else came first: under construction, paralysed, self-destructing.)
    if !text.contains(&crate::chat::unit_label(&game.sim, target.type_id)) {
        return (text, colour);
    }
    let on = Some(ta_sim::Target::Unit(target.id));
    let attacking = matches!(u.orders.front(), Some(ta_sim::Order::Attack(t) | ta_sim::Order::ManualFire(t)) if Some(*t) == on);
    let engaging = u.orders.is_empty() && u.build.is_none() && u.weapons.iter().find_map(|w| w.target) == on;
    if attacking {
        (t("Attacking").to_string(), colour)
    } else if engaging {
        (t("Engaging").to_string(), colour)
    } else {
        (text, colour)
    }
}

/// The unit's status: what it is doing across the top (its order, or what it builds or shoots
/// at) with a build's progress under it; each weapon by its number (the one beside its arc round
/// the sight's reticle), its state, its reload and its reach; and along the foot, the environment.
/// (What the guns are on is bracketed where it stands, in the window.)
fn glass_status(painter: &Painter, r: Rect, p: &Perspective, game: &Game, rd: &Readouts, now: f64) {
    let Some(u) = game.sim.unit(p.unit) else { return };
    let weapons = &rd.weapons;
    let g = glass_inside(r);
    // The environment along the foot, under a hairline.
    let line_h = (g.height() * 0.17).clamp(mm(7.0), mm(13.0));
    let foot = Rect::from_min_max(pos2(g.left(), g.bottom() - line_h), g.max);
    environment::status_line(&Pen::new(painter, mm(1.0), false, 1.0), foot, rd);
    painter.line_segment([pos2(g.left(), foot.top() - mm(1.5)), pos2(g.right(), foot.top() - mm(1.5))], Stroke::new(mm(0.35), fade(PHOSPHOR_DIM, 0.9)));
    let g = Rect::from_min_max(g.min, pos2(g.right(), foot.top() - mm(3.5)));
    // What it is doing, and (building, producing or repairing) how far along that is.
    let (doing, colour) = doing(game, u, p);
    let progress = u.build.and_then(|job| game.sim.unit(job.target)).map(|work| {
        let wt = game.sim.unit_type(work);
        let done = if work.is_complete() { (work.health / work.max_health(wt).max(Fx::ONE)).to_f32() } else { work.build_progress.to_f32() };
        done.clamp(0.0, 1.0)
    });
    if weapons.is_empty() {
        // Nothing to fire: what it is doing fills the screen, in the middle.
        let mut job = egui::text::LayoutJob::simple(doing.to_uppercase(), mono(mm(11.0)), on_screen(colour), g.width());
        job.halign = egui::Align::Center;
        let galley = painter.layout_job(job);
        let bar_h = if progress.is_some() { mm(10.0) } else { 0.0 };
        let top = g.center().y - (galley.size().y + bar_h) * 0.5;
        let below = top + galley.size().y;
        painter.galley(pos2(g.center().x, top), galley, on_screen(colour));
        if let Some(done) = progress {
            segment_bar(painter, Rect::from_min_size(pos2(g.left(), below + mm(4.0)), vec2(g.width(), mm(6.0))), 24, done, GREEN, fade(PHOSPHOR_DIM, 0.45));
        }
        return;
    }
    let head = mm(9.0);
    painter.text(g.min, Align2::LEFT_TOP, fitted(painter, doing.to_uppercase(), &mono(head), g.width()), mono(head), on_screen(colour));
    let mut top = g.top() + head * 1.45;
    if let Some(done) = progress {
        let bar = Rect::from_min_size(pos2(g.left(), top), vec2(g.width(), mm(4.0)));
        segment_bar(painter, bar, 24, done, GREEN, fade(PHOSPHOR_DIM, 0.45));
        top = bar.bottom() + mm(2.5);
    }
    painter.line_segment([pos2(g.left(), top), pos2(g.right(), top)], Stroke::new(mm(0.35), fade(PHOSPHOR_DIM, 0.9)));
    top += mm(3.0);
    // The weapons, a row each (those that read the same in one: a mod's units carry up to eight).
    let rows = Rect::from_min_max(pos2(g.left(), top), g.max);
    let n = weapons.len().max(1);
    let row_h = rows.height() / n as f32;
    let name_px = (row_h * if n == 1 { 0.3 } else { 0.36 }).min(mm(12.5));
    for (i, w) in weapons.iter().enumerate() {
        let y = rows.top() + i as f32 * row_h;
        let starved = w.starved;
        let fired = fired(p, w, now);
        let colour = if starved { RED } else if w.ready { GREEN } else { AMBER };
        let mid = y + name_px * 0.55;
        // Its number on a tag, lit while it is ready (and as it fires).
        let tag = Rect::from_center_size(pos2(g.left() + name_px * 0.42, mid), vec2(name_px * 0.84, name_px * 0.95));
        let lit = w.ready || fired || starved;
        painter.rect_filled(tag, mm(0.8), fade(colour, if lit { 0.95 } else { 0.4 }));
        painter.text(tag.center(), Align2::CENTER_CENTER, format!("{}", i + 1), mono(name_px * 0.8), Color32::from_rgb(10, 18, 8));
        // Its state in a word at the right; its name in what room is left.
        let state = if starved { t("NO ENERGY") } else if w.ready { t("READY") } else { t("LOADING") };
        let state_font = mono(name_px * 0.72);
        let state_w = painter.layout_no_wrap(state.to_string(), state_font.clone(), colour).size().x;
        painter.text(pos2(g.right(), mid), Align2::RIGHT_CENTER, state, state_font, colour);
        let name_x = tag.right() + name_px * 0.45;
        // (Narrowed a little to fit before it is cut short.)
        let (label, size) = shrunk_to(painter, w.name.to_uppercase(), name_px, g.right() - mm(3.0) - state_w - name_x, 0.72);
        painter.text(pos2(name_x, mid), Align2::LEFT_CENTER, label, mono(size), PHOSPHOR);
        // The reload, in segments.
        let bar = Rect::from_min_size(pos2(name_x, y + name_px * 1.3), vec2(g.right() - name_x, (row_h * 0.13).clamp(mm(3.2), mm(8.0))));
        segment_bar(painter, bar, 24, 1.0 - w.left, colour, fade(PHOSPHOR_DIM, 0.45));
        // Its reach and its reload (or the D-gun's energy), and a stockpile's count.
        let range = metres(w.range).round() as i32;
        let mut detail = if w.manual && w.cost > 0.0 {
            tf("RNG {range} M · {energy} E A SHOT", &[("range", &range), ("energy", &(w.cost.round() as i32))])
        } else {
            tf("RNG {range} M · RLD {seconds} S", &[("range", &range), ("seconds", &format!("{:.1}", w.reload))])
        };
        if w.stockpile {
            detail = format!("{detail} · {}", tf("STOCK {n}", &[("n", &rd.stockpile)]));
        }
        if bar.bottom() + mm(8.5) <= y + row_h {
            let font = mono(mm(6.4));
            painter.text(pos2(name_x, bar.bottom() + mm(1.8)), Align2::LEFT_TOP, kit::fit(painter, &detail, &font, g.right() - name_x), font, fade(PHOSPHOR, 0.65));
        }
    }
}

/// Whether a weapon has just fired: its tag lights for a moment (not with flashes reduced: a gun
/// firing fast blinked it).
fn fired(p: &Perspective, w: &WeaponState, now: f64) -> bool {
    !p.reduced_flashes && now - w.latest(&p.hud.last_shot) < 0.12
}

/// The glass panoramic display: one wide screen of three panes side by side, the aircraft's page,
/// the radar and the weapons.
#[allow(clippy::too_many_arguments)]
fn glass_pcd(painter: &Painter, r: Rect, p: &Perspective, game: &Game, rd: &Readouts, now: f64, pics: &mut rustc_hash::FxHashMap<usize, Option<egui::TextureHandle>>, portraits: &mut crate::instruments::Portraits, vfs: &ta_vfs::Vfs, ctx: &egui::Context) {
    let g = r.shrink(mm(3.5));
    let (panes, cuts) = pcd_panes(g, g.width() > mm(520.0), mm(1.5));
    for x in cuts {
        painter.line_segment([pos2(x, g.top() + mm(2.0)), pos2(x, g.bottom() - mm(2.0))], Stroke::new(mm(0.35), fade(PHOSPHOR_DIM, 0.9)));
    }
    // (The pages are laid out for a monitor's glass inside its bezel, which `glass_inside` takes
    // off: given a pane grown by as much, they fill it.)
    let grown = |pane: Rect| pane.expand2(vec2(mm(12.0), mm(8.0)));
    glass_unit(&painter.with_clip_rect(panes[0]), grown(panes[0]), p, game, rd, now, pics, portraits, vfs, ctx);
    glass_radar(&painter.with_clip_rect(panes[1]), panes[1], p, game, now);
    glass_status(&painter.with_clip_rect(panes[2]), grown(panes[2]), p, game, rd, now);
}

/// The three panes of the panoramic display's glass `g` (`wide`: the fighter's, a little broader
/// than the gunship's), each a `gap` clear of the hairlines between them, and where those are.
fn pcd_panes(g: Rect, wide: bool, gap: f32) -> ([Rect; 3], [f32; 2]) {
    let cuts = if wide { [0.32, 0.57] } else { [0.31, 0.53] };
    let x = cuts.map(|f| g.left() + g.width() * f);
    (
        [
            Rect::from_min_max(g.min, pos2(x[0] - gap, g.bottom())),
            Rect::from_min_max(pos2(x[0] + gap, g.top()), pos2(x[1] - gap, g.bottom())),
            Rect::from_min_max(pos2(x[1] + gap, g.top()), g.max),
        ],
        x,
    )
}

/// The unit's page: its picture (drawn from its model once the HUD has drawn it, TA's meanwhile),
/// name and kind, the hull and armour, its rank, kills and experience.
#[allow(clippy::too_many_arguments)]
fn glass_unit(painter: &Painter, r: Rect, p: &Perspective, game: &Game, rd: &Readouts, now: f64, pics: &mut rustc_hash::FxHashMap<usize, Option<egui::TextureHandle>>, portraits: &mut crate::instruments::Portraits, vfs: &ta_vfs::Vfs, ctx: &egui::Context) {
    let Some(u) = game.sim.unit(p.unit) else { return };
    let ty = game.sim.unit_type(u);
    let g = glass_inside(r);
    // (The page is laid out for a box of a set size, as big as the glass allows.)
    let (w, h) = pages::UNIT_SIZE;
    let Some(k) = fill_scale(g.size(), vec2(w, h), f32::MAX) else { return };
    let k = k * 0.93;
    let frame = Rect::from_center_size(g.center(), vec2(w * k, h * k));
    let alarm = p.hud.health < 0.35 && (p.reduced_flashes || (now * 3.0).fract() < 0.5);
    let tint = if alarm { Color32::from_rgb(255, 110, 90) } else { Color32::from_rgb(150, 236, 118) };
    let side = 58.0 * k;
    let portrait = crate::hud::portraits_wanted().then(|| crate::hud::portrait_key(game, p.type_id, side, u.owner)).and_then(|key| *portraits.entry(key).or_insert(None));
    let picture = match portrait {
        Some(id) => Some((id, fade(tint, 0.9))),
        None => pics
            .entry(p.type_id)
            .or_insert_with(|| {
                let name = ty.name.to_ascii_lowercase();
                let pcx = ta_formats::pcx::PcxImage::parse(&vfs.read_opt(&format!("unitpics/{name}.pcx"))?).ok()?;
                let img = egui::ColorImage::from_rgba_unmultiplied([pcx.width as usize, pcx.height as usize], &pcx.to_rgba());
                Some(ctx.load_texture(format!("cockpit-glass-pic-{name}"), img, egui::TextureOptions::LINEAR))
            })
            .clone()
            .map(|tex| (tex.id(), fade(tint, 0.7))),
    };
    let kill_flash = p.hud.kill_flash.is_some_and(|k| now - k < 1.2);
    let armour = walks(p.cockpit).then_some(rd.armour);
    pages::unit(&Pen::new(painter, k, false, 1.0), frame, rd, &pages::Face { picture, armour, alarm, kill_flash });
}

/// The radar: what the player's side sees round the unit, the view's way up, the window's field
/// as a fan, north marked, what the guns are on ringed, enemies firing at the unit pulsing (what
/// radar alone sees, a plain dim blip: not what it is, nor what it fires at).
fn glass_radar(painter: &Painter, r: Rect, p: &Perspective, game: &Game, now: f64) {
    let c = r.center();
    let rad = r.width().min(r.height()) * 0.5 - mm(2.5);
    let dim = fade(PHOSPHOR_DIM, 0.9);
    for k in 1..=2 {
        painter.circle_stroke(c, rad * k as f32 / 3.0, Stroke::new(mm(0.45), dim));
    }
    painter.circle_stroke(c, rad, Stroke::new(mm(0.6), fade(PHOSPHOR, 0.5)));
    painter.line_segment([pos2(c.x - rad, c.y), pos2(c.x + rad, c.y)], Stroke::new(mm(0.35), dim));
    painter.line_segment([pos2(c.x, c.y - rad), pos2(c.x, c.y + rad)], Stroke::new(mm(0.35), dim));
    // The window's field: what is in the fan is in sight (the picture's width, magnified).
    let half = view_across(p.fov / ZOOMS[p.zoom], p.aspect);
    let mut fan = Mesh::default();
    fan.colored_vertex(c, fade(PHOSPHOR, 0.16));
    for k in 0..=16 {
        let a = -half + 2.0 * half * k as f32 / 16.0;
        fan.colored_vertex(c + vec2(a.sin(), -a.cos()) * rad, fade(PHOSPHOR, 0.05));
        if k > 0 {
            fan.add_triangle(0, k, k + 1);
        }
    }
    painter.add(Shape::mesh(fan));
    for sd in [-1.0f32, 1.0] {
        painter.line_segment([c, c + vec2((sd * half).sin(), -(sd * half).cos()) * rad], Stroke::new(mm(0.35), fade(PHOSPHOR, 0.35)));
    }
    let look = p.forward.x.atan2(p.forward.z);
    let (s, co) = look.sin_cos();
    // Ticks round the rim every 30 degrees of bearing, and north.
    for k in 0..12 {
        let a = k as f32 * std::f32::consts::TAU / 12.0;
        let d = vec2(a.cos(), a.sin());
        painter.line_segment([c + d * rad, c + d * (rad - mm(if k % 3 == 0 { 3.5 } else { 2.0 }))], Stroke::new(mm(0.45), fade(PHOSPHOR, 0.6)));
    }
    let north = vec2(-s, co);
    painter.text(c + north * (rad - mm(8.5)), Align2::CENTER_CENTER, t("N"), mono(mm(7.5)), AMBER);
    // The sweep and its afterglow.
    sweep(painter, c, rad, p.hud.sweep, mm(0.7), 1.0);
    let Some(me) = game.sim.unit(p.unit) else { return };
    let here = Vec3::from_array(me.pos.to_f32());
    let range = 1400.0;
    let target = p.target(game);
    for u in game.sim.units() {
        if u.id == p.unit || u.dead || u.carried_by.is_some() || game.seen(u) == Seen::Hidden {
            continue;
        }
        let d = Vec3::from_array(u.pos.to_f32()) - here;
        if d.x.abs() > range || d.z.abs() > range {
            continue;
        }
        let ahead = d.x * s + d.z * co;
        let right = d.x * -co + d.z * s;
        let q = c + vec2(right, -ahead) / range * rad;
        if q.distance(c) > rad - mm(1.5) {
            continue;
        }
        let enemy = game.sim.is_enemy(me.owner, u.owner);
        // (Radar alone: a blip, no more.)
        if game.seen(u) == Seen::Radar {
            painter.circle_filled(q, mm(1.8), fade(AMBER_DIM, 0.9));
            continue;
        }
        let colour = if enemy { RED } else { PHOSPHOR };
        if game.sim.unit_type(u).locomotion == Locomotion::Air {
            let a = mm(2.6);
            painter.add(Shape::convex_polygon(vec![q + vec2(0.0, -a), q + vec2(a * 0.85, a * 0.7), q + vec2(-a * 0.85, a * 0.7)], colour, Stroke::NONE));
        } else {
            painter.circle_filled(q, if enemy { mm(2.4) } else { mm(1.8) }, colour);
        }
        if target == Some(u.id) {
            painter.circle_stroke(q, mm(3.6), Stroke::new(mm(0.5), AMBER_BRIGHT));
        }
        // (Firing at the unit: a ring swelling round it, as the threats' chevrons pulse.)
        if enemy && u.weapons.iter().any(|w| w.target == Some(ta_sim::Target::Unit(p.unit))) {
            let k = if p.reduced_flashes { 0.5 } else { ((now * 1.6) as f32).fract() };
            painter.circle_stroke(q, mm(3.0 + 4.0 * k), Stroke::new(mm(0.45), fade(RED, 1.0 - k)));
        }
    }
    // The hull's heading, from the middle.
    let h = me.heading as f32 * std::f32::consts::TAU / 65536.0 - look;
    let dir = vec2(-h.sin(), -h.cos());
    let side = vec2(-dir.y, dir.x);
    painter.add(Shape::convex_polygon(vec![c + dir * mm(4.0), c - dir * mm(2.0) + side * mm(2.4), c - dir * mm(2.0) - side * mm(2.4)], AMBER_BRIGHT, Stroke::NONE));
    // (How far its rim reaches, in metres.)
    painter.text(pos2(c.x, c.y + rad * 0.82), Align2::CENTER_CENTER, format!("{} {}", metres(range).round() as i32, t("M")), mono(mm(5.4)), fade(PHOSPHOR, 0.6));
}

/// Half the view's field across (radians): its field up and down (`fov`, radians: the cockpit's,
/// magnified) on a picture `aspect` times as wide as it is tall.
fn view_across(fov: f32, aspect: f32) -> f32 {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 16.0 / 9.0 };
    ((fov * 0.5).tan() * aspect).atan()
}

/// The drive page: the speed in figures, the throttle as a bar, the hull's attitude.
fn glass_drive(painter: &Painter, r: Rect, rd: &Readouts) {
    let rad = r.width().min(r.height()) * 0.5 - mm(1.5);
    let Some(k) = fill_scale(Vec2::splat(rad), Vec2::splat(pages::DRIVE_RADIUS), f32::MAX) else { return };
    pages::drive(&Pen::new(painter, k, false, 1.0), r.center(), rd);
}

/// The heading display: bearings sliding past a fixed mark, the bearing in figures at its right,
/// the hull's heading marked and the bearing of what the guns are on.
fn glass_compass(painter: &Painter, r: Rect, p: &Perspective, game: &Game) {
    let bearing = |d: Vec3| d.x.atan2(-d.z).to_degrees().rem_euclid(360.0);
    let look = bearing(p.forward);
    let g = r.shrink2(vec2(mm(3.5), mm(2.0)));
    // The figures at the right, boxed (as wide as three of them); the tape the rest.
    let readout = (g.height() * 1.45).max(mm(20.0));
    let tape = Rect::from_min_max(g.min, pos2(g.right() - readout - mm(3.0), g.bottom()));
    let box_rect = Rect::from_min_max(pos2(g.right() - readout, g.top()), g.max);
    painter.rect_stroke(box_rect, 0.0, Stroke::new(mm(0.5), fade(AMBER, 0.8)), egui::StrokeKind::Inside);
    painter.text(box_rect.center(), Align2::CENTER_CENTER, format!("{:03}", look.round() as i32 % 360), mono(g.height() * 0.62), AMBER_BRIGHT);
    // (Labelled every ten degrees, each label a bearing's first two figures, with room between
    // them.)
    let span = 56.0;
    let cx = tape.center().x;
    let at = |deg: f32| cx + ((deg - look + 540.0) % 360.0 - 180.0) / span * tape.width();
    let mut deg = (look - span * 0.5).ceil() as i32;
    while (deg as f32) <= look + span * 0.5 {
        let d = deg.rem_euclid(360);
        let x = at(deg as f32);
        if d % 5 == 0 {
            let major = d % 10 == 0;
            painter.line_segment([pos2(x, tape.top()), pos2(x, tape.top() + tape.height() * if major { 0.32 } else { 0.18 })], Stroke::new(mm(if major { 0.7 } else { 0.45 }), fade(AMBER, 0.85)));
            if major {
                let label = match d {
                    0 => t("N").to_string(),
                    90 => t("E").to_string(),
                    180 => t("S").to_string(),
                    270 => t("W").to_string(),
                    _ => format!("{:02}", d / 10),
                };
                painter.text(pos2(x, tape.top() + tape.height() * 0.68), Align2::CENTER_CENTER, label, mono(tape.height() * 0.5), AMBER);
            }
        }
        deg += 1;
    }
    // The lubber line: where the view looks, down from the top edge over the ticks.
    painter.line_segment([pos2(cx, tape.top()), pos2(cx, tape.top() + tape.height() * 0.4)], Stroke::new(mm(0.9), AMBER_BRIGHT));
    painter.add(Shape::convex_polygon(vec![pos2(cx, tape.top() + mm(2.4)), pos2(cx - mm(2.0), tape.top()), pos2(cx + mm(2.0), tape.top())], AMBER_BRIGHT, Stroke::NONE));
    // The hull's heading (walkers turn to where they are told to go), and the target's bearing.
    let Some(u) = game.sim.unit(p.unit) else { return };
    let h = u.heading as f32 * std::f32::consts::TAU / 65536.0;
    let hull = at(bearing(Vec3::new(h.sin(), 0.0, h.cos())));
    if (tape.left()..=tape.right()).contains(&hull) {
        let b = tape.bottom();
        painter.add(Shape::convex_polygon(vec![pos2(hull, b - mm(2.2)), pos2(hull + mm(1.8), b), pos2(hull - mm(1.8), b)], GREEN, Stroke::NONE));
    }
    // (Boxed in the ticks' band, over the figures: down the tape's height it struck a figure
    // through.)
    if let Some(target) = p.target(game).and_then(|id| game.scene.unit_position(id, game.alpha())) {
        let x = at(bearing(target - p.eye));
        if (tape.left()..=tape.right()).contains(&x) {
            let band = Rect::from_min_max(pos2(x - mm(1.6), tape.top()), pos2(x + mm(1.6), tape.top() + tape.height() * 0.4));
            painter.rect_stroke(band, 0.0, Stroke::new(mm(0.5), RED), egui::StrokeKind::Inside);
        }
    }
}

/// The Commander's economy (its side's): energy (the D-gun draws on it) and metal, each against
/// its storage, with what comes in and goes out a second.
fn glass_economy(painter: &Painter, r: Rect, game: &Game, owner: u8) {
    let Some(pl) = game.sim.players.get(owner as usize) else { return };
    let e = &pl.economy;
    let g = glass_inside(r);
    let rows = [
        (t("ENERGY"), e.energy.to_f32(), e.energy_capacity.to_f32(), e.energy_income.to_f32(), e.energy_wanted.to_f32(), AMBER),
        (t("METAL"), e.metal.to_f32(), e.metal_capacity.to_f32(), e.metal_income.to_f32(), e.metal_wanted.to_f32(), Color32::from_gray(215)),
    ];
    let row_h = g.height() / 2.0;
    for (i, (label, have, max, income, expense, colour)) in rows.iter().enumerate() {
        let y = g.top() + i as f32 * row_h;
        painter.text(pos2(g.left(), y), Align2::LEFT_TOP, *label, mono(mm(7.4)), PHOSPHOR);
        painter.text(pos2(g.right(), y), Align2::RIGHT_TOP, format!("{:.0}", have), mono(mm(7.4)), *colour);
        let bar = Rect::from_min_size(pos2(g.left(), y + mm(10.0)), vec2(g.width(), mm(6.0)));
        segment_bar(painter, bar, 12, (have / max.max(1.0)).clamp(0.0, 1.0), *colour, fade(PHOSPHOR_DIM, 0.45));
        painter.text(pos2(g.left(), bar.bottom() + mm(2.0)), Align2::LEFT_TOP, format!("+{:.0}", income), mono(mm(6.4)), GREEN);
        painter.text(pos2(g.right(), bar.bottom() + mm(2.0)), Align2::RIGHT_TOP, format!("-{:.0}", expense), mono(mm(6.4)), if *expense > *income { RED } else { fade(PHOSPHOR, 0.6) });
    }
}

// ---- The glass -------------------------------------------------------------------------------

/// The window the crew looks out through this frame (points): a modelled cockpit's between its
/// top and its sill, the periscope's round field, the magnified sight's circle; from behind the
/// unit, the picture over the panels along its foot and under the heading tape.
fn window(view: &View, p: &Perspective, modelled: bool) -> Window {
    let r = view.rect;
    let (w, h) = (r.width(), r.height());
    if p.chase {
        return Window { rect: Rect::from_min_max(pos2(r.left() + w * 0.03, r.top() + h * 0.12), pos2(r.right() - w * 0.03, r.top() + h * 0.76)), round: false };
    }
    if p.zoom > 0 && p.cockpit != Cockpit::Periscope {
        return Window { rect: Rect::from_center_size(r.center(), Vec2::splat(h * 0.92)), round: true };
    }
    if p.cockpit == Cockpit::Periscope {
        let (c, rad) = periscope_field(r);
        return Window { rect: Rect::from_center_size(c, Vec2::splat(rad * 2.0)), round: true };
    }
    let top = if modelled { p.lintel_y.map_or(r.top() + h * 0.08, |y| y / view.ppp) } else { r.top() + h * 0.08 };
    let bottom = if modelled { p.sill_y.map_or(r.bottom() - h * 0.25, |y| y / view.ppp) } else { r.bottom() - h * 0.25 };
    // (A ship's bridge has its sill close under the middle of the view.)
    let top = top.clamp(r.top() + h * 0.03, r.center().y - h * 0.2);
    let bottom = bottom.clamp(r.center().y + h * 0.05, r.bottom() - h * 0.02);
    Window { rect: Rect::from_min_max(pos2(r.left() + w * 0.04, top), pos2(r.right() - w * 0.04, bottom)), round: false }
}

/// The periscope's round field: its middle and radius (points), as large as the picture's height
/// allows with its bearing ring and figures round it in the picture, and no wider than a share of
/// its width.
fn periscope_field(r: Rect) -> (Pos2, f32) {
    // (The ring's ticks and figures reach this far past the field's rim, a pixel of a 1080-line
    // picture each, and keep a little off the picture's edge.)
    const RING: f32 = 50.0;
    let c = pos2(r.center().x, r.top() + r.height() * 0.47);
    (c, (r.width() * 0.29).min(r.height() * 0.47 - RING * lettering(r)).max(r.height() * 0.2))
}

/// Sparks from the window's top when the hull is nearly gone (a quarter or less of it): a burst
/// now and then, falling and fading (without the flash where it shorts out, with flashes
/// reduced).
fn sparks(painter: &Painter, window: &Window, p: &Perspective, on: f32, now: f64) {
    if p.hud.health >= crate::perspective::CRACK_AT[1] || p.reduced_motion || p.chase || on <= 0.0 {
        return;
    }
    let r = window.rect;
    // (The picture's own pixels, a unit a pixel of a 1080-line picture.)
    let u = painter.ctx().content_rect().height().max(1.0) / 1080.0;
    let alpha = |a: f32| (a * on) as u8;
    // A burst every 1.7 s from somewhere along the window's top: sparks sprayed out, falling and
    // cooling from white through orange.
    let period = 1.7;
    let n = (now / period).floor();
    for burst in [n - 1.0, n] {
        let age = (now - burst * period) as f32;
        if !(0.0..1.3).contains(&age) {
            continue;
        }
        let seed = hash(burst as i64 as u32 ^ 0x5bd1_e995);
        let from = pos2(r.left() + r.width() * (0.1 + (seed % 800) as f32 / 1000.0), r.top() + 4.0 * u);
        // The flash where it shorts out.
        if age < 0.12 && !p.reduced_flashes {
            painter.circle_filled(from, 14.0 * u, Color32::from_rgba_unmultiplied(255, 220, 160, alpha(120.0 * (1.0 - age / 0.12))));
        }
        for k in 0..16u32 {
            let h = hash(seed ^ k.wrapping_mul(0x9E37_79B9));
            let vx = ((h % 200) as f32 / 100.0 - 1.0) * 190.0 * u;
            let vy = ((h / 200 % 100) as f32 - 30.0) * 2.2 * u;
            let t = age * (0.75 + (h / 20000 % 50) as f32 / 100.0);
            let g = 900.0 * u;
            let at = from + vec2(vx * t, vy * t + 0.5 * g * t * t);
            let v = vec2(vx, vy + g * t);
            let tail = at - v * 0.04;
            let heat = (1.0 - t / 1.1).clamp(0.0, 1.0);
            if heat <= 0.0 {
                continue;
            }
            let col = Color32::from_rgba_unmultiplied(255, (140.0 + 100.0 * heat) as u8, (60.0 + 140.0 * heat * heat) as u8, alpha(240.0 * heat));
            painter.line_segment([tail, at], Stroke::new(3.2 * u, Color32::from_rgba_unmultiplied(255, 120, 40, alpha(60.0 * heat))));
            painter.line_segment([tail, at], Stroke::new(1.6 * u, col));
            painter.circle_filled(at, 1.4 * u, Color32::from_rgba_unmultiplied(255, 250, 220, alpha(255.0 * heat)));
        }
    }
}

fn glass(painter: &Painter, rect: Rect, window: &Window, p: &Perspective, on: f32, now: f64) {
    // (A pixel of a 1080-line picture.)
    let u = lettering(rect);
    // Night sight: a green wash and its grain (still, with flashes reduced: it sparkled).
    if p.night_vision {
        painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(40, 120, 40, (70.0 * on) as u8));
        let grain = if p.reduced_flashes { 0 } else { (now * 60.0) as u32 };
        for k in 0..220u32 {
            let h = hash(grain.wrapping_mul(977) ^ k.wrapping_mul(7919));
            let x = rect.left() + (h % 10_000) as f32 / 10_000.0 * rect.width();
            let y = rect.top() + ((h / 10_000) % 10_000) as f32 / 10_000.0 * rect.height();
            painter.rect_filled(Rect::from_min_size(pos2(x, y), Vec2::splat(1.5 * u)), 0.0, Color32::from_rgba_unmultiplied(170, 255, 150, (40.0 * on) as u8));
        }
    }
    // The vignette: the eye's closeness to the glass.
    let c = rect.center();
    let mut mesh = Mesh::default();
    let (rx, ry) = (rect.width() * 0.75, rect.height() * 0.75);
    let n = 48;
    for i in 0..n {
        let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        let inner = |a: f32| pos2(c.x + a.cos() * rx * 0.62, c.y + a.sin() * ry * 0.62);
        let outer = |a: f32| pos2(c.x + a.cos() * rx * 1.3, c.y + a.sin() * ry * 1.3);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(inner(a0), Color32::TRANSPARENT);
        mesh.colored_vertex(inner(a1), Color32::TRANSPARENT);
        mesh.colored_vertex(outer(a1), Color32::from_black_alpha((150.0 * on) as u8));
        mesh.colored_vertex(outer(a0), Color32::from_black_alpha((150.0 * on) as u8));
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(Shape::mesh(mesh));
    // A reflection across the glass.
    if !p.chase && p.cockpit != Cockpit::Periscope {
        let mut m = Mesh::default();
        let a = pos2(rect.left() + rect.width() * 0.58, rect.top());
        let b = pos2(rect.left() + rect.width() * 0.66, rect.top());
        let d = pos2(rect.left() + rect.width() * 0.36, rect.bottom());
        let e = pos2(rect.left() + rect.width() * 0.44, rect.bottom());
        let k = Color32::from_rgba_unmultiplied(255, 255, 255, (7.0 * on) as u8);
        for (q, col) in [(a, Color32::TRANSPARENT), (b, k), (e, Color32::TRANSPARENT), (d, k)] {
            m.colored_vertex(q, col);
        }
        m.add_triangle(0, 1, 2);
        m.add_triangle(0, 2, 3);
        painter.add(Shape::mesh(m));
    }
    // Cracks: jagged lines out from where the glass took the blow, in the window only, each
    // running out over a quarter of a second as it comes. (From behind the unit there is no glass
    // between the eye and the battle.)
    if p.chase {
        return;
    }
    let cracked = painter.with_clip_rect(window.rect.expand(4.0 * u));
    let white = |a: f32| Color32::from_white_alpha((a * on) as u8);
    for c in &p.hud.cracks {
        let origin = pos2(window.rect.left() + c.at.x * window.rect.width(), window.rect.top() + c.at.y * window.rect.height());
        let grown = crate::perspective::ease(((now - c.since) as f32 / 0.25).clamp(0.0, 1.0));
        let len = c.length * rect.height() * grown;
        let seed = c.seed;
        for branch in 0..6u32 {
            let mut a = c.angle + branch as f32 * 1.05 + ((hash(seed ^ branch) % 100) as f32 / 100.0 - 0.5);
            let mut q = origin;
            let mut pts = vec![q];
            let steps = 5 + hash(seed.wrapping_add(branch)) % 4;
            let reach = 0.55 + (hash(seed ^ (branch + 77)) % 45) as f32 / 100.0;
            for s in 0..steps {
                let h = hash(seed ^ (branch * 31 + s * 7));
                a += ((h % 100) as f32 / 100.0 - 0.5) * 0.9;
                q += vec2(a.cos(), a.sin()) * len * reach / steps as f32 * (0.6 + (h % 50) as f32 / 100.0);
                pts.push(q);
                // A spur off the way now and then.
                if h.is_multiple_of(3) && s + 1 < steps {
                    let b = a + if h.is_multiple_of(2) { 0.9 } else { -0.9 };
                    let spur = q + vec2(b.cos(), b.sin()) * len * 0.12;
                    cracked.line_segment([q, spur], Stroke::new(0.8 * u, white(45.0)));
                }
            }
            cracked.add(Shape::line(pts.clone(), Stroke::new(3.0 * u, Color32::from_black_alpha((110.0 * on) as u8))));
            cracked.add(Shape::line(pts, Stroke::new(1.2 * u, white(115.0))));
        }
        // The impact: a star of chips, frosted.
        cracked.circle_filled(origin, 9.0 * u * grown, white(22.0));
        cracked.circle_filled(origin, 3.2 * u, white(90.0));
        // A ring round it, part of the way.
        let rr = len * 0.22;
        let ring: Vec<Pos2> = (0..=14).map(|k| {
            let a = c.angle + 0.4 + k as f32 / 14.0 * 4.2;
            origin + vec2(a.cos(), a.sin()) * rr * (0.9 + (hash(seed ^ k) % 20) as f32 / 100.0)
        }).collect();
        cracked.add(Shape::line(ring, Stroke::new(0.9 * u, white(50.0))));
    }
}

/// Rain on the glass: drops that cling and swell, then run down it leaving a trail (on an
/// aircraft's canopy the wind blows them back and up); over the opening only, not the dashboard.
fn raindrops(painter: &Painter, rect: Rect, p: &Perspective, rain: f32, now: f64) {
    let flying = p.cockpit.flies();
    let u = lettering(rect);
    let glass_bottom = if p.model.is_some() && p.seat > 0.02 { 0.74 } else { 0.8 };
    let count = (rain * 110.0) as u32;
    let t = now as f32;
    for k in 0..count {
        let h = hash(k.wrapping_mul(747_796_405) ^ 0x9e37);
        let r1 = (h % 1000) as f32 / 1000.0;
        let r2 = ((h / 1000) % 1000) as f32 / 1000.0;
        let r3 = ((h / 1_000_000) % 1000) as f32 / 1000.0;
        // Each drop's life: it clings for a while, then runs (a period of 3 to 9 seconds).
        let period = 3.0 + r3 * 6.0;
        let phase = (t / period + r1).fract();
        let running = ((phase - 0.55) / 0.45).clamp(0.0, 1.0);
        let (x, y) = if flying {
            // Blown back along the canopy: outwards from the middle and up.
            let cx = r1 - 0.5;
            let cy = r2 - 0.5;
            let spread = 1.0 + running * 1.6;
            (0.5 + cx * spread, 0.5 + cy * spread - running * 0.2)
        } else {
            (r1, r2 * glass_bottom * 0.85 + running * running * 0.5)
        };
        if !(0.02..0.98).contains(&x) || y > glass_bottom || y < 0.03 {
            continue;
        }
        let at = pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height());
        let size = (1.5 + r3 * 3.5) * (1.0 + (1.0 - running) * phase * 0.6) * u;
        let a = rain.min(1.0);
        // The trail a running drop leaves, fading.
        if running > 0.0 {
            let back = if flying { (at - rect.center()).normalized() * -40.0 * u * running } else { vec2(0.0, -rect.height() * 0.09 * running) };
            painter.line_segment([at, at + back], Stroke::new(size * 0.55, Color32::from_white_alpha((22.0 * a) as u8)));
        }
        // The drop: dark rim, bright lensed centre, a highlight.
        painter.circle_filled(at, size, Color32::from_black_alpha((38.0 * a) as u8));
        painter.circle_filled(at, size * 0.78, Color32::from_white_alpha((34.0 * a) as u8));
        painter.circle_filled(at + vec2(-size * 0.28, -size * 0.3), size * 0.26, Color32::from_white_alpha((120.0 * a) as u8));
    }
}

/// The sight when magnified: a round field, black round it, with fine cross-hairs open in the
/// middle and its scales.
fn magnified_sight(painter: &Painter, rect: Rect, on: f32) {
    let c = rect.center();
    let r = rect.height() * 0.46;
    // (A pixel of a 1080-line picture.)
    let u = rect.height() / 1080.0;
    ring(painter, c, r, rect.width().max(rect.height()), Color32::from_black_alpha((250.0 * on) as u8));
    painter.circle_stroke(c, r, Stroke::new(3.0 * u, fade(METAL_LIT, on)));
    painter.circle_stroke(c, r + 6.0 * u, Stroke::new(8.0 * u, fade(METAL, on)));
    for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
        painter.line_segment([c + d * 14.0 * u, c + d * (r - 4.0 * u)], Stroke::new((1.2 * u).max(1.0), Color32::from_black_alpha((170.0 * on) as u8)));
    }
    // Mil scale along the horizontal.
    for k in -10..=10 {
        if k == 0 {
            continue;
        }
        let x = c.x + k as f32 * r * 0.08;
        let h = if k % 5 == 0 { 12.0 } else { 6.0 } * u;
        painter.line_segment([pos2(x, c.y + 18.0 * u), pos2(x, c.y + 18.0 * u + h)], Stroke::new((1.2 * u).max(1.0), fade(AMBER, on * 0.9)));
    }
}

/// A ring from radius `r` out to `outer`, filled (the outside of a round field of view).
fn ring(painter: &Painter, c: Pos2, r: f32, outer: f32, color: Color32) {
    let mut mesh = Mesh::default();
    let n = 72;
    for i in 0..n {
        let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        let base = mesh.vertices.len() as u32;
        for (a, rad) in [(a0, r), (a1, r), (a1, outer), (a0, outer)] {
            mesh.colored_vertex(pos2(c.x + a.cos() * rad, c.y + a.sin() * rad), color);
        }
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(Shape::mesh(mesh));
}

// ---- The frame ---------------------------------------------------------------------------------

/// The opening the crew looks out through, in fractions of the screen (a convex outline).
fn opening(cockpit: Cockpit) -> Vec<(f32, f32)> {
    match cockpit {
        Cockpit::Vehicle | Cockpit::Hover => vec![(0.035, 0.1), (0.1, 0.06), (0.9, 0.06), (0.965, 0.1), (0.975, 0.66), (0.93, 0.745), (0.07, 0.745), (0.025, 0.66)],
        Cockpit::Walker | Cockpit::Titan => vec![(0.03, 0.15), (0.2, 0.05), (0.8, 0.05), (0.97, 0.15), (0.975, 0.6), (0.82, 0.79), (0.18, 0.79), (0.025, 0.6)],
        Cockpit::Commander => vec![(0.015, 0.12), (0.26, 0.035), (0.74, 0.035), (0.985, 0.12), (0.99, 0.56), (0.84, 0.8), (0.16, 0.8), (0.01, 0.56)],
        Cockpit::Bridge => vec![(-0.01, 0.12), (1.01, 0.12), (1.01, 0.68), (-0.01, 0.68)],
        Cockpit::Canopy => {
            // An arch over the head, the glare shield across the bottom.
            let mut v: Vec<(f32, f32)> = (0..=16).map(|k| {
                let a = std::f32::consts::PI * (k as f32 / 16.0);
                (0.5 - a.cos() * 0.62, 0.8 - a.sin() * 0.92)
            }).collect();
            v.retain(|q| q.1 < 0.8);
            v.insert(0, (-0.12, 0.8));
            v.push((1.12, 0.8));
            v
        }
        Cockpit::Gunship => (0..28).map(|k| {
            let a = k as f32 / 28.0 * std::f32::consts::TAU;
            (0.5 + a.cos() * 0.56, 0.43 + a.sin() * 0.5)
        }).collect(),
        Cockpit::Emplacement => vec![(0.05, 0.22), (0.95, 0.22), (0.97, 0.3), (0.97, 0.66), (0.93, 0.7), (0.07, 0.7), (0.03, 0.66), (0.03, 0.3)],
        Cockpit::Periscope => (0..48).map(|k| {
            let a = k as f32 / 48.0 * std::f32::consts::TAU;
            (0.5 + a.cos() * 0.29, 0.47 + a.sin() * 0.46)
        }).collect(),
    }
}

/// The top of the painted frame's opening (points).
fn opening_top(cockpit: Cockpit, rect: Rect) -> f32 {
    rect.top() + rect.height() * opening(cockpit).iter().map(|q| q.1).fold(f32::MAX, f32::min)
}

/// The cockpit's frame round the opening, with its fittings, for a kind without a model of its
/// own (a submarine's periscope). Solid metal, it closes in from beyond the edges of the view as
/// the dive ends, as a model does ([`Perspective::seat`]).
fn frame(painter: &Painter, rect: Rect, p: &Perspective, seat: f32, trim: Color32, now: f64) {
    if seat <= 0.0 {
        return;
    }
    let on = 1.0;
    // (Its fittings in pixels of a 1080-line picture.)
    let u = lettering(rect);
    let periscope = p.cockpit == Cockpit::Periscope;
    let to_screen = |(x, y): (f32, f32)| pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height());
    // (The periscope's field is round, whatever the picture's shape.)
    let field = periscope_field(rect);
    let outline = opening(p.cockpit);
    let mid = if periscope { field.0 } else { (outline.iter().fold(Vec2::ZERO, |sum, &q| sum + to_screen(q).to_vec2()) / outline.len() as f32).to_pos2() };
    // Coming in: out from the opening's middle, past the edges, back to its place.
    let grow = 1.0 + (1.0 - crate::perspective::ease(seat)) * 1.6;
    let at = |x: f32, y: f32| mid + (to_screen((x, y)) - mid) * grow;
    let pts: Vec<Pos2> = if periscope {
        (0..48).map(|k| field.0 + clock(k as f32 / 48.0 * std::f32::consts::TAU) * field.1 * grow).collect()
    } else {
        outline.iter().map(|&(x, y)| at(x, y)).collect()
    };
    let c = if periscope { field.0 } else { at(0.5, 0.42) };
    // The metal between the opening and the screen's edge: a quad per edge of the opening,
    // out along the rays from the middle, lit at the opening's lip.
    let mut mesh = Mesh::default();
    let far = rect.width().max(rect.height()) * 1.5;
    let edge_out = |q: Pos2| c + (q - c).normalized() * far;
    let (inner, outer) = if periscope { (Color32::from_gray(4), Color32::BLACK) } else { (METAL_LIT, METAL_DEEP) };
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        // Lit from above: edges facing down (the top of the opening) catch less light.
        let normal = vec2(b.y - a.y, a.x - b.x).normalized();
        let lit = (0.55 + normal.y * -0.45).clamp(0.1, 1.0);
        let base = mesh.vertices.len() as u32;
        let lip = lerp_color(outer, inner, lit * on);
        mesh.colored_vertex(a, lip);
        mesh.colored_vertex(b, lip);
        mesh.colored_vertex(edge_out(b), fade(outer, on));
        mesh.colored_vertex(edge_out(a), fade(outer, on));
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(Shape::mesh(mesh));
    if periscope {
        periscope_marks(painter, c, &pts, on, p, u);
        return;
    }
    // The lip: a bevel along the opening, and bolts.
    let mut lip = pts.clone();
    lip.push(pts[0]);
    painter.add(Shape::line(lip.clone(), Stroke::new(5.0 * u, fade(METAL, on))));
    painter.add(Shape::line(lip, Stroke::new(1.4 * u, fade(METAL_EDGE, on * 0.8))));
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        let len = a.distance(b);
        let out = -vec2(b.y - a.y, a.x - b.x).normalized() * 14.0 * u;
        let n = (len / (90.0 * u)).floor() as usize;
        for k in 1..n {
            rivet(painter, a + (b - a) * (k as f32 / n as f32) + out, on, u);
        }
    }
    // Over the opening, where the frame has room for them (a tall opening leaves none): the
    // side's colour on a strip, and lamps over it, steady green while all is well, the hull
    // lamp red (blinking; lit, with flashes reduced).
    let top_y = pts.iter().map(|q| q.y).fold(f32::MAX, f32::min);
    let lamp_y = top_y - 34.0 * u;
    if lamp_y > rect.top() + 6.0 * u {
        let strip = Rect::from_center_size(pos2(rect.center().x, top_y - 16.0 * u), vec2(rect.width() * 0.16, 5.0 * u));
        painter.rect_filled(strip, u, fade(trim, on * 0.9));
        hazard(painter, Rect::from_center_size(strip.center() + vec2(-strip.width() * 0.5 - 30.0 * u, 0.0), vec2(44.0, 7.0) * u), on);
        hazard(painter, Rect::from_center_size(strip.center() + vec2(strip.width() * 0.5 + 30.0 * u, 0.0), vec2(44.0, 7.0) * u), on);
        let hurt = p.hud.health < 0.35;
        let blink = p.reduced_flashes || (now * 4.0).fract() < 0.5;
        // (Power, systems, hull, weapons.)
        for (k, (label, colour)) in [(n("PWR"), GREEN), (n("SYS"), GREEN), (n("HULL"), if hurt { RED } else { GREEN }), (n("WPN"), AMBER)].iter().enumerate() {
            let x = rect.center().x + (k as f32 - 1.5) * 64.0 * u;
            let lit = if k == 2 && hurt { blink } else { true };
            lamp(painter, pos2(x - 16.0 * u, lamp_y), 4.5 * u, *colour, lit, on);
            painter.text(pos2(x - 8.0 * u, lamp_y), Align2::LEFT_CENTER, t(label), mono(10.5 * u), fade(METAL_EDGE, on));
        }
    }
    match p.cockpit {
        Cockpit::Bridge => {
            // Window mullions, raked, either side of a clear middle pane.
            for x in [0.2, 0.8] {
                let lean = vec2((x - 0.5) * 30.0 * u, 0.0) * grow;
                let a = at(x, 0.12) - lean;
                let b = at(x, 0.68) + lean;
                painter.add(Shape::convex_polygon(vec![a + vec2(-7.0, 0.0) * u, a + vec2(7.0, 0.0) * u, b + vec2(9.0, 0.0) * u, b + vec2(-9.0, 0.0) * u], fade(METAL, on), Stroke::new(u, fade(METAL_LIT, on * 0.7))));
            }
        }
        Cockpit::Canopy => {
            // The canopy's bow over the middle.
            let a = at(0.5, 0.0) - vec2(0.0, 10.0 * u);
            let b = at(0.5, 0.045);
            painter.add(Shape::convex_polygon(vec![a + vec2(-9.0, 0.0) * u, a + vec2(9.0, 0.0) * u, b + vec2(5.0, 0.0) * u, b + vec2(-5.0, 0.0) * u], fade(METAL, on), Stroke::NONE));
        }
        _ => {}
    }
    // Levers and grips, tank tillers and a walker's sticks: they follow the unit as it drives
    // itself, its speed and its turning.
    let steer = p.hud.turn.x.clamp(-1.0, 1.0);
    let throttle = p.hud.speed.x.clamp(-0.2, 1.0);
    match p.cockpit {
        Cockpit::Vehicle | Cockpit::Hover => {
            for (x, s) in [(0.06, -1.0f32), (0.94, 1.0)] {
                // Each tiller leans with the throttle, and the one on the side turned to pulls back.
                let pull = throttle - (steer * s).max(0.0) * 0.8 + (steer * -s).max(0.0) * 0.3;
                lever(painter, at(x, 1.0) + vec2(0.0, 8.0 * u), rect.height() * 0.2, pull * 0.35, on, trim, u);
            }
        }
        Cockpit::Walker | Cockpit::Titan | Cockpit::Commander => {
            for (x, s) in [(0.08, -1.0f32), (0.92, 1.0)] {
                let lean = throttle * 0.25 + steer * s * 0.2;
                lever(painter, at(x, 1.0) + vec2(0.0, 12.0 * u), rect.height() * 0.15, lean, on, trim, u);
            }
        }
        _ => {}
    }
}

/// The periscope's marks round the field (its middle `c`, its edge `pts`): the bearing ring and
/// its cross-wires, in pixels of a 1080-line picture (`u`).
fn periscope_marks(painter: &Painter, c: Pos2, pts: &[Pos2], on: f32, p: &Perspective, u: f32) {
    let r = pts.iter().map(|q| q.distance(c)).fold(0.0f32, f32::max);
    painter.circle_stroke(c, r, Stroke::new(4.0 * u, fade(METAL_LIT, on)));
    let bearing = p.forward.x.atan2(-p.forward.z).to_degrees().rem_euclid(360.0);
    for k in 0..72 {
        let deg = k as f32 * 5.0;
        let a = (deg - bearing).to_radians() - std::f32::consts::FRAC_PI_2;
        let len = if k % 6 == 0 { 16.0 } else { 7.0 } * u;
        let (s, co) = a.sin_cos();
        let from = r + 4.0 * u;
        painter.line_segment([pos2(c.x + co * from, c.y + s * from), pos2(c.x + co * (from + len), c.y + s * (from + len))], Stroke::new((1.3 * u).max(1.0), fade(AMBER_DIM, on)));
        if k % 6 == 0 {
            painter.text(pos2(c.x + co * (r + 34.0 * u), c.y + s * (r + 34.0 * u)), Align2::CENTER_CENTER, format!("{:03}", deg as i32), mono(11.0 * u), fade(AMBER, on));
        }
    }
    painter.line_segment([pos2(c.x - r, c.y), pos2(c.x + r, c.y)], Stroke::new(u.max(1.0), Color32::from_black_alpha((160.0 * on) as u8)));
    painter.line_segment([pos2(c.x, c.y - r), pos2(c.x, c.y + r)], Stroke::new(u.max(1.0), Color32::from_black_alpha((160.0 * on) as u8)));
}

/// A bolt head, `u` a pixel of a 1080-line picture.
fn rivet(painter: &Painter, at: Pos2, on: f32, u: f32) {
    painter.circle_filled(at + vec2(0.8, 1.2) * u, 3.4 * u, fade(METAL_DEEP, on));
    painter.circle_filled(at, 3.0 * u, fade(METAL, on));
    painter.circle_filled(at + vec2(-0.9, -0.9) * u, 1.2 * u, fade(METAL_EDGE, on * 0.9));
}

fn hazard(painter: &Painter, r: Rect, on: f32) {
    painter.rect_filled(r, 0.0, fade(Color32::from_rgb(20, 18, 12), on));
    // Yellow bars, slanted, cut off at the strip's ends.
    let bar_w = r.height() * 5.0 / 7.0;
    let mut x = r.left() - r.height();
    while x < r.right() {
        let bar = [pos2(x, r.bottom()), pos2(x + r.height(), r.top()), pos2(x + r.height() + bar_w, r.top()), pos2(x + bar_w, r.bottom())];
        let clipped: Vec<Pos2> = bar.iter().map(|q| pos2(q.x.clamp(r.left(), r.right()), q.y)).collect();
        painter.add(Shape::convex_polygon(clipped, fade(Color32::from_rgb(196, 160, 40), on), Stroke::NONE));
        x += bar_w * 2.0;
    }
}

fn lamp(painter: &Painter, at: Pos2, r: f32, colour: Color32, lit: bool, on: f32) {
    // (Its rim and glow in proportion to it: 4.5 at 1080 lines.)
    let k = r / 4.5;
    painter.circle_filled(at, r + 1.5 * k, fade(METAL_DEEP, on));
    if lit {
        painter.circle_filled(at, r + 4.0 * k, fade(colour, on * 0.18));
        painter.circle_filled(at, r, fade(colour, on));
        painter.circle_filled(at + vec2(-r * 0.3, -r * 0.3), r * 0.35, fade(Color32::WHITE, on * 0.6));
    } else {
        painter.circle_filled(at, r, fade(colour, on * 0.2));
    }
}

/// A lever rising from `base`, `length` long, leaning `lean` (radians, forward positive), `u` a
/// pixel of a 1080-line picture.
#[allow(clippy::too_many_arguments)]
fn lever(painter: &Painter, base: Pos2, length: f32, lean: f32, on: f32, trim: Color32, u: f32) {
    let tip = base + vec2(lean.sin() * length * 0.6, -lean.cos().abs() * length * (0.8 + lean * 0.2));
    let dir = (tip - base).normalized();
    let side = vec2(-dir.y, dir.x);
    painter.add(Shape::convex_polygon(vec![base + side * 9.0 * u, base - side * 9.0 * u, tip - side * 6.0 * u, tip + side * 6.0 * u], fade(METAL, on), Stroke::new(u, fade(METAL_LIT, on))));
    painter.circle_filled(tip, 13.0 * u, fade(Color32::from_rgb(28, 26, 22), on));
    painter.circle_stroke(tip, 13.0 * u, Stroke::new(2.0 * u, fade(trim, on * 0.7)));
    painter.circle_filled(tip + vec2(-4.0, -4.0) * u, 3.5 * u, fade(METAL_EDGE, on * 0.5));
}

// ---- Over the world -------------------------------------------------------------------------------

/// The kbots: walkers, the experimental walkers and the Commander.
fn walks(cockpit: Cockpit) -> bool {
    matches!(cockpit, Cockpit::Walker | Cockpit::Titan | Cockpit::Commander)
}

/// Where the painted heading tape's top is (points down the screen).
fn compass_top(view: &View, p: &Perspective) -> f32 {
    let u = lettering(view.rect);
    // (Its plate reaches 5 over its top, and the hull's mark 13.)
    let over = if p.cockpit.flies() { 9.0 } else { 17.0 } * u;
    let behind = p.chase || p.zoom > 0;
    if behind {
        view.rect.top() + view.rect.height() * 0.03
    } else if p.model.is_none() {
        // Inside the painted frame's opening, clear of its lip and bolts (the lamps and the
        // strip over the opening are on the frame, above it).
        opening_top(p.cockpit, view.rect) + 19.0 * u + over
    } else {
        // Under the top of the model's window (a canopy's arch, a cab's frame), as it comes in.
        let under = p.lintel_y.map_or(f32::MIN, |y| y / view.ppp + 4.0 * u + over);
        (view.rect.top() + view.rect.height() * 0.085).max(under)
    }
}

/// A heading tape across the top of the view: where the eye looks, and where the hull points.
/// Not in the periscope (its bearing ring is round its field), but from behind the submarine.
/// Returns where it ends (points down the screen), if it is drawn.
fn compass(painter: &Painter, view: &View, p: &Perspective, game: &Game, on: f32) -> Option<f32> {
    if p.cockpit == Cockpit::Periscope && !p.chase {
        return None;
    }
    let bearing = |d: Vec3| d.x.atan2(-d.z).to_degrees().rem_euclid(360.0);
    let look = bearing(p.forward);
    let behind = p.chase || p.zoom > 0;
    let top = compass_top(view, p);
    // (A pixel of a 1080-line picture.)
    let k = lettering(view.rect);
    // (As wide as the cockpit's window leaves room for under its top, of a picture no wider than
    // 16:9; as many degrees to the point whatever its width.)
    let share = if behind || p.model.is_none() { crate::perspective::TAPE_WIDTH } else { p.cockpit.tape_width() };
    let width = foot_band(view.rect).width() * share;
    let cx = view.rect.center().x;
    let colour = if p.cockpit.flies() { PHOSPHOR } else { AMBER };
    let span = 80.0 * share / crate::perspective::TAPE_WIDTH;
    painter.rect_filled(Rect::from_center_size(pos2(cx, top + 16.0 * k), vec2(width + 20.0 * k, 42.0 * k)), 4.0 * k, Color32::from_black_alpha((140.0 * on) as u8));
    // (A tick every 5°, on whole fives: the labels go on the tens, each a bearing's first two
    // figures.)
    let mut deg = ((look - span * 0.5) / 5.0).ceil() as i32 * 5;
    while (deg as f32) <= look + span * 0.5 {
        let x = cx + (deg as f32 - look) / span * width;
        let d = deg.rem_euclid(360);
        let major = d % 10 == 0;
        painter.line_segment([pos2(x, top), pos2(x, top + if major { 13.0 } else { 6.0 } * k)], Stroke::new(1.5 * k, fade(colour, on * 0.9)));
        if major {
            // (The compass's points: north, east, south, west.)
            let label = match d {
                0 => t("N").to_string(),
                90 => t("E").to_string(),
                180 => t("S").to_string(),
                270 => t("W").to_string(),
                _ => format!("{:02}", d / 10),
            };
            painter.text(pos2(x, top + 25.0 * k), Align2::CENTER_CENTER, label, mono(15.0 * k), fade(colour, on));
        }
        deg += 5;
    }
    painter.add(Shape::convex_polygon(vec![pos2(cx, top + 37.0 * k), pos2(cx - 8.0 * k, top + 47.0 * k), pos2(cx + 8.0 * k, top + 47.0 * k)], fade(AMBER_BRIGHT, on), Stroke::NONE));
    // The hull's heading on the tape.
    if let Some(u) = game.sim.unit(p.unit)
        && !p.cockpit.flies()
    {
        let h = u.heading as f32 * std::f32::consts::TAU / 65536.0;
        let hull = bearing(Vec3::new(h.sin(), 0.0, h.cos()));
        let off = ((hull - look + 540.0) % 360.0) - 180.0;
        let x = cx + (off / span * width).clamp(-width * 0.5, width * 0.5);
        painter.add(Shape::convex_polygon(vec![pos2(x, top - 3.0 * k), pos2(x - 8.0 * k, top - 13.0 * k), pos2(x + 8.0 * k, top - 13.0 * k)], fade(GREEN, on), Stroke::NONE));
    }
    plated(painter, pos2(cx + width * 0.5 + 22.0 * k, top + 16.0 * k), Align2::LEFT_CENTER, format!("{:03}°", look.round() as i32 % 360), mono(18.0 * k), fade(colour, on));
    Some(top + 47.0 * k)
}

// ---- The instruments -------------------------------------------------------------------------------

/// Where the instruments sit for a cockpit.
struct Layout {
    speed: Rect,
    status: Rect,
    radar: Rect,
    hull: Rect,
}

impl Layout {
    fn for_cockpit(cockpit: Cockpit, r: Rect, bare: bool) -> Layout {
        let (w, h) = (r.width(), r.height());
        let at = |x: f32, y: f32, sw: f32, sh: f32| Rect::from_min_size(pos2(r.left() + x * w, r.top() + y * h), vec2(sw * w, sh * h));
        if bare || cockpit == Cockpit::Periscope {
            // Along the bottom edge, out of the way, down to one margin, in the middle of a
            // picture wider than 16:9: the dials alike, side by side at the left, the unit's
            // panel and its weapons' at the right (the panels are brought to one height in
            // `draw`). On a small picture no smaller than their pages can be read at, as far as
            // its width allows.
            let band = foot_band(r);
            let w = band.width();
            let (margin, between_dials, between_panels, middle) = (0.02 * w, 0.012 * w, 0.015 * w, 0.02 * w);
            let status_w = 0.23 * w;
            // (On a picture under 540 lines they would take more than three tenths of it: no more.)
            let mut dial = (0.2 * h).min(0.12 * w).max(least_dial().min(0.3 * h));
            let mut hull = vec2(0.125 * w, 0.15 * h).max(least_hull() * (0.3 * h / least_hull().y).min(1.0));
            let room = w - 2.0 * margin - between_dials - between_panels - middle - status_w;
            if 2.0 * dial + hull.x > room {
                let k = (room / (2.0 * dial + hull.x)).max(0.0);
                dial *= k;
                hull *= k;
            }
            let foot = r.top() + 0.98 * h;
            let (left, right) = (band.left() + margin, band.right() - margin);
            let status = Rect::from_min_max(pos2(right - status_w, foot - 0.2 * h), pos2(right, foot));
            let hull_right = status.left() - between_panels;
            return Layout {
                speed: Rect::from_min_size(pos2(left, foot - dial), Vec2::splat(dial)),
                radar: Rect::from_min_size(pos2(left + dial + between_dials, foot - dial), Vec2::splat(dial)),
                hull: Rect::from_min_max(pos2(hull_right - hull.x, foot - hull.y), pos2(hull_right, foot)),
                status,
            };
        }
        match cockpit {
            Cockpit::Canopy | Cockpit::Gunship => Layout {
                speed: at(0.10, 0.82, 0.13, 0.16),
                radar: at(0.44, 0.815, 0.12, 0.175),
                hull: at(0.26, 0.84, 0.14, 0.14),
                status: at(0.62, 0.82, 0.24, 0.16),
            },
            Cockpit::Commander | Cockpit::Walker | Cockpit::Titan => Layout {
                speed: at(0.13, 0.82, 0.13, 0.16),
                radar: at(0.44, 0.815, 0.12, 0.175),
                hull: at(0.28, 0.83, 0.13, 0.15),
                status: at(0.60, 0.82, 0.25, 0.16),
            },
            _ => Layout {
                speed: at(0.12, 0.775, 0.14, 0.2),
                radar: at(0.44, 0.77, 0.12, 0.21),
                hull: at(0.28, 0.79, 0.14, 0.18),
                status: at(0.59, 0.78, 0.27, 0.19),
            },
        }
    }
}

/// The middle of a picture, no wider than a 16:9 one: the panels along its foot stand in it (on a
/// 32:9 screen they stood five thousand points apart, at its ends).
fn foot_band(r: Rect) -> Rect {
    Rect::from_center_size(r.center(), vec2(r.width().min(r.height() * 16.0 / 9.0), r.height()))
}

/// How large a page painted along the picture's foot is lettered at least (points to its unit):
/// at 720 lines it was five or six points.
const PAGE_LEAST: f32 = 1.3;

/// The least a painted dial is across (points): its drive page lettered at [`PAGE_LEAST`], in its
/// bezel.
fn least_dial() -> f32 {
    2.0 * (PAGE_LEAST * pages::DRIVE_RADIUS + 6.0)
}

/// The least the unit's painted panel is (points): its page lettered at [`PAGE_LEAST`], on the
/// glass inside its bezel, as tall as `hull_height` makes it.
fn least_hull() -> Vec2 {
    vec2(PAGE_LEAST * pages::UNIT_SIZE.0 + 12.0, (PAGE_LEAST * pages::UNIT_SIZE.1 + 12.0) * 130.0 / 120.0)
}

/// A screen set into the dashboard: its bezel, dark glass and scan lines (none of them on a
/// modelled cockpit's own glass, `bare`, which has its own). Returns the glass.
fn screen(painter: &Painter, r: Rect, on: f32, bare: bool) -> Rect {
    if bare {
        return r.shrink(r.height() * 0.04);
    }
    painter.rect_filled(r.expand(5.0), 4.0, fade(METAL_DEEP, on));
    painter.rect_stroke(r.expand(5.0), 4.0, Stroke::new(1.2, fade(METAL_LIT, on * 0.8)), egui::StrokeKind::Inside);
    painter.rect_filled(r, 2.0, fade(SCREEN, on));
    let mut y = r.top() + 1.0;
    while y < r.bottom() {
        painter.line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, Color32::from_black_alpha((50.0 * on) as u8)));
        y += 3.0;
    }
    r.shrink(4.0)
}

/// A painted round instrument in its slot: its middle and the radius of its face, its bezel 6
/// round that standing on the slot's foot (the speed dial and the radar alike, side by side).
fn dial(r: Rect) -> (Pos2, f32) {
    let side = r.width().min(r.height());
    (pos2(r.center().x, r.bottom() - side * 0.5), side * 0.5 - 6.0)
}

/// The drive page painted over the view (a periscope's, a view from behind, or a screen of a
/// cockpit that is still coming in): the speed in figures on a round plate.
fn speed_gauge(painter: &Painter, r: Rect, p: &Perspective, rd: &Readouts, on: f32, bare: bool) {
    if p.cockpit.flies() || !rd.mobile {
        return;
    }
    let Some((c, rad, k)) = drive_on(r, bare, most_over(painter)) else { return };
    if !bare {
        painter.circle_filled(c, rad + 6.0, fade(METAL_DEEP, on));
        painter.circle_stroke(c, rad + 6.0, Stroke::new(2.0, fade(METAL_LIT, on)));
        painter.circle_filled(c, rad, fade(Color32::from_rgb(18, 18, 15), on));
    }
    pages::drive(&Pen::new(painter, k, false, on), c, rd);
}

/// An aircraft's attitude: the horizon on a ball, turning with the bank and rising and falling
/// with the nose.
fn attitude(painter: &Painter, r: Rect, p: &Perspective, on: f32, bare: bool) {
    let (c, rad) = if bare { (r.center(), r.width().min(r.height()) * 0.47) } else { dial(r) };
    if !(rad.is_finite() && rad >= 8.0) {
        return;
    }
    if !bare {
        painter.circle_filled(c, rad + 6.0, fade(METAL_DEEP, on));
    }
    // The ball: sky above the horizon, ground below, drawn as a disc cut by the horizon line.
    let pitch = p.forward.y.clamp(-1.0, 1.0).asin();
    let roll = p.roll;
    let shift = (pitch / 0.8).clamp(-1.0, 1.0) * rad * 0.8;
    let (s, co) = roll.sin_cos();
    // The ground over the whole ball, then the sky: the ball's rim on the sky side of the
    // tilted horizon (screen y grows down), as a fan from the horizon's middle.
    painter.circle_filled(c, rad, fade(Color32::from_rgb(52, 36, 20), on));
    let horizon_dir = vec2(co, -s);
    let mid = c + vec2(s, co) * shift;
    let mut mesh = Mesh::default();
    let sky_col = fade(Color32::from_rgb(40, 96, 150), on);
    let steps = 64;
    for k in 0..steps {
        let a0 = k as f32 / steps as f32 * std::f32::consts::TAU;
        let a1 = (k + 1) as f32 / steps as f32 * std::f32::consts::TAU;
        let q0 = c + vec2(a0.cos(), a0.sin()) * rad;
        let q1 = c + vec2(a1.cos(), a1.sin()) * rad;
        let side = |q: Pos2| (q - c).x * s + (q - c).y * co - shift;
        if side(q0) < 0.0 && side(q1) < 0.0 {
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(mid, sky_col);
            mesh.colored_vertex(q0, sky_col);
            mesh.colored_vertex(q1, sky_col);
            mesh.add_triangle(base, base + 1, base + 2);
        }
    }
    painter.add(Shape::mesh(mesh));
    painter.line_segment([mid - horizon_dir * rad * 0.95, mid + horizon_dir * rad * 0.95], Stroke::new(1.6, fade(Color32::WHITE, on)));
    // The pitch marks, and the plane's own symbol fixed in the middle.
    for k in [-2.0f32, -1.0, 1.0, 2.0] {
        let m = mid + vec2(s, co) * (k * rad * 0.2);
        painter.line_segment([m - horizon_dir * rad * 0.18, m + horizon_dir * rad * 0.18], Stroke::new(1.0, fade(Color32::from_gray(210), on)));
    }
    painter.line_segment([c + vec2(-rad * 0.5, 0.0), c + vec2(-rad * 0.15, 0.0)], Stroke::new(2.4, fade(AMBER_BRIGHT, on)));
    painter.line_segment([c + vec2(rad * 0.15, 0.0), c + vec2(rad * 0.5, 0.0)], Stroke::new(2.4, fade(AMBER_BRIGHT, on)));
    painter.circle_filled(c, 2.5, fade(AMBER_BRIGHT, on));
}

/// The Commander's economy (its side's): energy (the D-gun draws on it) and metal, as bars
/// against storage.
fn economy(painter: &Painter, r: Rect, game: &Game, owner: u8, on: f32) {
    let Some(pl) = game.sim.players.get(owner as usize) else { return };
    if !r.is_positive() || !r.is_finite() {
        return;
    }
    let e = &pl.economy;
    let k = (r.height() / 80.0).clamp(0.6, 1.5);
    let rows = [(t("ENERGY"), e.energy.to_f32(), e.energy_capacity.to_f32(), AMBER), (t("METAL"), e.metal.to_f32(), e.metal_capacity.to_f32(), Color32::from_gray(210))];
    for (i, (label, have, max, colour)) in rows.iter().enumerate() {
        let y = r.top() + 6.0 * k + i as f32 * r.height() * 0.48;
        painter.text(pos2(r.left() + 6.0 * k, y), Align2::LEFT_TOP, *label, mono(13.0 * k), fade(PHOSPHOR, on));
        painter.text(pos2(r.right() - 6.0 * k, y), Align2::RIGHT_TOP, format!("{:.0}", have), mono(13.0 * k), fade(*colour, on));
        let bar = Rect::from_min_size(pos2(r.left() + 6.0 * k, y + 18.0 * k), vec2(r.width() - 12.0 * k, 8.0 * k));
        painter.rect_filled(bar, 1.0, fade(PHOSPHOR_DIM, on * 0.5));
        let frac = (have / max.max(1.0)).clamp(0.0, 1.0);
        painter.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac, bar.height())), 1.0, fade(*colour, on));
    }
}

/// The height a painted status panel's rows need (points), lettered at `scale`: its heading, and
/// a row each weapon.
fn status_height(weapons: usize, scale: f32) -> f32 {
    8.0 + 22.0 * scale + 36.0 * scale * weapons as f32
}

/// The height of a weapon's row on a painted status panel, in its lettering's unit: its tag, its
/// name and its reload bar under them (its reach and reload under that where there is room, to
/// 46).
const STATUS_ROW: f32 = 31.0;

/// The painted panels' heights along the picture's foot (points), the unit's beside a slot
/// `hull` tall, and its weapons' with a row for each of `weapons`: brought to one height, the
/// taller of what each needs; the weapons' rows taller still where they need it (a mod's unit
/// carries up to eight), up to three tenths of the `picture`'s height. And the lettering's unit.
fn panel_heights(hull: f32, weapons: usize, picture: f32) -> (f32, f32, f32) {
    let scale = (hull / 130.0).clamp(0.6, 1.5);
    let rows = status_height(weapons, scale);
    let need = hull_height(scale).max(rows).min(hull);
    (need, rows.min(picture * 0.3).max(need), scale)
}

/// A status panel `height` points tall with a row for each of `weapons` (`bare`: on a modelled
/// screen, sharing its height; else lettered at `scale`): its heading's height, a row's and its
/// lettering's unit, smaller where its rows need more room than it has.
fn status_rows(height: f32, weapons: usize, bare: bool, scale: f32) -> (f32, f32, f32) {
    if bare {
        let row = (height - 8.0 - 24.0) / weapons.max(2) as f32;
        (24.0, row, (row / 32.0).min(1.5))
    } else {
        let room = (height - 8.0 - 22.0 * scale) / weapons.max(1) as f32;
        (18.0 * scale, room.min(50.0 * scale), scale.min(room / STATUS_ROW))
    }
}

/// What the unit is doing, then its weapons: a lamp and a reload bar each. `scale`: a painted
/// panel's lettering (on a modelled screen, in proportion to the screen).
#[allow(clippy::too_many_arguments)]
fn status_panel(painter: &Painter, r: Rect, p: &Perspective, game: &Game, rd: &Readouts, on: f32, now: f64, bare: bool, scale: f32) {
    let Some(u) = game.sim.unit(p.unit) else { return };
    let weapons = &rd.weapons;
    // A modelled screen shares its height between the rows; a painted panel (as tall as the
    // hull's beside it, or taller, see `draw`) has rows of its lettering's height, in the middle
    // of the room under its heading, with their reach and reload where there is room for them:
    // lettered smaller where its rows need more room than it has (a mod's unit carries up to
    // eight weapons).
    let (head, row, k) = status_rows(r.height(), weapons.len(), bare, scale);
    if !(r.is_finite() && k.is_finite() && k >= LEAST && r.width() > 40.0 * k) {
        return;
    }
    let glass = screen(painter, r, on, bare);
    let (doing, colour) = doing(game, u, p);
    let head_font = if bare { mono(13.0) } else { mono(12.0 * k) };
    if weapons.is_empty() && !bare {
        // Nothing to fire: what it is doing, in the middle (and how far along a build is).
        let mut job = egui::text::LayoutJob::simple(doing.to_uppercase(), mono(14.0 * k), fade(on_screen(colour), on), glass.width() - 8.0);
        job.halign = egui::Align::Center;
        let galley = painter.layout_job(job);
        let bar = u.build.and_then(|job| game.sim.unit(job.target)).map(|work| {
            let wt = game.sim.unit_type(work);
            if work.is_complete() { (work.health / work.max_health(wt).max(Fx::ONE)).to_f32() } else { work.build_progress.to_f32() }
        });
        let bar_h = if bar.is_some() { 14.0 * k } else { 0.0 };
        let top = glass.center().y - (galley.size().y + bar_h) * 0.5;
        let below = top + galley.size().y;
        painter.galley(pos2(glass.center().x, top), galley, fade(on_screen(colour), on));
        if let Some(done) = bar {
            let bar = Rect::from_min_size(pos2(glass.left() + 8.0 * k, below + 6.0 * k), vec2(glass.width() - 16.0 * k, 7.0 * k));
            painter.rect_filled(bar, 1.5, fade(PHOSPHOR_DIM, on * 0.5));
            painter.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * done.clamp(0.0, 1.0), bar.height())), 1.5, fade(GREEN, on));
        }
        return;
    }
    painter.text(pos2(glass.left() + 2.0, glass.top() + 2.0), Align2::LEFT_TOP, fitted(painter, doing.to_uppercase(), &head_font, glass.width() - 4.0), head_font, fade(on_screen(colour), on));
    let first = glass.top() + head + if bare { 4.0 * k } else { (glass.height() - head - row * weapons.len() as f32) * 0.5 };
    for (i, w) in weapons.iter().enumerate() {
        let y = first + i as f32 * row;
        // (The D-gun wants energy: each such weapon its own.)
        let starved = w.starved;
        let fired = fired(p, w, now);
        let colour = if starved { RED } else if w.ready { GREEN } else { AMBER };
        // Its number on a tag (the one beside its arc round the reticle), lit while it is ready.
        let tag = Rect::from_center_size(pos2(glass.left() + 10.0 * k, y + 9.0 * k), vec2(14.0 * k, 16.0 * k));
        painter.rect_filled(tag, 2.0 * k, fade(colour, on * if w.ready || fired || starved { 0.95 } else { 0.4 }));
        painter.text(tag.center(), Align2::CENTER_CENTER, format!("{}", i + 1), mono(12.0 * k), fade(Color32::from_rgb(10, 18, 8), on));
        // Its state in a word; its name in what room is left.
        let state = if starved { t("NO ENERGY") } else if w.ready { t("READY") } else { t("LOADING") };
        let state_width = painter.layout_no_wrap(state.to_string(), mono(12.0 * k), colour).size().x;
        let label = fitted(painter, w.name.to_uppercase(), &mono(15.0 * k), glass.width() - 2.0 - 34.0 * k - state_width);
        painter.text(pos2(glass.left() + 24.0 * k, y), Align2::LEFT_TOP, label, mono(15.0 * k), fade(PHOSPHOR, on));
        painter.text(pos2(glass.right() - 2.0, y + 2.0 * k), Align2::RIGHT_TOP, state, mono(12.0 * k), fade(colour, on));
        let bar = Rect::from_min_size(pos2(glass.left() + 24.0 * k, y + 24.0 * k), vec2(glass.width() - 30.0 * k, 7.0 * k));
        painter.rect_filled(bar, 1.5, fade(PHOSPHOR_DIM, on * 0.5));
        painter.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * (1.0 - w.left), bar.height())), 1.5, fade(colour, on));
        // Its reach (metres) and its reload, where the row has room for them.
        if row > 46.0 * k {
            let range = metres(w.range).round() as i32;
            let detail = if w.manual && w.cost > 0.0 {
                tf("RANGE {range} M · {energy} ENERGY A SHOT", &[("range", &range), ("energy", &(w.cost.round() as i32))])
            } else {
                tf("RANGE {range} M · RELOAD {seconds} S", &[("range", &range), ("seconds", &format!("{:.1}", w.reload))])
            };
            let font = mono(11.5 * k);
            painter.text(pos2(glass.left() + 24.0 * k, y + 36.0 * k), Align2::LEFT_TOP, kit::fit(painter, &detail, &font, glass.width() - 26.0 * k), font, fade(PHOSPHOR, on * 0.65));
        }
    }
    if rd.stockpile > 0 || weapons.iter().any(|w| w.stockpile) {
        painter.text(pos2(glass.right() - 2.0, glass.bottom() - 2.0), Align2::RIGHT_BOTTOM, tf("MISSILES {n}", &[("n", &rd.stockpile)]), mono(14.0 * k.min(1.0)), fade(AMBER, on));
    }
}

/// A radar scope: what the player's side sees round the unit, the view's way up (what radar
/// alone sees, a plain dim blip).
fn radar(painter: &Painter, r: Rect, p: &Perspective, game: &Game, on: f32, bare: bool) {
    let (c, rad) = if bare { (r.center(), r.width().min(r.height()) * 0.5 - 2.0) } else { dial(r) };
    if !(rad.is_finite() && rad >= 8.0) {
        return;
    }
    if !bare {
        painter.circle_filled(c, rad + 6.0, fade(METAL_DEEP, on));
        painter.circle_stroke(c, rad + 6.0, Stroke::new(2.0, fade(METAL_LIT, on)));
        painter.circle_filled(c, rad, fade(SCREEN, on));
    }
    for k in 1..=2 {
        painter.circle_stroke(c, rad * k as f32 / 3.0, Stroke::new(1.1, fade(PHOSPHOR_DIM, on)));
    }
    painter.line_segment([pos2(c.x - rad, c.y), pos2(c.x + rad, c.y)], Stroke::new(0.9, fade(PHOSPHOR_DIM, on)));
    painter.line_segment([pos2(c.x, c.y - rad), pos2(c.x, c.y + rad)], Stroke::new(0.9, fade(PHOSPHOR_DIM, on)));
    // The sweep and its afterglow.
    sweep(painter, c, rad, p.hud.sweep, 1.5, on);
    let Some(me) = game.sim.unit(p.unit) else { return };
    let here = Vec3::from_array(me.pos.to_f32());
    let range = 1400.0;
    let look = p.forward.x.atan2(p.forward.z);
    let (s, co) = look.sin_cos();
    for u in game.sim.units() {
        if u.id == p.unit || u.dead || u.carried_by.is_some() || game.seen(u) == Seen::Hidden {
            continue;
        }
        let d = Vec3::from_array(u.pos.to_f32()) - here;
        if d.x.abs() > range || d.z.abs() > range {
            continue;
        }
        // Screen up is the view's way.
        let ahead = d.x * s + d.z * co;
        let right = d.x * -co + d.z * s;
        let q = c + vec2(right, -ahead) / range * rad;
        if q.distance(c) > rad - 2.0 {
            continue;
        }
        let enemy = game.sim.is_enemy(me.owner, u.owner);
        // (Radar alone: a blip, no more.)
        if game.seen(u) == Seen::Radar {
            painter.circle_filled(q, 2.6, fade(AMBER_DIM, on * 0.9));
            continue;
        }
        let colour = if enemy { RED } else { PHOSPHOR };
        let air = game.sim.unit_type(u).locomotion == Locomotion::Air;
        if air {
            painter.add(Shape::convex_polygon(vec![q + vec2(0.0, -5.0), q + vec2(4.5, 3.5), q + vec2(-4.5, 3.5)], fade(colour, on), Stroke::NONE));
        } else {
            painter.circle_filled(q, if enemy { 3.6 } else { 2.6 }, fade(colour, on));
        }
    }
    // The hull's heading.
    let h = me.heading as f32 * std::f32::consts::TAU / 65536.0 - look;
    painter.add(Shape::convex_polygon(vec![c + vec2(-h.sin(), -h.cos()) * 7.0, c + vec2(-(h + 2.5).sin(), -(h + 2.5).cos()) * 5.0, c + vec2(-(h - 2.5).sin(), -(h - 2.5).cos()) * 5.0], fade(AMBER_BRIGHT, on), Stroke::NONE));
}

/// A radar's sweep round `c` out to `rad`, its beam at `beam` (radians): the beam, and behind it
/// the phosphor's afterglow fading away as one fan (strokes side by side read as a comb of
/// spokes), its rim softened by a pixel.
fn sweep(painter: &Painter, c: Pos2, rad: f32, beam: f32, width: f32, on: f32) {
    // (How far round the afterglow reaches behind the beam (radians), in how many slices.)
    const TRAIL: f32 = 0.65;
    const SLICES: usize = 32;
    let glow = |f: f32| fade(PHOSPHOR, on * 0.3 * (1.0 - f) * (1.0 - f));
    let dir = |a: f32| vec2(a.cos(), a.sin());
    let mut mesh = Mesh::default();
    for k in 0..SLICES {
        let (f0, f1) = (k as f32 / SLICES as f32, (k + 1) as f32 / SLICES as f32);
        let (d0, d1) = (dir(beam - f0 * TRAIL), dir(beam - f1 * TRAIL));
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(c, glow((f0 + f1) * 0.5));
        mesh.colored_vertex(c + d0 * rad, glow(f0));
        mesh.colored_vertex(c + d1 * rad, glow(f1));
        mesh.colored_vertex(c + d0 * (rad + 1.0), Color32::TRANSPARENT);
        mesh.colored_vertex(c + d1 * (rad + 1.0), Color32::TRANSPARENT);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 4);
        mesh.add_triangle(base + 1, base + 4, base + 2);
    }
    painter.add(Shape::mesh(mesh));
    painter.line_segment([c, c + dir(beam) * rad], Stroke::new(width, fade(PHOSPHOR, on * 0.7)));
}

/// The height a painted hull panel needs (points), lettered at `scale`.
fn hull_height(scale: f32) -> f32 {
    120.0 * scale
}

/// The unit's page painted over the view (see `speed_gauge`): its picture, name and kind, hull
/// and armour, rank, kills and experience, as big as its screen allows (on a modelled screen, in
/// proportion to the screen). The picture is the unit drawn from its model (`portrait`, given its
/// size), else TA's.
#[allow(clippy::too_many_arguments)]
fn integrity(painter: &Painter, r: Rect, p: &Perspective, game: &Game, rd: &Readouts, on: f32, now: f64, portrait: &mut dyn FnMut(f32) -> Option<egui::TextureId>, pics: &mut rustc_hash::FxHashMap<usize, Option<egui::TextureHandle>>, vfs: &ta_vfs::Vfs, ctx: &egui::Context, bare: bool) {
    let Some(u) = game.sim.unit(p.unit) else { return };
    let ty = game.sim.unit_type(u);
    // (The page is laid out for a box of a set size, as big as the screen allows: none on a
    // screen of no size.)
    let Some((frame, k)) = unit_page_on(r, bare, most_over(painter)) else { return };
    screen(painter, r, on, bare);
    // (The hull's alarm blinks; steady, with flashes reduced.)
    let alarm = p.hud.health < 0.35 && (p.reduced_flashes || (now * 3.0).fract() < 0.5);
    // Green, as on the cockpit's screen; red when hurt badly.
    let tint = if alarm { Color32::from_rgb(255, 120, 100) } else { Color32::from_rgb(170, 255, 150) };
    let picture = match portrait(58.0 * k) {
        Some(id) => Some((id, tint)),
        None => pics
            .entry(p.type_id)
            .or_insert_with(|| {
                let name = ty.name.to_ascii_lowercase();
                let pcx = ta_formats::pcx::PcxImage::parse(&vfs.read_opt(&format!("unitpics/{name}.pcx"))?).ok()?;
                let img = egui::ColorImage::from_rgba_unmultiplied([pcx.width as usize, pcx.height as usize], &pcx.to_rgba());
                Some(ctx.load_texture(format!("cockpit-pic-{name}"), img, egui::TextureOptions::LINEAR))
            })
            .clone()
            .map(|tex| (tex.id(), fade(tint, 0.8))),
    };
    let kill_flash = p.hud.kill_flash.is_some_and(|k| now - k < 1.2);
    let armour = walks(p.cockpit).then_some(rd.armour);
    pages::unit(&Pen::new(painter, k, false, on), frame, rd, &pages::Face { picture, armour, alarm, kill_flash });
}

// ---- Events --------------------------------------------------------------------------------------

/// The link's line (the side and the unit), its font, and where it is while it shows.
fn boot_line(rect: Rect, p: &Perspective, since: f32, side: &str, game: &Game) -> Option<(String, FontId, Pos2)> {
    if p.leaving || since > 1.8 {
        return None;
    }
    let name = game.sim.types.get(p.type_id).def.display_name.to_uppercase();
    Some((format!("{} · {}", side.to_uppercase(), name), FontId::new(22.0 * lettering(rect), egui::FontFamily::Name(crate::theme::STRONG_FAMILY.into())), rect.center() - vec2(0.0, rect.height() * 0.12)))
}

/// The link coming up: the unit typed out. (The cockpit's own lamps and screens come on as it
/// closes in round the eye; a line swept down the whole screen over a black veil, frame and
/// all, read as a fault in the picture.)
fn boot(painter: &Painter, rect: Rect, p: &Perspective, since: f32, side: &str, game: &Game) {
    let Some((line, font, at)) = boot_line(rect, p, since, side, game) else { return };
    let shown: String = line.chars().take(((since - 0.4).max(0.0) * 30.0) as usize).collect();
    let a = (1.0 - ((since - 1.3) / 0.5).clamp(0.0, 1.0)).clamp(0.0, 1.0);
    painter.text(at, Align2::CENTER_CENTER, shown, font, fade(PHOSPHOR, a));
}

/// Where the view's keys may stand ([`hints`]): low, ending over `above` (the minimalist view's
/// radar) or on the glass's lower part, and else from `high` down (under the window's top and
/// what is written along it), clear of `avoid` (the sight's marks) and inside `across` (the
/// window), all in points; and how far they have come on (`alpha`: the view's leaving).
struct KeysRoom<'a> {
    above: Option<f32>,
    high: f32,
    avoid: &'a [Rect],
    across: Rect,
    alpha: f32,
}

/// The view's keys, shown for a few seconds once the link's message has gone (it stands where they
/// do), and again for a while when the player takes control of the unit or lets it go: riding in
/// one of their own that they may drive, E among them; driving, the keys that drive it.
/// `TA_PERSPECTIVE_HINTS=0` leaves them out, for captures; a benchmark's ride has none: nothing
/// there takes them. Low on the glass where they leave the sight's marks clear, else in one row
/// there, else high in the window (a gun house's slit, a ship's bridge, a Krogoth's cab and an
/// aircraft's symbology leave no room under the sight): where they first stood while they show.
/// Returns the room they take while they show.
fn hints(painter: &Painter, rect: Rect, p: &Perspective, since: f32, game: &Game, now: f64, place: KeysRoom) -> Option<Rect> {
    use crate::perspective::{KEYS_FROM as FROM, KEYS_TO as TO};
    // (After E, or a pad taken up: at once, for a little less long.)
    const TOGGLED_FOR: f32 = 7.5;
    static HIDDEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if place.alpha <= 0.0 || crate::benchmark::running() || *HIDDEN.get_or_init(|| std::env::var("TA_PERSPECTIVE_HINTS").is_ok_and(|v| v == "0")) {
        return None;
    }
    // (A pixel of a 1080-line picture.)
    let u = lettering(rect);
    let toggled = p.keys_again.filter(|&at| (0.0..TOGGLED_FOR).contains(&((now - at) as f32)));
    let a = match toggled {
        Some(at) => {
            let t = (now - at) as f32;
            (t / 0.3).clamp(0.0, 1.0) * (1.0 - ((t - (TOGGLED_FOR - 1.5)) / 1.5).clamp(0.0, 1.0))
        }
        None if (FROM..TO).contains(&since) => ((since - FROM) / 0.5).clamp(0.0, 1.0) * (1.0 - ((since - (TO - 1.5)) / 1.5).clamp(0.0, 1.0)),
        None => return None,
    } * place.alpha.min(1.0);
    // (A pad's buttons while it is what the player is using.)
    let text = match (p.pad, p.controlling, p.may_control(game)) {
        (false, true, _) => t("W S DRIVES · A D TURNS · CLICK FIRES · E LETS GO · ESC LEAVES"),
        (false, false, true) => t("MOUSE LOOKS ROUND · WHEEL ZOOM · V VIEW · N NIGHT · E TAKES CONTROL · ESC LEAVES"),
        (false, false, false) => t("MOUSE LOOKS ROUND · WHEEL ZOOM · V VIEW · N NIGHT · ESC LEAVES"),
        (true, true, _) => t("LEFT STICK DRIVES · RIGHT STICK AIMS · RB FIRES · Y LETS GO · START LEAVES"),
        (true, false, true) => t("RIGHT STICK LOOKS ROUND · LT RT ZOOM · X VIEW · LB NIGHT · Y TAKES CONTROL · START LEAVES"),
        (true, false, false) => t("RIGHT STICK LOOKS ROUND · LT RT ZOOM · X VIEW · LB NIGHT · START LEAVES"),
    };
    // Each key on a plate of its own ("V VIEW"), in rows centred across the picture; they come up
    // one after another.
    let chips: Vec<_> = text.split(" · ").map(|s| painter.layout_no_wrap(s.trim().to_string(), mono(14.0 * u), AMBER)).collect();
    let (pad, gap, row_gap) = (vec2(11.0, 5.0) * u, 8.0 * u, 7.0 * u);
    let wide = |i: usize| chips[i].size().x + 2.0 * pad.x;
    let h = chips.first().map_or(16.0 * u, |g| g.size().y) + 2.0 * pad.y;
    // In rows no wider than `room`, as many as it takes, as evenly filled as they can be (no word
    // left on its own).
    let lay = |room: f32| {
        let total: f32 = (0..chips.len()).map(|i| wide(i) + gap).sum::<f32>() - gap;
        let target = (total / (total / room).ceil().max(1.0)).min(room);
        let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
        let mut used = 0.0;
        for i in 0..chips.len() {
            let w = wide(i);
            if used > 0.0 && (used + gap + w > room || used + gap + w / 2.0 > target) {
                rows.push(Vec::new());
                used = 0.0;
            }
            used += if used > 0.0 { gap } else { 0.0 } + w;
            if let Some(r) = rows.last_mut() {
                r.push(i);
            }
        }
        rows
    };
    let row_width = |row: &[usize]| row.iter().map(|&i| wide(i)).sum::<f32>() + gap * row.len().saturating_sub(1) as f32;
    let tall = |rows: &[Vec<usize>]| rows.len() as f32 * (h + row_gap) - row_gap;
    // Low: ending over `above`, else round two thirds of the way down; in a modelled cockpit over
    // its sill (where the dashboard stands higher, a ship's bridge's, they lay over its lamps).
    let sill = p.sill_y.filter(|_| p.model.is_some() && !p.minimalist && p.seat > 0.02 && !p.chase && p.zoom == 0).map(|y| y / painter.ctx().pixels_per_point());
    let low = |tall: f32| {
        let y = place.above.map_or(rect.top() + rect.height() * 0.66 - tall / 2.0, |bottom| bottom - tall);
        match sill {
            Some(sill) => y.min(sill - 12.0 * u - tall).max(rect.top() + rect.height() * 0.4),
            None => y,
        }
    };
    // (In rows no wider than the middle of the picture, in proportion to its height; else in one
    // row across the window.)
    let compact = lay((rect.height() * 0.53).max(440.0 * u).min(rect.width() * 0.82));
    let one_row = lay((place.across.width() - 32.0 * u).max(1.0));
    let block = |rows: &[Vec<usize>], y: f32| {
        let width = rows.iter().map(|r| row_width(r)).fold(0.0, f32::max);
        Rect::from_min_size(pos2(rect.center().x - width / 2.0, y), vec2(width, tall(rows)))
    };
    let candidates = [(compact.clone(), low(tall(&compact))), (one_row.clone(), low(tall(&one_row))), (compact.clone(), place.high)];
    let clear = |r: Rect| !place.avoid.iter().any(|a| a.intersects(r));
    // (Where they stood since they came up, so that they do not jump about as the marks move.)
    let shown_since = toggled.unwrap_or(p.hud.entered + FROM as f64);
    let choice = match p.hud.keys_at.get() {
        Some((at, choice)) if at == shown_since => choice,
        _ => {
            let choice = candidates.iter().position(|(rows, y)| clear(block(rows, *y))).unwrap_or(0);
            p.hud.keys_at.set(Some((shown_since, choice)));
            choice
        }
    };
    let (rows, mut y) = (&candidates[choice].0, candidates[choice].1);
    let mut room = Rect::NOTHING;
    for row in rows {
        let mut x = rect.center().x - row_width(row) / 2.0;
        for &i in row {
            let g = &chips[i];
            let k = (a * 4.0 - i as f32 * 0.25).clamp(0.0, 1.0).min(a);
            let chip = Rect::from_min_size(pos2(x, y), vec2(wide(i), h));
            painter.rect_filled(chip, 3.0 * u, Color32::from_black_alpha((175.0 * k) as u8));
            painter.rect_stroke(chip, 3.0 * u, egui::Stroke::new(u.max(1.0), fade(AMBER, 0.45 * k)), egui::StrokeKind::Inside);
            painter.galley(chip.min + pad, g.clone(), fade(AMBER, k));
            room = room.union(chip);
            x += chip.width() + gap;
        }
        y += h + row_gap;
    }
    Some(room)
}

/// The unit is gone: the picture breaks up into static, and the link is lost; all of it going
/// (`staying`) as the camera climbs back out. `now` none (still), with flashes or motion reduced.
fn lost_signal(painter: &Painter, rect: Rect, age: f32, now: f64, staying: f32) {
    let k = (age / 0.35).clamp(0.0, 1.0) * staying.clamp(0.0, 1.0);
    if k <= 0.0 {
        return;
    }
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha((230.0 * k) as u8));
    let frame = (now * 30.0) as u32;
    let cells = 90;
    let cw = rect.width() / cells as f32;
    let rows = (rect.height() / cw).ceil() as u32;
    for y in 0..rows {
        for x in 0..cells {
            let h = hash(frame.wrapping_mul(2_654_435_761) ^ (y * 977 + x as u32 * 131));
            if !h.is_multiple_of(3) {
                continue;
            }
            let g = (h >> 8) as u8 % 200;
            painter.rect_filled(Rect::from_min_size(pos2(rect.left() + x as f32 * cw, rect.top() + y as f32 * cw), vec2(cw, cw)), 0.0, Color32::from_gray(g).gamma_multiply(0.55 * k));
        }
    }
    // A rolling bar.
    let bar_y = rect.top() + ((now * 0.6).fract() as f32) * rect.height();
    painter.rect_filled(Rect::from_min_size(pos2(rect.left(), bar_y), vec2(rect.width(), rect.height() * 0.08)), 0.0, Color32::from_white_alpha((25.0 * k) as u8));
    if (now * 2.5).fract() < 0.7 {
        // (On a plate round its words, as long as they are in the language shown, in proportion to
        // the picture.)
        let u = lettering(rect);
        let galley = painter.layout_no_wrap(t("SIGNAL LOST").to_string(), FontId::new(34.0 * u, egui::FontFamily::Name(crate::theme::STRONG_FAMILY.into())), RED);
        let plate = Rect::from_center_size(rect.center(), galley.size() + vec2(140.0, 24.0) * u);
        painter.rect_filled(plate, 3.0 * u, Color32::from_black_alpha((200.0 * k) as u8));
        painter.galley(plate.center() - galley.size() * 0.5, galley, fade(RED, k));
    }
}

/// A rectangle on whole pixels: the instruments drawn on the cockpit's screens move with it, and
/// lettering placed between pixels shimmers as it does.
fn snapped(r: Rect) -> Rect {
    Rect::from_min_max(pos2(r.min.x.round(), r.min.y.round()), pos2(r.max.x.round(), r.max.y.round()))
}

/// Monospaced text `size` points tall made to fit `width`: lettered smaller, down to `least` of
/// that, then cut short. The text and its size.
fn shrunk_to(painter: &Painter, text: String, size: f32, width: f32, least: f32) -> (String, f32) {
    let wide = painter.layout_no_wrap(text.clone(), mono(size), Color32::WHITE).size().x.max(1.0);
    let size = size * (width / wide).clamp(least, 1.0);
    (fitted(painter, text, &mono(size), width), size)
}

/// Text cut short (an ellipsis at its end) to fit `width` points, as it is laid out.
fn fitted(painter: &Painter, text: String, font: &FontId, width: f32) -> String {
    kit::fit(painter, &text, font, width)
}

/// Figures projected into the sight: glowing, with a soft dark edge to read over bright ground,
/// and no plate. Returns the room they take.
fn projected(painter: &Painter, at: Pos2, align: Align2, text: String, font: FontId, colour: Color32) -> Rect {
    let halo = Color32::from_black_alpha((colour.a() as f32 * 0.5) as u8);
    // (The edge in proportion to the figures: 1.5 at 15 points.)
    let e = font.size / 10.0;
    for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0), vec2(0.7, 0.7), vec2(-0.7, 0.7), vec2(0.7, -0.7), vec2(-0.7, -0.7)] {
        painter.text(at + d * e, align, text.clone(), font.clone(), halo);
    }
    painter.text(at, align, text, font, colour).expand(e)
}

/// Text on a dark plate, for what is written over the world (the plate in proportion to it: 7 by
/// 3 round 18 points).
fn plated(painter: &Painter, at: Pos2, align: Align2, text: String, font: FontId, colour: Color32) {
    let galley = painter.layout_no_wrap(text.clone(), font.clone(), colour);
    let k = font.size / 18.0;
    let r = align.anchor_size(at, galley.size()).expand2(vec2(7.0, 3.0) * k);
    painter.rect_filled(r, 3.0 * k, Color32::from_black_alpha((colour.a() as f32 * 0.6) as u8));
    painter.text(at, align, text, font, colour);
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimalist_health_and_radar_stay_centered_inside_the_view() {
        for size in [vec2(320.0, 240.0), vec2(1920.0, 1080.0), vec2(3440.0, 1440.0)] {
            let rect = Rect::from_min_size(pos2(10.0, 30.0), size);
            let health = minimalist_health_rect(rect, lettering(rect));
            let radar = minimalist_radar_rect(rect);
            assert!(rect.contains_rect(health) && rect.contains_rect(radar));
            assert_eq!(health.center().x, rect.center().x);
            assert_eq!(radar.center().x, rect.center().x);
            assert_eq!(radar.bottom(), rect.bottom() - 12.0);
            assert!(health.bottom() < rect.center().y);
            assert!(radar.top() > rect.center().y);
        }
    }

    #[test]
    fn the_panoramic_displays_panes_stand_side_by_side_across_its_glass() {
        for (width, height, wide) in [(594.0, 128.0, true), (453.0, 88.0, false)] {
            let g = Rect::from_min_size(pos2(10.0, 20.0), vec2(width, height));
            let (panes, cuts) = pcd_panes(g, wide, 4.0);
            assert_eq!((panes[0].left(), panes[2].right()), (g.left(), g.right()));
            assert!(cuts[0] < cuts[1]);
            for pair in panes.windows(2) {
                assert!(pair[1].left() - pair[0].right() >= 7.9, "{pair:?}");
            }
            assert!(panes.iter().all(|pane| pane.width() > 60.0 && pane.height() == height), "{panes:?}");
            // The radar's pane is wide enough for its disc, the weapons' the broadest.
            assert!(panes[1].width() >= height && panes[2].width() > panes[0].width());
        }
    }

    /// The pictures the panels along the foot are laid out on: 16:9 at 1080 and 1440 lines and at
    /// 720, 21:9, 32:9 (two of them), 16:10, 4:3 and smaller.
    const PICTURES: [(f32, f32); 10] = [(1920.0, 1080.0), (2560.0, 1440.0), (1280.0, 720.0), (2560.0, 1080.0), (3840.0, 1080.0), (5120.0, 1440.0), (1280.0, 800.0), (1024.0, 768.0), (800.0, 600.0), (640.0, 480.0)];

    #[test]
    fn the_panels_along_the_foot_keep_apart_in_the_middle_of_the_picture_and_can_be_read() {
        for (w, h) in PICTURES {
            let picture = Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h));
            let band = foot_band(picture);
            assert!(band.width() <= h * 16.0 / 9.0 + 0.01 && picture.contains_rect(band));
            for cockpit in [Cockpit::Vehicle, Cockpit::Periscope] {
                let l = Layout::for_cockpit(cockpit, picture, true);
                let slots = [l.speed, l.radar, l.hull, l.status];
                for (i, a) in slots.iter().enumerate() {
                    // (On a 32:9 screen they stood at its ends, five thousand points apart.)
                    assert!(band.expand(0.01).contains_rect(*a), "{w}x{h}: {a:?} out of {band:?}");
                    assert!(slots[i + 1..].iter().all(|b| !a.intersects(*b)), "{w}x{h}: {slots:?}");
                }
                // Their pages lettered at 1.3 points to their unit at least, on a picture of 720
                // lines or more (at 1280x720 they were lettered at five or six points).
                if h >= 720.0 {
                    let (_, rad) = dial(l.speed);
                    assert!(rad / pages::DRIVE_RADIUS >= PAGE_LEAST - 1e-3, "{w}x{h}: {rad}");
                    let (need, _, _) = panel_heights(l.hull.height(), 1, h);
                    let glass = Rect::from_min_max(pos2(l.hull.left(), l.hull.bottom() - need), l.hull.max).shrink(4.0);
                    let k = fill_scale(glass.size() - Vec2::splat(4.0), vec2(pages::UNIT_SIZE.0, pages::UNIT_SIZE.1), 4.0);
                    assert!(k.is_some_and(|k| k >= PAGE_LEAST - 1e-3), "{w}x{h}: {k:?}");
                }
            }
        }
    }

    #[test]
    fn a_weapon_panel_fits_its_rows_however_many_weapons_there_are() {
        for (w, h) in PICTURES {
            let picture = Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h));
            let l = Layout::for_cockpit(Cockpit::Vehicle, picture, true);
            // (A mod's unit carries up to eight, in as many rows when they read differently.)
            for weapons in 1..=8 {
                let (need, tall, scale) = panel_heights(l.hull.height(), weapons, h);
                assert!(tall >= need && tall <= (h * 0.3).max(need) + 0.01, "{w}x{h} {weapons}: {need} {tall}");
                let (head, row, k) = status_rows(tall, weapons, false, scale);
                // Each row holds its tag, its name and its reload bar, lettered no larger than the
                // panel's; all of them under the heading, on the panel's glass (inside its bezel).
                assert!(k > 0.0 && k <= scale + 1e-4 && row >= STATUS_ROW * k - 1e-3, "{w}x{h} {weapons}: row {row}, k {k}");
                assert!(head + row * weapons as f32 <= tall - 8.0 + 1e-3, "{w}x{h} {weapons}");
            }
        }
    }

    #[test]
    fn a_page_on_a_screen_of_no_size_or_far_too_big_is_never_lettered_out_of_all_measure() {
        let ctx = egui::Context::default();
        crate::theme::install_compact(&ctx);
        ctx.begin_pass(egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0))), ..Default::default() });
        let painter = ctx.layer_painter(LayerId::background());
        let most = most_over(&painter);
        let rd = Readouts::sample();
        let face = pages::Face { picture: None, armour: None, alarm: false, kill_flash: false };
        let w = pages::UNIT_SIZE.0;
        // (A modelled cockpit's screens as it slides past the eye: of no size, inside out, not a
        // number, thousands of points across; and one as it should be.)
        let screens = [
            Rect::NOTHING,
            Rect::ZERO,
            Rect::EVERYTHING,
            Rect::from_min_size(pos2(5.0, 5.0), vec2(3.0, 2.0)),
            Rect::from_min_max(pos2(60.0, 60.0), pos2(10.0, 20.0)),
            Rect::from_min_size(pos2(f32::NAN, 0.0), vec2(100.0, 100.0)),
            Rect::from_min_size(pos2(-9000.0, -6000.0), vec2(30000.0, 20000.0)),
            Rect::from_min_size(pos2(100.0, 100.0), vec2(240.0, 160.0)),
        ];
        let mut pages_drawn = 0;
        for r in screens {
            // (Where `integrity` and `speed_gauge` lay theirs on a screen, a modelled cockpit's or a
            // painted panel's.)
            for bare in [true, false] {
                if let Some((frame, k)) = unit_page_on(r, bare, most) {
                    assert!(frame.is_finite() && (frame.width() - w * k).abs() < 0.01, "{r:?}: {frame:?}");
                    pages::unit(&Pen::new(&painter, k, false, 1.0), frame, &rd, &face);
                    pages_drawn += 1;
                }
                if let Some((c, _, k)) = drive_on(r, bare, most) {
                    pages::drive(&Pen::new(&painter, k, false, 1.0), c, &rd);
                    pages_drawn += 1;
                }
            }
            // (And a weapon panel's rows on it: lettered at a size, or not at all.)
            let (_, _, k) = status_rows(r.height(), 3, true, 1.0);
            assert!(!k.is_finite() || k < LEAST || k <= 1.5, "{r:?}: {k}");
        }
        let out = ctx.end_pass();
        let sizes: Vec<f32> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                Shape::Text(text) => Some(text.galley.job.sections.iter().map(|s| s.format.font_id.size).collect::<Vec<_>>()),
                _ => None,
            })
            .flatten()
            .collect();
        out.drop_without_applying_deltas();
        // The screen as it should be and the one far too big have their pages, either way; the rest
        // none.
        assert_eq!(pages_drawn, 8);
        assert!(!sizes.is_empty() && sizes.iter().all(|&s| s.is_finite() && s > 0.5 && s <= 30.0 * most), "{sizes:?}");
    }

    #[test]
    fn the_lost_signals_plate_is_round_its_words_whatever_the_picture() {
        for (w, h) in [(1280.0, 720.0), (2560.0, 1440.0), (640.0, 480.0)] {
            let ctx = egui::Context::default();
            crate::theme::install_compact(&ctx);
            let picture = Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h));
            ctx.begin_pass(egui::RawInput { screen_rect: Some(picture), ..Default::default() });
            // (Still, its words showing, all there.)
            lost_signal(&ctx.layer_painter(LayerId::background()), picture, 1.0, 0.0, 1.0);
            let out = ctx.end_pass();
            let shapes: Vec<&Shape> = out.shapes.iter().map(|s| &s.shape).collect();
            let at = shapes.iter().position(|s| matches!(s, Shape::Text(_))).expect("its words");
            let (Shape::Text(words), Shape::Rect(plate)) = (shapes[at], shapes[at - 1]) else { panic!("{:?}", &shapes[at - 1..=at]) };
            let words = Rect::from_min_size(words.pos, words.galley.size());
            assert!(plate.rect.contains_rect(words) && plate.rect.width() > words.width() * 1.2, "{w}x{h}: {words:?} on {:?}", plate.rect);
            // (Lettered in proportion to the picture, 34 a pixel of a 1080-line one.)
            let size = 34.0 * lettering(picture);
            assert!(words.height() > size * 0.9 && words.height() < size * 1.6, "{w}x{h}: {words:?}");
            out.drop_without_applying_deltas();
        }
    }

    #[test]
    fn every_screen_of_a_cockpit_has_a_name_to_power_up_with() {
        for region in ["pcd", "speed", "radar", "hull", "weapons", "economy", "compass"] {
            assert!(screen_name(region, false).is_some_and(|n| !n.is_empty() && n == n.to_uppercase()), "{region}");
        }
        assert_ne!(screen_name("speed", true), screen_name("speed", false));
        // (Never the region's own name, untranslated: none for one not known.)
        assert_eq!(screen_name("mfd_left", false), None);
    }

    #[test]
    fn the_periscopes_field_is_round_with_its_bearing_ring_in_the_picture() {
        for (w, h) in PICTURES {
            let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h));
            let (c, rad) = periscope_field(r);
            // (Its figures stand 34 past the rim, 11 tall, a pixel of a 1080-line picture each.)
            let ring = rad + 44.0 * lettering(r);
            assert!(c.y - ring >= r.top() && c.y + ring <= r.bottom() && c.x - ring >= r.left() && c.x + ring <= r.right(), "{w}x{h}: {c:?} {rad}");
            assert!(rad <= w * 0.29 + 0.01 && rad >= h * 0.2, "{w}x{h}: {rad}");
        }
    }

    #[test]
    fn the_fields_fan_on_the_radar_is_as_wide_as_the_picture() {
        let fov = 64f32.to_radians();
        // A tank's 64 degrees up and down see 48 either side across a 16:9 picture; wider
        // pictures further round, narrower and magnified less; a picture of no shape as 16:9.
        assert!((view_across(fov, 16.0 / 9.0).to_degrees() - 48.0).abs() < 0.5);
        assert!(view_across(fov, 32.0 / 9.0) > view_across(fov, 21.0 / 9.0) && view_across(fov, 21.0 / 9.0) > view_across(fov, 16.0 / 9.0));
        assert!(view_across(fov, 4.0 / 3.0) < view_across(fov, 16.0 / 9.0) && view_across(fov / ZOOMS[1], 16.0 / 9.0) < view_across(fov, 16.0 / 9.0));
        assert_eq!(view_across(fov, f32::NAN), view_across(fov, 16.0 / 9.0));
    }
}
