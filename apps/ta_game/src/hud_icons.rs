//! The modern HUD's order and state buttons and their icons. The icons are original art drawn by
//! `tools/hud/icons.py`: each glyph die-stamped with a bevel lit from the top left, in an atlas
//! with a row for each size the interface may draw it at (egui's wgpu renderer has no
//! mipmaps), so a glyph is drawn from the row nearest its size on screen, on whole pixels.

use egui::{Color32, Context, CursorIcon, Id, Painter, Pos2, Rect, Response, Sense, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2};
use ta_ui::metal::{self, Slab};

use crate::theme::*;

/// What an order or state button shows. (The order of the atlas's cells: `tools/hud/icons.py`'s
/// GLYPHS.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Stop,
    /// TA's MOVE: an arrow on its way.
    Move,
    Attack,
    Fight,
    Patrol,
    Guard,
    Capture,
    /// TA's REPAIR: a wrench.
    Repair,
    /// TA's LOAD: an arrow up into a crate (UNLOAD's the other way).
    Load,
    Unload,
    Resurrect,
    Reclaim,
    DGun,
    HoldFire,
    ReturnFire,
    FireAtWill,
    HoldPosition,
    Maneuver,
    Roam,
    Cloaked,
    Visible,
    /// On/off (metal makers, radar, shields...).
    Power,
    SelfDestruct,
    /// Extractors over an area: a dashed ring around a small extractor.
    AreaMex,
    /// Perspective: an eye with a sight for its pupil.
    Perspective,
    /// The radial menu's: a crane (what a builder builds), an "i" (information), the way back, the
    /// rest of the choices, and an X (leave it).
    Build,
    Info,
    Back,
    More,
    Cancel,
    /// A constructor's duty: a cog, work that goes on by itself.
    Duty,
    /// A factory's queue: a crate on a belt, on its way out.
    Queue,
}

#[cfg(test)]
impl Glyph {
    /// Every glyph, in the atlas's order.
    pub const ALL: [Glyph; 32] = [
        Glyph::Stop, Glyph::Move, Glyph::Attack, Glyph::Fight, Glyph::Patrol, Glyph::Guard, Glyph::Capture, Glyph::Repair, Glyph::Load, Glyph::Unload,
        Glyph::Resurrect, Glyph::Reclaim, Glyph::DGun, Glyph::HoldFire, Glyph::ReturnFire, Glyph::FireAtWill, Glyph::HoldPosition, Glyph::Maneuver,
        Glyph::Roam, Glyph::Cloaked, Glyph::Visible, Glyph::Power, Glyph::SelfDestruct, Glyph::AreaMex, Glyph::Perspective,
        Glyph::Build, Glyph::Info, Glyph::Back, Glyph::More, Glyph::Cancel, Glyph::Duty, Glyph::Queue,
    ];
}

/// The atlas: `tools/hud/icons.py` writes it (and its SIZES and PAD are these).
const ATLAS: &[u8] = include_bytes!("../hud/icons.png");
const SIZES: [u32; 10] = [14, 16, 18, 20, 24, 28, 32, 40, 48, 64];
const PAD: u32 = 2;

fn atlas(ctx: &Context) -> Option<TextureHandle> {
    let id = Id::new("hud_icons::atlas");
    if let Some(texture) = ctx.data(|d| d.get_temp::<Option<TextureHandle>>(id)) {
        return texture;
    }
    let texture = image::load_from_memory(ATLAS).ok().map(|img| {
        let img = img.to_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        ctx.load_texture("hud_icons", egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), TextureOptions::LINEAR)
    });
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    texture
}

/// Where a glyph of `size` pixels lies in the atlas (its row's top left).
fn cell(glyph: Glyph, row: usize) -> (u32, u32) {
    let y = SIZES[..row].iter().map(|s| s + 2 * PAD).sum::<u32>() + PAD;
    (glyph as u32 * (SIZES[row] + 2 * PAD) + PAD, y)
}

/// Paints `glyph` in `color` centred in `r` (square), from the atlas row nearest its size on
/// screen; within a fifth of it, at that row's own size, on whole pixels, so it stays sharp.
pub fn paint(painter: &Painter, r: Rect, glyph: Glyph, color: Color32) {
    let Some(texture) = atlas(painter.ctx()) else { return };
    let ppp = painter.ctx().pixels_per_point();
    let want = r.width().min(r.height()) * ppp;
    let row = (0..SIZES.len()).min_by(|&a, &b| (SIZES[a] as f32 / want).ln().abs().total_cmp(&(SIZES[b] as f32 / want).ln().abs())).unwrap_or(0);
    let px = SIZES[row] as f32;
    let (x, y) = cell(glyph, row);
    let [w, h] = texture.size();
    let uv = Rect::from_min_size(pos2(x as f32 / w as f32, y as f32 / h as f32), vec2(px / w as f32, px / h as f32));
    let side = if (px / want - 1.0).abs() <= 0.2 { px / ppp } else { want / ppp };
    let min = ((r.center() - Vec2::splat(side / 2.0)) * ppp).round() / ppp;
    painter.image(texture.id(), Rect::from_min_size(min, Vec2::splat(side)), uv, color);
}

/// [`paint`] with the dark edge below and to the right that TA's lettering has.
fn stamped(painter: &Painter, r: Rect, glyph: Glyph, color: Color32) {
    paint(painter, r.translate(Vec2::splat(1.0)), glyph, Color32::from_black_alpha(170));
    paint(painter, r, glyph, color);
}

/// The colour a glyph is lettered in on a steel or rust button.
fn ink(kind: Slab, hot: f32, active: bool) -> Color32 {
    if active { Color32::from_rgb(255, 246, 204) } else { kind.text(hot) }
}

/// An order button `width` wide (`compact`: a little lower): its glyph above, and below it the
/// key that gives the order on a small keycap ("F", or "ALT F" for Alt+F), so the keys are
/// learnt by looking. Pressed in and lit, with a lamp, while the order waits for its click.
pub fn order_button(ui: &mut Ui, glyph: Glyph, active: bool, key: &str, width: f32, height: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = if response.hovered() { 1.0 } else { 0.0 };
        let down = active || response.is_pointer_button_down_on();
        let painter = ui.painter();
        painter.add(metal::slab(ui.ctx(), rect, Slab::Steel, hot, down));
        let dy = if down { 1.0 } else { 0.0 };
        let color = ink(Slab::Steel, hot, active);
        let size = if height < 38.0 { 18.0 } else { 20.0 };
        stamped(painter, Rect::from_center_size(pos2(rect.center().x, rect.top() + size / 2.0 + 4.5 + dy), vec2(size, size)), glyph, color);
        keycap(painter, pos2(rect.center().x, rect.bottom() - 7.5 + dy), key, active, rect.width() - 3.0);
        if active {
            painter.add(metal::lamp(rect.right_top() + vec2(-5.5, 5.5), 2.2, GREEN, 1.0));
        }
    }
    if response.clicked() {
        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Button);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A key as a small keycap centred at `at`: its last key bright, the modifiers before it small
/// ("ALT F"), within `room` points (the modifiers go first when there is not room for them).
fn keycap(painter: &Painter, at: Pos2, key: &str, lit: bool, room: f32) {
    let mut parts: Vec<String> = key.split('+').map(|p| p.trim().to_uppercase()).filter(|p| !p.is_empty()).collect();
    let Some(last) = parts.pop() else { return };
    if last.chars().count() > 4 {
        return;
    }
    let bright = if lit { Color32::from_rgb(255, 238, 180) } else { Color32::from_rgb(222, 214, 188) };
    // (The modifier in a size and tone to read at a glance: Alt+F is not F.)
    let dim = if lit { Color32::from_rgb(226, 200, 128) } else { Color32::from_rgb(192, 184, 160) };
    let main = painter.layout_no_wrap(last, strong(9.5), bright);
    let mods = (!parts.is_empty()).then(|| painter.layout_no_wrap(parts.join(" "), strong(7.5), dim));
    let gap = 2.0;
    let width = |m: Option<&std::sync::Arc<egui::Galley>>| main.size().x + m.map_or(0.0, |m| m.size().x + gap) + 6.0;
    let mods = mods.filter(|m| width(Some(m)) <= room);
    let w = width(mods.as_ref()).max(14.0);
    let cap = Rect::from_center_size(at, vec2(w, 11.0));
    // A dark recess with a lighter lip below, as a key sunk into the plate.
    painter.rect_filled(cap.translate(vec2(0.0, 1.0)), 2.5, Color32::from_white_alpha(14));
    painter.rect_filled(cap, 2.5, if lit { Color32::from_rgb(52, 42, 18) } else { Color32::from_black_alpha(150) });
    painter.rect_stroke(cap, 2.5, egui::Stroke::new(1.0, Color32::from_black_alpha(200)), egui::StrokeKind::Inside);
    let mut x = cap.left() + (w - (width(mods.as_ref()) - 6.0)) / 2.0;
    if let Some(m) = mods {
        let (mw, mh) = (m.size().x, m.size().y);
        painter.galley(pos2(x, cap.center().y - mh / 2.0 + 0.5), m, dim);
        x += mw + gap;
    }
    let mh = main.size().y;
    painter.galley(pos2(x, cap.center().y - mh / 2.0), main, bright);
}

/// A standing order as a button with its glyph, `size` large; what it is set to on the pointer.
/// `lit` presses it in with its lamp (a cloak that is on); `warn` rusts it red (a countdown
/// running); `steps`, which of how many settings it is at, as a column of lamps at its right.
pub fn state_icon_button(ui: &mut Ui, glyph: Glyph, size: Vec2, lit: bool, warn: bool, steps: Option<(u8, u8)>) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = if response.hovered() { 1.0 } else { 0.0 };
        let down = lit || response.is_pointer_button_down_on();
        let kind = if warn { Slab::Rust } else { Slab::Steel };
        let painter = ui.painter();
        painter.add(metal::slab(ui.ctx(), rect, kind, hot, down));
        let c = rect.center() + vec2(if steps.is_some() { -3.0 } else { 0.0 }, if down { 1.0 } else { 0.0 });
        let glyph_size = (rect.height() - 8.0).clamp(14.0, 18.0);
        stamped(painter, Rect::from_center_size(c, vec2(glyph_size, glyph_size)), glyph, ink(kind, hot, lit));
        if let Some((at, of)) = steps {
            let spacing = ((rect.height() - 8.0) / f32::from(of.max(1))).min(6.0);
            let first = rect.center().y - spacing * f32::from(of.saturating_sub(1)) / 2.0;
            for k in 0..of {
                painter.add(metal::lamp(pos2(rect.right() - 5.0, first + spacing * f32::from(k)), 1.7, AMBER, if k == at { 1.0 } else { 0.0 }));
            }
        } else if lit {
            painter.add(metal::lamp(rect.right_top() + vec2(-5.5, 5.5), 2.2, GREEN, 1.0));
        }
    }
    if response.clicked() {
        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A standing order as a labelled button `size` large: its glyph at the left, in words what it is
/// set to ("FIRE AT WILL"), and at the right which of its settings it is at as a row of lamps
/// (`steps`: the one lit, how many there are). `lit` presses it in with its lamp (a cloak that is
/// on); `warn` rusts it red (a countdown running); `text_size` is the lettering the family of
/// buttons shares (see [`family_size`]).
#[allow(clippy::too_many_arguments)]
pub fn stance_button(ui: &mut Ui, glyph: Glyph, label: &str, size: Vec2, lit: bool, warn: bool, steps: Option<(u8, u8)>, text_size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = if response.hovered() { 1.0 } else { 0.0 };
        let down = lit || response.is_pointer_button_down_on();
        let kind = if warn { Slab::Rust } else { Slab::Steel };
        let painter = ui.painter();
        painter.add(metal::slab(ui.ctx(), rect, kind, hot, down));
        let dy = if down { 1.0 } else { 0.0 };
        let ink = ink(kind, hot, lit);
        let side = (rect.height() - 8.0).clamp(14.0, 18.0);
        let glyph_at = pos2(rect.left() + 5.0 + side / 2.0, rect.center().y + dy);
        stamped(painter, Rect::from_center_size(glyph_at, vec2(side, side)), glyph, ink);
        // The lamps down the right edge (a row of them: which setting it is at), or one when on.
        let mut edge = rect.right() - 5.0;
        if let Some((at, of)) = steps {
            for k in (0..of).rev() {
                painter.add(metal::lamp(pos2(edge - 2.0, rect.center().y + dy), 1.9, AMBER, if k == at { 1.0 } else { 0.0 }));
                edge -= 6.0;
            }
        } else if lit {
            painter.add(metal::lamp(pos2(edge - 2.5, rect.center().y + dy), 2.3, GREEN, 1.0));
            edge -= 8.0;
        }
        let left = glyph_at.x + side / 2.0 + 5.0;
        let room = (edge - 3.0 - left).max(8.0);
        let galley = lettering(painter, label, text_size, room, ink);
        let at = pos2(left + galley.size().x / 2.0, rect.center().y + dy);
        metal::slab_text(&painter.with_clip_rect(rect.shrink(1.0)), at, galley, kind.light_text(), ink, down);
    }
    if response.clicked() {
        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The size a family of buttons is lettered in: the buttons' own 10.5 points if every one of
/// `labels` (each with the room its button leaves it) fits on one line, else smaller, down to
/// 8. One size for the family, so that its lettering matches from button to button (one label
/// shrunk on its own looked like another face beside its neighbours).
pub fn family_size<'a>(painter: &Painter, labels: impl IntoIterator<Item = (&'a str, f32)> + Clone) -> f32 {
    let fits = |size: f32| labels.clone().into_iter().all(|(text, room)| painter.layout_no_wrap(text.to_string(), strong(size), Color32::WHITE).size().x <= room);
    let mut size = 10.5;
    while size > 8.0 && !fits(size) {
        size -= 0.5;
    }
    size
}

/// `text` within `room` points in lettering `size` points high: on one line, wrapped if it
/// does not fit at that (a long translation).
pub fn lettering(painter: &Painter, text: &str, size: f32, room: f32, color: Color32) -> std::sync::Arc<egui::Galley> {
    let galley = painter.layout_no_wrap(text.to_string(), strong(size), color);
    if galley.size().x <= room { galley } else { painter.layout(text.to_string(), strong(size), color, room) }
}

/// How many order buttons a row of `width` points has room for: five, or six from 200 up.
pub fn order_slots(width: f32) -> usize {
    if width >= 200.0 { 6 } else { 5 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLYPHS: u32 = Glyph::ALL.len() as u32;

    #[test]
    fn the_atlas_is_laid_out_as_the_tool_draws_it() {
        let img = image::load_from_memory(ATLAS).unwrap();
        let width = SIZES.iter().map(|s| (s + 2 * PAD) * GLYPHS).max().unwrap();
        let height: u32 = SIZES.iter().map(|s| s + 2 * PAD).sum();
        assert_eq!((img.width(), img.height()), (width, height));
        // Every glyph has something in its cell at every size, and nothing in the padding.
        let img = img.to_rgba8();
        for (row, &s) in SIZES.iter().enumerate() {
            for (g, &glyph) in Glyph::ALL.iter().enumerate() {
                assert_eq!(glyph as usize, g);
                let (x, y) = cell(glyph, row);
                let inside = (0..s).flat_map(|j| (0..s).map(move |i| (i, j))).filter(|&(i, j)| img.get_pixel(x + i, y + j)[3] > 128).count();
                assert!(inside > (s * s / 20) as usize, "glyph {g} at {s} px is empty");
                let edge = (0..s).filter(|&j| img.get_pixel(x - PAD, y + j)[3] > 0).count();
                assert_eq!(edge, 0, "glyph {g} at {s} px reaches its padding");
            }
        }
    }
}
