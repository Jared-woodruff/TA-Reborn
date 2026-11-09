//! End-of-game (and pause-menu) statistics: per-player totals and graphs over time.

use egui::{Color32, CornerRadius, Pos2, RichText, Sense, Stroke, StrokeKind, Vec2, vec2};
use ta_sim::{Fx, StatSample};
use ta_ui::widgets as w;

use crate::game::Game;
use crate::theme::*;

/// Which series the graph shows.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Series {
    #[default]
    Army,
    Metal,
    Energy,
    Units,
}

impl Series {
    fn value(self, s: &StatSample) -> f32 {
        match self {
            Series::Army => s.army_value.to_f32(),
            Series::Metal => s.metal_income.to_f32(),
            Series::Energy => s.energy_income.to_f32(),
            Series::Units => s.units as f32,
        }
    }

    /// A reading of it, as the graph writes it.
    fn text(self, v: f32) -> String {
        match self {
            Series::Units => format!("{v:.0}"),
            _ => big(Fx::from_f32(v)),
        }
    }
}

/// A player's name as shown in the statistics.
pub(crate) fn player_name(game: &Game, i: usize) -> String {
    let side = game.sim.players.get(i).map_or("", |p| p.side.as_str()).to_string();
    let who = game.player_label(i as u8);
    // Name the teams only when some players share one.
    let players = &game.sim.players;
    let shared = players.iter().enumerate().any(|(a, pa)| players.iter().skip(a + 1).any(|pb| pa.team == pb.team));
    match players.get(i) {
        Some(p) if shared => format!("{who} · {side} · {}", ta_ui::i18n::tf("Team {n}", &[("n", &(p.team + 1))])),
        _ => format!("{who} · {side}"),
    }
}

/// A figure rounded as [`big`] shows it.
fn shown(v: Fx) -> f32 {
    let v = v.to_f32();
    if v >= 1_000_000.0 {
        (v / 100_000.0).round() * 100_000.0
    } else if v >= 10_000.0 {
        (v / 1000.0).round() * 1000.0
    } else {
        v.round()
    }
}

pub(crate) fn big(v: Fx) -> String {
    let v = v.to_f32();
    if v >= 1_000_000.0 {
        format!("{:.1} M", v / 1_000_000.0)
    } else if v >= 10_000.0 {
        format!("{:.0} k", v / 1000.0)
    } else {
        format!("{v:.0}")
    }
}

/// A battle's time, minutes and seconds, from its ticks.
pub(crate) fn clock(tick: u64) -> String {
    let secs = tick / 30;
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// Totals table plus a graph of the chosen series (the battle's statistics page).
pub fn panel(ui: &mut egui::Ui, game: &Game, series: &mut Series) {
    let shown: Vec<ta_sim::Player> = game.sim.players.iter().filter(|p| game.may_inspect(p.index)).cloned().collect();
    table(ui, game, &shown, 30.0);
    ui.add_space(12.0);
    series_picker(ui, series);
    ui.add_space(6.0);
    chart(ui, game, &shown, None, *series, 180.0);
}

/// Each player's figures as they stand now (see [`chart`]: a result's, kept from the moment the
/// battle was decided).
pub fn finals(game: &Game) -> Vec<Option<StatSample>> {
    (0..game.sim.players.len()).map(|i| now(game, i)).collect()
}

/// The graph's top: four times a round step (1, 2 or 5 times a power of ten) at least a quarter
/// of the highest reading, so that its quarter lines read round.
fn rounded_top(highest: f32) -> f32 {
    let quarter = (highest / 4.0).max(1.0);
    let power = 10f32.powf(quarter.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * power).find(|&s| s >= quarter * 0.999).unwrap_or(10.0 * power);
    step * 4.0
}

/// The choice of the graph's series.
pub fn series_picker(ui: &mut egui::Ui, series: &mut Series) {
    w::segmented(ui, series, &[(Series::Army, "ARMY VALUE"), (Series::Metal, "METAL INCOME"), (Series::Energy, "ENERGY INCOME"), (Series::Units, "UNITS")]);
}

/// The totals across the whole width (`players`: the battle's, or as they were when it was
/// decided): each player's name, then the figures in columns of one width, right-aligned under
/// their captions; the best of each column in brass. Rows `row` high.
pub fn table(ui: &mut egui::Ui, game: &Game, players: &[ta_sim::Player], row: f32) {
    use ta_ui::i18n::{n, t};
    let headers = [n("BUILT"), n("LOST"), n("KILLS"), n("METAL MADE"), n("ENERGY MADE"), n("METAL SPENT"), n("ENERGY SPENT")];
    let figures: Vec<[f32; 7]> = players
        .iter()
        .map(|p| {
            let s = &p.stats;
            // (As they are shown: two figures that read the same are the same.)
            [s.built as f32, s.lost as f32, s.kills as f32, shown(s.metal_produced), shown(s.energy_produced), shown(s.metal_spent), shown(s.energy_spent)]
        })
        .collect();
    let text = |p: &ta_sim::Player, k: usize| -> String {
        let s = &p.stats;
        match k {
            0 => s.built.to_string(),
            1 => s.lost.to_string(),
            2 => s.kills.to_string(),
            3 => big(s.metal_produced),
            4 => big(s.energy_produced),
            5 => big(s.metal_spent),
            _ => big(s.energy_spent),
        }
    };
    // (Losing the fewest is best; a figure every player shares is no one's best.)
    let best: Vec<Option<f32>> = (0..headers.len())
        .map(|k| {
            let values = figures.iter().map(|f| f[k]);
            let pick = if k == 1 { values.clone().fold(f32::MAX, f32::min) } else { values.clone().fold(f32::MIN, f32::max) };
            let shared = figures.iter().filter(|f| f[k] == pick).count();
            (players.len() > 1 && shared < players.len() && (k == 1 || pick > 0.0)).then_some(pick)
        })
        .collect();
    let width = ui.available_width();
    let first = (width * 0.27).clamp(170.0, 300.0);
    let column = (width - first) / headers.len() as f32;
    let (head, _) = ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
    let painter = ui.painter();
    painter.text(Pos2::new(head.left() + 8.0, head.center().y), egui::Align2::LEFT_CENTER, t("PLAYER"), strong(10.5), STEEL);
    for (k, h) in headers.iter().enumerate() {
        let right = head.left() + first + column * (k + 1) as f32 - 8.0;
        let galley = w::fitted_galley(ui, |size| RichText::new(t(h)).font(strong(size)).color(STEEL), 10.5, column - 10.0, 0.75);
        painter.galley(Pos2::new(right - galley.size().x, head.center().y - galley.size().y / 2.0), galley, STEEL);
    }
    painter.hline(head.x_range(), head.bottom() + 0.5, Stroke::new(1.0, LINE));
    let figure = if row >= 36.0 { 16.0 } else { 14.0 };
    for (i, p) in players.iter().enumerate() {
        let (rect, _) = ui.allocate_exact_size(vec2(width, row), Sense::hover());
        let painter = ui.painter();
        // The player's own row lit a little more.
        let mine = p.index == game.local_player && !game.spectator;
        if mine {
            painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(196, 160, 72, 22));
        } else if i % 2 == 0 {
            painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_white_alpha(6));
        }
        painter.rect_filled(egui::Rect::from_min_size(Pos2::new(rect.left() + 1.0, rect.top() + 6.0), vec2(4.0, rect.height() - 12.0)), CornerRadius::ZERO, crate::palette::team_color(p.color));
        let name = w::fitted_galley(ui, |size| {
            let r = RichText::new(player_name(game, p.index as usize)).font(strong(size));
            if p.defeated { r.strikethrough().color(STEEL_DIM) } else { r.color(TEXT_BRIGHT) }
        }, 13.5, first - 22.0, 0.8);
        painter.galley(Pos2::new(rect.left() + 12.0, rect.center().y - name.size().y / 2.0), name, TEXT_BRIGHT);
        for k in 0..headers.len() {
            let right = rect.left() + first + column * (k + 1) as f32 - 8.0;
            let lead = best[k] == Some(figures[i][k]);
            let color = if lead { AMBER_BRIGHT } else if p.defeated { STEEL } else { TEXT };
            painter.text(Pos2::new(right, rect.center().y), egui::Align2::RIGHT_CENTER, text(p, k), if lead { strong(figure) } else { body(figure) }, color);
        }
    }
    // In a team game, each team's totals under a rule.
    let mut teams: Vec<u8> = Vec::new();
    for p in players {
        if !teams.contains(&p.team) {
            teams.push(p.team);
        }
    }
    if teams.len() < 2 || teams.len() == players.len() {
        return;
    }
    let (rule, _) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    ui.painter().hline(rule.x_range(), rule.center().y, Stroke::new(1.0, LINE));
    for team in teams {
        let members: Vec<usize> = players.iter().enumerate().filter(|(_, p)| p.team == team).map(|(i, _)| i).collect();
        let (rect, _) = ui.allocate_exact_size(vec2(width, row), Sense::hover());
        let painter = ui.painter();
        let ours = !game.spectator && players.get(game.local_player as usize).is_some_and(|p| p.team == team);
        if ours {
            painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(196, 160, 72, 14));
        }
        let count = if members.len() == 1 { ta_ui::i18n::t("1 player").to_string() } else { ta_ui::i18n::tf("{n} players", &[("n", &members.len())]) };
        let label = format!("{} · {}", ta_ui::i18n::tf("Team {n}", &[("n", &(team + 1))]).to_uppercase(), count.to_uppercase());
        painter.text(Pos2::new(rect.left() + 12.0, rect.center().y), egui::Align2::LEFT_CENTER, label, strong(12.0), STEEL);
        for k in 0..headers.len() {
            let right = rect.left() + first + column * (k + 1) as f32 - 8.0;
            let sum: f64 = members.iter().map(|&i| {
                let s = &players[i].stats;
                match k {
                    0 => s.built as f64,
                    1 => s.lost as f64,
                    2 => s.kills as f64,
                    3 => s.metal_produced.to_f64(),
                    4 => s.energy_produced.to_f64(),
                    5 => s.metal_spent.to_f64(),
                    _ => s.energy_spent.to_f64(),
                }
            }).sum();
            let shown = if k <= 2 { format!("{sum:.0}") } else { big(ta_sim::Fx::from_f64(sum)) };
            painter.text(Pos2::new(right, rect.center().y), egui::Align2::RIGHT_CENTER, shown, strong(figure), TEXT_BRIGHT);
        }
    }
}

/// A graph of `series` over the battle (`players` and their `finals`: the battle's as it stands
/// now when none, or as they were when it was decided), `height` high: each player's line in
/// their colour, a key to whose is whose, the scale at its quarter lines and the time along its
/// foot; under the pointer, a line across the battle at that moment with every player's reading
/// on it.
pub fn chart(ui: &mut egui::Ui, game: &Game, players: &[ta_sim::Player], finals: Option<&[Option<StatSample>]>, series: Series, height: f32) {
    // Each player's readings, every thirty seconds, and the battle as it stands now (so that
    // the lines run from the start to now, however short the battle).
    let last = |i: usize| match finals {
        Some(f) => f.get(i).copied().flatten(),
        None => now(game, i),
    };
    let readings: Vec<Vec<StatSample>> = players.iter().map(|p| p.stats.history.iter().copied().chain(last(p.index as usize).filter(|s| p.stats.history.last().is_none_or(|l| s.tick > l.tick))).collect()).collect();
    let end = readings.iter().filter_map(|r| r.last()).map(|s| s.tick).max().unwrap_or(1).max(1) as f32;
    let start = readings.iter().filter_map(|r| r.first()).map(|s| s.tick).min().unwrap_or(0) as f32;
    // Nothing to draw yet (the battle not begun): a short well with its message, and no
    // scale or lines to read.
    let drawn = readings.iter().any(|r| r.len() >= 2);
    if drawn {
        // The key: a swatch and a name each, wrapping between them.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(16.0, 4.0);
            for p in players {
                let name = w::galley(ui, RichText::new(player_name(game, p.index as usize)).font(body(12.5)).color(if p.defeated { STEEL } else { TEXT }));
                let (rect, _) = ui.allocate_exact_size(vec2(18.0 + name.size().x, name.size().y.max(14.0)), Sense::hover());
                let swatch = egui::Rect::from_center_size(Pos2::new(rect.left() + 6.0, rect.center().y), vec2(12.0, 4.0));
                ui.painter().rect_filled(swatch, CornerRadius::same(1), crate::palette::team_color(p.color));
                ui.painter().galley(Pos2::new(rect.left() + 18.0, rect.center().y - name.size().y / 2.0), name, TEXT);
            }
        });
        ui.add_space(4.0);
    }
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width().max(420.0), if drawn { height } else { 64.0 }), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect(rect, CornerRadius::same(2), INSET, Stroke::new(1.0, LINE), StrokeKind::Inside);
    if !drawn {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, ta_ui::i18n::t("Graphs fill in every 30 seconds"), body(13.0), STEEL_DIM);
        return;
    }
    let top = rounded_top(readings.iter().flatten().map(|s| series.value(s)).fold(1.0f32, f32::max));
    let inner = egui::Rect::from_min_max(rect.min + vec2(12.0, 18.0), rect.max - vec2(12.0, 22.0));
    let small = strong(11.0);
    // Quarter lines, with the scale at each.
    for k in 1..=4 {
        let y = inner.bottom() - inner.height() * k as f32 / 4.0;
        painter.hline(inner.x_range(), y, Stroke::new(1.0, LINE.gamma_multiply(0.6)));
        painter.text(Pos2::new(inner.left() + 2.0, y - 2.0), egui::Align2::LEFT_BOTTOM, series.text(top * k as f32 / 4.0), small.clone(), STEEL_DIM);
    }
    painter.hline(inner.x_range(), inner.bottom(), Stroke::new(1.0, LINE_HI));
    // The time along the foot: the start, the middle and the end.
    for (k, align) in [(0.0, egui::Align2::LEFT_TOP), (0.5, egui::Align2::CENTER_TOP), (1.0, egui::Align2::RIGHT_TOP)] {
        let tick = start + (end - start) * k;
        painter.text(Pos2::new(inner.left() + inner.width() * k, inner.bottom() + 4.0), align, clock(tick as u64), small.clone(), STEEL);
    }
    let at = |tick: f32, v: f32| Pos2::new(inner.left() + inner.width() * (tick - start) / (end - start).max(1.0), inner.bottom() - inner.height() * v / top);
    for (p, r) in players.iter().zip(&readings) {
        let pts: Vec<Pos2> = r.iter().map(|s| at(s.tick as f32, series.value(s))).collect();
        if pts.len() >= 2 {
            painter.add(egui::Shape::line(pts.clone(), Stroke::new(4.5, Color32::from_black_alpha(140))));
            painter.add(egui::Shape::line(pts, Stroke::new(2.5, crate::palette::team_color(p.color))));
        }
    }
    // Under the pointer: the moment's readings.
    if let Some(pointer) = response.hover_pos().filter(|p| inner.x_range().contains(p.x)) {
        let tick = start + (pointer.x - inner.left()) / inner.width().max(1.0) * (end - start);
        painter.vline(pointer.x, inner.y_range(), Stroke::new(1.0, LINE_HI));
        painter.text(Pos2::new(pointer.x, rect.top() + 3.0), egui::Align2::CENTER_TOP, clock(tick as u64), small.clone(), CREAM);
        let mut marks: Vec<(f32, f32, Color32)> = players
            .iter()
            .zip(&readings)
            .filter_map(|(p, r)| reading_at(r, tick, series).map(|v| (v, at(tick, v).y, crate::palette::team_color(p.color))))
            .collect();
        // (Their labels kept apart, top to bottom.)
        marks.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut last = f32::MIN;
        let right = pointer.x > inner.center().x;
        for (v, y, colour) in marks {
            painter.circle(Pos2::new(pointer.x, y), 3.5, colour, Stroke::new(1.0, Color32::from_black_alpha(200)));
            let label_y = y.max(last + 14.0).min(inner.bottom());
            last = label_y;
            let (x, align) = if right { (pointer.x - 7.0, egui::Align2::RIGHT_CENTER) } else { (pointer.x + 7.0, egui::Align2::LEFT_CENTER) };
            let galley = painter.layout_no_wrap(series.text(v), strong(12.0), colour);
            let box_rect = align.anchor_size(Pos2::new(x, label_y), galley.size()).expand2(vec2(4.0, 1.0));
            painter.rect_filled(box_rect, CornerRadius::same(2), Color32::from_black_alpha(190));
            painter.galley(box_rect.min + vec2(4.0, 1.0), galley, colour);
        }
    }
}

/// A player's reading at `tick` (between two readings, on the line between them; none outside
/// their readings).
fn reading_at(readings: &[StatSample], tick: f32, series: Series) -> Option<f32> {
    let after = readings.iter().position(|s| s.tick as f32 >= tick)?;
    let b = &readings[after];
    if after == 0 {
        return (b.tick as f32 <= tick + 0.5).then(|| series.value(b));
    }
    let a = &readings[after - 1];
    let k = (tick - a.tick as f32) / (b.tick as f32 - a.tick as f32).max(1.0);
    Some(series.value(a) + (series.value(b) - series.value(a)) * k)
}

/// Player `i`'s figures as they stand now, as the battle's thirty-second readings take them
/// (none before the battle's first tick).
fn now(game: &Game, i: usize) -> Option<StatSample> {
    let sim = &game.sim;
    let p = sim.players.get(i).filter(|_| sim.tick > 0)?;
    let army = sim
        .units()
        .filter(|u| u.owner as usize == i && !u.dead && u.is_complete())
        .map(|u| sim.unit_type(u))
        .filter(|t| t.locomotion != ta_sim::Locomotion::Static && t.weapons.iter().any(|w| w.is_some()))
        .fold(Fx::ZERO, |a, t| a + t.cost_metal + t.cost_energy / 60);
    Some(StatSample { tick: sim.tick, metal_income: p.economy.metal_income, energy_income: p.economy.energy_income, army_value: army, units: p.unit_count })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(tick: u64, units: u32) -> StatSample {
        StatSample { tick, metal_income: Fx::ZERO, energy_income: Fx::ZERO, army_value: Fx::ZERO, units }
    }

    #[test]
    fn a_reading_between_two_lies_on_the_line() {
        let r = [sample(0, 10), sample(900, 40)];
        assert_eq!(reading_at(&r, 450.0, Series::Units), Some(25.0));
        assert_eq!(reading_at(&r, 900.0, Series::Units), Some(40.0));
        assert_eq!(reading_at(&r, 0.0, Series::Units), Some(10.0));
        assert_eq!(reading_at(&r, 950.0, Series::Units), None);
        assert_eq!(clock(30 * 754), "12:34");
    }

    #[test]
    fn the_scale_steps_round() {
        assert_eq!(rounded_top(36_000.0), 40_000.0);
        assert_eq!(rounded_top(34.0), 40.0);
        assert_eq!(rounded_top(3.2), 4.0);
        assert_eq!(rounded_top(7_000.0), 8_000.0);
        assert_eq!(rounded_top(40_000.0), 40_000.0);
        assert!(rounded_top(0.0) >= 4.0);
    }
}
