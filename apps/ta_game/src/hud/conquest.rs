//! A Conquest battle's goals while it is fought: the system at stake, the enemy holding it,
//! what wins and what loses, and the veterans who came along.

use egui::{Align2, Color32, RichText, Vec2 as EVec2};
use ta_ui::i18n::{t, tf};

use super::{Clearance, Hud};
use crate::game::Game;
use crate::setup::Controller;
use crate::theme::*;

/// Width of the goals' text column.
const WIDTH: f32 = 250.0;

/// A line of the strip: its dot, its text, and a value at the right.
fn line(ui: &mut egui::Ui, dot: Color32, text: String, value: Option<String>) -> egui::Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(EVec2::new(8.0, 14.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.0, dot);
        ui.label(RichText::new(text).font(body(13.0)).color(TEXT_BRIGHT));
        if let Some(value) = value {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(value).font(mono(13.0)).color(TEXT));
            });
        }
    })
    .response
}

impl Hud {
    pub(super) fn conquest_panel(&mut self, ctx: &egui::Context, game: &Game, clear: Clearance) {
        let Some(cs) = &game.setup.conquest else { return };
        if game.spectator && game.playback.is_none() {
            return;
        }
        let me = game.local_player as usize;
        let team = game.setup.players.get(me).map_or(0, |s| s.team);
        let enemies: Vec<ta_ai::Difficulty> = game.setup.players.iter().enumerate()
            .filter(|&(p, s)| p != me && s.team != team)
            .filter_map(|(_, s)| match s.controller { Controller::Ai(d) => Some(d), Controller::Human => None })
            .collect();
        let rules = &game.setup.rules;
        let stayed = game.undeployed_conquest_veterans.len().min(cs.veterans.len());
        // The ranked units of ours standing: what a victory brings home (as `conquest::result_of`
        // counts them).
        let sim = &game.sim;
        let standing = sim.units()
            .filter(|u| u.owner == game.local_player && !u.dead && u.is_complete() && u.rank() >= 1)
            .filter(|u| {
                let t = sim.unit_type(u);
                !t.def.is_commander && t.locomotion != ta_sim::Locomotion::Static && t.max_speed > ta_sim::Fx::ZERO
            })
            .count();
        ta_ui::panel::strip(ctx, "conquest_goals", Align2::LEFT_TOP, EVec2::new(clear.left + 12.0, clear.top + 10.0), |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 3.0;
            let name = ta_ui::i18n::upper(&game.map_title);
            let title = if cs.defence { tf("DEFENCE OF {name}", &[("name", &name)]) } else { tf("BATTLE FOR {name}", &[("name", &name)]) };
            ui.label(RichText::new(title).font(strong(13.0)).color(if cs.defence { RED } else { AMBER_BRIGHT }).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
            // (Every enemy of a Conquest battle plays as well as the others.)
            let strength = enemies.first().map_or_else(String::new, |&d| crate::conquest_ui::strength_text(enemies.len(), d));
            ui.label(RichText::new(tf("CONQUEST · BATTLE {n}", &[("n", &(cs.battle + 1))])).font(strong(11.0)).color(STEEL));
            if !strength.is_empty() {
                ui.label(RichText::new(strength).font(body(12.5)).color(TEXT));
            }
            ui.add_space(2.0);
            if rules.commander_ends_game {
                let destroy = match (cs.defence, enemies.len() > 1) {
                    (true, _) => t("Destroy the attacking Commander"),
                    (false, true) => t("Destroy the enemy Commanders"),
                    (false, false) => t("Destroy the enemy Commander"),
                };
                line(ui, GREEN, destroy.into(), None);
                line(ui, RED, t("Don't lose your Commander").into(), None);
            } else {
                line(ui, GREEN, t("Destroy every enemy unit").into(), None);
            }
            // The Commander's perks, their names and what they do on hover.
            let perks: Vec<crate::conquest::Perk> = cs.perks.iter().filter_map(|id| crate::conquest::Perk::from_id(id)).collect();
            if !perks.is_empty() {
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(EVec2::new(8.0, 14.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.0, AMBER);
                    ui.label(RichText::new(t("Commander's perks")).font(body(13.0)).color(TEXT_BRIGHT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for &perk in perks.iter().rev() {
                            let (rect, response) = ui.allocate_exact_size(EVec2::new(18.0, 18.0), egui::Sense::hover());
                            crate::conquest_ui::paint_perk(ui.painter(), rect.center(), 15.0, perk);
                            // (Its name over what it does: no punctuation between them to translate.)
                            response.on_hover_ui(|ui| {
                                ui.label(RichText::new(t(perk.label())).font(strong(12.0)).color(AMBER_BRIGHT));
                                ui.label(RichText::new(t(perk.effect())).color(TEXT));
                            });
                        }
                    });
                });
            }
            // The veterans standing, those who came along and those ranked since (and any the
            // world had no room for).
            line(ui, Color32::from_rgb(150, 190, 240), t("Veterans standing").into(), Some(standing.to_string()))
                .on_hover_text(t("Ranked units still standing when the battle is won join your veterans."));
            if stayed > 0 {
                let text = if stayed > 1 { tf("{n} veterans could not land here and wait at home", &[("n", &stayed)]) } else { t("A veteran could not land here and waits at home").into() };
                ui.label(RichText::new(text).font(body(12.0)).color(STEEL));
            }
        });
    }
}
