//! Retained, bounded battle notices. Presentation only; not part of saves or lockstep state.

use std::collections::VecDeque;

use egui::{Color32, RichText};
use ta_ui::i18n::{t, tf};
use ta_ui::panel::{self, Exit, Panel};

use crate::theme::*;

const CAPACITY: usize = 200;
const FILTER_CHAR_LIMIT: usize = 1024;

fn filter_id(revision: u64) -> egui::Id { egui::Id::new(("battle_history_filter", revision)) }

#[derive(Clone, Debug)]
pub struct Entry {
    pub tick: u64,
    pub text: String,
    pub repeats: u32,
    pub serial: u64,
}

pub struct History {
    revision: u64,
    entries: VecDeque<Entry>,
    serial: u64,
}

fn next_revision() -> u64 {
    static REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Default for History {
    fn default() -> Self { Self { revision: next_revision(), entries: VecDeque::new(), serial: 0 } }
}

impl History {
    pub fn record(&mut self, tick: u64, text: &str) {
        self.serial = self.serial.saturating_add(1);
        let text: String = text.chars().take(1024).collect();
        if let Some(last) = self.entries.back_mut()
            && last.text == text && tick >= last.tick && tick - last.tick <= 60
        {
            last.tick = tick;
            last.repeats = last.repeats.saturating_add(1);
            last.serial = self.serial;
            return;
        }
        if self.entries.len() == CAPACITY { self.entries.pop_front(); }
        self.entries.push_back(Entry { tick, text, repeats: 1, serial: self.serial });
    }

    pub fn clear(&mut self) { self.entries.clear(); self.revision = next_revision(); }
    pub fn revision(&self) -> u64 { self.revision }
    pub fn latest(&self) -> Option<&Entry> { self.entries.back() }
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &Entry> { self.entries.iter() }
}

#[derive(Default)]
pub struct HistoryWindow {
    revision: Option<u64>,
    filter: String,
}

impl HistoryWindow {
    /// The window, while `open`: the notices newest first, each at its time in the battle (a
    /// repeat counted), found by what they say; `summary` (the resources and the selection, as
    /// the screen reader hears them) over them. Escape, the close cross or a click outside closes
    /// it.
    pub fn show(&mut self, ctx: &egui::Context, history: &History, open: &mut bool, summary: &str) {
        if !*open { return; }
        if self.revision != Some(history.revision()) {
            self.revision = Some(history.revision());
            self.filter.clear();
        }
        let screen = ctx.content_rect();
        let count = history.entries().count();
        let subtitle = if count == 1 { t("1 NOTICE").to_string() } else { tf("{n} NOTICES", &[("n", &count)]) };
        let page = Panel::new("BATTLE EVENT HISTORY").subtitle(&subtitle).width((screen.width() - 120.0).clamp(280.0, 640.0)).exit(Exit::Close);
        let response = panel::modal(ctx, "battle_history", page, true, |ui| {
            ui.label(RichText::new(summary).color(STEEL));
            ui.add_space(8.0);
            let id = filter_id(history.revision());
            // egui 0.36 exposes a text value but does not handle native SetValue for
            // TextEdit. Accept only this visible field's action, before building its
            // text/selection nodes, so UIA reads the replacement in the same frame.
            let mut replaced = false;
            ui.input_mut(|input| input.consume_accesskit_action_requests(id, |request| {
                if request.action == egui::accesskit::Action::SetValue
                    && let Some(egui::accesskit::ActionData::Value(value)) = &request.data
                {
                    self.filter = value.chars().filter(|&c| c != '\r' && c != '\n').take(FILTER_CHAR_LIMIT).collect();
                    replaced = true;
                    true
                } else { false }
            }));
            if replaced && let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(self.filter.chars().count()))));
                state.store(ctx, id);
            }
            let filter = ta_ui::widgets::search_field(ui, &mut self.filter, t("Filter notices"), ui.available_width(), Some(id));
            if self.filter.chars().count() > FILTER_CHAR_LIMIT {
                self.filter = self.filter.chars().take(FILTER_CHAR_LIMIT).collect();
            }
            // A placeholder becomes HelpText on Windows, not the field's accessible
            // name. Keep the name available after the player has entered a query.
            ctx.accesskit_node_builder(filter.id, |node| {
                node.set_label(t("Filter notices"));
                node.add_action(egui::accesskit::Action::SetValue);
            });
            ui.add_space(6.0);
            let query = self.filter.trim().to_lowercase();
            let height = (screen.height() - 330.0).clamp(80.0, 520.0);
            let area = egui::ScrollArea::vertical().id_salt(("history_entries", history.revision())).max_height(height).auto_shrink([false, false]);
            ta_ui::scroll::well(ui, area, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut found = 0;
                for entry in history.entries().rev().filter(|e| query.is_empty() || e.text.to_lowercase().contains(&query)) {
                    ui.push_id(entry.serial, |ui| notice(ui, entry, found % 2 == 1));
                    found += 1;
                }
                if found == 0 {
                    // In the middle of the well, under a mark of what is missing: a log's lines
                    // (nothing yet), a magnifier (nothing the filter finds).
                    let empty = if count == 0 { t("Nothing has happened yet.") } else { t("No matching notices") };
                    ui.add_space(((height - 76.0) * 0.42).max(10.0));
                    ui.vertical_centered(|ui| {
                        let (mark, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                        if count == 0 {
                            log_mark(ui.painter(), mark, STEEL_DIM);
                        } else {
                            ta_ui::widgets::paint_glyph(ui.painter(), ta_ui::widgets::Glyph::Search, mark.center(), 2.2, STEEL_DIM);
                        }
                        ui.add_space(10.0);
                        ui.label(RichText::new(empty).color(STEEL));
                    });
                }
            });
            ui.add_space(8.0);
            ui.label(RichText::new(tf("Newest first; repeated notices are grouped. Keeps the latest {count} notices for this battle session.", &[("count", &CAPACITY)])).font(body(12.0)).color(STEEL_DIM));
        });
        if response.exit { *open = false; }
    }
}

/// A battle log's mark for an empty history: a slate with a time's column and lines of notices.
fn log_mark(painter: &egui::Painter, rect: egui::Rect, colour: Color32) {
    let slate = rect.shrink2(egui::vec2(3.0, 5.0));
    painter.rect_stroke(slate, 3.0, egui::Stroke::new(2.0, colour), egui::StrokeKind::Inside);
    for (k, length) in [0.8, 0.55, 0.7].into_iter().enumerate() {
        let y = slate.min.y + 9.0 + k as f32 * 7.5;
        painter.line_segment([egui::pos2(slate.min.x + 6.0, y), egui::pos2(slate.min.x + 10.0, y)], egui::Stroke::new(2.0, colour));
        let x0 = slate.min.x + 14.0;
        painter.line_segment([egui::pos2(x0, y), egui::pos2(x0 + (slate.max.x - 6.0 - x0) * length, y)], egui::Stroke::new(2.0, colour));
    }
}

/// A notice in the list: its time in the battle (and how often it came, when more than once) in
/// a column of its own, what it said beside it; every other one a shade lighter.
fn notice(ui: &mut egui::Ui, entry: &Entry, shaded: bool) {
    let fill = if shaded { Color32::from_white_alpha(5) } else { Color32::TRANSPARENT };
    egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            let seconds = entry.tick / 30;
            ui.add_sized([52.0, 18.0], egui::Label::new(RichText::new(format!("{:02}:{:02}", seconds / 60, seconds % 60)).font(mono(12.5)).color(AMBER)));
            if entry.repeats > 1 {
                ui.label(RichText::new(format!("×{}", entry.repeats)).font(strong(12.0)).color(AMBER_BRIGHT));
            }
            ui.add(egui::Label::new(RichText::new(&entry.text).color(TEXT)).wrap());
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_filter_value_actions_are_named_bounded_and_scoped() {
        use egui::accesskit::{Action, ActionData, ActionRequest, TreeId};
        let ctx = egui::Context::default();
        crate::theme::install_compact(&ctx);
        ctx.enable_accesskit();
        let history = History::default();
        let id = filter_id(history.revision());
        let mut window = HistoryWindow::default();
        let draw = |window: &mut HistoryWindow, mut open: bool, events| {
            let mut output = ctx.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
                events,
                ..Default::default()
            }, |ui| window.show(ui.ctx(), &history, &mut open, "Resources"));
            output.textures_delta.clear();
            output
        };
        let replace = |target: egui::Id, value: &str| egui::Event::AccessKitActionRequest(ActionRequest {
            action: Action::SetValue, target_tree: TreeId::ROOT, target_node: target.accesskit_id(),
            data: Some(ActionData::Value(value.into())),
        });
        let initial = draw(&mut window, true, vec![]);
        let tree = initial.platform_output.accesskit_update.unwrap();
        let field = &tree.nodes.iter().find(|(node, _)| *node == id.accesskit_id()).unwrap().1;
        assert_eq!(field.label(), Some(t("Filter notices")));
        assert!(field.supports_action(Action::SetValue));

        draw(&mut window, true, vec![replace(egui::Id::new("another_field"), "wrong")]);
        assert!(window.filter.is_empty(), "actions for other controls must not enter this filter");
        draw(&mut window, true, vec![replace(id, "commander")]);
        assert_eq!(window.filter, "commander");
        draw(&mut window, false, vec![replace(id, "hidden")]);
        assert_eq!(window.filter, "commander", "a closed dialog must not accept a stale field action");
        draw(&mut window, true, vec![replace(id, &("\n界\r".repeat(FILTER_CHAR_LIMIT + 20)))]);
        assert_eq!(window.filter.chars().count(), FILTER_CHAR_LIMIT);
        assert!(window.filter.chars().all(|c| c == '界'));
        draw(&mut window, true, vec![replace(id, "")]);
        assert!(window.filter.is_empty(), "native replacement can also clear the query");
    }

    #[test]
    fn history_retains_expired_notices_and_bounds_memory() {
        let mut history = History::default();
        for tick in 0..250 { history.record(tick, &format!("Notice {tick}")); }
        assert_eq!(history.entries.len(), CAPACITY);
        assert_eq!(history.entries.front().unwrap().tick, 50);
        assert_eq!(history.latest().unwrap().text, "Notice 249");
        history.record(260, &"é".repeat(3000));
        assert_eq!(history.latest().unwrap().text.chars().count(), 1024);
    }

    #[test]
    fn history_coalesces_only_recent_consecutive_repeats() {
        let mut history = History::default();
        history.record(30, "Attack");
        history.record(60, "Attack");
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.latest().unwrap().repeats, 2);
        history.record(61, "Resources");
        history.record(62, "Attack");
        history.record(200, "Attack");
        assert_eq!(history.entries.len(), 4);
        let last = history.latest().unwrap().serial;
        let revision = history.revision();
        history.clear();
        assert_ne!(history.revision(), revision);
        assert_ne!(History::default().revision(), revision);
        history.record(0, "New timeline");
        assert!(history.latest().unwrap().serial > last, "readers can distinguish entries after a rewind");
    }
}
