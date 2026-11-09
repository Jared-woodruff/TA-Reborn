//! The skin editor: the skin being made on two units turning beside the controls that make it.
//! Its paint (a livery): a pattern chosen from tiles drawn in its own colours, the colours from
//! swatches (a picker, hex, palettes), the pattern's size and edges, its finish and its wear.
//! Its texture slots: a picture of one's own on the units' armour, for every unit or one.

use egui::{Align, Color32, Layout, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui, pos2, vec2};
use ta_skins::{Colour, Livery, Pattern};
use ta_ui::i18n::{n, t, tf};
use ta_ui::tooltip::TooltipExt as _;

use crate::app::{Action, LauncherApp};
use crate::skin_editor::{EVERY, PLAIN};
use crate::skins::Ask;
use crate::theme::*;
use crate::widgets::{self, ActionButton};

/// From this width the preview stands beside the controls.
const SPLIT: f32 = 760.0;
/// A pattern tile, in points.
const TILE: egui::Vec2 = vec2(88.0, 56.0);

/// Colour sets to start a livery from: a name, and its colours.
const PALETTES: [(&str, &[u32]); 20] = [
    (n("Olive drab"), &[0x6b705c, 0x4a5236, 0x2d3021]),
    (n("Desert"), &[0xc4a676, 0x9b7445, 0x5e4630]),
    (n("Arctic"), &[0xe4e7e6, 0xa6adb0, 0x5b646a]),
    (n("Jungle"), &[0x3f5a2b, 0x6b4f2a, 0x1f2a17]),
    (n("Navy"), &[0x8e99a0, 0x5f6c75, 0x333e48]),
    (n("Urban"), &[0xb7b9b8, 0x7f8386, 0x3e4144]),
    (n("Night"), &[0x1b1f27, 0x232a36, 0x12151a]),
    (n("Tiger"), &[0xc86a1e, 0x1c1712]),
    (n("Crimson"), &[0x8e1b1b, 0x2a2a2a, 0xc9a44c]),
    (n("Royal"), &[0x1f3a7a, 0xe8e2d0, 0xc9a44c]),
    (n("Sand and sky"), &[0xd8c8a0, 0x6a8db3, 0x3c4f62]),
    (n("Arm blue"), &[0x3f5f8f, 0xa9b8c8, 0x1e2a3a]),
    (n("Core red"), &[0x7a2a22, 0x3a3a3a, 0xb0a090]),
    (n("United Kingdom"), &[0x0d2a6b, 0xe9e9e4, 0xc8102e]),
    (n("United States"), &[0x2e3a6e, 0xe9e9e4, 0xb22234]),
    (n("Australia"), &[0x00733a, 0xffcd00]),
    (n("New Zealand"), &[0x161616, 0x55595d, 0xc3c7cb]),
    (n("Italy"), &[0x009246, 0xeeeeea, 0xce2b37]),
    (n("France"), &[0x0055a4, 0xeeeeea, 0xef4135]),
    (n("Germany"), &[0x1b1b1b, 0xdd0000, 0xffce00]),
];

/// Which colour of the livery a swatch edits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Swatch { Colour(usize), Trim }

impl LauncherApp {
    pub(crate) fn skin_editor_page(&mut self, ui: &mut Ui) {
        self.skin_editor_keys(ui);
        self.skin_editor_bar(ui);
        if let Some(status) = self.skins.status() { widgets::busy_line(ui, t(status)); }
        self.skin_editor_feedback(ui);
        let width = ui.available_width();
        let height = ui.available_height().max(120.0);
        if width >= SPLIT {
            let preview = (width * 0.4).clamp(300.0, 560.0);
            let gap = space::L;
            let top = ui.cursor().min;
            let left = Rect::from_min_size(top, vec2(preview, height));
            let right = Rect::from_min_size(pos2(top.x + preview + gap, top.y), vec2((width - preview - gap).max(1.0), height));
            ui.scope_builder(egui::UiBuilder::new().max_rect(left).layout(Layout::top_down(Align::Min)), |ui| self.skin_editor_preview(ui, height));
            ui.scope_builder(egui::UiBuilder::new().max_rect(right).layout(Layout::top_down(Align::Min)), |ui| {
                let mut area = egui::ScrollArea::vertical().id_salt("skin_editor_controls").max_height(height).auto_shrink([false, false]);
                if let Some(offset) = self.capture_scroll { area = area.vertical_scroll_offset(offset); }
                ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Backdrop(BG), |ui| {
                    ui.add_enabled_ui(!self.skins.busy(), |ui| self.skin_editor_controls(ui));
                });
            });
            ui.allocate_rect(left.union(right), Sense::hover());
        } else {
            let mut area = egui::ScrollArea::vertical().id_salt("skin_editor_page").max_height(height).auto_shrink([false, false]);
            if let Some(offset) = self.capture_scroll { area = area.vertical_scroll_offset(offset); }
            ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Backdrop(BG), |ui| {
                self.skin_editor_preview(ui, (height * 0.45).clamp(220.0, 360.0));
                ui.add_space(space::M);
                ui.add_enabled_ui(!self.skins.busy(), |ui| self.skin_editor_controls(ui));
            });
        }
    }

    /// Ctrl+Z undoes, Ctrl+Y (or Ctrl+Shift+Z) redoes, Ctrl+S saves: not while typing.
    fn skin_editor_keys(&mut self, ui: &Ui) {
        if !self.search_shortcuts_allowed(ui) || ui.ctx().egui_wants_keyboard_input() || self.skins.busy() { return; }
        let (undo, redo, save) = ui.input_mut(|input| (
            input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z),
            input.consume_key(egui::Modifiers::COMMAND, egui::Key::Y) || input.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z),
            input.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
        ));
        let Some(editor) = self.skins.editor.as_mut() else { return };
        if undo { editor.undo(); }
        if redo { editor.redo(); }
        if save && editor.dirty() && !self.skins.busy() { self.push(Action::SaveSkin); }
    }

    /// Back to the library; the skin's name and whether it is saved; undo, redo, revert, save.
    fn skin_editor_bar(&mut self, ui: &mut Ui) {
        let Some(editor) = self.skins.editor.as_ref() else { return };
        let (dirty, can_undo, can_redo, saved_once, name) = (editor.dirty(), editor.can_undo(), editor.can_redo(), editor.id.is_some(), editor.manifest.name.clone());
        let idle = !self.skins.busy();
        let mut back = false;
        let status = if !dirty { t("Saved") } else if saved_once { t("Unsaved changes") } else { t("Not saved yet") };
        // (The buttons beside the title where they fit, in a row of their own where not.)
        let labels = ["UNDO", "REDO", "REVERT", "SAVE AS…", "SAVE"];
        let buttons: f32 = labels.iter().map(|label| widgets::galley(ui, RichText::new(t(label)).font(strong(13.5)).extra_letter_spacing(ta_ui::i18n::tracking(0.8))).size().x + 40.0).sum();
        let title = 170.0 + widgets::galley(ui, RichText::new(&name).font(display(22.0))).size().x + widgets::galley(ui, RichText::new(status).font(strong(12.0))).size().x + 40.0;
        let one_row = title + buttons <= ui.available_width();
        let mut save_as = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
            back = ui.add_enabled_ui(idle, |ui| ta_ui::panel::back_button(ui, "ALL SKINS")).inner.clicked();
            ta_ui::metal::embossed_label(ui, RichText::new(&name).font(display(22.0)), TEXT_BRIGHT);
            if !dirty { widgets::status_mark(ui, widgets::Mark::Ok); }
            ui.label(RichText::new(status).color(if dirty { AMBER } else { GREEN }).font(strong(12.0)));
            if one_row {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| save_as = self.skin_editor_buttons(ui, idle, dirty, saved_once, can_undo, can_redo, true));
            }
        });
        if !one_row {
            ui.horizontal_wrapped(|ui| save_as = self.skin_editor_buttons(ui, idle, dirty, saved_once, can_undo, can_redo, false));
        }
        if save_as { self.skins.ask = Some(Ask::SaveAs(tf("{name} copy", &[("name", &name)]))); }
        let escape = idle && self.search_shortcuts_allowed(ui) && ui.memory(|memory| memory.focused().is_none())
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if back || escape { self.leave_skin_editor(); }
        ui.add_space(space::S);
    }

    /// Undo, redo, revert, save as and save (`reversed`: laid right to left). True when SAVE AS…
    /// was chosen.
    #[allow(clippy::too_many_arguments)]
    fn skin_editor_buttons(&mut self, ui: &mut Ui, idle: bool, dirty: bool, saved_once: bool, can_undo: bool, can_redo: bool, reversed: bool) -> bool {
        ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
        let now = ui.input(|input| input.time);
        let mut save_as = false;
        let mut order = [0, 1, 2, 3, 4];
        if reversed { order.reverse(); }
        for button in order {
            match button {
                0 => if ui.add_enabled(idle && can_undo, ActionButton::secondary("UNDO").height(30.0)).hover_tip(t("Undo (Ctrl+Z)")).clicked()
                    && let Some(editor) = self.skins.editor.as_mut() { editor.undo(); },
                1 => if ui.add_enabled(idle && can_redo, ActionButton::secondary("REDO").height(30.0)).hover_tip(t("Redo (Ctrl+Y)")).clicked()
                    && let Some(editor) = self.skins.editor.as_mut() { editor.redo(); },
                2 => if ui.add_enabled(idle && saved_once && dirty, ActionButton::secondary("REVERT").height(30.0)).hover_tip(t("Back to the skin as last saved")).clicked()
                    && let Some(editor) = self.skins.editor.as_mut() { editor.revert(now); },
                3 => save_as = ui.add_enabled(idle, ActionButton::secondary("SAVE AS…").height(30.0)).hover_tip(t("Save a copy under another name")).clicked(),
                _ => if ui.add_enabled(idle && dirty, ActionButton::primary("SAVE").height(30.0)).hover_tip(t("Save this skin (Ctrl+S)")).clicked() { self.push(Action::SaveSkin); },
            }
        }
        save_as
    }

    /// Leaves the editor, asking first when there are changes not saved.
    pub(crate) fn leave_skin_editor(&mut self) {
        match self.skins.editor.as_ref() {
            Some(editor) if editor.dirty() => self.skins.ask = Some(Ask::Leave),
            Some(editor) => {
                if let Some(id) = editor.id.clone() { self.skins.browser.inspecting = id.clone(); self.skins.browser.reveal = Some(id); }
                self.skins.editor = None;
            }
            None => {}
        }
    }

    /// What a save or a picture came to.
    fn skin_editor_feedback(&mut self, ui: &mut Ui) {
        let Some(feedback) = &self.skins.feedback else { return };
        let (message, error) = (feedback.message.clone(), feedback.error);
        ui.horizontal_top(|ui| {
            let link = widgets::galley(ui, RichText::new(t("DISMISS")).font(strong(11.5))).size().x + 16.0;
            ui.allocate_ui_with_layout(vec2((ui.available_width() - link).max(1.0), 0.0), Layout::top_down(Align::Min), |ui| {
                super::wrapped(ui, &message, if error { RED } else { GREEN });
            });
            if widgets::text_link(ui, "DISMISS").clicked() { self.skins.feedback = None; }
        });
    }

    /// The skin on two units turning, and which units.
    fn skin_editor_preview(&mut self, ui: &mut Ui, height: f32) {
        let reference = self.skin_reference();
        let root = self.store.dir().join("skins");
        let revision = self.skins.preview_revision;
        let Some(editor) = self.skins.editor.as_mut() else { return };
        let serial = editor.serial;
        let paint = crate::skins::preview::Paint::Editing { pack: editor.preview_pack(), serial, revision: editor.textures, livery: editor.manifest.livery.clone() };
        let units = editor.units.clone();
        widgets::card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            self.skins.browser.preview.show(ui, reference, root, paint, revision, &units, (height - 140.0).clamp(160.0, 420.0));
            ui.add_space(space::XS);
            let choices = self.skins.browser.preview.units().to_vec();
            if choices.is_empty() { return; }
            let width = ((ui.available_width() - space::S) / 2.0).floor().max(1.0);
            let Some(editor) = self.skins.editor.as_mut() else { return };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::S;
                for (side, (arm, label)) in [(true, t("ARM")), (false, t("CORE"))].into_iter().enumerate() {
                    let mut options: Vec<(usize, String)> = vec![(0, t("Example").to_string())];
                    options.extend(choices.iter().filter(|choice| choice.arm == arm).enumerate().map(|(at, choice)| (at + 1, choice.display.clone())));
                    let names: Vec<&str> = choices.iter().filter(|choice| choice.arm == arm).map(|choice| choice.name.as_str()).collect();
                    let mut chosen = names.iter().position(|name| name.eq_ignore_ascii_case(&editor.units[side])).map_or(0, |at| at + 1);
                    if crate::library::choice(ui, if arm { "skin_editor_arm" } else { "skin_editor_core" }, width, label, &mut chosen, &options).changed() {
                        editor.units[side] = chosen.checked_sub(1).and_then(|at| names.get(at)).map(|name| name.to_string()).unwrap_or_default();
                        if editor.scope.is_some() { editor.scope = None; }
                    }
                }
            });
        });
    }

    /// The controls: the skin's name, its paint, its finish, its wear, its texture slots.
    fn skin_editor_controls(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = space::M;
        self.skin_editor_paint(ui);
        if self.skins.editor.as_ref().is_some_and(|editor| editor.manifest.livery.is_some()) {
            self.skin_editor_finish(ui);
        }
        self.skin_editor_slots(ui);
        self.skin_editor_about(ui);
    }

    fn skin_editor_about(&mut self, ui: &mut Ui) {
        let now = ui.input(|input| input.time);
        let Some(editor) = self.skins.editor.as_mut() else { return };
        widgets::card(ui, |ui| {
            widgets::card_title(ui, "ABOUT");
            let (mut name, mut author, mut description) = (editor.manifest.name.clone(), editor.manifest.author.clone(), editor.manifest.description.clone());
            let half = ((ui.available_width() - space::M) / 2.0).max(1.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = space::M;
                ui.vertical(|ui| {
                    ui.set_width(half);
                    ui.label(widgets::caption("SKIN NAME"));
                    let field = ui.add(egui::TextEdit::singleline(&mut name).char_limit(80).desired_width(half).hint_text(t("My unit skin")));
                    ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(t("SKIN NAME")));
                });
                ui.vertical(|ui| {
                    ui.set_width(half);
                    ui.label(widgets::caption("CREATOR"));
                    let field = ui.add(egui::TextEdit::singleline(&mut author).char_limit(120).desired_width(half).hint_text(t("Your name or alias")));
                    ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(t("CREATOR")));
                });
            });
            ui.label(widgets::caption("DESCRIPTION"));
            let field = ui.add(egui::TextEdit::multiline(&mut description).char_limit(2000).desired_rows(2).desired_width(ui.available_width()).hint_text(t("What it is, and who it is for")));
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(t("DESCRIPTION")));
            // (Names must not be empty: an emptied one stays as it was until something is typed.)
            if name != editor.manifest.name && !name.trim().is_empty() { editor.change("name", now, |manifest| manifest.name = name); }
            if author != editor.manifest.author { editor.change("author", now, |manifest| manifest.author = author); }
            if description != editor.manifest.description { editor.change("description", now, |manifest| manifest.description = description); }
        });
    }

    /// The livery: on or off, its pattern, colours, palettes, size and edges.
    fn skin_editor_paint(&mut self, ui: &mut Ui) {
        let now = ui.input(|input| input.time);
        let ctx = ui.ctx().clone();
        let Some(editor) = self.skins.editor.as_mut() else { return };
        widgets::card(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::card_title(ui, "PAINT");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut on = editor.manifest.livery.is_some();
                    if widgets::toggle_named(ui, &mut on, "Paint a livery over the units").hover_tip(t("Paint a livery over the units")).changed() {
                        editor.set_livery_on(on, now);
                    }
                    ui.label(RichText::new(t("Paint a livery over the units")).color(STEEL));
                });
            });
            let Some(livery) = editor.manifest.livery.clone() else {
                super::wrapped(ui, t("The units keep TA Reborn's own paint. Texture slots below still paint over it."), STEEL);
                return;
            };
            // The pattern: a tile for each, in this livery's colours.
            ui.label(widgets::caption("PATTERN"));
            let tiles = pattern_tiles(&ctx, &mut self.skins.editor_tiles, &livery);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                for (pattern, texture) in Pattern::ALL.into_iter().zip(&tiles) {
                    if pattern_tile(ui, texture, pattern, livery.pattern == pattern).clicked() {
                        editor.livery("pattern", now, |livery| livery.pattern = pattern);
                    }
                }
            });
            ui.add_space(space::XS);
            // The colours: a swatch each (and one to add), the trim.
            ui.label(widgets::caption("COLOURS"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                let names = [t("Ground"), t("Pattern"), t("Detail")];
                for (at, label) in names.iter().enumerate().take(livery.colours.len()) {
                    let mut colour = livery.colours[at];
                    if colour_swatch(ui, Swatch::Colour(at), label, &mut colour) {
                        editor.livery(if at == 0 { "colour 1" } else if at == 1 { "colour 2" } else { "colour 3" }, now, |livery| livery.colours[at] = colour);
                    }
                }
                if livery.colours.len() < ta_skins::MAX_COLOURS && ui.add(ActionButton::secondary("+ COLOUR").height(30.0)).hover_tip(t("Add a colour to the pattern")).clicked() {
                    let next = PALETTES[0].1.get(livery.colours.len()).copied().unwrap_or(0x808080);
                    editor.livery("add colour", now, |livery| livery.colours.push(Colour::rgb(next)));
                }
                if livery.colours.len() > 1 && widgets::icon_button(ui, widgets::Glyph::Close, true).hover_tip(t("Take the last colour away")).clicked() {
                    editor.livery("remove colour", now, |livery| { livery.colours.pop(); });
                }
                ui.add_space(space::M);
                let mut trimmed = livery.trim.is_some();
                if widgets::check_box_named(ui, &mut trimmed, "Trim").hover_tip(t("A band of colour along the convex edges")).changed() {
                    editor.livery("trim", now, |livery| livery.trim = trimmed.then_some(Colour::rgb(0xc9a44c)));
                }
                ui.label(RichText::new(t("Trim")).color(TEXT));
                if let Some(trim) = livery.trim {
                    let mut colour = trim;
                    if colour_swatch(ui, Swatch::Trim, t("Trim"), &mut colour) {
                        editor.livery("trim colour", now, |livery| livery.trim = Some(colour));
                    }
                }
            });
            ui.add_space(space::XS);
            ui.label(widgets::caption("PALETTES"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for (name, colours) in PALETTES {
                    if palette_chip(ui, name, colours).clicked() {
                        let colours: Vec<Colour> = colours.iter().take(livery.colours.len().max(2)).map(|&rgb| Colour::rgb(rgb)).collect();
                        editor.livery("palette", now, |livery| livery.colours = colours);
                    }
                }
            });
            ui.add_space(space::XS);
            let mut value = livery.scale;
            if slider_row(ui, n("PATTERN SIZE"), t("How big the pattern's shapes are"), &mut value, Kind::Scale) {
                editor.livery("scale", now, |livery| livery.scale = value);
            }
            let mut value = livery.sharpness;
            if slider_row(ui, n("SHARPNESS"), t("Soft or crisp edges between the colours"), &mut value, Kind::Share) {
                editor.livery("sharpness", now, |livery| livery.sharpness = value);
            }
            ui.horizontal(|ui| {
                if ui.add(ActionButton::secondary("SHUFFLE").height(28.0)).hover_tip(t("Lay the same pattern out another way")).clicked() {
                    let seed = livery.seed.wrapping_mul(1_103_515_245).wrapping_add(12_345 + (now * 1000.0) as u32) % 100_000;
                    editor.livery("seed", now, |livery| livery.seed = seed);
                }
                ui.label(RichText::new(tf("Layout {n}", &[("n", &livery.seed)])).color(STEEL).size(12.0));
            });
        });
    }

    /// The livery's finish and weathering.
    fn skin_editor_finish(&mut self, ui: &mut Ui) {
        let now = ui.input(|input| input.time);
        let Some(editor) = self.skins.editor.as_mut() else { return };
        let Some(livery) = editor.manifest.livery.clone() else { return };
        widgets::card(ui, |ui| {
            widgets::card_title(ui, "FINISH AND WEAR");
            // (What it is painted on: TA Reborn's units, or TA's own as TA drew them.)
            let mut original = livery.original;
            let tip = t("Paint TA Reborn's units (rebuilt in detail, their paint redrawn) or TA's originals (their shapes and paint as TA drew them)");
            let caption = 150.0f32.min(ui.available_width() * 0.36);
            ui.horizontal(|ui| {
                let words = ui.allocate_ui_with_layout(vec2(caption, widgets::CONTROL_HEIGHT), Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(widgets::caption("UNITS")).truncate()).hover_tip(tip)
                }).inner;
                let response = widgets::segmented_fill(ui, &mut original, &[(false, n("TA REBORN'S")), (true, n("TA'S ORIGINALS"))]).labelled_by(words.id);
                response.hover_tip(tip);
            });
            if original != livery.original {
                editor.livery("original", now, |livery| livery.original = original);
            }
            let rows: [FinishRow; 7] = [
                (n("COVERAGE"), t("How much of the units' own paint it covers: none keeps their colours, worn as below").into(), livery.coverage, |l, v| l.coverage = v, "coverage"),
                (n("DETAIL"), t("How much of the paint's own panel lines and markings shows through").into(), livery.detail, |l, v| l.detail = v, "detail"),
                (n("GLOSS"), t("Matte or glossy").into(), livery.gloss, |l, v| l.gloss = v, "gloss"),
                (n("METAL"), t("Paint or bare metal").into(), livery.metal, |l, v| l.metal = v, "metal"),
                (n("WEAR"), t("Factory fresh or battle-worn: half is TA Reborn's own, more chips the paint to the steel").into(), livery.wear, |l, v| l.wear = v, "wear"),
                (n("GRIME"), t("Dirt in the creases, underneath and in streaks").into(), livery.grime, |l, v| l.grime = v, "grime"),
                (n("FADE"), t("Paint bleached by the sun: its colours washed out, its finish chalky").into(), livery.fade, |l, v| l.fade = v, "fade"),
            ];
            for (label, tip, value, set, control) in rows {
                let mut value = value;
                if slider_row(ui, label, &tip, &mut value, Kind::Share) {
                    editor.livery(control, now, |livery| set(livery, value));
                }
            }
        });
    }

    /// Pictures of one's own on the units' armour: every armour surface, plain-coloured armour,
    /// and each texture of the units shown, for every unit or for one.
    fn skin_editor_slots(&mut self, ui: &mut Ui) {
        let now = ui.input(|input| input.time);
        let ctx = ui.ctx().clone();
        let idle = !self.skins.busy();
        let slots = self.skins.browser.preview.slots();
        let units: Vec<(String, String)> = slots.iter().map(|slot| (slot.unit.clone(), slot.display.clone())).collect();
        let Some(editor) = self.skins.editor.as_mut() else { return };
        let mut assign = None;
        widgets::card(ui, |ui| {
            widgets::card_title(ui, "TEXTURE SLOTS");
            super::wrapped(ui, t("Put a picture of your own on the units' armour. It paints over the livery; team colours, lights and tracks keep theirs."), STEEL);
            // For every unit, or for one of those shown.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                ui.label(widgets::caption("FOR"));
                if ui.selectable_label(editor.scope.is_none(), t("Every unit")).clicked() { editor.scope = None; }
                for (unit, display) in &units {
                    let chosen = editor.scope.as_deref().is_some_and(|scope| scope.eq_ignore_ascii_case(unit));
                    if ui.selectable_label(chosen, display).clicked() { editor.scope = Some(unit.clone()); }
                }
            });
            let scope = editor.scope.clone();
            let painted = editor.manifest.livery.is_some();
            let mut rows: Vec<(String, String, Option<TextureHandle>)> = vec![
                (EVERY.to_string(), t("Every armour surface").to_string(), None),
                (PLAIN.to_string(), t("Plain-coloured armour").to_string(), None),
            ];
            for slot in slots.iter().filter(|slot| scope.as_deref().is_none_or(|unit| slot.unit.eq_ignore_ascii_case(unit))) {
                for (texture, picture) in &slot.textures {
                    if !rows.iter().any(|(name, _, _)| name.eq_ignore_ascii_case(texture)) { rows.push((texture.clone(), texture.clone(), Some(picture.clone()))); }
                }
            }
            if slots.is_empty() {
                super::wrapped(ui, t("The units' own textures are listed once the preview has loaded them."), STEEL);
            }
            let serial = editor.serial;
            self.skins.slot_pictures.retain(|(opened, _), _| *opened == serial);
            for (texture, label, original) in rows {
                let mapped = editor.mapping(scope.as_deref(), &texture).map(str::to_string);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = space::S;
                    let (rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                    let shown = mapped.as_ref().and_then(|path| slot_picture(&ctx, &mut self.skins.slot_pictures, editor.image(path), serial, path)).or(original);
                    match shown {
                        Some(texture) => { ui.painter().image(texture.id(), rect.shrink(2.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
                        None => { ui.painter().rect_filled(rect.shrink(2.0), 2.0, Color32::from_gray(36)); }
                    }
                    ui.painter().rect_stroke(rect.shrink(2.0), 2.0, Stroke::new(1.0, Color32::from_black_alpha(140)), StrokeKind::Inside);
                    let words = ui.available_width() - 210.0;
                    ui.allocate_ui_with_layout(vec2(words.max(80.0), 28.0), Layout::top_down(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add(egui::Label::new(RichText::new(&label).color(TEXT)).truncate());
                        let unpainted = if painted { t("Livery") } else { t("Its own paint") };
                        let state = mapped.as_deref().map_or_else(|| unpainted.to_string(), |path| path.rsplit('/').next().unwrap_or(path).to_string());
                        ui.add(egui::Label::new(RichText::new(state).size(11.5).color(if mapped.is_some() { AMBER } else { STEEL })).truncate());
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if mapped.is_some() && ui.add(ActionButton::secondary("CLEAR").height(26.0)).hover_tip(t("Take the picture away")).clicked() {
                            editor.clear(scope.as_deref(), &texture, now);
                        }
                        if ui.add_enabled(idle, ActionButton::secondary("ASSIGN IMAGE…").height(26.0)).hover_tip(t("A PNG or JPEG of your own: it is made square and opaque to fit")).clicked() {
                            assign = Some((scope.clone(), texture.clone()));
                        }
                    });
                });
            }
        });
        if let Some((scope, texture)) = assign { self.push(Action::AssignSkinImage { scope, texture }); }
    }
}

/// A finish slider: its caption, its tip, its value, how it sets it, and its name to undo by.
type FinishRow = (&'static str, String, f32, fn(&mut Livery, f32), &'static str);

/// How a slider shows its value.
#[derive(Clone, Copy)]
enum Kind { Scale, Share }

/// A slider in a row: its caption at the left, its value at the right. True when it changed.
fn slider_row(ui: &mut Ui, label: &str, tip: &str, value: &mut f32, kind: Kind) -> bool {
    let caption = 150.0f32.min(ui.available_width() * 0.36);
    let figure = widgets::value_box_width(ui, &["100%", "4.00×"]);
    ui.horizontal(|ui| {
        let words = ui.allocate_ui_with_layout(vec2(caption, widgets::CONTROL_HEIGHT), Layout::left_to_right(Align::Center), |ui| {
            ui.add(egui::Label::new(widgets::caption(label)).truncate()).hover_tip(tip)
        }).inner;
        let room = ui.available_width();
        let response = ui.allocate_ui(vec2(room, widgets::CONTROL_HEIGHT), |ui| match kind {
            Kind::Scale => widgets::value_slider(ui, egui::Slider::new(value, *ta_skins::SCALE.start()..=*ta_skins::SCALE.end()).logarithmic(true).fixed_decimals(2).suffix("×"), figure),
            Kind::Share => {
                let mut percent = (*value * 100.0).round();
                let response = widgets::value_slider(ui, egui::Slider::new(&mut percent, 0.0..=100.0).fixed_decimals(0).suffix("%"), figure);
                if response.changed() { *value = (percent / 100.0).clamp(0.0, 1.0); }
                response
            }
        }).inner.labelled_by(words.id);
        response.hover_tip(tip).changed()
    }).inner
}

/// A pattern's tile: its picture in the livery's colours over its name; the chosen one framed.
fn pattern_tile(ui: &mut Ui, texture: &TextureHandle, pattern: Pattern, chosen: bool) -> egui::Response {
    let label = t(super::skins::pattern_label(pattern));
    let (rect, response) = ui.allocate_exact_size(TILE + vec2(0.0, 18.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, chosen, label));
    widgets::reveal_focus(ui, &response);
    if ui.is_rect_visible(rect) {
        let picture = Rect::from_min_size(rect.min, TILE);
        let painter = ui.painter();
        painter.add(ta_ui::metal::well(picture, Color32::from_rgb(9, 10, 8)));
        painter.image(texture.id(), picture.shrink(2.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        let colour = if chosen { AMBER_BRIGHT } else if response.hovered() { TEXT_BRIGHT } else { TEXT };
        painter.text(pos2(rect.center().x, picture.bottom() + 9.0), egui::Align2::CENTER_CENTER, label, body(11.5), colour);
        if chosen {
            painter.rect_stroke(picture, 3.0, Stroke::new(2.0, AMBER), StrokeKind::Outside);
        } else if response.hovered() {
            painter.rect_stroke(picture, 3.0, Stroke::new(1.0, AMBER_DIM), StrokeKind::Outside);
        }
        widgets::focus_ring(ui, &response, rect);
    }
    if response.clicked() && !chosen { ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle); }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The pattern tiles in this livery's colours, drawn again only when what they show changes.
fn pattern_tiles(ctx: &egui::Context, cache: &mut Option<(String, Vec<TextureHandle>)>, livery: &Livery) -> Vec<TextureHandle> {
    let key = format!("{:?}|{}|{}|{}|{}", livery.colours, livery.scale, livery.sharpness, livery.seed, livery.colours.len());
    if let Some((made, textures)) = cache.as_ref() && *made == key { return textures.clone(); }
    let (width, height) = ((TILE.x * 2.0) as usize, (TILE.y * 2.0) as usize);
    let textures: Vec<TextureHandle> = Pattern::ALL.into_iter().map(|pattern| {
        let tile = crate::skin_patterns::tile(livery, pattern, width, height);
        ctx.load_texture(format!("skin pattern {pattern:?}"), tile, egui::TextureOptions::LINEAR)
    }).collect();
    *cache = Some((key, textures.clone()));
    textures
}

/// A picture assigned to a slot, as a small texture (made once per image of this opening of
/// the editor, `serial`).
fn slot_picture(ctx: &egui::Context, cache: &mut std::collections::HashMap<(u64, String), TextureHandle>, image: Option<&image::RgbaImage>, serial: u64, path: &str) -> Option<TextureHandle> {
    let key = (serial, path.to_string());
    if let Some(texture) = cache.get(&key) { return Some(texture.clone()); }
    let image = image?;
    let small = image::imageops::thumbnail(image, 32, 32);
    let picture = egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw());
    let texture = ctx.load_texture(format!("skin slot {serial} {path}"), picture, egui::TextureOptions::LINEAR);
    cache.insert(key, texture.clone());
    Some(texture)
}

/// A colour's swatch: its colour over its name; a click opens its picker, its hex and the
/// palettes' colours. True when the colour changed.
fn colour_swatch(ui: &mut Ui, swatch: Swatch, label: &str, colour: &mut Colour) -> bool {
    let id = ui.id().with(("skin_swatch", match swatch { Swatch::Colour(at) => at, Swatch::Trim => 9 }));
    let (rect, response) = ui.allocate_exact_size(vec2(64.0, 46.0), Sense::click());
    let [r, g, b] = colour.0;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, true, format!("{label}: {}", colour.hex())));
    widgets::reveal_focus(ui, &response);
    if ui.is_rect_visible(rect) {
        let chip = Rect::from_min_size(rect.min, vec2(rect.width(), 28.0));
        let painter = ui.painter();
        painter.rect_filled(chip, 3.0, Color32::from_rgb(r, g, b));
        painter.rect_stroke(chip, 3.0, Stroke::new(if response.hovered() { 2.0 } else { 1.0 }, if response.hovered() { AMBER } else { Color32::from_black_alpha(160) }), StrokeKind::Inside);
        painter.text(pos2(rect.center().x, chip.bottom() + 9.0), egui::Align2::CENTER_CENTER, label, body(11.5), TEXT);
        widgets::focus_ring(ui, &response, rect);
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).hover_tip(colour.hex());
    let mut changed = false;
    egui::Popup::menu(&response).id(id).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_width(250.0);
        let mut picked = Color32::from_rgb(r, g, b);
        if egui::widgets::color_picker::color_picker_color32(ui, &mut picked, egui::widgets::color_picker::Alpha::Opaque) {
            *colour = Colour([picked.r(), picked.g(), picked.b()]);
            changed = true;
        }
        ui.horizontal(|ui| {
            ui.label(widgets::caption("HEX"));
            let mut hex = colour.hex();
            let field = ui.add(egui::TextEdit::singleline(&mut hex).desired_width(90.0).font(mono(13.0)));
            if field.changed() && let Some(parsed) = Colour::parse(&hex) {
                *colour = parsed;
                changed = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            let mut seen = Vec::new();
            for &(_, colours) in &PALETTES {
                for &rgb in colours {
                    if seen.contains(&rgb) { continue; }
                    seen.push(rgb);
                    let shade = Colour::rgb(rgb);
                    let [r, g, b] = shade.0;
                    let (chip, click) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                    ui.painter().rect_filled(chip, 2.0, Color32::from_rgb(r, g, b));
                    ui.painter().rect_stroke(chip, 2.0, Stroke::new(1.0, if click.hovered() { AMBER } else { Color32::from_black_alpha(160) }), StrokeKind::Inside);
                    if click.on_hover_text(shade.hex()).clicked() { *colour = shade; changed = true; }
                }
            }
        });
    });
    changed
}

/// A palette's chip: its colours side by side, its name under them.
fn palette_chip(ui: &mut Ui, name: &str, colours: &[u32]) -> egui::Response {
    let label = t(name);
    let width = (widgets::galley(ui, RichText::new(label).size(11.0)).size().x + 12.0).max(66.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 38.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    widgets::reveal_focus(ui, &response);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let strip = Rect::from_min_size(rect.min, vec2(rect.width(), 18.0));
        let each = strip.width() / colours.len() as f32;
        for (at, &rgb) in colours.iter().enumerate() {
            let [r, g, b] = Colour::rgb(rgb).0;
            painter.rect_filled(Rect::from_min_size(strip.min + vec2(at as f32 * each, 0.0), vec2(each, strip.height())), 0.0, Color32::from_rgb(r, g, b));
        }
        painter.rect_stroke(strip, 2.0, Stroke::new(1.0, if response.hovered() { AMBER } else { Color32::from_black_alpha(160) }), StrokeKind::Inside);
        painter.text(pos2(rect.center().x, strip.bottom() + 9.0), egui::Align2::CENTER_CENTER, label, body(11.0), if response.hovered() { TEXT_BRIGHT } else { STEEL });
        widgets::focus_ring(ui, &response, rect);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).hover_tip(t("Use these colours"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn render(ctx: &egui::Context, app: &mut LauncherApp, width: f32, height: f32, events: Vec<egui::Event>) -> String {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))), events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| app.skins_page(ui));
        output.textures_delta.clear();
        fn text(shape: &egui::Shape, into: &mut String) {
            match shape { egui::Shape::Text(t) => { into.push_str(&t.galley.job.text); into.push('\n'); },
                egui::Shape::Vec(items) => for item in items { text(item, into); }, _ => {} }
        }
        let mut labels = String::new();
        for shape in &output.shapes { text(&shape.shape, &mut labels); }
        labels
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    /// A swatch opens its picker, with the colour's hex to type over.
    #[test]
    fn a_swatch_opens_its_picker_with_its_hex() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut colour = Colour::rgb(0x4f5d34);
        let mut changed = false;
        let frame = |ctx: &egui::Context, events: Vec<egui::Event>, colour: &mut Colour, changed: &mut bool| {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 600.0))), events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| { *changed |= colour_swatch(ui, Swatch::Colour(0), "Ground", colour); });
            output.textures_delta.clear();
            let mut words = String::new();
            fn text(shape: &egui::Shape, into: &mut String) {
                match shape { egui::Shape::Text(t) => { into.push_str(&t.galley.job.text); into.push('\n'); },
                    egui::Shape::Vec(items) => for item in items { text(item, into); }, _ => {} }
            }
            for shape in &output.shapes { text(&shape.shape, &mut words); }
            words
        };
        frame(&ctx, Vec::new(), &mut colour, &mut changed);
        let at = egui::pos2(20.0, 12.0);
        let click = vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ];
        frame(&ctx, click, &mut colour, &mut changed);
        let words = frame(&ctx, Vec::new(), &mut colour, &mut changed);
        assert!(words.contains("HEX") && words.contains("#4f5d34"), "the picker opens with the colour's hex: {words}");
        assert!(!changed, "opening the picker changes nothing");
        // (Its palette's colours, a click each.)
        assert!(egui::Popup::is_any_open(&ctx));
    }

    #[test]
    fn the_editor_shows_its_paint_undoes_and_asks_before_losing_changes() {
        let folder = std::env::temp_dir().join(format!("ta_skin_editor_page_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let config = folder.join("config.toml");
        let ctx = egui::Context::default();
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        app.skins.editor = Some(crate::skin_editor::Editor::open(ta_skins::load("builtin:desert", &folder).unwrap(), None));
        // (Text below the fold is not painted: tall enough to see it all.)
        for (width, height) in [(1400.0, 1400.0), (720.0, 2600.0)] {
            let labels = render(&ctx, &mut app, width, height, Vec::new());
            for words in ["PATTERN", "Blotches", "Hex plates", "COLOURS", "PALETTES", "PATTERN SIZE", "ALL SKINS", "Not saved yet"] {
                assert!(labels.contains(words), "{words} at {width}: {labels}");
            }
        }
        let labels = render(&ctx, &mut app, 1400.0, 2400.0, Vec::new());
        assert!(labels.contains("WEAR") && labels.contains("Every armour surface") && labels.contains("Plain-coloured armour"));
        // An edit, undone with Ctrl+Z, redone with Ctrl+Y.
        let editor = app.skins.editor.as_mut().unwrap();
        editor.livery("wear", 0.0, |livery| livery.wear = 1.0);
        app.show_shortcuts = true;
        render(&ctx, &mut app, 1400.0, 900.0, vec![key(egui::Key::Z, egui::Modifiers::COMMAND)]);
        assert_eq!(app.skins.editor.as_ref().unwrap().manifest.livery.as_ref().unwrap().wear, 1.0, "a modal keeps the editor's undo history untouched");
        app.show_shortcuts = false;
        render(&ctx, &mut app, 1400.0, 900.0, vec![key(egui::Key::Z, egui::Modifiers::COMMAND)]);
        assert_eq!(app.skins.editor.as_ref().unwrap().manifest.livery.as_ref().unwrap().wear, 0.6);
        render(&ctx, &mut app, 1400.0, 900.0, vec![key(egui::Key::Y, egui::Modifiers::COMMAND)]);
        assert_eq!(app.skins.editor.as_ref().unwrap().manifest.livery.as_ref().unwrap().wear, 1.0);
        // Leaving with changes asks first.
        render(&ctx, &mut app, 1400.0, 900.0, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
        assert_eq!(app.skins.ask, Some(Ask::Leave));
        assert!(app.skins.editor.is_some(), "nothing is lost before the answer");
        // The livery switched off: only the texture slots paint.
        app.skins.ask = None;
        app.skins.editor.as_mut().unwrap().set_livery_on(false, 10.0);
        let labels = render(&ctx, &mut app, 1400.0, 900.0, Vec::new());
        assert!(labels.contains("The units keep TA Reborn's own paint.") && !labels.contains("PATTERN SIZE"));
        drop(app);
        std::fs::remove_dir_all(folder).unwrap();
    }
}
