//! A scenario's goals while it is played: what wins, what loses, and how each stands (clocks,
//! the hold of the hill, metal made, the waves); and a skirmish's ceasefire (no rush) while it
//! lasts.

use egui::{Align2, Color32, CornerRadius, RichText, Vec2 as EVec2};
use ta_ui::i18n::{t, tf};

use super::{Clearance, Hud};
use crate::game::Game;
use crate::scenario::{GoalView, Kind};
use crate::theme::*;

/// Width of the goals' text column, and of their bars.
const WIDTH: f32 = 250.0;

fn clock(ticks: u64) -> String {
    let s = ticks / 30;
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn thousands(v: f32) -> String {
    let n = v.max(0.0) as u64;
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Hud {
    /// The ceasefire a skirmish opens with (its mutators' no rush): what it holds back, and
    /// how long it has to run.
    pub(super) fn ceasefire_panel(&mut self, ctx: &egui::Context, game: &Game, clear: Clearance) {
        let Some(left) = game.sim.ceasefire_left() else { return };
        let whole = game.sim.mutators.ceasefire_ticks().max(1);
        // (Under a scenario's goals, when it has them up there.)
        let top = ctx.memory(|m| m.area_rect(egui::Id::new("scenario_goals"))).filter(|_| game.scenario.is_some()).map_or(clear.top + 10.0, |r| r.max.y + 10.0);
        ta_ui::panel::strip(ctx, "ceasefire", Align2::LEFT_TOP, EVec2::new(clear.left + 12.0, top), |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new(t("CEASEFIRE")).font(strong(13.0)).color(AMBER_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // (The last ten seconds stand out.)
                    let soon = left < 10 * 30;
                    ui.label(RichText::new(clock(left)).font(mono(14.0)).color(if soon { AMBER_BRIGHT } else { TEXT_BRIGHT }));
                });
            });
            ui.label(RichText::new(t("Weapons do enemies no harm, and nobody builds near an enemy base.")).font(body(12.5)).color(TEXT));
            let (rect, _) = ui.allocate_exact_size(EVec2::new(WIDTH, 5.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(1), INSET);
            let mut fill = rect;
            fill.set_width(rect.width() * (left as f32 / whole as f32).clamp(0.0, 1.0));
            painter.rect_filled(fill, CornerRadius::same(1), AMBER.gamma_multiply(0.85));
        });
    }

    pub(super) fn scenario_panel(&mut self, ctx: &egui::Context, game: &Game, clear: Clearance) {
        let Some(run) = &game.scenario else { return };
        if game.spectator && game.playback.is_none() {
            return;
        }
        let Some(rules) = &game.sim.mission else { return };
        let now = game.sim.tick;
        let difficulty = match run.difficulty {
            ta_ai::Difficulty::Easy => t("EASY"),
            ta_ai::Difficulty::Normal => t("NORMAL"),
            ta_ai::Difficulty::Hard => t("HARD"),
        };
        ta_ui::panel::strip(ctx, "scenario_goals", Align2::LEFT_TOP, EVec2::new(clear.left + 12.0, clear.top + 10.0), |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new(ta_ui::i18n::upper(ta_ui::i18n::t(&run.title))).font(strong(13.0)).color(AMBER_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
                ui.label(RichText::new(difficulty).font(strong(11.0)).color(STEEL));
            });
            let me = game.local_player as usize;
            let mine = game.sim.players.get(me).map_or(0.0, |p| p.stats.metal_produced.to_f32());
            let theirs = game.sim.players.iter().filter(|p| game.sim.is_enemy(p.index, game.local_player)).map(|p| p.stats.metal_produced.to_f32()).fold(0.0, f32::max);
            for ((goal, winning), objective) in run.victory.iter().map(|g| (g, true)).chain(run.defeat.iter().map(|g| (g, false)))
                .zip(rules.victory.iter().chain(&rules.defeat))
            {
                let color = if winning { GREEN } else { RED };
                let held = rules.hold_time(objective);
                let (text, value, bar): (String, String, Option<f32>) = match goal {
                    GoalView::Survive { ticks } => (t("Survive").into(), clock(ticks.saturating_sub(now)), None),
                    GoalView::Time { ticks } => (t("Time left").into(), clock(ticks.saturating_sub(now)), None),
                    GoalView::Hold { ticks } => (t("Hold the marked ground").into(), format!("{} / {}", clock(held), clock(*ticks)), Some(held as f32 / *ticks.max(&1) as f32)),
                    GoalView::EnemyHolds { ticks } => (t("The enemy's hold").into(), format!("{} / {}", clock(held), clock(*ticks)), Some(held as f32 / *ticks.max(&1) as f32)),
                    GoalView::Reach { what } => (tf("Get {what} to the marked point", &[("what", what)]), String::new(), None),
                    GoalView::Destroy { what } => {
                        // The last few, once they are marked.
                        let left = if game.last_enemies.is_empty() { String::new() } else { tf("{n} left", &[("n", &game.last_enemies.len())]) };
                        (tf("Destroy {what}", &[("what", what)]), left, None)
                    }
                    // (As the scenarios page words it: "Destroy every Krogoth", not "Destroy
                    // Krogoth".)
                    GoalView::DestroyEvery { unit } => (tf("Destroy every {unit}", &[("unit", unit)]), String::new(), None),
                    GoalView::Lose { what } => (tf("Don't lose {what}", &[("what", what)]), String::new(), None),
                    GoalView::Metal { metal } => (t("Metal made").into(), format!("{} / {}", thousands(mine), thousands(*metal)), Some(mine / metal.max(1.0))),
                    GoalView::EnemyMetal { metal } => (t("The enemy's metal").into(), format!("{} / {}", thousands(theirs), thousands(*metal)), Some(theirs / metal.max(1.0))),
                };
                ui.horizontal_top(|ui| {
                    let (dot, _) = ui.allocate_exact_size(EVec2::new(8.0, 14.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.0, color);
                    ui.vertical(|ui| {
                        ui.set_width(WIDTH - 16.0);
                        ui.add(egui::Label::new(RichText::new(text).font(body(13.0)).color(TEXT_BRIGHT)).wrap());
                        if !value.is_empty() {
                            // A clock running out goes red in its last minute.
                            let urgent = matches!(goal, GoalView::Time { ticks } if ticks.saturating_sub(now) < 60 * 30);
                            ui.add(egui::Label::new(RichText::new(value).font(mono(13.0)).color(if urgent { RED } else { TEXT })).wrap());
                        }
                    });
                });
                if let Some(f) = bar {
                    let (rect, _) = ui.allocate_exact_size(EVec2::new(WIDTH, 5.0), egui::Sense::hover());
                    let painter = ui.painter();
                    painter.rect_filled(rect, CornerRadius::same(1), INSET);
                    let mut fill = rect;
                    fill.set_width(rect.width() * f.clamp(0.0, 1.0));
                    painter.rect_filled(fill, CornerRadius::same(1), color.gamma_multiply(0.85));
                }
            }
            // The waves: how many have come, and when the next one does.
            if run.waves > 0 {
                let sent = rules.waves_sent;
                let text = if run.kind == Kind::Endless { tf("Wave {n}", &[("n", &sent)]) } else { tf("Wave {n} of {total}", &[("n", &sent), ("total", &run.waves)]) };
                let next = rules.next_wave().map(|w| w.tick.saturating_sub(now));
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(EVec2::new(8.0, 14.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.0, AMBER);
                    ui.label(RichText::new(text).font(body(13.0)).color(TEXT_BRIGHT));
                    if let Some(next) = next {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let soon = next < 10 * 30;
                            ui.label(RichText::new(tf("next {clock}", &[("clock", &clock(next))])).font(mono(12.5)).color(if soon { AMBER_BRIGHT } else { STEEL }));
                        });
                    }
                });
            }
            if run.kind == Kind::Endless {
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(EVec2::new(8.0, 14.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.0, Color32::from_rgb(120, 180, 230));
                    ui.label(RichText::new(t("Held out for")).font(body(13.0)).color(TEXT_BRIGHT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(clock(now)).font(mono(13.0)).color(TEXT));
                    });
                });
            }
        });
    }
}
