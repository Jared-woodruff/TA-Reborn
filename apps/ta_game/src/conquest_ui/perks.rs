//! The Commander's perks: the choice each promotion brings (one of three offered, or later), and
//! the perks held as the pages show them.

use egui::{Color32, RichText, Sense, Stroke, pos2, vec2};
use ta_ui::i18n::{t, tf};
use ta_ui::panel::{self, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use super::{ConquestPage, galaxy, page_width, rank_name};
use crate::conquest::{Conquest, Perk};
use crate::theme::*;

impl ConquestPage {
    /// A promotion's choice, over the galaxy: three perks offered, each a card with CHOOSE, and
    /// LATER (asked again after the next battle, or from the empire's panel).
    pub(super) fn perk_modal(&mut self, ctx: &egui::Context, c: &Conquest) {
        let offer = c.perk_offer();
        // (The rank this choice is for: the first promotion's, then the next.)
        let rank = rank_name((c.perks.len() + 1).min(3) as u8);
        let subtitle = tf("{rank} · CHOOSE A PERK", &[("rank", &rank)]);
        let room = ctx.content_rect();
        let width = page_width(room.width(), 860.0);
        let page = Panel::new("PROMOTION").subtitle(&subtitle).width(width).max_height(room.height() - 40.0).accent(AMBER);
        let response = panel::modal_with_footer(ctx, "conquest_perks", page, true, 38.0, |ui| {
            w::pretty_label(ui, RichText::new(tf("Your Commander has risen to {rank}. Choose a perk it keeps for the rest of the war.", &[("rank", &rank)])).color(TEXT_BRIGHT));
            if c.perks_due() > 1 {
                w::pretty_label(ui, RichText::new(t("Another promotion's choice follows.")).color(STEEL));
            }
            ui.add_space(10.0);
            // Side by side where they fit, else one above the other.
            let across = ui.available_width() >= 600.0;
            let gap = 12.0;
            let card_width = if across { (ui.available_width() - 2.0 * gap) / 3.0 } else { ui.available_width() };
            let mut chosen = None;
            let mut card = |ui: &mut egui::Ui, k: usize, perk: Perk| {
                // (The cards come in one after another as the dialog opens.)
                panel::list_entry(ui, k, |ui| egui::Frame::new().fill(INSET).stroke(Stroke::new(1.0, LINE_HI)).inner_margin(12.0).corner_radius(4).show(ui, |ui| {
                    ui.set_width(card_width - 24.0);
                    ui.vertical_centered(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
                        ui.painter().circle(rect.center(), 25.0, Color32::from_rgba_unmultiplied(8, 10, 14, 220), Stroke::new(1.5, AMBER_DIM));
                        galaxy::paint_perk(ui.painter(), rect.center(), 30.0, perk);
                        ui.add_space(6.0);
                        ui.label(RichText::new(t(perk.label())).font(strong(14.0)).color(AMBER_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
                        ui.add_space(2.0);
                        ui.add(egui::Label::new(RichText::new(t(perk.effect())).color(TEXT)).wrap());
                        ui.add_space(8.0);
                        if ui.add(ActionButton::primary("CHOOSE").size((card_width - 48.0).min(180.0), 34.0)).clicked() {
                            chosen = Some(perk);
                        }
                    });
                }));
            };
            if across {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (k, &perk) in offer.iter().enumerate() {
                        card(ui, k, perk);
                    }
                });
            } else {
                for (k, &perk) in offer.iter().enumerate() {
                    card(ui, k, perk);
                    ui.add_space(8.0);
                }
            }
            if let Some(error) = &self.error {
                ui.add(egui::Label::new(RichText::new(error).color(RED)).wrap());
            }
            chosen
        }, |ui| {
            ui.vertical_centered(|ui| ui.add(ActionButton::secondary("LATER").size(160.0, 36.0)).clicked()).inner
        });
        if let Some(perk) = response.inner.0 {
            match Conquest::choose_perk_at(&Conquest::path(), c, perk) {
                Ok(current) => {
                    self.state = Some(current);
                    self.error = None;
                }
                Err(error) => self.error = Some(tf("Could not save the perk: {error}", &[("error", &error)])),
            }
        }
        if response.inner.1 || response.exit {
            self.perk_later = true;
        }
    }

    /// The Commander in the empire's panel: its rank and experience, its perks, and a choice due.
    pub(super) fn commander_section(&mut self, ui: &mut egui::Ui, c: &Conquest) {
        panel::section(ui, "THE COMMANDER");
        let xp = format!("{:.1}", c.commander_xp.to_f64());
        w::pretty_label(ui, RichText::new(tf("{rank} · {xp} XP", &[("rank", &rank_name(c.commander_rank())), ("xp", &xp)])).color(AMBER));
        if c.perks.is_empty() {
            w::pretty_label(ui, RichText::new(t("Each promotion earns the Commander a perk.")).color(STEEL));
        } else {
            perk_chips(ui, &c.perks);
        }
        if c.perks_due() > 0 && !c.won {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(t("A perk to choose")).color(AMBER_BRIGHT));
                if ui.add(ActionButton::primary("CHOOSE").height(28.0)).clicked() {
                    self.perk_later = false;
                }
            });
        }
    }
}

/// The perks held, as chips (each its glyph and name, what it does on hover): one widget each,
/// so that the row wraps.
pub(super) fn perk_chips(ui: &mut egui::Ui, perks: &[Perk]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(12.0, 6.0);
        for &perk in perks {
            let text = w::galley(ui, RichText::new(t(perk.label())).font(strong(11.0)).color(AMBER_BRIGHT));
            let (rect, response) = ui.allocate_exact_size(vec2(24.0 + text.size().x, text.size().y.max(20.0)), Sense::hover());
            galaxy::paint_perk(ui.painter(), pos2(rect.min.x + 10.0, rect.center().y), 16.0, perk);
            ui.painter().galley(pos2(rect.min.x + 24.0, rect.center().y - text.size().y / 2.0), text, AMBER_BRIGHT);
            response.on_hover_text(t(perk.effect()));
        }
    });
}
