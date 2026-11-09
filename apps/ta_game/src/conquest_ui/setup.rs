//! A new conquest: the side, the galaxy's size and the opposition at the left, START CONQUEST
//! always in sight under them; the galaxy they make drawn large at the right, named and laid
//! out as it will be played (NEW GALAXY draws another), how a conquest goes under it. On narrow
//! screens the choices scroll, then the galaxy and how it goes, START CONQUEST kept at the foot
//! of the screen.

use egui::{Align2, Color32, Rect, RichText, Sense, Vec2, pos2, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Panel};
use ta_ui::widgets::{self as w};

use super::{ConquestAction, ConquestPage, animation_time, galaxy, metal_steel, new_seed, panel_chrome};
use crate::conquest::{Conquest, Level, Size};
use crate::palette::team_color;
use crate::setup::Assets;
use crate::theme::*;

/// The side cards' height, and on a short screen.
const CARD: f32 = 132.0;
const SHORT_CARD: f32 = 116.0;

/// How a conquest goes, for the player choosing one.
const RULES: &str = n("Every system is a battlefield held by the enemy. Attack the systems next to yours; the nearer their capital, the harder they fight, and only those near yours are charted. Units that earn a rank come back as veterans and follow your Commander into the next battle. Every system you hold brings more metal and energy to the start of each fight, and a reward of its own: mines, reactors, a tank plant, an airfield, a war college, a sensor array or a supply depot. The enemy strikes back at what you take. Take the enemy capital to win.");

impl ConquestPage {
    pub(super) fn new_conquest(&mut self, ui: &mut egui::Ui, assets: &Assets) -> ConquestAction {
        let mut action = ConquestAction::None;
        let ctx = ui.ctx().clone();
        let screen = ui.max_rect();
        let time = animation_time(&ctx);
        ta_ui::motion::next_frame(&ctx, std::time::Duration::from_millis(33));
        galaxy::paint_space(ui.painter(), screen, time);
        let painter = ui.painter().clone();
        let compact = screen.width() < 1000.0 || screen.height() < 620.0;
        let title = panel::shadowed_title(&painter, pos2(screen.min.x + if compact { 20.0 } else { 64.0 }, screen.min.y + if compact { 16.0 } else { 34.0 }), "CONQUEST", if compact { 34.0 } else { 44.0 }, TEXT_BRIGHT);
        painter.text(pos2(title.min.x + 2.0, title.max.y + 4.0), Align2::LEFT_TOP, t("TAKE THE GALAXY SYSTEM BY SYSTEM"), strong(12.5), AMBER);
        egui::Area::new(egui::Id::new("conquest_new_back")).anchor(Align2::RIGHT_TOP, [if compact { -16.0 } else { -32.0 }, if compact { 16.0 } else { 34.0 }]).show(&ctx, |ui| {
            if panel::back_button(ui, "BACK").clicked() {
                action = ConquestAction::Back;
            }
        });
        let top = title.max.y + 40.0;
        let galaxy = self.new_galaxy(assets).cloned();
        let start_height = 58.0;
        if compact {
            // The choices and the galaxy scroll; START CONQUEST stays at the foot.
            let foot = screen.max.y - start_height - 24.0;
            let body = Rect::from_min_max(pos2(screen.min.x + 16.0, top), pos2(screen.max.x - 16.0, foot - 10.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                let area = egui::ScrollArea::vertical().id_salt("conquest_new_scroll").auto_shrink([false, false]).max_height(body.height());
                ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Picture, |ui| {
                    ui.set_width(ui.available_width());
                    self.new_choices(ui, assets, SHORT_CARD);
                    // (The galaxy before how a conquest goes: under that, it was far out of sight
                    // with START CONQUEST at hand.)
                    ui.add_space(12.0);
                    let width = ui.available_width();
                    let (well, _) = ui.allocate_exact_size(vec2(width, (width * 0.55).clamp(220.0, 340.0)), Sense::hover());
                    self.new_preview(ui, galaxy.as_ref(), well, time);
                    ui.add_space(6.0);
                    self.galaxy_line(ui, galaxy.as_ref());
                    ui.add_space(14.0);
                    w::card_title(ui, "HOW CONQUEST WORKS");
                    w::pretty_label(ui, RichText::new(t(RULES)).color(TEXT));
                });
            });
            let start = Rect::from_min_size(pos2(screen.min.x + 16.0, foot), vec2(screen.width() - 32.0, start_height));
            egui::Area::new(egui::Id::new("conquest_new_start")).fixed_pos(start.min).show(&ctx, |ui| {
                self.start_button(ui, assets, galaxy.as_ref(), start.size());
            });
        } else {
            let column = (screen.width() * 0.3).clamp(400.0, 480.0);
            // (On a short screen, a 1080p one at 150% among them, the side cards are lower and
            // the foot's margin narrower: OPPOSITION fell below the panel's foot.)
            let short = screen.height() < 800.0;
            let card = if short { SHORT_CARD } else { CARD };
            let left = Rect::from_min_max(pos2(screen.min.x + 64.0, top), pos2(screen.min.x + 64.0 + column, screen.max.y - if short { 28.0 } else { 48.0 }));
            let choices_height = left.height() - start_height - 18.0;
            let choices = egui::Area::new(egui::Id::new("conquest_new_choices")).fixed_pos(left.min).show(&ctx, |ui| {
                Panel::new("NEW CONQUEST").subtitle("CHOOSE YOUR WAR").width(column - panel_chrome()).max_height(choices_height).show(ui, |ui| {
                    self.new_choices(ui, assets, card);
                });
            });
            // (Under the choices, however tall they come out.)
            let start_top = (choices.response.rect.max.y + 16.0).min(left.max.y - start_height);
            let start = Rect::from_min_size(pos2(left.min.x, start_top), vec2(column, start_height));
            egui::Area::new(egui::Id::new("conquest_new_start")).fixed_pos(start.min).show(&ctx, |ui| {
                self.start_button(ui, assets, galaxy.as_ref(), start.size());
            });
            // The galaxy, as large as the rest of the screen allows, and under it how the war goes
            // (in the choices' panel it pushed them below its foot on a 1080p screen).
            let width = screen.max.x - 64.0 - (left.max.x + 36.0);
            let rules = painter.layout(t(RULES).to_string(), body(13.0), TEXT, width.clamp(1.0, 900.0));
            let right = Rect::from_min_max(pos2(left.max.x + 36.0, top), pos2(screen.max.x - 64.0, screen.max.y - 130.0 - rules.size().y));
            // (Headed as the panel's sections are, a rivet before cream capitals.)
            w::card_title_at(ui, ui.painter(), pos2(right.min.x, right.min.y - 28.0), ta_ui::i18n::n("YOUR GALAXY"));
            self.new_preview(ui, galaxy.as_ref(), right, time);
            egui::Area::new(egui::Id::new("conquest_new_line")).fixed_pos(pos2(right.min.x, right.max.y + 14.0)).show(&ctx, |ui| {
                ui.set_max_width(right.width());
                self.galaxy_line(ui, galaxy.as_ref());
            });
            let at = pos2(right.min.x, right.max.y + 58.0);
            let heading = w::card_title_at(ui, &painter, at, ta_ui::i18n::n("HOW CONQUEST WORKS"));
            painter.galley(pos2(at.x, heading.max.y + 6.0), rules, TEXT);
        }
        if let Some(error) = &self.error {
            let error = error.clone();
            egui::Area::new(egui::Id::new("conquest_new_error")).anchor(Align2::CENTER_BOTTOM, [0.0, -100.0]).order(egui::Order::Foreground).show(&ctx, |ui| {
                ui.add(egui::Label::new(RichText::new(error).color(RED)).wrap());
            });
        }
        action
    }

    /// The side (its cards `card` high), the galaxy's size and the opposition.
    fn new_choices(&mut self, ui: &mut egui::Ui, assets: &Assets, card: f32) {
        ui.label(w::caption("SIDE"));
        let card_width = ((ui.available_width() - 16.0) / 2.0).min(170.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            for side in ["ARM", "CORE"] {
                if self.side_card(ui, assets, side, vec2(card_width, card)).clicked() {
                    self.side = side.to_string();
                }
            }
        });
        ui.add_space(14.0);
        // (The systems' count beside the sizes, not on a line of its own: on a 1080p screen at
        // 150% the line pushed OPPOSITION below the panel's foot.)
        panel::setting(ui, "GALAXY", None, |ui| {
            ui.horizontal_wrapped(|ui| {
                let sizes: Vec<(Size, &str)> = Size::ALL.iter().map(|&s| (s, s.label())).collect();
                w::segmented(ui, &mut self.size, &sizes);
                ui.add_space(4.0);
                ui.label(RichText::new(tf("{n} systems", &[("n", &self.size.systems())])).font(body(12.5)).color(STEEL_DIM));
            });
        });
        let hint = match self.level {
            Level::Relaxed => ta_ui::i18n::n("Easy computer players, Normal at the capital; the enemy never strikes back"),
            Level::Standard => ta_ui::i18n::n("Easy near home, Normal in the middle, Hard at the capital; the enemy sometimes strikes back"),
            Level::Hard => ta_ui::i18n::n("Normal near home, Hard everywhere else; the enemy often strikes back"),
        };
        panel::setting(ui, "OPPOSITION", Some(hint), |ui| {
            let levels: Vec<(Level, &str)> = Level::ALL.iter().map(|&l| (l, l.label())).collect();
            w::segmented(ui, &mut self.level, &levels);
        });
    }

    /// The galaxy the choices make, drawn in `well` (named, as it will be played).
    fn new_preview(&mut self, ui: &mut egui::Ui, galaxy: Option<&Conquest>, well: Rect, time: f32) {
        let painter = ui.painter_at(well);
        galaxy::paint_space(&painter, well, time);
        match galaxy {
            Some(c) => {
                let bounds = well.shrink(6.0);
                let k = galaxy::scale_for(bounds.shrink2(vec2(60.0, 40.0)));
                let view = galaxy::View { area: bounds.shrink2(vec2(56.0 * k, 38.0 * k)), bounds, k, time };
                let mut shown = ui.new_child(egui::UiBuilder::new().max_rect(well));
                self.draw_galaxy(&mut shown, c, view, false);
            }
            None => {
                let text = painter.layout(t("Not enough skirmish maps for a galaxy (four are needed).").to_string(), body(13.0), STEEL, well.width() - 32.0);
                painter.galley(well.center() - text.size() / 2.0, text, STEEL);
            }
        }
        galaxy::frame(ui.painter(), well);
    }

    /// Under the galaxy: its size, how far the capital is, and NEW GALAXY.
    fn galaxy_line(&mut self, ui: &mut egui::Ui, galaxy: Option<&Conquest>) {
        ui.horizontal_wrapped(|ui| {
            if let Some(c) = galaxy {
                ui.label(RichText::new(tf("{n} systems · the enemy capital lies {jumps} jumps from your home", &[("n", &c.systems.len()), ("jumps", &c.capital_jumps())])).font(body(13.0)).color(STEEL));
            }
            if ui.add(w::ActionButton::secondary("NEW GALAXY")).clicked() {
                self.seed = new_seed().wrapping_add(self.seed.wrapping_mul(6364136223846793005));
            }
        });
    }

    fn start_button(&mut self, ui: &mut egui::Ui, assets: &Assets, galaxy: Option<&Conquest>, size: Vec2) {
        if panel::hero_button(ui, "START CONQUEST", size, galaxy.is_some()).clicked()
            && let Some(c) = galaxy
        {
            let c = Conquest::new(&assets.selectable_maps, &self.side, self.level, self.size, self.seed).unwrap_or_else(|| c.clone());
            self.start_at(c, &Conquest::path());
        }
    }

    /// A side's card: its emblem and name, lit in our colour when it is ours.
    fn side_card(&mut self, ui: &mut egui::Ui, assets: &Assets, side: &str, size: Vec2) -> egui::Response {
        let ours = self.side.eq_ignore_ascii_case(side);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, ours, side));
        if response.clicked() && !ours {
            ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle);
        }
        let hot = ui.ctx().animate_bool_with_time(response.id.with("hot"), response.hovered(), 0.12);
        let lit = ui.ctx().animate_bool_with_time(response.id.with("ours"), ours, 0.18);
        let colour = team_color(0);
        let painter = ui.painter();
        painter.add(ta_ui::metal::well(rect, INSET.lerp_to_gamma(Color32::from_rgb(30, 30, 26), hot * 0.5)));
        // Ours: a soft glow of our colour behind the emblem, a band of it along the foot.
        if lit > 0.0 {
            let glow = rect.center() - vec2(0.0, 12.0);
            for k in 0..16 {
                let f = 1.0 - k as f32 / 16.0;
                painter.circle_filled(glow, rect.height() * 0.46 * f, colour.gamma_multiply(0.022 * lit));
            }
            let band = Rect::from_min_max(pos2(rect.min.x + 1.0, rect.max.y - 4.0), pos2(rect.max.x - 1.0, rect.max.y - 1.0));
            painter.add(ta_ui::metal::vgrad(band, colour.gamma_multiply(lit), colour.lerp_to_gamma(Color32::BLACK, 0.4).gamma_multiply(lit)));
        }
        // (Clear of the name under it on a lower card.)
        let size_pt = (rect.height() * 0.5).min(rect.height() - 64.0).round();
        let at = Rect::from_center_size(rect.center() - vec2(0.0, 12.0), vec2(size_pt, size_pt));
        let body = metal_steel().lerp_to_gamma(colour, lit * 0.85).lerp_to_gamma(Color32::WHITE, hot * 0.1);
        self.paint_emblem(ui, assets, side, at, body, lit);
        let painter = ui.painter();
        let name = painter.layout_no_wrap(side.to_uppercase(), display(22.0), Color32::WHITE);
        let name_at = pos2(rect.center().x - name.size().x / 2.0, rect.max.y - 44.0);
        ta_ui::metal::embossed(painter, name_at, name, if ours { CREAM } else { STEEL_DIM.lerp_to_gamma(STEEL, hot) });
        let (word, tone) = if ours { (t("YOUR SIDE"), AMBER) } else { (t("THE ENEMY"), Color32::from_rgb(170, 82, 66)) };
        painter.text(pos2(rect.center().x, rect.max.y - 15.0), Align2::CENTER_CENTER, word, strong(10.5), tone);
        w::focus_ring(ui, &response, rect);
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}
