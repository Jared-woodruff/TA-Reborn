//! The dialog that imports Supreme Commander: Forged Alliance's maps (`crate::supcom`): where the
//! game is, its maps (and the player's own) with the game's pictures of them to choose from, and
//! the import under way, map by map.

use egui::{Color32, CornerRadius, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{Exit, Panel};
use ta_ui::tooltip::TooltipExt as _;

use super::path_label;
use crate::app::{Action, LauncherApp};
use crate::supcom::{self, State};
use crate::theme::*;
use crate::widgets::{self, ActionButton};

/// A map's picture in the list (pixels).
const THUMB: f32 = 56.0;
/// The narrowest a map's card is; as many as fit stand in a row.
const CARD: f32 = 250.0;
const GAP: f32 = 8.0;

impl LauncherApp {
    /// Why the chosen maps cannot be imported now, if they cannot.
    pub(crate) fn supcom_blocked(&self) -> Option<&'static str> {
        if self.detection.install().is_none() {
            Some(t("Find your Total Annihilation install on the Play page first."))
        } else if self.game_running() {
            Some(t("Close the game first: maps cannot be imported while it runs."))
        } else if self.installer.busy() || self.maps.importing() {
            Some(t("Wait for the mod or the maps being installed, then import these."))
        } else {
            None
        }
    }

    pub(crate) fn supcom_dialog(&mut self, ctx: &egui::Context) {
        if !self.supcom.open {
            return;
        }
        let width = (ctx.content_rect().width() - 80.0).clamp(1.0, 880.0);
        let height = (ctx.content_rect().height() - 24.0).max(1.0);
        let title = t("SUPREME COMMANDER MAPS").to_string();
        let blocked = self.supcom_blocked();
        let mut actions = Vec::new();
        let importing = self.supcom.importing();
        let response = ta_ui::panel::modal(ctx, "launcher_supcom", Panel::new(&title).exit(Exit::Close).width(width).max_height(height), true, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add(egui::Label::new(RichText::new(t("Supreme Commander: Forged Alliance's maps, made into TA Reborn maps from your own copy of the game to play in skirmish: their land and ground, forests and rocks, mass points and starting places come with them.")).color(TEXT)).wrap());
            ui.add_space(space::S);
            self.supcom_source(ui, importing, &mut actions);
            let Some(found) = self.supcom.found.clone() else { return };
            if found.install.is_none() {
                return;
            }
            ui.add_space(space::S);
            if found.maps.is_empty() {
                ui.add(egui::Label::new(RichText::new(t("No skirmish maps were found in this copy of the game.")).color(AMBER)).wrap());
                return;
            }
            // How many are chosen, and what they take.
            let chosen: Vec<&supcom::Entry> = found.maps.iter().filter(|e| self.supcom.chosen.contains(&e.folder)).collect();
            let gb = supcom::size_estimate(&chosen) as f64 / 1e9;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                if ui.add_enabled(!importing, egui::Button::new(RichText::new(t("SELECT ALL")).font(strong(11.5)))).clicked() {
                    self.supcom.chosen = found.maps.iter().map(|e| e.folder.clone()).collect();
                }
                if ui.add_enabled(!importing, egui::Button::new(RichText::new(t("SELECT NONE")).font(strong(11.5)))).clicked() {
                    self.supcom.chosen.clear();
                }
                let summary = if gb >= 0.1 {
                    tf("{n} of {total} maps chosen, about {size} GB on the disk", &[("n", &chosen.len()), ("total", &found.maps.len()), ("size", &format!("{gb:.1}"))])
                } else {
                    tf("{n} of {total} maps chosen", &[("n", &chosen.len()), ("total", &found.maps.len())])
                };
                ui.label(RichText::new(summary).color(STEEL));
            });
            ui.add_space(space::XS);
            // The list leaves room for what follows it.
            let list_height = (ui.available_height() - 120.0).max(140.0);
            egui::ScrollArea::vertical().id_salt("supcom_maps").max_height(list_height).auto_shrink([false, true]).show(ui, |ui| {
                let width = ui.available_width();
                let columns = (((width + GAP) / (CARD + GAP)).floor() as usize).max(1);
                let card = (width - GAP * (columns - 1) as f32) / columns as f32;
                ui.spacing_mut().item_spacing = vec2(GAP, GAP);
                for row in found.maps.chunks(columns) {
                    ui.horizontal(|ui| {
                        for entry in row {
                            self.supcom_row(ui, entry, importing, card);
                        }
                    });
                }
            });
            ui.add_space(space::S);
            self.supcom_footer(ui, chosen.len(), blocked, &mut actions);
        });
        if response.exit {
            self.supcom.open = false;
        }
        for a in actions {
            self.push(a);
        }
    }

    /// Where the game is (or that it was not found), and the player's own maps' folders.
    fn supcom_source(&mut self, ui: &mut Ui, importing: bool, actions: &mut Vec<Action>) {
        if self.supcom.scanning() && self.supcom.found.is_none() {
            widgets::busy_line(ui, t("Looking for Supreme Commander: Forged Alliance…"));
            return;
        }
        let found = self.supcom.found.clone().unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            ui.label(widgets::caption("FORGED ALLIANCE"));
            if let Some(store) = found.install.as_ref().and_then(|game| game.store.name()) {
                widgets::tag(ui, store, GREEN);
            }
            let label = if found.install.is_some() { n("CHANGE FOLDER…") } else { n("CHOOSE FOLDER…") };
            if ui.add_enabled(!importing, ActionButton::secondary(label).height(26.0)).hover_tip(t("Choose the folder Supreme Commander: Forged Alliance is installed in.")).clicked() {
                actions.push(Action::SupcomBrowse);
            }
        });
        match &found.install {
            Some(game) => {
                path_label(ui, &game.root, TEXT);
            }
            None => {
                ui.add(egui::Label::new(RichText::new(t("Supreme Commander: Forged Alliance was not found. TA Reborn looks for GOG's and Steam's copies of it: if yours is elsewhere, choose its folder.")).color(AMBER)).wrap());
            }
        }
        if let Some(error) = &found.error {
            ui.add(egui::Label::new(RichText::new(error).color(RED)).wrap());
        }
        if found.install.is_some() {
            ui.add_space(space::XS);
            ui.label(widgets::caption("YOUR OWN MAPS"));
            if found.own_folders.is_empty() {
                ui.add(egui::Label::new(RichText::new(t("None found (downloaded maps and Forged Alliance Forever's are listed when they are in your Documents folder).")).color(STEEL)).wrap());
            }
            for folder in &found.own_folders {
                path_label(ui, folder, TEXT);
            }
        }
    }

    /// A map of the list: its box, the game's picture of it, its name, size and players, and
    /// whether it was imported before.
    fn supcom_row(&mut self, ui: &mut Ui, entry: &supcom::Entry, importing: bool, width: f32) {
        let mut chosen = self.supcom.chosen.contains(&entry.folder);
        let (rect, response) = ui.allocate_exact_size(vec2(width, THUMB + 8.0), Sense::click());
        let hovered = response.hovered() && !importing;
        let painter = ui.painter_at(rect);
        if hovered || chosen {
            painter.rect(rect, CornerRadius::same(3), if chosen { Color32::from_rgba_unmultiplied(196, 160, 82, 18) } else { Color32::from_white_alpha(6) }, Stroke::new(1.0, if chosen { AMBER.gamma_multiply(0.5) } else { STEEL.gamma_multiply(0.3) }), StrokeKind::Inside);
        }
        // The box.
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(rect.min + vec2(8.0, (rect.height() - 20.0) / 2.0), vec2(20.0, 20.0))));
        let name = &entry.scenario.name;
        let toggled = child.add_enabled_ui(!importing, |ui| widgets::check_box_named(ui, &mut chosen, name)).inner.changed();
        // The picture.
        let thumb = egui::Rect::from_min_size(rect.min + vec2(38.0, 4.0), vec2(THUMB, THUMB));
        if let Some((w, h, rgba)) = &entry.preview {
            let texture = self.supcom.textures.entry(entry.folder.clone()).or_insert_with(|| {
                ui.ctx().load_texture(format!("supcom:{}", entry.folder.display()), egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], rgba), egui::TextureOptions::LINEAR)
            });
            painter.image(texture.id(), thumb, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            painter.rect_filled(thumb, CornerRadius::same(2), Color32::from_black_alpha(120));
        }
        painter.rect_stroke(thumb, CornerRadius::same(2), Stroke::new(1.0, STEEL.gamma_multiply(0.5)), StrokeKind::Inside);
        // Its name, size and players, and where it came from.
        let text_x = thumb.right() + 12.0;
        let room = (rect.right() - text_x - 8.0).max(1.0);
        let title = egui::WidgetText::from(RichText::new(name.as_str()).font(strong(14.0)).color(TEXT_BRIGHT)).into_galley(ui, Some(egui::TextWrapMode::Truncate), room, egui::TextStyle::Body);
        painter.galley(egui::pos2(text_x, rect.top() + 10.0), title, TEXT_BRIGHT);
        let mut detail = tf("{size} · {n} players", &[("size", &supcom::km(&entry.scenario)), ("n", &entry.scenario.armies.len())]).to_string();
        if entry.players_own {
            detail.push_str(" · ");
            detail.push_str(t("your own map"));
        }
        let line = egui::WidgetText::from(RichText::new(detail).font(body(12.5)).color(STEEL)).into_galley(ui, Some(egui::TextWrapMode::Truncate), room, egui::TextStyle::Body);
        painter.galley(egui::pos2(text_x, rect.top() + 34.0), line, STEEL);
        if let Some(label) = entry.state.label() {
            // (Its mark at the card's foot, under the picture's right.)
            let color = if entry.state == State::Imported { GREEN } else { AMBER };
            let mark = egui::WidgetText::from(RichText::new(t(label)).font(strong(9.5)).color(color)).into_galley(ui, Some(egui::TextWrapMode::Truncate), room, egui::TextStyle::Body);
            let at = egui::pos2(rect.right() - mark.size().x - 8.0, rect.top() + 6.0);
            painter.galley(at, mark, color);
        }
        let response = response.on_hover_text(entry.scenario.description.clone());
        if (response.clicked() && !importing) || toggled {
            if response.clicked() {
                chosen = !chosen;
            }
            if chosen {
                self.supcom.chosen.insert(entry.folder.clone());
            } else {
                self.supcom.chosen.remove(&entry.folder);
            }
        }
    }

    /// The import under way (map by map), or the button to start one; what the last came to.
    fn supcom_footer(&mut self, ui: &mut Ui, chosen: usize, blocked: Option<&'static str>, actions: &mut Vec<Action>) {
        if self.supcom.importing() {
            let p = self.supcom.progress.lock().unwrap().clone();
            let fraction = if p.total == 0 { None } else { Some(((p.done as f32 + p.share.clamp(0.0, 1.0)) / p.total as f32).clamp(0.0, 1.0)) };
            let label = tf("Making {map} ({i} of {n})…", &[("map", &p.current), ("i", &(p.done + 1).min(p.total)), ("n", &p.total)]);
            let width = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(vec2(width, 10.0), Sense::hover());
            ta_ui::progress::paint(ui, rect, fraction, true);
            ui.label(RichText::new(label).color(TEXT));
            if ui.add(ActionButton::secondary("STOP AFTER THIS MAP")).hover_tip(t("The maps made so far are kept.")).clicked() {
                actions.push(Action::SupcomStop);
            }
            return;
        }
        if let Some(outcome) = &self.supcom.outcome {
            let (text, color) = match &outcome.error {
                Some(e) => (tf("The maps could not be imported: {why}", &[("why", e)]).to_string(), RED),
                None if outcome.imported.is_empty() && outcome.failed.is_empty() => (t("Nothing was imported.").to_string(), STEEL),
                None => (tf("Imported {n} maps: they are in the map library, and in skirmish.", &[("n", &outcome.imported.len())]).to_string(), GREEN),
            };
            ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
            if !outcome.failed.is_empty() {
                ui.collapsing(tf("{n} maps could not be made", &[("n", &outcome.failed.len())]), |ui| {
                    for (name, why) in &outcome.failed {
                        ui.add(egui::Label::new(RichText::new(format!("{name}: {why}")).color(AMBER)).wrap());
                    }
                });
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let label = tf("IMPORT {n} MAPS", &[("n", &chosen)]);
            let can = chosen > 0 && blocked.is_none();
            if ui.add_enabled(can, ActionButton::primary(&label)).hover_tip(t("Makes the chosen maps into TA Reborn maps, in a map pack of their own. The largest take a minute each.")).disabled_tip(blocked.unwrap_or_else(|| t("Choose the maps to import."))).clicked() {
                actions.push(Action::SupcomImport);
            }
            if ui.add(ActionButton::secondary("CLOSE")).clicked() {
                self.supcom.open = false;
            }
        });
    }
}
