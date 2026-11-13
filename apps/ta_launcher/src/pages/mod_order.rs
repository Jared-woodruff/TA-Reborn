//! The Mods page's load order: the mods that are on, in the order the game loads them (where two
//! change the same file, the later one wins), to drag into place or move with the keys or the
//! buttons; under them the order the game will use, or what is wrong with it and its quick fixes.

use egui::{Align, Align2, Id, Layout, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use ta_ui::i18n::{t, tf};
use ta_ui::tooltip::TooltipExt as _;

use super::sentence;
use crate::app::{Action, LauncherApp, ModsTab};
use crate::modlist::{self, Resolution};
use crate::theme::*;
use crate::widgets::{self, Glyph, Mark};

/// A row's height.
const ROW: f32 = 56.0;

impl LauncherApp {
    pub(super) fn load_order_view(&mut self, ui: &mut Ui) {
        ui.add(egui::Label::new(RichText::new(t("Mods load from the top down: where two change the same file, the lower one wins. Drag a mod to move it, or use Alt with the arrow keys.")).color(STEEL)).wrap());
        ui.add_space(space::S);
        let enabled = self.config.mods.enabled.clone();
        if enabled.is_empty() {
            super::wrapped(ui, t("No mods are on: the game runs as it is. Turn mods on in the library."), STEEL);
            if widgets::text_link(ui, "TO THE LIBRARY").clicked() { self.mods_tab = ModsTab::Library; }
            return;
        }
        // (The order the game will use, or what is wrong, takes the foot of the page.)
        let footer = if matches!(self.mods.resolution, Resolution::Ok(_)) { 64.0 } else { 96.0 };
        let height = (ui.available_height() - footer).max(ROW * 2.0);
        let mut area = egui::ScrollArea::vertical().id_salt("mod_load_order").max_height(height).auto_shrink([false, true]);
        if let Some(offset) = self.capture_scroll { area = area.vertical_scroll_offset(offset); }
        let mut moves = None;
        let mut actions = Vec::new();
        ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Backdrop(BG), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut rows = Vec::new();
            for (index, id) in enabled.iter().enumerate() {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                rows.push(rect);
                if let Some(action) = self.load_order_row(ui, rect, index, id, enabled.len(), &mut moves) { actions.push(action); }
            }
            self.drag_rows(ui, &rows, &mut moves);
        });
        if let Some((from, to)) = moves {
            modlist::move_to(&mut self.config.mods.enabled, from, to);
            self.mods.refresh(&self.config.mods.enabled);
            ui.ctx().request_repaint();
        }
        for action in actions { self.push(action); }
        ui.add_space(space::S);
        self.load_order_status(ui, &enabled);
        // (A row's right-click, or Shift+F10: the mod's menu, its moves among the rest.)
        self.mod_menu(ui, ui.available_width() >= super::mods::SPLIT, true);
    }

    /// One mod of the order: where it is, what it is, its check, and its moves.
    fn load_order_row(&mut self, ui: &mut Ui, rect: Rect, index: usize, id: &str, count: usize, moves: &mut Option<(usize, usize)>) -> Option<Action> {
        let row = ui.interact(rect, Id::new(("load_order_row", id)), Sense::click_and_drag());
        let entry = self.mods.get(id).cloned();
        let name = entry.as_ref().map_or(id.to_string(), |entry| entry.info.manifest.name.clone());
        row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, tf("{n}. {name}", &[("n", &(index + 1)), ("name", &name)])));
        widgets::reveal_focus(ui, &row);
        if let Some((at, keys)) = crate::library::menu::asked(ui, &row) {
            crate::library::menu::open(ui.ctx(), super::mods::mod_menu_id(), id, at, keys.then_some(row.id));
        }
        if row.drag_started() { self.mod_library.dragging = Some((id.to_string(), index)); }
        if self.mod_library.order_reveal.as_deref() == Some(id) {
            self.mod_library.order_reveal = None;
            ui.scroll_to_rect(rect, None);
        }
        // The keys: Alt with Up and Down a step, with Home and End to either end.
        if row.has_focus() && self.search_shortcuts_allowed(ui) {
            let key = ui.input_mut(|input| [egui::Key::ArrowUp, egui::Key::ArrowDown, egui::Key::Home, egui::Key::End]
                .into_iter().find(|key| input.consume_key(egui::Modifiers::ALT, *key)));
            let to = match key {
                Some(egui::Key::ArrowUp) => index.checked_sub(1),
                Some(egui::Key::ArrowDown) => (index + 1 < count).then_some(index + 1),
                Some(egui::Key::Home) => Some(0),
                Some(egui::Key::End) => Some(count - 1),
                _ => None,
            };
            if let Some(to) = to.filter(|to| *to != index) {
                *moves = Some((index, to));
                self.mod_library.order_reveal = Some(id.to_string());
            }
        }
        let mut action = None;
        if ui.is_rect_visible(rect) {
            let dragged = self.mod_library.dragging.as_ref().is_some_and(|(dragged, _)| dragged == id);
            let hot = row.hovered() || dragged;
            let painter = ui.painter();
            ta_ui::panel::row_state(painter, rect, dragged, hot);
            painter.add(ta_ui::metal::seam(rect.x_range().shrink(4.0), rect.bottom() - 1.0));
            // The grip: two columns of dots.
            for column in 0..2 {
                for dot in 0..3 {
                    painter.circle_filled(pos2(rect.left() + 12.0 + column as f32 * 5.0, rect.center().y - 6.0 + dot as f32 * 6.0), 1.6, if hot { STEEL } else { STEEL_DIM });
                }
            }
            painter.text(pos2(rect.left() + 28.0, rect.center().y), Align2::LEFT_CENTER, format!("{:02}", index + 1), display(22.0), AMBER_DIM);
            widgets::focus_ring(ui, &row, rect.shrink(3.0));
        }
        // The controls at the right, the words in what is left.
        let controls = Rect::from_min_max(pos2(rect.left() + 64.0, rect.top()), pos2(rect.right() - 8.0, rect.bottom()));
        let mut used = 0.0;
        ui.scope_builder(egui::UiBuilder::new().max_rect(controls).layout(Layout::right_to_left(Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            match &entry {
                Some(_) => {
                    let mut on = true;
                    if widgets::toggle_named(ui, &mut on, &name).hover_tip(t("Turn the mod on or off for the next game")).changed() { action = Some(Action::DisableMod(id.to_string())); }
                }
                None => {
                    if widgets::icon_button(ui, Glyph::Close, true).hover_tip(t("Remove from the load order")).clicked() { action = Some(Action::DisableMod(id.to_string())); }
                }
            }
            ui.add_space(6.0);
            for (glyph, to, enabled) in [(Glyph::Bottom, count - 1, index + 1 < count), (Glyph::Down, index + 1, index + 1 < count), (Glyph::Up, index.saturating_sub(1), index > 0), (Glyph::Top, 0, index > 0)] {
                let tip = match glyph { Glyph::Top => t("Move to the top"), Glyph::Up => t("Load earlier"), Glyph::Down => t("Load later"), _ => t("Move to the bottom") };
                if widgets::icon_button(ui, glyph, enabled).hover_tip(tip).clicked() {
                    *moves = Some((index, to));
                    self.mod_library.order_reveal = Some(id.to_string());
                }
            }
            if ui.available_width() > 260.0 {
                ui.add_space(6.0);
                match &entry {
                    Some(_) => {
                        let (label, colour) = super::mod_report::validation_badge(&self.mod_validation.status(id));
                        let badge = widgets::tag(ui, label, colour).interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand)
                            .hover_tip(t("Open the mod's report (MOD INFO)"));
                        if badge.clicked() { action = Some(Action::ShowModReport(id.to_string())); }
                    }
                    None => { widgets::tag(ui, "NOT FOUND", RED); }
                }
            }
            // (Hidden from the library, it loads all the same: said here, where it does.)
            if self.config.mods.is_hidden(id) && ui.available_width() > 220.0 {
                ui.add_space(2.0);
                widgets::tag(ui, &super::mods::hidden_label(), AMBER).hover_tip(t("Hidden from the library: it still loads, in this order."));
            }
            used = controls.width() - ui.available_width();
        });
        let words = Rect::from_min_max(pos2(rect.left() + 64.0, rect.top()), pos2((rect.right() - 8.0 - used - 10.0).max(rect.left() + 100.0), rect.bottom()));
        if ui.is_rect_visible(words) {
            let detail = match &entry {
                Some(entry) => {
                    let mut parts = vec![super::mods::folder_label(self.mods.folder_label(entry)).to_string()];
                    if !entry.info.manifest.version.is_empty() { parts.push(entry.info.manifest.version.clone()); }
                    parts.join(" · ")
                }
                None => t("Enabled in your settings, but not installed in any mod folder.").to_string(),
            };
            let line = |text: &str, font: egui::FontId, colour| egui::WidgetText::from(RichText::new(text).font(font).color(colour))
                .into_galley(ui, Some(egui::TextWrapMode::Truncate), words.width().max(1.0), egui::TextStyle::Body);
            let title = line(&name, strong(15.0), TEXT_BRIGHT);
            let under = line(&detail, body(12.0), STEEL);
            let top = words.center().y - (title.size().y + under.size().y + 1.0) * 0.5;
            let below = top + title.size().y + 1.0;
            ui.painter().galley(pos2(words.left(), top), title, TEXT_BRIGHT);
            ui.painter().galley(pos2(words.left(), below), under, STEEL);
        }
        action
    }

    /// A row being dragged: a brass line where it would land, and the move when it is let go.
    fn drag_rows(&mut self, ui: &mut Ui, rows: &[Rect], moves: &mut Option<(usize, usize)>) {
        let Some((id, _)) = self.mod_library.dragging.clone() else { return };
        // (Where the mod is now: the order may have changed under the drag. Gone, the drag is.)
        let Some(from) = self.config.mods.enabled.iter().position(|e| *e == id) else {
            self.mod_library.dragging = None;
            return;
        };
        let Some(pointer) = ui.ctx().pointer_latest_pos() else { return };
        // (The slot: how many rows lie above the pointer.)
        let slot = rows.iter().filter(|row| row.center().y < pointer.y).count();
        let y = if slot == 0 { rows.first().map_or(pointer.y, Rect::top) } else { rows[slot - 1].bottom() };
        if let Some(first) = rows.first() {
            ui.painter().hline(first.x_range().shrink(4.0), y, Stroke::new(3.0, AMBER_BRIGHT));
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if ui.input(|input| !input.pointer.any_down()) {
            let to = if slot > from { slot - 1 } else { slot };
            if to != from { *moves = Some((from, to)); }
            self.mod_library.dragging = None;
        }
        ui.ctx().request_repaint();
    }

    /// The order the game will use, or what is wrong with it and how to put it right.
    fn load_order_status(&mut self, ui: &mut Ui, enabled: &[String]) {
        match self.mods.resolution.clone() {
            Resolution::Ok(order) => {
                ui.horizontal_wrapped(|ui| {
                    widgets::status_mark(ui, Mark::Ok);
                    ui.label(widgets::caption("EFFECTIVE ORDER"));
                    let mut chain = vec![t("Base game").to_owned()];
                    chain.extend(order.iter().map(|id| self.mods.get(id).map_or(id.clone(), |m| m.info.manifest.name.clone())));
                    ui.add(egui::Label::new(RichText::new(chain.join("  →  ")).color(TEXT)).wrap());
                });
            }
            Resolution::Err(message) => {
                ui.horizontal_wrapped(|ui| {
                    widgets::status_mark(ui, Mark::Error);
                    ui.add(egui::Label::new(RichText::new(sentence(&message)).color(RED)).wrap());
                });
                let mut fixes: Vec<String> = Vec::new();
                for id in enabled {
                    for dependency in self.mods.disabled_dependencies(id, enabled) {
                        if !fixes.contains(&dependency) { fixes.push(dependency); }
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    for dependency in fixes {
                        let name = self.mods.get(&dependency).map_or(dependency.clone(), |m| m.info.manifest.name.clone());
                        if ui.add(widgets::ActionButton::primary(&tf("ENABLE {name}", &[("name", &ta_ui::i18n::upper(&name))])).height(28.0)).clicked() {
                            self.push(Action::EnableMod(dependency));
                        }
                    }
                });
            }
        }
    }
}
