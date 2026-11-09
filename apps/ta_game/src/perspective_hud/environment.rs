//! The environment: the hour of the day, the wind's strength and the way it blows, the weather and
//! the height of the ground. Every cockpit shows it once: as a status line along the foot of the
//! console's weapons page (or the panoramic display's), or, where no console is in view (the
//! periscope, behind the unit, the magnified sight), as a strip of small plates in the picture's
//! top left corner. Never on the glass.

use egui::{Align2, Color32, Rect, Vec2, pos2, vec2};
use ta_ui::i18n::t;

use super::data::{Daylight, Readouts, clock_time};
use super::kit::{Glyph, Pen, clock};
use crate::effects::Weather;
use crate::theme::{AMBER, AMBER_BRIGHT};

const INK: Color32 = Color32::from_rgb(150, 236, 118);
const MOON: Color32 = Color32::from_rgb(196, 208, 244);
const SKY: Color32 = Color32::from_rgb(142, 190, 236);

/// The weather as a symbol and a word.
pub fn weather_word(env: &super::data::Environment) -> (Glyph, String, Color32) {
    match env.weather {
        Weather::None => (Glyph::Sun, t("FAIR").to_string(), AMBER_BRIGHT),
        Weather::Rain if env.rain > 0.55 => (Glyph::Storm, t("STORM").to_string(), SKY),
        Weather::Rain if env.rain > 0.05 => (Glyph::Rain, t("RAIN").to_string(), SKY),
        Weather::Rain => (Glyph::Cloud, t("FAIR").to_string(), SKY),
        Weather::Snow => (Glyph::Snow, t("SNOW").to_string(), Color32::from_rgb(222, 236, 250)),
        Weather::Embers => (Glyph::Embers, t("EMBERS").to_string(), Color32::from_rgb(255, 150, 70)),
        Weather::Dust => (Glyph::Dust, t("DUST").to_string(), Color32::from_rgb(226, 186, 120)),
        Weather::Spores => (Glyph::Spores, t("SPORES").to_string(), Color32::from_rgb(170, 230, 120)),
        Weather::Glitter => (Glyph::Glitter, t("GLITTER").to_string(), Color32::from_rgb(200, 220, 255)),
    }
}

/// The clock's symbol and face: the sun, the moon, or TA's own fixed daylight.
pub fn clock_face(hours: Option<f32>) -> (Glyph, String, Color32) {
    match hours {
        None => (Glyph::Sun, t("DAY").to_string(), AMBER_BRIGHT),
        Some(h) => {
            let (glyph, col) = match Daylight::at(h) {
                Daylight::Day => (Glyph::Sun, AMBER_BRIGHT),
                Daylight::Twilight => (Glyph::Horizon, AMBER),
                Daylight::Night => (Glyph::Moon, MOON),
            };
            (glyph, clock_time(h), col)
        }
    }
}

/// One plate of the strip. (The wind's says how hard it blows and the bearing it comes from.)
struct Chip {
    glyph: Glyph,
    colour: Color32,
    text: String,
    /// The wind's arrow instead of a symbol: the way it blows, from the view's way up.
    wind: Option<f32>,
}

/// The strip's plates, from the left.
fn chips(rd: &Readouts) -> Vec<Chip> {
    let env = &rd.env;
    let (g, face, col) = clock_face(env.hours);
    let (wg, word, wcol) = weather_word(env);
    vec![
        Chip { glyph: g, colour: col, text: face, wind: None },
        Chip { glyph: Glyph::Sun, colour: INK, text: format!("{} {:.0} {:03.0}°", t("WIND"), env.wind, (env.wind_to + 180.0).round().rem_euclid(360.0)), wind: Some((env.wind_to - rd.look).to_radians()) },
        Chip { glyph: wg, colour: wcol, text: word, wind: None },
        Chip { glyph: Glyph::Peak, colour: AMBER, text: format!("{} {:.0}", t("ELEV"), env.elevation), wind: None },
    ]
}

/// One chip's symbol (or the wind's arrow) about `c`, `r` units across.
fn symbol(pen: &Pen, c: egui::Pos2, chip: &Chip, r: f32) {
    match chip.wind {
        Some(angle) => pen.arrow(c - clock(angle) * pen.s(r * 0.9), angle, r * 1.8, r * 0.9, r * 0.2, chip.colour),
        None => pen.glyph(chip.glyph, c, r, chip.colour),
    }
}

/// One plate: its symbol and its figures.
fn draw_chip(pen: &Pen, r: Rect, chip: &Chip, size: f32) {
    pen.plate(r, INK, 0.45);
    symbol(pen, pos2(r.left() + pen.s(size * 1.15), r.center().y), chip, size * 0.58);
    pen.text(pos2(r.left() + pen.s(size * 2.1), r.center().y), Align2::LEFT_CENTER, chip.text.clone(), size, INK);
}

/// The plates' size, lettered `size` units tall: each one's width, the height of a plate and the
/// gap between them.
fn measure(pen: &Pen, chips: &[Chip], size: f32) -> (Vec<f32>, f32, f32) {
    let (height, gap) = (pen.s(size * 1.75), pen.s(size * 0.4));
    (chips.iter().map(|c| pen.s(size * 2.45) + pen.width(&c.text, size)).collect(), height, gap)
}

/// The plates in a row, lettered `size` units tall, the first's left edge at `x`, `top` down from
/// the screen's top. Returns the room they take.
fn row(pen: &Pen, x: f32, top: f32, rd: &Readouts, size: f32) -> Rect {
    let chips = chips(rd);
    let (widths, height, gap) = measure(pen, &chips, size);
    let total: f32 = widths.iter().sum::<f32>() + gap * (chips.len() - 1) as f32;
    let mut left = x;
    for (chip, w) in chips.iter().zip(&widths) {
        draw_chip(pen, Rect::from_min_size(pos2(left, top), vec2(*w, height)), chip, size);
        left += w + gap;
    }
    Rect::from_min_size(pos2(x, top), vec2(total, height))
}

/// The strip in a corner of the picture, where no console is in view: small plates in a row from
/// `at` (its top left), in the pen's unit (a pixel of a 1080-line picture). Returns the room it
/// takes.
pub fn corner(pen: &Pen, at: egui::Pos2, rd: &Readouts) -> Rect {
    row(pen, at.x, at.y, rd, 14.0)
}

/// The status line along the foot of a console's page: the four, each its symbol and its figures,
/// spread across `r` (the pen's unit is a millimetre of glass), lettered to fit. Returns the room
/// they took.
pub fn status_line(pen: &Pen, r: Rect, rd: &Readouts) -> Rect {
    let chips = chips(rd);
    // As tall as the line allows, narrowed until the four fit side by side.
    let mut size = (r.height() / pen.u * 0.62).min(8.0);
    let room = |size: f32| chips.iter().map(|c| pen.s(size * 1.6) + pen.width(&c.text, size)).sum::<f32>();
    while size > 3.0 && room(size) > r.width() * 0.94 {
        size -= 0.25;
    }
    let gap = (r.width() - room(size)).max(0.0) / chips.len() as f32;
    let mut x = r.left() + gap * 0.5;
    let mut took = Rect::NOTHING;
    for chip in &chips {
        let c = pos2(x + pen.s(size * 0.55), r.center().y);
        symbol(pen, c, chip, size * 0.55);
        let text = pen.text(pos2(x + pen.s(size * 1.6), r.center().y), Align2::LEFT_CENTER, chip.text.clone(), size, INK.gamma_multiply(0.85));
        took = took.union(text).union(Rect::from_center_size(c, Vec2::splat(pen.s(size * 1.1))));
        x = text.right() + gap;
    }
    took
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perspective_hud::data::Environment;

    fn weather(weather: Weather, rain: f32) -> Environment {
        Environment { weather, rain, ..Readouts::sample().env }
    }

    #[test]
    fn the_weather_is_a_symbol_and_a_word() {
        assert_eq!(weather_word(&weather(Weather::None, 0.0)).1, "FAIR");
        // A showering world is fair between showers, rains, and storms when it pours.
        assert_eq!(weather_word(&weather(Weather::Rain, 0.0)).1, "FAIR");
        assert_eq!(weather_word(&weather(Weather::Rain, 0.3)), (Glyph::Rain, "RAIN".to_string(), SKY));
        assert_eq!(weather_word(&weather(Weather::Rain, 0.9)).0, Glyph::Storm);
        assert_eq!(weather_word(&weather(Weather::Snow, 0.0)).1, "SNOW");
        assert_eq!(weather_word(&weather(Weather::Glitter, 0.0)).1, "GLITTER");
    }

    #[test]
    fn the_clock_has_the_sun_by_day_and_the_moon_by_night() {
        assert_eq!(clock_face(None).1, "DAY");
        assert_eq!((clock_face(Some(12.0)).0, clock_face(Some(12.0)).1), (Glyph::Sun, "12:00".to_string()));
        assert_eq!(clock_face(Some(6.4)).0, Glyph::Horizon);
        assert_eq!((clock_face(Some(0.5)).0, clock_face(Some(0.5)).1), (Glyph::Moon, "00:30".to_string()));
    }

    /// Runs `draw` on a picture `k` times a 1080-line one's size.
    fn on_picture<T>(k: f32, draw: impl FnOnce(&Pen) -> T) -> T {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0 * k, 1080.0 * k))), ..Default::default() });
        let painter = ctx.layer_painter(egui::LayerId::background());
        let out = draw(&Pen::new(&painter, k, true, 1.0));
        ctx.end_pass().drop_without_applying_deltas();
        out
    }

    #[test]
    fn the_corner_strip_stands_where_it_is_put_and_scales_with_its_unit() {
        let run = |k: f32| on_picture(k, |pen| corner(pen, pos2(16.0 * k, 14.0 * k), &Readouts::sample()));
        let (one, two) = (run(1.0), run(2.0));
        assert_eq!(one.left_top(), pos2(16.0, 14.0));
        // (Small: a corner's, not across the view.)
        assert!(one.width() > 200.0 && one.width() < 560.0 && one.height() > 16.0 && one.height() < 32.0, "{one:?}");
        // The same strip on a picture twice as tall takes twice the room.
        assert!((two.width() / one.width() - 2.0).abs() < 0.05, "{} against {}", two.width(), one.width());
    }

    #[test]
    fn the_status_line_fits_the_width_it_is_given() {
        // A console page's foot, 140 mm by 12 at four pixels a millimetre, and a narrow one.
        for width in [140.0f32, 70.0] {
            let r = Rect::from_min_size(pos2(20.0, 20.0), vec2(width * 4.0, 48.0));
            let took = on_picture(1.0, |pen| status_line(&Pen::new(pen.painter, 4.0, false, 1.0), r, &Readouts::sample()));
            assert!(took.width() > r.width() * 0.6 && r.expand(2.0).contains_rect(took), "{width}: {took:?} in {r:?}");
        }
    }
}
