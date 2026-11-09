//! Calendar decoration shared by the launcher and loading screens. No overlay receives
//! input, covers a label, or keeps an unfocused/reduced-motion window animating.
//!
//! Christmas: snow falling in three depths (the nearest flakes soft and large, some of them
//! crystals turning slowly), snow lying in drifts along the pipes with icicles under them, and
//! strings of lights glowing softly. New Year: soft firework bursts that open and fade (none
//! with reduced flashes), confetti tumbling down, gold ribbon wound round the pipes with tassels,
//! and gold stars. Halloween: bats flitting through the gutters over a low drifting mist and
//! rising embers, orange and purple bands round the pipes, and strings of pumpkin lanterns
//! flickering like candles (glowing steadily with reduced flashes). Each puts a cap on the
//! emblem (a Santa hat, the armies' balloons, a witch's hat), and its own funny field notes on
//! the loading screens ([`notes`]), the notes' plate dressed for it ([`plate`]).

use std::time::Duration;

use egui::{Color32, Context, Id, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use ta_config::{Launcher, SeasonalTheme};

use crate::i18n::n;

/// A season the game dresses up for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Celebration {
    Christmas,
    NewYear,
    Halloween,
}

/// The season today, as the launcher's settings have it (its own choice, or the calendar's when
/// automatic; none with the seasonal effects off): the game's menus dress up for it too.
pub fn today(settings: &Launcher) -> Option<Celebration> {
    let (month, day) = calendar();
    on(settings, month, day)
}

/// The season on `month` and `day` (1-based), as the launcher's settings have it (what [`today`]
/// gives on that day).
pub fn on(settings: &Launcher, month: u16, day: u16) -> Option<Celebration> {
    let (month, day) = if settings.seasonal_effects && settings.seasonal_theme == SeasonalTheme::Automatic { (month, day) } else { (0, 0) };
    celebration(settings, month, day)
}

#[derive(Clone, Default)]
struct Frame {
    celebration: Option<Celebration>,
    elapsed: f64,
    previous: Option<f64>,
    firework_elapsed: f64,
    /// Reduced flashes: no bursts, and lights that glow steadily.
    quiet: bool,
}

fn frame_id() -> Id {
    Id::new("ta_ui::seasonal_frame")
}

/// Resolve the calendar once per UI frame. Advancing our own clock avoids a snowfall
/// jump after focus returns; reduced motion freezes the existing arrangement.
pub fn begin(ctx: &Context, settings: &Launcher, reduced_flashes: bool) {
    let celebration = today(settings);
    let now = ctx.input(|i| i.time);
    let moving = !crate::motion::still(ctx);
    ctx.data_mut(|d| {
        let frame = d.get_temp_mut_or_default::<Frame>(frame_id());
        if celebration.is_some() && frame.celebration == celebration && moving {
            let delta = frame.previous.map_or(0.0, |previous| (now - previous).clamp(0.0, 0.1));
            frame.elapsed += delta;
            if !reduced_flashes {
                frame.firework_elapsed += delta;
            }
        }
        frame.previous = Some(now);
        frame.celebration = celebration;
        frame.quiet = reduced_flashes;
    });
}

fn celebration(settings: &Launcher, month: u16, day: u16) -> Option<Celebration> {
    if !settings.seasonal_effects {
        return None;
    }
    match settings.seasonal_theme {
        SeasonalTheme::Christmas => Some(Celebration::Christmas),
        SeasonalTheme::NewYear => Some(Celebration::NewYear),
        SeasonalTheme::Halloween => Some(Celebration::Halloween),
        SeasonalTheme::Automatic => match (month, day) {
            // New Year takes precedence while the two holiday windows overlap.
            (12, 31) | (1, 1) => Some(Celebration::NewYear),
            (12, 1..=30) | (1, 2..=6) => Some(Celebration::Christmas),
            (10, 24..=31) | (11, 1) => Some(Celebration::Halloween),
            _ => None,
        },
    }
}

/// The season's funny field notes for the loading screens, their plate's heading and its colour;
/// none out of season. They take turns with the usual notes, the season's first.
pub fn notes(ctx: &Context) -> Option<(&'static str, Color32, &'static [&'static str])> {
    match frame(ctx).celebration? {
        Celebration::Christmas => Some((n("FESTIVE FIELD NOTES"), Color32::from_rgb(232, 96, 84), CHRISTMAS_NOTES)),
        Celebration::NewYear => Some((n("NEW YEAR FIELD NOTES"), GOLD, NEW_YEAR_NOTES)),
        Celebration::Halloween => Some((n("SPOOKY FIELD NOTES"), PUMPKIN, HALLOWEEN_NOTES)),
    }
}

const CHRISTMAS_NOTES: &[&str] = &[
    n("Santa's sleigh is not a transport. Load your presents into an Atlas instead."),
    n("The D-gun does not gift-wrap. Take it up with your Commander."),
    n("A Fusion Reactor keeps the fairy lights on all winter. Just never build one next to the tree."),
    n("Snow falls on the Arm and the Core alike. So do Big Bertha shells."),
    n("Coal in your stocking? Reclaim it. Metal is metal."),
    n("Peewees sing terrible carols, but they never miss a patrol."),
    n("Stockings full of metal, storage full of energy: a Commander's perfect Christmas morning."),
    n("Silent night, holy night: keep a radar tower running, just in case."),
    n("Snowball fights are fun until somebody brings a Krogoth."),
];

const NEW_YEAR_NOTES: &[&str] = &[
    n("New Year's resolution: build more metal extractors. This year, keep it."),
    n("Fireworks are pretty, but a nuclear silo's are louder. Celebrate responsibly."),
    n("Count down from ten, then count your units. The second count matters more."),
    n("Out with the old Commander? You only get one. Look after it all year."),
    n("Start the year with a bang: the Core recommends the Doomsday Machine, the Arm recommends anything else."),
    n("Fireworks at midnight, artillery at dawn: a Commander's New Year."),
    n("Raise a glass of coolant to the units lost last year, then build more of them."),
    n("Midnight is the perfect time for a sneak attack. So is every other time."),
];

const HALLOWEEN_NOTES: &[&str] = &[
    n("Trick or treat? The Core always picks the D-gun."),
    n("A self-destructing Krogoth is a better jump scare than any ghost."),
    n("Wrecks rise from the dead with a Necro. Spooky, and very economical."),
    n("Blips vanishing off your radar? Not ghosts, just jammers. Probably."),
    n("Carve your pumpkins before the Big Bertha does."),
    n("Never split the party. Especially when the party is twelve Peewees."),
    n("Fog of war too tame? Set the line of sight to True and let the hills hide what lurks behind them. If you dare."),
    n("No Commander has ever come back from the dead. Keep yours alive."),
    n("Bats do not show up on radar. Neither do Vamps."),
    n("The scariest sound in the dark is your energy running out."),
];

fn frame(ctx: &Context) -> Frame {
    ctx.data(|d| d.get_temp(frame_id())).unwrap_or_default()
}

fn calendar() -> (u16, u16) {
    let (_, month, day) = local_date();
    (month, day)
}

/// Today's date where the player is: year, month and day.
#[cfg(windows)]
pub fn local_date() -> (u16, u16, u16) {
    // The OS writes a SYSTEMTIME; the wrapper owns and initializes that output.
    let date = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    (date.wYear, date.wMonth, date.wDay)
}

/// The time of day where the player is: hour and minute (a chat line's time).
#[cfg(windows)]
pub fn local_clock() -> (u16, u16) {
    // The OS writes a SYSTEMTIME; the wrapper owns and initializes that output.
    let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    (now.wHour, now.wMinute)
}

/// The time of day (UTC elsewhere, as [`local_date`]): hour and minute.
#[cfg(not(windows))]
pub fn local_clock() -> (u16, u16) {
    let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    (((seconds / 3600) % 24) as u16, ((seconds / 60) % 60) as u16)
}

/// Today's date where the player is: year, month and day.
#[cfg(not(windows))]
pub fn local_date() -> (u16, u16, u16) {
    // The portable fallback uses the UTC calendar and needs no locale/timezone process.
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86400);
    utc_date(days)
}

#[cfg(any(not(windows), test))]
fn utc_date(days: u64) -> (u16, u16, u16) {
    // Gregorian eras, with March at the start so leap days fall at the end of a year.
    let z = days as i64 + 719468;
    let era = z / 146097;
    let day_of_era = z - era * 146097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month + 2) / 5 + 1;
    let month = month + if month < 10 { 3 } else { -9 };
    ((year_of_era + era * 400 + i64::from(month <= 2)) as u16, month as u16, day as u16)
}

/// Vector decorations follow the emblem at any DPI, leaving its centre visible. At a
/// 48-point emblem the balloons extend at most 10 points sideways and 5 points above;
/// the Christmas hat uses the existing 9-point clearance above the emblem.
pub fn logo(painter: &Painter, rect: Rect) {
    match frame(painter.ctx()).celebration {
        Some(Celebration::NewYear) => { new_year_logo(painter, rect); return; }
        Some(Celebration::Halloween) => { witch_hat(painter, rect); return; }
        Some(Celebration::Christmas) => {}
        None => return,
    }
    let scale = rect.width() / 48.0;
    let origin = pos2(rect.center().x - 1.0 * scale, rect.top() + 12.0 * scale);
    let at = |x: f32, y: f32| origin + vec2(x - y * 0.07, y + x * 0.07) * scale;
    let outline = vec![at(-18.0, 0.0), at(-12.0, -13.0), at(-4.0, -19.0), at(7.0, -20.0), at(16.0, -12.0), at(19.0, 1.0)];
    painter.add(Shape::convex_polygon(outline, Color32::from_rgb(152, 27, 38), Stroke::new(1.0 * scale, Color32::from_rgb(70, 21, 30))));
    painter.add(Shape::convex_polygon(vec![at(-15.0, -1.0), at(-9.0, -12.0), at(-2.0, -17.0), at(5.0, -17.0), at(11.0, -1.0)], Color32::from_rgb(198, 43, 52), Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![at(5.0, -20.0), at(15.0, -17.0), at(25.0, -5.0), at(19.0, -1.0), at(12.0, -9.0)], Color32::from_rgb(172, 32, 43), Stroke::NONE));
    painter.line_segment([at(8.0, -17.0), at(16.0, -8.0)], Stroke::new(1.3 * scale, Color32::from_rgb(219, 68, 68)));
    painter.line_segment([at(-17.0, 3.0), at(18.0, 3.0)], Stroke::new(9.0 * scale, Color32::from_rgb(178, 198, 207)));
    painter.line_segment([at(-17.0, 1.0), at(18.0, 1.0)], Stroke::new(7.0 * scale, Color32::from_rgb(237, 244, 244)));
    for i in 0..11 {
        let x = -18.0 + i as f32 * 3.6;
        painter.circle_filled(at(x, 1.0), (3.6 + (i % 3) as f32 * 0.2) * scale, Color32::from_rgb(239, 247, 247));
    }
    painter.circle_filled(at(24.0, -2.0), 5.5 * scale, Color32::from_rgb(181, 201, 210));
    painter.circle_filled(at(23.5, -3.0), 4.8 * scale, Color32::from_rgb(244, 249, 248));
}

/// The season on a pipe: the pipe runs from `left` to `right`, its top edge at `top`, `width`
/// thick; whatever hangs under it (icicles, lights, tassels) reaches at most `hang` below its
/// lower edge, so that it stays clear of the labels under the pipe.
pub fn trim(painter: &Painter, left: f32, right: f32, top: f32, width: f32, hang: f32) {
    let frame = frame(painter.ctx());
    if right <= left {
        return;
    }
    let time = frame.elapsed as f32;
    match frame.celebration {
        Some(Celebration::Christmas) => {
            for (x0, x1, seed) in patches(left, right, 230.0, 0.30, 0.82) {
                snow(painter, x0, x1, top, width, seed, 1.0);
                if hang >= 4.0 {
                    icicles(painter, x0, x1, top + width, hang, seed);
                }
            }
            lights(painter, left, right, top, width, hang, time, frame.quiet);
        }
        Some(Celebration::NewYear) => {
            for (x0, x1, seed) in patches(left, right, 260.0, 0.22, 0.70) {
                ribbon(painter, x0, x1, top, width, seed);
                if hang >= 6.0 {
                    for (end, side) in [(x0, 0), (x1, 1)] {
                        tassel(painter, pos2(end, top + width), hang, time, seed.wrapping_mul(2).wrapping_add(side));
                    }
                }
            }
            let mut x = left + 120.0;
            while x < right - 40.0 {
                gold_star(painter, pos2(x, top + width * 0.5), (width * 0.5 + hang + 0.5).min(3.6), hash(x as u32 / 7) * 0.6);
                x += 210.0;
            }
        }
        Some(Celebration::Halloween) => {
            for (x0, x1, seed) in patches(left, right, 250.0, 0.20, 0.55) {
                bands(painter, x0, x1, top, width, seed);
            }
            lanterns(painter, left, right, top, width, hang, time, frame.quiet);
        }
        None => {}
    }
}

/// Orange and purple bands wound round a pipe, leaning with the winding.
fn bands(painter: &Painter, x0: f32, x1: f32, top: f32, width: f32, seed: u32) {
    let lean = width * 0.8;
    let band = 4.0;
    let mut x = x0;
    let mut k = seed;
    while x + band + lean < x1 {
        let colour = if k.is_multiple_of(2) { PUMPKIN } else { PURPLE };
        let quad = vec![pos2(x, top - 0.6), pos2(x + band, top - 0.6), pos2(x + band + lean, top + width + 0.6), pos2(x + lean, top + width + 0.6)];
        painter.add(Shape::convex_polygon(quad, colour, Stroke::NONE));
        x += 7.0;
        k = k.wrapping_add(1);
    }
}

/// A string of pumpkin lanterns, each flickering like a candle inside (steady with reduced
/// flashes): hanging under the pipe where there is room, wound along it where there is not.
#[allow(clippy::too_many_arguments)]
fn lanterns(painter: &Painter, left: f32, right: f32, top: f32, width: f32, hang: f32, time: f32, quiet: bool) {
    const PITCH: f32 = 44.0;
    let hanging = hang >= 6.0;
    let radius = if hanging { ((hang - 2.0) * 0.5).clamp(2.2, 3.6) } else { (width * 0.5 + 1.4).min(3.4) };
    let (line, sag) = if hanging { (top + width + 0.6, (hang - 2.0 * radius - 1.0).clamp(0.5, 3.0)) } else { (top + width * 0.5, 0.0) };
    let first = (left / PITCH).ceil() as i32;
    let last = (right / PITCH).floor() as i32;
    if last <= first {
        return;
    }
    let wire: Vec<Pos2> = (first * 4..=last * 4).map(|q| {
        let x = q as f32 * PITCH / 4.0;
        let t = (q.rem_euclid(4)) as f32 / 4.0;
        pos2(x, line + sag * (t * std::f32::consts::PI).sin())
    }).collect();
    painter.add(Shape::line(wire, Stroke::new(0.9, Color32::from_rgb(30, 24, 30))));
    for i in first..last {
        let s = (i as u32).wrapping_mul(31);
        let at = pos2((i as f32 + 0.5) * PITCH, line + sag + if hanging { radius + 0.4 } else { 0.0 });
        // A candle's flicker: two waves out of step.
        let flicker = if quiet { 0.85 } else { 0.72 + 0.28 * (time * 6.7 + hash(s) * 6.3).sin() * (time * 2.9 + hash(s.wrapping_add(1)) * 6.3).sin().abs() };
        // (Its glow only as far as the room under the pipe goes.)
        let halo = (top + width + hang - at.y).clamp(radius * 0.9, radius * 2.0);
        pumpkin(painter, at, radius, flicker, halo);
    }
}

/// A pumpkin `radius` across, lit from inside (`glow` 0 to 1) and glowing round to `halo`: its
/// ribs, its stem, the candle inside.
fn pumpkin(painter: &Painter, at: Pos2, radius: f32, glow: f32, halo: f32) {
    painter.circle_filled(at, halo, PUMPKIN.gamma_multiply(0.14 * glow));
    let body = |dx: f32, rx: f32, colour: Color32| {
        let points: Vec<Pos2> = (0..14).map(|i| {
            let a = i as f32 / 14.0 * std::f32::consts::TAU;
            at + vec2(dx + a.cos() * rx, a.sin() * radius * 0.88)
        }).collect();
        painter.add(Shape::convex_polygon(points, colour, Stroke::NONE));
    };
    let dark = Color32::from_rgb(168, 72, 18);
    body(-radius * 0.38, radius * 0.66, dark);
    body(radius * 0.38, radius * 0.66, dark);
    body(0.0, radius * 0.7, Color32::from_rgb(232, 120, 32).gamma_multiply(0.75 + 0.25 * glow));
    // The light inside, through the face.
    painter.circle_filled(at + vec2(0.0, radius * 0.1), radius * 0.36, Color32::from_rgb(255, 214, 120).gamma_multiply(0.5 + 0.5 * glow));
    painter.line_segment([at - vec2(0.0, radius * 0.8), at - vec2(-radius * 0.25, radius * 1.25)], Stroke::new((radius * 0.32).max(0.7), Color32::from_rgb(70, 92, 40)));
}

/// A witch's hat over the emblem: a wide brim and a crooked crown bending over to the right,
/// lit at their edges so they stand out against the dark, with an orange band and a gold
/// buckle (within the Santa hat's clearance above the emblem).
fn witch_hat(painter: &Painter, rect: Rect) {
    let scale = rect.width() / 48.0;
    let origin = pos2(rect.center().x, rect.top() + 12.0 * scale);
    let at = |x: f32, y: f32| origin + vec2(x + y * 0.05, y - x * 0.05) * scale;
    let edge = Stroke::new(1.1 * scale, Color32::from_rgb(122, 82, 164));
    let brim: Vec<Pos2> = (0..32).map(|i| {
        let a = i as f32 / 32.0 * std::f32::consts::TAU;
        at(a.cos() * 20.0, a.sin() * 3.6)
    }).collect();
    painter.add(Shape::convex_polygon(brim, HAT, edge));
    // The crown: a strip between its two sides, from the brim to the tip.
    let left = [(-10.5, -1.0), (-7.5, -6.5), (-5.0, -11.5), (-2.8, -15.5), (1.0, -18.6), (5.5, -20.0), (12.0, -18.0)];
    let right = [(10.5, -1.0), (6.5, -6.5), (4.5, -11.0), (3.8, -14.0), (5.0, -16.2), (8.0, -17.3), (12.0, -18.0)];
    let mut crown = egui::Mesh::default();
    for (l, r) in left.iter().zip(&right) {
        crown.colored_vertex(at(l.0, l.1), HAT);
        crown.colored_vertex(at(r.0, r.1), HAT);
    }
    for i in 0..left.len() as u32 - 1 {
        crown.add_triangle(i * 2, i * 2 + 1, i * 2 + 2);
        crown.add_triangle(i * 2 + 1, i * 2 + 3, i * 2 + 2);
    }
    painter.add(Shape::mesh(crown));
    let outline: Vec<Pos2> = left.iter().chain(right.iter().rev().skip(1)).map(|&(x, y)| at(x, y)).collect();
    painter.add(Shape::line(outline, edge));
    // Its sheen, and the band with its buckle.
    painter.add(Shape::line(vec![at(-7.6, -3.0), at(-5.6, -8.0), at(-3.4, -12.0)], Stroke::new(1.4 * scale, Color32::from_rgb(98, 64, 136))));
    painter.add(Shape::convex_polygon(vec![at(-10.3, -1.4), at(10.3, -1.4), at(8.4, -4.4), at(-8.8, -4.4)], PUMPKIN, Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![at(-2.3, -1.0), at(2.3, -1.0), at(2.3, -4.8), at(-2.3, -4.8)], Color32::TRANSPARENT, Stroke::new(1.1 * scale, GOLD)));
}

/// Where a pipe carries its season: runs of `share` to `most` of every `pitch` points.
fn patches(left: f32, right: f32, pitch: f32, share: f32, most: f32) -> Vec<(f32, f32, u32)> {
    let mut out = Vec::new();
    let mut i = (left / pitch).floor() as i32;
    loop {
        let start = i as f32 * pitch;
        if start > right {
            break;
        }
        let seed = (i + 1000) as u32;
        let length = pitch * (share + (most - share) * hash(seed.wrapping_mul(3)));
        let x0 = (start + pitch * 0.08 * hash(seed.wrapping_mul(5))).max(left);
        let x1 = (x0 + length).min(right);
        if x1 - x0 > 12.0 {
            out.push((x0, x1, seed));
        }
        i += 1;
    }
    out
}

/// Snow lying on a pipe: thickest in its middle, its top rippled, tapered at both ends; `deep`
/// times as deep as on the launcher's pipes.
fn snow(painter: &Painter, x0: f32, x1: f32, top: f32, width: f32, seed: u32, deep: f32) {
    let steps = ((x1 - x0) / 4.0).ceil().max(2.0) as usize;
    let depth = (2.6 + 1.6 * hash(seed.wrapping_mul(7))) * deep;
    let mut mesh = egui::Mesh::default();
    let mut ridge = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = x0 + (x1 - x0) * t;
        let taper = (t * (1.0 - t) * 4.0).min(1.0).sqrt();
        let ripple = 0.78 + 0.22 * (t * 11.0 + seed as f32).sin() + 0.12 * (t * 27.0 + seed as f32 * 1.7).sin();
        let y = top - depth * taper * ripple;
        ridge.push(pos2(x, y));
        mesh.colored_vertex(pos2(x, y), Color32::from_rgb(238, 246, 250));
        mesh.colored_vertex(pos2(x, top + width * 0.42), Color32::from_rgb(178, 199, 212));
    }
    for i in 0..steps as u32 {
        mesh.add_triangle(i * 2, i * 2 + 1, i * 2 + 2);
        mesh.add_triangle(i * 2 + 1, i * 2 + 3, i * 2 + 2);
    }
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::line(ridge, Stroke::new(0.8, Color32::from_white_alpha(150))));
}

/// Icicles under a run of snow, as long as there is room for.
fn icicles(painter: &Painter, x0: f32, x1: f32, bottom: f32, hang: f32, seed: u32) {
    let mut x = x0 + 8.0 + 10.0 * hash(seed.wrapping_mul(11));
    let mut k = 0u32;
    while x < x1 - 6.0 {
        let s = seed.wrapping_mul(13).wrapping_add(k);
        let length = (3.0 + 7.0 * hash(s)).min(hang);
        let half = 0.9 + 0.6 * hash(s.wrapping_add(1));
        let tip = pos2(x + 0.4, bottom + length);
        painter.add(Shape::convex_polygon(vec![pos2(x - half, bottom - 0.5), pos2(x + half, bottom - 0.5), tip],
            Color32::from_rgba_unmultiplied(205, 226, 236, 215), Stroke::NONE));
        painter.line_segment([pos2(x - half * 0.4, bottom), tip], Stroke::new(0.6, Color32::from_white_alpha(170)));
        x += 9.0 + 16.0 * hash(s.wrapping_add(2));
        k += 1;
    }
}

/// A string of lights: red, green, gold and blue, each glowing softly in its own time
/// (steadily with reduced flashes). Where there is room under the pipe the string hangs
/// there, sagging between its hooks; where there is not, it is wound along the pipe itself.
#[allow(clippy::too_many_arguments)]
fn lights(painter: &Painter, left: f32, right: f32, top: f32, width: f32, hang: f32, time: f32, quiet: bool) {
    const PITCH: f32 = 34.0;
    const COLOURS: [Color32; 4] = [Color32::from_rgb(236, 70, 62), Color32::from_rgb(96, 214, 112),
        Color32::from_rgb(248, 200, 92), Color32::from_rgb(96, 170, 250)];
    let hanging = hang >= 5.0;
    let (line, sag, drop) = if hanging { (top + width + 0.6, (hang - 3.4).clamp(1.0, 4.5), 1.6) } else { (top + width * 0.62, 0.0, 0.0) };
    let first = (left / PITCH).ceil() as i32;
    let last = (right / PITCH).floor() as i32;
    if last <= first {
        return;
    }
    let wire: Vec<Pos2> = (first * 4..=last * 4).map(|q| {
        let x = q as f32 * PITCH / 4.0;
        let t = (q.rem_euclid(4)) as f32 / 4.0;
        pos2(x, line + sag * (t * std::f32::consts::PI).sin())
    }).collect();
    painter.add(Shape::line(wire, Stroke::new(0.9, Color32::from_rgb(28, 40, 30))));
    for i in first..last {
        let at = pos2((i as f32 + 0.5) * PITCH, line + sag + drop);
        let colour = COLOURS[i.rem_euclid(4) as usize];
        let glow = if quiet { 0.85 } else { 0.72 + 0.28 * (time * 1.3 + hash((i as u32).wrapping_mul(17)) * 6.3).sin() };
        // (Its halo only as far as the room under the pipe goes.)
        let halo = (top + width + hang - at.y).clamp(1.9, 4.2);
        painter.circle_filled(at, halo, colour.gamma_multiply(0.16 * glow));
        painter.rect_filled(Rect::from_center_size(at - vec2(0.0, 2.1), vec2(1.8, 1.4)), 0.4, Color32::from_rgb(60, 66, 58));
        painter.circle_filled(at, 1.9, colour.gamma_multiply(0.55 + 0.45 * glow));
        painter.circle_filled(at - vec2(0.5, 0.6), 0.7, Color32::from_white_alpha((120.0 + 100.0 * glow) as u8));
    }
}

/// Gold ribbon wound round a pipe: bands across it, leaning with the wind of the winding.
fn ribbon(painter: &Painter, x0: f32, x1: f32, top: f32, width: f32, seed: u32) {
    let lean = width * 0.7;
    let band = 3.2;
    let mut x = x0;
    let mut k = 0;
    while x + band + lean < x1 {
        let colour = if seed.wrapping_add(k).is_multiple_of(5) { CYAN } else { GOLD };
        let quad = vec![pos2(x, top - 0.6), pos2(x + band, top - 0.6), pos2(x + band + lean, top + width + 0.6), pos2(x + lean, top + width + 0.6)];
        painter.add(Shape::convex_polygon(quad, colour, Stroke::NONE));
        painter.line_segment([pos2(x + 0.5, top - 0.2), pos2(x + 0.5 + lean, top + width + 0.2)], Stroke::new(0.7, Color32::from_white_alpha(120)));
        x += 8.5;
        k += 1;
    }
}

/// A tassel of gold strands hanging from a ribbon's end, swaying a little.
fn tassel(painter: &Painter, at: Pos2, hang: f32, time: f32, seed: u32) {
    let length = (hang - 1.0).min(8.0);
    let sway = 0.8 * (time * 1.1 + hash(seed) * 6.3).sin();
    painter.circle_filled(at + vec2(0.0, 0.8), 1.4, GOLD);
    for (k, dx) in [-1.0f32, 0.0, 1.0].iter().enumerate() {
        let end = at + vec2(dx * 1.1 + sway * (1.0 + k as f32 * 0.3), length * (0.85 + 0.15 * hash(seed.wrapping_add(k as u32))));
        painter.line_segment([at + vec2(dx * 0.6, 1.2), end], Stroke::new(0.8, if k == 1 { CYAN } else { GOLD }));
    }
}

/// A five-pointed gold star with a lit facet.
fn gold_star(painter: &Painter, centre: Pos2, radius: f32, turn: f32) {
    star(painter, centre, radius, 5, Color32::from_rgb(214, 168, 72), turn);
    star(painter, centre - vec2(0.3, 0.4), radius * 0.55, 5, Color32::from_rgb(255, 232, 160), turn);
}

/// A small cap at the outer end of each navigation plate. Its label and hit area are
/// unchanged, including keyboard focus and screen-reader semantics.
pub fn navigation(painter: &Painter, rect: Rect) {
    match frame(painter.ctx()).celebration {
        Some(Celebration::Christmas) => snow(painter, rect.right() - 40.0, rect.right() - 12.0, rect.top() + 1.0, 2.0, 4, 1.0),
        Some(Celebration::NewYear) => {
            let center = pos2(rect.right() - 18.0, rect.top() + 3.5);
            gold_star(painter, center, 4.4, 0.0);
            painter.circle_filled(center - vec2(10.0, -1.0), 1.1, CYAN);
            painter.circle_filled(center + vec2(8.0, 1.5), 0.9, CYAN);
        }
        Some(Celebration::Halloween) => pumpkin(painter, pos2(rect.right() - 18.0, rect.top() + 5.5), 4.2, 0.85, 8.0),
        None => {}
    }
}

/// The season on the loading screen's notes plate, while its own notes are shown: snow along
/// its top and a sprig of holly, gold stars at its corners, or a cobweb in its corner with a
/// spider let down from the top.
pub fn plate(painter: &Painter, rect: Rect, scale: f32) {
    let frame = frame(painter.ctx());
    let time = frame.elapsed as f32;
    match frame.celebration {
        Some(Celebration::Christmas) => {
            // Drifts along the top (from its corner, where the holly is, and on along it).
            let (left, right) = (rect.left() + 4.0 * scale, rect.right() - 4.0 * scale);
            snow(painter, left, left + (right - left) * 0.38, rect.top() + 1.0, 2.0 * scale, 7, 1.5 * scale);
            snow(painter, left + (right - left) * 0.55, right - (right - left) * 0.08, rect.top() + 1.0, 2.0 * scale, 12, 1.2 * scale);
            holly(painter, rect.left_top() + vec2(17.0, 5.0) * scale, 1.5 * scale);
        }
        Some(Celebration::NewYear) => {
            for (corner, turn) in [(rect.left_top() + vec2(13.0, 13.0) * scale, 0.2), (rect.right_top() + vec2(-13.0, 13.0) * scale, -0.2)] {
                gold_star(painter, corner, 6.0 * scale, turn);
            }
            // Confetti caught on the plate's top edge.
            for k in 0..9u32 {
                let x = rect.left() + rect.width() * (0.1 + 0.8 * hash(k.wrapping_mul(97).wrapping_add(5)));
                let colour = [GOLD, CYAN, ROSE, CREAM][k as usize % 4];
                let spin = hash(k.wrapping_add(41)) * 3.0;
                let size = vec2(4.2, 2.0) * scale;
                let corners = [vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0)].map(|c| {
                    let c = c * size * 0.5;
                    pos2(x, rect.top() + 2.5 * scale) + vec2(c.x * spin.cos() - c.y * spin.sin(), c.x * spin.sin() + c.y * spin.cos())
                });
                painter.add(Shape::convex_polygon(corners.to_vec(), colour, Stroke::NONE));
            }
        }
        Some(Celebration::Halloween) => {
            cobweb(painter, rect.left_top() + vec2(5.0, 5.0) * scale, 26.0 * scale);
            // The spider, bobbing on its thread (still with reduced motion: the clock stops).
            let anchor = rect.right_top() + vec2(-28.0 * scale, 4.0 * scale);
            let drop = (10.0 + 3.0 * (time * 1.3).sin()) * scale;
            painter.line_segment([anchor, anchor + vec2(0.0, drop)], Stroke::new(0.8, Color32::from_white_alpha(140)));
            spider(painter, anchor + vec2(0.0, drop + 2.4 * scale), 3.2 * scale);
        }
        None => {}
    }
}

/// A sprig of holly: two leaves and three berries.
fn holly(painter: &Painter, at: Pos2, scale: f32) {
    let leaf = Color32::from_rgb(46, 122, 64);
    for (dx, lean) in [(-1.0f32, -0.5f32), (1.0, 0.5)] {
        let tip = at + vec2(dx * 9.0, lean * 4.0 - 2.0) * scale;
        let base = at + vec2(dx * 1.0, 0.0) * scale;
        let side = vec2(0.0, 2.6) * scale;
        let middle = base + (tip - base) * 0.5;
        painter.add(Shape::convex_polygon(vec![base, middle - side, tip, middle + side], leaf, Stroke::new(0.6, Color32::from_rgb(24, 70, 36))));
    }
    for (dx, dy) in [(-1.6f32, 1.6f32), (1.6, 1.6), (0.0, -0.4)] {
        painter.circle_filled(at + vec2(dx, dy) * scale, 1.8 * scale, Color32::from_rgb(208, 40, 44));
    }
}

/// A cobweb in a corner: threads fanning out from it, and the spiral between them.
fn cobweb(painter: &Painter, corner: Pos2, reach: f32) {
    let thread = Stroke::new(0.7, Color32::from_white_alpha(70));
    let spokes: Vec<f32> = (0..5).map(|k| k as f32 / 4.0 * std::f32::consts::FRAC_PI_2).collect();
    for &a in &spokes {
        painter.line_segment([corner, corner + vec2(a.cos(), a.sin()) * reach], thread);
    }
    for ring in 1..=4 {
        let r = reach * ring as f32 / 4.6;
        let points: Vec<Pos2> = spokes.iter().map(|a| corner + vec2(a.cos(), a.sin()) * r).collect();
        // (Each strand sagging a little between its spokes.)
        for pair in points.windows(2) {
            let middle = pair[0] + (pair[1] - pair[0]) * 0.5;
            let sag = middle + (middle - corner).normalized() * -r * 0.08;
            painter.add(Shape::line(vec![pair[0], sag, pair[1]], thread));
        }
    }
}

/// A small spider: a body, a head and four legs a side, in a purple light enough to see on the
/// dark plate, its body lit from above.
fn spider(painter: &Painter, at: Pos2, size: f32) {
    let ink = Color32::from_rgb(104, 82, 124);
    for side in [-1.0f32, 1.0] {
        for k in 0..4 {
            let lift = (k as f32 - 1.5) * 0.55;
            let knee = at + vec2(side * size * 1.3, lift * size - size * 0.6);
            let foot = at + vec2(side * size * 2.0, lift * size * 1.4 + size * 0.4);
            painter.add(Shape::line(vec![at, knee, foot], Stroke::new((size * 0.22).max(0.6), ink)));
        }
    }
    painter.circle_filled(at, size, ink);
    painter.circle_filled(at - vec2(0.0, size * 1.1), size * 0.6, ink);
    painter.circle_filled(at - vec2(size * 0.3, size * 0.35), size * 0.32, Color32::from_rgb(160, 136, 182));
}

const GOLD: Color32 = Color32::from_rgb(228, 197, 128);
const CYAN: Color32 = Color32::from_rgb(134, 211, 214);
const ROSE: Color32 = Color32::from_rgb(236, 128, 146);
const CREAM: Color32 = Color32::from_rgb(246, 238, 214);
const PUMPKIN: Color32 = Color32::from_rgb(240, 134, 40);
const PURPLE: Color32 = Color32::from_rgb(126, 72, 168);
const HAT: Color32 = Color32::from_rgb(44, 26, 60);

/// Two tethered faction balloons share the seasonal clock, so their gentle sway stops
/// in the background and with reduced motion. The spare header margin bounds their
/// complete animation; neither balloon can cross the title or the window's top edge.
fn new_year_logo(painter: &Painter, rect: Rect) {
    let scale = rect.width() / 48.0;
    let time = frame(painter.ctx()).elapsed as f32;
    for (arm, side, phase) in [(true, -1.0, 0.0), (false, 1.0, 2.1)] {
        let sway = (time * 0.72 + phase).sin();
        let bob = (time * 0.91 + phase).sin();
        let center = pos2(rect.center().x, rect.top())
            + vec2(side * 23.0 + sway * 1.1, (if arm { 9.0 } else { 11.0 }) + bob * 1.5) * scale;
        let lean = sway * 0.055;
        let at = |x: f32, y: f32| center + vec2(x * lean.cos() - y * lean.sin(), x * lean.sin() + y * lean.cos()) * scale;
        let knot = at(0.0, 11.7);
        let tie = pos2(rect.center().x + side * 15.0 * scale, rect.bottom() - 3.0 * scale);
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            [knot, knot + vec2(side * 3.0, 9.0) * scale, tie + vec2(side * 7.0, -7.0) * scale, tie],
            false,
            Color32::TRANSPARENT,
            Stroke::new(0.65 * scale, Color32::from_rgb(160, 171, 183)),
        ));
        let (light, edge, outline) = if arm {
            (Color32::from_rgb(63, 147, 236), Color32::from_rgb(20, 64, 136), Color32::from_rgb(111, 185, 242))
        } else {
            (Color32::from_rgb(239, 83, 87), Color32::from_rgb(133, 25, 43), Color32::from_rgb(251, 144, 141))
        };
        // A radial triangle fan gives the balloon volume without textures, mip shimmer,
        // brightness pulses or a flashing highlight as it moves.
        let mut body = egui::Mesh::default();
        body.colored_vertex(at(-2.0, -3.0), light);
        let mut rim = Vec::with_capacity(33);
        for i in 0..32 {
            let angle = i as f32 * std::f32::consts::TAU / 32.0;
            let y = angle.sin();
            let point = at(angle.cos() * 8.5 * (1.0 - y.max(0.0) * 0.12), y * 10.8);
            rim.push(point);
            body.colored_vertex(point, edge);
        }
        for i in 0..32 {
            body.add_triangle(0, i + 1, (i + 1) % 32 + 1);
        }
        painter.add(Shape::mesh(body));
        rim.push(rim[0]);
        painter.add(Shape::line(rim, Stroke::new(0.65 * scale, outline.gamma_multiply(0.7))));
        painter.add(Shape::convex_polygon(vec![at(0.0, 10.0), at(-1.7, 13.0), at(1.7, 13.0)], edge, Stroke::NONE));
        painter.add(Shape::line(vec![at(-5.7, -3.5), at(-5.0, -5.7), at(-3.4, -7.2)], Stroke::new(1.2 * scale, Color32::from_white_alpha(104))));
        crate::faction::mark(painter, &at, scale, arm);
    }
    crate::motion::next_frame(painter.ctx(), Duration::from_millis(33));
}

fn star(painter: &Painter, center: Pos2, radius: f32, rays: usize, color: Color32, turn: f32) {
    // Triangles avoid epaint's convex-polygon restriction for concave stars.
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, color);
    for i in 0..rays * 2 {
        let angle = i as f32 * std::f32::consts::PI / rays as f32 - std::f32::consts::FRAC_PI_2 + turn;
        let r = if i % 2 == 0 { radius } else { radius * if rays == 5 { 0.45 } else { 0.27 } };
        mesh.colored_vertex(center + vec2(angle.cos(), angle.sin()) * r, color);
    }
    for i in 0..rays * 2 {
        mesh.add_triangle(0, i as u32 + 1, ((i + 1) % (rays * 2)) as u32 + 1);
    }
    painter.add(Shape::mesh(mesh));
}

/// Painted after the background and before page widgets: particles can cross empty
/// gutters, but never make text harder to read or intercept clicks. Work is bounded.
pub fn background(painter: &Painter, rect: Rect) {
    let frame = frame(painter.ctx());
    if frame.celebration.is_none() || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let painter = painter.with_clip_rect(rect);
    // Each area its own pattern: two gutters side by side must not mirror one another.
    let salt = (rect.min.x.to_bits() ^ rect.min.y.to_bits().rotate_left(13)).wrapping_mul(0x9e37_79b9) >> 8;
    match frame.celebration {
        Some(Celebration::NewYear) => new_year_background(&painter, rect, &frame, salt),
        Some(Celebration::Halloween) => halloween_background(&painter, rect, &frame, salt),
        _ => snowfall(&painter, rect, &frame, salt),
    }
    crate::motion::next_frame(painter.ctx(), Duration::from_millis(33));
}

/// Snow in three depths: far flakes small and slow, near ones large, soft and quicker, a few
/// of them six-armed crystals turning slowly.
fn snowfall(painter: &Painter, rect: Rect, frame: &Frame, salt: u32) {
    let total = ((rect.area() / 5200.0) as usize).clamp(24, 180);
    let t = frame.elapsed as f32;
    let height = rect.height() + 16.0;
    // (count share, radius range, opacity, speed range, sway)
    for (depth, (share, r0, r1, alpha, v0, v1, sway)) in [(0.55, 1.0, 1.6, 130.0, 8.0, 13.0, 4.0), (0.30, 1.7, 2.6, 185.0, 14.0, 22.0, 7.0), (0.15, 2.8, 4.2, 225.0, 24.0, 34.0, 11.0)].into_iter().enumerate() {
        let count = ((total as f32 * share) as usize).max(3);
        for index in 0..count {
            let seed = ((depth as u32 + 1) * 1009 + index as u32).wrapping_add(salt);
            let speed = v0 + hash(seed.wrapping_mul(13)) * (v1 - v0);
            let y = (hash(seed.wrapping_mul(17)) * height + t * speed).rem_euclid(height) - 8.0;
            let x = (hash(seed.wrapping_mul(23)) * rect.width() + (t * (0.25 + 0.2 * hash(seed.wrapping_mul(7))) + seed as f32).sin() * sway).rem_euclid(rect.width());
            let radius = r0 + hash(seed.wrapping_mul(29)) * (r1 - r0);
            let at = rect.min + vec2(x, y);
            let colour = Color32::from_rgba_unmultiplied(224, 240, 248, alpha as u8);
            if depth == 2 && index % 3 == 0 {
                crystal(painter, at, radius * 1.9, t * 0.4 + seed as f32, colour);
            } else {
                if depth > 0 {
                    painter.circle_filled(at, radius * 1.9, colour.gamma_multiply(0.18));
                }
                painter.circle_filled(at, radius, colour);
            }
        }
    }
}

/// A snow crystal: six arms with a pair of branches each.
fn crystal(painter: &Painter, centre: Pos2, radius: f32, turn: f32, colour: Color32) {
    let stroke = Stroke::new((radius * 0.16).max(0.8), colour);
    for arm in 0..6 {
        let angle = turn + arm as f32 * std::f32::consts::TAU / 6.0;
        let dir = vec2(angle.cos(), angle.sin());
        let tip = centre + dir * radius;
        painter.line_segment([centre, tip], stroke);
        let fork = centre + dir * radius * 0.55;
        for side in [-1.0f32, 1.0] {
            let branch = vec2((angle + side * 0.75).cos(), (angle + side * 0.75).sin());
            painter.line_segment([fork, fork + branch * radius * 0.35], stroke);
        }
    }
    painter.circle_filled(centre, radius * 0.18, colour);
}

/// Halloween's gutters: a low mist drifting over the ground, embers rising from it, and bats
/// flitting across over them.
fn halloween_background(painter: &Painter, rect: Rect, frame: &Frame, salt: u32) {
    let time = frame.elapsed as f32;
    mist(painter, rect, time, salt);
    embers(painter, rect, time, frame.quiet, salt);
    bats(painter, rect, time, salt);
}

/// A low mist: soft bands near the foot, drifting and swelling slowly, each fading in above and
/// out below, and thinning to nothing at the area's sides.
fn mist(painter: &Painter, rect: Rect, time: f32, salt: u32) {
    let smooth = |x: f32| { let x = x.clamp(0.0, 1.0); x * x * (3.0 - 2.0 * x) };
    for band in 0..3u32 {
        let s = salt.wrapping_add(band.wrapping_mul(71));
        let base = rect.bottom() - rect.height() * (0.03 + 0.08 * band as f32);
        let thick = rect.height() * (0.10 + 0.04 * hash(s));
        let drift = time * (6.0 + 5.0 * hash(s.wrapping_add(3))) * if band % 2 == 0 { 1.0 } else { -1.0 };
        let alpha = 26.0 + 14.0 * hash(s.wrapping_add(5));
        let steps = 24;
        let mut mesh = egui::Mesh::default();
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let x = rect.left() + rect.width() * t;
            let swell = 0.55 + 0.45 * ((x + drift) * 0.012 + s as f32).sin() * ((x - drift * 0.7) * 0.021).cos();
            let crest = base - thick * swell.max(0.15);
            let colour = Color32::from_rgba_unmultiplied(150, 128, 176, (alpha * smooth(t / 0.22) * smooth((1.0 - t) / 0.22)) as u8);
            mesh.colored_vertex(pos2(x, crest), Color32::TRANSPARENT);
            mesh.colored_vertex(pos2(x, base), colour);
            mesh.colored_vertex(pos2(x, base + thick * 0.7), Color32::TRANSPARENT);
        }
        for i in 0..steps as u32 {
            let (a, b) = (i * 3, i * 3 + 3);
            for row in 0..2 {
                mesh.add_triangle(a + row, a + row + 1, b + row);
                mesh.add_triangle(a + row + 1, b + row + 1, b + row);
            }
        }
        painter.add(Shape::mesh(mesh));
    }
}

/// Embers rising slowly from the mist and fading, flickering (steady with reduced flashes).
fn embers(painter: &Painter, rect: Rect, time: f32, quiet: bool, salt: u32) {
    let count = ((rect.area() / 30000.0) as usize).clamp(6, 40);
    let height = rect.height() * 0.6;
    for index in 0..count {
        let s = (index as u32 + 7).wrapping_add(salt).wrapping_mul(2654435761);
        let speed = 6.0 + 8.0 * hash(s.wrapping_add(1));
        let rise = (hash(s.wrapping_add(2)) * height + time * speed).rem_euclid(height);
        let x = rect.left() + (hash(s.wrapping_add(3)) * rect.width() + (time * 0.6 + hash(s) * 6.3).sin() * 6.0).rem_euclid(rect.width());
        let at = pos2(x, rect.bottom() - rise);
        let fade = 1.0 - rise / height;
        let flicker = if quiet { 0.8 } else { 0.6 + 0.4 * (time * 5.0 + hash(s.wrapping_add(4)) * 6.3).sin().abs() };
        painter.circle_filled(at, 2.6, PUMPKIN.gamma_multiply(0.12 * fade * flicker));
        painter.circle_filled(at, 1.0, Color32::from_rgb(255, 176, 80).gamma_multiply(0.85 * fade * flicker));
    }
}

/// Bats flitting across: each on its own way and height, wings beating.
fn bats(painter: &Painter, rect: Rect, time: f32, salt: u32) {
    let count = ((rect.area() / 60000.0) as u32).clamp(2, 9);
    for k in 0..count {
        let s = k.wrapping_add(salt).wrapping_mul(40503);
        let way = if hash(s.wrapping_add(1)) < 0.5 { 1.0 } else { -1.0 };
        let speed = 22.0 + 20.0 * hash(s.wrapping_add(2));
        let span = rect.width() + 60.0;
        let x = (hash(s.wrapping_add(3)) * span + time * speed * way).rem_euclid(span) - 30.0;
        let y = rect.top() + rect.height() * (0.08 + 0.55 * hash(s.wrapping_add(4))) + 12.0 * (time * 0.8 + hash(s) * 6.3).sin();
        let beat = (time * (8.0 + 5.0 * hash(s.wrapping_add(5))) + hash(s.wrapping_add(6)) * 6.3).sin();
        bat(painter, pos2(rect.left() + x, y), 5.0 + 4.5 * hash(s.wrapping_add(7)), beat);
    }
}

/// A bat `size` across a wing, its wings at `beat` (-1 down to 1 up): a silhouette in the
/// purple of a Halloween night (a black one would vanish on the dark plates), the nearer and
/// bigger the brighter.
fn bat(painter: &Painter, at: Pos2, size: f32, beat: f32) {
    let near = ((size - 5.0) / 4.5).clamp(0.0, 1.0);
    let mix = |far: f32, close: f32| (far + (close - far) * near) as u8;
    let ink = Color32::from_rgb(mix(50.0, 98.0), mix(38.0, 68.0), mix(62.0, 132.0));
    for side in [-1.0f32, 1.0] {
        let tip = at + vec2(side * size * 1.6, -beat * size * 0.9);
        let elbow = at + vec2(side * size * 0.8, -beat * size * 0.5 - size * 0.25);
        // The wing's scalloped trailing edge, from its tip back to the body.
        let edge = [at + vec2(side * size * 1.25, -beat * size * 0.55 + size * 0.25), at + vec2(side * size * 0.75, -beat * size * 0.25 + size * 0.35), at + vec2(side * size * 0.3, size * 0.3)];
        let mut mesh = egui::Mesh::default();
        for p in [at, elbow, tip, edge[0], edge[1], edge[2]] {
            mesh.colored_vertex(p, ink);
        }
        for (a, b, c) in [(0, 1, 5), (1, 4, 5), (1, 3, 4), (1, 2, 3)] {
            mesh.add_triangle(a, b, c);
        }
        painter.add(Shape::mesh(mesh));
    }
    painter.circle_filled(at, size * 0.32, ink);
    painter.circle_filled(at - vec2(0.0, size * 0.32), size * 0.22, ink);
}

fn new_year_background(painter: &Painter, rect: Rect, frame: &Frame, salt: u32) {
    if frame.quiet {
        // Reduced flashes: no bursts at all, only a few still sparkles where they would be.
        for (x, y, radius) in [(0.09, 0.19, 3.0), (0.89, 0.65, 3.4), (0.22, 0.72, 2.4), (0.74, 0.24, 2.6)] {
            gold_star(painter, rect.min + vec2(rect.width() * x, rect.height() * y), radius, 0.0);
        }
    } else {
        fireworks(painter, rect, frame.firework_elapsed as f32, salt);
    }
    confetti(painter, rect, frame.elapsed as f32, salt);
}

/// Soft bursts: sparks thrown out from a point, falling a little as they slow, fading as they
/// go. No flash where they open, no wash over the screen: each burst is only its sparks.
fn fireworks(painter: &Painter, rect: Rect, time: f32, salt: u32) {
    let bursts = ((rect.area() / 150000.0) as u32).clamp(2, 6);
    const PALETTE: [Color32; 4] = [GOLD, CYAN, ROSE, CREAM];
    for k in (0..bursts).map(|k| k.wrapping_add(salt)) {
        let period = 4.2 + hash(k.wrapping_mul(7).wrapping_add(1)) * 2.6;
        let offset = hash(k.wrapping_mul(7).wrapping_add(2)) * period;
        let since = (time + offset).rem_euclid(period);
        let round = ((time + offset) / period).floor() as u32;
        const OPEN: f32 = 1.8;
        if since > OPEN {
            continue;
        }
        let s = k.wrapping_mul(97).wrapping_add(round.wrapping_mul(13));
        let centre = rect.min + vec2(rect.width() * (0.06 + 0.88 * hash(s)), rect.height() * (0.10 + 0.55 * hash(s.wrapping_add(1))));
        let reach = 34.0 + 30.0 * hash(s.wrapping_add(2));
        let colour = PALETTE[(hash(s.wrapping_add(3)) * 4.0) as usize % 4];
        let f = since / OPEN;
        let out = 1.0 - (1.0 - f).powi(3);
        let fade = (1.0 - f).powf(1.6);
        let sparks = 20;
        for i in 0..sparks {
            let angle = i as f32 * std::f32::consts::TAU / sparks as f32 + hash(s.wrapping_add(10 + i)) * 0.2;
            let dir = vec2(angle.cos(), angle.sin());
            let drop = vec2(0.0, 18.0 * f * f);
            let head = centre + dir * reach * out + drop;
            let tail = centre + dir * reach * (out - 0.16).max(0.0) + drop * 0.8;
            painter.line_segment([tail, head], Stroke::new(1.5, colour.gamma_multiply(0.62 * fade)));
            painter.circle_filled(head, 1.8, colour.gamma_multiply(0.95 * fade));
        }
    }
}

/// Confetti tumbling down: little cards turning over as they fall and drift.
fn confetti(painter: &Painter, rect: Rect, time: f32, salt: u32) {
    const PALETTE: [Color32; 4] = [GOLD, CYAN, ROSE, CREAM];
    let count = ((rect.area() / 8500.0) as usize).clamp(16, 110);
    let height = rect.height() + 12.0;
    for index in 0..count {
        let seed = (index as u32 + 1).wrapping_add(salt);
        let speed = 10.0 + hash(seed.wrapping_mul(13)) * 14.0;
        let y = (hash(seed.wrapping_mul(17)) * height + time * speed).rem_euclid(height) - 6.0;
        let x = (hash(seed.wrapping_mul(23)) * rect.width() + (time * 0.5 + seed as f32).sin() * 9.0).rem_euclid(rect.width());
        let centre = rect.min + vec2(x, y);
        let angle = time * (0.8 + hash(seed.wrapping_mul(31)) * 1.6) + hash(seed.wrapping_mul(37)) * 6.3;
        let flip = (time * (1.6 + hash(seed.wrapping_mul(41)) * 2.0) + seed as f32).cos();
        let (w, h) = (2.3 * flip.abs().max(0.15), 4.4);
        let (c, s) = (angle.cos(), angle.sin());
        let corner = |dx: f32, dy: f32| centre + vec2(dx * c - dy * s, dx * s + dy * c);
        let colour = PALETTE[index % 4].gamma_multiply(if flip > 0.0 { 0.9 } else { 0.62 });
        painter.add(Shape::convex_polygon(vec![corner(-w, -h * 0.5), corner(w, -h * 0.5), corner(w, h * 0.5), corner(-w, h * 0.5)], colour, Stroke::NONE));
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

    #[test]
    fn seasonal_calendar_gives_new_year_precedence_and_respects_opt_out() {
        let mut settings = Launcher::default();
        for (month, day, expected) in [
            (11, 30, None), (12, 1, Some(Celebration::Christmas)),
            (12, 30, Some(Celebration::Christmas)), (12, 31, Some(Celebration::NewYear)),
            (1, 1, Some(Celebration::NewYear)), (1, 2, Some(Celebration::Christmas)),
            (1, 6, Some(Celebration::Christmas)), (1, 7, None), (7, 14, None),
            (12, 0, None), (12, 32, None),
            (10, 23, None), (10, 24, Some(Celebration::Halloween)), (10, 31, Some(Celebration::Halloween)),
            (11, 1, Some(Celebration::Halloween)), (11, 2, None),
        ] {
            assert_eq!(celebration(&settings, month, day), expected, "{month}/{day}");
        }
        for (theme, expected) in [(SeasonalTheme::Christmas, Celebration::Christmas), (SeasonalTheme::NewYear, Celebration::NewYear), (SeasonalTheme::Halloween, Celebration::Halloween)] {
            settings.seasonal_theme = theme;
            assert_eq!(celebration(&settings, 7, 14), Some(expected));
            settings.seasonal_effects = false;
            assert_eq!(celebration(&settings, 7, 14), None);
            assert_eq!(celebration(&settings, 12, 31), None);
            settings.seasonal_effects = true;
        }
        settings.seasonal_effects = false;
        settings.seasonal_theme = SeasonalTheme::Automatic;
        assert_eq!(celebration(&settings, 12, 25), None);
        assert_eq!(celebration(&settings, 1, 1), None);
        assert_eq!(utc_date(0), (1970, 1, 1));
        assert_eq!(utc_date(19782), (2024, 2, 29)); // a leap day
        assert_eq!(utc_date(20800), (2026, 12, 13));
        assert_eq!(utc_date(20819), (2027, 1, 1));
    }

    #[test]
    fn seasonal_motion_freezes_when_reduced_or_unfocused() {
        for seasonal_theme in [SeasonalTheme::Christmas, SeasonalTheme::NewYear, SeasonalTheme::Halloween] {
            let ctx = Context::default();
            let settings = Launcher { seasonal_theme, ..Default::default() };
            let draw = |time: f64, still: bool, reduced: bool| {
                crate::motion::set_still(&ctx, still);
                crate::motion::set_reduced(&ctx, reduced);
                let mut output = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| begin(ui.ctx(), &settings, false));
                output.textures_delta.clear();
                frame(&ctx).elapsed
            };
            draw(0.0, false, false);
            let moving = draw(0.1, false, false);
            assert!(moving > 0.0);
            assert_eq!(draw(0.2, false, true), moving);
            assert_eq!(draw(4.0, true, false), moving);
            assert!((draw(100.0, false, false) - moving) <= 0.100001, "focus returns without a clock jump");
        }
    }

    fn logo_shapes(ctx: &Context, rect: Rect) -> Vec<Shape> {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("balloon_test")));
            logo(&painter, rect);
        });
        output.textures_delta.clear();
        output.shapes.into_iter().map(|clipped| clipped.shape).filter(|shape| *shape != Shape::Noop).collect()
    }

    #[test]
    fn balloons_fit_header_and_loading_emblem_clearance_throughout_their_motion() {
        let ctx = Context::default();
        for size in [48.0, 116.0, 174.0] {
            let scale = size / 48.0;
            let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(size, size));
            let allowed = Rect::from_min_max(rect.min - vec2(10.0, 5.0) * scale, rect.max + vec2(10.0, 0.0) * scale);
            for step in 0..128 {
                ctx.data_mut(|d| d.insert_temp(frame_id(), Frame {
                    celebration: Some(Celebration::NewYear), elapsed: step as f64 * 0.5, ..Default::default()
                }));
                let shapes = logo_shapes(&ctx, rect);
                assert!(!shapes.is_empty());
                for shape in shapes {
                    let bounds = shape.visual_bounding_rect();
                    assert!(allowed.contains_rect(bounds), "balloon outside reserved space at {size} / {step}: {bounds:?}");
                }
            }
        }
    }

    #[test]
    fn hats_keep_to_the_emblems_clearance() {
        let ctx = Context::default();
        for (celebration, above) in [(Celebration::Christmas, 9.0), (Celebration::Halloween, 9.0)] {
            ctx.data_mut(|d| d.insert_temp(frame_id(), Frame { celebration: Some(celebration), ..Default::default() }));
            for size in [48.0, 116.0, 174.0] {
                let scale = size / 48.0;
                let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(size, size));
                let allowed = Rect::from_min_max(rect.min - vec2(10.0, above) * scale, rect.max + vec2(10.0, 0.0) * scale);
                for shape in logo_shapes(&ctx, rect) {
                    let bounds = shape.visual_bounding_rect();
                    assert!(allowed.contains_rect(bounds), "{celebration:?} at {size}: {bounds:?} outside {allowed:?}");
                }
            }
        }
    }

    #[test]
    fn each_season_has_its_own_notes() {
        let ctx = Context::default();
        let mut seen = Vec::new();
        for theme in [SeasonalTheme::Christmas, SeasonalTheme::NewYear, SeasonalTheme::Halloween] {
            let settings = Launcher { seasonal_theme: theme, ..Default::default() };
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| begin(ui.ctx(), &settings, false));
            output.textures_delta.clear();
            let (label, _, notes) = notes(&ctx).expect("a season's notes");
            assert!(notes.len() >= 8 && !seen.contains(&label));
            seen.push(label);
        }
        let off = Launcher { seasonal_effects: false, ..Default::default() };
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| begin(ui.ctx(), &off, false));
        output.textures_delta.clear();
        assert!(notes(&ctx).is_none(), "out of season, or with the effects off, none");
    }

    #[test]
    fn balloons_follow_motion_preference_and_disappear_when_disabled() {
        let ctx = Context::default();
        let rect = Rect::from_min_size(pos2(40.0, 40.0), vec2(48.0, 48.0));
        let mut settings = Launcher { seasonal_theme: SeasonalTheme::NewYear, ..Default::default() };
        let update = |time, settings: &Launcher| {
            let mut output = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| begin(ui.ctx(), settings, false));
            output.textures_delta.clear();
            logo_shapes(&ctx, rect)
        };
        let first = update(0.0, &settings);
        let moving = update(0.1, &settings);
        assert_ne!(moving, first, "the two balloons gently sway");
        crate::motion::set_reduced(&ctx, true);
        assert_eq!(update(0.2, &settings), moving, "reduced motion preserves the exact resting arrangement");
        crate::motion::set_reduced(&ctx, false);
        crate::motion::set_still(&ctx, true);
        assert_eq!(update(0.3, &settings), moving, "an unfocused window stops animating");
        settings.seasonal_effects = false;
        assert!(update(0.4, &settings).is_empty(), "disabled effects paint no seasonal logo decoration");
    }

    /// Every decoration draws for both celebrations over areas anywhere on a screen (their seeds
    /// salted by where they are), with its work bounded, and nothing hangs from a pipe further
    /// than it is allowed to.
    #[test]
    fn decorations_draw_bounded_work_anywhere_and_keep_to_their_room() {
        for (seasonal_theme, quiet) in [(SeasonalTheme::Christmas, false), (SeasonalTheme::NewYear, false), (SeasonalTheme::NewYear, true), (SeasonalTheme::Halloween, false), (SeasonalTheme::Halloween, true)] {
            let ctx = Context::default();
            let settings = Launcher { seasonal_theme, ..Default::default() };
            for step in 0..4 {
                let mut output = ctx.run_ui(egui::RawInput { time: Some(step as f64 * 0.7), ..Default::default() }, |ui| begin(ui.ctx(), &settings, quiet));
                output.textures_delta.clear();
            }
            for rect in [Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0)), Rect::from_min_size(pos2(1517.3, 33.1), vec2(190.0, 600.0)),
                Rect::from_min_size(pos2(-40.0, 900.0), vec2(30.0, 12.0))] {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| background(ui.painter(), rect));
                output.textures_delta.clear();
                assert!(output.shapes.len() < 2000, "{seasonal_theme:?}: {} shapes", output.shapes.len());
            }
            for hang in [0.0, 2.0, 3.0, 9.0] {
                let (top, width) = (200.0, 5.0);
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    trim(ui.painter(), 0.0, 1600.0, top, width, hang);
                    navigation(ui.painter(), Rect::from_min_size(pos2(0.0, 300.0), vec2(220.0, 46.0)));
                });
                output.textures_delta.clear();
                let lowest = output.shapes.iter().filter(|s| s.shape.visual_bounding_rect().max.y < 290.0)
                    .map(|s| s.shape.visual_bounding_rect().max.y).fold(f32::MIN, f32::max);
                assert!(lowest <= top + width + hang + 1.0, "{seasonal_theme:?}: hangs to {lowest} with room for {hang}");
            }
        }
    }

    #[test]
    fn new_year_reduced_flashes_keeps_firework_outlines_fixed() {
        let ctx = Context::default();
        let settings = Launcher { seasonal_theme: SeasonalTheme::NewYear, ..Default::default() };
        let draw = |time: f64, reduced_flashes: bool| {
            let mut output = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| begin(ui.ctx(), &settings, reduced_flashes));
            output.textures_delta.clear();
            frame(&ctx)
        };
        draw(0.0, false);
        let moving = draw(0.1, false);
        assert!(moving.firework_elapsed > 0.0);
        let held = draw(0.2, true);
        assert_eq!(held.firework_elapsed, moving.firework_elapsed);
        assert!(held.elapsed > moving.elapsed, "gentle confetti remains independent from fireworks");
        assert_eq!(draw(2.0, true).firework_elapsed, moving.firework_elapsed);
        let resumed = draw(2.1, false);
        assert!(resumed.firework_elapsed > moving.firework_elapsed);
        assert!((resumed.firework_elapsed - moving.firework_elapsed) <= 0.100001);
    }
}
