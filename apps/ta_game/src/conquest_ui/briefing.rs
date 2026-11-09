//! A battle's briefing, between ATTACK (or DEFEND) and the battle: the battlefield with where we
//! land and where the enemy holds (or our base and where the attackers come from), what wins it
//! (and what loses it), the enemy's strength, what the system brings, the force going in and
//! the supply it starts with. LAUNCH BATTLE (or Enter) starts it; Escape, a click outside or
//! BACK returns to the galaxy.

use egui::{Key, Modifiers, RichText, Sense, pos2, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use super::{ConquestAction, ConquestPage, enemy_line, galaxy, page_width};
use crate::conquest::Conquest;
use crate::palette::team_color;
use crate::setup::Assets;
use crate::theme::*;

impl ConquestPage {
    /// The briefing for system `i`: the battle's setup when LAUNCH BATTLE is chosen.
    pub(super) fn briefing_modal(&mut self, ctx: &egui::Context, assets: &Assets, portraits: &mut crate::encyclopaedia::Portraits, c: &Conquest, i: usize) -> Option<ConquestAction> {
        let s = &c.systems[i];
        let defence = c.defendable(i);
        let title = if defence { tf("DEFENCE OF {name}", &[("name", &s.name.to_uppercase())]) } else { tf("BATTLE FOR {name}", &[("name", &s.name.to_uppercase())]) };
        // (A defence's attackers are as strong as the system they strike from.)
        let threat = c.counter.filter(|_| defence).map_or(s.threat, |counter| c.systems[counter.from].threat);
        // (A system with no world named: no separator left hanging at the end.)
        let subtitle = if s.planet.trim().is_empty() {
            tf("CONQUEST · BATTLE {n}", &[("n", &(c.battles + 1))])
        } else {
            tf("CONQUEST · BATTLE {n} · {world}", &[("n", &(c.battles + 1)), ("world", &s.planet.to_uppercase())])
        };
        let room = ctx.content_rect();
        let width = page_width(room.width(), 900.0);
        let setup = c.setup(i, &assets.maps);
        let ready = setup.is_some() && self.pending_deployment.is_none();
        let page = Panel::new(&title).subtitle(&subtitle).width(width).max_height(room.height() - 40.0).exit(Exit::Close).accent(AMBER);
        let response = panel::modal_with_footer(ctx, "conquest_briefing", page, true, 54.0, |ui| {
            // The picture beside the orders where they fit, as tall as the room leaves (a short
            // window shows a smaller picture rather than one to scroll past).
            let wide = ui.available_width() >= 520.0;
            let tall = (ui.available_height() - 64.0).max(180.0);
            let picture_side = if wide { (ui.available_width() * 0.46).min(400.0).min(tall) } else { ui.available_width().min(tall) };
            let battlefield = |ui: &mut egui::Ui| {
                w::card_title(ui, "THE BATTLEFIELD");
                match crate::map_preview::picture(ui.ctx(), assets, &s.map, picture_side) {
                    Some(picture) => {
                        let tall = (picture_side - 10.0) / picture.aspect();
                        let (frame, _) = ui.allocate_exact_size(vec2(picture_side, tall.min(picture_side) + 10.0), Sense::hover());
                        let placed = crate::map_preview::show(ui.painter(), &picture, frame);
                        // Our landing zone and the enemy's bases, as the battle places them.
                        let (enemies, _) = c.enemies(i);
                        if let Some(m) = assets.maps.iter().find(|m| m.name == s.map) {
                            let enemies = enemies.min(m.start_positions.saturating_sub(1)).max(1);
                            let starts = m.info.schemas.first().map(|sc| sc.start_positions.clone()).unwrap_or_default();
                            for (k, &(_, x, z)) in starts.iter().enumerate().take(1 + enemies) {
                                let fill = if k == 0 { team_color(0) } else { team_color(1) };
                                crate::map_preview::start_marker(ui.painter(), placed.marker_at(x, z, 12.0), fill, &format!("{}", k + 1));
                            }
                        }
                    }
                    None => {
                        let (frame, _) = ui.allocate_exact_size(vec2(picture_side, picture_side * 0.6), Sense::hover());
                        ui.painter().add(ta_ui::metal::well(frame, INSET));
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let legend = if defence { [(team_color(0), t("YOUR BASE")), (team_color(1), t("THE ATTACKERS"))] } else { [(team_color(0), t("YOUR LANDING ZONE")), (team_color(1), t("THE ENEMY'S BASES"))] };
                    for (fill, label) in legend {
                        let (dot, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 5.0, fill);
                        ui.label(RichText::new(label).font(strong(10.5)).color(STEEL));
                    }
                    if let Some(m) = assets.maps.iter().find(|m| m.name == s.map) {
                        w::tag(ui, &m.size, STEEL);
                    }
                });
            };
            let orders = |page: &mut ConquestPage, ui: &mut egui::Ui, portraits: &mut crate::encyclopaedia::Portraits| {
                panel::section(ui, "OBJECTIVE");
                let (enemies, _) = c.enemies(i);
                let capital = i == c.capital;
                let objective = match (defence, capital, enemies > 1) {
                    (true, ..) => t("Destroy the attacking Commander to hold the system."),
                    (_, true, true) => t("Destroy both enemy Commanders to take their capital and win the campaign."),
                    (_, true, false) => t("Destroy the enemy Commander to take their capital and win the campaign."),
                    _ => t("Destroy the enemy Commander to take the system."),
                };
                w::pretty_label(ui, RichText::new(objective).color(TEXT_BRIGHT));
                let warning = if defence { t("If your Commander falls the system falls with it, and the veterans who went in.") } else { t("If your Commander falls the battle is lost, and the veterans who went in with it.") };
                w::pretty_label(ui, RichText::new(warning).color(STEEL));
                panel::section(ui, "THE ENEMY");
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(46.0, 14.0), Sense::hover());
                    galaxy::paint_threat(ui.painter(), pos2(rect.min.x + 2.0, rect.center().y), threat, 11.0);
                    w::pretty_label(ui, RichText::new(enemy_line(c, i, Some(&assets.maps))).color(TEXT));
                });
                if let Some(counter) = c.counter.filter(|_| defence) {
                    w::pretty_label(ui, RichText::new(tf("They strike from {from}.", &[("from", &c.systems[counter.from].name)])).color(STEEL));
                }
                // What the system brings: won by taking it, lost with it.
                if c.reward(i) != crate::conquest::Reward::None {
                    panel::section(ui, if defence { n("AT STAKE") } else { n("TAKING IT BRINGS") });
                    super::panels::reward_row(ui, assets, c, c.reward(i));
                }
                panel::section(ui, "YOUR FORCE");
                page.force_row(ui, assets, portraits, c, 40.0);
                if !c.perks.is_empty() {
                    super::perks::perk_chips(ui, &c.perks);
                }
                let (metal, energy) = c.bonus();
                w::pretty_label(ui, RichText::new(tf("+{metal} metal and +{energy} energy at the start", &[("metal", &metal), ("energy", &energy)])).color(AMBER));
                if ui.add(ActionButton::secondary("MANAGE VETERANS").height(28.0)).clicked() {
                    page.briefing = None;
                    page.selected = None;
                    page.empire_tab = super::EmpireTab::Veterans;
                }
            };
            if wide {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 24.0;
                    ui.allocate_ui_with_layout(vec2(picture_side, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| battlefield(ui));
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        orders(self, ui, portraits);
                    });
                });
            } else {
                ui.vertical_centered(|ui| {
                    ui.allocate_ui_with_layout(vec2(picture_side, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| battlefield(ui));
                });
                ui.add_space(8.0);
                orders(self, ui, portraits);
            }
            if setup.is_none() {
                ui.add_space(8.0);
                ui.add(egui::Label::new(RichText::new(t("This battlefield is unavailable. Enable its map pack, then return to Conquest.")).color(AMBER)).wrap());
            }
        }, |ui| {
            // LAUNCH BATTLE (or Enter) and BACK, in sight however the briefing scrolls.
            let mut launch = None;
            ui.horizontal(|ui| {
                let back = (ui.available_width() * 0.26).clamp(110.0, 140.0);
                let launch_width = (ui.available_width() - back - ui.spacing().item_spacing.x).max(180.0);
                if panel::hero_button(ui, "LAUNCH BATTLE", vec2(launch_width, 54.0), ready).clicked() {
                    launch = Some(true);
                }
                if ui.add(ActionButton::secondary("BACK").size(back, 54.0)).clicked() {
                    launch = Some(false);
                }
            });
            if ready && ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::Enter)) {
                launch = Some(true);
            }
            launch
        });
        let launch = response.inner.1;
        if response.exit || launch == Some(false) {
            self.briefing = None;
        }
        if launch == Some(true) && let Some(setup) = setup {
            self.briefing = None;
            return Some(ConquestAction::Start(setup));
        }
        None
    }
}
