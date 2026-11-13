//! The Mods page: one library of the catalogue's mods and those installed, as cards (or the rows
//! of a list) to search, filter and order, the chosen mod's details beside them (in their place
//! on a narrow page): its picture, what it is, what can be done with it and what it changes of
//! the game and of the other mods. A right-click on a card or a row (or Shift+F10) opens what can
//! be done with that mod. Mods the player hid leave the library (and its counts) for its HIDDEN
//! view, as they were otherwise: installed, on or off. The load order of the mods that are on is
//! the page's other view (`mod_order.rs`); the mod folders open in a dialog.

use egui::{Align, Color32, CornerRadius, Id, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
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
        self.mod_library.changes.poll(&ctx);
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
            &["INSTALL FROM FILE…", "MOD FOLDERS…", "RESCAN"], |ui, height| {
            if ui.add(ActionButton::primary("INSTALL FROM FILE…").height(height)).hover_tip(t("Install a mod you have downloaded (or drop it on this window)")).clicked() {
                self.push(Action::BrowsePackage);
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
            let paint = |ui: &Ui, frame: Rect| this.paint_banner(ui, frame, item, false);
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
        let on = self.config.mods.enabled.contains(&item.id);
        let stage = self.installer.stages.get(&item.id).filter(|stage| stage.active());
        let detail = if let Some(stage) = stage {
            getmods::stage_progress(stage).1
        } else if let Some(newer) = update {
            tf("Update: {version}", &[("version", &newer)])
        } else if let Some(entry) = item.catalogue {
            tf("by {team}", &[("team", &entry.team)])
        } else {
            let kind = t(item.kind(&self.mods).label()).to_string();
            match version.filter(|version| !version.is_empty()) {
                Some(version) => format!("{kind} · {version}"),
                None => kind,
            }
        };
        let lamp = if stage.is_some() { Some(SKY) } else if on { Some(GREEN) } else if update.is_some() { Some(AMBER) } else if item.installed { Some(STEEL) } else { None };
        let check = if !item.installed { None } else {
            match self.mod_validation.status(&item.id) {
                crate::validation::ValidationStatus::Ready(ta_data::validation::Status::Errors) => Some((RED, n("ERRORS FOUND"))),
                _ => None,
            }
        };
        let lamps = lamp.into_iter().chain(check.map(|(colour, _)| colour)).collect();
        (detail, lamps, check.map(|(_, words)| words))
    }

    /// A mod's banner: its picture (or a plate of its own with its name), and on the details'
    /// larger one, how it fared (its review, or this copy's check).
    fn paint_banner(&self, ui: &Ui, frame: Rect, item: &Item, badge: bool) {
        let texture = item.catalogue.and_then(|entry| self.media.pictures.get(entry.id));
        let bundled = item.catalogue.is_some_and(|entry| entry.bundled_image().is_some());
        let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(frame));
        getmods::paint_picture(&painter, frame, texture, &item.id, &item.name, bundled, 0.0);
        painter.add(ta_ui::metal::well(frame, Color32::TRANSPARENT));
        if badge {
            let (label, colour) = self.fit_badge(item);
            getmods::badge(ui, &painter, frame.left_top() + vec2(10.0, 10.0), t(label), colour, egui::Align2::LEFT_TOP);
        }
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
        let mut actions: Vec<Action> = Vec::new();
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
                        self.paint_banner(ui, rect, &item, true);
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
                self.paint_banner(ui, rect, &item, true);
                self.mod_heading(ui, &item, &mut actions);
                ui.add_space(space::XS);
                self.mod_about(ui, &item, &mut actions);
                self.mod_files(ui, &item, &mut actions);
            }
        }));
        for action in actions { self.push(action); }
    }

    /// A mod's name and creators, and what can be done with it.
    fn mod_heading(&mut self, ui: &mut Ui, item: &Item, actions: &mut Vec<Action>) {
        ui.add(egui::Label::new(RichText::new(item.name.to_uppercase()).font(display(24.0)).color(TEXT_BRIGHT)
            .extra_letter_spacing(ta_ui::i18n::tracking(1.0))).wrap());
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
                    ui.add(egui::Label::new(RichText::new(tf("by {team}", &[("team", &manifest.authors.join(", "))])).font(strong(12.5)).color(AMBER)).wrap());
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                    let mut toggled = on;
                    if widgets::toggle_named(ui, &mut toggled, &manifest.name).hover_tip(t("Turn the mod on or off for the next game")).changed() {
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
                let shown = if manifest.version.is_empty() { t("Unknown") } else { &manifest.version };
                tags(ui, &|ui: &mut Ui| {
                    widgets::tag(ui, &tf("Version: {version}", &[("version", &shown)]), STEEL);
                    widgets::tag(ui, if entry.is_archive() { "ARCHIVE" } else { "FOLDER" }, STEEL);
                });
                if !manifest.description.is_empty() {
                    ui.add(egui::Label::new(RichText::new(t(&manifest.description)).font(body(13.5)).color(TEXT)).wrap());
                }
                if !manifest.depends.is_empty() {
                    let names: Vec<String> = manifest.depends.iter().map(|id| self.mods.get(id).map_or(id.clone(), |m| m.info.manifest.name.clone())).collect();
                    ui.label(RichText::new(tf("requires {mods}", &[("mods", &names.join(", "))])).color(STEEL));
                }
            }
            (None, None) => {}
        }
    }

    /// What an installed mod changes, its documents and where it is.
    fn mod_files(&mut self, ui: &mut Ui, item: &Item, actions: &mut Vec<Action>) {
        let Some(entry) = self.mods.get(&item.id).cloned() else { return };
        let on = self.config.mods.enabled.contains(&item.id);
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
                    let archives = changes.replaces.join(", ");
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
        let (new, old) = (name(&id), others.iter().map(|id| name(id)).collect::<Vec<_>>().join(", "));
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
