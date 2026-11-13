//! Window chrome for the game's pages and dialogs, and the larger controls that live in
//! them: title-menu entries, list rows, key caps, the big call-to-action button, labelled
//! settings.
//!
//! A [`Panel`] is a plate of grimy metal in a steel pipe frame, as TA's dialogs are, with
//! a header (a title lettered as TA's are, an optional subtitle, a back or close button)
//! over a pipe capped by joints in its accent colour. [`page`] shows one centred over whatever
//! is behind (the front end's live battle, which the menu dims with [`page_backdrop`]),
//! sliding in from the side the player came from ([`set_direction`]); [`modal`]
//! dims the screen and takes the input while it is open, rising into place. Escape goes
//! back from a page with a BACK button (unless it only left a text field or a drop-down).

use crate::tooltip::TooltipExt as _;
use std::hash::Hash;

use egui::emath::TSTransform;
use egui::{
    Align, Align2, Area, Color32, Context, CornerRadius, CursorIcon, Frame, Id, Key, Layout, Margin, Modifiers, Order,
    Pos2, Rect, Response, RichText, Sense, Shadow, Shape, Stroke, StrokeKind, Ui, Vec2, WidgetInfo, WidgetType, pos2,
    vec2,
};

use crate::i18n::t;
use crate::metal::{self, Slab};
use crate::sound::{self, UiSound};
use crate::theme::*;
use crate::widgets::{Glyph, focus_ring, galley, icon_button, reveal_focus};

// ---- Panels --------------------------------------------------------------------------------

/// Thickness of a panel's pipe frame, and of a floating strip's.
const PANEL_PIPE: f32 = 5.0;
const STRIP_PIPE: f32 = 3.0;

/// What closes a panel from its header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    None,
    /// A "BACK" button (pages that return to the previous one).
    Back,
    /// A "LEAVE" button (a multiplayer lobby).
    Leave,
    /// A close cross (dialogs).
    Close,
}

/// Window chrome: see the module documentation.
pub struct Panel<'a> {
    title: &'a str,
    subtitle: Option<&'a str>,
    width: f32,
    exit: Exit,
    accent: Color32,
    max_height: Option<f32>,
}

pub struct PanelResponse<R> {
    pub inner: R,
    /// The back or close button was clicked.
    pub exit: bool,
    pub rect: Rect,
}

impl<'a> Panel<'a> {
    pub fn new(title: &'a str) -> Self {
        Panel { title, subtitle: None, width: 480.0, exit: Exit::None, accent: AMBER, max_height: None }
    }

    /// A line under the title (letter-spaced caps look best).
    pub fn subtitle(mut self, subtitle: &'a str) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    /// Width of the content (the panel adds its margins).
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn exit(mut self, exit: Exit) -> Self {
        self.exit = exit;
        self
    }

    /// Colour of the joint plates capping the header's pipe (brass by default; red for
    /// warnings...).
    pub fn accent(mut self, accent: Color32) -> Self {
        self.accent = accent;
        self
    }

    /// The panel's whole height, its frame included, at most `height`: a taller body scrolls
    /// (in the body, `ui.available_height()` is the room left in it).
    pub fn max_height(mut self, height: f32) -> Self {
        self.max_height = Some(height);
        self
    }

    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> PanelResponse<R> {
        let response = self.show_with_footer(ui, 0.0, add_contents, |_| ());
        PanelResponse { inner: response.inner.0, exit: response.exit, rect: response.rect }
    }

    /// [`Panel::show`] with a footer under the body (a dialog's buttons), `footer_height` tall:
    /// with [`Panel::max_height`] the body scrolls over it and the footer stays in sight.
    pub fn show_with_footer<R, F>(self, ui: &mut Ui, footer_height: f32, add_contents: impl FnOnce(&mut Ui) -> R, add_footer: impl FnOnce(&mut Ui) -> F) -> PanelResponse<(R, F)> {
        let background = ui.painter().add(Shape::Noop);
        let mut exit = false;
        // Inside the pipes, a gap as wide as the cards' (the title a little closer to the top).
        let side = (metal::pipe_inset(PANEL_PIPE) + space::L).round() as i8;
        let margin = Margin { left: side, right: side, top: side - 4, bottom: side };
        let inner = Frame::new().inner_margin(margin).show(ui, |ui| {
            crate::theme::roomy(ui.style_mut());
            ui.set_width(self.width);
            if !self.title.is_empty() || self.exit != Exit::None {
                ui.horizontal(|ui| {
                    let top = ui.min_rect().min.y;
                    let mut title_mid = None;
                    // The title in the room the way out leaves it: a longer one (a unit's name,
                    // a translation) set smaller in the same place, so the panel keeps its width.
                    let room = self.width
                        - match self.exit {
                            Exit::None => 0.0,
                            Exit::Close => 26.0 + space::S,
                            Exit::Back => back_button_width(ui, "BACK") + space::S,
                            Exit::Leave => back_button_width(ui, "LEAVE") + space::S,
                        };
                    ui.vertical(|ui| {
                        ui.set_max_width(room.max(40.0));
                        if !self.title.is_empty() {
                            const SIZE: f32 = 31.0;
                            let rich = |size: f32| RichText::new(t(self.title)).font(display(size)).extra_letter_spacing(crate::i18n::tracking(1.8));
                            let full = galley(ui, rich(SIZE)).size();
                            let size = if full.x + 2.0 > room { (SIZE * room / (full.x + 2.0) - 0.25).max(SIZE * 0.6) } else { SIZE };
                            let slack = (full.y - galley(ui, rich(size)).size().y) / 2.0;
                            ui.add_space(slack);
                            let title = metal::embossed_label(ui, rich(size), TEXT_BRIGHT);
                            ui.add_space(slack);
                            // (The middle of its capitals: the label's foot holds their shadow.)
                            title_mid = Some(title.rect.min.y + (title.rect.height() - 2.0) / 2.0);
                        }
                        if let Some(subtitle) = self.subtitle {
                            ui.add(egui::Label::new(RichText::new(t(subtitle)).font(strong(12.0)).color(STEEL).extra_letter_spacing(crate::i18n::tracking(1.4))).wrap());
                        }
                    });
                    // The way out on the right, level with the title's capitals.
                    let height = if self.exit == Exit::Close { 26.0 } else { 30.0 };
                    let drop = title_mid.map_or(0.0, |mid| (mid - top - height / 2.0).max(0.0));
                    ui.with_layout(Layout::top_down(Align::Max), |ui| {
                        ui.add_space(drop);
                        match self.exit {
                            Exit::Close => exit = icon_button(ui, Glyph::Close, true).hover_tip(t("Close")).clicked(),
                            Exit::Back => {
                                exit = back_button(ui, "BACK").hover_tip(t("Back (Esc)")).clicked();
                                if escape_goes_back(ui.ctx()) {
                                    sound::play(ui.ctx(), UiSound::Back);
                                    exit = true;
                                }
                            }
                            Exit::Leave => exit = back_button(ui, "LEAVE").clicked(),
                            Exit::None => {}
                        }
                    });
                });
                // The header rule: a pipe capped at both ends by joints in the accent.
                ui.add_space(space::S);
                let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
                ui.painter().add(metal::pipe_rule(rule.x_range(), rule.center().y, 4.0, Some(self.accent)));
                ui.add_space(space::M);
            }
            // (The footer's room under the body, and the gap above it: the space after the body
            // and a little more.)
            let footer = if footer_height > 0.0 { footer_height + ui.spacing().item_spacing.y + space::M } else { 0.0 };
            let body = match self.max_height {
                None => add_contents(ui),
                Some(height) => {
                    // Down to the plate's foot, less its margin and the footer's room. (An area
                    // offers only the room it took last frame: the height is asked for outright.)
                    let room = (ui.min_rect().min.y - margin.top as f32 + height - margin.bottom as f32 - ui.cursor().min.y - footer).max(48.0);
                    let area = egui::ScrollArea::vertical().id_salt("ta_ui::panel::body").max_height(room).min_scrolled_height(room).auto_shrink([false, true]);
                    crate::scroll::show(ui, area, crate::scroll::Behind::Plate, add_contents).inner
                }
            };
            if footer > 0.0 {
                ui.add_space(space::M);
            }
            (body, add_footer(ui))
        });
        let rect = inner.response.rect;
        ui.painter().set(background, panel_shapes(ui.ctx(), rect));
        // (Whether a text field is being typed in, for the next frame's Escape.)
        let typing = ui.ctx().egui_wants_keyboard_input();
        ui.ctx().data_mut(|d| d.insert_temp(typing_id(), typing));
        PanelResponse { inner: inner.inner, exit, rect }
    }
}

fn typing_id() -> Id {
    Id::new("ta_ui::panel::typing")
}

/// Escape was pressed to go back: not to leave a text field (egui has already let go of its
/// focus, so this asks about the frame before) or to close a drop-down.
fn escape_goes_back(ctx: &Context) -> bool {
    let typing = ctx.data(|d| d.get_temp::<bool>(typing_id())).unwrap_or(false);
    !typing && !egui::Popup::is_any_open(ctx) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
}

// ---- Transitions ---------------------------------------------------------------------------

/// How long a page takes to come in ([`crate::motion::Pace`]'s, at full motion).
pub const PAGE_TIME: f32 = crate::motion::Pace::FULL.page_in;
/// How far a page slides, and a dialog rises.
const PAGE_SLIDE: f32 = 40.0;
const DIALOG_RISE: f32 = 14.0;

/// Which way the player went to the page about to come into view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    /// Deeper in (it slides in from the right).
    #[default]
    Forward,
    /// Back out (from the left).
    Back,
}

fn direction_id() -> Id {
    Id::new("ta_ui::panel::direction")
}

/// Sets the direction the next page to come into view slides in from.
pub fn set_direction(ctx: &Context, direction: Direction) {
    ctx.data_mut(|d| d.insert_temp(direction_id(), direction));
}

/// How far along its entrance whatever is drawn under `id` is: 0 to 1, eased. It starts
/// over whenever it was not drawn the frame before; also returns the direction it came
/// from.
pub fn entrance(ctx: &Context, id: Id, duration: f32) -> (f32, Direction) {
    entrance_held(ctx, id, duration, false)
}

/// [`entrance`], not begun while `held` (nothing of it in sight, its clock waiting): a page or
/// dialog whose layout has not settled yet ([`settled`]).
fn entrance_held(ctx: &Context, id: Id, duration: f32, held: bool) -> (f32, Direction) {
    #[derive(Clone, Copy)]
    struct Shown {
        pass: u64,
        since: f64,
        direction: Direction,
    }
    let key = id.with("ta_ui::panel::entrance");
    let pass = ctx.cumulative_pass_nr();
    let now = ctx.input(|i| i.time);
    let shown = ctx.data_mut(|d| {
        let shown = match d.get_temp::<Shown>(key) {
            Some(s) if s.pass + 1 >= pass && held => Shown { pass, since: now, ..s },
            Some(s) if s.pass + 1 >= pass => Shown { pass, ..s },
            _ => Shown { pass, since: now, direction: d.get_temp(direction_id()).unwrap_or_default() },
        };
        d.insert_temp(key, shown);
        shown
    });
    if held {
        // (Laid out again in this frame, where egui allows another pass: it settles in half
        // the frames, and nothing waits on it long.)
        ctx.request_discard("a new page or dialog settles its layout before it is shown");
        ctx.request_repaint();
        ctx.data_mut(|d| {
            d.insert_temp(since_id(), shown.since);
            d.insert_temp(held_id(), pass);
        });
        return (0.0, shown.direction);
    }
    let t = if crate::motion::reduced(ctx) { 1.0 } else { crate::motion::progress(now, shown.since, duration) };
    if t < 1.0 {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(since_id(), shown.since));
    (crate::motion::out_cubic(t), shown.direction)
}

/// The most frames a page or dialog new to the screen is kept out of sight while its layout
/// settles (a page whose size keeps changing is shown after them all the same).
const SETTLE_FRAMES: u32 = 4;

/// How a page or dialog's layout has come out frame by frame since it came: egui lays a new area
/// out unseen, to measure it, then where that measure puts it (often smaller, shown smaller to
/// fit, or off the middle), and only then where it belongs; shown at once (reduced motion) or
/// fading in, those first frames jumped.
#[derive(Clone, Copy, Debug, Default)]
struct Settle {
    pass: u64,
    frames: u32,
    size: Vec2,
    steady: bool,
}

fn settle_id(id: Id) -> Id {
    id.with("ta_ui::panel::settle")
}

/// Whether what is drawn under `id` was laid out last frame as the frame before (or has been in
/// sight long enough): until then it is drawn out of sight and its entrance waits.
fn settled(ctx: &Context, id: Id) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<Settle>(settle_id(id))).is_some_and(|s| s.pass + 1 >= pass && s.steady)
}

/// Notes how what is drawn under `id` came out this frame (`rect`, its panel as laid out).
fn settle(ctx: &Context, id: Id, rect: Rect) {
    let key = settle_id(id);
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let settle = match d.get_temp::<Settle>(key) {
            Some(s) if s.pass + 1 >= pass => {
                let frames = s.frames + 1;
                // (Once steady it stays so: a page that grows later is not hidden again.)
                let steady = s.steady || (s.size - rect.size()).length() < 1.0 || frames >= SETTLE_FRAMES;
                Settle { pass, frames, size: rect.size(), steady }
            }
            _ => Settle { pass, frames: 0, size: rect.size(), steady: false },
        };
        d.insert_temp(key, settle);
    });
}

fn since_id() -> Id {
    Id::new("ta_ui::panel::since")
}

/// When the page or dialog last drawn (the one being drawn, inside it) began to come into view:
/// for a list in it to build up from ([`stagger`]).
pub fn since(ctx: &Context) -> f64 {
    ctx.data(|d| d.get_temp(since_id())).unwrap_or_default()
}

/// Draws `add` fading and sliding in (from `from`, an offset) with the others of a list:
/// item `index` starts `stagger` seconds after the one before, all counted from `since`
/// (e.g. the list's [`entrance`] start, [`since`]). For menus that build up item by item.
pub fn stagger<R>(ui: &mut Ui, since: f64, index: usize, stagger: f32, from: Vec2, add: impl FnOnce(&mut Ui) -> R) -> R {
    let now = ui.input(|i| i.time);
    let t = if crate::motion::reduced(ui.ctx()) { 1.0 } else { crate::motion::progress(now - index as f64 * stagger as f64, since, PAGE_TIME) };
    if t >= 1.0 {
        // (In its place: drawn as it is, not in a scope of its own.)
        return add(ui);
    }
    ui.ctx().request_repaint();
    let t = crate::motion::out_cubic(t);
    ui.scope(|ui| {
        ui.multiply_opacity(t);
        ui.with_visual_transform(TSTransform::from_translation(from * (1.0 - t)), add).inner
    })
    .inner
}

/// [`stagger`] for the entries of a list in a page or dialog: entry `index` fades in and rises a
/// little, a moment after the one before, as the page comes in ([`since`]); entries far down
/// the list come in with the last of those in sight (`cap`: how many wait their turn).
pub fn list_entry<R>(ui: &mut Ui, index: usize, add: impl FnOnce(&mut Ui) -> R) -> R {
    const CAP: usize = 10;
    let pace = crate::motion::Pace::of(ui.ctx());
    let since = since(ui.ctx());
    stagger(ui, since + 0.06, index.min(CAP), pace.stagger, vec2(0.0, 8.0 * pace.travel), add)
}

// ---- Exits ---------------------------------------------------------------------------------

/// What a page or dialog last looked like (its layer's shapes as last drawn), kept so that it can
/// go out when it is no longer drawn: a page sliding away as the next comes in, a dialog sinking
/// as it closes. Only while [`exits`] runs once a frame (the launcher's windows do without).
#[derive(Clone)]
struct Picture {
    layer: egui::LayerId,
    pass: u64,
    time: f64,
    shapes: std::sync::Arc<Vec<egui::epaint::ClippedShape>>,
    to_global: TSTransform,
    kind: Leaves,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Leaves {
    /// A page: it slides away from the side the next comes in from.
    Page,
    /// A dialog, sinking a little into the middle of its room as it goes.
    Dialog { centre: Pos2 },
}

#[derive(Clone)]
struct Going {
    picture: Picture,
    since: f64,
    /// Which way (a page): the way the one coming in pushes it.
    direction: Direction,
}

#[derive(Clone, Default)]
struct Exits {
    shown: Vec<Picture>,
    going: Vec<Going>,
}

fn exits_id() -> Id {
    Id::new("ta_ui::panel::exits")
}

/// Whether [`exits`] runs (it ran in the pass before this one): the game's windows, not the
/// launcher's. Only there is a page or dialog kept to go out, or held while what comes in its
/// place settles.
fn exits_run(ctx: &Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(exits_id().with("pass"))).is_some_and(|p| p + 1 >= pass)
}

/// Keeps what `layer` looks like this frame, to let it go out when it is no longer drawn.
fn remember(ctx: &Context, layer: egui::LayerId, kind: Leaves) {
    let pass = ctx.cumulative_pass_nr();
    if !exits_run(ctx) {
        return;
    }
    let shapes: Vec<egui::epaint::ClippedShape> = ctx.graphics(|g| g.get(layer).map(|list| list.all_entries().cloned().collect())).unwrap_or_default();
    let picture = Picture {
        layer,
        pass,
        time: ctx.input(|i| i.time),
        shapes: std::sync::Arc::new(shapes),
        to_global: ctx.layer_transform_to_global(layer).unwrap_or(TSTransform::IDENTITY),
        kind,
    };
    ctx.data_mut(|d| {
        let exits = d.get_temp_mut_or_default::<Exits>(exits_id());
        exits.shown.retain(|p| p.layer != layer);
        exits.shown.push(picture);
    });
}

/// Lets the pages and dialogs that were drawn the frame before and not this one go out: each
/// fades and moves away over a moment, as it last looked (nothing in it answers the pointer),
/// once what comes in its place is in sight. Call it once a frame, after every page and dialog
/// has been drawn. With reduced motion they are simply gone then.
pub fn exits(ctx: &Context) {
    let pass = ctx.cumulative_pass_nr();
    let now = ctx.input(|i| i.time);
    let pace = crate::motion::Pace::of(ctx);
    let direction = ctx.data(|d| d.get_temp::<Direction>(direction_id())).unwrap_or_default();
    let mut exits = ctx.data_mut(|d| {
        d.insert_temp(exits_id().with("pass"), pass);
        d.get_temp::<Exits>(exits_id()).unwrap_or_default()
    });
    // Something new held out of sight this frame while its layout settles (`settled`): what is
    // going waits in full view for it, so that the one comes in as the other goes, with no bare
    // frame between them (with reduced motion it is simply gone once the new one is in sight).
    let held = ctx.data(|d| d.get_temp::<u64>(held_id())) == Some(pass);
    let reduced = pace.page_out <= 0.0;
    let shown = std::mem::take(&mut exits.shown);
    for picture in shown {
        if picture.pass == pass {
            exits.shown.push(picture);
        } else if picture.pass + 1 == pass && (now - picture.time) < 0.25 && (!reduced || held) && !exits.going.iter().any(|g| g.picture.layer == picture.layer) {
            // Gone this frame: it goes out from here (unless the clock has jumped, a loading
            // screen between: then it went with the screen it was on).
            exits.going.push(Going { picture, since: now, direction });
        }
    }
    if held {
        // (Only those that have not begun to go: one well on its way is not called back.)
        for going in exits.going.iter_mut().filter(|g| now - g.since < 0.05) {
            going.since = now;
        }
    }
    // (A page or dialog drawn again while going: it is back, no ghost of it.)
    exits.going.retain(|g| {
        let back = exits.shown.iter().any(|p| p.layer == g.picture.layer);
        let len = match g.picture.kind {
            Leaves::Page => pace.page_out,
            Leaves::Dialog { .. } => pace.dialog_out,
        };
        !back && if reduced { held } else { crate::motion::progress(now, g.since, len) < 1.0 }
    });
    for going in &exits.going {
        let len = match going.picture.kind {
            Leaves::Page => pace.page_out,
            Leaves::Dialog { .. } => pace.dialog_out,
        };
        let p = if reduced { 0.0 } else { crate::motion::progress(now, going.since, len) };
        let go = crate::motion::in_cubic(p) * pace.travel;
        let alpha = 1.0 - crate::motion::smooth(p);
        let moved = match going.picture.kind {
            // Pushed the other way from the one coming in: left as a page comes in from the
            // right, right going back.
            Leaves::Page => TSTransform::from_translation(vec2(if going.direction == Direction::Back { 1.0 } else { -1.0 } * PAGE_SLIDE * 0.75 * go, 0.0)),
            Leaves::Dialog { centre } => {
                let scale = 1.0 - 0.03 * go;
                TSTransform::from_translation(centre.to_vec2() + vec2(0.0, DIALOG_RISE * 0.6 * go)) * TSTransform::from_scaling(scale) * TSTransform::from_translation(-centre.to_vec2())
            }
        };
        paint_going(ctx, going, moved, alpha);
    }
    if !exits.going.is_empty() {
        // What is coming in over what is going: the pages and dialogs drawn this frame above
        // the pictures of those going (each new area goes on top of its order), in the order
        // they were drawn.
        for picture in &exits.shown {
            if exits.going.iter().any(|g| g.picture.layer.order == picture.layer.order) {
                ctx.move_to_top(picture.layer);
            }
        }
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(exits_id(), exits));
}

fn held_id() -> Id {
    Id::new("ta_ui::panel::held")
}

/// Forgets what was shown (a loading screen comes, or the screen changes outright): nothing of
/// it goes out over what comes next.
pub fn forget_exits(ctx: &Context) {
    ctx.data_mut(|d| d.remove::<Exits>(exits_id()));
}

/// A page or dialog going: its picture in a layer of its own over where it was, moved by `moved`
/// (in its own coordinates) and faded to `alpha`.
fn paint_going(ctx: &Context, going: &Going, moved: TSTransform, alpha: f32) {
    let layer = going.picture.layer;
    let id = layer.id.with("ta_ui::panel::going");
    let ghost = egui::LayerId::new(layer.order, id);
    // (An area of its own, so that it is drawn in the order of the others; it takes no input.)
    Area::new(id).order(layer.order).interactable(false).fade_in(false).fixed_pos(Pos2::ZERO).show(ctx, |_| {});
    ctx.set_transform_layer(ghost, going.picture.to_global * moved);
    let mut painter = egui::Painter::new(ctx.clone(), ghost, Rect::EVERYTHING);
    painter.set_opacity(alpha);
    for shape in going.picture.shapes.iter() {
        painter.with_clip_rect(shape.clip_rect).add(shape.shape.clone());
    }
}

/// The shapes behind a panel: its shadow, the plate and the pipe frame.
fn panel_shapes(ctx: &Context, rect: Rect) -> Shape {
    let shadow = Shadow { offset: [0, 12], blur: 36, spread: 4, color: Color32::from_black_alpha(170) };
    Shape::Vec(vec![
        Shape::Rect(shadow.as_shape(rect, CornerRadius::same(3))),
        Shape::rect_filled(rect.expand(1.0), CornerRadius::same(3), metal::STEEL_EDGE),
        metal::plate(ctx, rect, PANEL),
        metal::pipe_frame(rect, PANEL_PIPE),
    ])
}

/// A panel centred on the screen over what is behind it (front-end pages): it slides in
/// from the side the player came from ([`set_direction`]) as it fades in.
pub fn page<R>(ctx: &egui::Context, id: impl Hash + std::fmt::Debug, panel: Panel<'_>, add_contents: impl FnOnce(&mut Ui) -> R) -> PanelResponse<R> {
    let id = Id::new(id);
    let (t, direction) = entrance_held(ctx, id, PAGE_TIME, exits_run(ctx) && !settled(ctx, id));
    let slide = if direction == Direction::Back { -PAGE_SLIDE } else { PAGE_SLIDE };
    let shown = Area::new(id)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            // Shown smaller to fit (see `fit_to_screen`), the page is clipped where the screen's
            // edges fall in its own coordinates, not at the screen's size.
            if let Some(to_global) = ctx.layer_transform_to_global(ui.layer_id()) {
                ui.set_clip_rect(to_global.inverse() * ctx.content_rect());
            }
            ui.multiply_opacity(t);
            ui.with_visual_transform(TSTransform::from_translation(vec2(slide * (1.0 - t), 0.0)), |ui| panel.show(ui, add_contents)).inner
        });
    fit_to_screen(ctx, shown.response.layer_id, shown.response.rect, ctx.content_rect());
    settle(ctx, id, shown.response.rect);
    remember(ctx, shown.response.layer_id, Leaves::Page);
    shown.inner
}

/// Space kept round a page that has to shrink to fit.
const FIT_MARGIN: f32 = 10.0;

/// A page or dialog bigger than the window (a small window, or a display scaled up) is shown
/// smaller, all of it, round the middle of `room` (the screen, or the part of it a dialog keeps
/// to) where it is anchored; clicks follow it (the transform is its layer's, which egui applies
/// to the pointer too). `rect` is the panel as laid out.
fn fit_to_screen(ctx: &egui::Context, layer: egui::LayerId, rect: Rect, room: Rect) {
    let fit = room.shrink(FIT_MARGIN);
    let scale = (fit.width() / rect.width().max(1.0)).min(fit.height() / rect.height().max(1.0)).min(1.0);
    let transform = if scale < 0.995 {
        let centre = room.center().to_vec2();
        TSTransform::from_translation(centre) * TSTransform::from_scaling(scale) * TSTransform::from_translation(-centre)
    } else {
        TSTransform::IDENTITY
    };
    if ctx.layer_transform_to_global(layer).unwrap_or(TSTransform::IDENTITY) != transform {
        ctx.set_transform_layer(layer, transform);
        ctx.request_repaint();
    }
}

/// A dialog: the screen behind dims and ignores the pointer while it is open. `exit` is also
/// set by Escape and by a click outside the panel when `dismissable`.
pub fn modal<R>(ctx: &egui::Context, id: impl Hash + std::fmt::Debug, panel: Panel<'_>, dismissable: bool, add_contents: impl FnOnce(&mut Ui) -> R) -> PanelResponse<R> {
    modal_within(ctx, id, panel, dismissable, ctx.content_rect(), add_contents)
}

/// [`modal`] in the middle of `room`, the part of the screen it keeps to (a battle's, clear of
/// its interface's bars), and shown smaller if it does not fit there; the whole screen dims.
pub fn modal_within<R>(ctx: &egui::Context, id: impl Hash + std::fmt::Debug, panel: Panel<'_>, dismissable: bool, room: Rect, add_contents: impl FnOnce(&mut Ui) -> R) -> PanelResponse<R> {
    modal_shown(ctx, id, dismissable, room, |ui| panel.show(ui, add_contents))
}

/// [`modal`] with a footer that stays in sight under its body however that scrolls (its
/// buttons), `footer_height` tall: see [`Panel::show_with_footer`].
pub fn modal_with_footer<R, F>(
    ctx: &egui::Context,
    id: impl Hash + std::fmt::Debug,
    panel: Panel<'_>,
    dismissable: bool,
    footer_height: f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
    add_footer: impl FnOnce(&mut Ui) -> F,
) -> PanelResponse<(R, F)> {
    modal_shown(ctx, id, dismissable, ctx.content_rect(), |ui| panel.show_with_footer(ui, footer_height, add_contents, add_footer))
}

/// A modal dialog in the middle of `room`, `show` drawing its panel.
fn modal_shown<R>(ctx: &egui::Context, id: impl Hash + std::fmt::Debug, dismissable: bool, room: Rect, show: impl FnOnce(&mut Ui) -> PanelResponse<R>) -> PanelResponse<R> {
    let id = Id::new(id);
    // (Opening, as TA's panels did, with a servo's whirr: on the first frame shown.)
    let key = id.with("ta_ui::panel::opened");
    let pass = ctx.cumulative_pass_nr();
    let opened = ctx.data_mut(|d| {
        let last = d.get_temp::<u64>(key);
        d.insert_temp(key, pass);
        last.is_none_or(|last| last + 1 < pass)
    });
    if opened {
        crate::sound::play(ctx, crate::sound::UiSound::Panel);
    }
    let (t, _) = entrance_held(ctx, id, crate::motion::Pace::FULL.dialog_in, exits_run(ctx) && !settled(ctx, id));
    // Dialogs sit in the middle of their room: they grow from just short of their size
    // there, rising into place, as the screen behind darkens.
    let centre = room.center().to_vec2();
    let scale = 0.96 + 0.04 * t;
    let rise = TSTransform::from_translation(centre + vec2(0.0, DIALOG_RISE * (1.0 - t))) * TSTransform::from_scaling(scale) * TSTransform::from_translation(-centre);
    let backdrop = Color32::from_rgba_premultiplied(2, 4, 6, modal_dim(ctx)).gamma_multiply(t);
    let response = egui::Modal::new(id)
        .area(egui::Modal::default_area(id).fade_in(false).anchor(Align2::CENTER_CENTER, room.center() - ctx.content_rect().center()))
        .frame(Frame::NONE)
        // Painted below over the whole screen, which egui's own would not cover once the
        // dialog is shown smaller to fit (see `fit_to_screen`).
        .backdrop_color(Color32::TRANSPARENT)
        .show(ctx, |ui| {
            let screen = ctx.layer_transform_to_global(ui.layer_id()).map_or(ctx.content_rect(), |to_global| to_global.inverse() * ctx.content_rect());
            ui.set_clip_rect(screen);
            ui.painter().rect_filled(screen, 0.0, backdrop);
            ui.multiply_opacity(t);
            ui.with_visual_transform(rise, show).inner
        });
    fit_to_screen(ctx, response.response.layer_id, response.inner.rect, room);
    settle(ctx, id, response.inner.rect);
    remember(ctx, response.response.layer_id, Leaves::Dialog { centre: room.center() });
    let dismissed = dismissable && response.should_close();
    let mut inner = response.inner;
    inner.exit |= dismissed;
    inner
}

/// The margin inside a floating [`strip`]: its pipe frame and a gap.
pub fn strip_margin() -> Margin {
    let pad = metal::pipe_inset(STRIP_PIPE) + space::M;
    Margin::symmetric((pad + 4.0) as i8, pad as i8)
}

/// A floating strip (notices, the replay bar): the panel plate without a header, at a
/// fixed anchor.
pub fn strip<R>(ctx: &egui::Context, id: impl Hash + std::fmt::Debug, anchor: Align2, offset: Vec2, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    strip_area(Area::new(Id::new(id)).anchor(anchor, offset), ctx, 1.0, add_contents)
}

/// [`strip`], `t` (0 to 1) of the way into view: faded and dropping into place.
pub fn strip_shown<R>(ctx: &egui::Context, id: impl Hash + std::fmt::Debug, anchor: Align2, offset: Vec2, t: f32, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    strip_area(Area::new(Id::new(id)).anchor(anchor, offset), ctx, t.clamp(0.0, 1.0), add_contents)
}

/// A floating strip in a free band of a page (`slot`): centred on `centre_x` where it fits,
/// moved along to keep inside the band, its text wrapping at the band's width; it fades and
/// drops into place when it comes into view.
pub fn strip_in<R>(ctx: &egui::Context, id: impl Hash + std::fmt::Debug, slot: Rect, centre_x: f32, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let id = Id::new(id);
    let (t, _) = entrance(ctx, id, PAGE_TIME);
    let area = Area::new(id).pivot(Align2::CENTER_TOP).fixed_pos(pos2(centre_x, slot.min.y)).constrain_to(slot).fade_in(false);
    let width = (slot.width() - strip_margin().sum().x).max(160.0);
    strip_area(area, ctx, t, |ui| {
        ui.set_max_width(width);
        add_contents(ui)
    })
}

/// The plate of a strip in `area`, round `add_contents`: `t` (0 to 1) of the way into view,
/// faded and dropping into place.
fn strip_area<R>(area: Area, ctx: &egui::Context, t: f32, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    area.order(Order::Foreground)
        .show(ctx, |ui| {
            ui.multiply_opacity(t);
            ui.with_visual_transform(TSTransform::from_translation(vec2(0.0, -8.0 * (1.0 - t))), |ui| {
                let background = ui.painter().add(Shape::Noop);
                let inner = Frame::new().inner_margin(strip_margin()).show(ui, add_contents);
                let rect = inner.response.rect;
                let shapes = vec![
                    Shape::Rect(Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(140) }.as_shape(rect, CornerRadius::same(2))),
                    Shape::rect_filled(rect.expand(1.0), CornerRadius::same(3), metal::STEEL_EDGE),
                    metal::plate(ui.ctx(), rect, PANEL),
                    metal::pipe_frame(rect, STRIP_PIPE),
                ];
                ui.painter().set(background, Shape::Vec(shapes));
                inner.inner
            })
            .inner
        })
        .inner
}

// ---- Buttons -------------------------------------------------------------------------------

/// "BACK": a gunmetal plate pointed to the left, as TA's previous buttons are.
pub fn back_button(ui: &mut Ui, label: &str) -> Response {
    let label = t(label);
    let text = back_lettering(ui, label);
    let size = vec2(text.size().x + 44.0, 30.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    reveal_focus(ui, &response);
    if response.clicked() {
        sound::play(ui.ctx(), UiSound::Back);
    }
    if ui.is_rect_visible(rect) {
        let hot = ui.ctx().animate_bool_with_time(response.id.with("hot"), response.hovered(), 0.1);
        let hot = hot.max(crate::widgets::press_light(ui, &response));
        let down = response.is_pointer_button_down_on();
        let painter = ui.painter();
        painter.add(metal::arrow_slab(rect, true, hot, down));
        let center = pos2(rect.min.x + rect.height() * 0.5 + 4.0 + (rect.width() - rect.height() * 0.5 - 4.0) / 2.0, rect.center().y);
        metal::slab_text(painter, center, text, true, Slab::Steel.text(hot), down);
        focus_ring(ui, &response, rect);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A [`back_button`]'s lettering (`label` translated).
fn back_lettering(ui: &Ui, label: &str) -> std::sync::Arc<egui::Galley> {
    galley(ui, RichText::new(label).font(strong(13.5)).extra_letter_spacing(crate::i18n::tracking(1.0)))
}

/// How wide a [`back_button`] lettered `label` is.
fn back_button_width(ui: &Ui, label: &str) -> f32 {
    back_lettering(ui, t(label)).size().x + 44.0
}

/// A large brass call to action ("START BATTLE"), bolted on; dull gunmetal, and inert,
/// when `enabled` is false.
pub fn hero_button(ui: &mut Ui, label: &str, size: Vec2, enabled: bool) -> Response {
    let label = t(label);
    let (rect, response) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    reveal_focus(ui, &response);
    if response.clicked() {
        sound::play(ui.ctx(), UiSound::BigButton);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), enabled && response.hovered(), 0.15);
    let lit = crate::widgets::press_light(ui, &response);
    let down = enabled && response.is_pointer_button_down_on();
    let painter = ui.painter();
    let (kind, color) = if enabled {
        painter.add(metal::slab(ui.ctx(), rect, Slab::Brass, hover.max(lit * 1.6), down));
        (Slab::Brass, ON_AMBER)
    } else {
        painter.add(metal::slab(ui.ctx(), rect, Slab::Steel, 0.0, false));
        painter.rect_filled(rect, CornerRadius::same(2), Color32::from_black_alpha(110));
        (Slab::Steel, STEEL_DIM)
    };
    if size.y >= 40.0 {
        let inset = (size.y * 0.16).clamp(6.0, 9.0);
        for c in [rect.left_top() + vec2(inset, inset), rect.right_top() + vec2(-inset, inset), rect.left_bottom() + vec2(inset, -inset), rect.right_bottom() + vec2(-inset, -inset)] {
            painter.add(metal::bolt(c, (size.y * 0.065).clamp(2.6, 3.6)));
        }
    }
    let text = painter.layout_no_wrap(label.to_owned(), display((size.y * 0.5).round()), color);
    let (tri_w, tri_h, gap) = (size.y * 0.24, size.y * 0.3, size.y * 0.2);
    let left = rect.center().x - (tri_w + gap + text.size().x) / 2.0;
    let cy = rect.center().y + if down { 1.0 } else { 0.0 };
    let triangle = |dx: f32, dy: f32| vec![pos2(left + dx, cy - tri_h / 2.0 + dy), pos2(left + tri_w + dx, cy + dy), pos2(left + dx, cy + tri_h / 2.0 + dy)];
    let edge = if kind.light_text() { Color32::from_black_alpha(190) } else { Color32::from_white_alpha(80) };
    painter.add(Shape::convex_polygon(triangle(0.0, 1.0), edge, Stroke::NONE));
    painter.add(Shape::convex_polygon(triangle(0.0, 0.0), color, Stroke::NONE));
    metal::slab_text(painter, pos2(left + tri_w + gap + text.size().x / 2.0, rect.center().y), text, kind.light_text(), color, down);
    focus_ring(ui, &response, rect);
    if enabled { response.on_hover_cursor(CursorIcon::PointingHand) } else { response }
}

/// An entry of the title menu: a dark slot in a steel rim, as TA's main menu has them, with
/// a lamp that lights when it is pointed at (or focused) and a label that slides right.
/// `detail` is a second, smaller line.
pub fn menu_button(ui: &mut Ui, label: &str, detail: Option<&str>, enabled: bool) -> Response {
    menu_entry(ui, label, detail, enabled, false)
}

/// [`menu_button`], smaller when `compact` (short windows).
pub fn menu_entry(ui: &mut Ui, label: &str, detail: Option<&str>, enabled: bool, compact: bool) -> Response {
    menu_entry_beacon(ui, label, detail, enabled, compact, false)
}

/// [`menu_entry`], and with `beacon` the one to start with: its lamp breathing amber, its rim
/// edged in brass and its line in brass, for a player who has not been here before.
pub fn menu_entry_beacon(ui: &mut Ui, label: &str, detail: Option<&str>, enabled: bool, compact: bool, beacon: bool) -> Response {
    let (label, detail) = (t(label), detail.map(t));
    let height = match (detail.is_some(), compact) {
        (true, false) => 62.0,
        (false, false) => 48.0,
        (true, true) => 50.0,
        (false, true) => 38.0,
    };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    reveal_focus(ui, &response);
    if response.clicked() {
        sound::play(ui.ctx(), UiSound::BigButton);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    // (Keyboard focus lights an entry up like the pointer does.)
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), enabled && (response.hovered() || response.has_focus()), 0.12);
    let down = enabled && response.is_pointer_button_down_on();
    let painter = ui.painter();
    // The steel rim, then the slot inside it.
    painter.add(metal::slab(ui.ctx(), rect, Slab::Steel, hover * 0.6, false));
    let slot = rect.shrink(3.0);
    painter.add(metal::well(slot, Color32::from_rgb(20, 19, 16).lerp_to_gamma(Color32::from_rgb(40, 37, 30), hover)));
    let lamp = pos2(slot.min.x + 15.0, rect.center().y);
    if beacon {
        let time = ui.input(|i| i.time);
        let pulse = 0.55 + 0.45 * (time * 2.2).sin() as f32;
        painter.rect_stroke(rect.shrink(1.0), CornerRadius::same(2), Stroke::new(1.5, AMBER.gamma_multiply(0.55 + 0.45 * pulse)), StrokeKind::Inside);
        painter.add(metal::lamp(lamp, if compact { 3.5 } else { 4.0 }, AMBER, if down { 1.0 } else { pulse.max(hover) }));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(40));
    } else {
        painter.add(metal::lamp(lamp, if compact { 3.5 } else { 4.0 }, GREEN, if down { 1.0 } else { hover * 0.9 }));
    }
    let text_color = if !enabled { STEEL_DIM } else { TEXT_BRIGHT.lerp_to_gamma(Color32::from_rgb(255, 246, 206), hover) };
    let x = slot.min.x + 32.0 + 6.0 * hover;
    // (Up to the chevron; longer words shrink, then are cut short.)
    let room = slot.max.x - 40.0 - x;
    let title = crate::widgets::fitted_galley(ui, |size| RichText::new(label).font(display(size)).extra_letter_spacing(crate::i18n::tracking(2.0)), if compact { 21.0 } else { 25.0 }, room, 0.7);
    let title_y = if detail.is_some() { rect.min.y + if compact { 6.0 } else { 9.0 } } else { rect.center().y - title.size().y / 2.0 };
    metal::embossed(painter, pos2(x, title_y), title, text_color);
    if let Some(detail) = detail {
        let line = crate::widgets::fitted_galley(ui, |size| RichText::new(detail).font(body(size)), if compact { 11.5 } else { 12.5 }, room, 0.8);
        let tone = if !enabled { STEEL_DIM } else if beacon { AMBER_BRIGHT } else { STEEL };
        painter.galley(pos2(x + 1.0, rect.max.y - if compact { 7.0 } else { 10.0 } - line.size().y), line, tone);
    }
    if enabled {
        // A chevron on the right, brass when pointed at.
        let c = pos2(rect.max.x - 22.0 + 4.0 * hover, rect.center().y);
        let chevron = STEEL_DIM.lerp_to_gamma(AMBER_BRIGHT, hover);
        let points = |d: f32| vec![pos2(c.x - 4.0 + d, c.y - 7.0 + d), pos2(c.x + 3.0 + d, c.y + d), pos2(c.x - 4.0 + d, c.y + 7.0 + d)];
        painter.add(Shape::line(points(1.0), Stroke::new(2.5, Color32::from_black_alpha(170))));
        painter.add(Shape::line(points(0.0), Stroke::new(2.2, chevron)));
    }
    focus_ring(ui, &response, rect);
    if enabled { response.on_hover_cursor(CursorIcon::PointingHand) } else { response }
}

/// The last entry of one of the title menu's groups, TA's "previous menu": the same slot as
/// [`menu_entry`], lower, its chevron on the left pointing back and its label in steel.
pub fn menu_back_entry(ui: &mut Ui, label: &str, compact: bool) -> Response {
    let label = t(label);
    let height = if compact { 34.0 } else { 40.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    reveal_focus(ui, &response);
    if response.clicked() {
        sound::play(ui.ctx(), UiSound::Back);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered() || response.has_focus(), 0.12);
    let painter = ui.painter();
    painter.add(metal::slab(ui.ctx(), rect, Slab::Steel, hover * 0.6, false));
    let slot = rect.shrink(3.0);
    painter.add(metal::well(slot, Color32::from_rgb(20, 19, 16).lerp_to_gamma(Color32::from_rgb(40, 37, 30), hover)));
    // The chevron where the other entries have their lamp, moving back when pointed at.
    let c = pos2(slot.min.x + 16.0 - 3.0 * hover, rect.center().y);
    let chevron = STEEL.lerp_to_gamma(AMBER_BRIGHT, hover);
    let points = |d: f32| vec![pos2(c.x + 3.0 + d, c.y - 6.0 + d), pos2(c.x - 3.0 + d, c.y + d), pos2(c.x + 3.0 + d, c.y + 6.0 + d)];
    painter.add(Shape::line(points(1.0), Stroke::new(2.4, Color32::from_black_alpha(170))));
    painter.add(Shape::line(points(0.0), Stroke::new(2.1, chevron)));
    let x = slot.min.x + 32.0 + 4.0 * hover;
    let title = crate::widgets::fitted_galley(ui, |size| RichText::new(label).font(display(size)).extra_letter_spacing(crate::i18n::tracking(2.0)), if compact { 17.0 } else { 19.0 }, slot.max.x - 16.0 - x, 0.7);
    let color = STEEL.lerp_to_gamma(TEXT_BRIGHT, hover);
    metal::embossed(painter, pos2(x, rect.center().y - title.size().y / 2.0), title, color);
    focus_ring(ui, &response, rect);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

// ---- Lists ---------------------------------------------------------------------------------

/// The height of a [`list_row`].
pub const LIST_ROW: f32 = 28.0;

/// A full-width selectable row: `label` on the left, `detail` right-aligned. Selected rows
/// get a brass lead and tint; disabled ones are dimmed and inert.
pub fn list_row(ui: &mut Ui, selected: bool, enabled: bool, label: RichText, detail: Option<RichText>) -> Response {
    row(ui, selected, enabled, None, label, None, detail)
}

/// [`list_row`] numbered `n` of `count`: the numbers in a column of figures, right-aligned,
/// the names after them all starting at one place.
pub fn numbered_row(ui: &mut Ui, selected: bool, enabled: bool, n: usize, count: usize, label: RichText, detail: Option<RichText>) -> Response {
    row(ui, selected, enabled, Some((n, count)), label, None, detail)
}

/// [`list_row`] with a tag before its detail: a few letters, already translated, in a small dark
/// slot of their colour (where a map comes from: "CC").
pub fn tagged_row(ui: &mut Ui, selected: bool, enabled: bool, label: RichText, tag: Option<(&str, Color32)>, detail: Option<RichText>) -> Response {
    row(ui, selected, enabled, None, label, tag, detail)
}

fn row(ui: &mut Ui, selected: bool, enabled: bool, number: Option<(usize, usize)>, label: RichText, tag: Option<(&str, Color32)>, detail: Option<RichText>) -> Response {
    let height = LIST_ROW;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), if enabled { Sense::click() } else { Sense::hover() });
    let text = match number {
        Some((n, _)) => format!("{n} {}", label.text()),
        None => label.text().to_owned(),
    };
    response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, &text));
    reveal_focus(ui, &response);
    if response.clicked() && !selected {
        sound::play(ui.ctx(), UiSound::Toggle);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hot = enabled && response.hovered();
    let painter = ui.painter();
    row_state(painter, rect, selected, hot);
    let color = if !enabled {
        STEEL_DIM
    } else if selected {
        AMBER_BRIGHT
    } else if hot {
        TEXT_BRIGHT
    } else {
        TEXT
    };
    let right = detail.map(|d| galley(ui, d));
    // The number's column, as wide as the largest number's figures.
    let lead = match number {
        Some((n, count)) => {
            let font = egui::TextStyle::Body.resolve(ui.style());
            let column = count.max(1).to_string().len() as f32 * crate::widgets::figure_width(painter, &font);
            crate::widgets::paint_figures(painter, rect.min.x + 12.0 + column, rect.center().y, &n.to_string(), &font, color);
            column + 10.0
        }
        None => 0.0,
    };
    // The tag's lettering and its slot's width (its letters and 5 a side).
    let tag = tag.map(|(text, tint)| {
        let tint = if enabled { tint } else { STEEL_DIM };
        let text = galley(ui, RichText::new(text).font(strong(9.5)).color(tint).extra_letter_spacing(crate::i18n::tracking(0.8)));
        let width = text.size().x + 10.0;
        (text, tint, width)
    });
    let right_width = right.as_ref().map_or(0.0, |g| g.size().x + 12.0);
    let tag_width = tag.as_ref().map_or(0.0, |(_, _, width)| width + 8.0);
    let room = rect.width() - 24.0 - lead - right_width - tag_width;
    let label = egui::WidgetText::from(label).into_galley(ui, Some(egui::TextWrapMode::Truncate), room, egui::TextStyle::Body);
    painter.galley(pos2(rect.min.x + 12.0 + lead, rect.center().y - label.size().y / 2.0), label, color);
    if let Some((text, tint, width)) = tag {
        // (Its right end 8 before the detail, or at the row's margin.)
        let end = rect.max.x - 10.0 - right.as_ref().map_or(0.0, |g| g.size().x + 8.0);
        let slot = Rect::from_min_max(pos2(end - width, rect.center().y - 8.0), pos2(end, rect.center().y + 8.0));
        painter.rect(slot, CornerRadius::same(2), Color32::from_black_alpha(90), Stroke::new(1.0, tint.gamma_multiply(0.45)), egui::StrokeKind::Inside);
        painter.galley((slot.center() - text.size() / 2.0).round(), text, tint);
    }
    if let Some(right) = right {
        painter.galley(pos2(rect.max.x - 10.0 - right.size().x, rect.center().y - right.size().y / 2.0), right, if enabled { STEEL } else { STEEL_DIM });
    }
    focus_ring(ui, &response, rect.shrink(7.0));
    if enabled { response.on_hover_cursor(CursorIcon::PointingHand) } else { response }
}

/// Under a row of a list, what state it is in: chosen, a brass lead on its left and a brass
/// tint fading across it (as every list marks its choice); pointed at, lighter.
pub fn row_state(painter: &egui::Painter, rect: Rect, selected: bool, hot: bool) {
    if selected {
        crate::widgets::gradient_h(painter, rect, AMBER_DEEP, Color32::from_rgba_unmultiplied(60, 43, 19, 40));
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), CornerRadius::ZERO, AMBER);
    } else if hot {
        painter.rect_filled(rect, CornerRadius::same(1), PANEL_HI);
    }
}

/// A recessed well for lists and text (the map list, a briefing): TA's olive-black.
pub fn well<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let background = ui.painter().add(Shape::Noop);
    let inner = Frame::new().inner_margin(Margin::same(6)).show(ui, add_contents);
    ui.painter().set(background, metal::well(inner.response.rect, INSET));
    inner.inner
}

// ---- Text ----------------------------------------------------------------------------------

/// A key as a small gunmetal key cap ("Ctrl+F1", "Right-click").
pub fn keycap(ui: &mut Ui, text: &str) -> Response {
    let text = galley(ui, RichText::new(text).font(strong(12.0)));
    let size = vec2(text.size().x + 14.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.add(metal::slab(ui.ctx(), rect, Slab::Steel, 0.0, false));
    metal::slab_text(painter, rect.center() - vec2(0.0, 0.5), text, true, CREAM, false);
    response
}

/// A dialog's keys, at its foot: each key as a cap and what it does ("Esc" "RESUME"), in a line
/// that wraps where the dialog is narrow.
pub fn key_hints(ui: &mut Ui, hints: &[(&str, &str)]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        for (i, (key, does)) in hints.iter().enumerate() {
            if i > 0 {
                ui.add_space(10.0);
            }
            // (Each pair whole on its line.)
            let label = galley(ui, RichText::new(t(does)).font(strong(11.5)).color(STEEL).extra_letter_spacing(crate::i18n::tracking(0.8)));
            let cap = galley(ui, RichText::new(*key).font(strong(12.0))).size().x + 14.0;
            let width = cap + 6.0 + label.size().x;
            if ui.available_width() < width && ui.cursor().min.x > ui.min_rect().min.x + 1.0 {
                ui.end_row();
            }
            ui.scope(|ui| {
                ui.set_max_width(width);
                ui.horizontal(|ui| {
                    keycap(ui, key);
                    let (rect, _) = ui.allocate_exact_size(label.size(), Sense::hover());
                    ui.painter().galley(rect.min, label, STEEL);
                });
            });
        }
    });
}

/// A section heading inside panels (see [`crate::widgets::card_title`]): set apart from what
/// comes before it, so it reads with what follows.
pub fn section(ui: &mut Ui, title: &str) {
    if ui.cursor().min.y > ui.min_rect().min.y + 1.0 {
        ui.add_space(space::L);
    }
    crate::widgets::card_title(ui, title);
}

/// An engraved seam across the available width.
pub fn rule(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().add(metal::seam(rect.x_range(), rect.center().y));
}

// ---- Settings ------------------------------------------------------------------------------

/// Caption above a control, with an optional hint below it.
pub fn setting(ui: &mut Ui, label: &str, hint: Option<&str>, add: impl FnOnce(&mut Ui)) {
    let top = ui.cursor().min.y;
    ui.label(crate::widgets::caption(label));
    add(ui);
    if let Some(hint) = hint {
        crate::widgets::pretty_label(ui, RichText::new(t(hint)).font(body(12.5)).color(STEEL_DIM));
    }
    crate::scroll::keep_together(ui, Rect::from_x_y_ranges(ui.min_rect().x_range(), top..=ui.min_rect().max.y));
    ui.add_space(10.0);
}

/// Caption and hint on the left, a switch on the right. Returns whether it changed.
pub fn switch_row(ui: &mut Ui, label: &str, hint: &str, value: &mut bool) -> bool {
    let mut changed = false;
    let row = ui.horizontal(|ui| {
        // The hint wraps short of the switch instead of running under it.
        let text_width = (ui.available_width() - 110.0).max(120.0);
        ui.vertical(|ui| {
            ui.set_max_width(text_width);
            ui.label(crate::widgets::caption(label));
            if !hint.is_empty() {
                crate::widgets::pretty_label(ui, RichText::new(t(hint)).font(body(12.5)).color(STEEL_DIM));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // The state's word after the switch, in room for the longer of the two, so the
            // switch stays put when it is flipped.
            let word = |text: &str| galley(ui, RichText::new(t(text)).font(strong(12.0)));
            let room = word("ON").size().x.max(word("OFF").size().x);
            let (state, color) = if *value { (word("ON"), GREEN) } else { (word("OFF"), STEEL) };
            let (slot, _) = ui.allocate_exact_size(vec2(room, state.size().y), Sense::hover());
            ui.painter().galley(slot.min, state, color);
            changed = crate::widgets::toggle(ui, value).changed();
        });
    });
    crate::scroll::keep_together(ui, row.response.rect);
    ui.add_space(10.0);
    changed
}

// ---- Backgrounds ---------------------------------------------------------------------------

/// Darkens the left side and the bottom of `rect` (the title screen over the live battle),
/// so the menu reads on any terrain; `amount` (0 to 1) fades it in and out.
pub fn vignette(painter: &egui::Painter, rect: Rect, amount: f32) {
    let shade = |a: f32| Color32::from_black_alpha((a * amount) as u8);
    crate::widgets::gradient_h(painter, Rect::from_min_max(rect.min, pos2(rect.min.x + rect.width() * 0.45, rect.max.y)), shade(185.0), shade(0.0));
    crate::widgets::gradient_v(painter, Rect::from_min_max(pos2(rect.min.x, rect.max.y - rect.height() * 0.3), rect.max), shade(0.0), shade(150.0));
    crate::widgets::gradient_v(painter, Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + 90.0)), shade(120.0), shade(0.0));
}

/// How dark a [`modal`]'s backdrop is (alpha): the screen behind a dialog that takes the input.
pub const MODAL_DIM: u8 = 178;

/// How dark [`modal`]s' backdrops are (alpha) from now on: lighter behind a picture that steps
/// back from its dialogs itself (the battle's, blurred and graded down). [`MODAL_DIM`] until set.
pub fn set_modal_dim(ctx: &Context, dim: u8) {
    ctx.data_mut(|d| d.insert_temp(Id::new("ta_ui::panel::modal_dim"), dim));
}

fn modal_dim(ctx: &Context) -> u8 {
    ctx.data(|d| d.get_temp(Id::new("ta_ui::panel::modal_dim"))).unwrap_or(MODAL_DIM)
}

/// How much a page's backdrop darkens the battle (alpha) in the middle of the screen and in its
/// corners: lighter than a [`modal`]'s, the battle going on behind.
const PAGE_DIM_MIDDLE: f32 = 64.0;
const PAGE_DIM_CORNERS: f32 = 128.0;

/// Darkens the battle behind a page (`amount` 0 to 1), more towards the screen's edges and
/// corners, so the page is the clear focus with the battle still going on behind it.
pub fn page_backdrop(painter: &egui::Painter, rect: Rect, amount: f32) {
    const ACROSS: u32 = 16;
    const DOWN: u32 = 9;
    let mut mesh = egui::Mesh::default();
    let (centre, half) = (rect.center(), rect.size() / 2.0);
    for j in 0..=DOWN {
        for i in 0..=ACROSS {
            let p = pos2(rect.min.x + rect.width() * i as f32 / ACROSS as f32, rect.min.y + rect.height() * j as f32 / DOWN as f32);
            // (1 at the middle of each edge.)
            let d = vec2((p.x - centre.x) / half.x, (p.y - centre.y) / half.y).length();
            let s = ((d - 0.35) / 0.95).clamp(0.0, 1.0);
            let a = amount * (PAGE_DIM_MIDDLE + (PAGE_DIM_CORNERS - PAGE_DIM_MIDDLE) * s * s * (3.0 - 2.0 * s));
            // (The modal's cool black.)
            mesh.colored_vertex(p, Color32::from_rgba_premultiplied((a * 2.0 / 150.0) as u8, (a * 4.0 / 150.0) as u8, (a * 6.0 / 150.0) as u8, a as u8));
        }
    }
    for j in 0..DOWN {
        for i in 0..ACROSS {
            let k = j * (ACROSS + 1) + i;
            mesh.add_triangle(k, k + 1, k + ACROSS + 2);
            mesh.add_triangle(k, k + ACROSS + 2, k + ACROSS + 1);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// A soft dark pool under text straight on the battle (a footer's line), `rect` the text's.
pub fn scrim(rect: Rect) -> Shape {
    let pool = Shadow { offset: [0, 0], blur: 32, spread: 6, color: Color32::from_black_alpha(150) };
    Shape::Rect(pool.as_shape(rect, CornerRadius::same(12)))
}

/// A title in the display font, outlined and shadowed as TA's are, for text straight on
/// the battle.
pub fn shadowed_title(painter: &egui::Painter, pos: Pos2, text: &str, size: f32, color: Color32) -> Rect {
    let galley = painter.layout_no_wrap(t(text).to_owned(), display(size), color);
    let rect = Rect::from_min_size(pos, galley.size());
    for (dx, dy, a) in [(0.0, 4.0, 90), (2.0, 6.0, 50)] {
        painter.galley_with_override_text_color(pos + vec2(dx, dy), galley.clone(), Color32::from_black_alpha(a));
    }
    metal::embossed(painter, pos, galley, color);
    rect
}

/// A title with a soft coloured glow (VICTORY, DEFEAT), centred in the available width.
pub fn glow_title(ui: &mut Ui, text: &str, size: f32, color: Color32) -> Response {
    let galley = galley(ui, RichText::new(t(text)).font(display(size)).extra_letter_spacing(crate::i18n::tracking(size * 0.08)));
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y), Sense::hover());
    let pos = pos2(rect.center().x - galley.size().x / 2.0, rect.min.y);
    let painter = ui.painter();
    // The glow: the title's letters blurred (made once for the title and its size), in its
    // colour behind it. (Sharp copies of the title round it showed as bands following each
    // letter's outline.)
    if let Some((glow, reach)) = glow_mask(ui.ctx(), &galley, size * 0.065) {
        painter.image(glow.id(), Rect::from_min_size(pos - vec2(reach, reach), galley.size() + vec2(reach, reach) * 2.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), color.gamma_multiply(0.85));
    }
    metal::embossed(painter, pos, galley, color);
    response
}

/// `galley`'s letters (as the font's atlas has them) blurred by `sigma` points, in white with
/// the coverage for alpha, and how far the picture reaches past the galley's box (points); kept
/// for the text, its font and the screen's scale.
fn glow_mask(ctx: &egui::Context, galley: &egui::Galley, sigma: f32) -> Option<(egui::TextureHandle, f32)> {
    let ppp = ctx.pixels_per_point();
    let key = Id::new(("glow_title", galley.job.text.as_str(), galley.job.sections.first().map(|s| s.format.font_id.size.to_bits()), ppp.to_bits(), sigma.to_bits()));
    if let Some(found) = ctx.data(|d| d.get_temp::<(egui::TextureHandle, f32)>(key)) {
        return Some(found);
    }
    let reach = (sigma * 3.0).ceil();
    let pad = (reach * ppp).ceil() as usize;
    let (w, h) = ((galley.size().x * ppp).ceil() as usize + 2 * pad, (galley.size().y * ppp).ceil() as usize + 2 * pad);
    if w * h == 0 || w * h > 4096 * 1024 {
        return None;
    }
    // The letters' coverage, from the atlas, where each is drawn.
    let mut mask = vec![0.0f32; w * h];
    // (A copy of the atlas, once for the title.)
    let atlas = ctx.fonts(|fonts| fonts.image());
    {
        for row in &galley.rows {
            for glyph in &row.row.glyphs {
                let uv = glyph.uv_rect;
                if uv.is_nothing() {
                    continue;
                }
                let at = (row.pos + glyph.pos.to_vec2() + uv.offset) * ppp;
                let (x0, y0) = ((at.x.round() as isize) + pad as isize, (at.y.round() as isize) + pad as isize);
                for ty in uv.min[1] as usize..uv.max[1] as usize {
                    for tx in uv.min[0] as usize..uv.max[0] as usize {
                        let (x, y) = (x0 + (tx - uv.min[0] as usize) as isize, y0 + (ty - uv.min[1] as usize) as isize);
                        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && tx < atlas.size[0] && ty < atlas.size[1] {
                            let cover = atlas.pixels[ty * atlas.size[0] + tx].a() as f32 / 255.0;
                            let m = &mut mask[y as usize * w + x as usize];
                            *m = m.max(cover);
                        }
                    }
                }
            }
        }
    }
    // Blurred, across then down (a Gaussian's weights).
    let s = sigma * ppp;
    let radius = (s * 3.0).ceil() as isize;
    let kernel: Vec<f32> = (-radius..=radius).map(|i| (-(i * i) as f32 / (2.0 * s * s)).exp()).collect();
    let total: f32 = kernel.iter().sum();
    let blur = |src: &[f32], step: (usize, usize), len: usize, lines: usize| -> Vec<f32> {
        let mut out = vec![0.0f32; src.len()];
        for line in 0..lines {
            for i in 0..len {
                let mut sum = 0.0;
                for (k, wgt) in kernel.iter().enumerate() {
                    let j = i as isize + k as isize - radius;
                    if j >= 0 && (j as usize) < len {
                        sum += src[line * step.1 + j as usize * step.0] * wgt;
                    }
                }
                out[line * step.1 + i * step.0] = sum / total;
            }
        }
        out
    };
    let across = blur(&mask, (1, w), w, h);
    let down = blur(&across, (w, 1), h, w);
    let pixels: Vec<Color32> = down.iter().map(|&a| Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
    let texture = ctx.load_texture("glow_title", egui::ColorImage::new([w, h], pixels), egui::TextureOptions::LINEAR);
    let reach = pad as f32 / ppp;
    ctx.data_mut(|d| d.insert_temp(key, (texture.clone(), reach)));
    Some((texture, reach))
}

/// Construction activity with no known percentage. Reduced-motion mode keeps a
/// static unfinished section; hidden or background rails never request frames.
pub fn busy_bar(ui: &mut Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width().max(1.0), 16.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::new(egui::WidgetType::ProgressIndicator));
    crate::progress::paint(ui, rect, None, true);
    response
}

/// Construction progress with a readable percentage; narrow layouts keep valid geometry.
pub fn progress_bar(ui: &mut Ui, fraction: f32, width: f32) -> Response {
    let fraction = crate::progress::fraction(fraction);
    let width = width.min(ui.available_width()).max(1.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    let label = format!("{:.0}%", fraction * 100.0);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, ui.is_enabled(), &label));
    let label_width = width.min(48.0);
    let rail = Rect::from_min_max(rect.min, pos2((rect.max.x - label_width).max(rect.min.x), rect.max.y)).shrink2(vec2(0.0, 4.0));
    crate::progress::paint(ui, rail, Some(fraction), true);
    ui.painter().with_clip_rect(ui.clip_rect().intersect(rect)).text(pos2(rect.max.x, rect.center().y), Align2::RIGHT_CENTER, label, strong(13.0), TEXT_BRIGHT);
    response
}

/// A panel width that fits the screen: `preferred`, or less on a narrow window.
pub fn fit_width(ctx: &egui::Context, preferred: f32) -> f32 {
    preferred.min(ctx.content_rect().width() - 96.0).max(320.0)
}

/// A list or text height that fits the screen: the window height minus `reserved`, between
/// `min` and `max`.
pub fn fit_height(ctx: &egui::Context, reserved: f32, min: f32, max: f32) -> f32 {
    (ctx.content_rect().height() - reserved).clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body taller than the panel scrolls; the footer under it stays inside the panel.
    #[test]
    fn a_footer_stays_in_sight_under_a_tall_body() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        // (Not interface text: kept out of the languages' template.)
        let title = String::from("A DIALOG");
        for frame in 0..3 {
            let mut panel_rect = Rect::NOTHING;
            let mut footer = Rect::NOTHING;
            ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() }, |ui| {
                let response = Panel::new(&title).width(400.0).max_height(360.0).show_with_footer(
                    ui,
                    40.0,
                    |ui| ui.allocate_space(vec2(400.0, 1000.0)),
                    |ui| ui.allocate_exact_size(vec2(400.0, 40.0), Sense::hover()).0,
                );
                panel_rect = response.rect;
                footer = response.inner.1;
            })
            .drop_without_applying_deltas();
            if frame == 2 {
                assert!(panel_rect.height() <= 361.0, "the panel kept to its height: {panel_rect:?}");
                assert!(panel_rect.contains_rect(footer), "the footer in sight: {footer:?} in {panel_rect:?}");
            }
        }
    }

    /// Frames of a page (`shown`) or none, at `time`, with [`exits`] after them as the game runs
    /// it: how many shapes the page's picture going out has, if it is going.
    fn frame_with_page(ctx: &Context, time: f64, shown: bool, dialog: bool) -> Option<usize> {
        let mut going = None;
        // (Not interface text: kept out of the languages' template.)
        let title = String::from("A PAGE");
        ctx.run_ui(egui::RawInput { time: Some(time), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0))), ..Default::default() }, |ui| {
            if shown {
                if dialog {
                    modal(ui.ctx(), "test_dialog", Panel::new(&title).width(300.0), false, |ui| ui.allocate_space(vec2(300.0, 40.0)));
                } else {
                    page(ui.ctx(), "test_page", Panel::new(&title).width(300.0), |ui| ui.allocate_space(vec2(300.0, 40.0)));
                }
            }
            exits(ui.ctx());
            let layer = egui::LayerId::new(if dialog { Order::Foreground } else { Order::Middle }, Id::new(if dialog { "test_dialog" } else { "test_page" }).with("ta_ui::panel::going"));
            going = ui.ctx().graphics(|g| g.get(layer).map(|list| list.all_entries().filter(|s| !matches!(s.shape, Shape::Noop)).count())).filter(|&n| n > 0);
        })
        .drop_without_applying_deltas();
        going
    }

    /// A page no longer drawn goes out over a moment, as it last looked, and is gone after it;
    /// one drawn again has no ghost; a dialog closing does the same.
    #[test]
    fn a_page_or_dialog_that_is_left_goes_out_over_a_moment() {
        for dialog in [false, true] {
            let ctx = Context::default();
            crate::theme::install(&ctx);
            for k in 0..4 {
                assert_eq!(frame_with_page(&ctx, k as f64 * 0.05, true, dialog), None, "shown: nothing going ({dialog})");
            }
            let first = frame_with_page(&ctx, 0.2, false, dialog);
            assert!(first.is_some_and(|n| n > 3), "left: its picture goes out ({dialog}): {first:?}");
            assert!(frame_with_page(&ctx, 0.26, false, dialog).is_some(), "still going a moment later ({dialog})");
            assert_eq!(frame_with_page(&ctx, 0.6, false, dialog), None, "gone after its time ({dialog})");
            // Shown again, and left again: it goes again; shown in the next frame, nothing goes.
            frame_with_page(&ctx, 1.0, true, dialog);
            frame_with_page(&ctx, 1.05, true, dialog);
            assert!(frame_with_page(&ctx, 1.1, false, dialog).is_some());
            assert_eq!(frame_with_page(&ctx, 1.15, true, dialog), None, "back again: no ghost of it ({dialog})");
        }
    }

    /// A page left for another that is still settling out of sight stays in view until the other is
    /// in sight (with reduced motion too, and then it is gone at once): never a frame with neither.
    #[test]
    fn a_page_left_waits_in_sight_for_the_one_coming_in() {
        // Whether `layer` shows anything this frame: a shape not faded to nothing, a text not
        // faded out.
        fn shows(ctx: &Context, layer: egui::LayerId) -> bool {
            ctx.graphics(|g| {
                g.get(layer).is_some_and(|list| {
                    list.all_entries().any(|c| match &c.shape {
                        Shape::Noop => false,
                        Shape::Text(text) => text.opacity_factor > 0.02 && text.override_text_color.or(Some(text.fallback_color)).is_some_and(|c| c.a() > 0),
                        _ => false,
                    })
                })
            })
        }
        for reduced in [false, true] {
            let ctx = Context::default();
            crate::theme::install(&ctx);
            crate::motion::set_reduced(&ctx, reduced);
            // (Not interface text: kept out of the languages' template.)
            let title = String::from("A PAGE");
            // A frame of page `which` at `time`: whether the first page's picture going out is in
            // sight, and whether the second page is.
            let frame = |time: f64, which: &str| {
                let mut out = (false, false);
                ctx.run_ui(egui::RawInput { time: Some(time), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0))), ..Default::default() }, |ui| {
                    page(ui.ctx(), which, Panel::new(&title).width(300.0), |ui| ui.allocate_space(vec2(300.0, 40.0)));
                    exits(ui.ctx());
                    let ghost = egui::LayerId::new(Order::Middle, Id::new("first").with("ta_ui::panel::going"));
                    out.0 = ui.ctx().graphics(|g| g.get(ghost).is_some_and(|list| list.all_entries().any(|s| !matches!(s.shape, Shape::Noop))));
                    out.1 = shows(ui.ctx(), egui::LayerId::new(Order::Middle, Id::new("second")));
                })
                .drop_without_applying_deltas();
                out
            };
            for k in 0..8 {
                frame(k as f64 * 0.03, "first");
            }
            let mut time = 0.3;
            let mut second_seen = false;
            for _ in 0..20 {
                let (first, second) = frame(time, "second");
                assert!(first || second, "a frame with neither in sight at {time} ({reduced})");
                if reduced && second {
                    assert!(!first, "reduced motion: one or the other, not both");
                }
                second_seen |= second;
                time += 0.03;
            }
            assert!(second_seen, "the second came into sight ({reduced})");
            assert!(!frame(time + 1.0, "second").0, "the first's picture is gone in the end ({reduced})");
        }
    }

    /// With reduced motion, after a jump of the clock (a loading screen between) or once the
    /// screen has been forgotten, what is left is simply gone.
    #[test]
    fn nothing_goes_out_with_reduced_motion_or_after_the_screen_changed() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        crate::motion::set_reduced(&ctx, true);
        frame_with_page(&ctx, 0.0, true, false);
        frame_with_page(&ctx, 0.05, true, false);
        assert_eq!(frame_with_page(&ctx, 0.1, false, false), None, "reduced motion");
        crate::motion::set_reduced(&ctx, false);
        frame_with_page(&ctx, 1.0, true, false);
        frame_with_page(&ctx, 1.05, true, false);
        assert_eq!(frame_with_page(&ctx, 3.0, false, false), None, "the clock jumped: it went with its screen");
        frame_with_page(&ctx, 4.0, true, false);
        frame_with_page(&ctx, 4.05, true, false);
        forget_exits(&ctx);
        assert_eq!(frame_with_page(&ctx, 4.1, false, false), None, "forgotten");
    }

    /// A page new to the screen is held out of sight until its layout comes out as it did the
    /// frame before, or for a few frames at most; once in sight it stays, however it grows; and
    /// shown again later, it settles again.
    #[test]
    fn a_new_page_is_held_until_its_layout_settles() {
        let ctx = Context::default();
        let id = Id::new("page");
        // Frames of the page laid out `height` tall (`None`: not drawn): whether it was in sight.
        let frame = |height: Option<f32>| {
            let mut seen = false;
            ctx.run_ui(egui::RawInput::default(), |ui| {
                if let Some(height) = height {
                    seen = settled(ui.ctx(), id);
                    settle(ui.ctx(), id, Rect::from_min_size(Pos2::ZERO, vec2(400.0, height)));
                }
            })
            .drop_without_applying_deltas();
            seen
        };
        // Measured unseen, then too short, then as it belongs: in sight once it is as before.
        let seen: Vec<bool> = [100.0, 300.0, 300.0, 300.0, 500.0, 300.0].into_iter().map(|h| frame(Some(h))).collect();
        assert_eq!(seen, [false, false, false, true, true, true], "held while it settles; then it stays");
        // Gone a frame and back: it settles again.
        frame(None);
        assert!(!frame(Some(300.0)));
        // A page that never comes out the same twice is in sight after a few frames all the same.
        frame(None);
        let seen: Vec<bool> = (0..8).map(|k| frame(Some(100.0 + k as f32 * 10.0))).collect();
        let first = seen.iter().position(|&s| s).expect("in sight in the end");
        assert!(first as u32 <= SETTLE_FRAMES + 1 && seen[first..].iter().all(|&s| s), "{seen:?}");
    }

    /// Where nothing runs [`exits`] (the launcher's windows) a page is never held: it comes in
    /// from its first frame as it always has.
    #[test]
    fn without_exits_a_page_is_not_held() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        // (Not interface text: kept out of the languages' template.)
        let title = String::from("A PAGE");
        let mut held = Vec::new();
        for k in 0..3 {
            ctx.run_ui(egui::RawInput { time: Some(k as f64 * 0.05), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0))), ..Default::default() }, |ui| {
                page(ui.ctx(), "launcher_page", Panel::new(&title).width(300.0), |ui| ui.allocate_space(vec2(300.0, 40.0)));
                held.push(ui.ctx().data(|d| d.get_temp::<u64>(held_id())).is_some());
            })
            .drop_without_applying_deltas();
        }
        assert_eq!(held, [false; 3]);
    }

    #[test]
    fn entrances_run_once_and_start_over_when_shown_again() {
        let ctx = Context::default();
        let id = Id::new("page");
        let frame = |time: f64, shown: bool| {
            let mut t = None;
            ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| {
                if shown {
                    t = Some(entrance(ui.ctx(), id, 0.2));
                }
            })
            .drop_without_applying_deltas();
            t
        };
        set_direction(&ctx, Direction::Back);
        assert_eq!(frame(1.0, true), Some((0.0, Direction::Back)), "a page comes in from the start");
        let (t, _) = frame(1.1, true).unwrap();
        assert!(t > 0.5 && t < 1.0, "halfway in time, most of the way in ({t})");
        assert_eq!(frame(1.3, true).map(|e| e.0), Some(1.0));
        frame(1.4, false);
        set_direction(&ctx, Direction::Forward);
        assert_eq!(frame(1.5, true), Some((0.0, Direction::Forward)), "shown again: from the start, the new way");
        crate::motion::set_reduced(&ctx, true);
        assert_eq!(frame(1.51, true), Some((1.0, Direction::Forward)), "the current animation finishes immediately");
        frame(1.6, false);
        assert_eq!(frame(1.7, true), Some((1.0, Direction::Forward)), "new pages appear without an entrance");
    }
}
