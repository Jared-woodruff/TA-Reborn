//! The dialog that imports Supreme Commander: Forged Alliance's maps (`crate::supcom`), its
//! factions (`crate::supcom_factions`) and its music (`crate::supcom_music`), a tab each: where
//! the game is, its maps (and the player's own) with the game's pictures of them to choose from,
//! its four factions with their units counted and their commanders' pictures, or its soundtrack's
//! tracks by their parts, and the import under way (or how the last went). The dialog is as tall
//! on every tab: the tabs stay at its top and each tab's contents scroll under them where they run
//! past the room. The maps' list ends between two rows and draws only the
//! rows in sight, and one past either edge for the keys to reach (each map's picture uploaded
//! once); the buttons stay in sight under the body however small the window, and why one cannot
//! be pressed is written beside it.

use egui::{Color32, CornerRadius, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{Exit, Panel};
use ta_ui::tooltip::TooltipExt as _;

use super::path_label;
use crate::app::{Action, LauncherApp};
use crate::supcom::{self, Phase, State, Tab};
use crate::supcom_factions;
use crate::supcom_music;
use crate::theme::*;
use crate::widgets::{self, ActionButton};

/// A map's picture in the list (points).
const THUMB: f32 = 56.0;
/// A map's card, and so a row of the list, this tall; a little shorter or taller, between these,
/// where the rows share the list's room ([`list_size`]).
const ROW: f32 = THUMB + 8.0;
const ROW_SHORTEST: f32 = 46.0;
const ROW_TALLEST: f32 = 84.0;
/// The narrowest a map's card is; as many as fit stand in a row.
const CARD: f32 = 250.0;
const GAP: f32 = 8.0;
/// The footer's height until it has been laid out (its row of buttons).
const FOOTER: f32 = 44.0;
/// The tallest the footer's list of what could not be made grows (it scrolls beyond).
const FAILED_HEIGHT: f32 = 96.0;
/// The tallest the list of what could not be read grows (it scrolls beyond).
const SKIPPED_HEIGHT: f32 = 120.0;
/// A faction's commander's picture (its build picture, as the game shows it) and its card.
const PORTRAIT: f32 = 64.0;
const FACTION_ROW: f32 = PORTRAIT + 8.0;
/// The narrowest a faction's card is: two stand in a row where they fit.
const FACTION_CARD: f32 = 330.0;
/// The narrowest a track's line is in the music's list (up to three stand in a row), its height,
/// and the room between two in a row (wide: a track's length and the next one's name stay apart).
const TRACK: f32 = 240.0;
const TRACK_ROW: f32 = 22.0;
const TRACK_GAP: f32 = 32.0;
/// The dialog's size at most: the same height on either tab, whatever each holds.
const WIDTH: f32 = 880.0;
const HEIGHT: f32 = 820.0;

/// What a tab's footer shows, taken before it is drawn (apart from the body, which the dialog
/// draws first).
struct Footer {
    tab: Tab,
    importing: bool,
    stopping: bool,
    progress: supcom::Progress,
    /// What the last import came to: its words and colour, and what could not be made (its name,
    /// and why).
    outcome: Option<(String, Color32, Vec<(String, String)>)>,
    /// There are maps (factions) to choose from: the game found, they listed.
    listed: bool,
    chosen: usize,
    blocked: Option<&'static str>,
    /// What is chosen is installed already, made by this TA Reborn: IMPORT would make it again
    /// for nothing (the factions' and the music's tabs).
    current: bool,
}

/// Where the game is, as a tab's look found it.
#[derive(Default)]
struct Source {
    copies: Vec<ta_supcom::install::Install>,
    install: Option<ta_supcom::install::Install>,
    picked_gone: Option<std::path::PathBuf>,
    error: Option<String>,
}

/// What the footer was asked to do.
#[derive(Default)]
struct Asked {
    close: bool,
    actions: Vec<Action>,
}

/// A map's or a faction's card, to choose it by.
struct Pick<'a> {
    /// Its own id, the same wherever it is in the list (as rows scroll in and out of sight, an id
    /// counted from what came before it would change, and the keys' focus be lost).
    id: egui::Id,
    width: f32,
    height: f32,
    picture: Option<egui::TextureHandle>,
    picture_side: f32,
    title: &'a str,
    title_size: f32,
    /// The lines under the title: their words and colour.
    lines: &'a [(&'a str, Color32)],
    /// Whether it was imported before (its mark, and the mark's colour).
    mark: Option<(&'a str, Color32)>,
    /// What the card says, for screen readers (its box's name).
    spoken: &'a str,
    /// Why it cannot be chosen now: an import under way.
    held: Option<&'static str>,
    /// What the pointer resting on it shows.
    hover: Option<&'a str>,
}

impl LauncherApp {
    /// Why the chosen maps cannot be imported now, if they cannot.
    pub(crate) fn supcom_blocked(&self) -> Option<&'static str> {
        if self.detection.install().is_none() {
            Some(t("Find your Total Annihilation install on the Play page first."))
        } else if self.game_running() {
            Some(t("Close the game first: maps cannot be imported while it runs."))
        } else if self.supcom.factions.importing() {
            Some(t("Wait for the Supreme Commander factions being made, then import these."))
        } else if self.supcom.music.importing() {
            Some(t("Wait for the Supreme Commander music being made, then import these."))
        } else if self.installer.busy() {
            Some(t("Wait for the mod being installed, then import these."))
        } else if self.maps.importing() {
            Some(t("Wait for the maps being imported, then import these."))
        } else {
            supcom::pack_copy(&self.mods).and_then(|copy| copy.blocks())
        }
    }

    /// Why the chosen factions cannot be imported now, if they cannot. (They need nothing of TA's:
    /// the game is found to play them.)
    pub(crate) fn factions_blocked(&self) -> Option<&'static str> {
        if self.game_running() {
            Some(t("Close the game first: the factions cannot be imported while it runs."))
        } else if self.supcom.importing() {
            Some(t("Wait for the Supreme Commander maps being made, then import these."))
        } else if self.supcom.music.importing() {
            Some(t("Wait for the Supreme Commander music being made, then import these."))
        } else if self.installer.busy() {
            Some(t("Wait for the mod being installed, then import these."))
        } else if self.maps.importing() {
            Some(t("Wait for the maps being imported, then import these."))
        } else {
            // (A copy in TA Reborn's program folder comes before the mods folder, where the mod is
            // made: made there it would change nothing.)
            supcom_factions::mod_copy(&self.mods).and_then(|copy| copy.blocks())
        }
    }

    /// Why the game's music cannot be imported now, if it cannot. (It needs nothing of TA's.)
    pub(crate) fn music_blocked(&self) -> Option<&'static str> {
        if self.game_running() {
            Some(t("Close the game first: the music cannot be imported while it runs."))
        } else if self.supcom.importing() {
            Some(t("Wait for the Supreme Commander maps being made, then import the music."))
        } else if self.supcom.factions.importing() {
            Some(t("Wait for the Supreme Commander factions being made, then import the music."))
        } else if self.installer.busy() {
            Some(t("Wait for the mod being installed, then import the music."))
        } else if self.maps.importing() {
            Some(t("Wait for the maps being imported, then import the music."))
        } else {
            supcom_music::mod_copy(&self.mods).and_then(|copy| copy.blocks())
        }
    }

    pub(crate) fn supcom_dialog(&mut self, ctx: &egui::Context) {
        if !self.supcom.open {
            return;
        }
        let width = (ctx.content_rect().width() - 80.0).clamp(1.0, WIDTH);
        let height = (ctx.content_rect().height() - 24.0).clamp(1.0, HEIGHT);
        let footer = self.supcom_footer_state();
        // (The footer as tall as it came out last frame: its buttons stay in sight under a body
        // that scrolls.)
        let footer_id = egui::Id::new("launcher_supcom_footer");
        let footer_height = ctx.data(|d| d.get_temp::<f32>(footer_id)).unwrap_or(FOOTER);
        let mut actions = Vec::new();
        let mut asked = Asked::default();
        let panel = Panel::new("SUPREME COMMANDER").exit(Exit::Close).width(width).max_height(height);
        let response = ta_ui::panel::modal_with_footer(ctx, "launcher_supcom", panel, true, footer_height, |ui| self.supcom_body(ui, &mut actions), |ui| {
            let laid = ui.scope(|ui| supcom_footer(ui, &footer, &mut asked)).response.rect.height();
            if (laid - footer_height).abs() > 0.5 {
                ui.ctx().data_mut(|d| d.insert_temp(footer_id, laid));
                ui.ctx().request_discard("the Supreme Commander dialog's footer changed its height");
            }
        });
        // (Not while the question over it is up: Escape is its.)
        if (response.exit && self.supcom.confirm_replace.is_none()) || asked.close {
            self.supcom.close();
        }
        for a in actions.into_iter().chain(asked.actions) {
            self.push(a);
        }
    }

    /// What the footer of the tab shown shows.
    fn supcom_footer_state(&self) -> Footer {
        match self.supcom.tab {
            Tab::Maps => {
                let found = self.supcom.found();
                Footer {
                    tab: Tab::Maps,
                    importing: self.supcom.importing(),
                    stopping: self.supcom.stopping(),
                    progress: self.supcom.progress.lock().unwrap().clone(),
                    outcome: self.supcom.outcome.as_ref().map(|o| (o.summary(), o.went().colour(), o.failed.clone())),
                    listed: found.as_ref().is_some_and(|found| found.install.is_some() && !found.maps.is_empty()),
                    chosen: found.as_ref().map_or(0, |found| found.maps.iter().filter(|e| self.supcom.is_chosen(&e.folder)).count()),
                    blocked: self.supcom_blocked(),
                    current: false,
                }
            }
            Tab::Factions => {
                let factions = &self.supcom.factions;
                let found = factions.found();
                Footer {
                    tab: Tab::Factions,
                    importing: factions.importing(),
                    stopping: factions.stopping(),
                    progress: factions.progress.lock().unwrap().clone(),
                    outcome: factions.outcome.as_ref().map(|o| (o.summary(), o.went().colour(), o.skipped.clone())),
                    listed: found.as_ref().is_some_and(|found| found.install.is_some() && !found.factions.is_empty()),
                    chosen: factions.chosen_entries().len(),
                    blocked: self.factions_blocked(),
                    // (Each chosen made by this converter, and the mod holds no other.)
                    current: found.as_ref().is_some_and(|found| !found.factions.is_empty()
                        && found.factions.iter().all(|e| if factions.is_chosen(e.side()) { e.state == State::Imported } else { e.state == State::New })
                        && found.factions.iter().any(|e| e.state == State::Imported)),
                }
            }
            Tab::Music => {
                let music = &self.supcom.music;
                let found = music.found();
                Footer {
                    tab: Tab::Music,
                    importing: music.importing(),
                    stopping: music.stopping(),
                    progress: music.progress.lock().unwrap().clone(),
                    outcome: music.outcome.as_ref().map(|o| (o.summary(), o.went().colour(), o.skipped.clone())),
                    listed: found.as_ref().is_some_and(|found| found.install.is_some() && !found.tracks.is_empty()),
                    // (The soundtrack is made whole: nothing to choose.)
                    chosen: found.as_ref().map_or(0, |found| found.tracks.len()),
                    blocked: self.music_blocked(),
                    current: found.as_ref().is_some_and(|found| found.state == State::Imported),
                }
            }
        }
    }

    /// The dialog's body: its tabs, and the one shown in the room under them, scrolling there.
    fn supcom_body(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.spacing_mut().item_spacing.y = 6.0;
        let mut tab = self.supcom.tab;
        let tabs = Tab::ALL.map(|tab| (tab, tab.label()));
        if widgets::segmented(ui, &mut tab, &tabs).changed() {
            self.supcom.set_tab(tab);
        }
        ui.add_space(space::XS);
        // (All the room left: the dialog as tall on every tab, the tabs where they are.)
        let room = (ui.available_height() - 1.0).max(2.0 * ROW);
        let area = ScrollArea::vertical().id_salt(("supcom_tab", self.supcom.tab.label())).max_height(room).min_scrolled_height(room).auto_shrink([false, false]);
        ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Plate, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            self.other_tab_news(ui);
            match self.supcom.tab {
                Tab::Maps => self.maps_body(ui, actions),
                Tab::Factions => self.factions_body(ui, actions),
                Tab::Music => self.music_body(ui, actions),
            }
        });
    }

    /// The other tabs' import under way, or how each went where it ended unseen: a line at the
    /// top for each, with the way to its tab.
    fn other_tab_news(&mut self, ui: &mut Ui) {
        let shown = self.supcom.tab;
        for other in Tab::ALL.into_iter().filter(|&tab| tab != shown) {
            let (running, words, colour) = match other {
                Tab::Maps if self.supcom.importing() => (true, progress_words(Tab::Maps, &self.supcom.progress.lock().unwrap(), self.supcom.stopping()), TEXT),
                Tab::Maps => match self.supcom.outcome.as_ref().filter(|_| self.supcom.unseen) {
                    Some(outcome) => (false, outcome.summary(), outcome.went().colour()),
                    None => continue,
                },
                Tab::Factions if self.supcom.factions.importing() => (true, progress_words(Tab::Factions, &self.supcom.factions.progress.lock().unwrap(), self.supcom.factions.stopping()), TEXT),
                Tab::Factions => match self.supcom.factions.outcome.as_ref().filter(|_| self.supcom.factions.unseen) {
                    Some(outcome) => (false, outcome.summary(), outcome.went().colour()),
                    None => continue,
                },
                Tab::Music if self.supcom.music.importing() => (true, progress_words(Tab::Music, &self.supcom.music.progress.lock().unwrap(), self.supcom.music.stopping()), TEXT),
                Tab::Music => match self.supcom.music.outcome.as_ref().filter(|_| self.supcom.music.unseen) {
                    Some(outcome) => (false, outcome.summary(), outcome.went().colour()),
                    None => continue,
                },
            };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(widgets::caption(other.label()));
                if running {
                    let (rail, _) = ui.allocate_exact_size(vec2(60.0, 8.0), Sense::hover());
                    ta_ui::progress::paint(ui, rail, None, true);
                }
                ui.add(egui::Label::new(RichText::new(words).color(colour)).wrap());
                if widgets::text_link(ui, "SHOW").clicked() {
                    self.supcom.set_tab(other);
                }
            });
            ui.add_space(space::XS);
        }
    }

    /// The maps' tab: what it does, where the game is, and its maps to choose from.
    fn maps_body(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let importing = self.supcom.importing();
        ui.add(egui::Label::new(RichText::new(t("Supreme Commander: Forged Alliance's maps, made into TA Reborn maps from your own copy of the game to play in skirmish: their land and ground, forests and rocks, mass points and starting places come with them.")).color(TEXT)).wrap());
        ui.add_space(space::S);
        self.supcom_source(ui, actions);
        let Some(found) = self.supcom.found() else { return };
        if found.install.is_none() {
            return;
        }
        ui.add_space(space::S);
        if found.maps.is_empty() {
            ui.add(egui::Label::new(RichText::new(t("No skirmish maps were found in this copy of the game.")).color(AMBER)).wrap());
            return;
        }
        // How many are chosen, and what they take.
        let chosen: Vec<&supcom::Entry> = found.maps.iter().filter(|e| self.supcom.is_chosen(&e.folder)).collect();
        let total = found.maps.len();
        let summary = if chosen.is_empty() {
            tf("{total} maps to choose from", &[("total", &total)])
        } else {
            tf("{n} of {total} maps chosen: about {size} on the disk, and {time} to make", &[("n", &chosen.len()), ("total", &total), ("size", &supcom::size_text(supcom::size_estimate(&chosen))), ("time", &supcom::time_text(supcom::time_estimate(&chosen)))])
        };
        let held = importing.then(|| t("Held until the import ends"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for (label, on) in [(n("SELECT ALL"), true), (n("SELECT NONE"), false)] {
                let button = ui.add_enabled(!importing, egui::Button::new(RichText::new(t(label)).font(strong(11.5))));
                if button.disabled_tip(held.unwrap_or_default()).clicked() {
                    self.supcom.choose_all(on);
                }
            }
            ui.label(RichText::new(summary).color(STEEL));
            if let Some(held) = held {
                ui.label(RichText::new(held).color(AMBER));
            }
        });
        ui.add_space(space::XS);
        // The list in the room left, its rows sharing it (one at least: a short window's tab
        // scrolls, the buttons under it in sight).
        let (height, row) = list_size(ui.available_height());
        self.supcom_list(ui, &found, held, height, row);
    }

    /// The factions' tab: what it does, where the game is, and its four factions to choose from.
    fn factions_body(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let importing = self.supcom.factions.importing();
        ui.add(egui::Label::new(RichText::new(t("Supreme Commander: Forged Alliance's four factions, made into TA Reborn units from your own copy of the game to play in skirmish, against each other and TA's own: every unit and building with the game's own model, paint and numbers, in its faction's build menus.")).color(TEXT)).wrap());
        ui.add_space(space::S);
        self.supcom_source(ui, actions);
        let Some(found) = self.supcom.factions.found() else { return };
        if found.install.is_none() || found.factions.is_empty() {
            return;
        }
        ui.add_space(space::S);
        let chosen = self.supcom.factions.chosen_entries();
        let total = found.factions.len();
        let summary = if chosen.is_empty() {
            tf("{total} factions to choose from", &[("total", &total)])
        } else {
            let units: usize = chosen.iter().map(supcom_factions::Entry::units).sum();
            tf("{n} of {total} factions chosen, {units} units, about {size} on the disk", &[("n", &chosen.len()), ("total", &total), ("units", &units), ("size", &supcom::size_text(supcom_factions::size_estimate(&chosen)))])
        };
        let held = importing.then(|| t("Held until the import ends"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.label(RichText::new(summary).color(STEEL));
            if let Some(held) = held {
                ui.label(RichText::new(held).color(AMBER));
            }
        });
        ui.add_space(space::XS);
        // The factions, two to a row where they fit.
        let width = ui.available_width();
        let columns = (((width + GAP) / (FACTION_CARD + GAP)).floor() as usize).clamp(1, 2);
        let card = ((width - GAP * (columns - 1) as f32) / columns as f32).max(1.0);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = vec2(GAP, GAP);
            for row in found.factions.chunks(columns) {
                ui.horizontal(|ui| {
                    for entry in row {
                        self.faction_card(ui, entry, held, card);
                    }
                });
            }
        });
        // (Made again, the mod holds the factions chosen only.)
        if found.factions.iter().any(|e| e.state != State::New && !self.supcom.factions.is_chosen(e.side())) {
            ui.add(egui::Label::new(RichText::new(t("The factions not chosen are taken out of the mod when it is made again.")).color(AMBER)).wrap());
        }
        // (The mod is turned on once made, and the units it adds are compared with the other
        // players' online: said where the factions are chosen.)
        ui.add(egui::Label::new(RichText::new(t("Once made, the factions' mod is on: choose a faction as a player's side when you set up a skirmish. Online, everyone in a battle needs it, made by the same version of TA Reborn; turn it off on the Mods page to join battles without it.")).color(STEEL)).wrap());
        ui.add_space(space::XS);
        // (A footnote: at the foot of the room where the tab leaves some, under the cards where it
        // does not.)
        let note = RichText::new(t("Not played yet: Forged Alliance's own sounds and weapon effects (TA's of their kinds play in their place), shields, stealth fields and teleporting.")).color(STEEL);
        let tall = widgets::wrapped_galley(ui, note.clone(), ui.available_width()).size().y;
        let spare = ui.available_height() - tall - ui.spacing().item_spacing.y - 2.0;
        if spare > 0.0 {
            ui.add_space(spare);
        }
        ui.add(egui::Label::new(note).wrap());
    }

    /// A faction: its box, its commander's picture, its name, its units by kind, and whether it
    /// was imported before.
    fn faction_card(&mut self, ui: &mut Ui, entry: &supcom_factions::Entry, held: Option<&'static str>, width: f32) {
        let side = entry.side();
        let units = tf("{n} units", &[("n", &entry.units())]);
        let kinds = tf("{buildings} buildings · {land} land · {air} air · {naval} naval", &[("buildings", &entry.buildings), ("land", &entry.land), ("air", &entry.air), ("naval", &entry.naval)]);
        let mark = entry.state.label().map(|label| (t(label), if entry.state == State::Imported { GREEN } else { AMBER }));
        let spoken = match mark {
            Some((mark, _)) => tf("{side}, {units}, {kinds}, {state}", &[("side", &side), ("units", &units), ("kinds", &kinds), ("state", &mark)]),
            None => tf("{side}, {units}, {kinds}", &[("side", &side), ("units", &units), ("kinds", &kinds)]),
        };
        let mut chosen = self.supcom.factions.is_chosen(side);
        let picture = self.supcom.factions.texture(ui.ctx(), side);
        let lines = [(units.as_str(), TEXT), (kinds.as_str(), STEEL)];
        let pick = Pick { id: egui::Id::new(("supcom_faction", side)), width, height: FACTION_ROW, picture, picture_side: PORTRAIT, title: side, title_size: 15.0, lines: &lines, mark, spoken: &spoken, held, hover: None };
        let (toggled, gained) = pick_card(ui, &pick, &mut chosen);
        if toggled {
            self.supcom.factions.choose(side, chosen);
        }
        // (Its whole card brought into sight with the keys, not its box alone.)
        if gained {
            ui.scroll_to_rect(Rect::from_min_size(ui.min_rect().max - vec2(width, FACTION_ROW), vec2(width, FACTION_ROW)), None);
        }
    }

    /// The music's tab: what it does, where the game is, and its soundtrack's tracks by their
    /// parts (it is made whole: nothing to choose).
    fn music_body(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.add(egui::Label::new(RichText::new(t("Supreme Commander: Forged Alliance's soundtrack, made into TA Reborn's music from your own copy of the game: its menu theme, building and battle themes, briefings and films, each track a lossless file.")).color(TEXT)).wrap());
        ui.add_space(space::S);
        self.supcom_source(ui, actions);
        let Some(found) = self.supcom.music.found() else { return };
        if found.install.is_none() || found.tracks.is_empty() {
            return;
        }
        ui.add_space(space::S);
        // How many, how long, and what they take; whether they are in already.
        let minutes = (found.seconds() / 60.0).round() as u64;
        let summary = tf("{n} tracks, {minutes} minutes: about {size} on the disk", &[("n", &found.tracks.len()), ("minutes", &minutes), ("size", &supcom::size_text(supcom_music::size_estimate(&found.tracks)))]);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.label(RichText::new(summary).color(STEEL));
            if let Some(label) = found.state.label() {
                widgets::tag(ui, label, if found.state == State::Imported { GREEN } else { AMBER });
            }
        });
        // The tracks by their parts: a heading each, the tracks under it, up to three in a row.
        let width = ui.available_width();
        let columns = (((width + TRACK_GAP) / (TRACK + TRACK_GAP)).floor() as usize).clamp(1, 3);
        let cell = ((width - TRACK_GAP * (columns - 1) as f32) / columns as f32).max(1.0);
        for part in ta_supcom::soundtrack::Part::ALL {
            let tracks: Vec<&ta_supcom::soundtrack::Track> = found.tracks.iter().filter(|track| track.part == part).collect();
            if tracks.is_empty() {
                continue;
            }
            ui.add_space(space::XS);
            ui.label(widgets::caption(supcom_music::part_label(part)));
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing = vec2(TRACK_GAP, 0.0);
                for row in tracks.chunks(columns) {
                    ui.horizontal(|ui| {
                        for track in row {
                            track_line(ui, &supcom_music::track_name(track), track.seconds, cell);
                        }
                    });
                }
            });
        }
        ui.add_space(space::S);
        // (Made, the mod is on, and the game's SOUNDTRACK chooses whose music plays.)
        ui.add(egui::Label::new(RichText::new(t("Once made, the music's mod is on: choose SOUNDTRACK in the game's Options, Sound, for Total Annihilation's music, Forged Alliance's, both, or by side (Forged Alliance's while you command one of its factions). The jukebox lists its tracks. Online, the other players need not have it.")).color(STEEL)).wrap());
    }

    /// Where the game is (or that it was not found, or could not be looked for) as the tab shown
    /// found it, and on the maps' tab the player's own maps' folders.
    fn supcom_source(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let maps = self.supcom.tab == Tab::Maps;
        let (looking, failed, source) = match self.supcom.tab {
            Tab::Maps => {
                let found = self.supcom.found().unwrap_or_default();
                (self.supcom.scanning() && self.supcom.look.found.is_none(), self.supcom.look.failed, Source { copies: found.copies.clone(), install: found.install.clone(), picked_gone: found.picked_gone.clone(), error: found.error.clone() })
            }
            Tab::Factions => {
                let found = self.supcom.factions.found().unwrap_or_default();
                (self.supcom.factions.scanning() && self.supcom.factions.look.found.is_none(), self.supcom.factions.look.failed, Source { copies: found.copies.clone(), install: found.install.clone(), picked_gone: found.picked_gone.clone(), error: found.error.clone() })
            }
            Tab::Music => {
                let found = self.supcom.music.found().unwrap_or_default();
                (self.supcom.music.scanning() && self.supcom.music.look.found.is_none(), self.supcom.music.look.failed, Source { copies: found.copies.clone(), install: found.install.clone(), picked_gone: found.picked_gone.clone(), error: found.error.clone() })
            }
        };
        if looking {
            widgets::busy_line(ui, match self.supcom.tab {
                Tab::Maps => t("Looking for Supreme Commander: Forged Alliance…"),
                Tab::Factions => t("Looking for Supreme Commander: Forged Alliance and its units…"),
                Tab::Music => t("Looking for Supreme Commander: Forged Alliance and its music…"),
            });
            return;
        }
        let found = &source;
        // (The copy in use is not changed while any tab's import reads it, nor while a folder
        // chosen is looked through.)
        let held = self.supcom.any_importing().then(|| t("Held until the import ends"));
        let checking = self.supcom.checking().map(|dir| dir.to_path_buf());
        ui.horizontal_wrapped(|ui| {
            ui.label(widgets::caption("FORGED ALLIANCE"));
            if found.copies.len() > 1 {
                // (More than one copy found: each a switch, the one in use lit. Their maps make
                // the same TA maps.)
                for copy in &found.copies {
                    let on = found.install.as_ref().is_some_and(|game| ta_supcom::install::same_folder(&game.root, &copy.root));
                    let chip = ui.add_enabled_ui(held.is_none() && checking.is_none(), |ui| widgets::chip(ui, store_label(copy.store), on)).inner;
                    let chip = match held { Some(why) => chip.disabled_tip(why), None => chip.hover_tip(copy.root.display().to_string()) };
                    if chip.clicked() && !on {
                        actions.push(Action::SupcomUse(copy.root.clone()));
                    }
                }
            } else if let Some(store) = found.install.as_ref().and_then(|game| game.store.name()) {
                widgets::tag(ui, store, GREEN);
            }
            let label = if found.install.is_some() { n("CHANGE FOLDER…") } else { n("CHOOSE FOLDER…") };
            let button = ui.add_enabled(held.is_none() && checking.is_none(), ActionButton::secondary(label).height(26.0));
            let button = match held { Some(why) => button.disabled_tip(why), None => button.hover_tip(t("Choose the folder Supreme Commander: Forged Alliance is installed in.")) };
            if button.clicked() {
                actions.push(Action::SupcomBrowse);
            }
        });
        if let Some(dir) = &checking {
            widgets::busy_line(ui, &tf("Looking through {folder}…", &[("folder", &dir.display())]));
        }
        match &found.install {
            Some(game) => {
                path_label(ui, &game.root, TEXT);
            }
            None if failed => {
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::Label::new(RichText::new(t("Looking for the game failed.")).color(AMBER)).wrap());
                    if ui.add(ActionButton::secondary("LOOK AGAIN").height(26.0)).hover_tip(t("Look for Supreme Commander: Forged Alliance again.")).clicked() {
                        actions.push(Action::SupcomLookAgain);
                    }
                });
            }
            None if checking.is_none() => game_needed(ui, actions),
            None => {}
        }
        if let Some(gone) = &found.picked_gone {
            ui.add(egui::Label::new(RichText::new(tf("The folder chosen before, {folder}, no longer holds the game: the copy found is used meanwhile.", &[("folder", &gone.display())])).color(AMBER)).wrap());
        }
        if let Some(error) = &self.supcom.folder_error {
            ui.add(egui::Label::new(RichText::new(error).color(RED)).wrap());
        }
        if let Some(error) = &found.error {
            ui.add(egui::Label::new(RichText::new(error).color(RED)).wrap());
        }
        if !maps {
            return;
        }
        let found = self.supcom.found().unwrap_or_default();
        if found.install.is_some() {
            ui.add_space(space::XS);
            if found.own_folders.is_empty() {
                // (Most players have none: a line, the folder they would be in on hover.)
                ui.horizontal_wrapped(|ui| {
                    ui.label(widgets::caption("YOUR OWN MAPS"));
                    let none = ui.add(egui::Label::new(RichText::new(t("None found yet")).color(STEEL)).sense(Sense::hover()));
                    if let Some(hint) = &found.own_hint {
                        none.hover_tip(tf("Downloaded maps, and Forged Alliance Forever's, are listed from {folder} once it has some.", &[("folder", &hint.display())]));
                    }
                });
            } else {
                ui.label(widgets::caption("YOUR OWN MAPS"));
            }
            for folder in &found.own_folders {
                path_label(ui, folder, TEXT);
            }
            // (What could not be read is said, each with why: it is not listed.)
            if !found.skipped.is_empty() {
                let header = if found.skipped.len() == 1 { t("1 map could not be read").to_string() } else { tf("{n} maps could not be read", &[("n", &found.skipped.len())]) };
                ui.collapsing(RichText::new(header).color(AMBER), |ui| {
                    egui::ScrollArea::vertical().id_salt("supcom_skipped").max_height(SKIPPED_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
                        for line in &found.skipped {
                            ui.add(egui::Label::new(RichText::new(line).color(AMBER)).wrap());
                        }
                    });
                });
            }
        }
    }

    /// The maps to choose from, cards in rows `row` tall, the list `height` tall: only the rows in
    /// sight are drawn, and one past either edge, for the keys (and a pad) to reach and bring into
    /// sight.
    fn supcom_list(&mut self, ui: &mut Ui, found: &supcom::Found, held: Option<&'static str>, height: f32, row_height: f32) {
        // (As many cards in a row as fit beside the scroll lane.)
        let width = (ui.available_width() - ta_ui::scroll::LANE).max(1.0);
        let columns = (((width + GAP) / (CARD + GAP)).floor() as usize).max(1);
        let rows = found.maps.len().div_ceil(columns);
        let pitch = row_height + GAP;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = vec2(GAP, GAP);
            let area = ScrollArea::vertical().id_salt("supcom_maps").max_height(height).min_scrolled_height(height).auto_shrink([false, true]);
            // (The row whose box has just taken the keys.)
            let mut revealed = None;
            let out = ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Plate, |ui| {
                ui.spacing_mut().item_spacing = vec2(GAP, GAP);
                let top = ui.max_rect().top();
                let view = ui.clip_rect();
                let first = (((view.min.y - top) / pitch).floor().max(0.0) as usize).saturating_sub(1).min(rows);
                let last = ((((view.max.y - top) / pitch).ceil().max(0.0) as usize) + 1).min(rows);
                ui.set_height((rows as f32 * pitch - GAP).max(0.0));
                let band = Rect::from_x_y_ranges(ui.max_rect().x_range(), (top + first as f32 * pitch)..=(top + last.max(first) as f32 * pitch));
                ui.scope_builder(UiBuilder::new().max_rect(band), |ui| {
                    let card = ((ui.available_width() - GAP * (columns - 1) as f32) / columns as f32).max(1.0);
                    for row in first..last {
                        ui.horizontal(|ui| {
                            for entry in found.maps.iter().skip(row * columns).take(columns) {
                                if self.supcom_row(ui, entry, held, card, row_height) {
                                    revealed = Some(row);
                                }
                            }
                        });
                    }
                });
            });
            // Its whole row brought into sight, the list still ending between two rows (a row cut
            // at its foot is left out of sight), whatever the box's own reveal asked for.
            if let Some(row) = revealed {
                let (top, view) = (row as f32 * pitch, out.inner_rect.height());
                let mut state = out.state;
                if top < state.offset.y {
                    state.offset.y = top;
                } else if top + row_height > state.offset.y + view {
                    state.offset.y = top + row_height - view;
                }
                state.offset.y = state.offset.y.max(0.0);
                state.store(ui.ctx(), out.id);
                ui.ctx().request_repaint();
            }
        });
    }

    /// A map of the list: its box, the game's picture of it, its name, size and players, and
    /// whether it was imported before.
    fn supcom_row(&mut self, ui: &mut Ui, entry: &supcom::Entry, held: Option<&'static str>, width: f32, height: f32) -> bool {
        let name = entry.scenario.name.as_str();
        let (size, players) = (supcom::km(&entry.scenario), entry.scenario.armies.len());
        let detail = if entry.players_own {
            tf("{size} · {n} players · your own map", &[("size", &size), ("n", &players)])
        } else {
            tf("{size} · {n} players", &[("size", &size), ("n", &players)])
        };
        let mark = entry.state.label().map(|label| (t(label), if entry.state == State::Imported { GREEN } else { AMBER }));
        let spoken = match mark {
            Some((mark, _)) => tf("{name}, {detail}, {state}", &[("name", &name), ("detail", &detail), ("state", &mark)]),
            None => tf("{name}, {detail}", &[("name", &name), ("detail", &detail)]),
        };
        let mut chosen = self.supcom.is_chosen(&entry.folder);
        let picture = self.supcom.texture(ui.ctx(), &entry.folder);
        let lines = [(detail.as_str(), STEEL)];
        let description = entry.scenario.description.trim();
        let pick = Pick { id: egui::Id::new(("supcom_map", &entry.folder)), width, height, picture, picture_side: THUMB.min(height - 8.0), title: name, title_size: 14.0, lines: &lines, mark, spoken: &spoken, held, hover: (!description.is_empty()).then_some(description) };
        let (toggled, gained) = pick_card(ui, &pick, &mut chosen);
        if toggled {
            self.supcom.choose(&entry.folder, chosen);
        }
        gained
    }

    /// The question before an import takes the place of another mod of its id (one an import did
    /// not make): REPLACE AND IMPORT, or KEEP.
    pub(crate) fn supcom_replace_dialog(&mut self, ctx: &egui::Context) {
        let Some((_, copy)) = self.supcom.confirm_replace.clone() else { return };
        let version = if copy.version.trim().is_empty() || copy.version == "unverified" { t("Unknown").to_string() } else { copy.version.clone() };
        let mut replace = false;
        let width = (ctx.content_rect().width() - 80.0).clamp(1.0, 560.0);
        let response = ta_ui::panel::modal(ctx, "launcher_supcom_replace", Panel::new("REPLACE THIS MOD?").exit(Exit::Close).width(width).max_height((ctx.content_rect().height() - 24.0).max(1.0)).accent(AMBER), true, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let text = tf("Another mod, {name} (version {version}), is installed under the id this import uses: importing would take its place and remove its files.", &[("name", &copy.name), ("version", &version)]);
            ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap());
            ui.add(egui::Label::new(RichText::new(copy.path.display().to_string()).font(mono(12.0)).color(STEEL)).wrap());
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                replace = ui.add(ActionButton::danger("REPLACE AND IMPORT")).clicked();
                if ui.add(ActionButton::secondary("KEEP")).clicked() {
                    self.supcom.confirm_replace = None;
                }
            });
        });
        if replace {
            self.push(Action::SupcomReplace);
        } else if response.exit {
            self.supcom.confirm_replace = None;
        }
    }
}

/// A card to choose a map or a faction by: a click anywhere on it ticks its box; only the box
/// takes the keys, named with all the card says. Whether it was ticked or unticked, and whether
/// its box has just taken the keys.
fn pick_card(ui: &mut Ui, pick: &Pick<'_>, chosen: &mut bool) -> (bool, bool) {
    let (_, rect) = ui.allocate_space(vec2(pick.width, pick.height));
    let response = ui.interact(rect, pick.id, Sense::CLICK);
    let free = pick.held.is_none();
    let painter = ui.painter_at(rect);
    if (response.hovered() && free) || *chosen {
        painter.rect(rect, CornerRadius::same(3), if *chosen { Color32::from_rgba_unmultiplied(196, 160, 82, 18) } else { Color32::from_white_alpha(6) }, Stroke::new(1.0, if *chosen { AMBER.gamma_multiply(0.5) } else { STEEL.gamma_multiply(0.3) }), StrokeKind::Inside);
    }
    // The box.
    let mut child = ui.new_child(UiBuilder::new().id(pick.id.with("box")).max_rect(Rect::from_min_size(rect.min + vec2(8.0, (rect.height() - 20.0) / 2.0), vec2(20.0, 20.0))));
    let boxed = child.add_enabled_ui(free, |ui| widgets::check_box_named(ui, chosen, pick.spoken)).inner;
    let toggled = boxed.changed();
    let gained = boxed.gained_focus();
    if let Some(why) = pick.held {
        boxed.disabled_tip(why);
    }
    // The picture.
    let picture = Rect::from_min_size(rect.min + vec2(38.0, (rect.height() - pick.picture_side) / 2.0), vec2(pick.picture_side, pick.picture_side));
    match &pick.picture {
        Some(texture) => painter.image(texture.id(), picture, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE),
        None => painter.rect_filled(picture, CornerRadius::same(2), Color32::from_black_alpha(120)),
    };
    painter.rect_stroke(picture, CornerRadius::same(2), Stroke::new(1.0, STEEL.gamma_multiply(0.5)), StrokeKind::Inside);
    // Its mark at the top right; its title in the room the mark leaves beside it, its lines under
    // them, all of it in the card's middle.
    let text_x = picture.right() + 12.0;
    let room = (rect.right() - text_x - 8.0).max(1.0);
    let galley = |words: &str, font: egui::FontId, colour: Color32, room: f32| egui::WidgetText::from(RichText::new(words).font(font).color(colour)).into_galley(ui, Some(egui::TextWrapMode::Truncate), room, egui::TextStyle::Body);
    let mark = pick.mark.map(|(label, colour)| (galley(label, strong(9.5), colour, room), colour));
    let title_room = mark.as_ref().map_or(room, |(mark, _)| (room - mark.size().x - 8.0).max(1.0));
    let title = galley(pick.title, strong(pick.title_size), TEXT_BRIGHT, title_room);
    let lines: Vec<_> = pick.lines.iter().map(|(words, colour)| (galley(words, body(12.5), *colour, room), *colour)).collect();
    let tall = title.size().y + 4.0 + lines.iter().map(|(line, _)| line.size().y).sum::<f32>();
    let mut y = rect.top() + ((rect.height() - tall) / 2.0).max(4.0);
    painter.galley(egui::pos2(text_x, y), title.clone(), TEXT_BRIGHT);
    y += title.size().y + 4.0;
    for (line, colour) in lines {
        let height = line.size().y;
        painter.galley(egui::pos2(text_x, y), line, colour);
        y += height;
    }
    if let Some((mark, colour)) = mark {
        painter.galley(egui::pos2(rect.right() - mark.size().x - 8.0, rect.top() + 6.0), mark, colour);
    }
    let response = match (pick.held, pick.hover) {
        (Some(why), _) => response.hover_tip(why),
        (None, Some(hover)) => response.on_hover_text(hover),
        (None, None) => response,
    };
    if response.clicked() && free {
        *chosen = !*chosen;
        return (true, gained);
    }
    (toggled, gained)
}

/// The maps' list's height for `room`, and its rows': as many rows as fit near a card's own
/// height, sharing the room between them (each a little shorter or taller, within bounds), so the
/// list ends on a row's edge with no band of empty room under it.
fn list_size(room: f32) -> (f32, f32) {
    let rows = ((room + GAP) / (ROW + GAP)).round().max(1.0);
    let row = ((room + GAP) / rows - GAP).clamp(ROW_SHORTEST, ROW_TALLEST);
    (rows * (row + GAP) - GAP, row)
}

/// What an import under way is doing, as said under its rail.
fn progress_words(tab: Tab, p: &supcom::Progress, stopping: bool) -> String {
    match (tab, p.phase) {
        (Tab::Maps, Phase::Reading) => t("Reading Total Annihilation's and the game's files…").to_string(),
        (Tab::Maps, Phase::Making) if stopping => tf("Stopping once {map} is made ({i} of {n})…", &[("map", &p.current), ("i", &p.number()), ("n", &p.total)]),
        (Tab::Maps, Phase::Making) => tf("Making {map} ({i} of {n})…", &[("map", &p.current), ("i", &p.number()), ("n", &p.total)]),
        (Tab::Maps, Phase::Installing) => t("Installing the maps made…").to_string(),
        (Tab::Factions, Phase::Reading) => t("Reading the game's units…").to_string(),
        (Tab::Factions, Phase::Making) if stopping => t("Stopping: nothing is installed, and the factions stay as they were…").to_string(),
        (Tab::Factions, Phase::Making) => tf("Making {unit} ({i} of {n})…", &[("unit", &p.current), ("i", &p.number()), ("n", &p.total)]),
        (Tab::Factions, Phase::Installing) => t("Installing the factions…").to_string(),
        (Tab::Music, Phase::Reading) => t("Reading the game's music…").to_string(),
        (Tab::Music, Phase::Making) if stopping => t("Stopping: nothing is installed, and the music stays as it was…").to_string(),
        (Tab::Music, Phase::Making) => tf("Making {track} ({i} of {n})…", &[("track", &p.current), ("i", &p.number()), ("n", &p.total)]),
        (Tab::Music, Phase::Installing) => t("Installing the music…").to_string(),
    }
}

/// A track of the soundtrack in the music's list: its name, and how long it plays at the right,
/// `width` wide.
fn track_line(ui: &mut Ui, name: &str, seconds: f64, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, TRACK_ROW), Sense::hover());
    let painter = ui.painter_at(rect);
    let length = egui::WidgetText::from(RichText::new(clock(seconds)).font(mono(12.0)).color(STEEL)).into_galley(ui, Some(egui::TextWrapMode::Extend), width, egui::TextStyle::Body);
    let room = (width - length.size().x - 12.0).max(1.0);
    let title = egui::WidgetText::from(RichText::new(name).font(body(13.0)).color(TEXT)).into_galley(ui, Some(egui::TextWrapMode::Truncate), room, egui::TextStyle::Body);
    painter.galley(egui::pos2(rect.left(), rect.center().y - title.size().y / 2.0), title, TEXT);
    let at = egui::pos2(rect.right() - length.size().x - 4.0, rect.center().y - length.size().y / 2.0);
    painter.galley(at, length, STEEL);
}

/// How long a track plays, as minutes and seconds ("4:02").
fn clock(seconds: f64) -> String {
    let whole = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// The tab's import under way (map by map, unit by unit), or the button to start one, and what
/// the last came to; CLOSE either way, and why IMPORT cannot be pressed, in words beside it.
fn supcom_footer(ui: &mut Ui, footer: &Footer, asked: &mut Asked) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let maps = footer.tab == Tab::Maps;
    if footer.importing {
        let p = &footer.progress;
        let installing = p.phase == Phase::Installing;
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 10.0), Sense::hover());
        ta_ui::progress::paint(ui, rect, p.fraction(), true);
        ui.label(RichText::new(progress_words(footer.tab, p, footer.stopping)).color(TEXT));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let label = match (maps, footer.stopping) {
                (_, true) => n("STOPPING…"),
                (true, false) if p.phase == Phase::Making => n("STOP AFTER THIS MAP"),
                _ => n("STOP"),
            };
            let tip = match footer.tab {
                Tab::Maps => t("The maps made so far are kept."),
                Tab::Factions => t("Nothing is installed: the factions stay as they were."),
                Tab::Music => t("Nothing is installed: the music stays as it was."),
            };
            let stop = ui.add_enabled(!installing && !footer.stopping, ActionButton::secondary(label)).hover_tip(tip);
            let stop = if installing { stop.disabled_tip(t("Installing cannot be stopped.")) } else { stop };
            if stop.clicked() {
                asked.actions.push(match footer.tab {
                    Tab::Maps => Action::SupcomStop,
                    Tab::Factions => Action::SupcomFactionsStop,
                    Tab::Music => Action::SupcomMusicStop,
                });
            }
            if ui.add(ActionButton::secondary("CLOSE")).hover_tip(t("The import goes on with this closed.")).clicked() {
                asked.close = true;
            }
            let note = if installing { t("Installing cannot be stopped; it goes on with this closed.") } else { t("The import goes on with this closed: the launcher's footer shows how far it has got.") };
            ui.add(egui::Label::new(RichText::new(note).color(STEEL)).wrap());
        });
        return;
    }
    if let Some((summary, colour, failed)) = &footer.outcome {
        ui.add(egui::Label::new(RichText::new(summary).color(*colour)).wrap());
        if !failed.is_empty() {
            let header = match (footer.tab, failed.len()) {
                (Tab::Maps, 1) => t("1 map could not be made").to_string(),
                (Tab::Maps, n) => tf("{n} maps could not be made", &[("n", &n)]),
                (Tab::Factions, n) => tf("Units that could not be made: {n}", &[("n", &n)]),
                (Tab::Music, n) => tf("Tracks that could not be made: {n}", &[("n", &n)]),
            };
            ui.collapsing(header, |ui| {
                egui::ScrollArea::vertical().id_salt(("supcom_failed", footer.tab.label())).max_height(FAILED_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
                    for (name, why) in failed {
                        ui.label(RichText::new(name).font(strong(12.5)).color(TEXT));
                        ui.add(egui::Label::new(RichText::new(why).color(AMBER)).wrap());
                    }
                });
            });
        }
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if footer.listed {
            let (label, tip, none) = match footer.tab {
                Tab::Maps => (n("IMPORT MAPS"), t("Makes the chosen maps into TA Reborn maps, in a map pack of their own, turned on once made: the smallest in seconds, the largest in about a minute each on a fast computer."), t("Choose the maps to import.")),
                Tab::Factions => (n("IMPORT FACTIONS"), t("Makes the chosen factions into a mod of TA Reborn units, turned on once made. All four take about half a minute on a fast computer."), t("Choose the factions to import.")),
                Tab::Music => (n("IMPORT MUSIC"), t("Makes the soundtrack into TA Reborn's music, a lossless file a track, in a mod of its own, turned on once made: in seconds on a fast computer."), t("No tracks were found to import.")),
            };
            let why = footer.blocked.or((footer.chosen == 0).then_some(none));
            // (Everything chosen made already, by this TA Reborn: the button makes it again, no
            // longer the way on.)
            let (button, tip) = match (footer.current, footer.tab) {
                (true, Tab::Music) => (ActionButton::secondary("MAKE AGAIN"), t("The soundtrack is imported already, made by this version of TA Reborn: makes its mod again all the same.")),
                (true, _) => (ActionButton::secondary("MAKE AGAIN"), t("The factions chosen are imported already, made by this version of TA Reborn: makes their mod again all the same.")),
                (false, _) => (ActionButton::primary(label), tip),
            };
            if ui.add_enabled(why.is_none(), button).hover_tip(tip).disabled_tip(why.unwrap_or_default()).clicked() {
                asked.actions.push(match footer.tab {
                    Tab::Maps => Action::SupcomImport,
                    Tab::Factions => Action::SupcomFactionsImport,
                    Tab::Music => Action::SupcomMusicImport,
                });
            }
            if ui.add(ActionButton::secondary("CLOSE")).clicked() {
                asked.close = true;
            }
            // (Why IMPORT cannot be pressed, in words: a greyed button takes no keys to say it.)
            if let Some(why) = why {
                ui.add(egui::Label::new(RichText::new(why).color(if footer.blocked.is_some() { AMBER } else { STEEL })).wrap());
            } else if footer.current {
                ui.add(egui::Label::new(RichText::new(t("Imported and up to date.")).color(GREEN)).wrap());
            }
        } else if ui.add(ActionButton::secondary("CLOSE")).clicked() {
            asked.close = true;
        }
    });
}

/// The game not found (most players have no copy): what the import needs, that nothing of it is
/// downloaded or shipped, where to get it, and LOOK AGAIN for a copy installed since (CHOOSE
/// FOLDER… is in the row above, for one installed elsewhere).
fn game_needed(ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.add_space(space::XS);
    widgets::card(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(RichText::new(t("SUPREME COMMANDER: FORGED ALLIANCE NEEDED")).font(strong(15.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
        ui.add(egui::Label::new(RichText::new(t("The maps, factions and music are made on your computer from your own installed copy of Supreme Commander: Forged Alliance: GOG's Gold Edition, or Steam's Forged Alliance. The original Supreme Commander alone will not do. TA Reborn downloads nothing of it and includes none of it.")).color(TEXT)).wrap());
        ui.add(egui::Label::new(RichText::new(t("It looks for GOG's, Steam's and a boxed copy. Installed it since this opened? LOOK AGAIN. Installed it somewhere else? CHOOSE FOLDER… above.")).color(STEEL)).wrap());
        ui.add_space(space::XS);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for (label, url) in [(n("GET IT ON GOG"), ta_supcom::install::GOG_STORE_URL), (n("GET IT ON STEAM"), ta_supcom::install::STEAM_STORE_URL)] {
                if ui.add(ActionButton::secondary(label)).hover_tip(url).clicked() {
                    actions.push(Action::OpenUrl(url.into()));
                }
            }
            if ui.add(ActionButton::secondary("LOOK AGAIN")).hover_tip(t("Look for Supreme Commander: Forged Alliance again.")).clicked() {
                actions.push(Action::SupcomLookAgain);
            }
        });
    });
}

/// What a copy of the game is called on its switch: its store's name, or what it is.
fn store_label(store: ta_supcom::install::Store) -> &'static str {
    store.name().unwrap_or(match store {
        ta_supcom::install::Store::Retail => n("Boxed copy"),
        _ => n("Folder"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use egui::{Pos2, Rect};
    use std::path::PathBuf;

    fn launcher(name: &str) -> (PathBuf, egui::Context, LauncherApp) {
        let root = std::env::temp_dir().join(format!("ta-supcom-dialog-{name}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let ctx = egui::Context::default();
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", root.join("config.toml").to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        ta_ui::motion::set_reduced(&ctx, true);
        (root, ctx, app)
    }

    /// `count` of the game's maps, every third imported, the first named at length; a picture
    /// each.
    fn found(count: usize) -> (supcom::Found, supcom::Previews) {
        let maps: Vec<supcom::Entry> = (0..count).map(|i| supcom::Entry {
            folder: PathBuf::from(format!("C:/Game/maps/SCMP_{i:03}")),
            players_own: false,
            scenario: ta_supcom::map::Scenario {
                folder: format!("SCMP_{i:03}"),
                name: if i == 0 { "A Very Long Name For A Map Of The Game".into() } else { format!("Map {i:03}") },
                size: (512, 512),
                armies: vec!["ARMY_1".into(), "ARMY_2".into()],
                ..Default::default()
            },
            state: if i % 3 == 0 { State::Imported } else { State::New },
        }).collect();
        let previews = maps.iter().map(|e| (e.folder.clone(), egui::ColorImage::filled([8, 8], Color32::RED))).collect();
        let install = ta_supcom::install::Install { root: PathBuf::from("C:/Game"), store: ta_supcom::install::Store::Gog };
        (supcom::Found { install: Some(install), maps, ..Default::default() }, previews)
    }

    /// The factions' look of a copy of the game: the four, as the mod holds them.
    fn factions_found(states: [State; 4]) -> supcom_factions::Found {
        use ta_supcom::unit::Faction;
        let factions = Faction::PLAYABLE.into_iter().zip(states).map(|(faction, state)| supcom_factions::Entry { faction, buildings: 47, land: 20, air: 12, naval: 10, state }).collect();
        let install = ta_supcom::install::Install { root: PathBuf::from("C:/Game"), store: ta_supcom::install::Store::Gog };
        supcom_factions::Found { install: Some(install.clone()), copies: vec![install], factions, ..Default::default() }
    }

    /// The music's look of a copy of the game: a track or two of each part, the mod as `state`.
    fn music_found(state: State) -> supcom_music::Found {
        use ta_supcom::soundtrack::{Part, Track};
        let track = |part, number, stem: &str, seconds| Track { part, number, stem: stem.into(), bank: "Music.xwb", wave: 0, seconds, bytes: 40_000_000 };
        let tracks = vec![
            track(Part::Menu, 1, "menu", 81.0),
            track(Part::Building, 1, "building_1", 242.0),
            track(Part::Building, 2, "building_2", 180.4),
            track(Part::Battle, 1, "battle_1", 150.0),
            track(Part::Battle, 2, "battle_2", 151.0),
            track(Part::Briefing, 1, "briefing_1", 95.0),
            track(Part::Film, 5, "film_credits", 120.0),
        ];
        let install = ta_supcom::install::Install { root: PathBuf::from("C:/Game"), store: ta_supcom::install::Store::Gog };
        supcom_music::Found { install: Some(install.clone()), copies: vec![install], tracks, state, ..Default::default() }
    }

    /// A frame of the dialog, `size` points large: the words drawn, where, and what they are
    /// clipped to.
    fn frame(ctx: &egui::Context, app: &mut LauncherApp, size: egui::Vec2, events: Vec<egui::Event>) -> (egui::FullOutput, Vec<(String, Rect, Rect)>) {
        app.supcom.poll(ctx);
        app.supcom.factions.poll(ctx, app.supcom.open);
        app.supcom.music.poll(ctx, app.supcom.open);
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| app.supcom_dialog(ui.ctx()));
        output.textures_delta.clear();
        let mut texts = Vec::new();
        fn walk(shape: &egui::Shape, clip: Rect, into: &mut Vec<(String, Rect, Rect)>) {
            match shape {
                egui::Shape::Text(text) => into.push((text.galley.text().to_string(), text.galley.rect.translate(text.pos.to_vec2()), clip)),
                egui::Shape::Vec(shapes) => for shape in shapes { walk(shape, clip, into); },
                _ => {}
            }
        }
        for shape in &output.shapes { walk(&shape.shape, shape.clip_rect, &mut texts); }
        (output, texts)
    }

    /// Without the game (most players): the dialog says what is needed, and that nothing of it is
    /// downloaded or shipped, with its stores, CHOOSE FOLDER… and LOOK AGAIN, which looks for it
    /// afresh, the tab shown at once and the other when shown (never with an import under way).
    #[test]
    fn without_the_game_the_dialog_says_what_is_needed_and_looks_again() {
        let (root, ctx, mut app) = launcher("needed");
        app.supcom.open = true;
        app.supcom.tab = Tab::Maps;
        app.supcom.pretend_found(supcom::Found::default(), supcom::Previews::new());
        let size = egui::vec2(900.0, 900.0);
        for _ in 0..3 { frame(&ctx, &mut app, size, Vec::new()); }
        let (_, texts) = frame(&ctx, &mut app, size, Vec::new());
        for words in ["SUPREME COMMANDER: FORGED ALLIANCE NEEDED", "GET IT ON GOG", "GET IT ON STEAM", "LOOK AGAIN", "CHOOSE FOLDER…"] {
            assert!(texts.iter().any(|(text, _, _)| text == words), "{words}: {:?}", texts_of(&texts));
        }
        assert!(texts.iter().any(|(text, _, _)| text.contains("TA Reborn downloads nothing of it and includes none of it")), "{:?}", texts_of(&texts));
        assert!(!texts.iter().any(|(text, _, _)| text == "IMPORT MAPS"), "nothing to import: {:?}", texts_of(&texts));
        let (_, look, _) = texts.iter().find(|(text, _, _)| text == "LOOK AGAIN").unwrap().clone();
        app.actions.clear();
        let press = |pressed: bool| egui::Event::PointerButton { pos: look.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&ctx, &mut app, size, vec![egui::Event::PointerMoved(look.center()), press(true)]);
        frame(&ctx, &mut app, size, vec![press(false)]);
        assert!(app.actions.iter().any(|a| matches!(a, Action::SupcomLookAgain)), "LOOK AGAIN asked");
        // (Never this computer's copy: what a look finds in a test is canned.)
        app.supcom.look.canned = Some(std::sync::Arc::new(|| (supcom::Found::default(), supcom::Previews::new())));
        app.supcom.factions.look.canned = Some(std::sync::Arc::new(|| (supcom_factions::Found::default(), Default::default())));
        app.apply(Action::SupcomLookAgain, &eframe::Frame::_new_kittest());
        assert!(app.supcom.scanning(), "the tab shown looks again");
        assert!(!app.supcom.factions.look.current(), "the other looks again when shown");
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The list's rows share its room: it ends on a row's edge, its cards near their own height.
    #[test]
    fn the_lists_rows_share_its_room() {
        for room in [70.0, 100.0, 136.0, 180.0, 270.0, 400.0, 900.0] {
            let (height, row) = list_size(room);
            assert!((ROW_SHORTEST..=ROW_TALLEST).contains(&row), "{room}: rows {row} tall");
            let rows = (height + GAP) / (row + GAP);
            assert!((rows - rows.round()).abs() < 0.01, "{room}: whole rows");
            assert!(height <= room + 0.01 || row == ROW_SHORTEST, "{room}: within the room ({height})");
            assert!(room - height < ROW_TALLEST - ROW + 0.01 || height + GAP + ROW_SHORTEST > room, "{room}: no band of a row's room left ({height})");
        }
    }

    fn texts_of(texts: &[(String, Rect, Rect)]) -> Vec<&String> {
        texts.iter().map(|t| &t.0).collect()
    }

    /// The dialog's frame (its outermost painted words' reach is not it: its pipes are).
    fn dialog_rect(ctx: &egui::Context) -> Rect {
        ctx.memory(|m| m.area_rect(egui::Id::new("launcher_supcom"))).expect("the dialog's area")
    }

    /// In a small window (940 × 600 at 1.3 times, as reported) the buttons stay in sight under
    /// the list, which scrolls, and the tabs stay in sight above both tabs' contents.
    #[test]
    fn the_dialogs_buttons_and_tabs_stay_in_sight_in_a_small_window() {
        let (root, ctx, mut app) = launcher("small");
        app.supcom.open = true;
        let (found, previews) = found(54);
        app.supcom.pretend_found(found, previews);
        app.supcom.factions.pretend_found(factions_found([State::Imported, State::New, State::Outdated, State::New]), Default::default());
        app.supcom.music.pretend_found(music_found(State::New));
        let size = egui::vec2(723.0, 461.0);
        let screen = Rect::from_min_size(Pos2::ZERO, size);
        for tab in Tab::ALL {
            app.supcom.tab = tab;
            for _ in 0..4 { frame(&ctx, &mut app, size, Vec::new()); }
            let (_, texts) = frame(&ctx, &mut app, size, Vec::new());
            let import = match tab {
                Tab::Maps => "IMPORT MAPS",
                Tab::Factions => "IMPORT FACTIONS",
                Tab::Music => "IMPORT MUSIC",
            };
            for words in [import, "CLOSE", "MAPS", "FACTIONS", "MUSIC"] {
                let (_, rect, clip) = texts.iter().find(|(text, _, _)| text == words).unwrap_or_else(|| panic!("{words}: {:?}", texts_of(&texts)));
                assert!(screen.contains_rect(*rect) && clip.contains_rect(*rect), "{tab:?}: {words} in sight: {rect:?}, clipped to {clip:?}");
            }
        }
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The dialog is as tall on either tab, and the maps' list ends between two rows: no row is
    /// cut off at its foot (where ta_ui would hide it, leaving an empty band).
    #[test]
    fn the_dialog_is_as_tall_on_either_tab_and_its_list_ends_between_rows() {
        let (root, ctx, mut app) = launcher("height");
        app.supcom.open = true;
        let (found, previews) = found(54);
        app.supcom.pretend_found(found, previews);
        app.supcom.factions.pretend_found(factions_found([State::New; 4]), Default::default());
        app.supcom.music.pretend_found(music_found(State::New));
        for size in [egui::vec2(723.0, 461.0), egui::vec2(1120.0, 720.0), egui::vec2(1600.0, 900.0)] {
            let mut heights = Vec::new();
            for tab in Tab::ALL {
                app.supcom.tab = tab;
                for _ in 0..4 { frame(&ctx, &mut app, size, Vec::new()); }
                heights.push(dialog_rect(&ctx).height());
                if tab == Tab::Maps {
                    let (_, texts) = frame(&ctx, &mut app, size, Vec::new());
                    // (Every map name drawn is whole in its clip: none cut at the list's foot.)
                    let names: Vec<&(String, Rect, Rect)> = texts.iter().filter(|(text, _, _)| text.starts_with("Map ")).collect();
                    assert!(!names.is_empty(), "{size:?}");
                    let shown: Vec<_> = names.iter().filter(|(_, rect, clip)| clip.intersects(*rect)).collect();
                    assert!(shown.iter().all(|(_, rect, clip)| clip.contains_rect(*rect)), "{size:?}: a row cut by the list's foot");
                }
            }
            assert!(heights.iter().all(|h| (h - heights[0]).abs() < 1.0), "{size:?}: the dialog's height jumps between tabs: {heights:?}");
        }
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// With 400 maps the list draws the rows in sight (and one past either edge), not all of
    /// them, each map's picture made a texture once (its pixels let go of then), and none kept
    /// once the dialog is closed.
    #[test]
    fn the_list_draws_the_rows_in_sight_and_lets_its_pictures_go_when_closed() {
        let (root, ctx, mut app) = launcher("rows");
        app.supcom.open = true;
        let (found, previews) = found(400);
        app.supcom.pretend_found(found, previews);
        let size = egui::vec2(1280.0, 800.0);
        for _ in 0..3 { frame(&ctx, &mut app, size, Vec::new()); }
        let (_, texts) = frame(&ctx, &mut app, size, Vec::new());
        let drawn = texts.iter().filter(|(text, _, _)| text.starts_with("Map ")).count();
        assert!(drawn > 0 && drawn < 60, "{drawn} of 400 maps drawn");
        let (pixels, textures) = app.supcom.pictures_held();
        assert!(textures > 0 && textures < 60 && pixels + textures == 400, "{pixels} pictures as pixels, {textures} as textures");
        // Closed with its CLOSE: nothing held.
        let (_, texts) = frame(&ctx, &mut app, size, Vec::new());
        let (_, close, _) = texts.iter().find(|(text, _, _)| text == "CLOSE").unwrap().clone();
        let click = |at: Pos2| vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }];
        frame(&ctx, &mut app, size, click(close.center()));
        frame(&ctx, &mut app, size, vec![egui::Event::PointerButton { pos: close.center(), button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
        assert!(!app.supcom.open, "CLOSE closes it");
        app.supcom.poll(&ctx);
        assert_eq!(app.supcom.pictures_held(), (0, 0));
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The keys stop on each map once, at its box (not at its card as well), whose name says all
    /// the card shows: its name, size, players and whether it was imported. A long name gives the
    /// mark room. Going back up from the first row in sight scrolls the row above into sight
    /// rather than leaving the list.
    #[test]
    fn each_map_is_one_named_stop_for_the_keys_and_the_keys_scroll_the_list_both_ways() {
        let (root, ctx, mut app) = launcher("keys");
        app.supcom.open = true;
        let (six, previews) = found(6);
        app.supcom.pretend_found(six, previews);
        let size = egui::vec2(820.0, 900.0);
        for _ in 0..3 { frame(&ctx, &mut app, size, Vec::new()); }
        let (_, texts) = frame(&ctx, &mut app, size, Vec::new());
        let (_, title, _) = texts.iter().find(|(text, _, _)| text == "A Very Long Name For A Map Of The Game").expect("the long name").clone();
        let (_, mark, _) = texts.iter().filter(|(text, _, _)| text == "IMPORTED").min_by(|a, b| (a.1.top() - title.top()).abs().total_cmp(&(b.1.top() - title.top()).abs())).expect("its mark").clone();
        assert!(title.right() <= mark.left(), "the name ({title:?}) stops short of its mark ({mark:?})");
        let key = |key: egui::Key, modifiers: egui::Modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let mut boxes = Vec::new();
        for _ in 0..30 {
            let (output, _) = frame(&ctx, &mut app, size, vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
            if let Some(focused) = ctx.memory(|m| m.focused()).and_then(|id| ctx.read_response(id)) {
                assert!(focused.rect.height() < ROW_SHORTEST - 6.0, "a map's card took the keys: {:?}", focused.rect);
            }
            for event in &output.platform_output.events {
                if let egui::output::OutputEvent::FocusGained(info) = event
                    && info.typ == egui::WidgetType::Checkbox
                    && let Some(label) = &info.label
                    && !boxes.contains(label)
                {
                    boxes.push(label.clone());
                }
            }
        }
        assert_eq!(boxes.len(), 6, "{boxes:?}");
        assert!(boxes.iter().any(|label| label.starts_with("A Very Long Name For A Map Of The Game, 10 km · 2 players, IMPORTED")), "{boxes:?}");
        assert!(boxes.iter().any(|label| label == "Map 001, 10 km · 2 players"), "{boxes:?}");
        // A short window and many maps: down past the foot, then back up past the top.
        let (sixty, previews) = found(60);
        app.supcom.pretend_found(sixty, previews);
        let size = egui::vec2(900.0, 560.0);
        ctx.memory_mut(|m| if let Some(id) = m.focused() { m.surrender_focus(id) });
        for _ in 0..4 { frame(&ctx, &mut app, size, Vec::new()); }
        let focused_label = |output: &egui::FullOutput| output.platform_output.events.iter().rev().find_map(|event| match event {
            egui::output::OutputEvent::FocusGained(info) if info.typ == egui::WidgetType::Checkbox => info.label.clone(),
            _ => None,
        });
        let mut furthest = String::new();
        for _ in 0..40 {
            let (output, _) = frame(&ctx, &mut app, size, vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
            if let Some(label) = focused_label(&output) { furthest = label; }
        }
        assert!(furthest.starts_with("Map 0") && furthest.as_str() > "Map 015", "the keys went down the list, past its foot: {furthest}");
        let mut back = String::new();
        for _ in 0..12 {
            let (output, _) = frame(&ctx, &mut app, size, vec![key(egui::Key::Tab, egui::Modifiers::SHIFT)]);
            if let Some(label) = focused_label(&output) { back = label; }
        }
        assert!(back.starts_with("Map 0") && back.as_str() < furthest.as_str(), "Shift+Tab goes back up the list rather than out of it: from {furthest} to {back}");
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The factions' tab: the four factions, a card each with its units by kind and whether the
    /// mod holds it, a stop for the keys at each box named with all its card says; a faction
    /// unticked leaves it out; none chosen says so, and why IMPORT FACTIONS cannot be pressed.
    #[test]
    fn the_factions_tab_lists_each_faction_and_says_why_import_waits() {
        let (root, ctx, mut app) = launcher("factions");
        app.supcom.open = true;
        app.supcom.tab = Tab::Factions;
        let pictures = [("UEF", egui::ColorImage::filled([64, 64], Color32::BLUE))].into_iter().collect();
        app.supcom.factions.pretend_found(factions_found([State::Imported, State::New, State::Outdated, State::New]), pictures);
        let size = egui::vec2(900.0, 900.0);
        for _ in 0..4 { frame(&ctx, &mut app, size, Vec::new()); }
        let (_, texts) = frame(&ctx, &mut app, size, Vec::new());
        // (The mod holds the UEF and the Aeon: those are chosen at first.)
        assert!(texts.iter().any(|(text, _, _)| text == "2 of 4 factions chosen, 178 units, about 0.3 GB on the disk"), "{:?}", texts_of(&texts));
        assert!(texts.iter().any(|(text, _, _)| text.starts_with("Not played yet: Forged Alliance's own sounds")), "{:?}", texts_of(&texts));
        assert!(texts.iter().any(|(text, _, _)| text.contains("Online, everyone in a battle needs it")), "online play said: {:?}", texts_of(&texts));
        // The keys stop at each faction's box once.
        let tab = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let mut boxes = Vec::new();
        for _ in 0..20 {
            let (output, _) = frame(&ctx, &mut app, size, vec![tab.clone()]);
            for event in &output.platform_output.events {
                if let egui::output::OutputEvent::FocusGained(info) = event
                    && info.typ == egui::WidgetType::Checkbox
                    && let Some(label) = &info.label
                    && !boxes.contains(label)
                {
                    boxes.push(label.clone());
                }
            }
        }
        assert_eq!(boxes.len(), 4, "{boxes:?}");
        assert!(boxes.contains(&"UEF, 89 units, 47 buildings · 20 land · 12 air · 10 naval, IMPORTED".to_string()), "{boxes:?}");
        assert!(boxes.contains(&"CYBRAN, 89 units, 47 buildings · 20 land · 12 air · 10 naval".to_string()), "{boxes:?}");
        // Unticked, a faction the mod holds is said to be left out when it is made again.
        app.supcom.factions.choose("AEON", false);
        let (_, texts) = frame(&ctx, &mut app, size, Vec::new());
        assert!(texts.iter().any(|(text, _, _)| text.starts_with("The factions not chosen are taken out of the mod")), "{:?}", texts_of(&texts));
        // None chosen: said so, not "0 of 4 factions chosen, 0 units, about 0.0 GB", and why
        // IMPORT FACTIONS waits is written beside it.
        app.supcom.factions.choose("UEF", false);
        for _ in 0..2 { frame(&ctx, &mut app, size, Vec::new()); }
        let (_, texts) = frame(&ctx, &mut app, size, Vec::new());
        assert!(texts.iter().any(|(text, _, _)| text == "4 factions to choose from"), "{:?}", texts_of(&texts));
        assert!(texts.iter().any(|(text, _, _)| text == "Choose the factions to import."), "{:?}", texts_of(&texts));
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The music's tab: the soundtrack's tracks by their parts, each with how long it plays, how
    /// many, how long and what they take, and IMPORT MUSIC (nothing to choose: it is made whole);
    /// made by this version, MAKE AGAIN and up to date; while the factions are made, why it waits,
    /// and the factions' news with the way to their tab.
    #[test]
    fn the_music_tab_lists_the_soundtrack_by_its_parts_and_imports_it_whole() {
        let (root, ctx, mut app) = launcher("music");
        app.supcom.open = true;
        app.supcom.tab = Tab::Music;
        app.supcom.music.pretend_found(music_found(State::New));
        let size = egui::vec2(900.0, 900.0);
        for _ in 0..4 { frame(&ctx, &mut app, size, Vec::new()); }
        let (_, texts) = frame(&ctx, &mut app, size, Vec::new());
        for words in ["THE MENUS", "BUILDING", "BATTLE", "BRIEFINGS", "FILMS", "Menu theme", "Building theme 2", "Battle theme 1", "Briefing 1", "Credits", "1:21", "4:02", "3:00", "IMPORT MUSIC"] {
            assert!(texts.iter().any(|(text, _, _)| text == words), "{words}: {:?}", texts_of(&texts));
        }
        assert!(texts.iter().any(|(text, _, _)| text == "7 tracks, 17 minutes: about 0.1 GB on the disk"), "{:?}", texts_of(&texts));
        assert!(texts.iter().any(|(text, _, _)| text.contains("choose SOUNDTRACK in the game's Options, Sound")), "{:?}", texts_of(&texts));
        assert!(texts.iter().any(|(text, _, _)| text.contains("Online, the other players need not have it")), "{:?}", texts_of(&texts));
        assert!(!texts.iter().any(|(text, _, _)| text == "IMPORTED"), "not in yet: {:?}", texts_of(&texts));
        // Made by this version: MAKE AGAIN, and up to date.
        app.supcom.music.pretend_found(music_found(State::Imported));
        for _ in 0..2 { frame(&ctx, &mut app, size, Vec::new()); }
        let (_, texts) = frame(&ctx, &mut app, size, Vec::new());
        for words in ["IMPORTED", "MAKE AGAIN", "Imported and up to date."] {
            assert!(texts.iter().any(|(text, _, _)| text == words), "{words}: {:?}", texts_of(&texts));
        }
        // The factions being made: the music waits, said why, and the factions' news is at the top.
        app.supcom.factions.pretend_importing();
        for _ in 0..2 { frame(&ctx, &mut app, size, Vec::new()); }
        let (_, texts) = frame(&ctx, &mut app, size, Vec::new());
        assert!(texts.iter().any(|(text, _, _)| text == "Wait for the Supreme Commander factions being made, then import the music."), "{:?}", texts_of(&texts));
        assert!(texts.iter().any(|(text, _, _)| text == "SHOW"), "the way to the factions' tab: {:?}", texts_of(&texts));
        app.supcom.factions.finish_on_exit();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// During an import: its phase in words (reading, making, stopping once asked, installing),
    /// STOP greyed once pressed and while installing (said why in words), a CLOSE that says the
    /// import goes on, and the choices held, said so; the other tab says it is under way.
    #[test]
    fn an_import_under_way_says_what_it_does_and_that_it_goes_on_closed() {
        let (root, ctx, mut app) = launcher("during");
        app.supcom.open = true;
        let (found, previews) = found(12);
        app.supcom.pretend_found(found, previews);
        app.supcom.pretend_importing(&["scmp_001"]);
        {
            let mut p = app.supcom.progress.lock().unwrap();
            (p.phase, p.done, p.total, p.current) = (Phase::Making, 2, 12, "Seton's Clutch".into());
        }
        let size = egui::vec2(1120.0, 720.0);
        let painted = |app: &mut LauncherApp| {
            for _ in 0..3 { frame(&ctx, app, size, Vec::new()); }
            frame(&ctx, app, size, Vec::new()).1
        };
        let has = |texts: &[(String, Rect, Rect)], words: &str| texts.iter().any(|(text, _, _)| text == words);
        let texts = painted(&mut app);
        for words in ["Making Seton's Clutch (3 of 12)…", "STOP AFTER THIS MAP", "CLOSE", "Held until the import ends", "The import goes on with this closed: the launcher's footer shows how far it has got."] {
            assert!(has(&texts, words), "{words}: {:?}", texts_of(&texts));
        }
        app.supcom.stop();
        // (Stopped as asked: the pretend import ends at once; a real one makes the map in hand.)
        assert!(app.supcom.stopping());
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.supcom.importing() && std::time::Instant::now() < until {
            app.supcom.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // The factions installing: STOP greyed, said why; the maps' tab says the factions are made.
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        app.supcom.factions.pretend_found(factions_found([State::New; 4]), Default::default());
        app.supcom.factions.pretend_installing(done.clone());
        app.supcom.tab = Tab::Factions;
        let texts = painted(&mut app);
        for words in ["Installing the factions…", "STOP", "Installing cannot be stopped; it goes on with this closed."] {
            assert!(has(&texts, words), "{words}: {:?}", texts_of(&texts));
        }
        app.supcom.tab = Tab::Maps;
        let texts = painted(&mut app);
        assert!(has(&texts, "Installing the factions…") && has(&texts, "SHOW"), "the maps' tab tells of the factions' import: {:?}", texts_of(&texts));
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.supcom.factions.importing() && std::time::Instant::now() < until {
            frame(&ctx, &mut app, size, Vec::new());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }
}
