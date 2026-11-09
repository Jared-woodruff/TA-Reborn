//! The enemy Commanders' transmissions. A computer player's line comes in on a card at the top
//! left of the battlefield: the speaker's Commander on a little screen (scan lines, the static
//! of a link being made, a glitch now and then), their name and title, the words typed out as
//! they speak, their voice's trace under them. While they "type" the card shows the link being
//! established; one card at a time, the next waiting its turn. The lines stay on the chat (its
//! history); the card is how they arrive.

use std::collections::VecDeque;

use egui::{Color32, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2 as EVec2, pos2, vec2};
use ta_ui::i18n::t;
use ta_vfs::Vfs;

use super::{ACCENT, Hud};
use crate::chat::ChatLine;
use crate::game::Game;
use crate::theme::*;

/// Characters a second the words come up at.
const TYPING: f64 = 42.0;
/// Seconds a card stays once its words are up: at the least, and a little more for each
/// character (to read them); less when another waits.
const HOLD: f64 = 2.8;
const PER_CHAR: f64 = 0.045;
const HOLD_BUSY: f64 = 1.4;
/// Seconds of static as a link comes up, to slide in and out, and the longest a link waits
/// for its words.
const LINK: f64 = 0.45;
const SLIDE: f64 = 0.28;
const WAIT: f64 = 6.0;
/// The card's widest, its padding, and its screen.
const WIDTH: f32 = 400.0;
const PAD: f32 = 10.0;
const SCREEN: f32 = 88.0;

/// The transmissions: those waiting, and the card shown.
#[derive(Default)]
pub(super) struct Comms {
    queue: VecDeque<ChatLine>,
    shown: Option<Shown>,
}

/// The card shown: whose (player, name, title), its words (none while the link is made), when
/// it came up and when its words begin (the interface's clock).
struct Shown {
    player: u8,
    name: String,
    title: &'static str,
    words: Option<String>,
    began: f64,
    speaks: f64,
}

impl Shown {
    /// When its words are all up, and when it goes (its hold, `busy` when another waits).
    fn typed(&self) -> f64 {
        self.speaks + self.words.as_ref().map_or(0.0, |w| w.chars().count() as f64 / TYPING)
    }

    fn leaves(&self, busy: bool) -> f64 {
        let chars = self.words.as_ref().map_or(0, |w| w.chars().count()) as f64;
        self.typed() + if busy { HOLD_BUSY } else { HOLD + chars * PER_CHAR }
    }
}

impl Hud {
    /// The card of the transmission of the moment (see the module's notes), and the next one
    /// when its time is up.
    pub(super) fn comms(&mut self, ctx: &egui::Context, vfs: &Vfs, game: &mut Game) {
        let now = ctx.input(|i| i.time);
        self.comms.queue.extend(std::mem::take(&mut game.transmissions));
        let typing = game.chatter.as_ref().map(|c| c.typing(game.game_secs())).unwrap_or_default();
        let busy = !self.comms.queue.is_empty();
        let still = ta_ui::motion::still(ctx);
        // The card shown goes when its time is up (a link that never brought words, after a
        // while).
        if let Some(s) = &self.comms.shown {
            let over = match &s.words {
                Some(_) => now >= s.leaves(busy) + SLIDE,
                None => now - s.began >= WAIT && !typing.iter().any(|t| t.player == s.player),
            };
            if over {
                self.comms.shown = None;
            }
        }
        // Words that have come: on the link made for them, else on a card of their own.
        let linking = self.comms.shown.as_ref().is_some_and(|s| s.words.is_none());
        if (self.comms.shown.is_none() || linking) && let Some(line) = self.comms.queue.pop_front() {
            let player = line.player.unwrap_or(0);
            match self.comms.shown.as_mut().filter(|s| s.player == player) {
                Some(s) => {
                    s.words = Some(line.text);
                    s.speaks = now.max(s.began + LINK);
                }
                None => {
                    self.comms.shown = Some(Shown { player, name: line.from, title: line.title.unwrap_or(""), words: Some(line.text), began: now, speaks: now + LINK });
                }
            }
        }
        // Someone "typing": the link comes up.
        if self.comms.shown.is_none() && let Some(t) = typing.first() {
            self.comms.shown = Some(Shown { player: t.player, name: t.name.clone(), title: t.title, words: None, began: now, speaks: now + LINK });
        }
        let Some(player) = self.comms.shown.as_ref().map(|s| s.player) else { return };
        let pic = self.portrait(ctx, vfs, game, player, SCREEN);
        let Some(shown) = &self.comms.shown else { return };
        ctx.request_repaint();
        // Where: the top left of the battlefield, under a goals or lesson panel up in that
        // corner (as laid out last frame).
        let free = self.free_view(ctx);
        let width = WIDTH.min(free.width() - 28.0).max(260.0);
        let top = ["scenario_goals", "conquest_goals", "tutorial", "ceasefire"]
            .iter()
            .filter_map(|id| ctx.memory(|m| m.area_rect(egui::Id::new(*id))))
            .filter(|r| r.min.x < free.left() + width && r.max.y > free.top())
            .map(|r| r.max.y + 10.0)
            .fold(free.top() + 12.0, f32::max);
        let enter = ((now - shown.began) / SLIDE).clamp(0.0, 1.0) as f32;
        let leave = match &shown.words {
            Some(_) => ((shown.leaves(busy) + SLIDE - now) / SLIDE).clamp(0.0, 1.0) as f32,
            None => 1.0,
        };
        let ease = 1.0 - (1.0 - enter).powi(3);
        let slide = if still { 0.0 } else { (1.0 - ease) * (width + 30.0) + (1.0 - leave) * 24.0 };
        let opacity = if still { enter.min(leave) } else { leave.min(0.35 + 0.65 * ease) };
        let at = pos2(free.left() + 14.0 - slide, top);
        let side = game.sim.players.get(shown.player as usize).map(|p| p.side.to_uppercase()).unwrap_or_default();
        let colour = game.sim.players.get(shown.player as usize).map_or(ACCENT, |p| crate::palette::team_color(p.color));
        egui::Area::new(egui::Id::new("transmission")).order(egui::Order::Middle).fixed_pos(at).interactable(false).show(ctx, |ui| {
            ui.set_width(width);
            ui.multiply_opacity(opacity);
            card(ui, shown, width, now, still, pic, &side, colour);
        });
    }
}

/// The card: plate, screen, header, name and title, the words typed so far, the voice's trace.
#[allow(clippy::too_many_arguments)]
fn card(ui: &mut egui::Ui, shown: &Shown, width: f32, now: f64, still: bool, pic: Option<super::UnitPic>, side: &str, colour: Color32) {
    let readable = colour.lerp_to_gamma(Color32::WHITE, 0.38);
    let text_x = PAD + 4.0 + SCREEN + 12.0;
    let text_w = width - text_x - PAD;
    let painter = ui.painter().clone();
    // The words in full lay out the card (it does not grow as they come); those up so far.
    let words = shown.words.clone().unwrap_or_default();
    let full = painter.layout(if words.is_empty() { " ".to_string() } else { words.clone() }, body(14.0), CREAM, text_w);
    let shown_chars = if still { usize::MAX } else { ((now - shown.speaks).max(0.0) * TYPING) as usize };
    let typing = shown.words.is_some() && !still && now >= shown.speaks && now < shown.typed();
    let linking = shown.words.is_none() || now < shown.speaks;
    let typed: String = words.chars().take(shown_chars).collect();
    let header = painter.layout_no_wrap(t("INCOMING TRANSMISSION").to_string(), strong(10.5), AMBER);
    let name = painter.layout(shown.name.clone(), strong(19.0), readable, text_w);
    let title = if side.is_empty() { t(shown.title).to_string() } else { format!("{} · {side}", t(shown.title)) };
    let title = painter.layout(title, strong(10.5), STEEL, text_w);
    const TRACE: f32 = 12.0;
    let text_h = header.size().y + 3.0 + name.size().y + title.size().y + 6.0 + full.size().y + 6.0 + TRACE;
    let height = (SCREEN + 2.0 * PAD).max(text_h + 2.0 * PAD);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    // The plate: its shadow, metal, the speaker's colour down its edge.
    painter.add(Shape::rect_filled(rect.translate(vec2(0.0, 4.0)).expand(2.0), 6.0, Color32::from_black_alpha(100)));
    painter.add(ta_ui::metal::plate(ui.ctx(), rect, PANEL));
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, colour.gamma_multiply(0.5)), StrokeKind::Inside);
    painter.rect_filled(Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(4.0, rect.height() - 12.0)), 1.5, colour);
    let screen = Rect::from_min_size(rect.min + vec2(PAD + 4.0, PAD), vec2(SCREEN, SCREEN));
    self::screen(&painter, screen, now, still, pic, colour, linking, typing);
    // The words' side.
    let x = rect.min.x + text_x;
    let mut y = rect.min.y + PAD;
    painter.galley(pos2(x, y), header.clone(), AMBER);
    // A lamp at the header's end: red while the link is made, green as they speak.
    let blink = still || (now * 2.0).fract() < 0.6;
    let lamp = pos2(rect.max.x - PAD - 4.0, y + header.size().y / 2.0);
    painter.add(ta_ui::metal::lamp(lamp, 3.0, if linking { RED } else { GREEN }, if blink { 1.0 } else { 0.25 }));
    y += header.size().y + 3.0;
    painter.galley(pos2(x, y), name.clone(), readable);
    y += name.size().y;
    painter.galley(pos2(x, y), title.clone(), STEEL);
    y += title.size().y + 6.0;
    let typed_galley = painter.layout(typed.clone(), body(14.0), CREAM, text_w);
    painter.galley(pos2(x, y), typed_galley.clone(), CREAM);
    // A cursor at the words' end while they come (and while the link is made).
    if (typing || linking) && (now * 3.0).fract() < 0.6 {
        let end = typed_galley.rect.max;
        let last_line = typed_galley.rows.last().map_or(Rect::from_min_size(Pos2::ZERO, vec2(0.0, 16.0)), |r| r.rect());
        let cx = x + if typed.is_empty() { 0.0 } else { last_line.max.x + 2.0 };
        let cy = y + if typed.is_empty() { 0.0 } else { end.y - last_line.height() };
        painter.rect_filled(Rect::from_min_size(pos2(cx, cy + 2.0), vec2(7.0, last_line.height().max(14.0) - 4.0)), 1.0, CREAM.gamma_multiply(0.8));
    }
    // The voice's trace along the foot: alive while they speak.
    let trace = Rect::from_min_size(pos2(x, rect.max.y - PAD - TRACE), vec2(text_w, TRACE));
    voice_trace(&painter, trace, now, typing, colour);
}

/// The little screen: the Commander, lit in the speaker's colour, under scan lines and the
/// refresh's passing band; static while the link is made; a glitch now and then; the signal's
/// bars; brackets at its corners.
#[allow(clippy::too_many_arguments)]
fn screen(painter: &egui::Painter, rect: Rect, now: f64, still: bool, pic: Option<super::UnitPic>, colour: Color32, linking: bool, typing: bool) {
    let p = painter.with_clip_rect(rect);
    p.add(ta_ui::metal::vgrad(rect, colour.lerp_to_gamma(Color32::BLACK, 0.72), Color32::from_rgb(4, 8, 10)));
    let flicker = if still { 1.0 } else { 0.9 + 0.1 * ((now * 23.0).sin() * (now * 7.3).sin()) as f32 };
    let tint = Color32::WHITE.lerp_to_gamma(colour, 0.2).gamma_multiply(flicker);
    if let Some(pic) = pic {
        // A portrait from the shoulders up; TA's own picture whole.
        let uv = if pic.portrait { Rect::from_min_max(pos2(0.12, 0.02), pos2(0.88, 0.78)) } else { Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)) };
        p.image(pic.id, rect.shrink(2.0), uv, tint);
        // A glitch: a slice of the picture thrown aside for a moment, now and then.
        let cycle = (now / 3.7).fract();
        if !still && !linking && cycle < 0.03 {
            let k = (now * 50.0) as u64;
            let band = 0.25 + (hash(k) % 50) as f32 / 100.0;
            let slice = Rect::from_min_max(pos2(rect.min.x, rect.min.y + rect.height() * band), pos2(rect.max.x, rect.min.y + rect.height() * (band + 0.08)));
            let suv = Rect::from_min_max(pos2(uv.min.x, uv.min.y + uv.height() * band), pos2(uv.max.x, uv.min.y + uv.height() * (band + 0.08)));
            let dx = if hash(k + 1).is_multiple_of(2) { 4.0 } else { -4.0 };
            p.image(pic.id, slice.translate(vec2(dx, 0.0)), suv, tint.gamma_multiply(0.9));
        }
    }
    // Scan lines, and the band of the refresh passing down.
    let mut y = rect.min.y + 1.0;
    while y < rect.max.y {
        p.hline(rect.x_range(), y + 0.5, Stroke::new(1.0, Color32::from_black_alpha(55)));
        y += 3.0;
    }
    if !still {
        let band_y = rect.min.y + ((now * 0.55).fract() as f32) * (rect.height() + 20.0) - 10.0;
        p.add(ta_ui::metal::vgrad(Rect::from_min_max(pos2(rect.min.x, band_y), pos2(rect.max.x, band_y + 10.0)), Color32::from_white_alpha(0), Color32::from_white_alpha(16)));
    }
    // Static: the link being made, and as the words begin.
    if !still && linking {
        let k = (now * 30.0) as u64;
        for i in 0..90u64 {
            let h = hash(k * 131 + i);
            let (sx, sy) = ((h % 1000) as f32 / 1000.0, ((h >> 10) % 1000) as f32 / 1000.0);
            let w = 1.0 + ((h >> 20) % 4) as f32;
            let a = 40 + ((h >> 24) % 140) as u8;
            p.rect_filled(Rect::from_min_size(pos2(rect.min.x + sx * rect.width(), rect.min.y + sy * rect.height()), vec2(w, 1.5)), 0.0, Color32::from_white_alpha(a));
        }
    }
    // The edges darkened, as on a tube.
    for (i, a) in [(0.0, 120u8), (1.0, 80), (2.0, 40)] {
        p.rect_stroke(rect.shrink(i), 2.0, Stroke::new(1.0, Color32::from_black_alpha(a)), StrokeKind::Inside);
    }
    // The signal: four bars, climbing while the link is made.
    let lit = if linking && !still { 1 + ((now * 5.0) as usize % 4) } else { 4 };
    for k in 0..4 {
        let h = 3.0 + k as f32 * 2.0;
        let r = Rect::from_min_max(pos2(rect.max.x - 18.0 + k as f32 * 3.5, rect.min.y + 13.0 - h), pos2(rect.max.x - 16.0 + k as f32 * 3.5, rect.min.y + 13.0));
        p.rect_filled(r, 0.0, if k < lit { colour.lerp_to_gamma(Color32::WHITE, 0.3) } else { Color32::from_white_alpha(30) });
    }
    // A pulse at the foot while they speak.
    if typing && !still {
        p.rect_filled(Rect::from_min_size(pos2(rect.min.x + 5.0, rect.max.y - 8.0), vec2(4.0, 4.0)), 1.0, GREEN.gamma_multiply(if (now * 4.0).fract() < 0.5 { 1.0 } else { 0.3 }));
    }
    // Brackets at the corners, and the frame.
    let c = colour.lerp_to_gamma(Color32::WHITE, 0.25);
    let l = 10.0;
    for (corner, dx, dy) in [(rect.left_top(), 1.0, 1.0), (rect.right_top(), -1.0, 1.0), (rect.left_bottom(), 1.0, -1.0), (rect.right_bottom(), -1.0, -1.0)] {
        let o = corner + vec2(dx * 2.0, dy * 2.0);
        painter.line_segment([o, o + vec2(dx * l, 0.0)], Stroke::new(1.5, c));
        painter.line_segment([o, o + vec2(0.0, dy * l)], Stroke::new(1.5, c));
    }
    painter.rect_stroke(rect.expand(1.0), 2.0, Stroke::new(1.0, ta_ui::metal::STEEL_EDGE), StrokeKind::Outside);
}

/// The voice's trace: bars that rise and fall while the words come, a flat line otherwise.
fn voice_trace(painter: &egui::Painter, rect: Rect, now: f64, speaking: bool, colour: Color32) {
    let bars = ((rect.width() + 2.0) / 5.0) as usize;
    let mid = rect.center().y;
    for i in 0..bars {
        let x = rect.min.x + i as f32 * 5.0;
        let h = if speaking {
            let a = (now * 11.0 + i as f64 * 0.7).sin() * (now * 4.3 + i as f64 * 1.9).sin();
            let b = (now * 17.0 + i as f64 * 0.31).sin();
            (1.5 + (a.abs() * 0.75 + b.abs() * 0.25) as f32 * (rect.height() - 2.0)).min(rect.height())
        } else {
            1.5
        };
        painter.rect_filled(Rect::from_center_size(pos2(x + 1.5, mid), EVec2::new(3.0, h)), 1.0, colour.lerp_to_gamma(Color32::WHITE, 0.2).gamma_multiply(if speaking { 0.9 } else { 0.35 }));
    }
}

fn hash(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(words: Option<&str>) -> Shown {
        Shown { player: 1, name: "Cortex Theta".into(), title: "MACHINE INTELLIGENCE", words: words.map(str::to_string), began: 10.0, speaks: 10.0 + LINK }
    }

    /// The words take their time to come and stay long enough to read, less with another
    /// waiting.
    #[test]
    fn a_card_stays_as_long_as_its_words_need() {
        let short = shown(Some("Target acquired."));
        let long = shown(Some("Contact established. Probability of your survival: declining."));
        assert!(long.typed() > short.typed() && long.leaves(false) > short.leaves(false));
        assert!(short.leaves(true) < short.leaves(false));
        assert!(short.leaves(false) - short.began >= LINK + HOLD);
        assert_eq!(shown(None).typed(), shown(None).speaks);
    }
}
