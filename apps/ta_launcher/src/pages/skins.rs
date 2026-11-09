//! The skins library: every skin (the built-ins and yours) as cards with pictures of units
//! wearing it (or a list's rows), to search and filter; the armies' skins above them; the chosen
//! skin's details beside them (in their place on a narrow page): its preview turning, the
//! armies it can dress and what can be done with it; a right-click on a card or a row (or
//! Shift+F10) opens what can be done with that skin. The editor takes the page
//! (`skin_editor.rs`).

use egui::{Align, Color32, Id, Layout, Rect, RichText, Sense, Ui, pos2, vec2};
use ta_skins::{Entry, Pattern};
use ta_ui::i18n::{n, t, tf};
use ta_ui::tooltip::TooltipExt as _;

use crate::app::{Action, LauncherApp};
use crate::library::menu::{self, Icon, Menu};
use crate::library::{self, Card};
use crate::skins::{Ask, Filter, View};
use crate::theme::*;
use crate::widgets::{self, ActionButton};

/// From this width the details stand beside the library; below it they take its place.
const SPLIT: f32 = 760.0;
/// A card's picture well, over its width.
const WELL: f32 = 0.6;

fn skin_menu_id() -> Id {
    Id::new("skin_library_menu")
}

/// What a skin's context menu does: open its details (in the library's place on a narrow page),
/// what its buttons do, or ask before removing it.
#[derive(Clone, Debug)]
enum SkinChoice {
    Details,
    Do(Action),
    Remove,
}

impl LauncherApp {
    pub(crate) fn skins_page(&mut self, ui: &mut Ui) {
        if self.skins.editor.is_some() {
            self.skin_editor_page(ui);
            return;
        }
        let short = ui.available_height() < 600.0;
        self.skins_header(ui, short);
        if let Some(status) = self.skins.status() { widgets::busy_line(ui, t(status)); }
        self.skins_feedback(ui);
        if !self.skins.catalog.entries.iter().any(|entry| entry.id == self.skins.browser.inspecting)
            && let Some(first) = self.skins.catalog.entries.first() { self.skins.browser.inspecting = first.id.clone(); }
        let wide = ui.available_width() >= SPLIT;
        // (A wide page shows the details beside the library: a skin opened there stays so.)
        if wide { self.skins.browser.details_open = false; }
        if !wide && self.skins.browser.details_open {
            let back = ta_ui::panel::back_button(ui, "ALL SKINS").clicked();
            let escape = self.search_shortcuts_allowed(ui) && ui.memory(|memory| memory.focused().is_none())
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            if back || escape {
                self.skins.browser.details_open = false;
                self.skins.browser.reveal = Some(self.skins.browser.inspecting.clone());
            }
            ui.add_space(space::S);
            self.skin_details(ui, ui.available_height());
            return;
        }
        self.skin_armies(ui);
        self.skins_toolbar(ui);
        let visible = self.matching_skins();
        let height = ui.available_height().max(1.0);
        if wide {
            let details = (ui.available_width() * 0.4).clamp(320.0, 460.0);
            let gap = space::L;
            let library = (ui.available_width() - details - gap).max(1.0);
            let top = ui.cursor().min;
            let left = Rect::from_min_size(top, vec2(library, height));
            let right = Rect::from_min_size(pos2(top.x + library + gap, top.y), vec2(details, height));
            ui.scope_builder(egui::UiBuilder::new().max_rect(left).layout(Layout::top_down(Align::Min)), |ui| self.skin_library(ui, &visible, height));
            ui.scope_builder(egui::UiBuilder::new().max_rect(right).layout(Layout::top_down(Align::Min)), |ui| self.skin_details(ui, height));
            ui.allocate_rect(left.union(right), Sense::hover());
        } else {
            self.skin_library(ui, &visible, height);
        }
        self.skin_menu(ui, wide);
    }

    /// The skin menu while it is open: built from the skin as it is now, what it says done.
    fn skin_menu(&mut self, ui: &Ui, wide: bool) {
        let ctx = ui.ctx().clone();
        let Some(id) = menu::target(&ctx, skin_menu_id()) else { return };
        let Some(entry) = self.skins.catalog.entries.iter().find(|entry| entry.id == id).cloned() else {
            menu::close(&ctx, skin_menu_id());
            return;
        };
        let entries = self.skin_menu_entries(&entry, wide);
        match menu::show(ui, skin_menu_id(), &entries) {
            Some(SkinChoice::Details) => self.skins.browser.details_open = true,
            Some(SkinChoice::Do(action)) => self.push(action),
            Some(SkinChoice::Remove) => self.skins.ask = Some(Ask::Delete(entry.id.clone())),
            None => {}
        }
    }

    /// What can be done with a skin, as its details offer it: dress your units or the enemy's in
    /// it; change it (a built-in's copy, or your own), share it; its folder; remove it (your own).
    /// What cannot apply to it is left out (a built-in has no folder and stays); what cannot be
    /// done now says why.
    fn skin_menu_entries(&self, entry: &Entry, wide: bool) -> Menu<SkinChoice> {
        let name = builtin_name(&entry.id).unwrap_or_else(|| entry.name.clone());
        let own = self.config.graphics.unit_skin == entry.id;
        let enemy = self.config.graphics.enemy_skin() == entry.id;
        let armies = match (own, enemy) { (true, true) => t("Both armies"), (true, false) => t("Yours"), (false, true) => t("Enemy"), _ => "" };
        let caption = [card_detail(entry).as_str(), armies].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ");
        let mut menu = Menu::new(name, caption);
        if !wide {
            menu.add(n("VIEW DETAILS"), Icon::Details, SkinChoice::Details).keys("Enter");
            menu.rule();
        }
        let busy = self.skins.busy().then(|| t("Wait for the skin being read or saved."));
        menu.add(n("USE FOR YOUR UNITS"), Icon::Lamp(GREEN, own), SkinChoice::Do(Action::SelectSkin(entry.id.clone()))).main()
            .unless(own.then(|| t("Your units wear it now.")).or(busy));
        menu.add(n("USE FOR ENEMIES"), Icon::Lamp(AMBER, enemy), SkinChoice::Do(Action::SelectEnemySkin(entry.id.clone())))
            .unless(enemy.then(|| t("The enemy's units wear it now.")).or(busy));
        menu.rule();
        if entry.builtin {
            menu.add(n("CUSTOMIZE"), Icon::Copy, SkinChoice::Do(Action::CustomizeSkin(entry.id.clone()))).unless(busy);
        } else {
            menu.add(n("EDIT"), Icon::Edit, SkinChoice::Do(Action::EditSkin(entry.id.clone()))).unless(busy);
        }
        menu.add(n("EXPORT…"), Icon::Export, SkinChoice::Do(Action::ExportSkin(entry.id.clone()))).unless(busy);
        if let Some(path) = &entry.path {
            menu.add(n("OPEN SKIN FOLDER"), Icon::Folder, SkinChoice::Do(Action::OpenPath(path.clone())));
        }
        if !entry.builtin {
            menu.rule();
            menu.add(n("REMOVE"), Icon::Remove, SkinChoice::Remove).danger().unless(busy);
        }
        menu
    }

    /// The page's title, and what it does to the library as a whole: import a skin, make a new
    /// one, look again.
    fn skins_header(&mut self, ui: &mut Ui, short: bool) {
        let idle = !self.skins.busy();
        let busy = t("Wait for the skin being read or saved.");
        super::library_header(ui, n("UNIT SKINS"), n("Browse your collection, preview the finish and outfit each army."), short,
            &["IMPORT SKIN…", "NEW SKIN", "RESCAN", "OPEN SKINS FOLDER"], |ui, height| {
            if ui.add_enabled(idle, ActionButton::primary("IMPORT SKIN…").height(height)).hover_tip(t("Add a skin someone shared (a .taskin file). You can also drop one on this page.")).disabled_tip(busy).clicked() {
                self.push(Action::BrowseSkin);
            }
            if ui.add_enabled(idle, ActionButton::secondary("NEW SKIN").height(height)).hover_tip(t("Paint a skin of your own in the editor")).disabled_tip(busy).clicked() {
                self.push(Action::NewSkin);
            }
            if ui.add_enabled(idle, ActionButton::secondary("RESCAN").height(height)).hover_tip(t("Look for new or changed skins (F5)")).disabled_tip(busy).clicked() {
                self.push(Action::RescanSkins);
            }
            if ui.add(ActionButton::secondary("OPEN SKINS FOLDER").height(height)).hover_tip(t("Where your skins are kept")).clicked() {
                self.push(Action::OpenFolder(self.store.dir().join("skins")));
            }
        });
    }

    /// What an import, an export or a save came to (until dismissed).
    fn skins_feedback(&mut self, ui: &mut Ui) {
        let Some(feedback) = &self.skins.feedback else { return };
        let (message, warning, path, error) = (feedback.message.clone(), feedback.warning.clone(), feedback.path.clone(), feedback.error);
        ui.horizontal_top(|ui| {
            let link = widgets::galley(ui, RichText::new(t("DISMISS")).font(strong(11.5))).size().x + 16.0;
            ui.allocate_ui_with_layout(vec2((ui.available_width() - link).max(1.0), 0.0), Layout::top_down(Align::Min), |ui| {
                super::wrapped(ui, &message, if error { RED } else { GREEN });
                if let Some(warning) = &warning { super::wrapped(ui, warning, AMBER); }
                if let Some(path) = &path && widgets::text_link(ui, "OPEN FOLDER").clicked() {
                    let folder = if path.is_dir() { path.clone() } else { path.parent().unwrap_or(path).to_path_buf() };
                    self.push(Action::OpenPath(folder));
                }
            });
            if widgets::text_link(ui, "DISMISS").clicked() { self.skins.feedback = None; }
        });
        ui.add_space(space::XS);
    }

    /// The armies' skins (click one to see it), and whether the enemy's follows yours.
    fn skin_armies(&mut self, ui: &mut Ui) {
        let sides = [("YOURS AND ALLIES", self.config.graphics.unit_skin.clone(), GREEN), ("ENEMY UNITS", self.config.graphics.enemy_skin().to_owned(), AMBER)];
        let mut inspect = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 6.0);
            for (title, id, colour) in &sides {
                let name = selected_name(&self.skins.catalog, id);
                let available = self.skins.catalog.entries.iter().any(|entry| entry.id == *id);
                ui.label(widgets::caption(title));
                let (rect, response) = ui.allocate_exact_size(vec2(widgets::galley(ui, RichText::new(&name).font(strong(13.5))).size().x + 28.0, 26.0), Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, available, format!("{}: {name}", t(title))));
                if ui.is_rect_visible(rect) {
                    let painter = ui.painter();
                    ta_ui::panel::row_state(painter, rect, false, response.hovered() && available);
                    // (Edged as a button: a click shows that skin.)
                    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, if response.hovered() && available { LINE_HI } else { LINE }), egui::StrokeKind::Inside);
                    painter.add(ta_ui::metal::lamp(pos2(rect.left() + 10.0, rect.center().y), 3.4, *colour, 1.0));
                    painter.text(pos2(rect.left() + 20.0, rect.center().y), egui::Align2::LEFT_CENTER, &name, strong(13.5), if available { TEXT_BRIGHT } else { STEEL });
                }
                widgets::focus_ring(ui, &response, rect);
                if available && response.hover_tip(t("Preview this skin")).clicked() { inspect = Some(id.clone()); }
                ui.add_space(space::S);
            }
            let mut linked = self.config.graphics.enemy_unit_skin.is_empty();
            let changed = ui.add_enabled_ui(!self.skins.busy(), |ui| {
                let mut changed = widgets::check_box_named(ui, &mut linked, "Use the same skin for both sides").changed();
                let label = ui.add(egui::Label::new(RichText::new(t("Use the same skin for both sides")).color(TEXT)).sense(Sense::click()));
                if label.clicked() { linked = !linked; changed = true; }
                changed
            }).inner;
            if changed {
                self.push(Action::SelectEnemySkin(if linked { String::new() } else { self.config.graphics.unit_skin.clone() }));
            }
        });
        if let Some(id) = inspect {
            self.skins.browser.inspecting = id.clone();
            self.skins.browser.reveal = Some(id);
        }
        if !self.skins.busy() && [self.config.graphics.unit_skin.as_str(), self.config.graphics.enemy_skin()].iter()
            .any(|id| !self.skins.catalog.entries.iter().any(|entry| entry.id == *id)) && !self.skins.catalog.entries.is_empty() {
            super::wrapped(ui, t("A selected skin is unavailable. That side will use Reborn until you choose an available skin."), AMBER);
        }
        ui.add_space(space::S);
    }

    /// The search, the filters and the view, and how many skins they leave.
    fn skins_toolbar(&mut self, ui: &mut Ui) {
        let shortcuts = self.search_shortcuts_allowed(ui);
        let width = ui.available_width();
        let filters = [(Filter::All, n("ALL")), (Filter::Builtin, n("BUILT IN")), (Filter::Custom, n("YOURS"))];
        let hint = t("Search skins or creators…");
        let shown = self.matching_skins().len();
        let total = self.skins.catalog.entries.len();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            let search = if width >= 720.0 { (width * 0.36).clamp(220.0, 360.0) } else { width };
            library::search(ui, &mut self.skins.browser.query, skin_search_id(), hint, search, shortcuts);
            widgets::segmented(ui, &mut self.skins.browser.filter, &filters);
            widgets::segmented(ui, &mut self.skins.browser.view, &[(View::Grid, n("CARDS")), (View::List, n("LIST"))]);
            ui.add(egui::Label::new(RichText::new(tf("{n} of {total} skins", &[("n", &shown), ("total", &total)])).color(STEEL).size(12.0)).extend());
        });
        ui.add_space(space::S);
    }

    pub(super) fn matching_skins(&self) -> Vec<usize> {
        let query = self.skins.browser.query.trim().to_lowercase();
        self.skins.catalog.entries.iter().enumerate().filter_map(|(index, entry)| {
            let translated_match = entry.builtin && builtin_name(&entry.id).is_some_and(|name| name.to_lowercase().contains(&query));
            (crate::skins::matches(entry, self.skins.browser.filter, &query)
                || (translated_match && self.skins.browser.filter != Filter::Custom)).then_some(index)
        }).collect()
    }

    /// What the preview and the thumbnails are made from: the game and its mods.
    pub(crate) fn skin_reference(&self) -> crate::skins::Reference {
        crate::skins::Reference {
            game_dir: self.detection.install().map(|install| install.root.clone()),
            enabled: self.config.mods.enabled.clone(),
            available: self.mods.mods.iter().map(|m| m.info.clone()).collect(),
        }
    }

    /// The skins on show, as cards or as the rows of a list.
    fn skin_library(&mut self, ui: &mut Ui, visible: &[usize], height: f32) {
        if visible.is_empty() {
            ui.add_space(space::L);
            super::wrapped(ui, t("No skins match this search. Try another name or clear the filters."), STEEL);
            if widgets::text_link(ui, "CLEAR FILTERS").clicked() {
                self.skins.browser.query.clear();
                self.skins.browser.filter = Filter::All;
            }
            return;
        }
        let reveal = self.skins.browser.reveal.take().and_then(|id| visible.iter().position(|index| self.skins.catalog.entries[*index].id == id));
        let width = library::grid_width(ui.available_width());
        let view = self.skins.browser.view;
        let layout = match view {
            View::Grid => library::layout(width, 150.0, WELL, 50.0, 12.0),
            View::List => library::rows(width, 58.0, 0.0),
        };
        let well = ((layout.card.x - 12.0) * WELL).round();
        let reference = self.skin_reference();
        let (root, cache, revision) = (self.store.dir().join("skins"), library::cache_dir(&self.store.dir(), "skins"), self.skins.preview_revision);
        let ctx = ui.ctx().clone();
        self.skins.browser.preview.poll_thumbnails(&ctx, revision);
        let (own, enemy) = (self.config.graphics.unit_skin.clone(), self.config.graphics.enemy_skin().to_owned());
        let entries = &self.skins.catalog.entries;
        let browser = &mut self.skins.browser;
        let (mut chosen, mut opened, mut focused, mut asked) = (None, false, None, None);
        library::grid(ui, Id::new("skin_library_grid"), visible.len(), layout, height, reveal, |ui, at, rect| {
            let entry = &entries[visible[at]];
            let name = builtin_name(&entry.id).unwrap_or_else(|| entry.name.clone());
            let detail = card_detail(entry);
            let lamps: Vec<Color32> = [(own == entry.id, GREEN), (enemy == entry.id, AMBER)].into_iter().filter_map(|(on, colour)| on.then_some(colour)).collect();
            let armies = match (own == entry.id, enemy == entry.id) {
                (true, true) => t("Both armies"), (true, false) => t("Yours"), (false, true) => t("Enemy"), _ => "",
            };
            let spoken = [name.as_str(), detail.as_str(), armies].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(", ");
            let card = Card { title: &name, detail: &detail, lamps: &lamps, star: false, selected: entry.id == browser.inspecting, spoken: &spoken, ribbon: None };
            let id = Id::new(("skin_card", &entry.id));
            let (picture, failed) = if ui.is_rect_visible(rect) {
                browser.preview.thumbnail(&ctx, &entry.id, revision, root.clone(), &reference, cache.clone())
            } else { (None, false) };
            let paint = |ui: &Ui, frame: Rect| library::picture(ui, frame, picture.as_ref(), failed);
            let response = match view {
                View::Grid => library::card(ui, id, rect, well, card, paint),
                View::List => library::picture_row(ui, id, rect, 1.5, card, paint),
            };
            if let Some((at, keys)) = menu::asked(ui, &response) {
                asked = Some((entry.id.clone(), at, keys.then_some(response.id)));
            }
            if response.clicked() { chosen = Some(entry.id.clone()); opened = true; }
            if response.has_focus() {
                // (egui moves the keys between cards at the end of a frame and reports no
                // gained focus for it: the card that has them is followed instead.)
                if browser.focused.as_ref() != Some(&entry.id) { response.scroll_to_me(None); }
                focused = Some(entry.id.clone());
                if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) { opened = true; }
            }
            let tip = if armies.is_empty() { format!("{name}\n{detail}") } else { format!("{name}\n{detail} · {armies}") };
            response.hover_tip(tip)
        });
        if focused != browser.focused {
            if let Some(id) = &focused { browser.inspecting = id.clone(); }
            browser.focused = focused;
        }
        if let Some(id) = chosen { browser.inspecting = id; }
        if opened { browser.details_open = true; }
        // A right-click (or Shift+F10) chooses the skin and opens what can be done with it.
        if let Some((id, at, from)) = asked {
            browser.inspecting = id.clone();
            menu::open(ui.ctx(), skin_menu_id(), &id, at, from);
        }
    }

    /// The chosen skin: its name and armies, its preview turning, its words, what it is made of
    /// and what can be done with it.
    fn skin_details(&mut self, ui: &mut Ui, height: f32) {
        let Some(entry) = self.skins.catalog.entries.iter().find(|entry| entry.id == self.skins.browser.inspecting).cloned() else {
            if !self.skins.catalog.entries.is_empty() { super::wrapped(ui, t("Choose a skin to see it here."), STEEL); }
            return;
        };
        let mut area = egui::ScrollArea::vertical().id_salt(("skin_details", &entry.id)).max_height(height.max(1.0)).min_scrolled_height(0.0).auto_shrink([false, false]);
        if let Some(offset) = self.capture_scroll { area = area.vertical_scroll_offset(offset); }
        ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Backdrop(BG), |ui| widgets::card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            self.skin_heading(ui, &entry);
            ui.add_space(space::XS);
            let reference = self.skin_reference();
            let preview_height = (height * 0.5).clamp(170.0, 320.0);
            // (What units it is shown on, over the picture: a tank by default, as on the cards.)
            let options: Vec<_> = crate::skins::Subjects::ALL.iter().map(|s| (*s, s.label())).collect();
            widgets::segmented_fill(ui, &mut self.skins.browser.subjects, &options).hover_tip(t("Show the skin on other units"));
            let units = self.skins.browser.subjects.units();
            self.skins.browser.preview.show(ui, reference, self.store.dir().join("skins"), crate::skins::preview::Paint::Saved(entry.id.clone()), self.skins.preview_revision, &units, preview_height);
            ui.add_space(space::S);
            super::wrapped(ui, &description(&entry), TEXT);
            self.skin_facts(ui, &entry);
            ui.add_space(space::S);
            self.skin_actions(ui, &entry);
            if !self.skins.catalog.errors.is_empty() {
                ui.add_space(space::S);
                egui::CollapsingHeader::new(RichText::new(tf("{n} skins need attention", &[("n", &self.skins.catalog.errors.len())])).color(RED)).show(ui, |ui| {
                    for error in &self.skins.catalog.errors { super::wrapped(ui, error, RED); }
                });
            }
            ui.add(egui::Label::new(RichText::new(t("Applies when you next launch the game or benchmark.")).small().color(STEEL)).wrap());
        }));
    }

    /// The details' title, whose it is, and the armies it can dress.
    fn skin_heading(&mut self, ui: &mut Ui, entry: &Entry) {
        let own = self.config.graphics.unit_skin == entry.id;
        let enemy = self.config.graphics.enemy_skin() == entry.id;
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::Label::new(RichText::new(builtin_name(&entry.id).unwrap_or_else(|| entry.name.clone())).font(display(24.0)).color(TEXT_BRIGHT)).wrap());
            if entry.builtin { widgets::tag(ui, "BUILT IN", STEEL); }
        });
        if !entry.builtin && !entry.author.is_empty() { ui.label(RichText::new(tf("By {author}", &[("author", &entry.author)])).small().color(STEEL)); }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            // (What it is used for now: a badge, not a box to tick.)
            if own {
                widgets::tag(ui, "USED FOR YOUR UNITS", GREEN);
            } else if ui.add_enabled(!self.skins.busy(), ActionButton::primary("USE FOR YOUR UNITS")).clicked() {
                self.push(Action::SelectSkin(entry.id.clone()));
            }
            if enemy {
                widgets::tag(ui, "USED FOR ENEMIES", AMBER);
            } else if ui.add_enabled(!self.skins.busy(), ActionButton::secondary("USE FOR ENEMIES")).clicked() {
                self.push(Action::SelectEnemySkin(entry.id.clone()));
            }
        });
    }

    /// What a skin is made of: its livery's pattern and colours, its own images.
    fn skin_facts(&self, ui: &mut Ui, entry: &Entry) {
        let livery = entry.livery.as_ref();
        let pattern = match livery {
            Some(livery) if livery.coverage > 0.0 => t(pattern_label(livery.pattern)).to_string(),
            Some(livery) if livery.original => t("TA's originals").to_string(),
            Some(_) => t("TA's own colours").to_string(),
            None => t("None").to_string(),
        };
        let facts = [(n("LIVERY"), pattern), (n("IMAGES"), entry.texture_count.to_string()), (n("UNITS OF THEIR OWN"), entry.unit_count.to_string())];
        let width = ui.available_width();
        let columns = if width >= 330.0 { 3 } else { 2 };
        let gap = space::M;
        let cell = ((width - gap * (columns - 1) as f32) / columns as f32).floor().max(1.0);
        for row in facts.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (caption, value) in row { library::fact(ui, cell, caption, value); }
            });
        }
        if let Some(livery) = livery.filter(|livery| livery.coverage > 0.0) {
            swatches(ui, livery);
        }
    }

    /// What can be done with a skin: a built-in copied to make one's own, one's own edited,
    /// shared or removed.
    fn skin_actions(&mut self, ui: &mut Ui, entry: &Entry) {
        let idle = !self.skins.busy();
        let busy = t("Wait for the skin being read or saved.");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            if entry.builtin {
                if ui.add_enabled(idle, ActionButton::primary("CUSTOMIZE")).hover_tip(t("Make a copy of your own to change in the editor")).disabled_tip(busy).clicked() {
                    self.push(Action::CustomizeSkin(entry.id.clone()));
                }
            } else if ui.add_enabled(idle, ActionButton::primary("EDIT")).hover_tip(t("Change this skin in the editor")).disabled_tip(busy).clicked() {
                self.push(Action::EditSkin(entry.id.clone()));
            }
            if ui.add_enabled(idle, ActionButton::secondary("EXPORT…")).hover_tip(t("Save it as a .taskin file to share")).disabled_tip(busy).clicked() {
                self.push(Action::ExportSkin(entry.id.clone()));
            }
            if !entry.builtin && ui.add_enabled(idle, ActionButton::danger("REMOVE")).hover_tip(t("Move this skin to the Recycle Bin")).disabled_tip(busy).clicked() {
                self.skins.ask = Some(Ask::Delete(entry.id.clone()));
            }
        });
        if let Some(path) = &entry.path && widgets::text_link(ui, "OPEN SKIN FOLDER").hover_tip(path.display().to_string()).clicked() {
            self.push(Action::OpenPath(path.clone()));
        }
    }

    /// The Skins page's questions: remove a skin, leave the editor with changes, a new skin's name.
    pub(crate) fn skin_dialogs(&mut self, ctx: &egui::Context) {
        let Some(ask) = self.skins.ask.clone() else { return };
        use ta_ui::panel::{Exit, Panel};
        let width = (ctx.content_rect().width() - 80.0).clamp(1.0, 520.0);
        let max_height = (ctx.content_rect().height() - 24.0).max(1.0);
        match ask {
            Ask::Delete(id) => {
                let Some(entry) = self.skins.catalog.entries.iter().find(|entry| entry.id == id).cloned() else { self.skins.ask = None; return };
                let title = tf("REMOVE {name}?", &[("name", &ta_ui::i18n::upper(&entry.name))]);
                let mut remove = false;
                let response = ta_ui::panel::modal(ctx, "launcher_skin_delete", Panel::new(&title).exit(Exit::Close).width(width).max_height(max_height).accent(RED), true, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.add(egui::Label::new(RichText::new(t("It goes to the Recycle Bin: put it back from there if you change your mind. An army wearing it goes back to Reborn.")).color(TEXT)).wrap());
                    if let Some(path) = &entry.path { ui.add(egui::Label::new(RichText::new(path.display().to_string()).font(mono(12.0)).color(STEEL)).wrap()); }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        remove = ui.add(ActionButton::danger("REMOVE")).clicked();
                        if ui.add(ActionButton::secondary("KEEP")).clicked() { self.skins.ask = None; }
                    });
                });
                if remove {
                    self.skins.ask = None;
                    self.delete_skin(&entry);
                } else if response.exit {
                    self.skins.ask = None;
                }
            }
            Ask::Leave => {
                let name = self.skins.editor.as_ref().map_or(String::new(), |editor| editor.manifest.name.clone());
                let title = tf("SAVE {name}?", &[("name", &ta_ui::i18n::upper(&name))]);
                let (mut save, mut discard) = (false, false);
                let response = ta_ui::panel::modal(ctx, "launcher_skin_leave", Panel::new(&title).exit(Exit::Close).width(width).max_height(max_height).accent(AMBER), true, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.add(egui::Label::new(RichText::new(t("This skin has changes that are not saved.")).color(TEXT)).wrap());
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        save = ui.add_enabled(!self.skins.busy(), ActionButton::primary("SAVE")).clicked();
                        discard = ui.add(ActionButton::danger("DISCARD CHANGES")).clicked();
                        if ui.add(ActionButton::secondary("KEEP EDITING")).clicked() { self.skins.ask = None; }
                    });
                });
                if save {
                    self.skins.ask = None;
                    self.push(Action::SaveSkin);
                } else if discard {
                    self.skins.ask = None;
                    self.skins.editor = None;
                } else if response.exit {
                    self.skins.ask = None;
                }
            }
            Ask::SaveAs(mut name) => {
                let mut save = false;
                let response = ta_ui::panel::modal(ctx, "launcher_skin_save_as", Panel::new(t("SAVE AS A NEW SKIN")).exit(Exit::Close).width(width).max_height(max_height).accent(SKY), true, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.label(widgets::caption("SKIN NAME"));
                    let field = ui.add(egui::TextEdit::singleline(&mut name).char_limit(80).desired_width(ui.available_width()).hint_text(t("My unit skin")));
                    ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(t("SKIN NAME")));
                    if !field.has_focus() && !ui.memory(|memory| memory.has_focus(field.id)) && name.is_empty() { field.request_focus(); }
                    let enter = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        save = (ui.add_enabled(!name.trim().is_empty() && !self.skins.busy(), ActionButton::primary("SAVE")).clicked() || enter) && !name.trim().is_empty();
                        if ui.add(ActionButton::secondary("CANCEL")).clicked() { self.skins.ask = None; }
                    });
                });
                if save {
                    self.skins.ask = None;
                    self.push(Action::SaveSkinAs(name.trim().to_string()));
                } else if response.exit {
                    self.skins.ask = None;
                } else if self.skins.ask.is_some() {
                    self.skins.ask = Some(Ask::SaveAs(name));
                }
            }
        }
    }

    /// A skin of one's own to the Recycle Bin; an army wearing it back to Reborn.
    fn delete_skin(&mut self, entry: &Entry) {
        use ta_ui::recycle::Outcome;
        let Some(path) = entry.path.clone() else { return };
        if self.skins.busy() { return; }
        match ta_ui::recycle::recycle(&path) {
            Ok(Outcome::Kept) => self.notify(tf("{name} is left where it is", &[("name", &entry.name)]), AMBER),
            Ok(outcome) => {
                log::info!("removed skin {}: {} ({outcome:?})", entry.id, path.display());
                if self.config.graphics.unit_skin == entry.id { self.config.graphics.unit_skin = "reborn".into(); }
                if self.config.graphics.enemy_unit_skin == entry.id { self.config.graphics.enemy_unit_skin = "reborn".into(); }
                if self.skins.editor.as_ref().is_some_and(|editor| editor.id.as_deref() == Some(entry.id.as_str())) { self.skins.editor = None; }
                let text = match outcome {
                    Outcome::Recycled => tf("{name} is removed (it is in the Recycle Bin)", &[("name", &entry.name)]),
                    _ => tf("{name} is deleted (it could not go to the Recycle Bin)", &[("name", &entry.name)]),
                };
                self.notify(text, AMBER);
                self.rescan_skins();
            }
            Err(e) => {
                log::warn!("removing {}: {e}", path.display());
                self.notify(tf("Could not remove {name}: {error}", &[("name", &entry.name), ("error", &e)]), RED);
            }
        }
    }
}

pub(super) fn skin_search_id() -> egui::Id { egui::Id::new("launcher_skin_search") }

/// A card's line under its name: whose it is, or what its livery is.
fn card_detail(entry: &Entry) -> String {
    if entry.builtin {
        return match entry.livery.as_ref() {
            Some(livery) if livery.coverage > 0.0 => t(pattern_label(livery.pattern)).to_string(),
            Some(livery) if livery.original => t("TA as it was").to_string(),
            Some(_) => t("TA's own colours").to_string(),
            None => t("Rebuilt in detail").to_string(),
        };
    }
    if entry.author.is_empty() { t("Yours").to_string() } else { tf("By {author}", &[("author", &entry.author)]) }
}

/// A pattern's name, to translate.
pub(super) fn pattern_label(pattern: Pattern) -> &'static str {
    match pattern {
        Pattern::Solid => n("Solid"),
        Pattern::Blotch => n("Blotches"),
        Pattern::Splinter => n("Splinter"),
        Pattern::Digital => n("Digital"),
        Pattern::Stripes => n("Stripes"),
        Pattern::Hex => n("Hex plates"),
        Pattern::Festive => n("Festive"),
        Pattern::Tricolour => n("Tricolour"),
        Pattern::Union => n("Union"),
        Pattern::Stars => n("Stars"),
    }
}

fn description(entry: &Entry) -> String {
    if !entry.builtin { return entry.description.clone(); }
    match entry.id.as_str() {
        "reborn" => t("TA's designs rebuilt in detail: armour plates, bolts and road wheels on every unit, its paint redrawn crisp and clean.").into(),
        "builtin:heritage" => t("TA as it was: its own shapes and paint as TA drew them, battle-worn, chipped and grimy.").into(),
        "christmas" => t("Evergreen and festive red with snowflakes and gold trim.").into(),
        "builtin:halloween" => t("Witching purple and black splinters, trimmed in pumpkin orange.").into(),
        "builtin:desert" => t("Sand, ochre and earth blotches for desert operations.").into(),
        "builtin:winter" => t("Worn whitewash over grey for winter operations.").into(),
        "builtin:woodlands" => t("Hard-edged green, brown and black for woodland operations.").into(),
        "builtin:navy" => t("Splinters of haze and ocean grey, as a warship's dazzle paint.").into(),
        "builtin:urban" => t("Pixelated greys for fighting among the buildings.").into(),
        "builtin:midnight" => t("Dark gunmetal hex plating that disappears at night.").into(),
        "builtin:tiger" => t("Burnt orange slashed with black stripes.").into(),
        "builtin:ceremonial" => t("Parade white with polished gold edges.").into(),
        "builtin:uk" => t("Navy blue crossed with white and red, as the Union flag.").into(),
        "builtin:usa" => t("Stars and stripes in red, white and blue.").into(),
        "builtin:australia" => t("Green and gold, with stars of the Southern Cross.").into(),
        "builtin:new-zealand" => t("All black, trimmed in silver.").into(),
        "builtin:italy" => t("Il Tricolore: bands of green, white and red.").into(),
        "builtin:france" => t("Le Tricolore: bands of blue, white and red.").into(),
        "builtin:germany" => t("Bands of black, red and gold.").into(),
        _ => entry.description.clone(),
    }
}

/// A built-in skin's name in the interface's language.
pub(super) fn builtin_name(id: &str) -> Option<String> {
    Some(match id {
        "reborn" => t("Reborn"), "christmas" => t("Christmas"), "builtin:halloween" => t("Halloween"),
        "builtin:heritage" => t("Heritage"),
        "builtin:desert" => t("Desert"), "builtin:winter" => t("Winter"),
        "builtin:woodlands" => t("Woodlands"), "builtin:navy" => t("Navy"),
        "builtin:urban" => t("Urban"), "builtin:midnight" => t("Midnight"),
        "builtin:tiger" => t("Tiger"), "builtin:ceremonial" => t("Ceremonial"),
        "builtin:uk" => t("United Kingdom"), "builtin:usa" => t("United States"),
        "builtin:australia" => t("Australia"), "builtin:new-zealand" => t("New Zealand"),
        "builtin:italy" => t("Italy"), "builtin:france" => t("France"), "builtin:germany" => t("Germany"),
        _ => return None,
    }.to_string())
}

fn selected_name(catalog: &ta_skins::Catalog, id: &str) -> String {
    builtin_name(id).unwrap_or_else(|| catalog.entries.iter().find(|entry| entry.id == id).map_or_else(|| id.to_string(), |entry| entry.name.clone()))
}

/// A livery's colours (and its trim) as chips.
pub(super) fn swatches(ui: &mut Ui, livery: &ta_skins::Livery) {
    let colours: Vec<[u8; 3]> = livery.colours.iter().chain(&livery.trim).map(|colour| colour.0).collect();
    let (rect, _) = ui.allocate_exact_size(vec2(colours.len() as f32 * 30.0, 12.0), Sense::hover());
    for (i, [r, g, b]) in colours.into_iter().enumerate() {
        let chip = Rect::from_min_size(rect.min + vec2(i as f32 * 30.0, 0.0), vec2(26.0, 12.0));
        ui.painter().rect_filled(chip, 2.0, Color32::from_rgb(r, g, b));
        ui.painter().rect_stroke(chip, 2.0, egui::Stroke::new(1.0, Color32::from_black_alpha(120)), egui::StrokeKind::Inside);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn app(name: &str) -> (std::path::PathBuf, egui::Context, LauncherApp) {
        let folder = std::env::temp_dir().join(format!("ta_skin_page_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let config = folder.join("config.toml");
        let ctx = egui::Context::default();
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        (folder, ctx, app)
    }

    fn render(ctx: &egui::Context, app: &mut LauncherApp, width: f32, height: f32, events: Vec<egui::Event>) -> String {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))), events, ..Default::default() };
        let mut bounds = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(input, |ui| { app.skins_page(ui); bounds = ui.min_rect(); });
        output.textures_delta.clear();
        assert!(bounds.max.y <= height + 1.0, "the page must stay inside its viewport: {bounds:?}, height={height}");
        fn text(shape: &egui::Shape, into: &mut String) {
            match shape { egui::Shape::Text(t) => { into.push_str(&t.galley.job.text); into.push('\n'); },
                egui::Shape::Vec(items) => for item in items { text(item, into); }, _ => {} }
        }
        let mut labels = String::new();
        for shape in &output.shapes { text(&shape.shape, &mut labels); }
        labels
    }

    #[test]
    fn the_library_shows_cards_and_details_and_search_never_assigns() {
        let (folder, ctx, mut app) = app("library");
        app.skins.catalog = ta_skins::list(&folder.join("skins"));
        app.config.graphics.unit_skin = "builtin:desert".into();
        app.config.graphics.enemy_unit_skin = "builtin:navy".into();
        app.skins.browser.inspecting = "builtin:tiger".into();
        let labels = render(&ctx, &mut app, 1280.0, 1600.0, Vec::new());
        assert!(labels.contains("Woodlands") && labels.contains("Ceremonial"), "cards for the built-ins");
        assert!(labels.contains("Burnt orange slashed with black stripes."), "the chosen skin's details beside them");
        assert!(labels.contains("CUSTOMIZE") && !labels.contains("DELETE"), "a built-in is copied, not deleted");
        assert!(labels.contains("Rebuilt in detail") && labels.contains("TA as it was"), "Reborn and Heritage say how they differ");
        assert!(labels.contains("COMMANDERS") && labels.contains("AIRCRAFT"), "the preview's units to choose");
        app.skins.browser.query = "tiger".into();
        let labels = render(&ctx, &mut app, 1280.0, 800.0, Vec::new());
        assert!(labels.contains("1 of 19 skins"));
        assert!(!labels.contains("Woodlands\n"));
        assert_eq!(app.config.graphics.unit_skin, "builtin:desert");
        assert_eq!(app.config.graphics.enemy_skin(), "builtin:navy");
        // A narrow page: the cards, then the details in their place, and back.
        app.skins.browser.query.clear();
        app.skins.browser.details_open = true;
        let labels = render(&ctx, &mut app, 600.0, 760.0, Vec::new());
        assert!(labels.contains("ALL SKINS") && labels.contains("CUSTOMIZE"));
        let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        render(&ctx, &mut app, 600.0, 760.0, vec![escape]);
        assert!(!app.skins.browser.details_open);
        drop(app);
        std::fs::remove_dir_all(folder).unwrap();
    }

    /// A skin's menu, from a right-click or Shift+F10 on its card: what its details offer (to
    /// dress either army, customize, export), what cannot be done now said so (the army that
    /// wears it already), Escape closing it with the keys back on the card, a choice asked for.
    #[test]
    fn a_skins_menu_offers_its_actions_by_pointer_and_keys() {
        let (folder, ctx, mut app) = app("menu");
        app.skins.catalog = ta_skins::list(&folder.join("skins"));
        app.config.graphics.unit_skin = "builtin:desert".into();
        app.config.graphics.enemy_unit_skin = "builtin:navy".into();
        app.skins.browser.inspecting = "builtin:tiger".into();
        let frame = |app: &mut LauncherApp, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 1600.0))), events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| app.skins_page(ui));
            output.textures_delta.clear();
            let mut texts = Vec::new();
            fn walk(shape: &egui::Shape, into: &mut Vec<(String, Rect)>) {
                match shape {
                    egui::Shape::Text(text) => into.push((text.galley.text().to_string(), text.galley.rect.translate(text.pos.to_vec2()))),
                    egui::Shape::Vec(shapes) => for shape in shapes { walk(shape, into); },
                    _ => {}
                }
            }
            for shape in &output.shapes { walk(&shape.shape, &mut texts); }
            texts
        };
        let find = |texts: &[(String, Rect)], label: &str| texts.iter().find(|(text, _)| text == label).map(|(_, rect)| *rect);
        frame(&mut app, Vec::new());
        frame(&mut app, Vec::new());
        let desert = ctx.read_response(Id::new(("skin_card", "builtin:desert"))).expect("the Desert card").rect;
        let at = desert.center();
        frame(&mut app, vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() }]);
        frame(&mut app, vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() }]);
        let shown = frame(&mut app, Vec::new());
        assert_eq!(app.skins.browser.inspecting, "builtin:desert", "a right-click chooses it");
        for words in ["USE FOR YOUR UNITS", "Your units wear it now.", "USE FOR ENEMIES", "CUSTOMIZE", "EXPORT…"] {
            assert!(find(&shown, words).is_some(), "{words} in its menu");
        }
        assert!(find(&shown, "REMOVE").is_none(), "a built-in is never removed: nothing to offer");
        let enemies = find(&shown, "USE FOR ENEMIES").unwrap().center();
        frame(&mut app, vec![egui::Event::PointerMoved(enemies), egui::Event::PointerButton { pos: enemies, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
        frame(&mut app, vec![egui::Event::PointerButton { pos: enemies, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
        assert!(matches!(app.actions.as_slice(), [Action::SelectEnemySkin(id)] if id == "builtin:desert"), "{:?}", app.actions);
        app.actions.clear();
        // With the keys: Shift+F10 on the card with them; Escape gives them back.
        let key = |key, modifiers| vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
        let card = Id::new(("skin_card", "builtin:tiger"));
        frame(&mut app, vec![egui::Event::PointerGone]);
        ctx.memory_mut(|m| m.request_focus(card));
        frame(&mut app, Vec::new());
        frame(&mut app, key(egui::Key::F10, egui::Modifiers::SHIFT));
        let shown = frame(&mut app, Vec::new());
        assert_eq!(app.skins.browser.inspecting, "builtin:tiger");
        assert!(find(&shown, "CUSTOMIZE").is_some(), "the menu opens on the card with the keys");
        assert!(ctx.memory(|m| m.focused()).is_some_and(|focused| focused != card), "its first entry takes the keys");
        frame(&mut app, key(egui::Key::Escape, egui::Modifiers::NONE));
        frame(&mut app, Vec::new());
        frame(&mut app, Vec::new());
        assert_eq!(ctx.memory(|m| m.focused()), Some(card), "Escape closes it, the keys back on the card");
        assert!(app.actions.is_empty() && app.skins.browser.details_open == false);
        drop(app);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_large_library_draws_only_the_cards_in_view() {
        let (folder, ctx, mut app) = app("large");
        app.skins.catalog.entries = (0..2000).map(|i| Entry { id: format!("skin-{i:04}"), name: format!("Skin {i:04}"),
            author: format!("Maker {i:04}"), description: "Custom paint".into(), builtin: false, path: None, livery: None, texture_count: 1, unit_count: 0 }).collect();
        app.config.graphics.unit_skin = "skin-0000".into();
        app.skins.browser.inspecting = "skin-0000".into();
        let labels = render(&ctx, &mut app, 1120.0, 720.0, Vec::new());
        assert!(labels.contains("Skin 0000"));
        assert!(!labels.contains("Skin 1999"), "offscreen cards must not be drawn");
        app.skins.browser.query = "maker 1999".into();
        let labels = render(&ctx, &mut app, 1120.0, 720.0, Vec::new());
        assert!(labels.contains("Skin 1999"));
        assert_eq!(app.skins.browser.inspecting, "skin-0000", "filtering must not change the inspected skin");
        for (width, height) in [(470.0, 400.0), (720.0, 500.0), (1120.0, 560.0)] {
            render(&ctx, &mut app, width, height, Vec::new());
        }
        drop(app);
        std::fs::remove_dir_all(folder).unwrap();
    }
}
