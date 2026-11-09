//! The same loading screen at launch and between benchmark scenes. It needs only egui,
//! so the window can paint it before opening the game's archives or creating its renderer.
//! The caller supplies time; capture runs can hold a frame or advance it deterministically.
//!
//! The screen is a bay of the launcher's dark plates inside its pipe frame. The emblem is
//! being built: two nano-lathes on either side of it play their green beams on a build front
//! rising through it (the part still to come dimmed, outlined in green), it stands finished a
//! moment, and the next pass begins; a gauge ring turns slowly round it. Under it the stage
//! of loading, a construction rail at work, and the field notes on a riveted plate.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::Arc;
use std::time::Duration;

use egui::{Color32, Context, Galley, Id, LayerId, Mesh, Order, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

use crate::i18n::{n, t};
use crate::metal;
use crate::theme::*;

/// Hints without fixed key bindings or mouse-button assumptions. These also appear in
/// the battle's tips; using their existing English keys shares the translations.
const HINTS: &[&str] = &[
    n("Keep your metal and energy incomes above what you spend: building slows down for everything when a store runs dry."),
    n("Radar towers show enemy units long before they come into sight: build one early."),
    n("Guarding a factory with a constructor adds its build power to the factory's production."),
    n("Wrecks can be resurrected by resurrection kbots, or reclaimed for most of their metal."),
    n("Wind generators pay off on windy maps: the skirmish page shows each map's wind range."),
    n("Anti-air matters: bombers can wreck an undefended economy in one pass."),
    n("An advanced factory needs a lot of metal: build it once your extractors and power can feed it."),
    n("Metal makers turn spare energy into metal, and stop when your energy runs dry."),
];

const HINT_SECONDS: f64 = 8.0;

/// The build of the emblem: the front rises for `BUILD` seconds, the emblem stands finished
/// for `HOLD`, and the part to build dims back in over `RESET` before the next pass.
const BUILD: f32 = 3.0;
const HOLD: f32 = 1.5;
const RESET: f32 = 0.7;

/// The nano-lathe's green.
const NANO: Color32 = Color32::from_rgb(120, 236, 98);

/// The small heading distinguishes a benchmark from a normal game launch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadingKind {
    Game,
    Benchmark,
}

/// Paint-only presentation. Loading, cancellation, window events and the lifetime of
/// the screen belong to the caller; showing it never performs or waits for loading work.
pub struct LoadingScreen {
    kind: LoadingKind,
    hint_offset: usize,
    seasonal: Option<(ta_config::Launcher, bool)>,
}

impl LoadingScreen {
    pub const fn new(kind: LoadingKind) -> Self {
        Self { kind, hint_offset: 0, seasonal: None }
    }

    /// Starts later transitions on a different tip, without depending on wall-clock time.
    pub fn with_hint_offset(mut self, offset: usize) -> Self {
        self.set_hint_offset(offset);
        self
    }

    pub fn set_hint_offset(&mut self, offset: usize) {
        self.hint_offset = offset % HINTS.len();
    }

    /// Follow the same theme and opt-out as the launcher, including its calendar mode.
    /// Copy only presentation settings; a scene transition can change its hint without
    /// losing the seasonal preference or changing the accessibility announcement.
    pub fn with_seasonal(mut self, settings: &ta_config::Launcher, reduced_flashes: bool) -> Self {
        self.set_seasonal(settings, reduced_flashes);
        self
    }

    pub fn set_seasonal(&mut self, settings: &ta_config::Launcher, reduced_flashes: bool) {
        self.seasonal = Some((ta_config::Launcher {
            seasonal_effects: settings.seasonal_effects,
            seasonal_theme: settings.seasonal_theme,
            ..Default::default()
        }, reduced_flashes));
    }

    /// Covers the viewport and requests another animation frame. Install [`crate::theme`]
    /// on the context first. `status` and `detail` are already translated by the caller,
    /// which can include a scene name or an actual stage of loading. No percentage or
    /// estimated time is invented. `elapsed` is seconds on the caller's animation clock.
    pub fn show(&self, ctx: &Context, status: &str, detail: &str, elapsed: f64) {
        self.show_leaving(ctx, status, detail, elapsed, 0.0);
    }

    /// [`Self::show`], `leaving` (0 to 1) of the way out: everything on the bay fades, and the
    /// bay is left as it is, to lift off what comes next ([`curtain`]).
    ///
    /// As it comes up, its parts settle in a moment after the words (which are there from the
    /// first frame): the emblem and its name, then the field notes rising onto their plate. A
    /// new stage of loading takes the place of the one before, rising out as it comes in. With
    /// reduced motion all of it is simply there.
    pub fn show_leaving(&self, ctx: &Context, status: &str, detail: &str, elapsed: f64, leaving: f32) {
        let screen = ctx.viewport_rect();
        if !screen.is_finite() || screen.width() <= 0.0 || screen.height() <= 0.0 {
            return;
        }
        let elapsed = if elapsed.is_finite() { elapsed.max(0.0) } else { 0.0 };
        let still = crate::motion::still(ctx);
        let time = if still { 0.0 } else { elapsed };
        if let Some((settings, reduced_flashes)) = &self.seasonal {
            crate::seasonal::begin(ctx, settings, *reduced_flashes);
        }
        let bay = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("ta_ui::loading"))).with_clip_rect(screen);
        let (scale, pipe, frame, path) = bay_measures(screen);
        let margin = 40.0 * scale;
        let width = (screen.width() - margin * 2.0).clamp(1.0, 660.0 * scale.max(1.0));
        let x = screen.center().x;
        let compact = screen.height() < 360.0;

        // The bay: the launcher's dark plates lit from above, and its pipe frame round them.
        bay.add(metal::backdrop(ctx, screen, BG_DEEP));
        bay.add(metal::pipe_frame(frame, pipe));
        // Everything on it, fading as the screen goes.
        let mut painter = bay.clone();
        painter.multiply_opacity(1.0 - crate::motion::smooth(leaving));
        let painter = painter;
        // How far each part has come in (all of it at once when held still).
        let arrive = |delay: f64, len: f64| if still { 1.0 } else { crate::motion::out_cubic(((time - delay) / len) as f32) };
        if self.seasonal.is_some() {
            // Particles stay in the outer gutters. Status, hints and translated text
            // remain on the quiet centre panel even on small or high-DPI viewports.
            let gutter = ((screen.width() - width) * 0.5 - 12.0 * scale).max(0.0);
            for edge in [
                Rect::from_min_max(screen.min, pos2(screen.min.x + gutter, screen.max.y)),
                Rect::from_min_max(pos2(screen.max.x - gutter, screen.min.y), screen.max),
            ] {
                if edge.is_positive() {
                    crate::seasonal::background(&painter.with_clip_rect(edge), edge);
                }
            }
            let (left, right) = (path.min.x + 24.0 * scale, path.max.x - 24.0 * scale);
            crate::seasonal::trim(&painter, left, right, path.min.y - pipe * 0.5, pipe, 10.0 * scale);
            crate::seasonal::trim(&painter, left, right, path.max.y - pipe * 0.5, pipe, (screen.max.y - path.max.y - pipe * 0.5 - 1.0).max(0.0));
        }
        let heading = match self.kind {
            LoadingKind::Game => t("LOADING GAME"),
            LoadingKind::Benchmark => t("BENCHMARK"),
        };
        let spoken_status = if detail.is_empty() { status.to_owned() } else { format!("{status}: {detail}") };

        // Layout uses the actual translated text heights. Long scene names and hints
        // wrap rather than overflowing, including in a narrow or high-DPI window.
        let name = centered(&painter, t("TOTAL ANNIHILATION"), display(36.0 * scale), TEXT_BRIGHT, width);
        let reborn = centered(&painter, t("REBORN"), display(23.0 * scale), AMBER, width);
        let status = centered(&painter, status, strong((20.0 * scale).max(13.0)), TEXT, width);
        let detail = centered(&painter, detail, body((15.0 * scale).max(12.0)), STEEL, width);
        let pad = 18.0 * scale;
        let hint_font = body((15.0 * scale).max(12.0));
        let (note_label, note_colour, note) = self.note(ctx, time);
        let hint = balanced(&painter, t(note), hint_font, TEXT, (width - pad * 2.0).max(1.0));
        let hint_label = centered(&painter, t(note_label), strong((11.0 * scale).max(10.0)), note_colour, width);
        let plate_height = 12.0 * scale + hint_label.size().y + 9.0 * scale + hint.size().y + 14.0 * scale;
        let plate_width = (hint.size().x.max(hint_label.size().x + 40.0 * scale) + pad * 2.0).min(width);
        let emblem_height = if compact { 0.0 } else { 186.0 * scale };
        let name_height = if compact { 0.0 } else { name.size().y + reborn.size().y + 4.0 * scale };
        let content_height = emblem_height + name_height + 26.0 * scale + status.size().y
            + 9.0 * scale + detail.size().y + 30.0 * scale;
        // The emblem, the words, the rail and the notes as one group in the bay's middle, the
        // notes kept off the bottom pipe.
        let gap = 46.0 * scale;
        let top = path.min.y + metal::pipe_inset(pipe);
        let bottom = path.max.y - metal::pipe_inset(pipe) - 10.0 * scale;
        let content_top = top + ((bottom - top - content_height - gap - plate_height) * 0.5).max(margin * 0.6);
        let plate_top = (content_top + content_height + gap).min(bottom - plate_height);
        let plate = Rect::from_min_size(pos2(x - plate_width * 0.5, plate_top), vec2(plate_width, plate_height));

        // The heading on a small plate riveted over the frame's top pipe.
        let heading = centered(&painter, heading, strong((11.0 * scale).max(10.0)), CREAM, width);
        let tag = Rect::from_center_size(pos2(x, path.min.y), heading.size() + vec2(28.0 * scale, 10.0 * scale));
        painter.add(metal::riveted_plate(ctx, tag, PANEL_HI, 1.0));
        paint_centered(&painter, heading.clone(), x, tag.center().y - heading.size().y * 0.5);

        let mut y = content_top;
        if !compact {
            // The emblem settles in from a little smaller as the gauge round it comes up, and
            // its name after it.
            let come = arrive(0.0, 0.42);
            let mut emblem = painter.clone();
            emblem.multiply_opacity(come);
            let centre = pos2(x, y + 88.0 * scale);
            crate::widgets::soft_glow(&emblem, centre, 8.0 * scale, 150.0 * scale, Color32::from_rgba_unmultiplied(216, 168, 72, 34));
            gauge(&emblem, centre, scale * (0.92 + 0.08 * come), time);
            let size = 116.0 * scale;
            build(&emblem, centre, size, scale, time, still);
            if self.seasonal.is_some() {
                crate::seasonal::logo(&emblem, Rect::from_center_size(centre, vec2(size, size)));
            }
            y += emblem_height;
            let mut named = painter.clone();
            let words = arrive(0.08, 0.4);
            named.multiply_opacity(words);
            let lift = (1.0 - words) * 6.0 * scale;
            paint_centered(&named, name.clone(), x, y + lift);
            y += name.size().y + 4.0 * scale;
            paint_centered(&named, reborn.clone(), x, y + lift);
            y += reborn.size().y;
        }
        y += 26.0 * scale;
        let status_top = y;
        // A steady green lamp: work in hand.
        let lamp_at = pos2(x - status.size().x * 0.5 - 14.0 * scale, y + status.size().y * 0.5);
        painter.add(metal::lamp(lamp_at, 4.0 * scale.max(0.7), GREEN, 1.0));
        // A new stage takes the place of the one before: the old words rise out as the new
        // come up (`STAGE_CHANGE`); held still, the new ones are simply there.
        let (before, change) = said(ctx, (status.text(), detail.text()), elapsed);
        let change = if still { 1.0 } else { change };
        let detail_top = y + status.size().y + 9.0 * scale;
        if let Some((old_status, old_detail)) = before.filter(|_| change < 1.0) {
            let mut going = painter.clone();
            going.multiply_opacity(1.0 - crate::motion::smooth(change * 1.6));
            let rise = crate::motion::out_cubic(change) * 7.0 * scale;
            let old_status = centered(&painter, &old_status, strong((20.0 * scale).max(13.0)), TEXT, width);
            let old_detail = centered(&painter, &old_detail, body((15.0 * scale).max(12.0)), STEEL, width);
            paint_centered(&going, old_status.clone(), x, y - rise);
            paint_centered(&going, old_detail, x, y + old_status.size().y + 9.0 * scale - rise);
        }
        let mut coming = painter.clone();
        coming.multiply_opacity(crate::motion::smooth(change * 1.3));
        let drop = (1.0 - crate::motion::out_cubic(change)) * 7.0 * scale;
        paint_centered(&coming, status.clone(), x, y + drop);
        y = detail_top;
        paint_centered(&coming, detail.clone(), x, y + drop);
        // Painter text has no widget metadata. Publish the actual loading stage on one
        // stable live node, so assistive technology can follow the same window through
        // startup and scene transitions. Animated decoration and rotating hints never
        // change this node or interrupt the announcement.
        ctx.accesskit_node_builder(Id::new("ta_ui::loading_status"), |node| {
            node.set_role(egui::accesskit::Role::Status);
            node.set_label(spoken_status);
            node.set_live(egui::accesskit::Live::Polite);
            node.set_live_atomic();
            node.set_bounds(egui::accesskit::Rect {
                x0: (x - width * 0.5) as f64, y0: status_top as f64,
                x1: (x + width * 0.5) as f64, y1: (y + detail.size().y) as f64,
            });
        });
        y += detail.size().y + 24.0 * scale;
        let rail = Rect::from_center_size(pos2(x, y), vec2((420.0 * scale).min(width), (14.0 * scale).max(8.0)));
        crate::progress::paint_at(&painter, rail, None, true, time);
        for end in [rail.left_center() - vec2(9.0 * scale, 0.0), rail.right_center() + vec2(9.0 * scale, 0.0)] {
            painter.add(metal::bolt(end, (3.4 * scale).max(2.0)));
        }

        // The field notes on their plate (dressed for the season, in season), rising onto it a
        // moment after the rest has come. Tips fade only at their boundary, then remain
        // completely legible for most of the eight-second interval; the lamp dims as they
        // change, taking the next heading's colour, and a hairline along the plate's foot runs
        // out the note's time. There is no time-dependent texture allocation.
        let rise = arrive(0.16, 0.48);
        let mut notes = painter.clone();
        notes.multiply_opacity(rise);
        let plate = plate.translate(vec2(0.0, (1.0 - rise) * 12.0 * scale));
        notes.add(metal::riveted_plate(ctx, plate, PANEL, 0.6));
        let well = plate.shrink(5.0 * scale);
        notes.add(metal::well(well, Color32::from_black_alpha(70)));
        if self.seasonal.is_some() {
            crate::seasonal::plate(&notes, plate, scale);
        }
        let phase = time.rem_euclid(HINT_SECONDS);
        let fade = if time < 0.4 { 1.0 } else { (phase / 0.4).min((HINT_SECONDS - phase) / 0.4).clamp(0.0, 1.0) as f32 };
        let mut tip_painter = notes.clone();
        tip_painter.multiply_opacity(fade);
        let label_top = plate.min.y + 12.0 * scale;
        paint_centered(&tip_painter, hint_label.clone(), x, label_top);
        notes.add(metal::lamp(pos2(x - hint_label.size().x * 0.5 - 11.0 * scale, label_top + hint_label.size().y * 0.5), 3.0 * scale.max(0.7), note_colour, 0.35 + 0.65 * fade));
        paint_centered(&tip_painter, hint, x, label_top + hint_label.size().y + 9.0 * scale);
        if !still {
            // The note's time running out along the well's foot, in its heading's colour.
            let run = Rect::from_min_max(pos2(well.min.x + 3.0 * scale, well.max.y - 3.0 * scale), pos2(well.max.x - 3.0 * scale, well.max.y - 2.0 * scale));
            let gone = (phase / HINT_SECONDS) as f32;
            let end = egui::lerp(run.x_range(), gone);
            notes.rect_filled(Rect::from_min_max(run.min, pos2(end, run.max.y)), 0.0, note_colour.gamma_multiply(0.42 * fade));
        }
        crate::motion::next_frame(ctx, Duration::from_millis(16));
    }

    /// The note shown at `time`, with its plate's heading and the heading's colour. In season the
    /// season's own funny notes take turns with the usual ones, the season's first.
    fn note(&self, ctx: &Context, time: f64) -> (&'static str, Color32, &'static str) {
        let step = (time / HINT_SECONDS).floor() as usize;
        match self.seasonal.as_ref().and(crate::seasonal::notes(ctx)) {
            Some((label, colour, notes)) if step.is_multiple_of(2) => (label, colour, notes[(step / 2 + self.hint_offset) % notes.len()]),
            Some(_) => (n("FIELD NOTES"), AMBER, HINTS[(step / 2 + self.hint_offset) % HINTS.len()]),
            None => (n("FIELD NOTES"), AMBER, HINTS[(step + self.hint_offset) % HINTS.len()]),
        }
    }
}

/// How long a new stage of loading takes to replace the one before (seconds).
const STAGE_CHANGE: f32 = 0.3;

/// The bay's measures on `screen`: the scale of everything on it, its pipe's width, the frame's
/// rect and the pipe's path round it.
fn bay_measures(screen: Rect) -> (f32, f32, Rect, Rect) {
    let scale = (screen.height() / 720.0).min(screen.width() / 960.0).clamp(0.45, 1.3);
    let pipe = (5.0 * scale).max(3.0);
    let frame = screen.shrink((10.0 * scale).max(4.0));
    (scale, pipe, frame, metal::pipe_frame_path(frame, pipe))
}

/// What the screen says (its stage and detail) and what it said before, kept on the context:
/// the words before (when they have changed) and how far on the change is (1: done).
fn said(ctx: &Context, now: (&str, &str), elapsed: f64) -> (Option<(String, String)>, f32) {
    #[derive(Clone, Default)]
    struct Said {
        now: (String, String),
        before: Option<(String, String)>,
        since: f64,
    }
    let key = Id::new("ta_ui::loading::said");
    ctx.data_mut(|d| {
        let said = match d.get_temp::<Said>(key) {
            // (The first words: there from the first frame.)
            None => Said { now: (now.0.to_owned(), now.1.to_owned()), before: None, since: f64::NEG_INFINITY },
            Some(s) if (s.now.0.as_str(), s.now.1.as_str()) != now => Said { before: Some(s.now), now: (now.0.to_owned(), now.1.to_owned()), since: elapsed },
            // (A clock gone back: a new screen on this context.)
            Some(s) if elapsed < s.since => Said { before: None, since: f64::NEG_INFINITY, ..s },
            Some(s) => s,
        };
        let change = crate::motion::progress(elapsed, said.since, STAGE_CHANGE);
        let before = said.before.clone();
        d.insert_temp(key, said);
        (before, change)
    })
}

/// The loading screen's bay (its plates and pipe frame, as [`LoadingScreen`] draws them under
/// everything else) over the whole window at `alpha`: what a screen that has finished loading
/// leaves, lifting off what was loaded. Drawn over everything else.
pub fn curtain(ctx: &Context, alpha: f32) {
    let screen = ctx.viewport_rect();
    if alpha <= 0.0 || !screen.is_finite() || !screen.is_positive() {
        return;
    }
    let mut painter = ctx.layer_painter(LayerId::new(Order::Debug, Id::new("ta_ui::loading::curtain"))).with_clip_rect(screen);
    painter.set_opacity(alpha.min(1.0));
    let (_, pipe, frame, _) = bay_measures(screen);
    painter.add(metal::backdrop(ctx, screen, BG_DEEP));
    painter.add(metal::pipe_frame(frame, pipe));
}

fn centered(painter: &Painter, text: &str, font: egui::FontId, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, width);
    job.halign = egui::Align::Center;
    painter.layout_job(job)
}

/// [`centered`], its lines as even as they can be: as narrow as it can be laid out in as few
/// lines as at `width` (no last word left alone on a line).
fn balanced(painter: &Painter, text: &str, font: egui::FontId, color: Color32, width: f32) -> Arc<Galley> {
    let full = centered(painter, text, font.clone(), color, width);
    let rows = full.rows.len();
    if rows < 2 {
        return full;
    }
    let (mut lo, mut hi) = (width / rows as f32 * 0.8, width);
    for _ in 0..10 {
        let mid = (lo + hi) * 0.5;
        if centered(painter, text, font.clone(), color, mid).rows.len() > rows { lo = mid } else { hi = mid }
    }
    centered(painter, text, font, color, hi + 1.0)
}

fn paint_centered(painter: &Painter, text: Arc<Galley>, x: f32, y: f32) {
    // Center-aligned galleys use their origin as the centre of each row.
    painter.galley(pos2(x, y).round(), text, Color32::WHITE);
}

/// How far the emblem at work ([`paint_emblem_at_work`]) reaches round its centre at `scale` 1:
/// the gauge's arcs, and the nano-lathes clamped to its sides.
pub const EMBLEM_AT_WORK_REACH: f32 = 96.0;

/// The emblem at work as this screen has it, for other screens (the launcher's About page): its
/// glow, the gauge ring turning round it, and the nano-lathes either side building it pass after
/// pass; `scale` 1 is this screen's own size, `time` the animation's clock (seconds). Held `still`
/// it stands finished, the lathes resting.
pub fn paint_emblem_at_work(painter: &Painter, centre: Pos2, scale: f32, time: f64, still: bool) {
    crate::widgets::soft_glow(painter, centre, 8.0 * scale, 150.0 * scale, Color32::from_rgba_unmultiplied(216, 168, 72, 34));
    gauge(painter, centre, scale, time);
    build(painter, centre, 116.0 * scale, scale, time, still);
}

/// The gauge round the emblem: a dark track, brass ticks turning slowly (once a minute) and
/// two brass arcs running round it the other way.
fn gauge(painter: &Painter, centre: Pos2, scale: f32, time: f64) {
    let phase = (time.rem_euclid(60.0) as f32) * TAU / 60.0;
    let r = 84.0 * scale;
    painter.circle_stroke(centre, r, Stroke::new(7.0 * scale, Color32::from_black_alpha(120)));
    painter.circle_stroke(centre, r + 3.5 * scale, Stroke::new(1.0, Color32::from_rgba_unmultiplied(216, 180, 86, 40)));
    for i in 0..72 {
        let angle = phase + i as f32 * TAU / 72.0;
        let long = i % 6 == 0;
        let (r0, r1) = if long { (r - 3.5 * scale, r + 3.0 * scale) } else { (r - 2.0 * scale, r + 1.5 * scale) };
        let dir = vec2(angle.cos(), angle.sin());
        let colour = if long { Color32::from_rgba_unmultiplied(226, 188, 96, 170) } else { Color32::from_rgba_unmultiplied(160, 150, 128, 70) };
        painter.line_segment([centre + dir * r0, centre + dir * r1], Stroke::new((if long { 1.6 } else { 1.0 }) * scale.max(0.7), colour));
    }
    let run = (time.rem_euclid(16.0) as f32) * TAU / 16.0;
    for (radius, start, length, alpha) in [(r - 8.0 * scale, -run - FRAC_PI_2, 0.62, 170), (r + 8.0 * scale, run * 0.6 - FRAC_PI_2, 0.20, 80)] {
        let points = (0..=48).map(|i| {
            let angle = start + TAU * length * i as f32 / 48.0;
            centre + vec2(angle.cos(), angle.sin()) * radius
        }).collect();
        painter.add(Shape::line(points, Stroke::new(1.4 * scale, Color32::from_rgba_unmultiplied(216, 180, 86, alpha))));
    }
}

/// The emblem's hexagon (pointy at top and bottom): its circumradius is 0.496 of the
/// emblem's square (`branding/emblem.svg`).
fn hexagon(centre: Pos2, size: f32) -> [Pos2; 6] {
    let r = size * 0.496;
    let w = r * 0.866;
    [
        centre + vec2(0.0, -r), centre + vec2(w, -r * 0.5), centre + vec2(w, r * 0.5),
        centre + vec2(0.0, r), centre + vec2(-w, r * 0.5), centre + vec2(-w, -r * 0.5),
    ]
}

/// The part of the convex `polygon` above the line `y` (as the screen goes: smaller y).
fn above(polygon: &[Pos2], y: f32) -> Vec<Pos2> {
    let mut out = Vec::new();
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
        if a.y <= y { out.push(a); }
        if (a.y < y) != (b.y < y) {
            let t = (y - a.y) / (b.y - a.y);
            out.push(a + (b - a) * t);
        }
    }
    out
}

/// The emblem being built: the nano-lathes either side play their beams on a front rising
/// through it, the part still to come dimmed and outlined in green; then it stands finished
/// and the next pass begins. Held still, it is simply finished.
fn build(painter: &Painter, centre: Pos2, size: f32, scale: f32, time: f64, still: bool) {
    crate::widgets::paint_emblem(painter, Rect::from_center_size(centre, vec2(size, size)));
    let hexagon = hexagon(centre, size);
    // The lathes ride the gauge ring at its sides, a little below its middle.
    let ring = 84.0 * scale;
    let lathes = [centre + vec2(-ring * 0.966, ring * 0.259), centre + vec2(ring * 0.966, ring * 0.259)];
    let cycle = BUILD + HOLD + RESET;
    let t = (time.rem_euclid(cycle as f64)) as f32;
    let working = !still && t < BUILD;
    for (side, at) in lathes.iter().enumerate() {
        nano_lathe(painter, *at, scale, side == 0, if working { 1.0 } else { 0.35 }, time);
    }
    if still {
        return;
    }
    let (top, bottom) = (centre.y - size * 0.496, centre.y + size * 0.496);
    // How much of the emblem is still to build, and how dim that part is.
    let (front, dim) = if t < BUILD {
        let s = t / BUILD;
        (bottom + (top - bottom) * (s * s * (3.0 - 2.0 * s)), 1.0)
    } else if t < BUILD + HOLD {
        (top, 0.0)
    } else {
        (bottom, ((t - BUILD - HOLD) / RESET).clamp(0.0, 1.0))
    };
    let todo = above(&hexagon, front);
    if todo.len() >= 3 && dim > 0.0 {
        painter.add(Shape::convex_polygon(todo.clone(), Color32::from_black_alpha((150.0 * dim) as u8), Stroke::NONE));
        // The frame still to fill, drawn in the lathe's green (all but its edge on the front).
        for i in 0..todo.len() {
            let (a, b) = (todo[i], todo[(i + 1) % todo.len()]);
            if (a.y - front).abs() < 0.5 && (b.y - front).abs() < 0.5 { continue; }
            painter.line_segment([a, b], Stroke::new(1.2 * scale, NANO.gamma_multiply(0.55 * dim)));
        }
    }
    if !working {
        return;
    }
    // The front: the width of the hexagon at its height, a bright line over a glow below it.
    let edge = |y: f32| -> (f32, f32) {
        let r = size * 0.496;
        let dy = (y - centre.y).abs();
        let half = if dy <= r * 0.5 { r * 0.866 } else { (r - dy) * 1.732 };
        (centre.x - half.max(0.0), centre.x + half.max(0.0))
    };
    let (l, r) = edge(front);
    if r - l > 1.0 {
        let glow = 14.0 * scale;
        let mut mesh = Mesh::default();
        let (lb, rb) = edge((front + glow).min(bottom));
        mesh.colored_vertex(pos2(l, front), NANO.gamma_multiply(0.36));
        mesh.colored_vertex(pos2(r, front), NANO.gamma_multiply(0.36));
        mesh.colored_vertex(pos2(rb, front + glow), Color32::TRANSPARENT);
        mesh.colored_vertex(pos2(lb, front + glow), Color32::TRANSPARENT);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(Shape::mesh(mesh));
        painter.line_segment([pos2(l, front), pos2(r, front)], Stroke::new(1.8 * scale, Color32::from_rgb(196, 255, 176)));
        // The beams, each from its lathe's lens to an end of the front, sparks where they meet it.
        for (side, at) in lathes.iter().enumerate() {
            let lens = *at + vec2(if side == 0 { 12.0 } else { -12.0 } * scale, -6.0 * scale);
            let target = pos2(if side == 0 { l + 3.0 * scale } else { r - 3.0 * scale }, front);
            beam(painter, lens, target, scale, time, side as u32);
        }
    }
}

/// A nano-lathe clamped to the gauge ring: a gunmetal turret with a brass collar and a green
/// lens aimed in at the emblem, lit as strongly as `power` (0 to 1).
fn nano_lathe(painter: &Painter, at: Pos2, scale: f32, left: bool, power: f32, time: f64) {
    let s = scale;
    let toward = if left { 1.0 } else { -1.0 };
    // The clamp over the ring, and the turret's barrel in toward the emblem.
    painter.add(metal::joint(at + vec2(-4.0 * toward * s, 2.0 * s), 11.0 * s));
    let barrel = [at + vec2(2.0 * toward * s, -2.0 * s), at + vec2(12.0 * toward * s, -6.0 * s)];
    painter.line_segment(barrel, Stroke::new(5.5 * s, metal::STEEL_EDGE));
    painter.line_segment(barrel, Stroke::new(3.5 * s, metal::STEEL_SHADE));
    painter.circle_filled(at, 9.0 * s, metal::STEEL_EDGE);
    painter.circle_filled(at, 7.6 * s, metal::STEEL_SHADE);
    painter.circle_filled(at + vec2(-2.0 * s, -2.4 * s), 4.0 * s, metal::STEEL_BODY.gamma_multiply(0.7));
    painter.circle_stroke(at, 7.6 * s, Stroke::new(1.6 * s, Color32::from_rgb(190, 150, 70)));
    let breath = 0.82 + 0.18 * ((time as f32) * TAU / 1.6).sin() * power;
    let lens = barrel[1];
    painter.circle_filled(lens, 5.0 * s * power.max(0.5), NANO.gamma_multiply(0.18 * power * breath));
    painter.add(metal::lamp(lens, 2.4 * s, NANO, (power * breath).clamp(0.2, 1.0)));
}

/// A nano-lathe's beam: a soft green ray with motes running down it to its target, and sparks
/// thrown where it meets the work.
fn beam(painter: &Painter, from: Pos2, to: Pos2, scale: f32, time: f64, seed: u32) {
    let s = scale;
    let t = time as f32;
    painter.line_segment([from, to], Stroke::new(6.0 * s, NANO.gamma_multiply(0.10)));
    painter.line_segment([from, to], Stroke::new(2.6 * s, NANO.gamma_multiply(0.30)));
    painter.line_segment([from, to], Stroke::new(1.0 * s, Color32::from_rgb(206, 255, 190).gamma_multiply(0.85)));
    for i in 0..6 {
        let f = (t * 1.9 + i as f32 / 6.0 + seed as f32 * 0.37).fract();
        painter.circle_filled(from + (to - from) * f, (1.3 + 0.6 * (1.0 - f)) * s, Color32::from_rgb(220, 255, 205).gamma_multiply(0.75 * (1.0 - f * 0.5)));
    }
    for i in 0..7u32 {
        let k = seed * 31 + i * 7;
        let life = (t * 2.3 + hash(k) ).fract();
        let angle = -FRAC_PI_2 + (hash(k + 1) - 0.5) * 2.6;
        let speed = (12.0 + 16.0 * hash(k + 2)) * s;
        let p = to + vec2(angle.cos(), angle.sin()) * speed * life + vec2(0.0, 26.0 * s * life * life);
        let fade = 1.0 - life;
        painter.circle_filled(p, (0.8 + 0.9 * fade) * s, Color32::from_rgb(255, 226, 150).gamma_multiply(0.9 * fade));
    }
}

fn hash(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846ca68b);
    value ^= value >> 16;
    (value >> 8) as f32 / 16777216.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_with_screen(ctx: &Context, size: egui::Vec2, time: f64, screen: &LoadingScreen) -> egui::FullOutput {
        ctx.run_ui(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time: Some(time),
            ..Default::default()
        }, |ui| {
            screen.show(ui.ctx(), "Preparing the next scene", "Combined arms on Sapphire Reach", time);
        })
    }

    fn shapes(ctx: &Context, size: egui::Vec2, time: f64, screen: &LoadingScreen) -> Vec<egui::epaint::ClippedShape> {
        let mut output = frame_with_screen(ctx, size, time, screen);
        output.textures_delta.clear();
        output.shapes
    }

    #[test]
    fn loading_text_fits_small_and_large_viewports() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        for theme in [None, Some(ta_config::SeasonalTheme::Christmas), Some(ta_config::SeasonalTheme::NewYear), Some(ta_config::SeasonalTheme::Halloween)] {
            let mut screen = LoadingScreen::new(LoadingKind::Benchmark);
            if let Some(theme) = theme {
                screen.set_seasonal(&ta_config::Launcher { seasonal_theme: theme, ..Default::default() }, false);
            }
            for size in [vec2(320.0, 240.0), vec2(640.0, 360.0), vec2(640.0, 480.0), vec2(1280.0, 720.0), vec2(1920.0, 1080.0)] {
                let bounds = Rect::from_min_size(Pos2::ZERO, size);
                let mut out = frame_with_screen(&ctx, size, 2.0, &screen);
                out.textures_delta.clear();
                let particle_clips: Vec<_> = out.shapes.iter().map(|shape| shape.clip_rect)
                    .filter(|clip| clip.width() < bounds.width() * 0.5).collect();
                let mut text_bounds = Vec::new();
                for clipped in out.shapes {
                    if let Shape::Text(text) = clipped.shape {
                        let rect = text.galley.rect.translate(text.pos.to_vec2());
                        assert!(bounds.contains_rect(rect), "text outside {size:?}: {:?} {rect:?}", text.galley.job.text);
                        assert!((rect.center().x - bounds.center().x).abs() < 1.0, "text must stay centered");
                        assert!(particle_clips.iter().all(|clip| !clip.intersects(rect)), "seasonal particles must stay outside loading text");
                        text_bounds.push(rect);
                    }
                }
                text_bounds.sort_by(|a, b| a.min.y.total_cmp(&b.min.y));
                for pair in text_bounds.windows(2) {
                    assert!(pair[0].max.y <= pair[1].min.y, "loading labels overlap in {size:?}: {pair:?}");
                }
            }
        }
    }

    /// The texts painted at `time` with the stage `status`, `leaving` of the way out, each with
    /// whether it is seen (not faded to nothing).
    fn texts(ctx: &Context, time: f64, status: &str, leaving: f32) -> Vec<(String, bool)> {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0))), time: Some(time), ..Default::default() }, |ui| {
            LoadingScreen::new(LoadingKind::Game).show_leaving(ui.ctx(), status, "Coast To Coast", time, leaving);
        });
        out.textures_delta.clear();
        out.shapes
            .into_iter()
            .filter_map(|c| match c.shape {
                Shape::Text(text) => Some((text.galley.job.text.clone(), text.opacity_factor > 0.02 && text.fallback_color.a() > 0)),
                _ => None,
            })
            .collect()
    }

    /// The words are there from the first frame; the emblem's name and the notes come in after
    /// them; a new stage replaces the one before over a moment, and the old is gone after it.
    #[test]
    fn the_screen_comes_up_and_its_stages_follow_one_another() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        let seen = |texts: &[(String, bool)], word: &str| texts.iter().any(|(t, s)| t == word && *s);
        let first = texts(&ctx, 0.0, "Preparing graphics", 0.0);
        assert!(seen(&first, "Preparing graphics") && seen(&first, "Coast To Coast"), "the stage from the first frame");
        assert!(!seen(&first, "FIELD NOTES"), "the notes come after");
        let up = texts(&ctx, 1.0, "Preparing graphics", 0.0);
        assert!(seen(&up, "FIELD NOTES") && seen(&up, "TOTAL ANNIHILATION"), "everything up within a second");
        // A new stage: the old words still there as it begins, gone once it is done.
        let change = texts(&ctx, 2.0, "Loading units and textures", 0.0);
        assert!(seen(&change, "Preparing graphics"), "the old stage is still going");
        let done = texts(&ctx, 2.0 + STAGE_CHANGE as f64 + 0.05, "Loading units and textures", 0.0);
        assert!(seen(&done, "Loading units and textures") && !done.iter().any(|(t, _)| t == "Preparing graphics"), "{done:?}");
        // Leaving: nothing of the words left on the bay.
        let gone = texts(&ctx, 3.0, "Loading units and textures", 1.0);
        assert!(gone.iter().all(|(_, s)| !s), "the words fade off the bay: {gone:?}");
    }

    /// Held still (reduced motion), it is all there at once, and a new stage simply replaces the
    /// old.
    #[test]
    fn held_still_everything_is_there_at_once() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        crate::motion::set_reduced(&ctx, true);
        let first = texts(&ctx, 0.0, "Preparing graphics", 0.0);
        assert!(["Preparing graphics", "FIELD NOTES", "TOTAL ANNIHILATION"].iter().all(|w| first.iter().any(|(t, s)| t == w && *s)), "{first:?}");
        let change = texts(&ctx, 0.5, "Loading units and textures", 0.0);
        assert!(!change.iter().any(|(t, _)| t == "Preparing graphics"), "no old words left over: {change:?}");
    }

    /// The bay's curtain paints the bay over the window, and nothing when it has lifted.
    #[test]
    fn the_curtain_is_the_bay() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        let count = |alpha: f32| {
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0))), ..Default::default() }, |ui| curtain(ui.ctx(), alpha));
            out.textures_delta.clear();
            out.shapes.len()
        };
        assert!(count(1.0) > 0 && count(0.4) > 0);
        assert_eq!(count(0.0), 0);
    }

    #[test]
    fn seasonal_loading_keeps_opt_out_and_hint_changes() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        crate::motion::set_reduced(&ctx, true);
        let size = vec2(1280.0, 720.0);
        let settings = ta_config::Launcher { seasonal_effects: false, seasonal_theme: ta_config::SeasonalTheme::NewYear, ..Default::default() };
        let mut screen = LoadingScreen::new(LoadingKind::Benchmark).with_seasonal(&settings, true);
        screen.set_hint_offset(3);
        let plain = LoadingScreen::new(LoadingKind::Benchmark).with_hint_offset(3);
        // Warm fonts and the emblem before comparing paint commands.
        frame_with_screen(&ctx, size, 0.0, &screen).textures_delta.clear();
        let baseline = shapes(&ctx, size, 0.0, &plain);
        let opted_out = shapes(&ctx, size, 0.0, &screen);
        assert_eq!(opted_out, baseline, "a named theme cannot override opt-out");

        let settings = ta_config::Launcher { seasonal_effects: true, ..settings };
        screen.set_seasonal(&settings, true);
        screen.set_hint_offset(5);
        let themed = shapes(&ctx, size, 1.0, &screen);
        assert!(themed.len() > baseline.len(), "changing a transition hint must retain seasonal decoration");
        let expected = LoadingScreen::new(LoadingKind::Benchmark).with_hint_offset(5).with_seasonal(&settings, true);
        assert_eq!(themed, shapes(&ctx, size, 1.0, &expected));
        assert_eq!(screen.note(&ctx, HINT_SECONDS).2, HINTS[5]);

        assert_eq!(shapes(&ctx, size, 2.0, &plain), baseline,
            "a plain loading screen must not inherit stale decoration from this context");
        screen.set_seasonal(&ta_config::Launcher { seasonal_effects: false, ..settings }, true);
        screen.set_hint_offset(3);
        assert_eq!(shapes(&ctx, size, 2.0, &screen), baseline,
            "disabling seasonal effects clears decoration after a themed transition");
    }

    #[test]
    fn seasonal_loading_respects_reduced_motion() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        crate::motion::set_reduced(&ctx, true);
        let screen = LoadingScreen::new(LoadingKind::Benchmark).with_seasonal(
            &ta_config::Launcher { seasonal_theme: ta_config::SeasonalTheme::NewYear, ..Default::default() }, false);
        let size = vec2(1280.0, 720.0);
        frame_with_screen(&ctx, size, 0.0, &screen).textures_delta.clear();
        let first = shapes(&ctx, size, 0.0, &screen);
        let later = shapes(&ctx, size, HINT_SECONDS + 1.0, &screen);
        assert_eq!(first, later, "reduced motion holds balloons, particles, emblem and hints still");
    }

    #[test]
    fn loading_animation_reuses_its_textures() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        let size = vec2(1280.0, 720.0);
        for theme in [None, Some(ta_config::SeasonalTheme::Christmas), Some(ta_config::SeasonalTheme::NewYear), Some(ta_config::SeasonalTheme::Halloween)] {
            let mut screen = LoadingScreen::new(LoadingKind::Benchmark);
            if let Some(theme) = theme {
                screen.set_seasonal(&ta_config::Launcher { seasonal_theme: theme, ..Default::default() }, false);
            }
            frame_with_screen(&ctx, size, 0.0, &screen).textures_delta.clear();
            frame_with_screen(&ctx, size, 0.1, &screen).textures_delta.clear();
            for i in 1..60 {
                let mut out = frame_with_screen(&ctx, size, i as f64 / 30.0, &screen);
                let updates = out.textures_delta.set.len();
                out.textures_delta.clear();
                assert_eq!(updates, 0, "loading animation must not allocate new textures each frame ({theme:?})");
            }
        }
    }

    /// In season the season's notes come first and take turns with the usual ones; a plain screen
    /// (or one with the effects off) keeps to the usual ones, on the same context.
    #[test]
    fn seasonal_notes_take_turns_with_the_usual_ones() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        let size = vec2(1280.0, 720.0);
        let settings = ta_config::Launcher { seasonal_theme: ta_config::SeasonalTheme::Halloween, ..Default::default() };
        let screen = LoadingScreen::new(LoadingKind::Game).with_hint_offset(2).with_seasonal(&settings, false);
        frame_with_screen(&ctx, size, 0.0, &screen).textures_delta.clear();
        let (label, _, notes) = crate::seasonal::notes(&ctx).expect("Halloween's notes");
        let mut seen = Vec::new();
        for step in 0..6 {
            let (heading, _, note) = screen.note(&ctx, step as f64 * HINT_SECONDS + 1.0);
            if step % 2 == 0 {
                assert_eq!((heading, note), (label, notes[(step / 2 + 2) % notes.len()]));
            } else {
                assert_eq!((heading, note), ("FIELD NOTES", HINTS[(step / 2 + 2) % HINTS.len()]));
            }
            assert!(!seen.contains(&note), "no note twice in a row of six");
            seen.push(note);
        }
        let plain = LoadingScreen::new(LoadingKind::Game).with_hint_offset(2);
        assert_eq!(plain.note(&ctx, 0.0), ("FIELD NOTES", AMBER, HINTS[2]));
        let off = LoadingScreen::new(LoadingKind::Game).with_hint_offset(2)
            .with_seasonal(&ta_config::Launcher { seasonal_effects: false, ..settings }, false);
        frame_with_screen(&ctx, size, 0.0, &off).textures_delta.clear();
        assert_eq!(off.note(&ctx, 0.0), ("FIELD NOTES", AMBER, HINTS[2]));
    }

    #[test]
    fn loading_status_is_accessible_without_announcing_each_animation_frame() {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        ctx.enable_accesskit();
        let status_id = Id::new("ta_ui::loading_status").accesskit_id();
        let screen = LoadingScreen::new(LoadingKind::Benchmark).with_seasonal(
            &ta_config::Launcher { seasonal_theme: ta_config::SeasonalTheme::NewYear, ..Default::default() }, false);
        let read = |time| {
            let mut output = frame_with_screen(&ctx, vec2(1280.0, 720.0), time, &screen);
            output.textures_delta.clear();
            let tree = output.platform_output.accesskit_update.expect("active native accessibility tree");
            tree.nodes.into_iter().find(|(id, _)| *id == status_id).expect("loading status node").1
        };
        let first = read(0.0);
        let later = read(HINT_SECONDS + 1.0);
        assert_eq!(first.role(), egui::accesskit::Role::Status);
        assert_eq!(first.live(), Some(egui::accesskit::Live::Polite));
        assert!(first.is_live_atomic());
        assert_eq!(first.label(), Some("Preparing the next scene: Combined arms on Sapphire Reach"));
        assert_eq!(later.label(), first.label(), "animation and a new hint must not change the announcement");
        let mut output = ctx.run_ui(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0))),
            ..Default::default()
        }, |ui| LoadingScreen::new(LoadingKind::Game).show(ui.ctx(), "Preparing battlefield", "Coast To Coast", 20.0));
        output.textures_delta.clear();
        let tree = output.platform_output.accesskit_update.unwrap();
        let changed = tree.nodes.iter().find(|(id, _)| *id == status_id).unwrap();
        assert_eq!(changed.1.label(), Some("Preparing battlefield: Coast To Coast"), "a real stage change updates the same node");
    }
}
