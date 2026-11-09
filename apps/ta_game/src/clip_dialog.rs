//! The replay's EXPORT CLIP dialog (`hud.rs`, the replay bar): a social media clip's length (30
//! or 60 seconds) and shape (vertical or horizontal), then the stretches of the battle the game
//! found for it (`clips::candidates`, once the replay's scan is done), the best first and chosen:
//! each a card with a picture of its climax, when it is and what happens in it, and a line of how
//! the action builds. The pictures are taken by the app (`app.rs`, `ClipPreviews`): it plays the
//! replay to each climax behind the dialog and draws it as the clip will show it, in both shapes
//! at once, and hands them back ([`ClipDialog::put_picture`]). EXPORT CLIP asks for the clip as a
//! film (`film.rs`, [`crate::film::FilmSettings::clip`]).

use egui::{Align, Align2, Color32, Layout, Rect, Shape as EShape, Stroke, StrokeKind, pos2, vec2};
use glam::Vec2;
use ta_ui::i18n::{n, t, tf};
use ta_ui::widgets::ActionButton;

use crate::clips::{Candidate, ClipSpec, Shape};
use crate::film::{ClipChoice, FilmSettings};
use crate::highlights::MomentKind;
use crate::theme::*;

/// The dialog's column of labels (points), as the export dialog's.
const LABELS: f32 = 118.0;
/// The dialog's width, and the gap between cards.
const WIDTH: f32 = 600.0;
const GAP: f32 = 10.0;
/// The pictures of the climaxes: how large they are drawn (pixels, on their long side).
pub const PICTURE_LONG: u32 = 480;

/// What the dialog keeps from one clip to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipChoices {
    pub spec: ClipSpec,
    pub titles: bool,
    pub scoreboard: bool,
    pub sounds: bool,
    pub music: bool,
}

impl Default for ClipChoices {
    /// The game's music is left out to begin with: video sites may mute a clip that has it.
    fn default() -> ClipChoices {
        ClipChoices { spec: ClipSpec::default(), titles: true, scoreboard: true, sounds: true, music: false }
    }
}

/// A picture of a climax for the app to take: the battle's tick, where the camera looks and how
/// far the action spreads, and the clip's start and length (the camera's heading then).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewShot {
    pub tick: u64,
    pub focus: Vec2,
    pub spread: f32,
    pub from: u64,
    pub seconds: f32,
}

/// The dialog while open.
pub struct ClipDialog {
    pub choices: ClipChoices,
    /// The stretches found for a length and shape (worked out once the scan is done), and the one
    /// chosen.
    found: Option<(ClipSpec, Vec<Candidate>)>,
    chosen: usize,
    /// Pictures taken of climaxes, by tick and shape; those asked of the app, by tick.
    pictures: Vec<((u64, Shape), egui::TextureHandle)>,
    asked: Vec<u64>,
    /// Pictures to ask for (taken up by the app).
    wanted: Vec<PreviewShot>,
}

/// What the dialog was told.
pub enum Answer {
    Stay,
    Close,
    Export(FilmSettings),
}

/// A shape's name in the dialog: "Vertical 9:16".
pub fn shape_name(shape: Shape) -> &'static str {
    match shape {
        Shape::Vertical => n("Vertical 9:16"),
        Shape::Horizontal => n("Horizontal 16:9"),
    }
}

impl ClipDialog {
    pub fn new(choices: ClipChoices) -> ClipDialog {
        ClipDialog { choices, found: None, chosen: 0, pictures: Vec::new(), asked: Vec::new(), wanted: Vec::new() }
    }

    /// The pictures to take, asked for since the last call.
    pub fn take_wanted(&mut self) -> Vec<PreviewShot> {
        std::mem::take(&mut self.wanted)
    }

    /// A picture taken (`rgba`, `width` × `height`) of the climax at `tick` in `shape`.
    pub fn put_picture(&mut self, ctx: &egui::Context, tick: u64, shape: Shape, rgba: &[u8], width: usize, height: usize) {
        let image = egui::ColorImage::from_rgba_unmultiplied([width, height], rgba);
        let texture = ctx.load_texture(format!("clip_picture_{tick}_{shape:?}"), image, egui::TextureOptions::LINEAR);
        self.pictures.retain(|(key, _)| *key != (tick, shape));
        self.pictures.push(((tick, shape), texture));
    }

    /// Whether the pictures asked for are all taken.
    pub fn pictures_taken(&self) -> bool {
        self.asked.iter().all(|&tick| self.pictures.iter().any(|((t, _), _)| *t == tick))
    }

    fn picture(&self, tick: u64, shape: Shape) -> Option<&egui::TextureHandle> {
        self.pictures.iter().find(|(key, _)| *key == (tick, shape)).map(|(_, texture)| texture)
    }

    /// The stretches for the chosen length and shape, worked out (and their pictures asked for)
    /// once the scan is done; the one chosen kept across a change of shape (the same climax).
    fn find(&mut self, end: u64, scan: Option<&crate::replay_scan::ScanState>) {
        let spec = self.choices.spec;
        if self.found.as_ref().is_some_and(|(s, _)| *s == spec) {
            return;
        }
        let Some(scan) = scan.filter(|s| s.finished) else { return };
        let Some(log) = scan.action.as_ref() else {
            self.found = Some((spec, Vec::new()));
            return;
        };
        let was = self.found.as_ref().and_then(|(_, found)| found.get(self.chosen)).map(|c| c.climax);
        let found = crate::clips::candidates(log, &scan.marks, end, spec);
        for c in &found {
            if !self.asked.contains(&c.climax) {
                self.asked.push(c.climax);
                self.wanted.push(PreviewShot { tick: c.climax, focus: c.focus, spread: c.spread, from: c.from, seconds: (c.to - c.from) as f32 / 30.0 });
            }
        }
        self.chosen = was.and_then(|tick| found.iter().position(|c| c.climax == tick)).unwrap_or(0);
        self.found = Some((spec, found));
    }
}

/// The dialog over a replay `end` ticks long, with its scan (while it looks: how far it has got).
pub fn dialog(ctx: &egui::Context, d: &mut ClipDialog, end: u64, scan: Option<&crate::replay_scan::ScanState>) -> Answer {
    use ta_ui::widgets::{check_box_named, segmented};
    d.find(end, scan);
    let row = |ui: &mut egui::Ui, name: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            let (label, _) = ui.allocate_exact_size(vec2(LABELS, 28.0), egui::Sense::hover());
            ui.painter().text(label.left_center(), Align2::LEFT_CENTER, t(name), strong(11.5), STEEL);
            add(ui);
        });
        ui.add_space(3.0);
    };
    let note = |ui: &mut egui::Ui, text: &str| {
        ui.horizontal(|ui| {
            ui.add_space(LABELS + ui.spacing().item_spacing.x);
            ui.label(egui::RichText::new(text).font(body(12.5)).color(STEEL));
        });
        ui.add_space(4.0);
    };
    let spec = d.choices.spec;
    let (w, h) = spec.shape.size();
    let chosen = d.found.as_ref().and_then(|(_, found)| found.get(d.chosen)).cloned();
    let sound = (d.choices.sounds || d.choices.music).then_some(192_000);
    let folder = {
        let dir = crate::clips::clips_dir();
        let parts: Vec<String> = dir.components().rev().take(3).map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
        parts.into_iter().rev().collect::<Vec<_>>().join("\\")
    };
    let about = tf("{width} × {height} · about {size} · saved in {folder}", &[("width", &w), ("height", &h), ("size", &crate::film::size_text(spec.bytes(sound))), ("folder", &folder)]);
    let panel = ta_ui::panel::Panel::new(t("Export clip")).width(WIDTH).exit(ta_ui::panel::Exit::Close).max_height((ctx.content_rect().height() - 24.0).max(260.0));
    let mut export = false;
    let mut cancel = false;
    let response = ta_ui::panel::modal_with_footer(
        ctx,
        "clip_dialog",
        panel,
        true,
        74.0,
        |ui| {
            let mut seconds = d.choices.spec.seconds;
            row(ui, n("LENGTH"), &mut |ui| {
                segmented(ui, &mut seconds, &[(30, n("30 seconds")), (60, n("60 seconds"))]);
            });
            d.choices.spec.seconds = seconds;
            let mut shape = d.choices.spec.shape;
            row(ui, n("SHAPE"), &mut |ui| {
                segmented(ui, &mut shape, &[(Shape::Vertical, shape_name(Shape::Vertical)), (Shape::Horizontal, shape_name(Shape::Horizontal))]);
            });
            d.choices.spec.shape = shape;
            note(ui, match shape {
                Shape::Vertical => t("Fills a phone held upright: TikTok, Reels and Shorts"),
                Shape::Horizontal => t("For wide players: YouTube, X and the like"),
            });
            ui.add_space(4.0);
            ui.label(ta_ui::widgets::caption(t("THE BEST MOMENTS, CHOSEN FOR YOU")).color(STEEL));
            ui.add_space(6.0);
            cards(ui, d, end, scan);
            ui.add_space(8.0);
            row(ui, n("ON THE PICTURE"), &mut |ui| {
                check_box_named(ui, &mut d.choices.titles, n("Titles"));
                ui.label(egui::RichText::new(t("Titles")).font(body(13.5)).color(TEXT));
                ui.add_space(10.0);
                check_box_named(ui, &mut d.choices.scoreboard, n("Scoreboard"));
                ui.label(egui::RichText::new(t("Scoreboard")).font(body(13.5)).color(TEXT));
            });
            row(ui, n("SOUND"), &mut |ui| {
                check_box_named(ui, &mut d.choices.sounds, n("Battle sounds"));
                ui.label(egui::RichText::new(t("Battle sounds")).font(body(13.5)).color(TEXT));
                ui.add_space(10.0);
                check_box_named(ui, &mut d.choices.music, n("Music"));
                ui.label(egui::RichText::new(t("Music")).font(body(13.5)).color(TEXT));
            });
            if d.choices.music {
                note(ui, t("Video sites may mute a clip that has the game's music"));
            }
        },
        |ui| {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(&about).font(body(12.5)).color(STEEL));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add(ActionButton::secondary(n("CANCEL"))).clicked() {
                    cancel = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(chosen.is_some(), ActionButton::primary(n("EXPORT CLIP"))).clicked() {
                        export = true;
                    }
                });
            });
        },
    );
    match chosen {
        Some(c) if export => {
            let film = FilmSettings { titles: d.choices.titles, scoreboard: d.choices.scoreboard, sounds: d.choices.sounds, music: d.choices.music, ..Default::default() };
            Answer::Export(film.for_clip(ClipChoice { spec: d.choices.spec, from: c.from, to: c.to, climax: c.climax, focus: c.focus }))
        }
        _ if cancel || response.exit => Answer::Close,
        _ => Answer::Stay,
    }
}

/// The stretches found as cards (four across for the tall pictures, two for the wide), a click
/// choosing one; or how the search goes, or that there is nothing to show.
fn cards(ui: &mut egui::Ui, d: &mut ClipDialog, end: u64, scan: Option<&crate::replay_scan::ScanState>) {
    let width = ui.available_width();
    let line = |ui: &mut egui::Ui, text: String| {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 120.0), egui::Sense::hover());
        ui.painter().add(ta_ui::metal::well(rect, INSET));
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, body(14.0), TEXT);
    };
    let found = match &d.found {
        Some((_, found)) => found.clone(),
        None => {
            let percent = scan.map_or(0, |s| (s.done * 100 / end.max(1)) as u32);
            line(ui, tf("Finding the battle's best moments… {percent}%", &[("percent", &percent)]));
            ui.ctx().request_repaint();
            return;
        }
    };
    if found.is_empty() {
        line(ui, t("This replay has no fighting to make a clip of.").to_string());
        return;
    }
    let shape = d.choices.spec.shape;
    let columns = match shape {
        Shape::Vertical => 4,
        Shape::Horizontal => 2,
    };
    let card_w = (width - GAP * (columns - 1) as f32) / columns as f32;
    let (pw, ph) = shape.size();
    // (The picture inset 5 points within the card, at its own shape.)
    let picture_h = (card_w - 10.0) * ph as f32 / pw as f32 + 10.0;
    // (The wide cards' words never wrap: less room under them.)
    let card_h = picture_h + if columns == 4 { 98.0 } else { 86.0 };
    for row in found.chunks(columns).enumerate() {
        let (r, cards) = row;
        let (band, _) = ui.allocate_exact_size(vec2(width, card_h), egui::Sense::hover());
        for (i, c) in cards.iter().enumerate() {
            let index = r * columns + i;
            let rect = Rect::from_min_size(pos2(band.left() + i as f32 * (card_w + GAP), band.top()), vec2(card_w, card_h));
            let response = ui.interact(rect, ui.id().with(("clip_card", index)), egui::Sense::click());
            if response.clicked() {
                d.chosen = index;
            }
            let picture = d.picture(c.climax, shape).map(|t| t.id());
            card(ui, rect, picture_h, c, picture, index == 0, index == d.chosen, response.hovered());
            response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        ui.add_space(GAP);
    }
    if !d.pictures_taken() {
        ui.ctx().request_repaint();
    }
}

/// A stretch's card in `rect`: its picture (`picture_h` high; or a plate while it is taken), a
/// BEST mark on the first, when it is, what happens, how the action builds.
#[allow(clippy::too_many_arguments)]
fn card(ui: &egui::Ui, rect: Rect, picture_h: f32, c: &Candidate, picture: Option<egui::TextureId>, best: bool, chosen: bool, hovered: bool) {
    let p = ui.painter();
    p.rect_filled(rect, 4.0, if chosen { PANEL_HI } else { PANEL });
    let frame = Rect::from_min_size(rect.min + vec2(5.0, 5.0), vec2(rect.width() - 10.0, picture_h - 10.0));
    p.add(ta_ui::metal::well(frame.expand(2.0), INSET));
    match picture {
        Some(id) => {
            p.image(id, frame, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            // (A slow shimmer while the picture is taken.)
            let time = ui.input(|i| i.time) as f32;
            let glow = 0.5 + 0.5 * (time * 2.4 + rect.left() * 0.02).sin();
            p.rect_filled(frame, 2.0, Color32::from_white_alpha((6.0 + 10.0 * glow) as u8));
            let words = p.layout(t("Taking a picture…").to_string(), body(12.0), STEEL, frame.width() - 12.0);
            p.galley(frame.center() - words.size() / 2.0, words, STEEL);
        }
    }
    if best {
        let mark = p.layout_no_wrap(t("BEST").to_string(), strong(11.0), ON_AMBER);
        let plate = Rect::from_min_size(frame.min + vec2(6.0, 6.0), mark.size() + vec2(12.0, 6.0));
        p.rect_filled(plate, 3.0, AMBER_BRIGHT);
        p.galley(plate.min + vec2(6.0, 3.0), mark, ON_AMBER);
    }
    // When, what happens, what it cost.
    let mut y = rect.top() + picture_h + 2.0;
    let room = rect.width() - 12.0;
    for (text, font, colour) in [
        (c.span_text(), strong(12.5), TEXT_BRIGHT),
        (t(c.title).to_string(), display(15.0), match c.kind {
            MomentKind::Nuke => Color32::from_rgb(255, 120, 96),
            MomentKind::Commander => CREAM,
            MomentKind::Battle => AMBER_BRIGHT,
        }),
        (c.detail(), body(11.5), STEEL),
    ] {
        let g = p.layout(text, font, colour, room);
        p.galley(pos2(rect.left() + 6.0, y), g.clone(), colour);
        y += g.size().y + 2.0;
    }
    // How the action builds, the climax marked.
    let graph = Rect::from_min_max(pos2(rect.left() + 6.0, rect.bottom() - 22.0), pos2(rect.right() - 6.0, rect.bottom() - 6.0));
    if c.curve.len() > 1 {
        let x = |i: usize| graph.left() + graph.width() * i as f32 / (c.curve.len() - 1) as f32;
        let points: Vec<egui::Pos2> = c.curve.iter().enumerate().map(|(i, v)| pos2(x(i), graph.bottom() - graph.height() * v.clamp(0.0, 1.0))).collect();
        let mut fill = egui::Mesh::default();
        for (i, pt) in points.iter().enumerate() {
            fill.colored_vertex(*pt, AMBER.gamma_multiply(0.35));
            fill.colored_vertex(pos2(pt.x, graph.bottom()), AMBER.gamma_multiply(0.05));
            if i > 0 {
                let k = (i * 2) as u32;
                fill.add_triangle(k - 2, k - 1, k);
                fill.add_triangle(k - 1, k + 1, k);
            }
        }
        p.add(EShape::mesh(fill));
        p.add(EShape::line(points, Stroke::new(1.5, AMBER_BRIGHT)));
        let at = ((c.climax.saturating_sub(c.from)) as f32 / (c.to - c.from).max(1) as f32).clamp(0.0, 1.0);
        p.circle_filled(pos2(graph.left() + graph.width() * at, graph.top() + 2.0), 3.0, RED);
    }
    let edge = if chosen { Stroke::new(2.0, AMBER_BRIGHT) } else if hovered { Stroke::new(1.0, LINE_HI) } else { Stroke::new(1.0, LINE) };
    p.rect_stroke(rect, 4.0, edge, StrokeKind::Inside);
}
