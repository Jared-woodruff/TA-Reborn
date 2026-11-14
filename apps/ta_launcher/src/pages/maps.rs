//! The map library: every map the game can find, as cards with pictures of their own terrain
//! (or the rows of a list), to search, filter and order; the player's favourites first, then TA
//! Reborn's own maps (a brass ribbon on each), then the rest, each under its heading; the chosen
//! map's details beside them (in their place on a narrow page): its large picture with the start
//! positions, its facts and what can be done with it. A card's star (or F) marks a favourite; a
//! right-click on a card or a row (or Shift+F10) opens what can be done with that map.
use egui::{Align, Color32, Id, Layout, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui, pos2, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::tooltip::TooltipExt as _;
use crate::app::{Action, LauncherApp};
use crate::library::menu::{self, Icon, Menu};
use crate::library::{self, Card, Section};
use crate::maps::{self, Entry, Filter, Players, Size, Sort, View};
use crate::theme::*;
use crate::widgets::{self, ActionButton};

/// From this width the details stand beside the library; below it they take its place.
const SPLIT: f32 = 760.0;
/// A card's picture well, over its width.
const WELL: f32 = 0.66;

fn map_menu_id() -> Id {
    Id::new("map_library_menu")
}

/// What a map's context menu does: open its details (in the library's place on a narrow page),
/// or what its buttons do.
#[derive(Clone, Debug)]
enum MapChoice {
    Details,
    Do(Action),
}

impl LauncherApp {
    pub(crate) fn maps_page(&mut self, ui: &mut Ui) {
        let short = ui.available_height() < 600.0;
        let idle = !self.maps.busy() && !self.installer.busy() && !self.game_running();
        self.maps_header(ui, short, idle);
        if let Some(status) = self.maps.status() { widgets::busy_line(ui, t(status)); }
        self.maps_feedback(ui);
        let wide = ui.available_width() >= SPLIT;
        // (A wide page shows the details beside the library: a map opened there stays so.)
        if wide { self.maps.details_open = false; }
        if !wide && self.maps.details_open && !self.maps.selected.is_empty() {
            let back = ta_ui::panel::back_button(ui, "ALL MAPS").clicked();
            let escape = self.search_shortcuts_allowed(ui) && ui.memory(|memory| memory.focused().is_none())
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            if back || escape {
                self.maps.details_open = false;
                self.maps.reveal(self.maps.selected.clone());
            }
            ui.add_space(space::S);
            self.map_details(ui, ui.available_height(), idle);
            return;
        }
        self.maps_toolbar(ui);
        // F marks the chosen map a favourite, or no longer one (not while a field is typed in).
        if self.search_shortcuts_allowed(ui) && !ui.ctx().text_edit_focused() && !self.maps.selected.is_empty()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F)) {
            self.push(Action::Maps(maps::Action::Favourite(self.maps.selected.clone())));
        }
        let visible = self.maps.visible(&self.config.maps);
        self.maps.reconcile_selection(&self.config.maps);
        let height = ui.available_height().max(80.0);
        if wide {
            let details = (ui.available_width() * 0.38).clamp(300.0, 430.0);
            let gap = space::L;
            let library = (ui.available_width() - details - gap).max(1.0);
            let top = ui.cursor().min;
            let left = Rect::from_min_size(top, vec2(library, height));
            let right = Rect::from_min_size(pos2(top.x + library + gap, top.y), vec2(details, height));
            ui.scope_builder(egui::UiBuilder::new().max_rect(left).layout(Layout::top_down(Align::Min)), |ui| self.map_library(ui, &visible, height));
            ui.scope_builder(egui::UiBuilder::new().max_rect(right).layout(Layout::top_down(Align::Min)), |ui| self.map_details(ui, height, idle));
            ui.allocate_rect(left.union(right), Sense::hover());
        } else {
            self.map_library(ui, &visible, height);
        }
        self.map_menu(ui, wide, idle);
    }

    /// The map menu while it is open: built from the map as it is now, what it says done.
    fn map_menu(&mut self, ui: &Ui, wide: bool, idle: bool) {
        let ctx = ui.ctx().clone();
        let Some(key) = menu::target(&ctx, map_menu_id()) else { return };
        let Some(entry) = self.maps.catalogue.entries.iter().find(|entry| entry.key == key).cloned() else {
            menu::close(&ctx, map_menu_id());
            return;
        };
        let entries = self.map_menu_entries(&entry, wide, idle);
        match menu::show(ui, map_menu_id(), &entries) {
            Some(MapChoice::Details) => self.maps.details_open = true,
            Some(MapChoice::Do(action)) => self.push(action),
            None => {}
        }
    }

    /// What can be done with a map, as its details offer it: play it, set up a skirmish on it,
    /// edit a copy; favourite it, offer it in battles or not; its source, its files; remove its
    /// pack. What cannot apply to it is left out (a campaign map is never offered in battles);
    /// what cannot be done now says why.
    fn map_menu_entries(&self, entry: &Entry, wide: bool, idle: bool) -> Menu<MapChoice> {
        let preferences = &self.config.maps;
        let mut menu = Menu::new(entry.title.clone(), format!("{} · {}", map_detail(entry, self.maps.size_of(entry)), t(entry.status(preferences))));
        if !wide {
            menu.add(n("VIEW DETAILS"), Icon::Details, MapChoice::Details).keys("Enter");
            menu.rule();
        }
        let mounted = entry.active && !entry.shadowed;
        let game = self.detection.install().is_some();
        let playable = entry.available(preferences);
        let why = if !game { Some(t("Find your Total Annihilation install on the Play page first.")) }
            else if self.game_running() { Some(t("The game is running.")) }
            else if !idle { Some(t("Wait for the current map or mod work to finish.")) }
            else if !playable { Some(t(entry.status(preferences))) }
            else { None };
        menu.add(n("QUICK BATTLE"), Icon::Play, MapChoice::Do(Action::Maps(maps::Action::Play(entry.key.clone())))).main().unless(why);
        menu.add(n("SET UP SKIRMISH"), Icon::Setup, MapChoice::Do(Action::Maps(maps::Action::Skirmish(entry.key.clone())))).unless(why);
        let edit_why = if !game { Some(t("Find your Total Annihilation install on the Play page first.")) }
            else if !idle { self.maps_busy_reason() }
            else if !mounted { Some(t(entry.status(preferences))) }
            else { None };
        menu.add(n("EDIT A COPY"), Icon::Edit, MapChoice::Do(Action::Maps(maps::Action::Edit(entry.key.clone())))).unless(edit_why);
        menu.rule();
        let favourite = preferences.is_favourite(&entry.stem);
        let (label, star) = if favourite { (n("Remove from favourites"), false) } else { (n("Add to favourites"), true) };
        menu.add(label, Icon::Star(star), MapChoice::Do(Action::Maps(maps::Action::Favourite(entry.key.clone())))).keys("F");
        // (Offered in battles or not: a map for battles only, its source on.)
        if !entry.campaign && entry.players >= 2 {
            let enabled = preferences.is_enabled(&entry.layer, &entry.stem);
            let (label, open) = if enabled { (n("HIDE FROM BATTLES"), false) } else { (n("OFFER IN BATTLES"), true) };
            let why = if !mounted { Some(t(entry.status(preferences))) } else if !idle { self.maps_busy_reason() } else { None };
            menu.add(label, Icon::Eye(open), MapChoice::Do(Action::Maps(maps::Action::Toggle(entry.key.clone())))).unless(why);
        }
        menu.rule();
        let installed = self.mods.get(&entry.layer).is_some();
        if installed {
            menu.add(n("MANAGE SOURCE IN MODS"), Icon::Goto, MapChoice::Do(Action::Maps(maps::Action::ManageSource(entry.layer.clone()))));
        }
        let path = if entry.path.is_file() { entry.path.parent().unwrap_or(&entry.path).to_path_buf() } else { entry.path.clone() };
        menu.add(n("SHOW SOURCE FILES"), Icon::Folder, MapChoice::Do(Action::OpenPath(path)));
        if entry.pack() && installed {
            menu.rule();
            let busy = (self.game_running() || self.installer.busy() || self.maps.busy()).then(|| self.maps_busy_reason()).flatten();
            menu.add(n("REMOVE MAP PACK"), Icon::Remove, MapChoice::Do(Action::Maps(maps::Action::RemovePack(entry.layer.clone())))).danger().unless(busy);
        }
        menu
    }

    /// The page's title, and what it does to the library as a whole: import maps, make a new
    /// one, look again.
    fn maps_header(&mut self, ui: &mut Ui, short: bool, idle: bool) {
        let game = self.detection.install().is_some();
        let busy = self.maps_busy_reason();
        super::library_header(ui, n("MAPS"), n("Explore your installed worlds, manage battle maps and create your own."), short,
            &["IMPORT MAPS…", "SUPREME COMMANDER…", "NEW MAP", "RESCAN"], |ui, height| {
            if ui.add_enabled(idle, ActionButton::primary("IMPORT MAPS…").height(height)).hover_tip(t("Add maps from a map archive (UFO, HPI, ZIP, 7z or RAR). You can also drop one on this page."))
                .disabled_tip(busy.unwrap_or_default()).clicked() {
                self.push(Action::Maps(maps::Action::Browse));
            }
            if ui.add_enabled(idle, ActionButton::secondary("SUPREME COMMANDER…").height(height))
                .hover_tip(t("Import the maps of Supreme Commander: Forged Alliance from your own copy of the game (GOG's or Steam's), to play in skirmish."))
                .disabled_tip(busy.unwrap_or_default()).clicked() {
                self.push(Action::SupcomOpen);
            }
            let new_map_why = if !game { Some(t("Find your Total Annihilation install on the Play page first.")) } else { busy };
            if ui.add_enabled(idle && game, ActionButton::secondary("NEW MAP").height(height)).hover_tip(t("Open the map editor on a new map."))
                .disabled_tip(new_map_why.unwrap_or_default()).clicked() {
                self.push(Action::Maps(maps::Action::New));
            }
            if ui.add_enabled(idle, ActionButton::secondary("RESCAN").height(height)).hover_tip(t("Look through your game, mods and map packs again (F5)."))
                .disabled_tip(busy.unwrap_or_default()).clicked() {
                self.push(Action::Maps(maps::Action::Rescan));
            }
        });
    }

    /// What an import or a scan came to (until dismissed), and sources that could not be read.
    fn maps_feedback(&mut self, ui: &mut Ui) {
        if let Some((message, error)) = self.maps.feedback.clone() {
            ui.horizontal_top(|ui| {
                let link = widgets::galley(ui, RichText::new(t("DISMISS")).font(strong(11.5))).size().x + 16.0;
                ui.allocate_ui_with_layout(vec2((ui.available_width() - link).max(1.0), 0.0), Layout::top_down(Align::Min), |ui| {
                    super::wrapped(ui, &message, if error { RED } else { GREEN });
                });
                if widgets::text_link(ui, "DISMISS").clicked() { self.maps.feedback = None; }
            });
        }
        if !self.maps.catalogue.warnings.is_empty() {
            ui.collapsing(tf("{n} map sources need attention", &[("n", &self.maps.catalogue.warnings.len())]), |ui| {
                for warning in &self.maps.catalogue.warnings { super::wrapped(ui, warning, AMBER); }
            });
        }
    }

    /// The search, the filters, the order and the view, and how many maps they leave.
    fn maps_toolbar(&mut self, ui: &mut Ui) {
        let shortcuts = self.search_shortcuts_allowed(ui);
        let width = ui.available_width();
        let filters = [(Filter::All, n("ALL")), (Filter::Available, n("AVAILABLE")), (Filter::Unavailable, n("UNAVAILABLE")), (Filter::Campaign, n("CAMPAIGN"))];
        let hint = t("Search maps, worlds or sources…");
        if width >= 720.0 {
            ui.horizontal(|ui| {
                let search = (width * 0.4).clamp(220.0, 400.0);
                library::search(ui, &mut self.maps.query, Id::new("map_library_search"), hint, search, shortcuts);
                ui.add_space(space::S);
                widgets::segmented(ui, &mut self.maps.filter, &filters);
            });
        } else {
            library::search(ui, &mut self.maps.query, Id::new("map_library_search"), hint, width, shortcuts);
            widgets::segmented_fill(ui, &mut self.maps.filter, &filters);
        }
        let mut worlds: Vec<String> = self.maps.catalogue.entries.iter().map(|entry| entry.planet.clone()).filter(|planet| !planet.is_empty()).collect();
        worlds.sort_by_key(|world| world.to_lowercase());
        worlds.dedup();
        // (The sources by name: the game's own maps by where they come from, Core Contingency's
        // and Battle Tactics' apart from TA's.)
        let sources: Vec<String> = self.maps.catalogue.entries.iter().map(|entry| entry.source.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let shown = self.maps.visible(&self.config.maps).len();
        let total = self.maps.catalogue.entries.len();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            // (A control goes to the next row where the rest of this one is too short for it.)
            let room = |ui: &mut Ui, width: f32| if ui.available_size_before_wrap().x < width { ui.end_row(); };
            let any = t("Any").to_string();
            let players = [(Players::Any, any.clone()), (Players::Two, "2".into()), (Players::Four, "3–4".into()), (Players::Many, "5+".into())];
            let wide = |ui: &Ui, label: &str, names: &[String]| widgets::combo_width(ui, names.iter().map(|name| format!("{label}  {name}")), 90.0);
            let label = t("PLAYERS");
            let names: Vec<String> = players.iter().map(|(_, name)| name.clone()).collect();
            let width = wide(ui, label, &names);
            room(ui, width);
            library::choice(ui, "map_players", width, label, &mut self.maps.players, &players);
            let sizes = [(Size::Any, any.clone()), (Size::Small, t("Small").into()), (Size::Medium, t("Medium").into()), (Size::Large, t("Large").into()), (Size::Huge, t("Huge").into())];
            let label = t("SIZE");
            let names: Vec<String> = sizes.iter().map(|(_, name)| name.clone()).collect();
            let width = wide(ui, label, &names);
            room(ui, width);
            library::choice(ui, "map_size", width, label, &mut self.maps.size, &sizes)
                .hover_tip(t("By the longer side: small up to 8, medium up to 12, large up to 20, huge beyond."));
            // (Worlds and sources by their place in the list, the first being all of them.)
            let mut world = worlds.iter().position(|world| *world == self.maps.world).map_or(0, |at| at + 1);
            let choices: Vec<(usize, String)> = std::iter::once((0, t("All").to_string())).chain(worlds.iter().cloned().enumerate().map(|(at, world)| (at + 1, world))).collect();
            let label = t("WORLD");
            let names: Vec<String> = choices.iter().map(|(_, name)| name.clone()).collect();
            let width = wide(ui, label, &names).min(190.0);
            room(ui, width);
            if library::choice(ui, "map_world", width, label, &mut world, &choices).changed() {
                self.maps.world = world.checked_sub(1).and_then(|at| worlds.get(at)).cloned().unwrap_or_default();
            }
            let mut source = sources.iter().position(|name| *name == self.maps.source).map_or(0, |at| at + 1);
            let choices: Vec<(usize, String)> = std::iter::once((0, t("All").to_string())).chain(sources.iter().cloned().enumerate().map(|(at, name)| (at + 1, name))).collect();
            let label = t("SOURCE");
            let names: Vec<String> = choices.iter().map(|(_, name)| name.clone()).collect();
            let width = wide(ui, label, &names).min(210.0);
            room(ui, width);
            if library::choice(ui, "map_source", width, label, &mut source, &choices).changed() {
                self.maps.source = source.checked_sub(1).and_then(|at| sources.get(at)).cloned().unwrap_or_default();
            }
            let sorts = [(Sort::Name, t("Name").to_string()), (Sort::Players, t("Players").into()), (Sort::Size, t("Size").into()), (Sort::Recent, t("Last played").into()), (Sort::Source, t("Source").into())];
            let label = t("ORDER");
            let names: Vec<String> = sorts.iter().map(|(_, name)| name.clone()).collect();
            let width = wide(ui, label, &names);
            room(ui, width);
            library::choice(ui, "map_sort", width, label, &mut self.maps.sort, &sorts);
            let random = widgets::galley(ui, RichText::new(t("RANDOM")).font(strong(13.5))).size().x + 34.0;
            room(ui, widgets::CONTROL_HEIGHT + 2.0 + 8.0 + random);
            library::star_button(ui, &mut self.maps.favourites, t("Favourites only")).hover_tip(t("Show only your favourite maps"));
            if ui.add(ActionButton::secondary("RANDOM").height(widgets::CONTROL_HEIGHT)).hover_tip(t("Choose one of these maps at random")).clicked() {
                self.push(Action::Maps(maps::Action::Random));
            }
            room(ui, 150.0);
            widgets::segmented(ui, &mut self.maps.view, &[(View::Grid, n("CARDS")), (View::List, n("LIST"))]);
            let count = RichText::new(tf("{n} of {total} maps", &[("n", &shown), ("total", &total)])).color(STEEL).size(12.0);
            room(ui, widgets::galley(ui, count.clone()).size().x + 8.0);
            ui.add_space(4.0);
            ui.add(egui::Label::new(count).extend());
            if self.map_filters_set() {
                room(ui, 110.0);
                if widgets::text_link(ui, "CLEAR FILTERS").clicked() { self.clear_map_filters(); }
            }
        });
        ui.add_space(space::S);
    }

    fn map_filters_set(&self) -> bool {
        let maps = &self.maps;
        !maps.query.is_empty() || maps.filter != Filter::All || !maps.source.is_empty() || maps.players != Players::Any
            || maps.size != Size::Any || !maps.world.is_empty() || maps.favourites
    }

    fn clear_map_filters(&mut self) {
        let maps = &mut self.maps;
        maps.query.clear();
        maps.filter = Filter::All;
        maps.source.clear();
        maps.players = Players::Any;
        maps.size = Size::Any;
        maps.world.clear();
        maps.favourites = false;
    }

    /// The maps on show, as cards or as the rows of a list.
    fn map_library(&mut self, ui: &mut Ui, visible: &[usize], height: f32) {
        // (Still being looked through: the cards to come, not a word that there are none.)
        if visible.is_empty() && self.maps.catalogue.entries.is_empty() && (self.maps.busy() || self.maps.loaded_for.is_none()) {
            let width = library::grid_width(ui.available_width());
            library::skeleton(ui, library::layout(width, 170.0, WELL, 50.0, 12.0), height, WELL);
            return;
        }
        if visible.is_empty() {
            ui.add_space(space::L);
            super::wrapped(ui, t(if self.maps.catalogue.entries.is_empty() { "Install your Total Annihilation data or import a map archive to begin." }
                else { "No maps match. Clear the search or choose another filter." }), STEEL);
            if self.map_filters_set() && widgets::text_link(ui, "CLEAR FILTERS").clicked() { self.clear_map_filters(); }
            return;
        }
        let reveal = self.maps.take_selection_to_reveal().and_then(|key| visible.iter().position(|index| self.maps.catalogue.entries[*index].key == key));
        let width = library::grid_width(ui.available_width());
        let view = self.maps.view;
        let layout = match view {
            View::Grid => library::layout(width, 170.0, WELL, 50.0, 12.0),
            View::List => library::rows(width, 58.0, 0.0),
        };
        let well = ((layout.card.x - 12.0) * WELL).round();
        // The favourites first, then TA Reborn's own maps, then the rest (`visible`'s order).
        let [favourites, reborn, others] = self.maps.groups(visible, &self.config.maps);
        let sections = [
            Section { title: n("YOUR FAVOURITES"), note: None, count: favourites, accent: AMBER_BRIGHT, star: true },
            Section { title: n("TA REBORN MAPS"), note: Some(n("Made for TA Reborn: optimised for its engine, balanced for every start")), count: reborn, accent: AMBER, star: false },
            Section { title: n("MORE MAPS"), note: None, count: others, accent: TEXT, star: false },
        ];
        let mut chosen = None;
        let mut opened = false;
        let mut focused = None;
        let mut starred = None;
        let mut asked = None;
        library::sections(ui, Id::new("map_library_grid"), &sections, layout, height, reveal, |ui, at, rect| {
            let index = visible[at];
            let (picture, failed) = self.maps.thumb(index);
            let entry = &self.maps.catalogue.entries[index];
            let preferences = &self.config.maps;
            let size = self.maps.size_of(entry);
            let detail = map_detail(entry, size);
            let status = t(entry.status(preferences));
            let favourite = preferences.is_favourite(&entry.stem);
            let spoken = format!("{}, {detail}, {status}", entry.title);
            // (On a card the star is its own button, over the picture.)
            let card = Card { title: &entry.title, detail: &detail, lamps: &[lamp(entry, preferences)],
                star: favourite && view == View::List, selected: entry.key == self.maps.selected, spoken: &spoken,
                ribbon: entry.ribbon() };
            let id = Id::new(("map_card", &entry.key));
            let paint = |ui: &Ui, frame: Rect| library::picture(ui, frame, picture.as_ref(), failed);
            let response = match view {
                View::Grid => library::card(ui, id, rect, well, card, paint),
                View::List => library::picture_row(ui, id, rect, 1.3, card, paint),
            };
            if view == View::Grid && favourite_star(ui, id, rect, favourite, response.hovered()) {
                starred = Some(entry.key.clone());
            }
            if let Some((at, keys)) = menu::asked(ui, &response) {
                asked = Some((entry.key.clone(), at, keys.then_some(response.id)));
            }
            if response.clicked() { chosen = Some(entry.key.clone()); opened = true; }
            if response.has_focus() {
                // (egui moves the keys between cards at the end of a frame and reports no
                // gained focus for it: the card that has them is followed instead.)
                if self.maps.focused.as_ref() != Some(&entry.key) { response.scroll_to_me(None); }
                focused = Some(entry.key.clone());
                if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) { opened = true; }
            }
            response.hover_tip(format!("{}\n{} · {status}", entry.title, entry.source))
        });
        if focused != self.maps.focused {
            if let Some(key) = &focused { self.maps.selected = key.clone(); }
            self.maps.focused = focused;
        }
        if let Some(key) = chosen { self.maps.selected = key; }
        if opened { self.maps.details_open = true; }
        if let Some(key) = starred { self.push(Action::Maps(maps::Action::Favourite(key))); }
        // A right-click (or Shift+F10) chooses the map and opens what can be done with it.
        if let Some((key, at, from)) = asked {
            self.maps.selected = key.clone();
            menu::open(ui.ctx(), map_menu_id(), &key, at, from);
        }
    }

    /// The chosen map: its picture with the start positions, what can be done with it, its facts.
    fn map_details(&mut self, ui: &mut Ui, height: f32, idle: bool) {
        let Some(entry) = self.maps.catalogue.entries.iter().find(|entry| entry.key == self.maps.selected).cloned() else {
            if !self.maps.catalogue.entries.is_empty() { super::wrapped(ui, t("Choose a map to see it here."), STEEL); }
            return;
        };
        let (picture, failed) = self.maps.details_picture();
        let cells = self.maps.details.size(&entry.picture).or_else(|| self.maps.thumbs.size(&entry.picture));
        let mut area = egui::ScrollArea::vertical().id_salt(("map_details", &entry.key)).max_height(height).min_scrolled_height(0.0).auto_shrink([false, false]);
        if let Some(offset) = self.capture_scroll { area = area.vertical_scroll_offset(offset); }
        ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Backdrop(BG), |ui| widgets::card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let width = ui.available_width();
            if width >= 620.0 {
                // (Room for two columns: the picture at the left, the rest beside it.)
                let gap = space::XL;
                let left = ((width - gap) * 0.48).floor();
                let right = (width - gap - left).max(1.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.allocate_ui_with_layout(vec2(left, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(left);
                        map_picture(ui, &entry, picture.as_ref(), failed, cells, (height - 40.0).clamp(150.0, 640.0));
                    });
                    ui.allocate_ui_with_layout(vec2(right, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(right);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        self.map_heading(ui, &entry, idle);
                        ui.add_space(space::S);
                        self.map_about(ui, &entry, cells);
                    });
                });
            } else {
                self.map_heading(ui, &entry, idle);
                ui.add_space(space::S);
                map_picture(ui, &entry, picture.as_ref(), failed, cells, (height * 0.5).clamp(150.0, 420.0));
                ui.add_space(space::S);
                self.map_about(ui, &entry, cells);
            }
        }));
    }

    /// The details' title, state and actions.
    fn map_heading(&mut self, ui: &mut Ui, entry: &Entry, idle: bool) {
        ui.horizontal_top(|ui| {
            let star = widgets::CONTROL_HEIGHT + 2.0;
            let room = (ui.available_width() - star - space::S).max(1.0);
            ui.allocate_ui_with_layout(vec2(room, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(room);
                ui.add(egui::Label::new(RichText::new(&entry.title).font(display(24.0)).color(TEXT_BRIGHT)).wrap());
            });
            let mut favourite = self.config.maps.is_favourite(&entry.stem);
            let label = t(if favourite { "Remove from favourites" } else { "Add to favourites" });
            if library::star_button(ui, &mut favourite, label).hover_tip(label).clicked() {
                self.push(Action::Maps(maps::Action::Favourite(entry.key.clone())));
            }
        });
        let preferences = &self.config.maps;
        ui.horizontal_wrapped(|ui| {
            widgets::pill(ui, entry.status(preferences), lamp(entry, preferences));
            ui.label(RichText::new(&entry.source).color(STEEL));
        });
        if entry.reborn() {
            reborn_banner(ui);
        } else if entry.origin == ta_data::map::Origin::SupremeCommander {
            supcom_banner(ui);
        }
        ui.add_space(space::XS);
        self.map_actions(ui, entry, idle);
    }

    /// The details' facts, the map's own words and where it comes from.
    fn map_about(&mut self, ui: &mut Ui, entry: &Entry, cells: Option<(u32, u32)>) {
        self.map_facts(ui, entry, cells);
        if !entry.blurb.is_empty() {
            ui.add_space(space::XS);
            super::wrapped(ui, &entry.blurb, TEXT);
        }
        ui.add_space(space::S);
        self.map_notes(ui, entry);
    }

    fn map_actions(&mut self, ui: &mut Ui, entry: &Entry, idle: bool) {
        let mounted = entry.active && !entry.shadowed;
        let game = self.detection.install().is_some();
        let playable = entry.available(&self.config.maps);
        let why = if !game { Some(t("Find your Total Annihilation install on the Play page first.")) }
            else if self.game_running() { Some(t("The game is running.")) }
            else if !idle { Some(t("Wait for the current map or mod work to finish.")) }
            else if !playable { Some(t(entry.status(&self.config.maps))) }
            else { None };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            let play = ui.add_enabled(why.is_none(), ActionButton::primary("QUICK BATTLE").min_width(110.0));
            let play = match why { Some(why) => play.disabled_tip(why), None => play.hover_tip(t("A battle on this map against a computer player")) };
            if play.clicked() { self.push(Action::Maps(maps::Action::Play(entry.key.clone()))); }
            let setup = ui.add_enabled(why.is_none(), ActionButton::secondary("SET UP SKIRMISH"));
            let setup = match why { Some(why) => setup.disabled_tip(why), None => setup.hover_tip(t("Choose the players, teams and rules on the game's skirmish page, with this map chosen")) };
            if setup.clicked() { self.push(Action::Maps(maps::Action::Skirmish(entry.key.clone()))); }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            let edit_why = if !game { Some(t("Find your Total Annihilation install on the Play page first.")) }
                else if !idle { self.maps_busy_reason() }
                else if !mounted { Some(t(entry.status(&self.config.maps))) }
                else { None };
            let edit = ui.add_enabled(game && idle && mounted, ActionButton::secondary("EDIT A COPY").height(28.0));
            if edit.hover_tip(t("Open a copy of this map in the map editor: saving it makes a map pack of your own")).disabled_tip(edit_why.unwrap_or_default()).clicked() {
                self.push(Action::Maps(maps::Action::Edit(entry.key.clone())));
            }
            if mounted && !entry.campaign && entry.players >= 2 {
                let enabled = self.config.maps.is_enabled(&entry.layer, &entry.stem);
                let toggle = ui.add_enabled(idle, ActionButton::secondary(if enabled { "HIDE FROM BATTLES" } else { "OFFER IN BATTLES" }).height(28.0));
                if toggle.hover_tip(t("Hidden, this map is not offered when you choose one for a battle (this commander only). Campaigns, saved games and its files are kept."))
                    .disabled_tip(self.maps_busy_reason().unwrap_or_default()).clicked() {
                    self.push(Action::Maps(maps::Action::Toggle(entry.key.clone())));
                }
            }
        });
    }

    fn map_facts(&self, ui: &mut Ui, entry: &Entry, cells: Option<(u32, u32)>) {
        let size = cells.map(maps::units).or(entry.typed);
        let dash = "—".to_string();
        // (Whole numbers, as the game's skirmish page shows them.)
        let whole = |value: f64| format!("{value:.0}");
        let facts = [
            (n("PLAYERS"), if entry.players >= 2 { entry.players.to_string() } else { t(if entry.campaign { "Campaign" } else { "Editor only" }).to_string() }),
            (n("SIZE"), size.map_or_else(|| if entry.size.is_empty() { dash.clone() } else { entry.size.clone() }, |(w, h)| format!("{w} × {h}"))),
            (n("WORLD"), if entry.planet.is_empty() { dash.clone() } else { entry.planet.clone() }),
            (n("WIND"), format!("{}–{}", whole(entry.min_wind / 100.0), whole(entry.max_wind / 100.0))),
            (n("TIDAL"), whole(entry.tidal)),
            (n("GRAVITY"), whole(entry.gravity)),
        ];
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
        if entry.lava || entry.water_damage > 0.0 {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                if entry.water_damage > 0.0 { widgets::tag(ui, &tf("ACID SEA · {n} DAMAGE A SECOND", &[("n", &whole(entry.water_damage))]), RED); }
                if entry.lava { widgets::tag(ui, n("LAVA WORLD"), AMBER); }
            });
        }
    }

    fn map_notes(&mut self, ui: &mut Ui, entry: &Entry) {
        if !entry.active || entry.shadowed {
            super::wrapped(ui, t(if entry.shadowed { "Another enabled source supplies this map. Change the mod load order to use this version." }
                else { "This map belongs to a disabled mod or map pack. Enable its source in Mods to play or edit it." }), AMBER);
        } else if entry.campaign {
            super::wrapped(ui, t("Play this map through its campaign. You can open a copy in the editor."), STEEL);
        }
        let installed = self.mods.get(&entry.layer).is_some();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(14.0, 6.0);
            if installed && widgets::text_link(ui, "MANAGE SOURCE IN MODS").clicked() {
                self.push(Action::Maps(maps::Action::ManageSource(entry.layer.clone())));
            }
            let path = if entry.path.is_file() { entry.path.parent().unwrap_or(&entry.path).to_path_buf() } else { entry.path.clone() };
            if widgets::text_link(ui, "SHOW SOURCE FILES").hover_tip(path.display().to_string()).clicked() { self.push(Action::OpenPath(path)); }
        });
        if entry.pack() && installed {
            ui.add_space(space::XS);
            let idle = !self.game_running() && !self.installer.busy() && !self.maps.busy();
            if ui.add_enabled(idle, ActionButton::danger("REMOVE MAP PACK").height(28.0))
                .hover_tip(t("The imported map pack goes to the Recycle Bin, with every map in it.")).clicked() {
                self.push(Action::Maps(maps::Action::RemovePack(entry.layer.clone())));
            }
        }
    }
}

/// A card's favourite star over its picture's corner (on a favourite's always, on the others' under
/// the pointer): whether it was clicked.
fn favourite_star(ui: &mut Ui, card: Id, rect: Rect, favourite: bool, hovered: bool) -> bool {
    let spot = Rect::from_center_size(pos2(rect.right() - 22.0, rect.top() + 22.0), vec2(26.0, 26.0));
    let response = ui.interact(spot, card.with("star"), Sense::click());
    let label = t(if favourite { "Remove from favourites" } else { "Add to favourites" });
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, favourite, label));
    let hot = response.hovered();
    if (favourite || hovered || hot) && ui.is_rect_visible(spot) {
        let painter = ui.painter();
        painter.circle_filled(spot.center(), 12.0, Color32::from_black_alpha(if hot { 210 } else { 160 }));
        painter.circle_stroke(spot.center(), 12.0, Stroke::new(1.0, if favourite { AMBER_DIM } else { LINE }));
        library::star(painter, spot.center(), 7.0, if favourite { AMBER_BRIGHT } else if hot { TEXT_BRIGHT } else { STEEL });
    }
    let clicked = response.clicked();
    if clicked { ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle); }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).hover_tip(format!("{label} (F)"));
    clicked
}

/// A TA Reborn map's mark in its details: what makes the game's own maps the ones to play.
fn reborn_banner(ui: &mut Ui) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(196, 160, 72, 22))
        .stroke(Stroke::new(1.0, AMBER_DIM))
        .corner_radius(3)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(t("TA REBORN MAP")).font(strong(12.5)).color(AMBER_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
            super::wrapped(ui, t("Built for TA Reborn: optimised for its engine and balanced for every start. The best maps to play."), TEXT);
        });
}

/// Over a map made from one of Supreme Commander's: what it is, and that it is the player's own
/// making, from their copy of the game (its cliffs, forests and mass points kept).
fn supcom_banner(ui: &mut Ui) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(104, 184, 232, 18))
        .stroke(Stroke::new(1.0, SKY.gamma_multiply(0.55)))
        .corner_radius(3)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(t("SUPREME COMMANDER MAP")).font(strong(12.5)).color(SKY).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
            super::wrapped(ui, t("Made from a map of Supreme Commander: Forged Alliance, from your own copy of the game: its land, cliffs, forests, rocks, mass points and starts."), TEXT);
        });
}

/// A card's line under its title: players, size and world.
fn map_detail(entry: &Entry, size: Option<(u32, u32)>) -> String {
    let mut parts = Vec::new();
    if entry.players >= 2 { parts.push(tf("{n} players", &[("n", &entry.players)])); }
    else { parts.push(t(if entry.campaign { "Campaign" } else { "Editor only" }).to_string()); }
    if let Some((w, h)) = size { parts.push(format!("{w} × {h}")); }
    if !entry.planet.is_empty() { parts.push(entry.planet.clone()); }
    parts.join(" · ")
}

/// The lamp of a map's state: playable, a campaign's, deactivated by you, or out of reach.
fn lamp(entry: &Entry, preferences: &ta_config::maps::Maps) -> Color32 {
    if entry.available(preferences) { GREEN }
    else if entry.campaign { SKY }
    else if entry.active && !entry.shadowed && entry.players >= 2 { AMBER }
    else { STEEL_DIM }
}

/// The map's large picture in a well, as wide as there is room for (and not too tall), with
/// numbered markers where the players start.
fn map_picture(ui: &mut Ui, entry: &Entry, picture: Option<&TextureHandle>, failed: bool, cells: Option<(u32, u32)>, tallest: f32) {
    let width = ui.available_width();
    // (The ground the picture shows: the minimap's window, as the game's own pickers place
    // the start positions on it.)
    let window = cells.map(|(w, h)| {
        let (ww, wh) = ta_formats::tnt::minimap_window(w, h);
        (((ww * 16).min(w * 16)) as f32, ((wh * 16).min(h * 16)) as f32)
    });
    let aspect = picture.map(|texture| texture.size_vec2().x / texture.size_vec2().y.max(1.0))
        .or(window.map(|(w, h)| w / h.max(1.0))).filter(|aspect| aspect.is_finite() && *aspect > 0.0).unwrap_or(1.3);
    let height = (width / aspect).min(width * 0.85).min(tallest).max(80.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height + 4.0), Sense::hover());
    let frame = Rect::from_center_size(rect.center(), vec2((height * aspect).min(width), height) + vec2(4.0, 4.0));
    let painter = ui.painter();
    painter.add(ta_ui::metal::well(frame, Color32::from_rgb(9, 10, 8)));
    let inner = frame.shrink(2.0);
    match picture {
        Some(texture) => {
            painter.image(texture.id(), inner, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            painter.rect_stroke(inner, 1.0, Stroke::new(1.0, Color32::from_black_alpha(120)), StrokeKind::Inside);
            if let Some((w, h)) = window {
                for (slot, (x, z)) in entry.starts.iter().enumerate() {
                    let at = inner.min + vec2(x / w * inner.width(), z / h * inner.height());
                    if inner.expand(2.0).contains(at) { start_marker(ui, at, slot + 1); }
                }
            }
        }
        None if failed => {
            painter.text(inner.center(), egui::Align2::CENTER_CENTER, t("Preview unavailable"), body(13.0), STEEL);
        }
        None => {
            let rail = Rect::from_center_size(inner.center(), vec2((inner.width() * 0.4).clamp(40.0, 140.0), 8.0));
            ta_ui::progress::paint(ui, rail, None, true);
        }
    }
}

/// A start position's marker: its slot's number on a dark disc in a brass ring, as TA's
/// pickers number them.
fn start_marker(ui: &Ui, at: egui::Pos2, slot: usize) {
    let painter = ui.painter();
    let radius = 9.0;
    painter.circle_filled(at + vec2(0.0, 1.5), radius + 1.5, Color32::from_black_alpha(120));
    painter.circle(at, radius, Color32::from_rgb(20, 19, 16), Stroke::new(2.0, AMBER));
    let number = widgets::galley(ui, RichText::new(slot.to_string()).font(strong(11.5)).color(AMBER_BRIGHT));
    let size = number.size();
    painter.galley(at - size * 0.5, number, AMBER_BRIGHT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// A map's menu, from a right-click on its card or its row: what its details offer (to play
    /// it, set up a skirmish, edit a copy, favourite it, offer it in battles or not, its files),
    /// what cannot be done now saying why (no game found here); a choice asked for, and the map
    /// chosen.
    #[test]
    fn a_maps_menu_offers_its_actions_and_says_why_not() {
        let root = std::env::temp_dir().join(format!("ta_map_menu_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let ctx = egui::Context::default();
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", root.join("config.toml").to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        app.maps.catalogue = crate::maps::tests::sample_catalogue(&root);
        let frame = |app: &mut LauncherApp, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 900.0))), events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| app.maps_page(ui));
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
        let click = |app: &mut LauncherApp, at: egui::Pos2, button| {
            frame(app, vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button, pressed: true, modifiers: Default::default() }]);
            frame(app, vec![egui::Event::PointerButton { pos: at, button, pressed: false, modifiers: Default::default() }]);
            frame(app, Vec::new())
        };
        for view in [View::Grid, View::List] {
            app.maps.view = view;
            frame(&mut app, Vec::new());
            frame(&mut app, Vec::new());
            let key = app.maps.catalogue.entries.iter().find(|entry| entry.stem == "moon").unwrap().key.clone();
            let card = ctx.read_response(Id::new(("map_card", &key))).expect("the map's card").rect;
            let shown = click(&mut app, card.center() + vec2(0.0, 10.0), egui::PointerButton::Secondary);
            assert_eq!(app.maps.selected, key, "a right-click chooses it ({view:?})");
            for words in ["QUICK BATTLE", "SET UP SKIRMISH", "EDIT A COPY", "ADD TO FAVOURITES", "HIDE FROM BATTLES", "SHOW SOURCE FILES",
                "Find your Total Annihilation install on the Play page first."] {
                assert!(find(&shown, words).is_some(), "{words} in its menu ({view:?}): {:?}", shown.iter().map(|(t, _)| t).collect::<Vec<_>>());
            }
            let favourite = find(&shown, "ADD TO FAVOURITES").unwrap().center();
            click(&mut app, favourite, egui::PointerButton::Primary);
            let asked = std::mem::take(&mut app.actions);
            assert!(matches!(asked.as_slice(), [Action::Maps(maps::Action::Favourite(chosen))] if *chosen == key), "{asked:?}");
            frame(&mut app, Vec::new());
            assert!(menu::target(&ctx, map_menu_id()).is_none(), "choosing closes it");
        }
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }
}
