//! The pages of the cockpit's instruments that are the same in every crew station: the drive
//! page (speed as figures and a throttle bar, the hull's attitude) and the unit's page (hull and
//! armour, rank, kills and experience). Each is laid out in a box of the display's own size in
//! the pen's unit (a millimetre of glass), so the same page serves a screen on a console and a
//! panel painted over the view when there is no console.

use egui::{Align2, Color32, Pos2, Rect, TextureId, pos2, vec2};
use ta_ui::i18n::{t, tf};

use super::data::{RANK_AT, Readouts};
use super::kit::Pen;
use crate::palette::health_color;
use super::{PHOSPHOR, PHOSPHOR_DIM};
use crate::theme::{AMBER, AMBER_BRIGHT, RED};

/// The radius, in the pen's unit, the drive page is laid out for.
pub const DRIVE_RADIUS: f32 = 58.0;
/// The size, in the pen's unit, the unit page is laid out for.
pub const UNIT_SIZE: (f32, f32) = (146.0, 99.0);

/// The drive page, round: the speed in figures with its unit, the throttle as a bar (its stub
/// astern in red) and the hull's heading, pitch and roll as figures. `c` is its middle; the pen's unit is a
/// millimetre of a screen `DRIVE_RADIUS` across from the middle.
pub fn drive(pen: &Pen, c: Pos2, rd: &Readouts) {
    let at = |y: f32| c + vec2(0.0, pen.s(y));
    let dim = PHOSPHOR.gamma_multiply(0.62);
    let (state, colour) = if !rd.mobile || rd.speed.abs() < 0.5 {
        (t("HALT"), dim)
    } else if rd.speed < 0.0 {
        (t("REV"), RED)
    } else {
        (t("FWD"), PHOSPHOR)
    };
    pen.text(at(-35.0), Align2::CENTER_CENTER, state, 6.5, colour);
    // The speed, three places, the unlit ones' ghosts dim.
    pen.digits(at(-12.0), rd.speed.abs().round().min(999.0) as i32, 3, 30.0, AMBER_BRIGHT, AMBER.gamma_multiply(0.2));
    pen.text(at(9.0), Align2::CENTER_CENTER, t("KM/H"), 6.4, dim);
    // The throttle: twenty segments forward of the zero, four astern.
    let bar = Rect::from_center_size(at(17.5), vec2(pen.s(84.0), pen.s(5.0)));
    let (segments, astern) = (24usize, 4usize);
    let gap = (bar.width() / segments as f32 * 0.18).max(1.0);
    let w = (bar.width() - gap * (segments - 1) as f32) / segments as f32;
    let ahead = (rd.throttle.clamp(0.0, 1.0) * 20.0 - 0.02).ceil().max(0.0) as usize;
    let back = ((-rd.throttle).clamp(0.0, 0.2) / 0.2 * 4.0 - 0.02).ceil().max(0.0) as usize;
    for k in 0..segments {
        let lit = if k >= astern { k - astern < ahead } else { astern - 1 - k < back };
        let colour = match (lit, k < astern) {
            (true, true) => RED,
            (true, false) => AMBER_BRIGHT,
            (false, true) => RED.gamma_multiply(0.22),
            (false, false) => PHOSPHOR_DIM.gamma_multiply(0.5),
        };
        pen.painter.rect_filled(Rect::from_min_size(pos2(bar.left() + k as f32 * (w + gap), bar.top()), vec2(w, bar.height())), 0.0, pen.c(colour));
    }
    // The zero, marked under the bar.
    let zero = bar.left() + astern as f32 * (w + gap) - gap * 0.5;
    pen.line(pos2(zero, bar.bottom() + pen.s(0.8)), pos2(zero, bar.bottom() + pen.s(2.6)), 0.7, dim);
    // The hull's orientation: its heading, its pitch (nose up positive) and its roll (right side
    // down).
    let level = |v: f32| if v.abs() < 0.5 { 0.0 } else { v };
    let figures = [
        (-28.0, t("HDG"), format!("{:03.0}°", rd.heading.round().rem_euclid(360.0))),
        (0.0, t("PITCH"), format!("{:+03.0}°", level(rd.pitch))),
        (28.0, t("ROLL"), format!("{:+03.0}°", level(rd.roll))),
    ];
    for (x, label, figure) in figures {
        pen.text(at(27.0) + vec2(pen.s(x), 0.0), Align2::CENTER_CENTER, label, 5.0, dim);
        pen.text(at(34.5) + vec2(pen.s(x), 0.0), Align2::CENTER_CENTER, figure, 8.2, PHOSPHOR);
    }
}

/// What the unit's page shows besides its figures.
pub struct Face {
    /// The unit's picture and the colour it is tinted.
    pub picture: Option<(TextureId, Color32)>,
    /// A walker's armour by section (how whole each is, in the order of `perspective::Armour`):
    /// drawn on a diagram of the walker in the picture's place.
    pub armour: Option<[f32; 8]>,
    /// The hull is failing (its figures blink red); a kill was just scored.
    pub alarm: bool,
    pub kill_flash: bool,
}

/// Where the armour's sections are on the diagram of a walker (its middle top at the origin, 120
/// across and 146 tall), each as a rectangle from its top left to its bottom right; the head is a
/// disc. In the order of `perspective::Armour`.
const SECTIONS: [(f32, f32, f32, f32); 8] = [
    (-9.0, 0.0, 9.0, 18.0),
    (-17.0, 22.0, 17.0, 74.0),
    (-38.0, 26.0, -21.0, 70.0),
    (21.0, 26.0, 38.0, 70.0),
    (-60.0, 24.0, -42.0, 96.0),
    (42.0, 24.0, 60.0, 96.0),
    (-30.0, 80.0, -5.0, 146.0),
    (5.0, 80.0, 30.0, 146.0),
];

/// The armour by section on a diagram of the walker, filling `r`: each section as whole as its
/// share of the armour is, in the hull's colours.
pub fn armour(pen: &Pen, r: Rect, armour: &[f32; 8]) {
    let k = (r.height() / 150.0).min(r.width() / 124.0);
    let origin = pos2(r.center().x, r.center().y - 73.0 * k);
    // (Outlines a hairline of the page's.)
    let edge = 0.4;
    for (i, &(x0, y0, x1, y1)) in SECTIONS.iter().enumerate() {
        let whole = armour[i];
        let col = health_color(whole).gamma_multiply(0.4 + 0.6 * whole.max(0.15));
        let s = Rect::from_min_max(origin + vec2(x0 * k, y0 * k), origin + vec2(x1 * k, y1 * k));
        if i == 0 {
            pen.painter.circle_filled(s.center(), 9.0 * k, pen.c(col));
            pen.painter.circle_stroke(s.center(), 9.0 * k, egui::Stroke::new(pen.s(edge), pen.c(PHOSPHOR.gamma_multiply(0.8))));
        } else {
            pen.painter.rect_filled(s, 2.0 * k, pen.c(col));
            pen.frame(s, edge, PHOSPHOR.gamma_multiply(0.8));
        }
    }
}

/// The rank's name.
pub fn rank_name(rank: u8) -> String {
    match rank {
        0 => t("RECRUIT"),
        1 => t("VETERAN"),
        2 => t("HARDENED"),
        _ => t("ELITE"),
    }
    .to_string()
}

/// `size` shrunk (to at least `least` of it) for a semi-bold `text` to fit `room` points.
fn shrunk(pen: &Pen, text: &str, size: f32, room: f32, least: f32) -> f32 {
    let wide = pen.painter.layout_no_wrap(text.to_string(), crate::theme::strong(pen.s(size)), Color32::WHITE).size().x;
    size * (room / wide.max(1.0)).clamp(least, 1.0)
}

/// The unit's page: its picture and name, the hull in segments with its figures, its rank and kills
/// and the experience on the way to the next rank. `g` is its box, `UNIT_SIZE` in the pen's unit.
pub fn unit(pen: &Pen, g: Rect, rd: &Readouts, face: &Face) {
    let at = |x: f32, y: f32| g.min + vec2(pen.s(x), pen.s(y));
    let health = if face.alarm { RED } else { health_color(rd.health) };
    // The picture, framed, at the top left (a walker's armour by section in its place).
    let side = 58.0;
    let pic = Rect::from_min_size(at(0.0, 0.0), vec2(pen.s(side), pen.s(side)));
    match (&face.armour, face.picture) {
        (Some(sections), _) => {
            pen.painter.rect_filled(pic, 0.0, pen.c(Color32::from_rgb(10, 22, 11)));
            armour(pen, pic.shrink(pen.s(3.0)), sections);
        }
        (None, Some((id, tint))) => {
            pen.painter.rect_filled(pic, 0.0, pen.c(Color32::from_rgb(12, 28, 14)));
            pen.painter.image(id, pic, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), pen.c(tint));
        }
        (None, None) => {}
    }
    pen.frame(pic, 0.45, PHOSPHOR.gamma_multiply(0.6));
    // Its name and kind.
    let (x, room) = (side + 5.0, UNIT_SIZE.0 - side - 5.0);
    let name = rd.name.to_uppercase();
    let size = shrunk(pen, &name, 10.5, pen.s(room), 0.62);
    pen.name(at(x, 0.0), Align2::LEFT_TOP, name, size, PHOSPHOR);
    if !rd.kind.trim().eq_ignore_ascii_case(rd.name.trim()) {
        let kind = pen.fitted(&rd.kind.to_uppercase(), 6.2, pen.s(room));
        pen.text(at(x, 13.5), Align2::LEFT_TOP, kind, 6.2, PHOSPHOR.gamma_multiply(0.6));
    }
    // The hull: its name and share, the segments, and the hit points.
    pen.text(at(x, 25.0), Align2::LEFT_TOP, t("HULL"), 6.4, PHOSPHOR.gamma_multiply(0.7));
    pen.text(at(UNIT_SIZE.0, 23.0), Align2::RIGHT_TOP, format!("{:>3}%", (rd.health * 100.0).round() as i32), 10.0, health);
    pen.bar(Rect::from_min_size(at(x, 35.0), vec2(pen.s(room), pen.s(7.0))), 10, rd.health, health, PHOSPHOR_DIM.gamma_multiply(0.45));
    pen.text(at(x, 44.5), Align2::LEFT_TOP, format!("{} / {}", rd.hp, rd.hp_max), 6.4, PHOSPHOR.gamma_multiply(0.75));
    // The rank (chevrons for those won, ghosts for those to come) and the kills.
    let y = 70.0;
    for k in 0..3u8 {
        let cx = 3.0 + k as f32 * 8.0;
        let col = if k < rd.rank { AMBER } else { AMBER.gamma_multiply(0.2) };
        pen.path(vec![at(cx, y + 2.0), at(cx + 3.4, y - 1.4), at(cx + 6.8, y + 2.0)], 1.1, col);
    }
    pen.text(at(29.0, y), Align2::LEFT_CENTER, rank_name(rd.rank), 7.6, if rd.rank > 0 { AMBER } else { PHOSPHOR.gamma_multiply(0.6) });
    let kills = tf("KILLS {n}", &[("n", &rd.kills)]);
    pen.text(at(UNIT_SIZE.0, y), Align2::RIGHT_CENTER, kills, 7.6, if face.kill_flash { AMBER_BRIGHT } else { PHOSPHOR.gamma_multiply(0.8) });
    // The experience: the veterancy on a bar, a tick where each rank begins.
    let bar = Rect::from_min_size(at(13.0, 80.0), vec2(pen.s(UNIT_SIZE.0 - 13.0), pen.s(4.5)));
    pen.text(at(0.0, 82.2), Align2::LEFT_CENTER, t("XP"), 6.0, PHOSPHOR.gamma_multiply(0.7));
    pen.bar(bar, 24, rd.veterancy, AMBER, PHOSPHOR_DIM.gamma_multiply(0.45));
    for at_rank in RANK_AT {
        let x = bar.left() + bar.width() * at_rank;
        pen.line(pos2(x, bar.bottom() + pen.s(0.8)), pos2(x, bar.bottom() + pen.s(2.8)), 0.7, PHOSPHOR.gamma_multiply(0.7));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many shapes `draw` paints on a display of its own.
    fn shapes(draw: impl FnOnce(&Pen)) -> usize {
        let ctx = egui::Context::default();
        crate::theme::install_compact(&ctx);
        ctx.begin_pass(egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0))), ..Default::default() });
        let painter = ctx.layer_painter(egui::LayerId::background());
        draw(&Pen::new(&painter, 1.0, false, 1.0));
        let out = ctx.end_pass();
        let n = out.shapes.len();
        out.drop_without_applying_deltas();
        n
    }

    #[test]
    fn the_ranks_have_their_names() {
        assert_eq!([0, 1, 2, 3, 9].map(rank_name), ["RECRUIT", "VETERAN", "HARDENED", "ELITE", "ELITE"]);
    }

    #[test]
    fn the_drive_page_shows_a_unit_standing_astern_and_ahead() {
        let mut rd = Readouts::sample();
        let ahead = shapes(|pen| drive(pen, pos2(200.0, 200.0), &rd));
        rd.speed = -12.0;
        rd.throttle = -0.15;
        let astern = shapes(|pen| drive(pen, pos2(200.0, 200.0), &rd));
        rd.speed = 0.0;
        rd.throttle = 0.0;
        rd.mobile = false;
        let halted = shapes(|pen| drive(pen, pos2(200.0, 200.0), &rd));
        assert!(ahead > 30 && astern > 30 && halted > 30, "{ahead} {astern} {halted}");
    }

    #[test]
    fn a_walkers_page_shows_its_armour_by_section() {
        let rd = Readouts::sample();
        let plain = shapes(|pen| unit(pen, Rect::from_min_size(pos2(10.0, 10.0), vec2(146.0, 99.0)), &rd, &Face { picture: None, armour: None, alarm: false, kill_flash: false }));
        let walker = shapes(|pen| unit(pen, Rect::from_min_size(pos2(10.0, 10.0), vec2(146.0, 99.0)), &rd, &Face { picture: None, armour: Some(rd.armour), alarm: false, kill_flash: false }));
        // (Eight sections, each filled and outlined, on a dark ground.)
        assert!(walker >= plain + 17, "{walker} against {plain}");
    }

    #[test]
    fn the_unit_page_marks_the_ranks_won() {
        let face = Face { picture: None, armour: None, alarm: false, kill_flash: false };
        let mut rd = Readouts::sample();
        let veteran = shapes(|pen| unit(pen, Rect::from_min_size(pos2(10.0, 10.0), vec2(146.0, 99.0)), &rd, &face));
        rd.rank = 0;
        rd.veterancy = 0.0;
        let recruit = shapes(|pen| unit(pen, Rect::from_min_size(pos2(10.0, 10.0), vec2(146.0, 99.0)), &rd, &face));
        // (The experience bar lights fewer of its segments and the chevrons are ghosts: the same
        // shapes, other colours.)
        assert_eq!(veteran, recruit);
        assert!(veteran > 30, "{veteran}");
    }
}
