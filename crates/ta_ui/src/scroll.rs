//! Scroll areas in TA's materials. egui's own bar floats out of sight until the pointer comes
//! near it, and its fade takes the colour of a frame's fill, which the panels' painted metal
//! has none of: a list or a text cut off at a well's edge looked either complete or broken.
//!
//! [`show`] keeps a lane beside contents that run past their area, with a slot down it and a
//! steel rod in the slot for a thumb (brighter under the pointer, brass while it is dragged),
//! and fades the contents into what is behind them ([`Behind`]) at an edge with more beyond
//! it. [`whole_rows`] fits a list's height to its rows, so the cut falls between two of them.
//!
//! Lists in a well or on a plate stop between two rows of any height as well: what the
//! contents painted is read back, and where the area's foot would cut a line of text or
//! anything short (a row's plate, a key cap, a button; [`keep_together`] joins more into one)
//! they end above it, over a blank strip. Once still, their top edge eases off a row it cuts
//! too.

use std::ops::Range;

use egui::layers::ShapeIdx;
use egui::scroll_area::ScrollAreaOutput;
use egui::style::{ScrollFadeStyle, ScrollStyle};
use egui::{Color32, CornerRadius, Id, Key, LayerId, Margin, Mesh, Modifiers, Painter, Pos2, Rangef, Rect, ScrollArea, Sense, Shape, Stroke, StrokeKind, Ui, WidgetInfo, WidgetType, emath, pos2};

use crate::metal;
use crate::theme::*;

/// The lane beside the contents a scroll bar takes, while they run past their area.
pub const LANE: f32 = 12.0;
/// A thumb is never shorter than this.
const THUMB_MIN: f32 = 28.0;
/// How far in from an edge the contents fade, at most.
const FADE: f32 = 24.0;
/// How far they fade (1: into nothing but what is behind them).
const FADE_DEPTH: f32 = 0.9;
/// A shape taller than this (a picture, a card, a plate behind many rows), or than this share
/// of the view, may be cut by its edges; a line of text and anything shorter is kept whole.
/// Nor do the contents end further than that above the foot to keep a row whole.
const ROW_MAX: f32 = 120.0;
const WHOLE: f32 = 0.5;
/// How quickly still contents ease off a row their top edge cuts (seconds, the ease's time
/// constant).
const SETTLE: f32 = 0.05;

/// What is behind a scroll area's contents: they fade into it at an edge with more beyond.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Behind {
    /// A recessed well ([`crate::panel::well`]).
    Well,
    /// A panel's or a card's grimy plate.
    Plate,
    /// A screen's backdrop of tiled plates ([`metal::backdrop`]) on this colour.
    Backdrop(Color32),
    /// A flat colour.
    Fill(Color32),
    /// Pictures (a set of tiles): cut off cleanly, as a picture is, with no fade.
    Picture,
}

/// The scroll bar egui draws itself, for scroll areas not shown through [`show`] (a drop-down
/// list's): the same lane, its thumb in sight before the pointer comes near.
pub fn style() -> ScrollStyle {
    ScrollStyle {
        floating: true,
        bar_width: 8.0,
        floating_width: 5.0,
        floating_allocated_width: LANE,
        handle_min_length: THUMB_MIN,
        bar_inner_margin: 0.0,
        bar_outer_margin: 2.0,
        foreground_color: true,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.4,
        interact_background_opacity: 0.7,
        dormant_handle_opacity: 0.45,
        active_handle_opacity: 0.7,
        interact_handle_opacity: 1.0,
        content_margin: Margin::ZERO,
        fade: ScrollFadeStyle { strength: 0.0, size: 0.0 },
    }
}

/// The style [`show`] gives egui: the lane kept, and the whole of it the bar's to click and
/// drag, but nothing of egui's painted (the slot and the rod are painted here).
fn lane_style() -> ScrollStyle {
    ScrollStyle {
        bar_width: LANE,
        floating_width: LANE,
        bar_outer_margin: 0.0,
        dormant_handle_opacity: 0.0,
        active_handle_opacity: 0.0,
        interact_handle_opacity: 0.0,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.0,
        ..style()
    }
}

/// Shows `area` with `add` in it, a thumb in a lane beside it when the contents run past it,
/// and the contents fading into what is `behind` them at an edge with more beyond.
pub fn show<R>(ui: &mut Ui, area: ScrollArea, behind: Behind, add: impl FnOnce(&mut Ui) -> R) -> ScrollAreaOutput<R> {
    let own = ui.spacing().scroll;
    ui.spacing_mut().scroll = lane_style();
    // (The room kept under the contents for their top edge to settle between two rows at
    // their end, by where the area is laid out.)
    let pad_id = ui.next_auto_id().with("ta_ui::scroll::pad");
    let pad = ui.ctx().data(|d| d.get_temp::<f32>(pad_id)).unwrap_or(0.0);
    let lane_id = ui.next_auto_id().with("ta_ui::scroll::lane");
    let room = ui.available_rect_before_wrap().width();
    let mut lane = ui.ctx().data(|d| d.get_temp::<Lane>(lane_id)).unwrap_or_default();
    if (lane.room - room).abs() > 0.5 {
        lane = Lane { room, ..Lane::default() };
    }
    let area = if lane.held.is_some() { area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible) } else { area };
    let mut width = 0.0;
    let mut laid = begin(ui);
    // (The contents get the scroll style back, for scroll areas of their own.)
    let out = area.show(ui, |ui| {
        ui.spacing_mut().scroll = own;
        width = ui.available_width();
        let inner = add(ui);
        reveal_focused_child(ui);
        laid.shapes.end = painted(ui);
        laid.top = ui.max_rect().top();
        laid.foot = ui.min_rect().bottom();
        if pad > 0.0 {
            ui.add_space(pad);
        }
        inner
    });
    ui.spacing_mut().scroll = own;
    lane.settle(out.id, width, out.content_size.y);
    ui.ctx().data_mut(|d| d.insert_temp(lane_id, lane));
    let foot = whole(ui, &out, behind, laid, Some(pad_id));
    paint(ui, &out, behind, foot);
    keyboard_scroll(ui, &out);
    out
}

/// How a scroll area's bar came and went over its last frames, for [`show`]. Contents that fit
/// beside the bar's lane but not without it (a picture as wide as the area and as tall as it is
/// wide) would make egui show the bar one frame and hide it the next, for as long as nothing
/// changed: the contents flickered between two widths. Once the widths they were given
/// alternate, the lane is kept for them, until the area's room or its contents change.
#[derive(Clone, Copy, Debug, Default)]
struct Lane {
    /// The room the area had across, lane and all.
    room: f32,
    /// The area, and the widths its contents were given over the last three frames (newest
    /// last).
    area: Option<Id>,
    widths: [f32; 3],
    /// The lane is kept for contents this tall beside it.
    held: Option<f32>,
}

impl Lane {
    fn settle(&mut self, area: Id, width: f32, height: f32) {
        if self.area != Some(area) {
            *self = Lane { room: self.room, area: Some(area), ..Lane::default() };
        }
        self.widths = [self.widths[1], self.widths[2], width];
        match self.held {
            // (Other contents, or as many again: let the bar decide afresh.)
            Some(kept) if (kept - height).abs() > 1.0 => self.held = None,
            Some(_) => {}
            None => {
                let [a, b, c] = self.widths;
                if a > 0.0 && (a - c).abs() < 0.25 && (a - b).abs() > 0.5 {
                    self.held = Some(height);
                }
            }
        }
    }
}

/// [`show`] for a long list of rows `row_height` high (and the item spacing apart), only those
/// in sight drawn: `add` gets their range.
pub fn show_rows<R>(ui: &mut Ui, area: ScrollArea, behind: Behind, row_height: f32, rows: usize, add: impl FnOnce(&mut Ui, std::ops::Range<usize>) -> R) -> ScrollAreaOutput<R> {
    let own = ui.spacing().scroll;
    ui.spacing_mut().scroll = lane_style();
    let mut laid = begin(ui);
    let out = area.show_rows(ui, row_height, rows, |ui, range| {
        ui.spacing_mut().scroll = own;
        let inner = add(ui, range);
        reveal_focused_child(ui);
        laid.shapes.end = painted(ui);
        inner
    });
    ui.spacing_mut().scroll = own;
    laid.top = out.inner_rect.min.y - out.state.offset.y;
    laid.foot = laid.top + out.content_size.y;
    let foot = whole(ui, &out, behind, laid, None);
    paint(ui, &out, behind, foot);
    keyboard_scroll(ui, &out);
    out
}

/// Native egui text fields, sliders and combo boxes need the same reveal behaviour as our
/// custom buttons. In particular Ctrl+F can focus a search box above a scrolled page.
fn reveal_focused_child(ui: &Ui) {
    if let Some(focused) = ui.memory(|m| m.focused())
        && let Some(response) = ui.ctx().read_response(focused)
        && response.layer_id == ui.layer_id()
        && ui.min_rect().contains(response.rect.center())
    {
        crate::widgets::reveal_focus(ui, &response);
    }
}

/// A focusable scroll viewport lets text-only panels and virtualized rows be reached with
/// the keyboard. It does not take pointer clicks from children, or steal navigation keys
/// from a focused slider/text field. The outline makes this Tab stop visible.
fn keyboard_scroll<R>(ui: &mut Ui, out: &ScrollAreaOutput<R>) {
    let max = (out.content_size - out.inner_rect.size()).max(egui::Vec2::ZERO);
    if max.x < 1.0 && max.y < 1.0 {
        return;
    }
    let response = ui.interact(out.inner_rect, out.id.with("keyboard"), Sense::focusable_noninteractive());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ScrollBar, ui.is_enabled(), crate::i18n::t("Scroll content (arrow keys, Page Up/Down, Home/End)")));
    crate::widgets::focus_ring(ui, &response, out.inner_rect.shrink(2.0));
    if !response.has_focus() {
        return;
    }
    let key = ui.input_mut(|i| {
        [Key::PageUp, Key::PageDown, Key::Home, Key::End, Key::ArrowUp, Key::ArrowDown, Key::ArrowLeft, Key::ArrowRight]
            .into_iter().find(|&key| i.consume_key(Modifiers::NONE, key))
    });
    if let Some(key) = key {
        let mut state = out.state;
        state.offset = keyboard_offset(key, state.offset, max, out.inner_rect.size());
        state.store(ui.ctx(), out.id);
        ui.ctx().data_mut(|d| d.remove::<Rest>(out.id.with("ta_ui::scroll::rest")));
        ui.ctx().request_repaint();
    }
}

fn keyboard_offset(key: Key, mut offset: egui::Vec2, max: egui::Vec2, view: egui::Vec2) -> egui::Vec2 {
    match key {
        Key::PageUp => offset.y -= (view.y * 0.85).max(32.0),
        Key::PageDown => offset.y += (view.y * 0.85).max(32.0),
        Key::Home => offset = egui::Vec2::ZERO,
        Key::End => offset = max,
        Key::ArrowUp => offset.y -= 32.0,
        Key::ArrowDown => offset.y += 32.0,
        Key::ArrowLeft => offset.x -= 32.0,
        Key::ArrowRight => offset.x += 32.0,
        _ => {}
    }
    offset.max(egui::Vec2::ZERO).min(max)
}

/// [`show`] in a recessed well ([`crate::panel::well`]): a list, a briefing.
pub fn well<R>(ui: &mut Ui, area: ScrollArea, add: impl FnOnce(&mut Ui) -> R) -> ScrollAreaOutput<R> {
    crate::panel::well(ui, |ui| show(ui, area, Behind::Well, add))
}

/// [`well`] for text in `margin`: the bar's lane takes the room of the margins on its right
/// (the well's and the text's), so the text keeps the well's whole width whether the bar
/// shows or not (it wrapped narrower beside the bar, a line longer).
pub fn text_well<R>(ui: &mut Ui, area: ScrollArea, margin: Margin, add: impl FnOnce(&mut Ui) -> R) -> ScrollAreaOutput<R> {
    // (panel::well's margin, kept on the left, above and below.)
    const WELL: i8 = 6;
    let background = ui.painter().add(Shape::Noop);
    let inner = egui::Frame::new().inner_margin(Margin { left: WELL, right: 0, top: WELL, bottom: WELL }).show(ui, |ui| {
        let outer = ui.available_width();
        show(ui, area, Behind::Well, |ui| {
            // Right of the text: its margin and the well's, less the lane when the bar shows.
            let lane = outer - ui.available_width();
            let right = (f32::from(margin.right) + f32::from(WELL) - lane).max(0.0).round() as i8;
            egui::Frame::new()
                .inner_margin(Margin { right, ..margin })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    add(ui)
                })
                .inner
        })
    });
    ui.painter().set(background, metal::well(inner.response.rect, INSET));
    inner.inner
}

/// Keeps `rect` whole at the edges of the scroll area being shown, as a line of text is: a row
/// of several parts, or a heading reaching into what follows it (so that it never ends a list).
pub fn keep_together(ui: &Ui, rect: Rect) {
    let layer = ui.layer_id();
    ui.ctx().data_mut(|d| {
        if let Some((on, rects)) = d.get_temp_mut_or_default::<Kept>(kept_id()).0.last_mut()
            && *on == layer
        {
            rects.push(rect);
        }
    });
}

/// What [`keep_together`] was given in the contents of each scroll area being shown, the
/// innermost last, with the layer they are painted in (a popup's are not theirs).
#[derive(Clone, Default)]
struct Kept(Vec<(LayerId, Vec<Rect>)>);

fn kept_id() -> Id {
    Id::new("ta_ui::scroll::kept")
}

/// What a scroll area's contents laid out: the shapes they painted in its layer, and where
/// they begin and end down the screen (without the room added under them).
struct Laid {
    shapes: Range<usize>,
    top: f32,
    foot: f32,
}

/// Starts reading back the contents about to be shown in a scroll area in `ui`.
fn begin(ui: &Ui) -> Laid {
    ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Kept>(kept_id()).0.push((ui.layer_id(), Vec::new())));
    let first = painted(ui);
    Laid { shapes: first..first, top: 0.0, foot: 0.0 }
}

/// How many shapes `ui`'s layer holds so far.
fn painted(ui: &Ui) -> usize {
    ui.ctx().graphics(|g| g.get(ui.layer_id()).map_or(0, |list| list.next_idx().0))
}

/// Where a scroll area's contents stood last frame, and where they are easing to.
#[derive(Clone, Copy, Debug, Default)]
struct Rest {
    offset: f32,
    target: Option<f32>,
}

/// Keeps rows whole at a scroll area's edges, its contents laid out as `laid`: they end above
/// a row their foot would cut (returned: where, for the fade), and once still, their top edge
/// eases off a row it cuts, up to show it or on past it, whichever is nearer (at their end,
/// over room `pad_id` keeps under them for that).
fn whole<R>(ui: &Ui, out: &ScrollAreaOutput<R>, behind: Behind, laid: Laid, pad_id: Option<Id>) -> Option<f32> {
    let ctx = ui.ctx();
    let kept = ctx.data_mut(|d| d.get_temp_mut_or_default::<Kept>(kept_id()).0.pop()).map(|(_, rects)| rects).unwrap_or_default();
    let view = out.inner_rect;
    // (Not a page over the backdrop: cards there come into sight as they are, cut or not.)
    if matches!(behind, Behind::Picture | Behind::Backdrop(_)) || out.content_size.y <= view.height().ceil() || view.height() < 1.0 {
        return None;
    }
    let layer = ui.layer_id();
    let limit = (view.height() * WHOLE).min(ROW_MAX);
    let mut spans = kept;
    ctx.graphics(|g| {
        if let Some(list) = g.get(layer) {
            for clipped in list.all_entries().skip(laid.shapes.start).take(laid.shapes.len()) {
                spans_of(&clipped.shape, clipped.clip_rect, limit, &mut spans);
            }
        }
    });
    let blocks = merged(spans.iter().map(|r| r.y_range()).collect());
    let pixel = |y: f32| (y * ctx.pixels_per_point()).round() / ctx.pixels_per_point();

    // The foot: above a row it would cut, the rest of the view left blank and out of reach.
    // Columns of rows side by side whose rows fall out of step (a list in two columns) each
    // end above a row of their own instead, when one foot across them would have to rise far
    // higher to clear both (a row's parts, its name and its figures, end together).
    let rise = |foot: Option<f32>| foot.map_or(0.0, |f| view.max.y - f);
    let across = foot_of(&blocks, view.y_range(), limit);
    let columns = columns_met(&spans, &blocks, view.max.y, view.x_range());
    let own: Vec<Option<f32>> = columns.iter().map(|&column| foot_of(&merged(spans.iter().filter(|r| r.x_range().intersects(column)).map(|r| r.y_range()).collect()), view.y_range(), limit)).collect();
    let apart = columns.len() > 1 && own.iter().any(Option::is_some) && across.is_none_or(|_| rise(across) > 1.5 * own.iter().map(|f| rise(*f)).fold(0.0, f32::max) + 8.0);
    let feet: Vec<(Rangef, Option<f32>)> = if apart { columns.iter().copied().zip(own).collect() } else { vec![(view.x_range(), across)] };
    let feet: Vec<(Rangef, Option<f32>)> = feet.into_iter().map(|(x, f)| (x, f.map(pixel))).collect();
    let foot = feet.iter().all(|(_, f)| f.is_some()).then(|| feet.iter().filter_map(|(_, f)| *f).fold(view.min.y, f32::max));
    if feet.iter().any(|(_, f)| f.is_some()) {
        let at = |x: f32| feet.iter().find(|(column, _)| column.contains(x)).and_then(|(_, f)| *f).unwrap_or(view.max.y);
        ctx.graphics_mut(|g| {
            if let Some(list) = g.get_mut(layer) {
                for i in laid.shapes.clone() {
                    list.mutate_shape(ShapeIdx(i), |c| c.clip_rect.max.y = c.clip_rect.max.y.min(at(c.shape.visual_bounding_rect().center().x)));
                }
            }
        });
        for (k, (column, f)) in feet.iter().enumerate() {
            if let Some(f) = f {
                // This only blocks pointer clicks on clipped rows. It is not a control,
                // and must not become an invisible stop in keyboard navigation.
                let _ = ui.interact(Rect::from_x_y_ranges(*column, *f..=view.max.y), out.id.with(("ta_ui::scroll::blank", k)), Sense::CLICK);
            }
        }
    }

    // The room under the contents for their top edge to settle past a row it cuts at their
    // end, worked out while that end is in sight.
    if let Some(pad_id) = pad_id {
        let end = laid.top + (laid.foot - laid.top - view.height()).max(0.0);
        if view.y_range().contains(end) {
            let pad = ease_off(&blocks, end, limit).map_or(0.0, |(_, on)| on.ceil());
            if ctx.data(|d| d.get_temp::<f32>(pad_id)).unwrap_or(0.0) != pad {
                ctx.data_mut(|d| d.insert_temp(pad_id, pad));
                ctx.request_repaint();
            }
        }
    }

    // Still (where it was last frame, no button held, no momentum): the top edge eases off a
    // row it cuts. Never up so far that the contents' end goes out of sight once in it.
    let rest_id = out.id.with("ta_ui::scroll::rest");
    let offset = out.state.offset.y;
    let max = (out.content_size.y - view.height()).max(0.0);
    let last = ctx.data(|d| d.get_temp::<Rest>(rest_id));
    let still = last.is_some_and(|l| (l.offset - offset).abs() < 0.01) && out.state.velocity().y == 0.0 && !ctx.input(|i| i.pointer.any_down());
    let target = match last.and_then(|l| l.target) {
        Some(target) if still => Some(target),
        _ if still => ease_off(&blocks, view.min.y, limit).and_then(|(up, on)| {
            let room = if laid.foot > view.max.y + 0.5 { f32::INFINITY } else { view.max.y - laid.foot };
            let up = (up <= room).then_some(offset - up).map(|o| o.max(0.0));
            let on = (offset + on <= max + 0.5).then_some((offset + on).min(max));
            match (up, on) {
                (Some(u), Some(o)) => Some(if offset - u <= o - offset { u } else { o }),
                (u, o) => u.or(o),
            }
            .map(pixel)
        }),
        _ => None,
    };
    let rest = match target.filter(|t| (t - offset).abs() > 0.25) {
        Some(target) => {
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            let eased = target + (offset - target) * (-dt / SETTLE).exp();
            let next = if (eased - target).abs() < 0.5 { target } else { eased };
            let mut state = out.state;
            state.offset.y = next;
            state.store(ctx, out.id);
            ctx.request_repaint();
            Rest { offset: next, target: (next != target).then_some(target) }
        }
        None => Rest { offset, target: None },
    };
    ctx.data_mut(|d| d.insert_temp(rest_id, rest));
    foot
}

/// Adds to `spans` what `shape` keeps whole where it shows within `clip`: each line of a text
/// (it may end between two of them), and the whole of any other shape no taller than
/// `tallest`.
fn spans_of(shape: &Shape, clip: Rect, tallest: f32, spans: &mut Vec<Rect>) {
    let keep = |rect: Rect, spans: &mut Vec<Rect>| {
        if rect.is_finite() && rect.height() > 0.0 && rect.height() <= tallest && rect.intersects(clip) {
            spans.push(rect);
        }
    };
    match shape {
        Shape::Noop => {}
        Shape::Vec(shapes) => {
            for shape in shapes {
                spans_of(shape, clip, tallest, spans);
            }
        }
        Shape::Text(text) if text.angle == 0.0 => {
            for row in &text.galley.rows {
                keep(row.rect().translate(text.pos.to_vec2()), spans);
            }
        }
        shape => keep(shape.visual_bounding_rect(), spans),
    }
}

/// The stretches kept whole, in order down the screen, joined where they overlap (by more
/// than a pixel: a seam along a row's foot does not join it to the next).
fn merged(mut spans: Vec<Rangef>) -> Vec<Rangef> {
    spans.sort_by(|a, b| a.min.total_cmp(&b.min));
    let mut blocks: Vec<Rangef> = Vec::with_capacity(spans.len());
    for span in spans {
        match blocks.last_mut() {
            Some(last) if span.min < last.max - 1.0 => last.max = last.max.max(span.max),
            _ => blocks.push(span),
        }
    }
    blocks
}

/// The columns side by side that what is kept whole stands in, across `view` (each reaching
/// halfway to the next): their stretches across, joined where they overlap.
fn columns_of(spans: &[Rect], view: Rangef) -> Vec<Rangef> {
    let mut across: Vec<Rangef> = spans.iter().map(|r| r.x_range()).collect();
    across.sort_by(|a, b| a.min.total_cmp(&b.min));
    let mut columns: Vec<Rangef> = Vec::new();
    for x in across {
        match columns.last_mut() {
            Some(last) if x.min <= last.max + 1.0 => last.max = last.max.max(x.max),
            _ => columns.push(x),
        }
    }
    let n = columns.len();
    (0..n).map(|k| Rangef::new(if k == 0 { view.min } else { (columns[k - 1].max + columns[k].min) / 2.0 }, if k + 1 == n { view.max } else { (columns[k].max + columns[k + 1].min) / 2.0 })).collect()
}

/// The columns of the rows a foot at `foot` meets (as [`columns_of`]): what lies across them
/// elsewhere (a heading, a row of tiles above them) does not join them.
fn columns_met(spans: &[Rect], blocks: &[Rangef], foot: f32, view: Rangef) -> Vec<Rangef> {
    let met = blocks.iter().find(|b| b.min < foot - 1.0 && b.max > foot + 1.0);
    let near: Vec<Rect> = spans.iter().filter(|r| met.is_none_or(|b| r.y_range().intersects(*b))).copied().collect();
    columns_of(&near, view)
}

/// Where contents shown in `view` end when its foot would cut one of `blocks`: midway in the
/// gap above that one (at its top with nothing above it in sight). None: the foot cuts
/// nothing, or keeping the block whole would leave more than `limit` blank.
fn foot_of(blocks: &[Rangef], view: Rangef, limit: f32) -> Option<f32> {
    let i = blocks.iter().position(|b| b.min < view.max - 1.0 && b.max > view.max + 1.0)?;
    let cut = match i.checked_sub(1).map(|j| blocks[j]) {
        Some(above) if above.max >= view.min => (above.max + blocks[i].min) / 2.0,
        _ => blocks[i].min,
    };
    (cut > view.min + 1.0 && view.max - cut <= limit).then_some(cut)
}

/// How far an edge at `y` cutting one of `blocks` would go to clear it: (up, on), each to the
/// middle of the gap beyond it (the other gap's half where there is none in sight, as rows
/// are alike). None: it cuts none, or one taller than `limit`.
fn ease_off(blocks: &[Rangef], y: f32, limit: f32) -> Option<(f32, f32)> {
    let i = blocks.iter().position(|b| b.min < y - 1.0 && b.max > y + 1.0)?;
    let block = blocks[i];
    if block.span() > limit {
        return None;
    }
    let above = i.checked_sub(1).map(|j| (block.min - blocks[j].max) / 2.0);
    let below = blocks.get(i + 1).map(|b| (b.min - block.max) / 2.0);
    let (above, below) = (above.or(below).unwrap_or(0.0), below.or(above).unwrap_or(0.0));
    Some((y - block.min + above, block.max - y + below))
}

/// The height of a list whose rows come `stride` apart (a row and the gap after it, `gap`)
/// that shows only whole rows and is at most `room` high: its edge falls between two rows.
pub fn whole_rows(room: f32, stride: f32, gap: f32) -> f32 {
    let rows = ((room + gap) / stride).floor().max(1.0);
    rows * stride - gap
}

/// The height of a view onto lines of text `line` high under a margin of `lead`, at most
/// `room` high: its edge falls between two lines.
pub fn whole_lines(room: f32, line: f32, lead: f32) -> f32 {
    lead + ((room - lead) / line).floor().max(1.0) * line
}

/// Where a scroll bar's thumb is along its track (`track`), for a view `view` long onto
/// contents `content` long scrolled `offset` along them: egui's own reckoning, so the thumb
/// painted is the one dragged.
fn thumb(track: Rangef, view: f32, content: f32, offset: f32) -> Rangef {
    let from_content = |c: f32| emath::remap_clamp(c, 0.0..=content, track.min..=track.max);
    let length = (from_content(offset + view) - from_content(offset)).max(THUMB_MIN.min(track.span()));
    let start = emath::remap_clamp(offset, 0.0..=(content - view).max(0.0), track.min..=(track.max - length));
    Rangef::new(start, start + length)
}

/// The bar of each way the contents run past their area, and their fades (the foot's where
/// they end, `foot`, above a row the area's foot would cut).
fn paint<R>(ui: &Ui, out: &ScrollAreaOutput<R>, behind: Behind, foot: Option<f32>) {
    let ctx = ui.ctx();
    // (Past their area this frame: egui shows the bar now, its response the lane it takes.)
    let bars: Vec<(usize, egui::Response)> =
        (0..2usize).filter(|&d| out.content_size[d] > out.inner_rect.size()[d].ceil()).filter_map(|d| ctx.read_response(out.id.with(d)).map(|bar| (d, bar))).collect();
    if bars.is_empty() {
        return;
    }
    // The contents' view: the area as it is shown, less the lanes.
    let mut view = out.inner_rect;
    for (d, bar) in &bars {
        if *d == 1 {
            view.max.x = bar.rect.min.x;
            view.max.y = view.max.y.min(bar.rect.max.y);
        } else {
            view.max.y = bar.rect.min.y;
            view.max.x = view.max.x.min(bar.rect.max.x);
        }
    }
    let painter = ui.painter();
    if bars.iter().any(|(d, _)| *d == 1) {
        let above = out.state.offset.y;
        let below = out.content_size.y - view.height() - above;
        let shown = Rect::from_min_max(view.min, pos2(view.max.x, foot.map_or(view.max.y, |f| f.min(view.max.y))));
        paint_fades(ctx, painter, shown, above, below, behind);
    }
    for (d, bar) in bars {
        // The bar runs along the view, in its lane.
        let (lane, track) = if d == 1 { (Rect::from_x_y_ranges(bar.rect.x_range(), view.y_range()), view.y_range()) } else { (Rect::from_x_y_ranges(view.x_range(), bar.rect.y_range()), view.x_range()) };
        let along = thumb(track, track.span(), out.content_size[d], out.state.offset[d]);
        let held = bar.dragged() || bar.is_pointer_button_down_on();
        let hot = ctx.animate_bool_with_time(out.id.with(("ta_ui::scroll::hot", d)), bar.hovered() || held, 0.12);
        paint_bar(painter, lane, along, d == 1, hot, held);
    }
}

/// A scroll bar: a narrow slot down the middle of its lane, and the thumb in it, a steel rod
/// (thicker and lighter when `hot`, brass while `held`).
fn paint_bar(painter: &Painter, lane: Rect, along: Rangef, vertical: bool, hot: f32, held: bool) {
    let across = |rect: Rect, width: f32| if vertical { Rect::from_x_y_ranges(Rangef::point(rect.center().x).expand(width / 2.0), rect.y_range()) } else { Rect::from_x_y_ranges(rect.x_range(), Rangef::point(rect.center().y).expand(width / 2.0)) };
    let slot = across(lane, 6.0);
    painter.add(Shape::rect_filled(slot, CornerRadius::same(3), Color32::from_black_alpha(110)));
    painter.add(Shape::rect_stroke(slot, CornerRadius::same(3), Stroke::new(1.0, Color32::from_black_alpha(150)), StrokeKind::Inside));
    let thumb = if vertical { Rect::from_x_y_ranges(slot.x_range(), along) } else { Rect::from_x_y_ranges(along, slot.y_range()) };
    let rod = across(thumb, emath::lerp(4.0..=6.0, hot));
    let (body, shine, shade) = if held {
        (Color32::from_rgb(166, 128, 52), AMBER_BRIGHT, Color32::from_rgb(92, 70, 26))
    } else {
        let lift = 0.18 * hot;
        (metal::STEEL_BODY.lerp_to_gamma(Color32::WHITE, lift), metal::STEEL_SHINE.lerp_to_gamma(Color32::WHITE, lift), metal::STEEL_SHADE.lerp_to_gamma(Color32::WHITE, lift))
    };
    painter.add(Shape::rect_filled(rod.expand(1.0), CornerRadius::same(3), metal::STEEL_EDGE));
    // Lit like the frames' pipes: a shine a third of the way across, shade beyond it.
    let (lit, rest) = if vertical {
        let x = rod.min.x + rod.width() * 0.35;
        (Rect::from_min_max(rod.min, pos2(x, rod.max.y)), Rect::from_min_max(pos2(x, rod.min.y), rod.max))
    } else {
        let y = rod.min.y + rod.height() * 0.35;
        (Rect::from_min_max(rod.min, pos2(rod.max.x, y)), Rect::from_min_max(pos2(rod.min.x, y), rod.max))
    };
    if vertical {
        painter.add(metal::hgrad(lit, body, shine));
        painter.add(metal::hgrad(rest, shine, shade));
    } else {
        painter.add(metal::vgrad(lit, body, shine));
        painter.add(metal::vgrad(rest, shine, shade));
    }
}

/// The contents fading into what is behind them in `view`'s top `FADE` when `above` more lies
/// above it, and in its bottom when `below` more lies below (deeper the further it goes on).
fn paint_fades(ctx: &egui::Context, painter: &Painter, view: Rect, above: f32, below: f32, behind: Behind) {
    if behind == Behind::Picture {
        return;
    }
    let size = FADE.min(view.height() / 4.0);
    if size < 2.0 {
        return;
    }
    for (more, top) in [(above, true), (below, false)] {
        if more <= 0.5 {
            continue;
        }
        // (Deeper as more comes into it, as egui's does, so it never jumps on.)
        let depth = FADE_DEPTH * (more / size).min(1.0);
        let band = if top { Rect::from_min_max(view.min, pos2(view.max.x, view.min.y + size)) } else { Rect::from_min_max(pos2(view.min.x, view.max.y - size), view.max) };
        // (How deep along the band's top and along its foot.)
        let (upper, lower) = if top { (depth, 0.0) } else { (0.0, depth) };
        match behind {
            Behind::Well => {
                painter.add(metal::vgrad(band, INSET.gamma_multiply(upper), INSET.gamma_multiply(lower)));
            }
            Behind::Fill(color) => {
                painter.add(metal::vgrad(band, color.gamma_multiply(upper), color.gamma_multiply(lower)));
            }
            Behind::Plate => {
                painter.add(metal::vgrad(band, PANEL.gamma_multiply(upper), PANEL.gamma_multiply(lower)));
                painter.add(grime_fade(ctx, band, upper, lower));
            }
            Behind::Backdrop(base) => {
                // The backdrop again over the band, in strips more opaque towards the edge.
                const STRIPS: usize = 12;
                for k in 0..STRIPS {
                    let (t0, t1) = (k as f32 / STRIPS as f32, (k + 1) as f32 / STRIPS as f32);
                    let strip = Rect::from_min_max(pos2(band.min.x, emath::lerp(band.y_range(), t0)), pos2(band.max.x, emath::lerp(band.y_range(), t1)));
                    let alpha = emath::lerp(upper..=lower, (t0 + t1) / 2.0);
                    let mut layer = painter.with_clip_rect(strip);
                    layer.multiply_opacity(alpha);
                    layer.add(metal::backdrop(ctx, strip, base));
                }
            }
            Behind::Picture => {}
        }
    }
}

/// The plates' grime over `band`, as strong as `top` along its top and `bottom` along its
/// foot: anchored to the screen as theirs is, so it lines up with the plate under the fade.
fn grime_fade(ctx: &egui::Context, band: Rect, top: f32, bottom: f32) -> Shape {
    let mut mesh = Mesh::with_texture(metal::grime(ctx));
    let s = 256.0;
    let uv = |p: Pos2| pos2(p.x / s, p.y / s);
    let tint = |a: f32| {
        let a = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgba_premultiplied(a, a, a, a)
    };
    for (p, a) in [(band.left_top(), top), (band.right_top(), top), (band.left_bottom(), bottom), (band.right_bottom(), bottom)] {
        mesh.vertices.push(egui::epaint::Vertex { pos: p, uv: uv(p), color: tint(a) });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    Shape::mesh(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_pages_clamp_and_reach_both_ends() {
        let view = egui::vec2(300.0, 200.0);
        let max = egui::vec2(100.0, 700.0);
        let zero = egui::Vec2::ZERO;
        assert_eq!(keyboard_offset(Key::PageDown, zero, max, view), egui::vec2(0.0, 170.0));
        assert_eq!(keyboard_offset(Key::PageUp, zero, max, view), zero);
        assert_eq!(keyboard_offset(Key::End, zero, max, view), max);
        assert_eq!(keyboard_offset(Key::PageDown, max, max, view), max);
        assert_eq!(keyboard_offset(Key::Home, max, max, view), zero);
        assert_eq!(keyboard_offset(Key::ArrowLeft, max, max, view), egui::vec2(68.0, 700.0));
    }

    #[test]
    fn only_the_focused_scroll_viewport_consumes_paging_keys() {
        let ctx = egui::Context::default();
        let mut id: Option<Id> = None;
        let mut run = |events: Vec<egui::Event>, focused: bool| {
            if let Some(id) = id {
                ctx.memory_mut(|m| if focused { m.request_focus(id.with("keyboard")); } else { m.surrender_focus(id.with("keyboard")); });
            }
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 240.0))), events, ..Default::default() };
            let mut offset = 0.0;
            let mut key_left = false;
            ctx.run_ui(raw, |ui| {
                let out = show(ui, ScrollArea::vertical().id_salt("test_keyboard").max_height(180.0), Behind::Picture, |ui| {
                    ui.set_min_height(900.0);
                });
                id = Some(out.id);
                offset = egui::scroll_area::State::load(ui.ctx(), out.id).unwrap().offset.y;
                key_left = ui.input(|i| i.events.iter().any(|event| matches!(event, egui::Event::Key { key: Key::PageDown, .. })));
            }).drop_without_applying_deltas();
            (offset, key_left)
        };
        let key = || vec![egui::Event::Key { key: Key::PageDown, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }];
        run(vec![], false);
        let (offset, left) = run(key(), false);
        assert_eq!(offset, 0.0);
        assert!(left, "unfocused scroll areas must leave keys for the game or another control");
        let (offset, left) = run(key(), true);
        assert!(offset > 100.0, "focused viewport should page down: {offset}");
        assert!(!left);
    }

    #[test]
    fn focusing_an_offscreen_native_text_field_reveals_it() {
        let ctx = egui::Context::default();
        let field = Id::new("offscreen_search");
        let mut text = String::new();
        let mut scroll = None;
        let mut offset = 0.0;
        for frame in 0..4 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 240.0))), ..Default::default() };
            ctx.run_ui(raw, |ui| {
                if frame == 2 {
                    // As SettingsView handles Ctrl+F, during the pass before drawing.
                    ui.memory_mut(|m| m.request_focus(field));
                }
                let out = show(ui, ScrollArea::vertical().id_salt("native_focus").max_height(180.0), Behind::Picture, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut text).id(field));
                    ui.set_min_height(900.0);
                });
                scroll = Some(out.id);
                offset = out.state.offset.y;
            }).drop_without_applying_deltas();
            if frame == 0 {
                let id = scroll.unwrap();
                let mut state = egui::scroll_area::State::load(&ctx, id).unwrap();
                state.offset.y = 600.0;
                state.store(&ctx, id);
            }
            if frame == 1 {
                assert!(offset >= 600.0);
            }
        }
        assert!(offset < 1.0, "newly focused search must return into view: {offset}");
    }

    #[test]
    fn whole_rows_end_between_two_rows() {
        // Rows 28 high, 1 apart: 15 of them take 15 * 29 - 1.
        assert_eq!(whole_rows(440.0, 29.0, 1.0), 434.0);
        assert_eq!(whole_rows(434.0, 29.0, 1.0), 434.0);
        assert_eq!(whole_rows(433.0, 29.0, 1.0), 405.0);
        assert_eq!(whole_rows(10.0, 29.0, 1.0), 28.0, "one row at least");
        assert_eq!(whole_lines(100.0, 20.0, 8.0), 88.0);
    }

    #[test]
    fn rows_of_any_height_stay_whole_at_the_edges() {
        let r = Rangef::new;
        // Lines of text with gaps between them, a shape overlapping one by two pixels (joined
        // to it), and a seam touching another (not joined).
        let blocks = merged(vec![r(6.0, 22.0), r(35.0, 51.0), r(64.0, 80.0), r(49.0, 53.0), r(79.5, 81.0)]);
        assert_eq!(blocks, vec![r(6.0, 22.0), r(35.0, 53.0), r(64.0, 80.0), r(79.5, 81.0)]);
        // A foot through the third ends midway in the gap above it.
        assert_eq!(foot_of(&blocks, r(0.0, 70.0), 40.0), Some(58.5));
        // A foot in a gap cuts nothing; nor does one that would leave too much blank.
        assert_eq!(foot_of(&blocks, r(0.0, 60.0), 40.0), None);
        assert_eq!(foot_of(&blocks, r(0.0, 70.0), 5.0), None);
        // A top edge through the second eases up or on, to the middle of the gap beyond.
        let (up, on) = ease_off(&blocks, 40.0, 40.0).unwrap();
        assert_eq!((40.0 - up, 40.0 + on), (28.5, 58.5));
        assert_eq!(ease_off(&blocks, 30.0, 40.0), None);
        assert_eq!(ease_off(&blocks, 40.0, 10.0), None, "taller than the limit");
    }

    #[test]
    fn columns_side_by_side_reach_halfway_to_each_other() {
        let rect = |x0: f32, x1: f32| Rect::from_x_y_ranges(Rangef::new(x0, x1), Rangef::new(0.0, 10.0));
        // Two columns of rows (one row overlapping another in it), across a view 0..300.
        let columns = columns_of(&[rect(10.0, 100.0), rect(160.0, 290.0), rect(50.0, 120.0)], Rangef::new(0.0, 300.0));
        assert_eq!(columns, vec![Rangef::new(0.0, 140.0), Rangef::new(140.0, 300.0)]);
        // One plate across them joins them.
        assert_eq!(columns_of(&[rect(10.0, 100.0), rect(160.0, 290.0), rect(0.0, 300.0)], Rangef::new(0.0, 300.0)).len(), 1);
    }

    #[test]
    fn columns_are_those_of_the_rows_the_foot_meets() {
        let rect = |x: (f32, f32), y: (f32, f32)| Rect::from_x_y_ranges(Rangef::new(x.0, x.1), Rangef::new(y.0, y.1));
        // A row of tiles across the view, and two columns of rows under it, out of step.
        let spans = [
            rect((100.0, 500.0), (0.0, 60.0)),
            rect((0.0, 280.0), (80.0, 100.0)),
            rect((0.0, 280.0), (110.0, 130.0)),
            rect((0.0, 280.0), (140.0, 160.0)),
            rect((320.0, 600.0), (80.0, 124.0)),
            rect((320.0, 600.0), (132.0, 176.0)),
        ];
        let blocks = merged(spans.iter().map(|r| r.y_range()).collect());
        let view = Rangef::new(0.0, 600.0);
        assert_eq!(columns_of(&spans, view).len(), 1, "over the whole view the tiles join them");
        assert_eq!(columns_met(&spans, &blocks, 150.0, view).len(), 2, "the foot meets two columns");
        assert_eq!(columns_met(&spans, &blocks, 30.0, view).len(), 1, "and the tiles' row is one");
    }

    #[test]
    fn the_thumb_spans_the_view_and_runs_the_track() {
        let track = Rangef::new(100.0, 400.0);
        let near = |a: Rangef, b: (f32, f32)| (a.min - b.0).abs() < 0.01 && (a.max - b.1).abs() < 0.01;
        // A view onto a third of the contents: a third of the track, at the top, then the foot.
        assert!(near(thumb(track, 300.0, 900.0, 0.0), (100.0, 200.0)));
        assert!(near(thumb(track, 300.0, 900.0, 600.0), (300.0, 400.0)));
        // Long contents: never shorter than THUMB_MIN.
        assert_eq!(thumb(track, 300.0, 90_000.0, 0.0).span(), THUMB_MIN);
    }

    /// Contents as tall as they are wide (a map's picture filling a details pane), in a view just
    /// short of them at its full width: beside the bar's lane they fit, without it they do not.
    /// The bar must settle, not come and go every frame (the pane flickered until a resize).
    #[test]
    fn contents_that_fit_beside_the_bar_do_not_make_it_flicker() {
        let ctx = egui::Context::default();
        let run = |screen: egui::Vec2, slack: f32| {
            let mut widths = Vec::new();
            for _ in 0..16 {
                let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), ..Default::default() };
                ctx.run_ui(raw, |ui| {
                    // (Just under the picture at the full width, just over it beside the lane.)
                    let view = (ui.available_width() - LANE * 0.5) * 0.75 + slack;
                    show(ui, ScrollArea::vertical().id_salt("test_flicker").max_height(view).auto_shrink([false, false]), Behind::Plate, |ui| {
                        let width = ui.available_width();
                        widths.push(width);
                        ui.allocate_exact_size(egui::vec2(width, width * 0.75), Sense::hover());
                    });
                }).drop_without_applying_deltas();
            }
            widths
        };
        let widths = run(egui::vec2(400.0, 600.0), 0.0);
        assert!(widths[8..].windows(2).all(|w| w[0] == w[1]), "the lane came and went: {widths:?}");
        // A bigger window lays the contents out afresh; contents with room to spare keep the
        // whole width, no lane kept for a bar they never need.
        let widths = run(egui::vec2(500.0, 900.0), 200.0);
        assert_eq!(widths.last(), Some(&500.0), "{widths:?}");
    }
}
