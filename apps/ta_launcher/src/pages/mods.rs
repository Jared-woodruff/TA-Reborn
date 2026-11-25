//! The Mods page: one library of the catalogue's mods and those installed, as cards (or the rows
//! of a list) to search, filter and order, the chosen mod's details beside them (in their place
//! on a narrow page): its picture, what it is, what can be done with it and what it changes of
//! the game and of the other mods. A right-click on a card or a row (or Shift+F10) opens what can
//! be done with that mod. Mods the player hid leave the library (and its counts) for its HIDDEN
//! view, as they were otherwise: installed, on or off. The load order of the mods that are on is
//! the page's other view (`mod_order.rs`); the mod folders open in a dialog.

use egui::{Align, Color32, CornerRadius, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{Exit, Panel};
use ta_ui::tooltip::TooltipExt as _;

use super::getmods::{self, CardAction};
use super::{path_label, sentence};
use crate::app::{Action, LauncherApp, ModsTab};
use crate::catalogue::{CatalogueSort, Entry, Fit};
use crate::getmods::Stage;
use crate::library::menu::{self, Icon, Menu};
use crate::library::{self, Card};
use crate::mod_library::{self, Filter, Item, Sort, View};
use crate::modlist::{self, Resolution};
use crate::theme::*;
use crate::widgets::{self, ActionButton, Glyph, Mark};

/// From this width the details stand beside the library; below it they take its place.
pub(super) const SPLIT: f32 = 760.0;
/// A card's banner, over its width.
const WELL: f32 = 0.5;

/// The Mods page's context menu (the library's cards and rows, the load order's rows).
pub(super) fn mod_menu_id() -> Id {
    Id::new("mod_library_menu")
}

/// What a mod's context menu does.
#[derive(Clone, Debug)]
pub(super) enum ModChoice {
    /// Its details: in the library's place on a narrow page (from the load order, the library's).
    Details,
    /// What its buttons do.
    Do(Action),
    /// To this place in the load order.
    Move(usize),
}

const MANIFEST_EXAMPLE: &str = "id = \"my_mod\"\nname = \"My Mod\"\nversion = \"1.0\"\nauthors = [\"You\"]\ndescription = \"What it changes\"\ndepends = []      # ids that must be enabled too\nload_after = []   # ids to load before this one, if enabled";

impl LauncherApp {
    pub(crate) fn mods_page(&mut self, ui: &mut Ui) {
        self.media.start(self.store.dir().join("cache").join("moddb"), self.offline);
        let ctx = ui.ctx().clone();
        // (The installs' news is taken in by the app, whatever the page.)
        if self.media.poll(&ctx) { ctx.request_repaint(); }
        self.mod_media.get_mut().poll(&ctx);
        self.mod_library.changes.poll(&ctx);
        // (A mod's screenshots on the whole window, over the page: first, to take its keys. Closed
        // as asked, the keys go back to the screenshot they were opened from once the page takes
        // them again, a frame after: egui lets go of the keys of a widget under a dialog.)
        let back = ctx.data_mut(|data| {
            let back = data.get_temp::<Id>(lightbox_back_id());
            data.remove::<Id>(lightbox_back_id());
            back
        });
        if let Some(back) = back.filter(|_| self.mod_lightbox.is_none()) { ctx.memory_mut(|memory| memory.request_focus(back)); }
        self.mod_lightbox_ui(ui);
        let now = ctx.input(|i| i.time);
        if self.celebrate.as_ref().is_some_and(|(_, at)| ((now - at) as f32) < getmods::CELEBRATION) {
            ta_ui::motion::next_frame(&ctx, std::time::Duration::from_millis(33));
        }
        let short = ui.available_height() < 600.0;
        self.mods_header(ui, short);
        self.mods_tabs(ui);
        self.mod_problems(ui);
        self.game_folder_offers(ui);
        self.package_installs(ui);
        match self.mods_tab {
            ModsTab::Library => self.mod_library_view(ui),
            ModsTab::LoadOrder => self.load_order_view(ui),
        }
    }

    fn mods_header(&mut self, ui: &mut Ui, short: bool) {
        super::library_header(ui, n("MODS"), n("Mods change Total Annihilation's units, maps and look. Lower in the load order wins."), short,
            &["INSTALL FROM FILE…", "SUPREME COMMANDER…", "MOD FOLDERS…", "RESCAN"], |ui, height| {
            if ui.add(ActionButton::primary("INSTALL FROM FILE…").height(height)).hover_tip(t("Install a mod you have downloaded (or drop it on this window)")).clicked() {
                self.push(Action::BrowsePackage);
            }
            if ui.add(ActionButton::secondary("SUPREME COMMANDER…").height(height))
                .hover_tip(t("Import the factions of Supreme Commander: Forged Alliance from your own copy of the game (GOG's or Steam's): their units and buildings, to play in skirmish; its soundtrack, to play with TA's music or in its place; and its look for the game's menus.")).clicked() {
                self.push(Action::SupcomOpen(crate::supcom::Tab::Factions));
            }
            if ui.add(ActionButton::secondary("MOD FOLDERS…").height(height)).hover_tip(t("The folders mods are looked for in")).clicked() {
                self.show_mod_folders = true;
            }
            if ui.add(ActionButton::secondary("RESCAN").height(height)).hover_tip(t("Look for new or changed mods (F5)")).clicked() {
                self.push(Action::RescanMods);
            }
        });
    }

    /// The page's two views: the library, and the load order of the mods that are on.
    fn mods_tabs(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 26.0;
            let library = tf("LIBRARY · {n}", &[("n", &mod_library::counts(&self.mods, &self.config.mods.hidden).0)]);
            let order = tf("LOAD ORDER · {n}", &[("n", &self.config.mods.enabled.len())]);
            for (tab, label) in [(ModsTab::Library, library), (ModsTab::LoadOrder, order)] {
                if page_tab(ui, &label, self.mods_tab == tab).clicked() && self.mods_tab != tab {
                    ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Tab);
                    self.mods_tab = tab;
                }
            }
        });
        ui.add_space(space::S);
    }

    /// Mods that could not be read in the last scan, folded away.
    fn mod_problems(&mut self, ui: &mut Ui) {
        if self.mods.problems.is_empty() { return; }
        let title = tf("PROBLEMS ({n})", &[("n", &self.mods.problems.len())]);
        egui::CollapsingHeader::new(RichText::new(title).font(strong(12.5)).color(RED)).id_salt("mod_problems").show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("mod_problems_list").max_height(160.0).show(ui, |ui| {
                for problem in &self.mods.problems {
                    ui.horizontal_top(|ui| {
                        widgets::status_mark(ui, Mark::Error);
                        // Multi-line messages (TOML errors) point at columns with `^`; keep them aligned.
                        let text = RichText::new(sentence(problem)).color(TEXT);
                        let text = if problem.contains('\n') { text.font(mono(12.0)) } else { text };
                        ui.add(egui::Label::new(text).wrap());
                    });
                }
            });
            ui.label(RichText::new(t("Fix the files, then press Rescan. Broken mods are skipped.")).color(STEEL));
        });
    }

    /// The mods the original game's folder holds, made for engines of their own, that are not
    /// installed as mods: TA Reborn plays the original game without them, and adds them as mods
    /// (their archives linked from there) at a click.
    fn game_folder_offers(&mut self, ui: &mut Ui) {
        let offers: Vec<(String, String, usize)> = self.game_folder_mods.iter()
            .filter(|m| self.mods.get(&m.id).is_none() && !self.installer.stages.get(&m.id).is_some_and(Stage::active))
            .map(|m| (m.id.clone(), m.name.clone(), m.archives.len()))
            .collect();
        if offers.is_empty() { return; }
        widgets::card(ui, |ui| {
            widgets::card_title(ui, "IN YOUR TOTAL ANNIHILATION FOLDER");
            ui.add(egui::Label::new(RichText::new(t("These mods' own installers put them in the original game's folder, for engines of their own. TA Reborn plays the original game without them: add one to play it.")).color(STEEL)).wrap());
            ui.add_space(space::XS);
            for (id, name, archives) in &offers {
                ui.push_id(("game_folder_mod", id), |ui| {
                    ui.horizontal(|ui| {
                        // (The name at the left, the button at the right.)
                        let room = (ui.available_width() - 190.0).max(80.0);
                        ui.allocate_ui_with_layout(vec2(room, 26.0), Layout::left_to_right(Align::Center), |ui| {
                            ui.set_min_width(room);
                            ui.add(egui::Label::new(RichText::new(name).font(strong(14.0)).color(TEXT_BRIGHT)).truncate())
                                .on_hover_text(tf("{n} archives in the game's folder", &[("n", archives)]));
                        });
                        if ui.add(ActionButton::primary("ADD AS A MOD").height(26.0)).hover_tip(t("Add it to your mods, its files linked from the game's folder (which is left as it is)")).clicked() {
                            self.push(Action::AddGameFolderMod(id.clone()));
                        }
                    });
                });
            }
        });
        ui.add_space(space::S);
    }

    /// Mods the player brought, being installed (or that failed to): one row each. (The
    /// catalogue's show on their own cards.)
    fn package_installs(&mut self, ui: &mut Ui) {
        let rows: Vec<(String, String, Stage)> = self.installer.packages.iter()
            .filter_map(|(id, name)| self.installer.stages.get(id).filter(|s| !matches!(s, Stage::Done { .. } | Stage::Cancelled)).map(|s| (id.clone(), name.clone(), s.clone())))
            .collect();
        if rows.is_empty() { return; }
        widgets::card(ui, |ui| {
            widgets::card_title(ui, "INSTALLING");
            for (id, name, stage) in &rows {
                ui.push_id(("package_install", id), |ui| {
                    ui.horizontal(|ui| {
                        if matches!(stage, Stage::Failed(_)) { widgets::status_mark(ui, Mark::Error); }
                        let room = (ui.available_width() - if stage.active() { 260.0 } else { 40.0 }).max(80.0);
                        ui.add_sized(vec2(room, 22.0), egui::Label::new(RichText::new(name).font(strong(14.0)).color(TEXT_BRIGHT)).truncate());
                        if stage.active() {
                            let (fraction, label) = getmods::stage_progress(stage);
                            getmods::progress(ui, 210.0, fraction, &label);
                        }
                        let label = t(if stage.active() { "Cancel" } else { "Dismiss" });
                        if widgets::icon_button(ui, Glyph::Close, true).hover_tip(label).clicked() {
                            self.push(if stage.active() { Action::CancelInstall(id.clone()) } else { Action::DismissInstall(id.clone()) });
                        }
                    });
                    if let Stage::Failed(why) = stage {
                        ui.add(egui::Label::new(RichText::new(sentence(why)).color(RED)).wrap());
                    }
                });
            }
        });
        ui.add_space(space::S);
    }

    fn mod_library_view(&mut self, ui: &mut Ui) {
        let wide = ui.available_width() >= SPLIT;
        if wide { self.mod_library.details_open = false; }
        if !wide && self.mod_library.details_open && !self.mod_library.selected.is_empty() {
            let back = ta_ui::panel::back_button(ui, "ALL MODS").clicked();
            let escape = self.search_shortcuts_allowed(ui) && ui.memory(|memory| memory.focused().is_none())
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            if back || escape {
                self.mod_library.details_open = false;
                let selected = self.mod_library.selected.clone();
                self.mod_library.choose(&selected);
            }
            ui.add_space(space::S);
            self.mod_details(ui, ui.available_height());
            return;
        }
        // (Nothing hidden any more: the library as it was.)
        if self.mod_library.hidden_view && mod_library::counts(&self.mods, &self.config.mods.hidden).1 == 0 {
            self.mod_library.show_hidden(false);
        }
        self.mod_toolbar(ui);
        let visible = self.visible_mods();
        self.mod_library.reconcile(&visible);
        let height = ui.available_height().max(80.0);
        if wide {
            let details = (ui.available_width() * 0.4).clamp(320.0, 460.0);
            let gap = space::L;
            let library = (ui.available_width() - details - gap).max(1.0);
            let top = ui.cursor().min;
            let left = Rect::from_min_size(top, vec2(library, height));
            let right = Rect::from_min_size(pos2(top.x + library + gap, top.y), vec2(details, height));
            ui.scope_builder(egui::UiBuilder::new().max_rect(left).layout(Layout::top_down(Align::Min)), |ui| self.mod_grid(ui, &visible, height));
            ui.scope_builder(egui::UiBuilder::new().max_rect(right).layout(Layout::top_down(Align::Min)), |ui| self.mod_details(ui, height));
            ui.allocate_rect(left.union(right), Sense::hover());
        } else {
            self.mod_grid(ui, &visible, height);
        }
        self.mod_menu(ui, wide, false);
    }

    /// The library's mods on show, in order (the hidden ones in HIDDEN, the others otherwise).
    pub(crate) fn visible_mods(&self) -> Vec<Item> {
        let status = |id: &str| self.mod_validation.status(id);
        let facts = mod_library::Facts { mods: &self.mods, enabled: &self.config.mods.enabled, status: &status, hidden: &self.config.mods.hidden };
        self.mod_library.visible(&facts)
    }

    /// Hides a mod from the library, or puts it back: nothing else of it changes (installed, on
    /// or off, its place in the load order). The one after it in the list is chosen in its place,
    /// and a notice says where it went.
    pub(crate) fn set_mod_hidden(&mut self, id: &str, hidden: bool) {
        if self.config.mods.is_hidden(id) == hidden { return; }
        let before = self.visible_mods();
        self.config.mods.set_hidden(id, hidden);
        let after = self.visible_mods();
        if self.mod_library.selected == id && !after.iter().any(|item| item.id == id) {
            let at = before.iter().position(|item| item.id == id).unwrap_or(0);
            let next = before.iter().skip(at + 1).chain(before.iter().take(at).rev()).find(|item| after.iter().any(|shown| shown.id == item.id));
            if let Some(next) = next { self.mod_library.choose(&next.id.clone()); }
            // (Its details, in the library's place on a narrow page, leave with it.)
            self.mod_library.details_open = false;
        }
        let name = mod_library::items(&self.mods).into_iter().find(|item| item.id == id).map_or_else(|| id.to_string(), |item| item.name);
        let on = self.config.mods.enabled.iter().any(|enabled| enabled == id);
        let text = match (hidden, on) {
            (true, true) => tf("{name} is hidden from the library. It is still on: the load order lists it.", &[("name", &name)]),
            (true, false) => tf("{name} is hidden from the library: the toolbar's HIDDEN lists it.", &[("name", &name)]),
            (false, _) => tf("{name} is back in the library", &[("name", &name)]),
        };
        self.notify(text, GREEN);
        // (The last one put back: the library again, at it.)
        if self.mod_library.hidden_view && mod_library::counts(&self.mods, &self.config.mods.hidden).1 == 0 {
            self.mod_library.show_hidden(false);
            self.mod_library.choose(id);
        }
    }

    /// Every hidden mod back in the library.
    pub(crate) fn unhide_all_mods(&mut self) {
        if self.config.mods.hidden.is_empty() { return; }
        self.config.mods.hidden.clear();
        self.mod_library.show_hidden(false);
        self.notify("Every hidden mod is back in the library", GREEN);
    }

    /// The search, the filters, the order and the view; UPDATE ALL when there are updates.
    fn mod_toolbar(&mut self, ui: &mut Ui) {
        let shortcuts = self.search_shortcuts_allowed(ui);
        let width = ui.available_width();
        let filters = [(Filter::All, n("ALL")), (Filter::Installed, n("INSTALLED")), (Filter::Enabled, n("ON")), (Filter::Updates, n("UPDATES")), (Filter::PlaysWell, n("PLAYS WELL"))];
        let hint = t("Search mods");
        let id = Id::new("mod_library_search");
        if width >= 760.0 {
            ui.horizontal(|ui| {
                library::search(ui, &mut self.mod_library.query, id, hint, (width * 0.36).clamp(200.0, 360.0), shortcuts);
                ui.add_space(space::S);
                widgets::segmented(ui, &mut self.mod_library.filter, &filters);
            });
        } else {
            library::search(ui, &mut self.mod_library.query, id, hint, width, shortcuts);
            widgets::segmented_fill(ui, &mut self.mod_library.filter, &filters);
        }
        // (A hidden mod's update is on its own card, in HIDDEN: UPDATE ALL leaves it.)
        let updates: Vec<&'static Entry> = crate::catalogue::MODS.iter().filter(|entry| {
            self.installer.stages.get(entry.id).is_none_or(|stage| !stage.active())
                && mod_library::waiting_update(entry, self.mods.updatable_version(entry.id)).is_some() && !self.config.mods.is_hidden(entry.id)
        }).collect();
        let (library, hidden) = mod_library::counts(&self.mods, &self.config.mods.hidden);
        let hidden_view = self.mod_library.hidden_view;
        let shown = self.visible_mods().len();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            let room = |ui: &mut Ui, width: f32| if ui.available_size_before_wrap().x < width { ui.end_row(); };
            let sorts = [(Sort::Spotlight, t(CatalogueSort::Spotlight.label()).to_string()), (Sort::Name, t("Name").into()),
                (Sort::Newest, t(CatalogueSort::Newest.label()).into()), (Sort::Oldest, t(CatalogueSort::Oldest.label()).into()),
                (Sort::Smallest, t(CatalogueSort::Smallest.label()).into()), (Sort::Largest, t(CatalogueSort::Largest.label()).into())];
            let label = t("ORDER");
            let width = widgets::combo_width(ui, sorts.iter().map(|(_, name)| format!("{label}  {name}")), 120.0);
            room(ui, width);
            library::choice(ui, "mod_sort", width, label, &mut self.mod_library.sort, &sorts);
            room(ui, 150.0);
            widgets::segmented(ui, &mut self.mod_library.view, &[(View::Grid, n("CARDS")), (View::List, n("LIST"))]);
            // The mods the player hid: a switch to list them (once there are any), and to put
            // them all back.
            if hidden > 0 || hidden_view {
                let label = tf("HIDDEN · {n}", &[("n", &hidden)]);
                room(ui, widgets::galley(ui, RichText::new(&label).font(strong(11.0))).size().x + 40.0);
                let tip = if hidden_view { t("Back to the library") } else { t("List the mods you hid from the library, to put them back") };
                if widgets::chip(ui, &label, hidden_view).hover_tip(tip).clicked() {
                    self.mod_library.show_hidden(!hidden_view);
                }
                if hidden_view {
                    let width = widgets::galley(ui, RichText::new(t("UNHIDE ALL")).font(strong(13.5))).size().x + 34.0;
                    room(ui, width);
                    if ui.add(ActionButton::secondary("UNHIDE ALL").height(widgets::CONTROL_HEIGHT)).hover_tip(t("Put every hidden mod back in the library")).clicked() {
                        self.push(Action::UnhideAllMods);
                    }
                }
            }
            if !updates.is_empty() && !self.offline && !hidden_view {
                let label = tf("UPDATE ALL · {n}", &[("n", &updates.len())]);
                let width = widgets::galley(ui, RichText::new(&label).font(strong(13.5))).size().x + 34.0;
                room(ui, width);
                let running = self.game_running();
                if ui.add_enabled(!running, ActionButton::primary(&label).height(widgets::CONTROL_HEIGHT)).hover_tip(t("Download and install the tested releases newer than yours"))
                    .disabled_tip(t("Close the game first: it may be using these mods.")).clicked() {
                    self.push(Action::UpdateAll);
                }
            }
            let count = if hidden_view { tf("{shown} of {total} hidden mods", &[("shown", &shown), ("total", &hidden)]) }
                else { tf("{shown} of {total} mods", &[("shown", &shown), ("total", &library)]) };
            let count = RichText::new(count).color(STEEL).size(12.0);
            room(ui, widgets::galley(ui, count.clone()).size().x + 8.0);
            ui.add_space(4.0);
            ui.add(egui::Label::new(count).extend());
            if self.mod_library.filters_set() {
                room(ui, 110.0);
                if widgets::text_link(ui, "CLEAR FILTERS").clicked() {
                    self.mod_library.query.clear();
                    self.mod_library.filter = Filter::All;
                }
            }
        });
        if hidden_view {
            ui.add_space(2.0);
            super::wrapped(ui, t("Hidden mods are only left out of the library: those installed stay installed, and those that are on still load, in the load order."), STEEL);
        }
        ui.add_space(space::S);
    }

    /// The mods on show, as cards or as the rows of a list.
    fn mod_grid(&mut self, ui: &mut Ui, visible: &[Item], height: f32) {
        if visible.is_empty() {
            ui.add_space(space::L);
            let searching = !self.mod_library.query.trim().is_empty();
            if self.mod_library.hidden_view {
                super::wrapped(ui, t("No hidden mods match this search. Try another name or clear the filters."), STEEL);
            } else if self.mods.mods.is_empty() && self.mod_library.filter == Filter::Installed {
                super::wrapped(ui, t("No mods installed. GET one from ALL, drop a mod's archive on this window, or put a mod folder (with a mod.toml) or a classic .ufo, .hpi, .ccx or .gp3 archive in one of your MOD FOLDERS… and press RESCAN."), STEEL);
                ui.add_space(space::S);
                ui.label(widgets::caption("MOD.TOML EXAMPLE"));
                egui::Frame::new().fill(INSET).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(2)).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(MANIFEST_EXAMPLE).font(mono(12.0)).color(TEXT)).wrap());
                });
            } else {
                // (Each filter empty says what that means, unless a search emptied it.)
                let empty = match self.mod_library.filter {
                    _ if searching => n("No mods match this search. Try another name or clear the filters."),
                    Filter::Enabled => n("No mods are on: the game plays as it came."),
                    Filter::Updates => n("Everything is up to date."),
                    Filter::PlaysWell => n("None of your mods has passed its check yet: MOD INFO checks one."),
                    _ => n("No mods match this search. Try another name or clear the filters."),
                };
                super::wrapped(ui, t(empty), STEEL);
            }
            return;
        }
        let reveal = self.mod_library.take_reveal().and_then(|id| visible.iter().position(|item| item.id == id));
        let width = library::grid_width(ui.available_width());
        let view = self.mod_library.view;
        let layout = match view {
            View::Grid => library::layout(width, 200.0, WELL, 50.0, 12.0),
            View::List => library::rows(width, 64.0, 0.0),
        };
        let well = ((layout.card.x - 12.0) * WELL).round();
        let now = ui.input(|i| i.time);
        let this = &*self;
        let hidden_view = this.mod_library.hidden_view;
        let hidden_word = hidden_label();
        // (A card's ribbon only where it leaves its UNHIDE room: in a narrow card, in a long
        // language, the button says enough.)
        let ribbon = hidden_view && (view == View::List || {
            let ribbon = widgets::galley(ui, RichText::new(&hidden_word).font(strong(10.5)).extra_letter_spacing(ta_ui::i18n::tracking(1.0))).size().x + 19.0;
            ribbon + unhide_width(ui) + 20.0 <= layout.card.x - 12.0
        });
        let mut chosen = None;
        let mut opened = false;
        let mut focused = None;
        let mut asked = None;
        let mut unhide = None;
        library::grid(ui, Id::new("mod_library_grid"), visible.len(), layout, height, reveal, |ui, at, rect| {
            let item = &visible[at];
            let (detail, lamps, check) = this.mod_card_words(item);
            let spoken = match check { Some(check) => format!("{}, {detail}, {}", item.name, t(check)), None => format!("{}, {detail}", item.name) };
            // (In HIDDEN each card says so, and has its way back.)
            let card = Card { title: &item.name, detail: &detail, lamps: &lamps, star: false, selected: item.id == this.mod_library.selected, spoken: &spoken,
                ribbon: ribbon.then_some(hidden_word.as_str()) };
            let id = Id::new(("mod_card", &item.id));
            let over = ui.rect_contains_pointer(rect);
            let paint = |ui: &Ui, frame: Rect| { this.paint_banner(ui, frame, item, false, over); };
            let response = match view {
                View::Grid => library::card(ui, id, rect, well, card, paint),
                View::List => library::picture_row(ui, id, rect, 1.8, card, paint),
            };
            if hidden_view && unhide_button(ui, id, rect, view) {
                unhide = Some(item.id.clone());
            }
            if let Some((at, keys)) = menu::asked(ui, &response) {
                asked = Some((item.id.clone(), at, keys.then_some(response.id)));
            }
            // A mod installed a moment ago glows, its confetti flying.
            if let Some(age) = this.celebrate.as_ref().filter(|(fresh, _)| *fresh == item.id).map(|(_, at)| (now - at) as f32)
                .filter(|age| (0.0..getmods::CELEBRATION).contains(age) && !ta_ui::motion::still(ui.ctx())) {
                let glow = 1.0 - age / getmods::CELEBRATION;
                ui.painter().rect_stroke(rect.expand(1.0), CornerRadius::same(3), Stroke::new(2.0, GREEN.gamma_multiply(glow)), StrokeKind::Outside);
                getmods::confetti(ui.painter(), rect.left_bottom() + vec2(40.0, -20.0), age, &item.id);
            }
            if response.clicked() { chosen = Some(item.id.clone()); opened = true; }
            if response.has_focus() {
                // (egui moves the keys between cards at the end of a frame and reports no
                // gained focus for it: the card that has them is followed instead.)
                if this.mod_library.focused.as_ref() != Some(&item.id) { response.scroll_to_me(None); }
                focused = Some(item.id.clone());
                if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) { opened = true; }
            }
            response
        });
        if focused != self.mod_library.focused {
            if let Some(id) = &focused { self.mod_library.selected = id.clone(); }
            self.mod_library.focused = focused;
        }
        if let Some(id) = chosen { self.mod_library.selected = id; }
        if opened { self.mod_library.details_open = true; }
        if let Some(id) = unhide { self.push(Action::HideMod(id, false)); }
        // A right-click (or Shift+F10) chooses the mod and opens what can be done with it.
        if let Some((id, at, from)) = asked {
            self.mod_library.selected = id.clone();
            menu::open(ui.ctx(), mod_menu_id(), &id, at, from);
        }
    }

    /// The mod menu while it is open: built from the mod as it is now, what it says done.
    /// `wide`: the details stand beside the library; `in_order`: opened from the load order.
    pub(super) fn mod_menu(&mut self, ui: &Ui, wide: bool, in_order: bool) {
        let ctx = ui.ctx().clone();
        let Some(id) = menu::target(&ctx, mod_menu_id()) else { return };
        let item = mod_library::items(&self.mods).into_iter().find(|item| item.id == id);
        if item.is_none() && !self.config.mods.enabled.contains(&id) {
            menu::close(&ctx, mod_menu_id());
            return;
        }
        let entries = self.mod_menu_entries(&id, item.as_ref(), wide, in_order);
        let Some(choice) = menu::show(ui, mod_menu_id(), &entries) else { return };
        match choice {
            ModChoice::Details => {
                self.show_mod_details(&id);
                // (Beside the library, the details are there already.)
                if wide { self.mod_library.details_open = false; }
            }
            ModChoice::Do(action) => self.push(action),
            ModChoice::Move(to) => {
                if let Some(from) = self.config.mods.enabled.iter().position(|enabled| *enabled == id) {
                    modlist::move_to(&mut self.config.mods.enabled, from, to);
                    self.mods.refresh(&self.config.mods.enabled);
                    self.mod_library.order_reveal = Some(id);
                }
            }
        }
    }

    /// What can be done with the mod `id` (`item`: the library's, when it has it), in the order
    /// its details offer it: its details; get, update, play or install it; on or off; its place
    /// in the load order; its report, its files, its pages; hide it; remove it. What cannot apply
    /// to it is left out (moves for a mod that is off); what cannot be done now says why.
    fn mod_menu_entries(&self, id: &str, item: Option<&Item>, wide: bool, in_order: bool) -> Menu<ModChoice> {
        let installed = self.mods.get(id).cloned();
        let on = self.config.mods.enabled.iter().any(|enabled| enabled == id);
        let hidden = self.config.mods.is_hidden(id);
        let name = item.map(|item| item.name.clone()).or_else(|| installed.as_ref().map(|entry| entry.info.manifest.name.clone())).unwrap_or_else(|| id.to_string());
        let mut words = Vec::new();
        if let Some(item) = item { words.push(t(item.kind(&self.mods).label()).to_string()); }
        words.push(t(if on { n("ON") } else if installed.is_some() { n("INSTALLED") } else if item.is_some() { n("NOT INSTALLED") } else { n("NOT FOUND") }).to_string());
        if hidden { words.push(hidden_label()); }
        let mut menu = Menu::new(name, words.join(" · "));
        if in_order || !wide {
            let details = menu.add(n("VIEW DETAILS"), Icon::Details, ModChoice::Details);
            if !in_order { details.keys("Enter"); }
            menu.rule();
        }
        let game = self.detection.install().is_some();
        let running = self.game_running();
        let catalogue = item.and_then(|item| item.catalogue);
        // (A release only its creator's site hands out leads with that site, as its card does.)
        let manual = catalogue.is_some_and(|entry| entry.fit == Fit::Unverified || entry.file.is_some_and(|file| !file.can_download())) && installed.is_none();
        let website = |entry: &Entry| t(if entry.project.is_none() && entry.moddb().is_some() { n("MODDB ↗") } else { n("WEBSITE ↗") }).trim_end_matches('↗').trim_end().to_string();
        if let Some(entry) = catalogue {
            let stage = self.installer.stages.get(entry.id).filter(|stage| stage.active());
            let version = installed.as_ref().map(|entry| entry.info.manifest.version.clone());
            let offline = self.offline.then(|| t("The launcher is offline."));
            let held = running.then(|| t("Close the game first: it may be using this mod."))
                .or_else(|| self.mods.in_game_folder(id).then(|| t("This copy is in TA Reborn's program folder, which comes before your mods folder: replace it there to update it.")));
            let update = entry.file.filter(|file| file.can_download() && mod_library::update(entry, version.as_deref()).is_some());
            if manual {
                menu.add_text(website(entry), Icon::Web, ModChoice::Do(Action::OpenUrl(entry.page()))).main();
            }
            if stage.is_some() {
                menu.add(n("CANCEL"), Icon::Cancel, ModChoice::Do(Action::CancelInstall(entry.id.into())));
            } else {
                if let (Some(_), Some(file)) = (&version, update) {
                    menu.add_text(tf("UPDATE · {size}", &[("size", &getmods::megabytes(file.bytes))]), Icon::Download, ModChoice::Do(Action::GetMod(entry.id))).main().unless(offline.or(held));
                } else if let (None, Some(file)) = (&version, entry.file) && file.can_download() {
                    menu.add_text(tf("GET · {size}", &[("size", &getmods::megabytes(file.bytes))]), Icon::Download, ModChoice::Do(Action::GetMod(entry.id))).main().unless(offline);
                }
                if installed.is_some() {
                    let why = if !game { Some(t("Find your Total Annihilation install on the Play page first.")) } else if running { Some(t("The game is running.")) } else { None };
                    let play = menu.add(n("PLAY"), Icon::Play, ModChoice::Do(Action::PlayMod(id.into())));
                    play.unless(why);
                    if update.is_none() { play.main(); }
                }
                if entry.can_import() && entry.file.is_none_or(|file| version.as_deref() != Some(file.version)) && (installed.is_none() || held.is_none()) {
                    menu.add(n("INSTALL FROM FILE…"), Icon::File, ModChoice::Do(Action::InstallModFromFile(entry.id)));
                }
            }
        }
        if installed.is_some()
            && let Some(tab) = self.supcom_update(id)
        {
            menu.add(n("UPDATE…"), Icon::Download, ModChoice::Do(Action::SupcomOpen(tab))).main();
        }
        if installed.is_some() {
            if on {
                menu.add(n("TURN OFF"), Icon::Switch(false), ModChoice::Do(Action::DisableMod(id.into())));
            } else {
                menu.add(n("TURN ON"), Icon::Switch(true), ModChoice::Do(Action::EnableMod(id.into())));
            }
        } else if on {
            // (On in the settings, installed nowhere: the load order's NOT FOUND.)
            menu.add(n("Remove from the load order"), Icon::Cancel, ModChoice::Do(Action::DisableMod(id.into())));
        }
        // Its place in the load order, for a mod that is on (Alt with the arrows in the order).
        let order = &self.config.mods.enabled;
        if let Some(at) = order.iter().position(|enabled| enabled == id) {
            let last = order.len() - 1;
            let first = if last == 0 { Some(t("It is the only mod that is on.")) } else { (at == 0).then(|| t("It loads first already.")) };
            let end = if last == 0 { first } else { (at == last).then(|| t("It loads last already.")) };
            menu.rule();
            // (As the load order's buttons stand: to the top, up, down, to the bottom; a reason
            // said once for the two it holds back.)
            let moves = [(n("Move to the top"), Icon::Top, 0, first, "Alt+Home"), (n("Load earlier"), Icon::Up, at.saturating_sub(1), first, "Alt+↑"),
                (n("Load later"), Icon::Down, (at + 1).min(last), end, "Alt+↓"), (n("Move to the bottom"), Icon::Bottom, last, end, "Alt+End")];
            for (label, icon, to, why, keys) in moves {
                let entry = menu.add(label, icon, ModChoice::Move(to));
                entry.unless(why);
                if in_order { entry.keys(keys); }
            }
        }
        menu.rule();
        if item.is_some() {
            menu.add(n("MOD INFO"), Icon::Report, ModChoice::Do(Action::ShowModReport(id.into())));
        }
        if let Some(entry) = &installed {
            let place = if entry.is_archive() { entry.info.path.parent().map(|p| p.to_path_buf()) } else { Some(entry.info.path.clone()) };
            if let Some(place) = place {
                menu.add(n("SHOW SOURCE FILES"), Icon::Folder, ModChoice::Do(Action::OpenPath(place)));
            }
        }
        if let Some(entry) = catalogue {
            if !manual {
                menu.add_text(website(entry), Icon::Web, ModChoice::Do(Action::OpenUrl(entry.page())));
            }
            // (The tested release's host, credited beside the creator's own page.)
            if entry.project.is_some() && entry.moddb().is_some() {
                menu.add_text(t(n("MODDB ↗")).trim_end_matches('↗').trim_end().to_string(), Icon::Web, ModChoice::Do(Action::OpenUrl(entry.source_page())));
            }
        }
        if item.is_some() {
            menu.rule();
            if hidden {
                menu.add(n("UNHIDE"), Icon::Eye(true), ModChoice::Do(Action::HideMod(id.into(), false)));
            } else {
                menu.add(n("HIDE FROM LIBRARY"), Icon::Eye(false), ModChoice::Do(Action::HideMod(id.into(), true)));
            }
        }
        if installed.is_some() {
            menu.rule();
            menu.add(n("REMOVE"), Icon::Remove, ModChoice::Do(Action::Uninstall(id.into()))).danger().unless(self.removal_blocked_for(id));
        }
        menu
    }

    /// A card's line under its name, and its lamp: on, an update waiting, installed.
    /// A card's words, its lamps (its state; then red when the check found errors) and what
    /// the check found, for screen readers. (Warnings are common, and amber is an update's.)
    fn mod_card_words(&self, item: &Item) -> (String, Vec<Color32>, Option<&'static str>) {
        let installed = self.mods.get(&item.id);
        let version = installed.map(|entry| entry.info.manifest.version.as_str());
        let update = item.catalogue.and_then(|entry| mod_library::waiting_update(entry, self.mods.updatable_version(&item.id)));
        // (Supreme Commander's maps, factions, music or menus made by an older converter: made again in their dialog.)
        let remake = self.supcom_update(&item.id);
        let on = self.config.mods.enabled.contains(&item.id);
        let stage = self.installer.stages.get(&item.id).filter(|stage| stage.active());
        let detail = if let Some(stage) = stage {
            getmods::stage_progress(stage).1
        } else if let Some(newer) = update {
            tf("Update: {version}", &[("version", &newer)])
        } else if remake.is_some() {
            t("Update: import it again").to_string()
        } else if let Some(entry) = item.catalogue {
            tf("by {team}", &[("team", &entry.team)])
        } else {
            let kind = t(item.kind(&self.mods).label()).to_string();
            match version.filter(|version| !version.is_empty()) {
                Some(version) => format!("{kind} · {version}"),
                None => kind,
            }
        };
        let lamp = if stage.is_some() { Some(SKY) } else if on { Some(GREEN) } else if update.is_some() || remake.is_some() { Some(AMBER) } else if item.installed { Some(STEEL) } else { None };
        // (A mod that needs a newer TA Reborn than this is not mounted: as grave as errors.)
        let newer = installed.is_some_and(|entry| ta_vfs::mods::version_older(ta_vfs::mods::REBORN_VERSION, &entry.info.manifest.requires_reborn) == Some(true));
        let check = if !item.installed { None } else if newer { Some((RED, n("NEEDS A NEWER TA REBORN"))) } else {
            match self.mod_validation.status(&item.id) {
                crate::validation::ValidationStatus::Ready(ta_data::validation::Status::Errors) => Some((RED, n("ERRORS FOUND"))),
                _ => None,
            }
        };
        let lamps = lamp.into_iter().chain(check.map(|(colour, _)| colour)).collect();
        (detail, lamps, check.map(|(_, words)| words))
    }

    /// A mod's banner: its own picture (its manifest's), else the catalogue's, else a plate of its
    /// own with its name (and its icon); its gameplay preview playing in it, on the details as
    /// they show and on a card once the pointer has rested on it (`over`: the pointer is on the
    /// card), never while motion is held (the window in the background, or idle); and on the
    /// details' larger one (`badge`), how it fared (its review, or this copy's check), and a
    /// button to stop its preview as it plays, or to play it (with motion reduced, it waits to be
    /// asked). What the player asked of the preview by that button: play it (true) or stop it.
    fn paint_banner(&self, ui: &Ui, frame: Rect, item: &Item, badge: bool, over: bool) -> Option<bool> {
        let ctx = ui.ctx().clone();
        let own = self.mods.get(&item.id).map(|entry| &entry.info);
        let mut media = self.mod_media.borrow_mut();
        let banner = own.and_then(|info| media.banner(info));
        let icon = own.and_then(|info| media.icon(info));
        let (texture, bundled) = match banner {
            Some(banner) => (Some(banner), false),
            None => (item.catalogue.and_then(|entry| self.media.pictures.get(entry.id)).cloned(), item.catalogue.is_some_and(|entry| entry.bundled_image().is_some())),
        };
        // Its gameplay preview: on the details unless the player stopped it (with motion reduced,
        // once they ask), on a card once the pointer has rested on it (not with motion reduced).
        let preview = own.filter(|info| media.has_preview(info));
        let (reduced, held) = (ta_ui::motion::reduced(&ctx), ta_ui::motion::held(&ctx));
        let choice = self.preview_choice.as_ref().filter(|(id, _)| *id == item.id).map(|(_, play)| *play);
        let playing = preview.is_some() && !held && if badge {
            if reduced { choice == Some(true) } else { choice != Some(false) }
        } else {
            !reduced && media.resting(&ctx, &item.id, over)
        };
        let shown = if playing { preview.and_then(|info| media.preview(&ctx, info)) } else { None };
        drop(media);
        let showing = shown.is_some();
        let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(frame));
        match shown {
            Some((frame_texture, next)) => {
                getmods::paint_picture(&painter, frame, Some(&frame_texture), &item.id, &item.name, false, 0.0, None);
                let next = std::time::Duration::from_secs_f32(next.max(0.01));
                // (Asked for with motion reduced, it plays though idle animations hold still.)
                if reduced { ctx.request_repaint_after(next); } else { ta_ui::motion::next_frame(&ctx, next); }
            }
            None => getmods::paint_picture(&painter, frame, texture.as_ref(), &item.id, &item.name, bundled, 0.0, icon.as_ref()),
        }
        painter.add(ta_ui::metal::well(frame, Color32::TRANSPARENT));
        if badge {
            let (label, colour) = self.fit_badge(item);
            getmods::badge(ui, &painter, frame.left_top() + vec2(10.0, 10.0), t(label), colour, egui::Align2::LEFT_TOP);
        }
        // A preview: on the details, a button to stop it as it plays (or is read to play), or to
        // play it; a card's mark in its corner while it does not play.
        let mut ask = None;
        if preview.is_some() && badge && !held {
            let (glyph, label) = if playing { (Glyph::Stop, n("STOP PREVIEW")) } else { (Glyph::Play, n("PLAY PREVIEW")) };
            if banner_button(ui, &painter, frame, Id::new(("mod_preview_button", &item.id)), glyph, label).clicked() {
                ask = Some(!playing);
            }
        } else if preview.is_some() && !badge && !showing {
            let center = frame.right_bottom() - vec2(17.0, 17.0);
            painter.circle(center, 11.0, Color32::from_black_alpha(175), Stroke::new(1.0, STEEL_DIM));
            widgets::paint_glyph(&painter, widgets::Glyph::Play, center + vec2(1.0, 0.0), 0.6, TEXT_BRIGHT);
        }
        ask
    }

    /// How a mod fared: this copy's own check once installed, else the catalogue's review.
    fn fit_badge(&self, item: &Item) -> (&'static str, Color32) {
        if item.installed { return super::mod_report::validation_badge(&self.mod_validation.status(&item.id)); }
        match item.catalogue.map(|entry| entry.fit) {
            Some(Fit::Plays) => (n("REVIEW: PLAYS WELL"), GREEN),
            Some(Fit::Partly) => (n("REVIEW: KNOWN GAPS"), AMBER),
            Some(Fit::Unverified) => (n("NOT VERIFIED"), AMBER),
            Some(Fit::NoFile) => (n("ON MODDB ONLY"), STEEL),
            None => (n("NOT CHECKED"), STEEL),
        }
    }

    /// Why a mod cannot be removed now, if it cannot.
    /// Why the Maps page's work (imports, a new map, a rescan) waits, when it does.
    pub(crate) fn maps_busy_reason(&self) -> Option<&'static str> {
        if self.game_running() { Some(t("The game is running.")) }
        else if self.installer.busy() { Some(t("Wait for the installs to finish.")) }
        else if self.maps.busy() { Some(t("Wait for the current map or mod work to finish.")) }
        else { None }
    }

    pub(crate) fn removal_blocked(&self) -> Option<&'static str> {
        if self.game_running() { Some(t("The game is running.")) }
        else if self.installer.busy() { Some(t("Wait for the installs to finish.")) }
        else if self.maps.importing() { Some(t("Wait for the map import to finish.")) }
        // (An import installs its mod again as it ends, and turns it on.)
        else if self.supcom.importing() { Some(t("Wait for the Supreme Commander maps being made, then try again.")) }
        else if self.supcom.factions.importing() { Some(t("Wait for the Supreme Commander factions being made, then try again.")) }
        else if self.supcom.music.importing() { Some(t("Wait for the Supreme Commander music being made, then try again.")) }
        else if self.supcom.menus.importing() { Some(t("Wait for the Supreme Commander menus being made, then try again.")) }
        else { None }
    }

    /// Why the mod `id` cannot be removed now, if it cannot: [`Self::removal_blocked`], or its copy
    /// is in TA Reborn's program folder, which only an administrator may change.
    pub(crate) fn removal_blocked_for(&self, id: &str) -> Option<&'static str> {
        self.removal_blocked().or_else(|| self.mods.in_game_folder(id).then(|| t("This copy is in TA Reborn's program folder, which only an administrator may change: remove it there.")))
    }

    /// The chosen mod: its banner, what it is, what can be done with it, what it changes; in two
    /// columns where there is room (the banner and its words at the left).
    fn mod_details(&mut self, ui: &mut Ui, height: f32) {
        let Some(item) = mod_library::items(&self.mods).into_iter().find(|item| item.id == self.mod_library.selected) else {
            if !self.mods.mods.is_empty() || !crate::catalogue::MODS.is_empty() { super::wrapped(ui, t("Choose a mod to see it here."), STEEL); }
            return;
        };
        // (What the player asked of another mod's preview is forgotten as this one is shown.)
        if self.preview_choice.as_ref().is_some_and(|(id, _)| *id != item.id) { self.preview_choice = None; }
        let mut actions: Vec<Action> = Vec::new();
        let mut play = None;
        let mut area = egui::ScrollArea::vertical().id_salt(("mod_details", &item.id)).max_height(height).min_scrolled_height(0.0).auto_shrink([false, false]);
        if let Some(offset) = self.capture_scroll { area = area.vertical_scroll_offset(offset); }
        ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Backdrop(BG), |ui| widgets::card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 7.0;
            let width = ui.available_width();
            if width >= 620.0 {
                let gap = space::XL;
                let left = ((width - gap) * 0.46).floor();
                let right = (width - gap - left).max(1.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.allocate_ui_with_layout(vec2(left, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(left);
                        ui.spacing_mut().item_spacing.y = 7.0;
                        let (rect, _) = ui.allocate_exact_size(vec2(left, (left * 0.56).round()), Sense::hover());
                        play = play.or(self.paint_banner(ui, rect, &item, true, false));
                        self.mod_about(ui, &item, &mut actions);
                    });
                    ui.allocate_ui_with_layout(vec2(right, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(right);
                        ui.spacing_mut().item_spacing.y = 7.0;
                        self.mod_heading(ui, &item, &mut actions);
                        self.mod_files(ui, &item, &mut actions);
                    });
                });
            } else {
                let (rect, _) = ui.allocate_exact_size(vec2(width, (width * 0.4).clamp(100.0, 200.0).min(height * 0.4)), Sense::hover());
                play = play.or(self.paint_banner(ui, rect, &item, true, false));
                self.mod_heading(ui, &item, &mut actions);
                ui.add_space(space::XS);
                self.mod_about(ui, &item, &mut actions);
                self.mod_files(ui, &item, &mut actions);
            }
        }));
        // (Played where it waits to be asked, motion reduced; stopped where it plays of itself.
        // Otherwise, as it was.)
        if let Some(play) = play {
            let reduced = ta_ui::motion::reduced(ui.ctx());
            self.preview_choice = (play == reduced).then(|| (item.id.clone(), play));
        }
        for action in actions { self.push(action); }
    }

    /// A mod's name and creators, and what can be done with it.
    fn mod_heading(&mut self, ui: &mut Ui, item: &Item, actions: &mut Vec<Action>) {
        let title = egui::Label::new(RichText::new(item.name.to_uppercase()).font(display(24.0)).color(TEXT_BRIGHT)
            .extra_letter_spacing(ta_ui::i18n::tracking(1.0))).wrap();
        // (Its icon before its name, where it has one.)
        let icon = self.mods.get(&item.id).map(|entry| entry.info.clone()).and_then(|info| self.mod_media.borrow_mut().icon(&info));
        match icon {
            Some(icon) => {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                    ui.painter().rect_filled(rect.expand(1.0), CornerRadius::same(4), INSET);
                    ui.painter().add(egui::Shape::Rect(egui::epaint::RectShape::filled(rect, CornerRadius::same(3), Color32::WHITE).with_texture(icon.id(), getmods::square_uv(&icon))));
                    ui.add(title);
                });
            }
            None => { ui.add(title); }
        }
        let installed = self.mods.get(&item.id).cloned();
        let version = installed.as_ref().map(|entry| entry.info.manifest.version.clone());
        let on = self.config.mods.enabled.contains(&item.id);
        match (item.catalogue, &installed) {
            (Some(entry), _) => {
                let by = tf("by {team}", &[("team", &entry.team)]);
                ui.add(egui::Label::new(RichText::new(by).font(strong(12.5)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(0.6))).wrap());
                let stage = self.installer.stages.get(entry.id);
                let blocked = self.removal_blocked_for(&item.id);
                let held = self.game_running().then(|| t("Close the game first: it may be using this mod."))
                    .or_else(|| self.mods.in_game_folder(&item.id).then(|| t("This copy is in TA Reborn's program folder, which comes before your mods folder: replace it there to update it.")));
                if let Some(action) = getmods::footer_ui(ui, entry, version.as_deref(), on, stage, !self.offline, blocked, held) {
                    actions.push(match action {
                        CardAction::Get => Action::GetMod(entry.id),
                        CardAction::Cancel => Action::CancelInstall(entry.id.into()),
                        CardAction::Play => Action::PlayMod(entry.id.into()),
                        CardAction::TurnOn => Action::EnableMod(entry.id.into()),
                        CardAction::TurnOff => Action::DisableMod(entry.id.into()),
                        CardAction::FromFile => Action::InstallModFromFile(entry.id),
                        CardAction::Open(url) => Action::OpenUrl(url),
                        CardAction::Remove => Action::Uninstall(entry.id.into()),
                        CardAction::Info => Action::ShowModReport(entry.id.into()),
                    });
                }
            }
            (None, Some(entry)) => {
                let manifest = &entry.info.manifest;
                if !manifest.authors.is_empty() {
                    ui.add(egui::Label::new(RichText::new(tf("by {team}", &[("team", &super::listed(&manifest.authors))])).font(strong(12.5)).color(AMBER)).wrap());
                }
                let remake = self.supcom_update(&item.id);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    // (Made by an older converter: made again from the player's copy of the game.)
                    if let Some(tab) = remake
                        && ui.add(ActionButton::primary("UPDATE…").height(28.0)).hover_tip(t("Made by an older TA Reborn: import it again from your copy of the game to bring it up to date.")).clicked()
                    {
                        actions.push(Action::SupcomOpen(tab));
                    }
                    let mut toggled = on;
                    if widgets::toggle_named(ui, &mut toggled, crate::supcom::mod_name(manifest)).hover_tip(t("Turn the mod on or off for the next game")).changed() {
                        actions.push(if toggled { Action::EnableMod(item.id.clone()) } else { Action::DisableMod(item.id.clone()) });
                    }
                    ui.label(RichText::new(t(if on { "ON" } else { "OFF" })).font(strong(12.0)).color(if on { GREEN } else { STEEL }));
                    if super::mod_report::info_button(ui).clicked() { actions.push(Action::ShowModReport(item.id.clone())); }
                    let blocked = self.removal_blocked_for(&item.id);
                    let remove = ui.add_enabled(blocked.is_none(), ActionButton::secondary("REMOVE").height(28.0));
                    let remove = match blocked { Some(why) => remove.disabled_tip(why), None => remove.hover_tip(t("Remove this mod (to the Recycle Bin)")) };
                    if remove.clicked() { actions.push(Action::Uninstall(item.id.clone())); }
                });
            }
            (None, None) => return,
        }
        // Out of the library, or back in it: nothing else of the mod changes.
        let hidden = self.config.mods.is_hidden(&item.id);
        let (label, tip) = if hidden { (n("UNHIDE"), t("Put this mod back in the library")) }
            else { (n("HIDE FROM LIBRARY"), t("Leave this mod out of the library: it stays installed, on or off as it is. The toolbar's HIDDEN lists it.")) };
        if widgets::text_link(ui, label).hover_tip(tip).clicked() {
            actions.push(Action::HideMod(item.id.clone(), !hidden));
        }
    }

    /// What a mod is: its tags, its release, its words, what to know of it.
    fn mod_about(&mut self, ui: &mut Ui, item: &Item, actions: &mut Vec<Action>) {
        let installed = self.mods.get(&item.id).cloned();
        let version = installed.as_ref().map(|entry| entry.info.manifest.version.clone());
        let on = self.config.mods.enabled.contains(&item.id);
        let hidden = self.config.mods.is_hidden(&item.id);
        let tags = |ui: &mut Ui, extra: &dyn Fn(&mut Ui)| {
            ui.horizontal_wrapped(|ui| {
                widgets::tag(ui, item.kind(&self.mods).label(), STEEL);
                if installed.is_some() { widgets::tag(ui, if on { "ON" } else { "INSTALLED" }, if on { GREEN } else { SKY }); }
                if hidden { widgets::tag(ui, &hidden_label(), AMBER); }
                extra(ui);
            });
        };
        match (item.catalogue, &installed) {
            (Some(entry), _) => {
                tags(ui, &|ui: &mut Ui| {
                    if installed.is_some() && entry.fit == Fit::Partly { widgets::tag(ui, "KNOWN GAPS", AMBER); }
                    for tag in entry.tags { widgets::tag(ui, tag, STEEL); }
                });
                getmods::release_details(ui, entry);
                if let Some(version) = version.as_deref() && mod_library::update(entry, self.mods.updatable_version(&item.id)).is_some() {
                    let shown = if version.is_empty() || version == "unverified" { t("Unknown") } else { version };
                    ui.label(RichText::new(tf("Installed: {version}", &[("version", &shown)])).font(strong(12.5)).color(AMBER));
                }
                ui.add(egui::Label::new(RichText::new(t(entry.pitch)).font(body(13.5)).color(TEXT)).wrap());
                if !entry.note.is_empty() {
                    ui.add(egui::Label::new(RichText::new(t(entry.note)).font(body(12.5)).color(if entry.fit == Fit::Partly { AMBER } else { STEEL })).wrap());
                }
                if let Some(newer) = self.media.newer(entry)
                    && ui.link(RichText::new(tf("Newer on ModDB: {title} (not tried here yet)", &[("title", &newer.title)])).font(body(12.5))).clicked() {
                    actions.push(Action::OpenUrl(newer.link.clone()));
                }
                let issues = entry.review_issues();
                if !issues.is_empty() {
                    ui.add_space(space::XS);
                    ui.label(widgets::caption("KNOWN ISSUES"));
                    for issue in issues {
                        ui.horizontal_top(|ui| {
                            ui.label(RichText::new("·").color(AMBER));
                            ui.add(egui::Label::new(RichText::new(t(issue.title)).color(TEXT)).wrap());
                        });
                    }
                }
            }
            (None, Some(entry)) => {
                let manifest = &entry.info.manifest;
                if self.supcom_update(&item.id).is_some() {
                    ui.add(egui::Label::new(RichText::new(t("Made by an older TA Reborn: UPDATE… imports it again from your copy of the game.")).color(AMBER)).wrap());
                }
                let shown = if manifest.version.is_empty() { t("Unknown") } else { &manifest.version };
                tags(ui, &|ui: &mut Ui| {
                    widgets::tag(ui, &tf("Version: {version}", &[("version", &shown)]), STEEL);
                    widgets::tag(ui, if entry.is_archive() { "ARCHIVE" } else { "FOLDER" }, STEEL);
                    // (Its own words for what it is: as written.)
                    for tag in manifest.tags.iter().filter(|tag| !tag.trim().is_empty()).take(12) {
                        widgets::tag(ui, &tag.trim().to_uppercase(), STEEL);
                    }
                });
                if !manifest.description.is_empty() {
                    ui.add(egui::Label::new(RichText::new(t(&manifest.description)).font(body(13.5)).color(TEXT)).wrap());
                }
                if !manifest.depends.is_empty() {
                    let names: Vec<String> = manifest.depends.iter().map(|id| self.mods.get(id).map_or(id.clone(), |m| m.info.manifest.name.clone())).collect();
                    ui.add(egui::Label::new(RichText::new(tf("requires {mods}", &[("mods", &super::listed(&names))])).color(STEEL)).wrap());
                }
                if !manifest.conflicts.is_empty() {
                    let names: Vec<String> = manifest.conflicts.iter().map(|id| self.mods.get(id).map_or(id.clone(), |m| m.info.manifest.name.clone())).collect();
                    ui.add(egui::Label::new(RichText::new(tf("cannot be on with {mods}", &[("mods", &super::listed(&names))])).color(STEEL)).wrap());
                }
                if !manifest.requires_reborn.trim().is_empty() {
                    let older = ta_vfs::mods::version_older(ta_vfs::mods::REBORN_VERSION, &manifest.requires_reborn) == Some(true);
                    let words = if older { tf("needs TA Reborn {version} or later: this is {current}", &[("version", &manifest.requires_reborn.trim()), ("current", &ta_vfs::mods::REBORN_VERSION)]) }
                        else { tf("needs TA Reborn {version} or later", &[("version", &manifest.requires_reborn.trim())]) };
                    ui.add(egui::Label::new(RichText::new(words).color(if older { RED } else { STEEL })).wrap());
                }
            }
            (None, None) => {}
        }
        self.mod_presentation(ui, item, actions);
    }

    /// What an installed mod shows of itself beyond its words: its web pages (its manifest's
    /// `source`, where the catalogue does not give one, and `links`) and its screenshots, a click
    /// on one showing them on the whole window.
    fn mod_presentation(&mut self, ui: &mut Ui, item: &Item, actions: &mut Vec<Action>) {
        let Some(info) = self.mods.get(&item.id).map(|entry| entry.info.clone()) else { return };
        let manifest = &info.manifest;
        let mut links: Vec<(String, String)> = Vec::new();
        if item.catalogue.is_none() && ta_vfs::mods::web_address(&manifest.source) {
            links.push((t("MOD PAGE").to_string(), manifest.source.trim().to_string()));
        }
        links.extend(manifest.links.iter().filter(|(label, url)| !label.trim().is_empty() && ta_vfs::mods::web_address(url))
            .map(|(label, url)| (label.trim().to_uppercase(), url.trim().to_string())));
        if !links.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(14.0, 6.0);
                for (label, url) in links {
                    // (Links, to screen readers and the keys too: underlined as the keys come.)
                    let response = ui.link(RichText::new(format!("{label} ↗")).font(strong(12.0)).color(AMBER)).hover_tip(url.as_str());
                    widgets::reveal_focus(ui, &response);
                    if response.clicked() { actions.push(Action::OpenUrl(url)); }
                }
            });
        }
        if !manifest.theme.is_empty() {
            self.mod_theme_preview(ui, &info);
        }
        let count = crate::mod_media::ModMedia::screenshots(&info);
        if count == 0 { return; }
        ui.add_space(space::XS);
        ui.label(widgets::caption("SCREENSHOTS"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for index in 0..count {
                let (rect, _) = ui.allocate_exact_size(vec2(112.0, 63.0), Sense::hover());
                let id = Id::new(("mod_screenshot", &item.id, index));
                let response = ui.interact(rect, id, Sense::click());
                let shot = self.mod_media.borrow_mut().screenshot(&info, index, false);
                let label = tf("Screenshot {n} of {count}", &[("n", &(index + 1)), ("count", &count)]);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
                widgets::reveal_focus(ui, &response);
                library::picture(ui, rect, shot.texture(), shot.failed());
                if response.hovered() || response.has_focus() { ui.painter().rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.5, AMBER), StrokeKind::Inside); }
                widgets::focus_ring(ui, &response, rect);
                let response = response.on_hover_cursor(egui::CursorIcon::ZoomIn);
                // (One that cannot be shown says so, to the pointer and to screen readers.)
                let response = if shot.failed() {
                    ui.ctx().accesskit_node_builder(response.id, |node| node.set_description(t("This picture could not be shown.")));
                    response.hover_tip(t("This picture could not be shown."))
                } else { response };
                if response.clicked() { self.mod_lightbox = Some(crate::mod_media::Lightbox::new(&item.id, index, Some(id))); }
            }
        });
    }

    /// A mod's interface theme, as its battles' HUD shows it: its top bar (its picture, or its
    /// panels' colour; the side's name in its accent, its metal and energy), its panels' picture,
    /// its colours (each named, with its value), the sides of its own.
    fn mod_theme_preview(&mut self, ui: &mut Ui, info: &ta_vfs::mods::ModInfo) {
        let theme = &info.manifest.theme;
        let p = ta_ui::theme::Palette::REBORN.themed(&|key| theme.colour("", key).and_then(ta_ui::theme::parse_colour));
        ui.add_space(space::XS);
        ui.label(widgets::caption("INTERFACE THEME"));
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(460.0), 46.0), Sense::hover());
        let painter = ui.painter_at(rect);
        let (bar, panel) = {
            let mut media = self.mod_media.borrow_mut();
            (media.theme_picture(info, "bar"), media.theme_picture(info, "panel"))
        };
        // (As the HUD draws its pictures, a texel a point: the bar's repeated across and
        // stretched to the bar's height, its first and last rows' middles at the ends; the
        // panels' repeated both ways.)
        match &bar {
            Some(bar) => {
                let size = bar.size_vec2().max(vec2(1.0, 1.0));
                let uv = Rect::from_min_max(pos2(0.0, 0.5 / size.y), pos2(rect.width() / size.x, 1.0 - 0.5 / size.y));
                painter.image(bar.id(), rect, uv, Color32::WHITE);
            }
            None => { painter.rect_filled(rect, CornerRadius::same(2), p.panel); }
        }
        painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, p.line_hi), StrokeKind::Inside);
        // Its side's name, its metal and its energy, as the HUD's top bar has them.
        let y = rect.center().y;
        let mut x = rect.left() + 12.0;
        let name = painter.layout_no_wrap("ARM".into(), display(20.0), p.accent);
        let width = name.size().x;
        painter.galley(pos2(x, y - name.size().y / 2.0), name, p.accent);
        x += width + 16.0;
        let bars = [(t("METAL"), p.metal_bar, 0.7f32), (t("ENERGY"), p.energy_bar, 0.45)].map(|(label, colour, fill)| (painter.layout_no_wrap(label.to_string(), strong(11.0), p.steel), colour, fill));
        // (The two bars as wide as each other in what the words leave.)
        let words: f32 = bars.iter().map(|(words, _, _)| words.size().x + 8.0).sum();
        let width = ((rect.right() - 12.0 - x - words - 14.0) / 2.0).clamp(30.0, 140.0);
        for (words, colour, fill) in bars {
            let w = words.size().x;
            painter.galley(pos2(x, y - words.size().y / 2.0), words, p.steel);
            x += w + 8.0;
            let bar = Rect::from_min_size(pos2(x, y - 6.0), vec2(width, 12.0));
            if bar.right() > rect.right() - 8.0 { break; }
            painter.rect_filled(bar, CornerRadius::same(2), p.inset);
            painter.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * fill, bar.height())), CornerRadius::same(2), colour);
            x = bar.right() + 14.0;
        }
        // Its panels' picture, as on the command deck's plates: lit along its top and left edges,
        // shaded opposite.
        if let Some(panel) = &panel {
            let (plate, _) = ui.allocate_exact_size(vec2(rect.width(), 30.0), Sense::hover());
            let painter = ui.painter_at(plate);
            let size = panel.size_vec2().max(vec2(1.0, 1.0));
            painter.image(panel.id(), plate, Rect::from_min_max(pos2(0.0, 0.0), pos2(plate.width() / size.x, plate.height() / size.y)), Color32::WHITE);
            painter.hline(plate.x_range(), plate.min.y + 0.5, Stroke::new(1.0, Color32::from_white_alpha(48)));
            painter.vline(plate.min.x + 0.5, plate.y_range(), Stroke::new(1.0, Color32::from_white_alpha(24)));
            painter.hline(plate.x_range(), plate.max.y - 0.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
            painter.vline(plate.max.x - 0.5, plate.y_range(), Stroke::new(1.0, Color32::from_black_alpha(130)));
        }
        // Its colours, each with its name in mod.toml and its value (to screen readers too).
        let colours = [("accent", p.accent), ("text", p.text), ("dim", p.steel), ("panel", p.panel), ("metal", p.metal_bar), ("energy", p.energy_bar), ("good", p.good), ("bad", p.bad), ("info", p.info)];
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(12.0, 5.0);
            for (key, colour) in colours {
                let words = format!("{key} #{:02x}{:02x}{:02x}", colour.r(), colour.g(), colour.b());
                let text = widgets::galley(ui, RichText::new(&words).font(mono(11.0)).color(STEEL));
                let (at, response) = ui.allocate_exact_size(vec2(18.0 + 6.0 + text.size().x, 18.0), Sense::hover());
                let chip = Rect::from_min_size(at.min, vec2(18.0, 18.0));
                ui.painter().rect(chip, CornerRadius::same(3), colour, Stroke::new(1.0, LINE_HI), StrokeKind::Inside);
                ui.painter().galley(pos2(chip.right() + 6.0, at.center().y - text.size().y / 2.0), text, STEEL);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &words));
            }
            // (Sides with colours of their own: each in its accent.)
            for (side, look) in &theme.sides {
                let accent = look.colour("accent").and_then(ta_ui::theme::parse_colour).unwrap_or(p.accent);
                ui.label(RichText::new(side.to_uppercase()).font(strong(11.5)).color(accent));
            }
        });
        use ta_data::words::{Fill, Words};
        let mut extras = Vec::new();
        if !theme.font.trim().is_empty() { extras.push(Fill::Words(Words::text(n("its own font")))); }
        if theme.texture("", "panel").is_some() { extras.push(Fill::Words(Words::text(n("pictures behind its panels")))); }
        // (Listed as the language lists things: "A and B".)
        let listed = Words { text: "{extras}".into(), fills: vec![("extras".into(), Fill::List(extras.clone()))], raw: true }.render(&|text| t(text).to_string());
        let what = if extras.is_empty() { t("Battles played with this mod show the HUD in these colours.").to_string() }
            else { tf("Battles played with this mod show the HUD in these colours, with {extras}.", &[("extras", &listed)]) };
        ui.add(egui::Label::new(RichText::new(what).font(body(12.5)).color(STEEL)).wrap())
            .hover_tip(t("A player may turn mods' themes off: MODS' INTERFACE THEMES, in the game's Interface options."));
    }

    /// A mod's screenshots on the whole window, while one is chosen: a dialog over the page (which
    /// takes no clicks, keys or focus under it), the keys put on its close button as it opens;
    /// the arrows (and the keys ← →) step through them; Escape, the cross or a click beside the
    /// picture closes it, the keys going back to the screenshot it opened from. A question the
    /// launcher asks meanwhile stands over it, and takes the keys. It closes too as the details
    /// show another mod.
    fn mod_lightbox_ui(&mut self, ui: &mut Ui) {
        let Some(lightbox) = self.mod_lightbox.clone() else { return };
        let ctx = ui.ctx().clone();
        let info = (self.mods_tab == ModsTab::Library && self.mod_library.selected == lightbox.id)
            .then(|| self.mods.get(&lightbox.id).map(|entry| entry.info.clone())).flatten();
        let count = info.as_ref().map_or(0, crate::mod_media::ModMedia::screenshots);
        let Some(info) = info.filter(|_| count > 0) else {
            self.close_lightbox(&ctx, None);
            return;
        };
        let index = lightbox.index.min(count - 1);
        // The keys its own, unless a question stands over it.
        let layer = egui::LayerId::new(egui::Order::Foreground, Id::new("mod_lightbox"));
        let on_top = ctx.memory(|memory| memory.top_modal_layer().is_none_or(|top| top == layer))
            && !self.question_open() && !egui::Popup::is_any_open(&ctx);
        let (mut close, mut step) = (false, 0i32);
        let mut arrows = false;
        if on_top {
            ctx.input_mut(|input| {
                // (Seen before they are taken: egui moves the focus with them otherwise, ← and → too.)
                arrows = input.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::ArrowUp | egui::Key::ArrowDown | egui::Key::ArrowLeft | egui::Key::ArrowRight, pressed: true, .. }));
                close |= input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) { step -= 1; }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) { step += 1; }
            });
        }
        let screen = ctx.content_rect();
        let shot = self.mod_media.borrow_mut().screenshot(&info, index, true);
        let title = format!("{} · {}", info.manifest.name.to_uppercase(), tf("{n} of {count}", &[("n", &(index + 1)), ("count", &count)]));
        let mut focused_close = false;
        egui::Area::new(layer.id).order(egui::Order::Foreground).fixed_pos(screen.min).show(&ctx, |ui| {
            ui.set_min_size(screen.size());
            // (To screen readers, a dialog named as its title.)
            ctx.accesskit_node_builder(layer.id.with("move"), |node| {
                node.set_role(egui::accesskit::Role::Dialog);
                node.set_modal();
                node.set_label(title.as_str());
            });
            let painter = ui.painter().clone();
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(232));
            // (The backdrop and the picture take clicks, never the keys.)
            if ui.interact(screen, Id::new("mod_lightbox_back"), Sense::CLICK).clicked() { close = true; }
            let area = Rect::from_min_max(screen.min + vec2(72.0, 56.0), screen.max - vec2(72.0, 40.0));
            // Its title, kept clear of the close button and cut short on a small window; its
            // buttons. (Before what changes from frame to frame, so that they keep the widget
            // ids egui gives them, and the close button the keys.)
            let room = (screen.width() - 2.0 * 64.0).max(1.0);
            ui.put(Rect::from_center_size(pos2(screen.center().x, screen.top() + 28.0), vec2(room, 24.0)),
                egui::Label::new(RichText::new(&title).font(strong(14.0)).color(TEXT_BRIGHT)).truncate());
            let button = |ui: &mut Ui, at: Pos2, glyph: widgets::Glyph| {
                ui.put(Rect::from_center_size(at, vec2(36.0, 36.0)), |ui: &mut Ui| widgets::glyph_button(ui, glyph, vec2(36.0, 36.0), widgets::Tone::Secondary, true))
            };
            let close_button = button(ui, pos2(screen.right() - 30.0, screen.top() + 28.0), widgets::Glyph::Close);
            if lightbox.focus && on_top {
                close_button.request_focus();
                focused_close = true;
            }
            if close_button.clicked() { close = true; }
            if count > 1 {
                for (glyph, x, by, name) in [(widgets::Glyph::Left, screen.left() + 36.0, -1, n("Previous")), (widgets::Glyph::Right, screen.right() - 36.0, 1, n("Next"))] {
                    let response = button(ui, pos2(x, area.center().y), glyph);
                    ctx.accesskit_node_builder(response.id, |node| node.set_label(t(name)));
                    if response.clicked() { step += by; }
                }
            }
            // The picture, as large as fits between the arrows.
            match shot.texture() {
                Some(texture) => {
                    let size = texture.size_vec2();
                    let scale = (area.width() / size.x.max(1.0)).min(area.height() / size.y.max(1.0));
                    let shown = Rect::from_center_size(area.center(), size * scale);
                    painter.image(texture.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    painter.rect_stroke(shown, CornerRadius::ZERO, Stroke::new(1.0, LINE), StrokeKind::Outside);
                    let picture = ui.interact(shown, Id::new("mod_lightbox_picture"), Sense::CLICK);
                    picture.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, tf("Screenshot {n} of {count}", &[("n", &(index + 1)), ("count", &count)])));
                }
                None => {
                    let well = Rect::from_center_size(area.center(), vec2(320.0, 180.0));
                    library::picture(ui, well, None, shot.failed());
                    if shot.failed() {
                        let words = Rect::from_min_size(pos2(well.left() - 80.0, well.bottom() + 10.0), vec2(well.width() + 160.0, 24.0));
                        ui.put(words, egui::Label::new(RichText::new(t("This picture could not be shown.")).color(STEEL)).truncate());
                    }
                }
            }
        });
        // (egui moves the keys at the frame's end to whatever lies the arrows' way, the page
        // behind too: here they step through the screenshots instead.)
        if arrows { ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None)); }
        if close {
            self.close_lightbox(&ctx, lightbox.from);
        } else {
            // Over the page, whose widgets take no clicks, keys or focus under it (from the next
            // frame: egui's modal layer; not set as it closes, so that the page takes them at once).
            ctx.memory_mut(|memory| memory.set_modal_layer(layer));
            self.mod_lightbox = Some(crate::mod_media::Lightbox {
                index: (index as i32 + step).rem_euclid(count as i32) as usize,
                focus: lightbox.focus && !focused_close,
                ..lightbox
            });
        }
    }

    /// The screenshots on the whole window closed, their pictures let go; the keys back on the
    /// screenshot they were opened from (`from`), when they close as asked.
    fn close_lightbox(&mut self, ctx: &egui::Context, from: Option<Id>) {
        self.mod_lightbox = None;
        self.mod_media.get_mut().close_full();
        if let Some(from) = from { ctx.data_mut(|data| data.insert_temp(lightbox_back_id(), from)); }
    }

    /// The Mods library left (another page, the load order): its screenshots on the whole window
    /// closed, what the player asked of a preview forgotten, and what only the library shows let
    /// go (the previews, read and being read, and the pictures on the whole window).
    pub(crate) fn leave_mod_library(&mut self) {
        self.mod_lightbox = None;
        self.preview_choice = None;
        self.mod_media.get_mut().release();
    }

    /// What an installed mod changes, its documents and where it is.
    fn mod_files(&mut self, ui: &mut Ui, item: &Item, actions: &mut Vec<Action>) {
        let Some(entry) = self.mods.get(&item.id).cloned() else { return };
        let on = self.config.mods.enabled.contains(&item.id);
        // (Its check in brief, once there is one: a click opens the report.)
        if let Some(saved) = self.mod_validation.report(&item.id) {
            ui.add_space(space::S);
            ui.label(widgets::caption("COMPATIBILITY"));
            if super::mod_report::report_brief(ui, saved) { actions.push(Action::ShowModReport(item.id.clone())); }
        }
        ui.add_space(space::S);
        self.mod_changes(ui, &item.id, on);
        ui.add_space(space::XS);
        let place = if entry.is_archive() { entry.info.path.parent().map(|p| p.to_path_buf()) } else { Some(entry.info.path.clone()) };
        ui.label(widgets::caption(&ta_ui::i18n::upper(folder_label(self.mods.folder_label(&entry)))));
        path_label(ui, &entry.info.path, STEEL);
        if let Some(place) = place && widgets::text_link(ui, "SHOW SOURCE FILES").hover_tip(place.display().to_string()).clicked() {
            actions.push(Action::OpenPath(place));
        }
    }

    /// What the mod changes of the game and of the other mods on (worked out in the
    /// background), and its documents.
    fn mod_changes(&mut self, ui: &mut Ui, id: &str, on: bool) {
        let request = mod_library::ChangesRequest {
            game_dir: self.detection.install().map(|install| install.root.clone()),
            builtin: ta_vfs::builtin_dir(),
            mods: self.mods.mods.iter().map(|entry| entry.info.clone()).collect(),
            order: match &self.mods.resolution { Resolution::Ok(order) => order.clone(), Resolution::Err(_) => self.config.mods.enabled.clone() },
            target: id.to_string(),
        };
        let key = request.key();
        ui.label(widgets::caption("WHAT IT CHANGES"));
        match self.mod_library.changes.get(move || request, &key) {
            None => widgets::busy_line(ui, t("Working it out…")),
            Some(Err(error)) => { super::wrapped(ui, &sentence(error), RED); }
            Some(Ok(changes)) => {
                if !on { ui.label(RichText::new(t("When it is on:")).color(STEEL)); }
                ui.label(RichText::new(tf("Its files: {n}", &[("n", &changes.files)])).color(TEXT));
                if changes.overrides.is_empty() && changes.loses_to.is_empty() && changes.replaces.is_empty() {
                    ui.label(RichText::new(t("It replaces nothing of the game's or of the other mods'.")).color(STEEL));
                }
                if !changes.replaces.is_empty() {
                    let archives = super::listed(&changes.replaces);
                    super::wrapped(ui, &tf("Archives of {source} it takes the place of: {archives}", &[("source", &t("Total Annihilation")), ("archives", &archives)]), TEXT)
                        .hover_tip(t("With the mod on, these are not read at all, as installing the mod in TA's folder overwrote them"));
                }
                for (source, count) in &changes.overrides {
                    ui.label(RichText::new(tf("Files of {source} it replaces: {n}", &[("n", count), ("source", source)])).color(TEXT));
                }
                for (source, count) in &changes.loses_to {
                    ui.label(RichText::new(tf("Its files {source} replaces (loaded later): {n}", &[("n", count), ("source", source)])).color(AMBER));
                }
                if !changes.docs.is_empty() {
                    ui.add_space(space::XS);
                    ui.label(widgets::caption("DOCUMENTS"));
                    let docs = changes.docs.clone();
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(14.0, 4.0);
                        for path in docs {
                            let name = path.file_name().map_or(String::new(), |name| name.to_string_lossy().to_string());
                            if ui.link(RichText::new(name).font(body(12.5))).hover_tip(path.display().to_string()).clicked() { self.push(Action::OpenPath(path)); }
                        }
                    });
                }
            }
        }
    }

    /// The folders mods are looked for in, in a dialog.
    pub(crate) fn mod_folders_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_mod_folders { return; }
        let folders = self.mods.folders.clone();
        let extra = self.config.mods.extra_dir.clone();
        let mut actions = Vec::new();
        let width = (ctx.content_rect().width() - 80.0).clamp(1.0, 640.0);
        let height = (ctx.content_rect().height() - 24.0).max(1.0);
        let title = t("MOD FOLDERS").to_string();
        let response = ta_ui::panel::modal(ctx, "launcher_mod_folders", Panel::new(&title).exit(Exit::Close).width(width).max_height(height), true, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add(egui::Label::new(RichText::new(t("Mods are found in these folders, looked through in this order: where two hold a mod of the same id, the first one wins.")).color(TEXT)).wrap());
            ui.add_space(space::S);
            for folder in &folders {
                ui.horizontal_wrapped(|ui| {
                    ui.label(widgets::caption(&ta_ui::i18n::upper(folder_label(folder.label))));
                    widgets::tag(ui, if folder.exists { "FOUND" } else { "NOT CREATED" }, if folder.exists { GREEN } else { STEEL });
                    // The program's own folder may need admin rights, so it is never created here.
                    let can_open = folder.exists || folder.label != modlist::APP_FOLDER_LABEL;
                    if ui.add_enabled(can_open, ActionButton::secondary("OPEN").height(26.0)).clicked() { actions.push(Action::OpenFolder(folder.path.clone())); }
                });
                path_label(ui, &folder.path, if folder.exists { TEXT } else { STEEL });
                ui.add_space(space::S);
            }
            if extra.is_none() {
                ui.label(widgets::caption("EXTRA FOLDER"));
                ui.add(egui::Label::new(RichText::new(t("Not set — add one to use mods stored elsewhere")).color(STEEL)).wrap());
            }
            ui.add_space(space::S);
            ui.horizontal_wrapped(|ui| {
                let label = if extra.is_some() { n("CHANGE EXTRA FOLDER…") } else { n("ADD EXTRA FOLDER…") };
                if ui.add(ActionButton::secondary(label)).clicked() { actions.push(Action::BrowseExtraModsDir); }
                if extra.is_some() && ui.add(ActionButton::secondary("REMOVE EXTRA FOLDER")).clicked() { actions.push(Action::ClearExtraModsDir); }
            });
        });
        if response.exit { self.show_mod_folders = false; }
        for action in actions { self.push(action); }
    }

    /// Before a second total conversion goes on: turn the other off, or keep both.
    pub(crate) fn conversion_dialog(&mut self, ctx: &egui::Context) {
        let Some((id, others)) = self.conversion_prompt.clone() else { return };
        let name = |id: &str| crate::catalogue::get(id).map_or(id.to_string(), |entry| entry.name.to_string());
        let (new, old) = (name(&id), super::listed(&others.iter().map(|id| name(id)).collect::<Vec<_>>()));
        let title = tf("TURN ON {name}?", &[("name", &ta_ui::i18n::upper(&new))]);
        let mut choice = None;
        let width = (ctx.content_rect().width() - 80.0).clamp(1.0, 560.0);
        let response = ta_ui::panel::modal(ctx, "launcher_conversion", Panel::new(&title).exit(Exit::Close).width(width).accent(AMBER), true, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add(egui::Label::new(RichText::new(tf("{name} remakes the whole game, as {others} does: such mods are rarely made to be played together.", &[("name", &new), ("others", &old)])).color(TEXT)).wrap());
            ui.add_space(space::XS);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if ui.add(ActionButton::primary(&tf("SWITCH TO {name}", &[("name", &ta_ui::i18n::upper(&new))]))).hover_tip(tf("Turn {others} off", &[("others", &old)])).clicked() { choice = Some(true); }
                if ui.add(ActionButton::secondary("KEEP BOTH ON")).clicked() { choice = Some(false); }
                if ui.add(ActionButton::secondary("CANCEL")).clicked() { self.conversion_prompt = None; }
            });
        });
        if let Some(switch) = choice {
            self.conversion_prompt = None;
            if switch { modlist::turn_on_conversion(&mut self.config.mods.enabled, &id); }
            else { modlist::set_enabled(&mut self.config.mods.enabled, &id, true); }
        } else if response.exit {
            self.conversion_prompt = None;
        }
    }
}

/// Where the screenshots on the whole window keep, as they close, the screenshot of the details'
/// row to give the keys back to.
fn lightbox_back_id() -> Id {
    Id::new("mod_lightbox_back_to")
}

/// A mod folder's label, translated (`ta_vfs` names them in English).
pub(super) fn folder_label(label: &str) -> &str {
    match label {
        "Program folder" => t("Program folder"),
        "User folder" => t("User folder"),
        "Extra folder" => t("Extra folder"),
        other => other,
    }
}

/// A mod's mark as hidden, in capitals. (Its own word: a map's HIDDEN agrees with a map where
/// languages mark it, and a mod is not always of a map's gender.)
pub(super) fn hidden_label() -> String {
    ta_ui::i18n::upper(t("Hidden"))
}

/// A hidden mod's way back, on a dark plate over its picture (a card's top right corner, a row's
/// small picture): an open eye and UNHIDE, brass under the pointer. Whether it was clicked.
fn unhide_button(ui: &mut Ui, card: Id, rect: Rect, view: View) -> bool {
    let label = t("UNHIDE");
    let text = unhide_text(ui);
    let size = vec2(unhide_width(ui), 24.0);
    let spot = match view {
        View::Grid => Rect::from_min_size(pos2(rect.right() - 12.0 - size.x, rect.top() + 12.0), size),
        View::List => {
            let side = rect.height() - 10.0;
            Rect::from_center_size(pos2(rect.left() + 8.0 + side * 0.9, rect.center().y), size)
        }
    };
    let response = ui.interact(spot, card.with("unhide"), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    widgets::reveal_focus(ui, &response);
    if ui.is_rect_visible(spot) {
        let hot = response.hovered() || response.has_focus();
        let painter = ui.painter();
        painter.rect_filled(spot, 3.0, Color32::from_black_alpha(if hot { 225 } else { 190 }));
        painter.rect_stroke(spot, 3.0, Stroke::new(1.0, if hot { AMBER } else { AMBER_DIM }), StrokeKind::Inside);
        let colour = if hot { AMBER_BRIGHT } else { TEXT };
        menu::paint_icon(painter, Icon::Eye(true), pos2(spot.left() + 14.0, spot.center().y), colour);
        painter.galley_with_override_text_color(pos2(spot.left() + 25.0, spot.center().y - text.size().y / 2.0), text, colour);
        widgets::focus_ring(ui, &response, spot);
    }
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand).hover_tip(t("Put this mod back in the library"));
    let clicked = response.clicked();
    if clicked { ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle); }
    clicked
}

/// A button in a banner's lower right corner (the details' PLAY PREVIEW and STOP PREVIEW), as the
/// widget `id`: a dark plate, `glyph` and `label` (translated here), brass under the pointer or
/// with the keys; named for screen readers, kept in sight and ringed when the keys come to it.
/// `painter` paints in the banner.
fn banner_button(ui: &Ui, painter: &egui::Painter, frame: Rect, id: Id, glyph: Glyph, label: &str) -> egui::Response {
    let label = t(label);
    let text = widgets::galley(ui, RichText::new(label).font(strong(11.5)).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
    let size = vec2(text.size().x + 40.0, 28.0);
    let rect = Rect::from_min_size(frame.right_bottom() - size - vec2(10.0, 10.0), size);
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    widgets::reveal_focus(ui, &response);
    if response.clicked() { ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Button); }
    let hot = response.hovered() || response.has_focus();
    painter.rect(rect, CornerRadius::same(3), Color32::from_black_alpha(if hot { 230 } else { 190 }), Stroke::new(1.0, if hot { AMBER } else { STEEL_DIM }), StrokeKind::Inside);
    widgets::paint_glyph(painter, glyph, pos2(rect.left() + 16.0, rect.center().y), 0.75, AMBER_BRIGHT);
    painter.galley(pos2(rect.left() + 30.0, rect.center().y - text.size().y / 2.0), text, TEXT_BRIGHT);
    widgets::focus_ring(ui, &response, rect);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn unhide_text(ui: &Ui) -> std::sync::Arc<egui::Galley> {
    widgets::galley(ui, RichText::new(t("UNHIDE")).font(strong(11.0)).extra_letter_spacing(ta_ui::i18n::tracking(0.8)))
}

/// How wide a card's UNHIDE is: its eye and its word.
fn unhide_width(ui: &Ui) -> f32 {
    unhide_text(ui).size().x + 34.0
}

/// A tab over a page: its label in large letters, underlined in brass when it is the one shown.
fn page_tab(ui: &mut Ui, label: &str, selected: bool) -> egui::Response {
    let text = widgets::galley(ui, RichText::new(label).font(display(20.0)).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
    let (rect, response) = ui.allocate_exact_size(text.size() + vec2(4.0, 12.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), selected, label));
    widgets::reveal_focus(ui, &response);
    let hot = response.hovered();
    let color = if selected { TEXT_BRIGHT } else if hot { TEXT } else { STEEL };
    ui.painter().galley(rect.min + vec2(2.0, 0.0), text, color);
    let line = Rect::from_min_size(pos2(rect.min.x, rect.max.y - 3.0), vec2(rect.width(), 3.0));
    if selected {
        ui.painter().rect_filled(line, CornerRadius::ZERO, AMBER);
    } else if hot {
        ui.painter().rect_filled(line, CornerRadius::ZERO, LINE_HI);
    }
    widgets::focus_ring(ui, &response, rect.expand(2.0));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(shape: &egui::Shape, text: &mut String) {
        match shape {
            egui::Shape::Text(item) => { text.push_str(&item.galley.job.text); text.push('\n'); }
            egui::Shape::Vec(items) => for item in items { collect(item, text); },
            _ => {}
        }
    }

    /// The library shows the catalogue and the player's mods; a card opens its details on a
    /// narrow page; a second total conversion asks first; the load order moves with the keys.
    #[test]
    fn the_library_shows_every_mod_and_the_load_order_moves_with_the_keys() {
        use clap::Parser;
        let dir = std::env::temp_dir().join(format!("ta_mod_library_page_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (id, name) in [("naval", "Naval Additions"), ("tweaks", "Ship Tweaks")] {
            std::fs::create_dir_all(dir.join("mods").join(id)).unwrap();
            std::fs::write(dir.join("mods").join(id).join("mod.toml"), format!("id = \"{id}\"\nname = \"{name}\"\n")).unwrap();
        }
        let ctx = egui::Context::default();
        let config = dir.join("config.toml");
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        app.config.mods.enabled = vec!["naval".into(), "tweaks".into(), "missing-mod".into()];
        app.mods.refresh(&app.config.mods.enabled);
        let narrow = |events: Vec<egui::Event>| egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(700.0, 900.0))), events, ..Default::default() };
        let run = |app: &mut LauncherApp, events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(narrow(events), |ui| app.mods_page(ui));
            output.textures_delta.clear();
            let mut text = String::new();
            for shape in output.shapes { collect(&shape.shape, &mut text); }
            text
        };
        run(&mut app, Vec::new());
        let text = run(&mut app, Vec::new());
        assert!(text.contains("Total Mayhem"), "the catalogue's mods lead");
        // The player's own come after them: choosing one brings it into view.
        app.mod_library.choose("naval");
        run(&mut app, Vec::new());
        let text = run(&mut app, Vec::new());
        assert!(text.contains("Naval Additions"));
        // A card opens its details in the library's place.
        let card = ctx.read_response(Id::new(("mod_card", "naval"))).expect("the mod's card is drawn");
        let at = card.rect.center();
        for pressed in [true, false] {
            run(&mut app, vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }]);
        }
        assert!(app.mod_library.details_open && app.mod_library.selected == "naval");
        let text = run(&mut app, Vec::new());
        assert!(text.contains("WHAT IT CHANGES"));
        // A second total conversion asks first; a mod of the player's own does not.
        app.config.mods.enabled.push("escalation".into());
        app.enable_mod("totalmayhem");
        assert_eq!(app.conversion_prompt, Some(("totalmayhem".to_string(), vec!["escalation".to_string()])));
        app.conversion_prompt = None;
        app.config.mods.enabled.retain(|id| id != "escalation");
        app.config.mods.enabled.retain(|id| id != "tweaks");
        app.enable_mod("tweaks");
        assert!(app.conversion_prompt.is_none() && app.config.mods.enabled.iter().any(|id| id == "tweaks"));
        // The load order: Alt with the arrow keys moves the mod with the keys; a mod that is not
        // installed stays listed until it is taken out.
        app.mods_tab = ModsTab::LoadOrder;
        app.mods.refresh(&app.config.mods.enabled);
        let text = run(&mut app, Vec::new());
        assert!(text.contains("missing-mod") && text.contains("NOT FOUND"));
        assert_eq!(app.config.mods.enabled, ["naval", "missing-mod", "tweaks"]);
        ctx.memory_mut(|memory| memory.request_focus(Id::new(("load_order_row", "naval"))));
        run(&mut app, Vec::new());
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT };
        run(&mut app, vec![key(egui::Key::ArrowDown)]);
        assert_eq!(app.config.mods.enabled, ["missing-mod", "naval", "tweaks"]);
        run(&mut app, vec![key(egui::Key::End)]);
        assert_eq!(app.config.mods.enabled, ["missing-mod", "tweaks", "naval"]);
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Mods made for other engines in the original game's folder are offered at the top of the
    /// page until they are added; ADD AS A MOD installs one, its files linked from there.
    #[test]
    fn the_game_folders_mods_are_offered_until_added() {
        use clap::Parser;
        let dir = std::env::temp_dir().join(format!("ta_mod_page_game_folder_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let game = dir.join("game");
        ta_vfs::test_archive::write(&game.join("TAESC.gp3"), &[("unitse/armcom.fbi", b"escalation"), ("guie/armcom1.gui", b"escalation")]);
        let ctx = egui::Context::default();
        let config = dir.join("config.toml");
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        app.game_folder_mods = ta_vfs::mods::game_folder_mods(&game);
        let screen = |events: Vec<egui::Event>| egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 900.0))), events, ..Default::default() };
        let run = |app: &mut LauncherApp, events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(screen(events), |ui| app.mods_page(ui));
            output.textures_delta.clear();
            texts(&output)
        };
        run(&mut app, Vec::new());
        let shown = run(&mut app, Vec::new());
        assert!(shown.iter().any(|(text, _)| text == "IN YOUR TOTAL ANNIHILATION FOLDER"), "{shown:?}");
        let add = shown.iter().find(|(text, _)| text == "ADD AS A MOD").map(|(_, rect)| rect.center()).expect("its button");
        for pressed in [true, false] {
            run(&mut app, vec![egui::Event::PointerMoved(add), egui::Event::PointerButton { pos: add, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }]);
        }
        let actions = std::mem::take(&mut app.actions);
        assert!(matches!(actions.as_slice(), [Action::AddGameFolderMod(id)] if id == "escalation"), "{actions:?}");
        app.apply(Action::AddGameFolderMod("escalation".into()), &eframe::Frame::_new_kittest());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while app.installer.stages.get("escalation").is_some_and(Stage::active) && std::time::Instant::now() < deadline {
            app.installer.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(matches!(app.installer.stages.get("escalation"), Some(Stage::Done { .. })), "{:?}", app.installer.stages.get("escalation"));
        app.refresh_mods();
        assert!(app.mods.get("escalation").is_some(), "installed as the catalogue's TA: Escalation");
        assert!(game.join("TAESC.gp3").is_file(), "the game's folder is left as it is");
        run(&mut app, Vec::new());
        assert!(!run(&mut app, Vec::new()).iter().any(|(text, _)| text == "ADD AS A MOD"), "offered no more");
        app.installer.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The text shown and where, from a frame's shapes.
    fn texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
        fn walk(shape: &egui::Shape, into: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(text) => into.push((text.galley.text().to_string(), text.galley.rect.translate(text.pos.to_vec2()))),
                egui::Shape::Vec(shapes) => for shape in shapes { walk(shape, into); },
                _ => {}
            }
        }
        let mut found = Vec::new();
        for shape in &output.shapes { walk(&shape.shape, &mut found); }
        found
    }

    /// A right-click on a mod's card opens its menu (its moves, on or off, its report, its files,
    /// HIDE FROM LIBRARY, removal); HIDE takes it out of the library and its counts, kept in the
    /// settings, and nothing else (it stays on, the load order lists it); HIDDEN lists it with its
    /// UNHIDE, which puts it back.
    #[test]
    fn a_mods_menu_hides_it_and_the_hidden_view_puts_it_back() {
        use clap::Parser;
        let dir = std::env::temp_dir().join(format!("ta_mod_menu_hide_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (id, name) in [("naval", "Naval Additions"), ("tweaks", "Ship Tweaks")] {
            std::fs::create_dir_all(dir.join("mods").join(id)).unwrap();
            std::fs::write(dir.join("mods").join(id).join("mod.toml"), format!("id = \"{id}\"\nname = \"{name}\"\n")).unwrap();
        }
        let ctx = egui::Context::default();
        let config = dir.join("config.toml");
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        app.config.mods.enabled = vec!["naval".into()];
        app.mods.refresh(&app.config.mods.enabled);
        app.mod_library.view = View::List;
        let run = |app: &mut LauncherApp, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 1000.0))), events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| app.mods_page(ui));
            output.textures_delta.clear();
            texts(&output)
        };
        let click = |app: &mut LauncherApp, at: egui::Pos2, button: egui::PointerButton| {
            run(app, vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button, pressed: true, modifiers: Default::default() }]);
            run(app, vec![egui::Event::PointerButton { pos: at, button, pressed: false, modifiers: Default::default() }]);
            run(app, Vec::new());
            run(app, Vec::new())
        };
        let find = |texts: &[(String, Rect)], label: &str| texts.iter().find(|(text, _)| text == label).map(|(_, rect)| *rect);
        let all = mod_library::items(&app.mods).len();
        app.mod_library.choose("naval");
        run(&mut app, Vec::new());
        let shown = run(&mut app, Vec::new());
        assert!(find(&shown, &format!("LIBRARY · {all}")).is_some(), "every mod counted");
        let row = ctx.read_response(Id::new(("mod_card", "naval"))).expect("the mod's row is drawn").rect;
        let shown = click(&mut app, row.center(), egui::PointerButton::Secondary);
        for words in ["Naval Additions", "TURN OFF", "MOVE TO THE TOP", "It is the only mod that is on.", "MOD INFO", "SHOW SOURCE FILES", "HIDE FROM LIBRARY", "REMOVE"] {
            assert!(find(&shown, words).is_some(), "{words} in the menu: {:?}", shown.iter().map(|(t, _)| t).collect::<Vec<_>>());
        }
        assert_eq!(app.mod_library.selected, "naval", "a right-click chooses it");
        let hide = find(&shown, "HIDE FROM LIBRARY").unwrap();
        click(&mut app, hide.center(), egui::PointerButton::Primary);
        let asked: Vec<Action> = std::mem::take(&mut app.actions);
        assert!(matches!(asked.as_slice(), [Action::HideMod(id, true)] if id == "naval"), "{asked:?}");
        app.set_mod_hidden("naval", true);
        assert_eq!(app.config.mods.hidden, ["naval"]);
        assert_eq!(app.config.mods.enabled, ["naval"], "hiding never turns a mod off");
        assert_ne!(app.mod_library.selected, "naval", "another mod is chosen in its place");
        run(&mut app, Vec::new());
        let shown = run(&mut app, Vec::new());
        assert!(find(&shown, &format!("LIBRARY · {}", all - 1)).is_some(), "the library's count leaves it out");
        assert!(find(&shown, "Naval Additions").is_none(), "its row is gone");
        let switch = find(&shown, "HIDDEN · 1").expect("the toolbar lists the hidden");
        // Kept in the settings (the profile's, as the maps' favourites).
        app.store.flush(&mut app.config).unwrap();
        assert_eq!(ta_config::Config::load_from(&app.store.path).mods.hidden, ["naval"]);
        // The load order still lists it, marked.
        app.mods_tab = ModsTab::LoadOrder;
        let order = run(&mut app, Vec::new());
        assert!(find(&order, "Naval Additions").is_some() && find(&order, "HIDDEN").is_some(), "on, and in the load order");
        app.mods_tab = ModsTab::Library;
        run(&mut app, Vec::new());
        let shown = click(&mut app, switch.center(), egui::PointerButton::Primary);
        assert!(app.mod_library.hidden_view && app.mod_library.selected == "naval");
        assert!(find(&shown, "1 of 1 hidden mods").is_some() && find(&shown, "UNHIDE ALL").is_some());
        let row = ctx.read_response(Id::new(("mod_card", "naval"))).expect("its row in the hidden view").rect;
        let unhide = shown.iter().filter(|(text, _)| text == "UNHIDE").map(|(_, rect)| *rect).find(|rect| row.contains(rect.center()))
            .expect("its row's UNHIDE");
        click(&mut app, unhide.center(), egui::PointerButton::Primary);
        let asked: Vec<Action> = std::mem::take(&mut app.actions);
        assert!(matches!(asked.as_slice(), [Action::HideMod(id, false)] if id == "naval"), "{asked:?}");
        app.set_mod_hidden("naval", false);
        assert!(app.config.mods.hidden.is_empty() && !app.mod_library.hidden_view, "the last put back: the library again");
        assert_eq!(app.mod_library.selected, "naval", "at it");
        let shown = run(&mut app, Vec::new());
        assert!(find(&shown, &format!("LIBRARY · {all}")).is_some() && find(&shown, "HIDDEN · 1").is_none());
        // UNHIDE ALL puts every one back.
        app.set_mod_hidden("naval", true);
        app.set_mod_hidden("tweaks", true);
        app.unhide_all_mods();
        assert!(app.config.mods.hidden.is_empty());
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A launcher on the Mods page over a folder of its own whose mods folder holds `mods` (id,
    /// manifest lines after its id and name, the pictures it names made as PNGs of their names).
    fn launcher(name: &str, mods: &[(&str, &str, &[&str])]) -> (egui::Context, LauncherApp, std::path::PathBuf) {
        use clap::Parser;
        let dir = std::env::temp_dir().join(format!("ta_mod_page_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (id, manifest, pictures) in mods {
            let folder = dir.join("mods").join(id);
            std::fs::create_dir_all(folder.join("media")).unwrap();
            std::fs::write(folder.join("mod.toml"), format!("id = \"{id}\"\nname = \"{id} mod\"\n{manifest}")).unwrap();
            for picture in *pictures {
                image::RgbImage::from_pixel(640, 360, image::Rgb([30, 60, 90])).save(folder.join("media").join(picture)).unwrap();
            }
        }
        let ctx = egui::Context::default();
        let config = dir.join("config.toml");
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--page", "mods", "--config", config.to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        app.refresh_mods();
        (ctx, app, dir)
    }

    /// One whole frame of the launcher (as eframe runs it) in a window 1280 × 900: what is shown,
    /// and the name screen readers are given of the widget with the keys.
    fn whole_frame(ctx: &egui::Context, app: &mut LauncherApp, events: Vec<egui::Event>) -> (Vec<(String, Rect)>, Option<String>) {
        frame_of(ctx, app, events, true)
    }

    /// [`whole_frame`], the window in the foreground (`focused`) or in the background.
    fn frame_of(ctx: &egui::Context, app: &mut LauncherApp, events: Vec<egui::Event>, focused: bool) -> (Vec<(String, Rect)>, Option<String>) {
        use eframe::App as _;
        ctx.enable_accesskit();
        let mut frame = eframe::Frame::_new_kittest();
        let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0))), events, focused, ..Default::default() };
        raw.viewports.entry(raw.viewport_id).or_default().focused = Some(focused);
        let mut output = ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx(), &mut frame);
            app.ui(ui, &mut frame);
        });
        output.textures_delta.clear();
        let shown = texts(&output);
        let focused = output.platform_output.accesskit_update.and_then(|tree| {
            let focus = tree.focus;
            tree.nodes.into_iter().find(|(id, _)| *id == focus).and_then(|(_, node)| node.label().or(node.value()).map(str::to_string))
        });
        (shown, focused)
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    fn click(at: Pos2) -> Vec<Vec<egui::Event>> {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        vec![vec![egui::Event::PointerMoved(at), button(true)], vec![button(false)], Vec::new()]
    }

    /// A mod's screenshots on the whole window are a dialog over the page: the keys go to its
    /// close button as it opens, Tab goes round its own buttons only, the arrows step through the
    /// screenshots (never moving the keys onto the page behind), the page's keys (Ctrl+F, the
    /// pages' Ctrl+number) are not taken under it, and Escape closes it, the keys back on the
    /// screenshot it opened from. A question over it takes Escape first. It closes as the details
    /// show another mod, and when the page is left (coming back does not bring it up again).
    #[test]
    fn the_screenshots_on_the_whole_window_are_a_dialog_over_the_page() {
        let shots: &[&str] = &["one.png", "two.png", "three.png"];
        let (ctx, mut app, dir) = launcher("lightbox", &[("shots", "[media]\nscreenshots = [\"media/one.png\", \"media/two.png\", \"media/three.png\"]\n", shots), ("other", "", &[])]);
        app.show_mod_details("shots");
        let thumb = Id::new(("mod_screenshot", "shots", 1));
        for _ in 0..4 { whole_frame(&ctx, &mut app, Vec::new()); }
        let at = ctx.read_response(thumb).expect("the screenshots' row is drawn").rect.center();
        for events in click(at) { whole_frame(&ctx, &mut app, events); }
        assert_eq!(app.mod_lightbox.as_ref().map(|lightbox| (lightbox.id.as_str(), lightbox.index)), Some(("shots", 1)));
        assert!(app.dialog_open() && !app.question_open(), "a dialog of the page's");
        let (shown, focused) = whole_frame(&ctx, &mut app, Vec::new());
        assert!(shown.iter().any(|(text, _)| text == "SHOTS MOD · 2 of 3"), "its title");
        assert_eq!(focused.as_deref(), Some("Close"), "the keys on its close button");
        // Tab goes round its own buttons, never onto the page behind.
        // (Round from the last to the first, egui gives the keys on in the frame after.)
        let mut met = Vec::new();
        for _ in 0..4 {
            whole_frame(&ctx, &mut app, vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
            met.push(whole_frame(&ctx, &mut app, Vec::new()).1);
        }
        assert_eq!(met, [Some("Previous"), Some("Next"), Some("Close"), Some("Previous")].map(|name| name.map(String::from)), "round its own buttons");
        for _ in 0..3 {
            if whole_frame(&ctx, &mut app, Vec::new()).1.as_deref() == Some("Close") { break; }
            whole_frame(&ctx, &mut app, vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
        }
        // The arrows step; the keys stay where they are.
        let (_, focused) = whole_frame(&ctx, &mut app, vec![key(egui::Key::ArrowRight, egui::Modifiers::NONE)]);
        let (_, after) = whole_frame(&ctx, &mut app, Vec::new());
        assert_eq!(app.mod_lightbox.as_ref().map(|lightbox| lightbox.index), Some(2));
        assert_eq!((focused.as_deref(), after.as_deref()), (Some("Close"), Some("Close")), "the keys not moved by the arrows");
        whole_frame(&ctx, &mut app, vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)]);
        assert_eq!(whole_frame(&ctx, &mut app, Vec::new()).1.as_deref(), Some("Close"), "nor by the arrows up and down");
        // With the keys on Next, ← steps back and they stay on Next: egui's own move to the button
        // on its left is cancelled for ← and → too, which the screenshots take.
        for _ in 0..4 {
            if whole_frame(&ctx, &mut app, Vec::new()).1.as_deref() == Some("Next") { break; }
            whole_frame(&ctx, &mut app, vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
        }
        assert_eq!(whole_frame(&ctx, &mut app, Vec::new()).1.as_deref(), Some("Next"));
        let (_, focused) = whole_frame(&ctx, &mut app, vec![key(egui::Key::ArrowLeft, egui::Modifiers::NONE)]);
        let (_, after) = whole_frame(&ctx, &mut app, Vec::new());
        assert_eq!(app.mod_lightbox.as_ref().map(|lightbox| lightbox.index), Some(1), "stepped back");
        assert_eq!((focused.as_deref(), after.as_deref()), (Some("Next"), Some("Next")), "the keys not moved by ←");
        // The page's keys are not taken under it.
        whole_frame(&ctx, &mut app, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        assert!(!ctx.memory(|memory| memory.has_focus(Id::new("mod_library_search"))), "Ctrl+F finds nothing behind it");
        whole_frame(&ctx, &mut app, vec![key(egui::Key::Num1, egui::Modifiers::COMMAND)]);
        assert_eq!(app.page, crate::app::Page::Mods, "Ctrl+1 changes no page under it");
        // A question over it takes Escape first; then it closes, the keys back on its screenshot.
        app.show_shortcuts = true;
        for _ in 0..2 { whole_frame(&ctx, &mut app, Vec::new()); }
        whole_frame(&ctx, &mut app, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
        assert!(!app.show_shortcuts && app.mod_lightbox.is_some(), "the question closed, the screenshots still open");
        for _ in 0..2 { whole_frame(&ctx, &mut app, Vec::new()); }
        whole_frame(&ctx, &mut app, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
        assert!(app.mod_lightbox.is_none(), "Escape closes it");
        whole_frame(&ctx, &mut app, Vec::new());
        assert!(ctx.memory(|memory| memory.has_focus(thumb)), "the keys back on the screenshot it opened from");
        // Another mod's details close it.
        for events in click(ctx.read_response(thumb).unwrap().rect.center()) { whole_frame(&ctx, &mut app, events); }
        assert!(app.mod_lightbox.is_some());
        app.show_wanted_mod("other");
        whole_frame(&ctx, &mut app, Vec::new());
        assert!(app.mod_lightbox.is_none(), "another mod shown: closed");
        // Leaving the page closes it; coming back does not bring it up again.
        app.show_mod_details("shots");
        for _ in 0..3 { whole_frame(&ctx, &mut app, Vec::new()); }
        for events in click(ctx.read_response(thumb).unwrap().rect.center()) { whole_frame(&ctx, &mut app, events); }
        assert!(app.mod_lightbox.is_some());
        app.open_page(crate::app::Page::Play);
        whole_frame(&ctx, &mut app, Vec::new());
        app.open_page(crate::app::Page::Mods);
        whole_frame(&ctx, &mut app, Vec::new());
        assert!(app.mod_lightbox.is_none() && !app.dialog_open(), "left with the page");
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// With motion reduced, a mod's preview waits in its details for PLAY PREVIEW (a button by that
    /// name, the keys' to reach), which becomes STOP PREVIEW as it plays; STOP forgets it, and so
    /// do another mod and leaving the library. Without, it plays of itself, and STOP stops it.
    #[test]
    fn a_preview_plays_and_stops_at_the_players_word() {
        let (ctx, mut app, dir) = launcher("preview", &[("moving", "[media]\npreview = \"media/moving.gif\"\n", &[]), ("other", "", &[])]);
        let gif = dir.join("mods/moving/media/moving.gif");
        let mut encoder = image::codecs::gif::GifEncoder::new(std::fs::File::create(&gif).unwrap());
        encoder.encode_frames((0..4).map(|n| image::Frame::from_parts(image::RgbaImage::from_pixel(64, 36, image::Rgba([n * 60, 0, 0, 255])), 0, 0, image::Delay::from_numer_denom_ms(100, 1)))).unwrap();
        drop(encoder);
        app.config.gameplay.reduced_motion = true;
        app.show_mod_details("moving");
        let button = Id::new(("mod_preview_button", "moving"));
        let find = |shown: &[(String, Rect)], label: &str| shown.iter().any(|(text, _)| text == label);
        let mut shown = Vec::new();
        for _ in 0..200 {
            shown = whole_frame(&ctx, &mut app, Vec::new()).0;
            if find(&shown, "PLAY PREVIEW") { break; }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(find(&shown, "PLAY PREVIEW"), "motion reduced: it waits to be asked");
        // The keys reach it, by its name.
        ctx.memory_mut(|memory| memory.request_focus(button));
        let (_, focused) = whole_frame(&ctx, &mut app, Vec::new());
        assert_eq!(focused.as_deref(), Some("PLAY PREVIEW"));
        whole_frame(&ctx, &mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(app.preview_choice, Some(("moving".to_string(), true)), "asked to play");
        let shown = whole_frame(&ctx, &mut app, Vec::new()).0;
        assert!(find(&shown, "STOP PREVIEW") && !find(&shown, "PLAY PREVIEW"), "it can be stopped as it plays");
        whole_frame(&ctx, &mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(app.preview_choice, None, "stopped: forgotten");
        // Asked again, then another mod chosen, or the library left: forgotten.
        whole_frame(&ctx, &mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert!(app.preview_choice.is_some());
        app.show_mod_details("other");
        whole_frame(&ctx, &mut app, Vec::new());
        assert_eq!(app.preview_choice, None, "another mod's details: forgotten");
        app.show_mod_details("moving");
        whole_frame(&ctx, &mut app, Vec::new());
        app.preview_choice = Some(("moving".into(), true));
        app.mods_tab = ModsTab::LoadOrder;
        whole_frame(&ctx, &mut app, Vec::new());
        assert_eq!(app.preview_choice, None, "the library left: forgotten");
        // Motion not reduced: it plays of itself, and STOP PREVIEW stops it.
        app.mods_tab = ModsTab::Library;
        app.config.gameplay.reduced_motion = false;
        let mut shown = Vec::new();
        for _ in 0..200 {
            shown = whole_frame(&ctx, &mut app, Vec::new()).0;
            if find(&shown, "STOP PREVIEW") { break; }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(find(&shown, "STOP PREVIEW"), "it plays of itself");
        let at = ctx.read_response(button).unwrap().rect.center();
        for events in click(at) { whole_frame(&ctx, &mut app, events); }
        assert_eq!(app.preview_choice, Some(("moving".to_string(), false)), "stopped");
        assert!(find(&whole_frame(&ctx, &mut app, Vec::new()).0, "PLAY PREVIEW"));
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// While motion is held (the window in the background), no preview plays or is read, on a
    /// card the pointer rests on or in the details, and the details offer no button for it; in
    /// the foreground it plays, the details offering to stop it.
    #[test]
    fn no_preview_plays_or_is_read_while_motion_is_held() {
        let (ctx, mut app, dir) = launcher("held", &[("moving", "[media]
preview = \"media/moving.gif\"
", &[])]);
        let gif = dir.join("mods/moving/media/moving.gif");
        let mut encoder = image::codecs::gif::GifEncoder::new(std::fs::File::create(&gif).unwrap());
        encoder.encode_frames((0..4u8).map(|n| image::Frame::from_parts(image::RgbaImage::from_pixel(64, 36, image::Rgba([n * 60, 0, 0, 255])), 0, 0, image::Delay::from_numer_denom_ms(100, 1)))).unwrap();
        drop(encoder);
        app.mod_library.query = "moving".into();
        app.show_mod_details("moving");
        let buttons = |shown: &[(String, Rect)]| shown.iter().any(|(text, _)| text == "PLAY PREVIEW" || text == "STOP PREVIEW");
        for _ in 0..5 { frame_of(&ctx, &mut app, Vec::new(), false); }
        let card = ctx.read_response(Id::new(("mod_card", "moving"))).expect("its card").rect.center();
        // (The pointer at rest on its card well past the moment a preview would play.)
        let mut shown = Vec::new();
        for frame in 0..60 {
            let events = if frame == 0 { vec![egui::Event::PointerMoved(card)] } else { Vec::new() };
            shown = frame_of(&ctx, &mut app, events, false).0;
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(app.mod_media.borrow().previews_held(), 0, "nothing read while motion is held");
        assert!(!buttons(&shown), "no button for it either");
        let mut playing = false;
        for _ in 0..300 {
            shown = frame_of(&ctx, &mut app, Vec::new(), true).0;
            playing = app.mod_media.borrow().previews_held() > 0 && buttons(&shown);
            if playing { break; }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(playing && shown.iter().any(|(text, _)| text == "STOP PREVIEW"), "in the foreground it plays, to be stopped");
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A load order that does not resolve offers what puts it right: to turn off either of two
    /// mods that cannot be on together, the mod that needs a newer TA Reborn, and the one that
    /// needs a mod installed nowhere (said to be not installed, nothing offered to turn on).
    #[test]
    fn a_load_order_that_does_not_resolve_offers_to_turn_mods_off() {
        let (ctx, mut app, dir) = launcher("order_fixes", &[("clash-a", "conflicts = [\"clash-b\"]\n", &[]), ("clash-b", "", &[]),
            ("future", "requires_reborn = \"99.0\"\n", &[]), ("lonely", "depends = [\"ghost-mod\"]\n", &[])]);
        app.mods_tab = ModsTab::LoadOrder;
        let shown_with = |app: &mut LauncherApp, enabled: &[&str]| {
            app.config.mods.enabled = enabled.iter().map(|id| id.to_string()).collect();
            whole_frame(&ctx, app, Vec::new());
            whole_frame(&ctx, app, Vec::new()).0
        };
        let has = |shown: &[(String, Rect)], label: &str| shown.iter().any(|(text, _)| text == label);
        let shown = shown_with(&mut app, &["clash-b", "clash-a"]);
        assert!(has(&shown, "TURN OFF CLASH-A MOD") && has(&shown, "TURN OFF CLASH-B MOD"), "either of the two");
        let shown = shown_with(&mut app, &["future"]);
        assert!(has(&shown, "TURN OFF FUTURE MOD"), "the one that needs a newer TA Reborn");
        let shown = shown_with(&mut app, &["lonely"]);
        assert!(shown.iter().any(|(text, _)| text.contains("needs ghost-mod, which is not installed")), "{shown:?}");
        assert!(has(&shown, "TURN OFF LONELY MOD") && !shown.iter().any(|(text, _)| text.starts_with("ENABLE")), "nothing to turn on");
        let at = shown.iter().find(|(text, _)| text == "TURN OFF LONELY MOD").unwrap().1.center();
        for events in click(at) { whole_frame(&ctx, &mut app, events); }
        assert!(app.config.mods.enabled.is_empty(), "turned off");
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn find_focuses_search_and_escape_clears_only_when_shortcuts_are_available() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut query = String::from("naval");
        let input = |key, modifiers| egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 500.0))),
            events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
            ..Default::default()
        };
        let id = egui::Id::new("test_mod_search");
        ctx.run_ui(input(egui::Key::F, egui::Modifiers::COMMAND), |ui| {
            library::search(ui, &mut query, id, "Search mods", 300.0, true);
        }).drop_without_applying_deltas();
        assert!(ctx.memory(|memory| memory.has_focus(id)));
        ctx.run_ui(input(egui::Key::Escape, egui::Modifiers::NONE), |ui| {
            library::search(ui, &mut query, id, "Search mods", 300.0, false);
        }).drop_without_applying_deltas();
        assert_eq!(query, "naval", "a dialog's Escape must not clear the underlying search");
        ctx.memory_mut(|memory| memory.request_focus(id));
        ctx.run_ui(input(egui::Key::Escape, egui::Modifiers::NONE), |ui| {
            library::search(ui, &mut query, id, "Search mods", 300.0, true);
        }).drop_without_applying_deltas();
        assert!(query.is_empty());
    }
}
