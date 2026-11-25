//! The Supreme Commander dialog's wizard (`crate::supcom_wizard`), in the dialog's own plate: its
//! steps (the game found, what to make, the import, what it came to), what drives its import from
//! the app's update ([`LauncherApp::wizard_tick`]: each part's choice set as its tab would set it,
//! then its tab's own import started, once the part before has ended), and its ways in: the Mods
//! page's offer of it and UPDATE ALL there, and the tabs' own.

use std::collections::BTreeSet;
use std::path::PathBuf;

use egui::{Color32, CornerRadius, RichText, ScrollArea, Sense, Stroke, Ui, vec2};
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{Exit, Panel};
use ta_ui::tooltip::TooltipExt as _;

use super::supcom::{Asked, Pick, failed_list, pick_card, progress_words};
use crate::app::{Action, LauncherApp};
use crate::supcom::{self, Installed, Phase, State, Tab, Went};
use crate::supcom_wizard::{Fate, Kind, ORDER, Part, Step};
use crate::theme::*;
use crate::widgets::{self, ActionButton, Glyph, Mark};

/// The wizard's size at most: as large on every step.
const WIDTH: f32 = 760.0;
const HEIGHT: f32 = 660.0;
/// The footer's height until it has been laid out (its row of buttons).
const FOOTER: f32 = 44.0;
/// A part's picture in the choice's cards, and a card's height at least.
const PICTURE: f32 = 56.0;
const ROW: f32 = PICTURE + 16.0;

/// What a part of the wizard makes, as its tab's look found it.
pub(crate) enum Choice {
    /// Its look not in yet.
    Looking,
    /// The game not found.
    NoGame,
    /// What it makes (`chosen`), in words, and about what it takes on the disk and how long to
    /// make (seconds), as its tab estimates them.
    Make { chosen: Chosen, detail: String, bytes: u64, seconds: u64 },
    /// Nothing to make: why (an update of maps no longer in the game's folder nor the player's).
    Nothing(String),
    /// It cannot be made from this copy of the game: why.
    Cannot(String),
}

/// What a part's import is given to make: its tab's choice.
pub(crate) enum Chosen {
    Maps(BTreeSet<PathBuf>),
    Factions(BTreeSet<String>),
    /// The music, the menus' look: made whole.
    Whole,
}

/// What the wizard's footer shows, taken before it is drawn.
struct Footer {
    step: Step,
    kind: Kind,
    /// Why NEXT (IMPORT) cannot be pressed now, in words beside it.
    why: Option<String>,
    /// The part in hand, STOP pressed, and the part in hand installing what it made.
    in_hand: Option<Tab>,
    stopping: bool,
    installing: bool,
    /// The factions or the maps were made: PLAY starts a skirmish's setup.
    skirmish: bool,
    /// Why PLAY cannot be pressed now.
    play: Option<&'static str>,
}

impl LauncherApp {
    /// The wizard in the dialog's place of the tabs (the dialog's own plate, its size the same on
    /// every step, its buttons in sight under a body that scrolls).
    pub(super) fn wizard_dialog(&mut self, ctx: &egui::Context) {
        let width = (ctx.content_rect().width() - 80.0).clamp(1.0, WIDTH);
        let height = (ctx.content_rect().height() - 24.0).clamp(1.0, HEIGHT);
        let footer = self.wizard_footer_state();
        let footer_id = egui::Id::new("launcher_supcom_wizard_footer");
        let footer_height = ctx.data(|d| d.get_temp::<f32>(footer_id)).unwrap_or(FOOTER);
        let mut actions = Vec::new();
        let mut asked = Asked::default();
        let panel = Panel::new("SUPREME COMMANDER").exit(Exit::Close).width(width).max_height(height);
        let response = ta_ui::panel::modal_with_footer(ctx, "launcher_supcom", panel, true, footer_height, |ui| self.wizard_body(ui, &mut actions), |ui| {
            let laid = ui.scope(|ui| wizard_footer(ui, &footer, &mut asked)).response.rect.height();
            if (laid - footer_height).abs() > 0.5 {
                ui.ctx().data_mut(|d| d.insert_temp(footer_id, laid));
                ui.ctx().request_discard("the Supreme Commander wizard's footer changed its height");
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

    /// The wizard's body: its steps in a row, and the one shown under them, scrolling there.
    fn wizard_body(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.spacing_mut().item_spacing.y = 6.0;
        let (kind, step) = (self.supcom.wizard.kind, self.supcom.wizard.step);
        steps_row(ui, kind, step);
        ui.add_space(space::XS);
        // (All the room left: the dialog as tall on every step.)
        let room = (ui.available_height() - 1.0).max(2.0 * ROW);
        let area = ScrollArea::vertical().id_salt(("supcom_wizard", step.label())).max_height(room).min_scrolled_height(room).auto_shrink([false, false]);
        ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Plate, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            match step {
                Step::Game => self.wizard_game(ui, actions),
                Step::Choose => self.wizard_choose(ui, actions),
                Step::Import | Step::Done => self.wizard_parts(ui, actions),
            }
        });
    }

    /// The game's step: what the wizard does, and the copy of the game found (with CHANGE
    /// FOLDER…), or what is needed without one.
    fn wizard_game(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.add(egui::Label::new(RichText::new(t("Supreme Commander: Forged Alliance's factions, maps, soundtrack and look for the menus, made into TA Reborn's from your own copy of the game: one after another, each into a mod of its own, turned on once made.")).color(TEXT)).wrap());
        ui.add_space(space::S);
        self.supcom_source_of(ui, actions, Tab::Maps, false);
        // (What it makes of the copy found, a picture of each part.)
        if self.supcom.game_at(Tab::Maps).is_some_and(|game| game.is_some()) {
            ui.add_space(space::L);
            self.part_tiles(ui);
        }
    }

    /// The four parts side by side, each a picture of what it makes over its name: the factions'
    /// commanders, a map, a loudspeaker for the music, one of the menus' pictures.
    fn part_tiles(&mut self, ui: &mut Ui) {
        let gap = 12.0;
        let width = ((ui.available_width() - gap * 3.0) / 4.0).floor().max(40.0);
        let height = (width * 0.75).round();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for tab in ORDER {
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 6.0;
                    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
                    self.paint_tile(ui, rect, tab);
                    ui.label(RichText::new(t(tab.label())).font(strong(12.5)).color(TEXT).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
                });
            }
        });
    }

    /// A part's tile: its picture (cut to the tile's shape), on a dark plate until it is read.
    fn paint_tile(&mut self, ui: &Ui, rect: egui::Rect, tab: Tab) {
        let ctx = ui.ctx().clone();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(2), Color32::from_black_alpha(120));
        match tab {
            Tab::Factions => {
                // (The four commanders in a row.)
                let sides: Vec<&'static str> = self.supcom.factions.found().map(|found| found.factions.iter().map(|e| e.side()).collect()).unwrap_or_default();
                let count = sides.len().max(1) as f32;
                let side = ((rect.width() - 8.0 - 4.0 * (count - 1.0)) / count).min(rect.height() - 8.0).max(1.0);
                let left = rect.center().x - (side * count + 4.0 * (count - 1.0)) / 2.0;
                for (i, name) in sides.into_iter().enumerate() {
                    if let Some(texture) = self.supcom.factions.texture(&ctx, name) {
                        let at = egui::Rect::from_min_size(egui::pos2(left + i as f32 * (side + 4.0), rect.center().y - side / 2.0), vec2(side, side));
                        painter.image(texture.id(), at, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                }
            }
            Tab::Maps => {
                let folder = self.supcom.found().and_then(|found| {
                    let own: Vec<&supcom::Entry> = found.maps.iter().filter(|e| !e.players_own).collect();
                    own.iter().find(|e| e.folder.file_name().is_some_and(|n| n.eq_ignore_ascii_case("SCMP_009"))).or(own.first()).map(|e| e.folder.clone())
                });
                if let Some(texture) = folder.and_then(|folder| self.supcom.texture(&ctx, &folder)) {
                    painter.image(texture.id(), rect, cover(&texture, rect), Color32::WHITE);
                }
            }
            Tab::Music => widgets::paint_glyph(&painter, Glyph::Speaker, rect.center(), rect.height() / 28.0, STEEL),
            Tab::Menus => {
                let first = self.supcom.menus.found().and_then(|found| found.contents.as_ref().and_then(|c| c.pictures.first().copied()));
                if let Some(texture) = first.and_then(|name| self.supcom.menus.texture(&ctx, name)) {
                    painter.image(texture.id(), rect, cover(&texture, rect), Color32::WHITE);
                }
            }
        }
        painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, STEEL.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    }

    /// What to make: a card for each part offered, ticked, with what it makes and takes (or why
    /// it cannot be made), the total, and the way to the tabs to choose in detail.
    fn wizard_choose(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let words = match self.supcom.wizard.kind {
            Kind::Rest => t("The parts not imported yet: each one ticked is made in turn, into a mod of its own, turned on once made."),
            _ => t("Each part ticked is made in turn, into a mod of its own, turned on once made."),
        };
        ui.add(egui::Label::new(RichText::new(words).color(TEXT)).wrap());
        // (The game no longer found: what is needed, here as on the game's step.)
        if self.supcom.game_at(Tab::Maps) == Some(None) {
            ui.add_space(space::XS);
            self.supcom_source_of(ui, actions, Tab::Maps, false);
        }
        ui.add_space(space::XS);
        let width = ui.available_width();
        let (mut bytes, mut seconds, mut ticked) = (0u64, 0u64, 0usize);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for tab in self.supcom.wizard.offered.clone() {
                let choice = self.wizard_choice(tab);
                let made = self.part_card(ui, tab, &choice, width);
                if let (true, Choice::Make { bytes: b, seconds: s, .. }) = (made, &choice) {
                    (bytes, seconds, ticked) = (bytes + b, seconds + s, ticked + 1);
                }
            }
        });
        ui.add_space(space::XS);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            if ticked > 0 {
                ui.label(RichText::new(tf("In all: about {size} on the disk, and {time} to make.", &[("size", &supcom::size_text(bytes)), ("time", &supcom::time_text(seconds))])).color(TEXT));
            }
            if widgets::text_link(ui, "CHOOSE IN DETAIL…").hover_tip(t("The dialog's tabs: the maps one by one (your own too), the factions you choose, each part made or made again on its own.")).clicked() {
                actions.push(Action::SupcomTabs);
            }
        });
    }

    /// A part's card in the choice: its box (ticked: it is made), its picture, its name, what it
    /// makes and takes; held unticked, and said why, where it cannot be made. Whether it is to be
    /// made.
    fn part_card(&mut self, ui: &mut Ui, tab: Tab, choice: &Choice, width: f32) -> bool {
        let blocked = self.part_blocked_for_good(tab);
        let looking = t("Looking for Supreme Commander: Forged Alliance…");
        let missing = t("Waiting for Supreme Commander: Forged Alliance to be found.");
        let estimate;
        let (lines, held): (Vec<(&str, Color32)>, Option<&str>) = match choice {
            Choice::Looking => (vec![(looking, STEEL)], Some(looking)),
            Choice::NoGame => (vec![(missing, AMBER)], Some(missing)),
            Choice::Nothing(words) => (vec![(words.as_str(), STEEL)], Some(words.as_str())),
            Choice::Cannot(why) => (vec![(why.as_str(), AMBER)], Some(why.as_str())),
            Choice::Make { detail, .. } if blocked.is_some() => (vec![(detail.as_str(), TEXT), (blocked.unwrap_or_default(), AMBER)], blocked),
            Choice::Make { detail, bytes, seconds, .. } => {
                estimate = tf("About {size} on the disk, {time} to make", &[("size", &supcom::size_text(*bytes)), ("time", &supcom::time_text(*seconds))]);
                (vec![(detail.as_str(), TEXT), (estimate.as_str(), STEEL)], None)
            }
        };
        let makeable = held.is_none();
        let mut ticked = makeable && self.supcom.wizard.is_ticked(tab);
        let (picture, glyph) = self.part_picture(ui.ctx(), tab, choice);
        let title = t(tab.label());
        let spoken = std::iter::once(title).chain(lines.iter().map(|(words, _)| *words)).collect::<Vec<_>>().join(", ");
        let pick = Pick { id: egui::Id::new(("supcom_wizard_part", tab.label())), width, height: ROW, picture, crop: true, glyph, picture_side: PICTURE, title, title_size: 15.0, lines: &lines, wrap: true, mark: None, spoken: &spoken, held, hover: None };
        let (toggled, _) = pick_card(ui, &pick, &mut ticked);
        if toggled && makeable {
            self.supcom.wizard.tick(tab, ticked);
        }
        makeable && self.supcom.wizard.is_ticked(tab)
    }

    /// A part's picture in its card: a map of those it makes (Seton's Clutch where it is one), a
    /// faction's commander, one of the menus' pictures; the music a loudspeaker.
    fn part_picture(&mut self, ctx: &egui::Context, tab: Tab, choice: &Choice) -> (Option<egui::TextureHandle>, Option<Glyph>) {
        match (tab, choice) {
            (Tab::Maps, Choice::Make { chosen: Chosen::Maps(folders), .. }) => {
                let shown = folders.iter().find(|f| f.file_name().is_some_and(|n| n.eq_ignore_ascii_case("SCMP_009"))).or_else(|| folders.iter().next()).cloned();
                (shown.and_then(|folder| self.supcom.texture(ctx, &folder)), None)
            }
            (Tab::Factions, Choice::Make { chosen: Chosen::Factions(sides), .. }) => {
                let side = self.supcom.factions.found().and_then(|found| found.factions.iter().map(|e| e.side()).find(|side| sides.contains(*side)));
                (side.and_then(|side| self.supcom.factions.texture(ctx, side)), None)
            }
            (Tab::Menus, Choice::Make { .. }) => {
                let first = self.supcom.menus.found().and_then(|found| found.contents.as_ref().and_then(|c| c.pictures.first().copied()));
                (first.and_then(|name| self.supcom.menus.texture(ctx, name)), None)
            }
            (Tab::Music, _) => (None, Some(Glyph::Speaker)),
            _ => (None, None),
        }
    }

    /// The import's step and its last: each part as it stands (waiting, being made with its
    /// progress, made, or not and why); at the last, what to do next. Without the game, what is
    /// needed: the import waits for it.
    fn wizard_parts(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let done = self.supcom.wizard.step == Step::Done;
        let words = match (done, self.supcom.wizard.kind) {
            (true, _) => t("What each part came to: those made are installed, and turned on."),
            (false, Kind::Update) => t("Each part an older TA Reborn made is made again from your copy of the game, in turn, as before."),
            (false, _) => t("Each part ticked is made in turn, into a mod of its own, turned on once made."),
        };
        ui.add(egui::Label::new(RichText::new(words).color(TEXT)).wrap());
        let waiting_for_game = !done && self.supcom.wizard.next().is_some_and(|tab| self.supcom.game_at(tab) == Some(None));
        if waiting_for_game {
            ui.add_space(space::XS);
            self.supcom_source_of(ui, actions, Tab::Maps, false);
        }
        ui.add_space(space::XS);
        let stopping = self.supcom.wizard.stopping;
        for part in self.supcom.wizard.parts.clone() {
            self.part_line(ui, &part, stopping);
        }
        if done {
            ui.add_space(space::S);
            ui.add(egui::Label::new(RichText::new(t("SUPREME COMMANDER… on the Mods and Maps pages makes or removes each part on its own: more of the game's maps and your own, the factions you choose, a part made again.")).color(STEEL)).wrap());
        }
    }

    /// A part of the import as it stands: its mark, its name, and its words (its progress and rail
    /// while it is made; its import's own words once made, what it could not make folded under
    /// them).
    fn part_line(&mut self, ui: &mut Ui, part: &Part, stopping: bool) {
        let tab = part.tab;
        egui::Frame::new().fill(Color32::from_black_alpha(60)).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(3)).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                fate_mark(ui, &part.fate);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.label(RichText::new(t(tab.label())).font(strong(14.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
                    let line = |ui: &mut Ui, words: &str, colour: Color32| {
                        ui.add(egui::Label::new(RichText::new(words).color(colour)).wrap());
                    };
                    match &part.fate {
                        Fate::Waiting(_) if stopping => line(ui, t("Not made: the import was stopped before it."), STEEL),
                        Fate::Waiting(Some(why)) => line(ui, why, STEEL),
                        Fate::Waiting(None) => line(ui, t("Waiting"), STEEL),
                        Fate::Asking => line(ui, t("Waiting for your answer: another mod of its id is installed."), AMBER),
                        Fate::Making => {
                            let progress = self.supcom.progress_at(tab);
                            line(ui, &progress_words(tab, &progress, self.supcom.stopping_at(tab)), TEXT);
                            let (rail, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
                            ta_ui::progress::paint(ui, rail, progress.fraction(), true);
                        }
                        Fate::Ended { went, words, failed } => {
                            line(ui, words, went.colour());
                            failed_list(ui, tab, failed, "supcom_wizard_failed");
                        }
                        Fate::Failed(why) => line(ui, why, RED),
                        Fate::Left => line(ui, t("Not made: the import was stopped before it."), STEEL),
                    }
                });
            });
        });
    }

    /// What the wizard's footer shows.
    fn wizard_footer_state(&self) -> Footer {
        let wizard = &self.supcom.wizard;
        let in_hand = wizard.in_hand();
        let why = match wizard.step {
            Step::Game => self.wizard_game_why(),
            Step::Choose => self.wizard_import_why(),
            Step::Import | Step::Done => None,
        };
        let made = |tab: Tab| wizard.fate(tab).is_some_and(|f| matches!(f, Fate::Ended { went: Went::Well | Went::Partly, .. }));
        let play = if self.detection.install().is_none() {
            Some(t("Find your Total Annihilation install on the Play page first."))
        } else if self.game_running() {
            Some(t("The game is running."))
        } else {
            None
        };
        Footer {
            step: wizard.step,
            kind: wizard.kind,
            why,
            in_hand,
            stopping: wizard.stopping,
            installing: in_hand.is_some_and(|tab| self.supcom.progress_at(tab).phase == Phase::Installing),
            skirmish: made(Tab::Factions) || made(Tab::Maps),
            play,
        }
    }

    /// Why NEXT cannot be pressed on the game's step: the game being looked for, or not found.
    fn wizard_game_why(&self) -> Option<String> {
        if self.supcom.scanning_at(Tab::Maps) || self.supcom.checking().is_some() || self.supcom.game_at(Tab::Maps).is_none() {
            Some(t("Looking for Supreme Commander: Forged Alliance…").to_string())
        } else if self.supcom.game_at(Tab::Maps) == Some(None) {
            Some(t("Waiting for Supreme Commander: Forged Alliance to be found.").to_string())
        } else {
            None
        }
    }

    /// Why IMPORT cannot be pressed: nothing ticked that can be made, a part still looked for, or
    /// what keeps the first part from starting now (the game running, an install, an import).
    fn wizard_import_why(&self) -> Option<String> {
        let ticked: Vec<Tab> = self.supcom.wizard.offered.iter().copied().filter(|tab| self.supcom.wizard.is_ticked(*tab)).collect();
        if ticked.iter().any(|tab| matches!(self.wizard_choice(*tab), Choice::Looking)) {
            return Some(t("Looking for Supreme Commander: Forged Alliance…").to_string());
        }
        let makeable: Vec<Tab> = ticked.into_iter().filter(|tab| self.part_makeable(*tab)).collect();
        let Some(first) = makeable.first() else { return Some(t("Tick a part to make.").to_string()) };
        self.tab_blocked(*first).map(str::to_string)
    }

    /// Whether the part `tab` can be made now, as its card shows it: something to make, nothing
    /// in its way for good.
    fn part_makeable(&self, tab: Tab) -> bool {
        matches!(self.wizard_choice(tab), Choice::Make { .. }) && self.part_blocked_for_good(tab).is_none()
    }

    /// What the part `tab` makes, as its tab's look found it: for the first import and the rest,
    /// the game's own maps not imported yet (the player's own are the MAPS tab's to choose) and
    /// every faction; for an update, the maps an older converter made and the factions the mod
    /// holds; the music and the menus' look whole.
    pub(crate) fn wizard_choice(&self, tab: Tab) -> Choice {
        let update = self.supcom.wizard.kind == Kind::Update;
        if self.supcom.scanning_at(tab) {
            return Choice::Looking;
        }
        match self.supcom.game_at(tab) {
            None => return Choice::Looking,
            Some(None) => return Choice::NoGame,
            Some(Some(_)) => {}
        }
        match tab {
            Tab::Maps => {
                let Some(found) = self.supcom.found() else { return Choice::Looking };
                let only = self.supcom.wizard.maps_only.as_ref();
                let entries: Vec<&supcom::Entry> = found.maps.iter()
                    .filter(|e| if update { e.state == State::Outdated } else { !e.players_own && e.state != State::Imported })
                    .filter(|e| only.is_none_or(|names| e.folder.file_name().is_some_and(|f| names.iter().any(|n| f.to_string_lossy().eq_ignore_ascii_case(n)))))
                    .collect();
                if entries.is_empty() {
                    return if let Some(error) = &found.error {
                        Choice::Cannot(error.clone())
                    } else if update {
                        Choice::Nothing(t("None of the maps an older TA Reborn made is in this copy of the game or your own maps' folders now: the pack keeps them as they are.").to_string())
                    } else if found.maps.iter().any(|e| !e.players_own) {
                        Choice::Nothing(t("The game's maps are imported already.").to_string())
                    } else {
                        Choice::Cannot(t("No skirmish maps were found in this copy of the game.").to_string())
                    };
                }
                let detail = if update {
                    tf("The {n} maps an older TA Reborn made, made again", &[("n", &entries.len())])
                } else {
                    tf("The game's {n} skirmish maps (your own maps are on the MAPS tab)", &[("n", &entries.len())])
                };
                Choice::Make { bytes: supcom::size_estimate(&entries), seconds: supcom::time_estimate(&entries), chosen: Chosen::Maps(entries.iter().map(|e| e.folder.clone()).collect()), detail }
            }
            Tab::Factions => {
                let Some(found) = self.supcom.factions.found() else { return Choice::Looking };
                let entries: Vec<crate::supcom_factions::Entry> = found.factions.iter().filter(|e| !update || e.state != State::New).cloned().collect();
                if entries.is_empty() {
                    return match (&found.error, update) {
                        (Some(error), _) => Choice::Cannot(error.clone()),
                        (None, true) => Choice::Nothing(t("The factions' mod holds none of this copy's factions: nothing to make again.").to_string()),
                        (None, false) => Choice::Cannot(t("No units of the four factions were found in this copy of the game.").to_string()),
                    };
                }
                let sides: Vec<&str> = entries.iter().map(|e| e.side()).collect();
                let units: usize = entries.iter().map(|e| e.units()).sum();
                let detail = tf("{factions}: {units} units, each faction a side to play in skirmish", &[("factions", &super::listed(&sides)), ("units", &units)]);
                Choice::Make {
                    bytes: crate::supcom_factions::size_estimate(&entries),
                    seconds: crate::supcom_factions::time_estimate(&entries),
                    chosen: Chosen::Factions(sides.iter().map(|s| s.to_string()).collect()),
                    detail,
                }
            }
            Tab::Music => {
                let Some(found) = self.supcom.music.found() else { return Choice::Looking };
                if found.tracks.is_empty() {
                    return Choice::Cannot(found.error.clone().unwrap_or_else(|| t("No tracks were found to import.").to_string()));
                }
                let minutes = (found.seconds() / 60.0).round() as u64;
                let detail = tf("The soundtrack's {n} tracks, {minutes} minutes, a lossless file each: SOUNDTRACK plays them", &[("n", &found.tracks.len()), ("minutes", &minutes)]);
                Choice::Make { bytes: crate::supcom_music::size_estimate(&found.tracks), seconds: crate::supcom_music::time_estimate(&found.tracks), chosen: Chosen::Whole, detail }
            }
            Tab::Menus => {
                let Some(found) = self.supcom.menus.found() else { return Choice::Looking };
                let Some(contents) = &found.contents else {
                    return Choice::Cannot(found.error.clone().unwrap_or_else(|| t("Nothing of the look was found to import.").to_string()));
                };
                let detail = tf("The game's menus in its colours, {pictures} pictures, panel art, {sounds} sounds and menu music", &[("pictures", &contents.pictures.len()), ("sounds", &contents.sounds.len())]);
                Choice::Make { bytes: crate::supcom_menus::SIZE, seconds: crate::supcom_menus::SECONDS, chosen: Chosen::Whole, detail }
            }
        }
    }

    /// What keeps the part `tab` from being made at all, until the player does something about it:
    /// TA's own install not found (the maps are made of its features), or the copy of its mod an
    /// import cannot replace (in TA Reborn's program folder, or an archive).
    fn part_blocked_for_good(&self, tab: Tab) -> Option<&'static str> {
        if tab == Tab::Maps && self.detection.install().is_none() {
            return Some(t("Find your Total Annihilation install on the Play page first."));
        }
        let (id, name) = tab.mod_of();
        Installed::listed(&self.mods, id, name).and_then(|copy| copy.blocks())
    }

    /// What keeps the tab `tab`'s import from starting now, in its tab's words.
    fn tab_blocked(&self, tab: Tab) -> Option<&'static str> {
        match tab {
            Tab::Maps => self.supcom_blocked(),
            Tab::Factions => self.factions_blocked(),
            Tab::Music => self.music_blocked(),
            Tab::Menus => self.menus_blocked(),
        }
    }

    /// The Supreme Commander imports not made yet (their mods not installed as an import made
    /// them), in the wizard's order.
    pub(crate) fn supcom_missing(&self) -> Vec<Tab> {
        ORDER.into_iter().filter(|tab| {
            let (id, name) = tab.mod_of();
            Installed::listed(&self.mods, id, name).is_none_or(|copy| !copy.ours)
        }).collect()
    }

    /// Those an older TA Reborn made, to be made again (UPDATE ALL), in the wizard's order.
    pub(crate) fn supcom_outdated(&self) -> Vec<Tab> {
        ORDER.into_iter().filter(|tab| supcom::update_tab(&self.mods, tab.mod_of().0).is_some()).collect()
    }

    // ---- The wizard's actions --------------------------------------------------------------

    /// Opens the dialog at the wizard as `kind`: every part, those not imported yet, or those an
    /// older TA Reborn made (its import begun at once); at it as it is, where its import is under
    /// way or what it came to is not seen yet; at the tabs, where there is nothing to offer or a
    /// tab's import is under way.
    pub(crate) fn open_supcom_wizard(&mut self, kind: Kind) {
        let roots = self.supcom_roots();
        let tab = self.supcom.tab;
        if self.supcom.wizard.running() || self.supcom.wizard.unseen {
            self.supcom.show_wizard_again(tab, roots);
            return;
        }
        let parts = match kind {
            Kind::First => ORDER.to_vec(),
            Kind::Rest => self.supcom_missing(),
            Kind::Update => self.supcom_outdated(),
        };
        if parts.is_empty() || self.supcom.any_importing() {
            self.open_supcom(tab);
            return;
        }
        self.supcom.show_wizard(kind, &parts, tab, roots);
    }

    /// `--show supcom-wizard[:<step>[:<maps>]]` (and `supcom-rest`, `supcom-update`): the wizard
    /// as `kind`, gone on by itself to `step` once the game is looked through (`choose`; `import`
    /// and `done` begin its import, every part ticked), its maps' part making only the maps
    /// named in `maps` (their folders, a comma between). Whether it imports (a capture waits for
    /// it).
    pub(crate) fn show_supcom_wizard(&mut self, kind: Kind, step: &str, maps: &str) -> bool {
        let parts = match kind {
            Kind::First => ORDER.to_vec(),
            Kind::Rest => self.supcom_missing(),
            Kind::Update => self.supcom_outdated(),
        };
        let roots = self.supcom_roots();
        let tab = self.supcom.tab;
        // (Nothing of the kind to make: the tabs, as UPDATE ALL and IMPORT IN ONE GO… open them.)
        if parts.is_empty() {
            self.open_supcom(tab);
            return false;
        }
        self.supcom.show_wizard(kind, &parts, tab, roots);
        self.supcom.wizard.auto = match step {
            "choose" => Some(Step::Choose),
            "import" => Some(Step::Import),
            "done" => Some(Step::Done),
            _ => None,
        };
        if !maps.is_empty() {
            self.supcom.wizard.maps_only = Some(maps.split(',').map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).collect());
        }
        matches!(self.supcom.wizard.auto, Some(Step::Import | Step::Done)) || kind == Kind::Update
    }

    /// NEXT and BACK: to the game's step or the choice's, while the import has not begun.
    pub(crate) fn wizard_go(&mut self, step: Step) {
        let wizard = &mut self.supcom.wizard;
        if wizard.shown && matches!(wizard.step, Step::Game | Step::Choose) && matches!(step, Step::Game | Step::Choose) {
            wizard.step = step;
        }
    }

    /// IMPORT: the parts ticked that can be made, made one after another (the first starts in the
    /// app's update, at once).
    pub(crate) fn begin_wizard_import(&mut self) {
        if !self.supcom.wizard.shown || self.supcom.wizard.step != Step::Choose || self.wizard_import_why().is_some() {
            return;
        }
        let parts: Vec<Tab> = self.supcom.wizard.offered.iter().copied().filter(|tab| self.supcom.wizard.is_ticked(*tab) && self.part_makeable(*tab)).collect();
        if !parts.is_empty() {
            self.supcom.wizard.begin(parts);
        }
    }

    /// STOP: the import ends once the part in hand is made; the maps' import (the last part, and
    /// the longest) once the map in hand is, the maps made so far kept.
    pub(crate) fn stop_wizard(&mut self) {
        self.supcom.wizard.stop();
        if self.supcom.wizard.in_hand() == Some(Tab::Maps) {
            self.supcom.stop();
        }
    }

    /// CHOOSE IN DETAIL… (and BACK from the parts not imported yet): the tabs in the wizard's
    /// place, at the tab the dialog was opened at.
    pub(crate) fn supcom_tabs(&mut self) {
        if self.supcom.wizard.running() {
            return;
        }
        self.supcom.wizard = Default::default();
        let tab = self.supcom.tab;
        self.supcom.set_tab(tab);
    }

    /// The Mods page's offer taken up (the dialog opens, at the wizard) or put away: not made
    /// again either way (the settings keep it).
    pub(crate) fn take_supcom_offer(&mut self, take: bool) {
        self.supcom.offer.done = true;
        if let Err(e) = supcom::save_offered(&self.store.dir()) {
            log::warn!("the Supreme Commander offer could not be kept in the settings: {e}");
        }
        if take {
            self.open_supcom(Tab::Factions);
        }
    }

    // ---- The wizard's import, driven from the app's update ---------------------------------

    /// The wizard's import, a step on: the part in hand taken in once its import has ended (or
    /// once the question over another mod of its id is answered), then the next part started:
    /// its look in, its choice set as its tab's, nothing in its way (it waits for what passes,
    /// and fails, said why, for what does not), its tab's own import started. Once every part's
    /// turn is past, its last step, and a notice where the dialog is closed. (A capture's steps
    /// first.)
    pub(crate) fn wizard_tick(&mut self) {
        self.wizard_auto();
        if self.supcom.wizard.step != Step::Import {
            return;
        }
        if let Some(tab) = self.supcom.wizard.in_hand() {
            let importing = self.supcom.importing_at(tab);
            match self.supcom.wizard.fate(tab) {
                // (REPLACE AND IMPORT: under way; KEEP: not made.)
                Some(Fate::Asking) if importing => self.supcom.wizard.set(tab, Fate::Making),
                Some(Fate::Asking) if self.supcom.confirm_replace.is_some() => return,
                Some(Fate::Asking) => self.supcom.wizard.set(tab, Fate::Failed(t("Not made: the other mod of its id was kept.").to_string())),
                _ if importing => return,
                _ => {
                    let fate = self.part_outcome(tab);
                    self.supcom.wizard.set(tab, fate);
                }
            }
            if self.supcom.wizard.in_hand().is_some() {
                return;
            }
        }
        if self.supcom.wizard.stopping {
            self.supcom.wizard.leave_rest();
        }
        while let Some(tab) = self.supcom.wizard.next() {
            let fate = self.start_part(tab);
            let over = fate.over();
            self.supcom.wizard.set(tab, fate);
            if !over {
                return;
            }
        }
        if self.supcom.wizard.step == Step::Import && !self.supcom.wizard.running() {
            let open = self.supcom.open;
            self.supcom.wizard.finish(open);
            if !open {
                let went = self.supcom.wizard.went();
                let words = match went {
                    Went::Well => t("The Supreme Commander imports are made and turned on: SUPREME COMMANDER… on the Mods page says what each came to."),
                    Went::Nothing => t("The Supreme Commander imports were stopped before anything was made."),
                    _ => t("The Supreme Commander imports are done, but not all went well: SUPREME COMMANDER… on the Mods page says why."),
                };
                self.notify(words, went.colour());
            }
        }
    }

    /// A capture's wizard, gone on by itself once every tab's look is in: to what to make, or its
    /// import begun.
    fn wizard_auto(&mut self) {
        let Some(target) = self.supcom.wizard.auto else { return };
        if !self.supcom.wizard.shown {
            self.supcom.wizard.auto = None;
            return;
        }
        if Tab::ALL.into_iter().any(|tab| self.supcom.scanning_at(tab) || self.supcom.game_at(tab).is_none()) {
            return;
        }
        if matches!(target, Step::Choose | Step::Import | Step::Done) {
            self.wizard_go(Step::Choose);
        }
        if matches!(target, Step::Import | Step::Done) {
            self.begin_wizard_import();
        }
        self.supcom.wizard.auto = None;
    }

    /// The part `tab` started, as far as it can be now: its fate.
    fn start_part(&mut self, tab: Tab) -> Fate {
        let looking = || Fate::Waiting(Some(t("Looking for Supreme Commander: Forged Alliance…").to_string()));
        // Its look: under way, not made (or made with the dialog closed), failed, or without the
        // game.
        if self.supcom.scanning_at(tab) {
            return looking();
        }
        if self.supcom.look_failed_at(tab) {
            return Fate::Failed(t("Looking for the game failed.").to_string());
        }
        match self.supcom.game_at(tab) {
            None => {
                self.supcom.rescan_tab(tab);
                return looking();
            }
            Some(None) => return Fate::Waiting(Some(t("Waiting for Supreme Commander: Forged Alliance to be found.").to_string())),
            Some(Some(_)) => {}
        }
        // Its choice, as its tab's.
        match self.wizard_choice(tab) {
            Choice::Looking | Choice::NoGame => return looking(),
            Choice::Nothing(words) => return Fate::Ended { went: Went::Nothing, words, failed: Vec::new() },
            Choice::Cannot(why) => return Fate::Failed(why),
            Choice::Make { chosen: Chosen::Maps(folders), .. } => self.supcom.chosen = Some(folders),
            Choice::Make { chosen: Chosen::Factions(sides), .. } => self.supcom.factions.chosen = Some(sides),
            Choice::Make { chosen: Chosen::Whole, .. } => {}
        }
        if let Some(why) = self.part_blocked_for_good(tab) {
            return Fate::Failed(why.to_string());
        }
        // (The game started meanwhile, a mod being installed: it waits for them, said why.)
        if let Some(why) = self.tab_blocked(tab) {
            return Fate::Waiting(Some(why.to_string()));
        }
        // (What it came to is its own from here: an earlier import's left behind would be
        // taken for it.)
        self.forget_outcome(tab);
        match self.begin_supcom_import(tab, false) {
            Err(why) => Fate::Failed(why.to_string()),
            Ok(()) if self.supcom.importing_at(tab) => Fate::Making,
            Ok(()) if self.supcom.confirm_replace.is_some() => Fate::Asking,
            Ok(()) => Fate::Failed(self.outcome_error(tab).unwrap_or_else(|| t("The import stopped unexpectedly.").to_string())),
        }
    }

    /// What the part `tab`'s import came to, in its own words, with what it could not make.
    fn part_outcome(&self, tab: Tab) -> Fate {
        let told = match tab {
            Tab::Maps => self.supcom.outcome.as_ref().map(|o| (o.went(), o.summary(), o.failed.clone())),
            Tab::Factions => self.supcom.factions.outcome.as_ref().map(|o| (o.went(), o.summary(), o.skipped.clone())),
            Tab::Music => self.supcom.music.outcome.as_ref().map(|o| (o.went(), o.summary(), o.skipped.clone())),
            Tab::Menus => self.supcom.menus.outcome.as_ref().map(|o| (o.went(), o.summary(), o.skipped.clone())),
        };
        match told {
            Some((went, words, failed)) => Fate::Ended { went, words, failed },
            None => Fate::Failed(t("The import stopped unexpectedly.").to_string()),
        }
    }

    /// The tab `tab`'s last import forgotten, before the wizard starts it again.
    fn forget_outcome(&mut self, tab: Tab) {
        match tab {
            Tab::Maps => self.supcom.outcome = None,
            Tab::Factions => self.supcom.factions.outcome = None,
            Tab::Music => self.supcom.music.outcome = None,
            Tab::Menus => self.supcom.menus.outcome = None,
        }
    }

    /// Why the tab `tab`'s import could not start, as it said it.
    fn outcome_error(&self, tab: Tab) -> Option<String> {
        match tab {
            Tab::Maps => self.supcom.outcome.as_ref().and_then(|o| o.error.clone()),
            Tab::Factions => self.supcom.factions.outcome.as_ref().and_then(|o| o.error.clone()),
            Tab::Music => self.supcom.music.outcome.as_ref().and_then(|o| o.error.clone()),
            Tab::Menus => self.supcom.menus.outcome.as_ref().and_then(|o| o.error.clone()),
        }
    }

    // ---- Its ways in ----------------------------------------------------------------------

    /// The tabs' ways to the wizard, at the top of their contents: UPDATE ALL for the parts an
    /// older TA Reborn made, and the parts not imported yet in one go (held while an import is
    /// under way).
    pub(super) fn wizard_offers(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let held = self.supcom.any_importing().then(|| t("Held until the import ends"));
        for (kind, parts) in [(Kind::Update, self.supcom_outdated()), (Kind::Rest, self.supcom_missing())] {
            if parts.is_empty() {
                continue;
            }
            let named = parts_named(&parts);
            let (words, colour, label, tip) = match kind {
                Kind::Update => (tf("Made by an older TA Reborn: {parts}.", &[("parts", &named)]), AMBER, n("UPDATE ALL"), t("Makes each of them again from your copy of the game, one after another, as before.")),
                _ => (tf("Not imported yet: {parts}.", &[("parts", &named)]), STEEL, n("IMPORT IN ONE GO…"), t("Makes them one after another, each into a mod of its own, turned on once made.")),
            };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add(egui::Label::new(RichText::new(words).color(colour)).wrap());
                let button = ui.add_enabled(held.is_none(), ActionButton::secondary(label).height(26.0));
                let button = match held {
                    Some(why) => button.disabled_tip(why),
                    None => button.hover_tip(tip),
                };
                if button.clicked() {
                    actions.push(Action::SupcomWizard(kind));
                }
            });
            ui.add_space(space::XS);
        }
    }

    /// The Mods page's word on the Supreme Commander imports, above the library: UPDATE ALL where
    /// an older TA Reborn made some; else, once, the offer of the wizard where the game is found
    /// on this computer and nothing of it is imported (until taken up or put away).
    pub(super) fn supcom_offers(&mut self, ui: &mut Ui) {
        let busy = self.supcom.any_importing() || self.supcom.wizard.running();
        let outdated = self.supcom_outdated();
        if !outdated.is_empty() {
            if busy {
                return;
            }
            let running = self.game_running();
            let words = tf("Supreme Commander imports were made by an older TA Reborn: {parts}.", &[("parts", &parts_named(&outdated))]);
            let update = offer_bar(ui, AMBER, &words, |ui| {
                let button = ui.add_enabled(!running, ActionButton::primary("UPDATE ALL").height(28.0));
                let button = if running { button.disabled_tip(t("Close the game first: it may be using these mods.")) } else { button.hover_tip(t("Makes each of them again from your copy of the game, one after another, as before.")) };
                button.clicked().then_some(Action::SupcomWizard(Kind::Update))
            });
            if let Some(action) = update {
                self.push(action);
            }
            return;
        }
        if self.supcom.offer.done || busy || self.supcom_missing().len() < Tab::ALL.len() {
            return;
        }
        self.supcom.offer.seek();
        self.supcom.offer.poll(ui.ctx());
        if self.supcom.offer.found != Some(true) {
            return;
        }
        let taken = offer_bar(ui, SKY, t("Supreme Commander: Forged Alliance is installed on this computer: its factions, maps, soundtrack and look for the menus can be made into TA Reborn's from it."), |ui| {
            if ui.add(ActionButton::primary("IMPORT…").height(28.0)).hover_tip(t("Opens the Supreme Commander import: the game found, what to make, and the import, one part after another.")).clicked() {
                return Some(Action::SupcomOffer(true));
            }
            let later = ui.add(ActionButton::secondary("NOT NOW").height(28.0)).hover_tip(t("This is not offered again: SUPREME COMMANDER… above makes it whenever you like."));
            later.clicked().then_some(Action::SupcomOffer(false))
        });
        if let Some(action) = taken {
            self.push(action);
        }
    }
}

/// A slim card over the Mods library: a lamp, its words, and its buttons after them (`buttons`:
/// what they asked for).
fn offer_bar(ui: &mut Ui, lamp: Color32, words: &str, buttons: impl FnOnce(&mut Ui) -> Option<Action>) -> Option<Action> {
    let asked = widgets::card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (rect, _) = ui.allocate_exact_size(vec2(10.0, 18.0), Sense::hover());
            ui.painter().add(ta_ui::metal::lamp(rect.center(), 3.5, lamp, 1.0));
            ui.add(egui::Label::new(RichText::new(words).color(TEXT)).wrap());
            buttons(ui)
        })
        .inner
    })
    .inner;
    ui.add_space(space::S);
    asked
}

/// The part of `texture` that covers `rect` (cut to its shape, not squeezed into it), as texture
/// coordinates.
fn cover(texture: &egui::TextureHandle, rect: egui::Rect) -> egui::Rect {
    let (have, want) = (texture.aspect_ratio(), rect.aspect_ratio());
    if have > want {
        let w = want / have;
        egui::Rect::from_min_max(egui::pos2((1.0 - w) / 2.0, 0.0), egui::pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = have / want;
        egui::Rect::from_min_max(egui::pos2(0.0, (1.0 - h) / 2.0), egui::pos2(1.0, (1.0 + h) / 2.0))
    }
}

/// The parts named as a sentence names them ("the factions and the music").
fn parts_named(parts: &[Tab]) -> String {
    super::listed(&parts.iter().map(|tab| t(tab.part())).collect::<Vec<_>>())
}

/// The wizard's steps in a row, numbered, the one shown lit.
fn steps_row(ui: &mut Ui, kind: Kind, step: Step) {
    let steps = Step::of(kind);
    let at = steps.iter().position(|s| *s == step).unwrap_or(0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        for (i, s) in steps.iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new("›").font(strong(13.0)).color(STEEL_DIM));
            }
            let colour = if i == at { AMBER_BRIGHT } else if i < at { STEEL } else { STEEL_DIM };
            ui.label(RichText::new(format!("{}  {}", i + 1, t(s.label()))).font(strong(12.0)).color(colour).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
        }
    });
}

/// A part's mark as it stands: a lamp while it waits (dim) or is made (lit), a check once made
/// well, amber where only some was made, a cross where nothing was.
fn fate_mark(ui: &mut Ui, fate: &Fate) {
    let lamp = |ui: &mut Ui, colour: Color32, on: f32| {
        let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        ui.painter().add(ta_ui::metal::lamp(rect.center(), 4.0, colour, on));
    };
    match fate {
        Fate::Waiting(_) => lamp(ui, STEEL, 0.0),
        Fate::Asking | Fate::Making => lamp(ui, AMBER, 1.0),
        Fate::Ended { went: Went::Well, .. } => {
            widgets::status_mark_named(ui, Mark::Ok, None);
        }
        Fate::Ended { went: Went::Partly, .. } => lamp(ui, AMBER, 1.0),
        Fate::Ended { went: Went::Badly, .. } | Fate::Failed(_) => {
            widgets::status_mark_named(ui, Mark::Error, None);
        }
        Fate::Ended { went: Went::Nothing, .. } | Fate::Left => {
            widgets::status_mark_named(ui, Mark::Absent, None);
        }
    }
}

/// The wizard's buttons: NEXT on the game's step; IMPORT and BACK on the choice's; STOP while it
/// imports (STOP AFTER THIS MAP with the maps in hand), saying that it goes on closed; PLAY (a
/// skirmish's setup where factions or maps were made) at the end; CLOSE on every step; why NEXT
/// or IMPORT cannot be pressed, in words beside them.
fn wizard_footer(ui: &mut Ui, footer: &Footer, asked: &mut Asked) {
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let why = footer.why.clone().unwrap_or_default();
        match footer.step {
            Step::Game => {
                if ui.add_enabled(footer.why.is_none(), ActionButton::primary("NEXT ›")).disabled_tip(why.as_str()).clicked() {
                    asked.actions.push(Action::SupcomWizardStep(Step::Choose));
                }
                if ui.add(ActionButton::secondary("CLOSE")).clicked() {
                    asked.close = true;
                }
            }
            Step::Choose => {
                let import = ui.add_enabled(footer.why.is_none(), ActionButton::primary("IMPORT")).hover_tip(t("Makes the parts ticked one after another; the import goes on with this closed.")).disabled_tip(why.as_str());
                if import.clicked() {
                    asked.actions.push(Action::SupcomWizardImport);
                }
                if ui.add(ActionButton::secondary("BACK")).clicked() {
                    asked.actions.push(if footer.kind == Kind::First { Action::SupcomWizardStep(Step::Game) } else { Action::SupcomTabs });
                }
                if ui.add(ActionButton::secondary("CLOSE")).clicked() {
                    asked.close = true;
                }
            }
            Step::Import => {
                let maps = footer.in_hand == Some(Tab::Maps);
                let label = if footer.stopping { n("STOPPING…") } else if maps { n("STOP AFTER THIS MAP") } else { n("STOP") };
                let tip = if maps { t("The maps made so far are kept.") } else { t("The part being made is finished first; those after it are not made.") };
                let held = maps && footer.installing;
                let stop = ui.add_enabled(!footer.stopping && !held, ActionButton::secondary(label)).hover_tip(tip);
                let stop = if held { stop.disabled_tip(t("Installing cannot be stopped.")) } else { stop };
                if stop.clicked() {
                    asked.actions.push(Action::SupcomWizardStop);
                }
                if ui.add(ActionButton::secondary("CLOSE")).hover_tip(t("The import goes on with this closed.")).clicked() {
                    asked.close = true;
                }
                ui.add(egui::Label::new(RichText::new(t("The import goes on with this closed: the launcher's footer shows how far it has got.")).color(STEEL)).wrap());
            }
            Step::Done => {
                let label = if footer.skirmish { n("PLAY A SKIRMISH") } else { n("PLAY") };
                let play = ui.add_enabled(footer.play.is_none(), ActionButton::primary(label));
                let play = match footer.play {
                    Some(why) => play.disabled_tip(why),
                    None if footer.skirmish => play.hover_tip(t("Starts the game at a new skirmish's setup: choose a Supreme Commander faction or map there.")),
                    None => play,
                };
                if play.clicked() {
                    asked.actions.push(if footer.skirmish { Action::PlayWith(vec!["--skip-intro".into(), "--open-skirmish".into()]) } else { Action::Play });
                    asked.close = true;
                }
                if ui.add(ActionButton::secondary("CLOSE")).clicked() {
                    asked.close = true;
                }
            }
        }
        // (Why NEXT or IMPORT cannot be pressed, in words: a greyed button takes no keys to say it.)
        if !why.is_empty() {
            ui.add(egui::Label::new(RichText::new(why).color(STEEL)).wrap());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use clap::Parser;
    use eframe::App as _;
    use egui::{Pos2, Rect};
    use ta_supcom::install::{Install, Store};

    use crate::supcom::Import;
    use crate::{supcom_factions as factions, supcom_menus as menus, supcom_music as music};

    type Texts = Vec<(String, Rect, Rect)>;

    fn launcher(name: &str) -> (PathBuf, egui::Context, LauncherApp) {
        let root = std::env::temp_dir().join(format!("ta-supcom-wizard-{name}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let ctx = egui::Context::default();
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", root.join("config.toml").to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        ta_ui::motion::set_reduced(&ctx, true);
        // (Never this computer's copy of the game: every look canned, none found until a test
        // says what.)
        app.supcom.look.canned = Some(Arc::new(|| (supcom::Found::default(), Default::default())));
        app.supcom.factions.look.canned = Some(Arc::new(|| (factions::Found::default(), Default::default())));
        app.supcom.music.look.canned = Some(Arc::new(|| (music::Found::default(), Default::default())));
        app.supcom.menus.look.canned = Some(Arc::new(|| (menus::Found::default(), Default::default())));
        STARTED.with(|started| started.borrow_mut().clear());
        (root, ctx, app)
    }

    /// TA's own install, as the launcher finds it (the maps are made of its features).
    fn fake_ta(app: &mut LauncherApp, root: &Path) {
        let game = root.join("fake_game");
        std::fs::create_dir_all(&game).unwrap();
        for file in ta_vfs::install::REQUIRED_FILES {
            std::fs::write(game.join(file), b"").unwrap();
        }
        app.config.game_dir = Some(game.clone());
        app.detection = crate::detect::detect(Some(&game), true);
    }

    fn gog() -> Install {
        Install { root: PathBuf::from("C:/Game"), store: Store::Gog }
    }

    /// Five of the game's maps and one of the player's own, as `states` has them.
    fn maps_found(states: [State; 6]) -> supcom::Found {
        let maps = (0..6).map(|i| {
            let (folder, own) = if i < 5 { (format!("C:/Game/maps/SCMP_{i:03}"), false) } else { ("D:/Own/maps/mine.v0001".to_string(), true) };
            let name = Path::new(&folder).file_name().unwrap().to_string_lossy().into_owned();
            let scenario = ta_supcom::map::Scenario { folder: name, name: format!("Map {i:03}"), size: (512, 512), armies: vec!["ARMY_1".into(), "ARMY_2".into()], ..Default::default() };
            supcom::Entry { folder: PathBuf::from(folder), players_own: own, scenario, state: states[i] }
        }).collect();
        supcom::Found { install: Some(gog()), copies: vec![gog()], maps, installed: states.iter().any(|s| *s != State::New), ..Default::default() }
    }

    fn factions_found(states: [State; 4]) -> factions::Found {
        let factions = ta_supcom::unit::Faction::PLAYABLE.into_iter().zip(states).map(|(faction, state)| factions::Entry { faction, buildings: 47, land: 20, air: 12, naval: 10, state }).collect();
        factions::Found { install: Some(gog()), copies: vec![gog()], factions, ..Default::default() }
    }

    fn music_found(state: State) -> music::Found {
        use ta_supcom::soundtrack::{Part, Track};
        let tracks = (1..=7).map(|i| Track { part: if i == 1 { Part::Menu } else { Part::Battle }, number: i, stem: format!("track_{i}"), bank: "Music.xwb", wave: 0, seconds: 150.0, bytes: 40_000_000 }).collect();
        music::Found { install: Some(gog()), copies: vec![gog()], tracks, state, ..Default::default() }
    }

    fn menus_found(state: State) -> menus::Found {
        let contents = ta_supcom::menus::Contents { skin: ta_supcom::menus::Skin { colours: Vec::new() }, pictures: vec!["space_1"], panel: true, sounds: vec!["UI_Back_MouseDown"], music_seconds: Some(80.0) };
        menus::Found { install: Some(gog()), copies: vec![gog()], contents: Some(contents), state, ..Default::default() }
    }

    /// Every tab's look finds the game, with these.
    fn with_the_game(app: &mut LauncherApp, maps: supcom::Found, factions: factions::Found, music: music::Found, menus: menus::Found) {
        let previews: supcom::Previews = maps.maps.iter().map(|e| (e.folder.clone(), egui::ColorImage::filled([8, 8], Color32::RED))).collect();
        app.supcom.look.canned = Some(Arc::new(move || (maps.clone(), previews.clone())));
        app.supcom.factions.look.canned = Some(Arc::new(move || (factions.clone(), Default::default())));
        app.supcom.music.look.canned = Some(Arc::new(move || (music.clone(), Default::default())));
        app.supcom.menus.look.canned = Some(Arc::new(move || (menus.clone(), Default::default())));
    }

    /// An import's mod installed as its import names it, made by the converter of `version`.
    fn installed(root: &Path, tab: Tab, version: &str) {
        let (id, name) = tab.mod_of();
        let folder = root.join("mods").join(id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("mod.toml"), format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"{version}\"\n")).unwrap();
    }

    thread_local! {
        /// The parts the wizard started, in order, each with what its tab chose then (the maps'
        /// folders, the factions' sides).
        static STARTED: RefCell<Vec<(Tab, Vec<String>)>> = const { RefCell::new(Vec::new()) };
        /// What lets the factions' import held by [`pretend_held`] end, installed.
        static HELD: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    }

    fn record(import: &Import, tab: Tab) {
        let chosen = match tab {
            Tab::Maps => import.chosen.iter().flatten().map(|f| f.file_name().unwrap().to_string_lossy().into_owned()).collect(),
            Tab::Factions => import.factions.chosen.iter().flatten().cloned().collect(),
            Tab::Music | Tab::Menus => Vec::new(),
        };
        STARTED.with(|started| started.borrow_mut().push((tab, chosen)));
    }

    /// Each part's import, ending at once, made.
    fn pretend_at_once(import: &mut Import, tab: Tab) {
        record(import, tab);
        match tab {
            Tab::Maps => import.pretend_import_ending(supcom::Outcome { imported: vec!["Map 001".into()], made: vec!["scmp_001".into()], ..Default::default() }),
            Tab::Factions => import.factions.pretend_imported(factions::Outcome { factions: vec!["UEF".into(), "AEON".into()], units: 178, ..Default::default() }),
            Tab::Music => import.music.pretend_imported(music::Outcome { tracks: 7, ..Default::default() }),
            Tab::Menus => import.menus.pretend_imported(menus::Outcome { made: true, ..Default::default() }),
        }
    }

    /// The factions' import held until [`HELD`] is set (then installed, whatever STOP said), the
    /// maps' until asked to stop; the others at once.
    fn pretend_held(import: &mut Import, tab: Tab) {
        match tab {
            Tab::Factions => {
                record(import, tab);
                import.factions.pretend_installing(HELD.with(|held| held.borrow().clone().expect("held")));
            }
            Tab::Maps => {
                record(import, tab);
                import.pretend_importing(&["scmp_001"]);
            }
            Tab::Music | Tab::Menus => pretend_at_once(import, tab),
        }
    }

    fn texts_of(shapes: &[egui::epaint::ClippedShape]) -> Texts {
        fn walk(shape: &egui::Shape, clip: Rect, into: &mut Texts) {
            match shape {
                egui::Shape::Text(text) => into.push((text.galley.text().to_string(), text.galley.rect.translate(text.pos.to_vec2()), clip)),
                egui::Shape::Vec(shapes) => for shape in shapes { walk(shape, clip, into); },
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for shape in shapes {
            walk(&shape.shape, shape.clip_rect, &mut texts);
        }
        texts
    }

    /// A frame of the launcher's update and the dialog, `size` points large, the actions asked for
    /// applied after it: the words drawn.
    fn frame(ctx: &egui::Context, app: &mut LauncherApp, size: egui::Vec2, events: Vec<egui::Event>) -> Texts {
        let mut window = eframe::Frame::_new_kittest();
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            app.logic(ui.ctx(), &mut window);
            app.supcom_dialog(ui.ctx());
            app.supcom_replace_dialog(ui.ctx());
        });
        for action in std::mem::take(&mut app.actions) {
            app.apply(action, &window);
        }
        output.textures_delta.clear();
        texts_of(&output.shapes)
    }

    /// A frame of the whole launcher (its page under the dialog): the words drawn.
    fn whole(ctx: &egui::Context, app: &mut LauncherApp, events: Vec<egui::Event>) -> Texts {
        let mut window = eframe::Frame::_new_kittest();
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1120.0, 720.0))), events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            app.logic(ui.ctx(), &mut window);
            app.ui(ui, &mut window);
        });
        output.textures_delta.clear();
        texts_of(&output.shapes)
    }

    /// A few frames (a dialog lays itself out over its first), the last one's words.
    fn settled(ctx: &egui::Context, app: &mut LauncherApp, size: egui::Vec2) -> Texts {
        for _ in 0..4 {
            frame(ctx, app, size, Vec::new());
        }
        frame(ctx, app, size, Vec::new())
    }

    fn has(texts: &Texts, words: &str) -> bool {
        texts.iter().any(|(text, _, _)| text == words)
    }

    fn words(texts: &Texts) -> Vec<&String> {
        texts.iter().map(|(text, _, _)| text).collect()
    }

    /// Where `words` are drawn (their middle).
    fn at(texts: &Texts, words: &str) -> Pos2 {
        texts.iter().find(|(text, _, _)| text == words).map(|(_, rect, _)| rect.center()).unwrap_or_else(|| panic!("{words}: {:?}", self::words(texts)))
    }

    /// A click on `words`, as the pointer gives it (pressed, then let go a frame later).
    fn click(ctx: &egui::Context, app: &mut LauncherApp, size: egui::Vec2, texts: &Texts, words: &str) {
        let at = at(texts, words);
        let press = |pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(ctx, app, size, vec![egui::Event::PointerMoved(at), press(true)]);
        frame(ctx, app, size, vec![press(false)]);
    }

    /// Frames until the wizard's import has ended.
    fn until_done(ctx: &egui::Context, app: &mut LauncherApp, size: egui::Vec2) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while (app.supcom.wizard.running() || app.supcom.any_importing()) && std::time::Instant::now() < until {
            frame(ctx, app, size, Vec::new());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!app.supcom.wizard.running() && !app.supcom.any_importing(), "the import ended");
    }

    fn started() -> Vec<(Tab, Vec<String>)> {
        STARTED.with(|started| started.borrow().clone())
    }

    /// With nothing imported, SUPREME COMMANDER… opens the wizard: the copy found (its store, CHANGE
    /// FOLDER…); NEXT, what to make, a card for each part, ticked, saying what it makes (the
    /// game's own maps, not the player's), what it takes and how long, and the total; a part
    /// unticked is left out; IMPORT makes the rest one after another, the factions first and the
    /// maps last, each tab's choice set before its import starts; what each came to, in its
    /// import's own words (MENU THEME set), turned on; CLOSE, and the dialog is put away.
    #[test]
    fn with_nothing_imported_the_dialog_opens_at_the_wizard_and_walks_its_steps() {
        let (root, ctx, mut app) = launcher("walk");
        fake_ta(&mut app, &root);
        with_the_game(&mut app, maps_found([State::New; 6]), factions_found([State::New; 4]), music_found(State::New), menus_found(State::New));
        app.supcom.pretend_start = Some(pretend_at_once);
        let size = vec2(1120.0, 720.0);
        app.apply(Action::SupcomOpen(Tab::Factions), &eframe::Frame::_new_kittest());
        assert!(app.supcom.open && app.supcom.wizard.shown && app.supcom.wizard.step == Step::Game && app.supcom.wizard.kind == Kind::First);
        assert_eq!(app.supcom.tab, Tab::Factions, "the tab CHOOSE IN DETAIL… shows");
        // The game's step: the copy found.
        let texts = settled(&ctx, &mut app, size);
        for words in ["1  FORGED ALLIANCE", "2  WHAT TO MAKE", "3  IMPORTING", "4  DONE", "FORGED ALLIANCE", "GOG", "CHANGE FOLDER…", "NEXT ›", "CLOSE"] {
            assert!(has(&texts, words), "{words}: {:?}", self::words(&texts));
        }
        assert!(!has(&texts, "IMPORT FACTIONS"), "not the tabs: {:?}", self::words(&texts));
        click(&ctx, &mut app, size, &texts, "NEXT ›");
        assert_eq!(app.supcom.wizard.step, Step::Choose);
        // What to make: the four, in the order they are made, each ticked.
        let texts = settled(&ctx, &mut app, size);
        let order: Vec<f32> = ["FACTIONS", "MUSIC", "MENUS", "MAPS"].iter().map(|words| at(&texts, words).y).collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "factions, music, menus, maps: {order:?}");
        assert!(has(&texts, "UEF, CYBRAN, AEON and SERAPHIM: 356 units, each faction a side to play in skirmish"), "{:?}", self::words(&texts));
        assert!(has(&texts, "The game's 5 skirmish maps (your own maps are on the MAPS tab)"), "the game's own, not the player's: {:?}", self::words(&texts));
        assert!(has(&texts, "The soundtrack's 7 tracks, 18 minutes, a lossless file each: SOUNDTRACK plays them"), "{:?}", self::words(&texts));
        let maps: Vec<supcom::Entry> = maps_found([State::New; 6]).maps.into_iter().filter(|e| !e.players_own).collect();
        let maps: Vec<&supcom::Entry> = maps.iter().collect();
        let estimate = tf("About {size} on the disk, {time} to make", &[("size", &supcom::size_text(supcom::size_estimate(&maps))), ("time", &supcom::time_text(supcom::time_estimate(&maps)))]);
        assert!(has(&texts, &estimate), "{estimate}: {:?}", self::words(&texts));
        assert!(texts.iter().any(|(text, _, _)| text.starts_with("In all: about ")), "{:?}", self::words(&texts));
        assert!(has(&texts, "CHOOSE IN DETAIL…") && has(&texts, "IMPORT") && has(&texts, "BACK"));
        assert!(ORDER.iter().all(|tab| app.supcom.wizard.is_ticked(*tab)), "all ticked at first");
        // The music unticked: left out of the total and of the import.
        let total = |texts: &Texts| texts.iter().find(|(text, _, _)| text.starts_with("In all: ")).map(|(text, _, _)| text.clone()).unwrap();
        let before = total(&texts);
        click(&ctx, &mut app, size, &texts, "MUSIC");
        assert!(!app.supcom.wizard.is_ticked(Tab::Music));
        let texts = settled(&ctx, &mut app, size);
        assert_ne!(total(&texts), before, "the total without the music");
        click(&ctx, &mut app, size, &texts, "IMPORT");
        assert_eq!(app.supcom.wizard.step, Step::Import);
        assert_eq!(app.supcom.wizard.parts.iter().map(|p| p.tab).collect::<Vec<_>>(), [Tab::Factions, Tab::Menus, Tab::Maps]);
        until_done(&ctx, &mut app, size);
        // Made one after another, each with its tab's choice: every faction, the game's own maps.
        let factions: Vec<String> = ["AEON", "CYBRAN", "SERAPHIM", "UEF"].map(String::from).to_vec();
        let maps: Vec<String> = (0..5).map(|i| format!("SCMP_{i:03}")).collect();
        assert_eq!(started(), [(Tab::Factions, factions), (Tab::Menus, Vec::new()), (Tab::Maps, maps)]);
        // What each came to, in its import's own words; the settings and mods it turned on.
        assert_eq!(app.supcom.wizard.step, Step::Done);
        let texts = settled(&ctx, &mut app, size);
        for words in [
            "Imported the UEF and AEON factions: 178 units in all. Their mod is on: choose one as a player's side when you set up a skirmish.",
            "Imported Forged Alliance's look for the menus. Its mod is on, and MENU THEME is now FORGED ALLIANCE: change that in Settings, Interface.",
            "Imported 1 map: it is in the map library, and in skirmish.",
            "PLAY A SKIRMISH",
        ] {
            assert!(has(&texts, words), "{words}: {:?}", self::words(&texts));
        }
        assert!(texts.iter().any(|(text, _, _)| text.starts_with("SUPREME COMMANDER… on the Mods and Maps pages makes or removes each part on its own")), "{:?}", self::words(&texts));
        assert!(!has(&texts, "MUSIC"), "the music left out: {:?}", self::words(&texts));
        assert_eq!(app.config.gameplay.menu_theme, ta_config::MenuTheme::ForgedAlliance);
        for id in [factions::MOD_ID, menus::MOD_ID, supcom::PACK_ID] {
            assert!(app.config.mods.enabled.iter().any(|on| on == id), "{id} on");
        }
        assert_eq!(app.config.audio.soundtrack, ta_config::Soundtrack::TotalAnnihilation, "the music not made, SOUNDTRACK as it was");
        assert!(!app.supcom.factions.unseen && !app.supcom.menus.unseen && !app.supcom.unseen, "the wizard told them, not the tabs' news");
        click(&ctx, &mut app, size, &texts, "CLOSE");
        assert!(!app.supcom.open && !app.supcom.wizard.shown, "put away once seen");
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Without the game the wizard's first step says what is needed (the stores, CHOOSE FOLDER…,
    /// LOOK AGAIN, which looks again for every part), and NEXT waits for it.
    #[test]
    fn without_the_game_the_wizard_says_what_is_needed() {
        let (root, ctx, mut app) = launcher("needed");
        let size = vec2(1120.0, 720.0);
        app.apply(Action::SupcomOpen(Tab::Maps), &eframe::Frame::_new_kittest());
        let texts = settled(&ctx, &mut app, size);
        for words in ["SUPREME COMMANDER: FORGED ALLIANCE NEEDED", "GET IT ON GOG", "GET IT ON STEAM", "CHOOSE FOLDER…", "LOOK AGAIN", "NEXT ›", "Waiting for Supreme Commander: Forged Alliance to be found."] {
            assert!(has(&texts, words), "{words}: {:?}", self::words(&texts));
        }
        click(&ctx, &mut app, size, &texts, "NEXT ›");
        assert_eq!(app.supcom.wizard.step, Step::Game, "NEXT waits for the game");
        assert!(Tab::ALL.iter().all(|tab| !app.supcom.scanning_at(*tab)));
        app.apply(Action::SupcomLookAgain, &eframe::Frame::_new_kittest());
        assert!(Tab::ALL.iter().all(|tab| app.supcom.scanning_at(*tab)), "every part's look made again");
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// STOP ends the import once the part in hand is made: the factions' import goes on to its end
    /// (it is not stopped), the parts after it are left, said so. With the maps in hand (the last
    /// part, the longest), the maps' import ends once the map in hand is made, those made kept.
    #[test]
    fn stop_ends_the_import_after_the_part_in_hand() {
        let (root, ctx, mut app) = launcher("stop");
        fake_ta(&mut app, &root);
        with_the_game(&mut app, maps_found([State::New; 6]), factions_found([State::New; 4]), music_found(State::New), menus_found(State::New));
        let done = Arc::new(AtomicBool::new(false));
        HELD.with(|held| *held.borrow_mut() = Some(done.clone()));
        app.supcom.pretend_start = Some(pretend_held);
        let size = vec2(1120.0, 720.0);
        app.apply(Action::SupcomOpen(Tab::Factions), &eframe::Frame::_new_kittest());
        settled(&ctx, &mut app, size);
        app.apply(Action::SupcomWizardStep(Step::Choose), &eframe::Frame::_new_kittest());
        app.apply(Action::SupcomWizardImport, &eframe::Frame::_new_kittest());
        let texts = settled(&ctx, &mut app, size);
        assert_eq!(app.supcom.wizard.in_hand(), Some(Tab::Factions));
        assert!(has(&texts, "STOP") && has(&texts, "The import goes on with this closed: the launcher's footer shows how far it has got."), "{:?}", self::words(&texts));
        click(&ctx, &mut app, size, &texts, "STOP");
        assert!(app.supcom.wizard.stopping && !app.supcom.factions.stopping(), "the factions are not stopped: they are finished first");
        let texts = settled(&ctx, &mut app, size);
        assert!(has(&texts, "STOPPING…"), "{:?}", self::words(&texts));
        assert!(app.supcom.factions.importing(), "still being made");
        done.store(true, Ordering::Relaxed);
        until_done(&ctx, &mut app, size);
        assert_eq!(started().iter().map(|(tab, _)| *tab).collect::<Vec<_>>(), [Tab::Factions], "nothing started after the part in hand");
        assert!(matches!(app.supcom.wizard.fate(Tab::Factions), Some(Fate::Ended { went: Went::Well, .. })));
        for tab in [Tab::Music, Tab::Menus, Tab::Maps] {
            assert_eq!(app.supcom.wizard.fate(tab), Some(&Fate::Left), "{tab:?}");
        }
        let texts = settled(&ctx, &mut app, size);
        assert_eq!(texts.iter().filter(|(text, _, _)| text == "Not made: the import was stopped before it.").count(), 3, "{:?}", self::words(&texts));
        click(&ctx, &mut app, size, &texts, "CLOSE");
        // The maps alone, in hand: STOP AFTER THIS MAP asks their import to stop after it.
        STARTED.with(|started| started.borrow_mut().clear());
        app.apply(Action::SupcomOpen(Tab::Maps), &eframe::Frame::_new_kittest());
        settled(&ctx, &mut app, size);
        app.apply(Action::SupcomWizardStep(Step::Choose), &eframe::Frame::_new_kittest());
        for tab in [Tab::Factions, Tab::Music, Tab::Menus] {
            app.supcom.wizard.tick(tab, false);
        }
        app.apply(Action::SupcomWizardImport, &eframe::Frame::_new_kittest());
        let texts = settled(&ctx, &mut app, size);
        assert_eq!(app.supcom.wizard.in_hand(), Some(Tab::Maps));
        click(&ctx, &mut app, size, &texts, "STOP AFTER THIS MAP");
        assert!(app.supcom.stopping(), "the maps' import stops after the map in hand");
        until_done(&ctx, &mut app, size);
        let texts = settled(&ctx, &mut app, size);
        assert!(has(&texts, "Stopped after 1 map: it is in the map library, and in skirmish. The maps not made yet stay chosen."), "{:?}", self::words(&texts));
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// UPDATE ALL makes again only what an older converter made, and as it was: the maps an older
    /// converter made (the player's own among them; not those made by this one, nor those never
    /// made), the factions the mod holds (not the others), the menus' look; not the music made by
    /// this TA Reborn. The Mods page says so in one bar; the dialog too, at the top of its tabs.
    #[test]
    fn update_all_makes_again_only_what_an_older_converter_made() {
        let (root, ctx, mut app) = launcher("update");
        fake_ta(&mut app, &root);
        installed(&root, Tab::Factions, "1");
        installed(&root, Tab::Music, &music::VERSION.to_string());
        installed(&root, Tab::Menus, "0");
        installed(&root, Tab::Maps, "0");
        app.refresh_mods();
        assert_eq!(app.supcom_outdated(), [Tab::Factions, Tab::Menus, Tab::Maps]);
        assert!(app.supcom_missing().is_empty());
        let maps = maps_found([State::Outdated, State::Imported, State::New, State::Outdated, State::Imported, State::Outdated]);
        with_the_game(&mut app, maps, factions_found([State::Outdated, State::New, State::Outdated, State::New]), music_found(State::Imported), menus_found(State::Outdated));
        app.supcom.pretend_start = Some(pretend_at_once);
        // The Mods page's bar.
        app.page = crate::app::Page::Mods;
        let mut texts = Vec::new();
        for _ in 0..4 {
            texts = whole(&ctx, &mut app, Vec::new());
        }
        assert!(has(&texts, "Supreme Commander imports were made by an older TA Reborn: the factions, the menus' look and the maps."), "{:?}", self::words(&texts));
        let update = at(&texts, "UPDATE ALL");
        let press = |pressed: bool| egui::Event::PointerButton { pos: update, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        whole(&ctx, &mut app, vec![egui::Event::PointerMoved(update), press(true)]);
        whole(&ctx, &mut app, vec![press(false)]);
        assert!(app.supcom.open && app.supcom.wizard.shown && app.supcom.wizard.kind == Kind::Update, "UPDATE ALL opens the wizard's import");
        assert_eq!(app.supcom.wizard.parts.iter().map(|p| p.tab).collect::<Vec<_>>(), [Tab::Factions, Tab::Menus, Tab::Maps]);
        let size = vec2(1120.0, 720.0);
        let texts = settled(&ctx, &mut app, size);
        assert!(has(&texts, "1  IMPORTING") && has(&texts, "2  DONE"), "{:?}", self::words(&texts));
        until_done(&ctx, &mut app, size);
        assert_eq!(started(), [
            (Tab::Factions, vec!["AEON".to_string(), "UEF".to_string()]),
            (Tab::Menus, Vec::new()),
            (Tab::Maps, vec!["SCMP_000".to_string(), "SCMP_003".to_string(), "mine.v0001".to_string()]),
        ]);
        assert_eq!(app.config.gameplay.menu_theme, ta_config::MenuTheme::ForgedAlliance, "the menus' setting follows as their import makes it");
        // The tabs say it too, with the way to it.
        app.supcom.close();
        app.supcom.open = true;
        app.supcom.tab = Tab::Music;
        let texts = settled(&ctx, &mut app, size);
        assert!(has(&texts, "Made by an older TA Reborn: the factions, the menus' look and the maps."), "{:?}", self::words(&texts));
        click(&ctx, &mut app, size, &texts, "UPDATE ALL");
        assert!(app.supcom.wizard.shown && app.supcom.wizard.kind == Kind::Update);
        until_done(&ctx, &mut app, size);
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// After a first import SUPREME COMMANDER… opens the tabs, as before, which offer what is not
    /// imported yet in one go: the wizard at what to make, with only those parts; BACK to the
    /// tabs.
    #[test]
    fn after_a_first_import_the_tabs_open_and_offer_the_rest_in_one_go() {
        let (root, ctx, mut app) = launcher("rest");
        fake_ta(&mut app, &root);
        installed(&root, Tab::Music, &music::VERSION.to_string());
        app.refresh_mods();
        with_the_game(&mut app, maps_found([State::New; 6]), factions_found([State::New; 4]), music_found(State::Imported), menus_found(State::New));
        let size = vec2(1120.0, 720.0);
        app.apply(Action::SupcomOpen(Tab::Maps), &eframe::Frame::_new_kittest());
        assert!(app.supcom.open && !app.supcom.wizard.shown && app.supcom.tab == Tab::Maps, "the tabs");
        let texts = settled(&ctx, &mut app, size);
        assert!(has(&texts, "IMPORT MAPS"), "{:?}", self::words(&texts));
        assert!(has(&texts, "Not imported yet: the factions, the menus' look and the maps."), "{:?}", self::words(&texts));
        click(&ctx, &mut app, size, &texts, "IMPORT IN ONE GO…");
        assert!(app.supcom.wizard.shown && app.supcom.wizard.kind == Kind::Rest && app.supcom.wizard.step == Step::Choose);
        assert_eq!(app.supcom.wizard.offered, [Tab::Factions, Tab::Menus, Tab::Maps]);
        let texts = settled(&ctx, &mut app, size);
        assert!(has(&texts, "1  WHAT TO MAKE") && !has(&texts, "MUSIC"), "{:?}", self::words(&texts));
        click(&ctx, &mut app, size, &texts, "BACK");
        assert!(app.supcom.open && !app.supcom.wizard.shown, "back at the tabs");
        let texts = settled(&ctx, &mut app, size);
        assert!(has(&texts, "IMPORT MAPS"), "{:?}", self::words(&texts));
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The Mods page offers the wizard once the game is found with nothing of it imported: IMPORT…
    /// opens it; NOT NOW puts the offer away, never made again (kept in the settings). Nothing
    /// is offered without the game.
    #[test]
    fn the_mods_page_offers_the_wizard_once_where_the_game_is_found() {
        const OFFER: &str = "Supreme Commander: Forged Alliance is installed on this computer: its factions, maps, soundtrack and look for the menus can be made into TA Reborn's from it.";
        let (root, ctx, mut app) = launcher("offer");
        app.page = crate::app::Page::Mods;
        let painted = |app: &mut LauncherApp| {
            let mut texts = Vec::new();
            for _ in 0..4 {
                texts = whole(&ctx, app, Vec::new());
            }
            texts
        };
        assert!(!has(&painted(&mut app), OFFER), "not without the game");
        app.supcom.offer.pretend(true);
        let texts = painted(&mut app);
        assert!(has(&texts, OFFER) && has(&texts, "IMPORT…") && has(&texts, "NOT NOW"), "{:?}", self::words(&texts));
        let press = |at: Pos2, pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let import = at(&texts, "IMPORT…");
        whole(&ctx, &mut app, vec![egui::Event::PointerMoved(import), press(import, true)]);
        whole(&ctx, &mut app, vec![press(import, false)]);
        assert!(app.supcom.open && app.supcom.wizard.shown && app.supcom.wizard.step == Step::Game, "IMPORT… opens the wizard");
        assert!(app.supcom.offer.done && supcom::load_offered(&root), "taken up: not offered again");
        app.supcom.close();
        assert!(!has(&painted(&mut app), OFFER));
        drop(app);
        // NOT NOW, in a launcher where it was not taken up yet.
        let _ = std::fs::remove_file(root.join("supcom.toml"));
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", root.join("config.toml").to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        app.page = crate::app::Page::Mods;
        app.supcom.offer.pretend(true);
        let texts = painted(&mut app);
        let later = at(&texts, "NOT NOW");
        whole(&ctx, &mut app, vec![egui::Event::PointerMoved(later), press(later, true)]);
        whole(&ctx, &mut app, vec![press(later, false)]);
        assert!(!app.supcom.open && app.supcom.offer.done && supcom::load_offered(&root));
        assert!(!has(&painted(&mut app), OFFER));
        drop(app);
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", root.join("config.toml").to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        assert!(app.supcom.offer.done, "the next launch remembers");
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A part whose mod's id another mod has is asked about before it is made; KEEP leaves it
    /// out, said so, and the next part goes on (the question not asked again).
    #[test]
    fn a_part_kept_over_another_mod_of_its_id_is_left_out_and_the_rest_go_on() {
        let (root, ctx, mut app) = launcher("keep");
        let theirs = root.join("mods").join(factions::MOD_ID);
        std::fs::create_dir_all(&theirs).unwrap();
        std::fs::write(theirs.join("mod.toml"), format!("id = \"{}\"\nname = \"Bob's factions\"\nversion = \"2\"\n", factions::MOD_ID)).unwrap();
        app.refresh_mods();
        with_the_game(&mut app, maps_found([State::New; 6]), factions_found([State::New; 4]), music_found(State::New), menus_found(State::New));
        app.supcom.pretend_start = Some(pretend_at_once);
        let size = vec2(1120.0, 720.0);
        app.apply(Action::SupcomOpen(Tab::Factions), &eframe::Frame::_new_kittest());
        assert!(app.supcom.wizard.shown, "another's mod of the id is not an import");
        settled(&ctx, &mut app, size);
        app.apply(Action::SupcomWizardStep(Step::Choose), &eframe::Frame::_new_kittest());
        app.supcom.wizard.tick(Tab::Maps, false);
        app.apply(Action::SupcomWizardImport, &eframe::Frame::_new_kittest());
        let texts = settled(&ctx, &mut app, size);
        assert_eq!(app.supcom.wizard.fate(Tab::Factions), Some(&Fate::Asking));
        assert!(has(&texts, "REPLACE AND IMPORT") && has(&texts, "Waiting for your answer: another mod of its id is installed."), "{:?}", self::words(&texts));
        click(&ctx, &mut app, size, &texts, "KEEP");
        until_done(&ctx, &mut app, size);
        assert_eq!(app.supcom.wizard.fate(Tab::Factions), Some(&Fate::Failed("Not made: the other mod of its id was kept.".into())));
        assert!(app.supcom.confirm_replace.is_none(), "not asked again");
        assert_eq!(started().iter().map(|(tab, _)| *tab).collect::<Vec<_>>(), [Tab::Music, Tab::Menus]);
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// In a small window (723 × 461 points, as 940 × 600 at 1.3 times) the wizard's buttons stay
    /// in sight on every step, and its steps above.
    #[test]
    fn the_wizards_buttons_stay_in_sight_in_a_small_window() {
        let (root, ctx, mut app) = launcher("small");
        fake_ta(&mut app, &root);
        with_the_game(&mut app, maps_found([State::New; 6]), factions_found([State::New; 4]), music_found(State::New), menus_found(State::New));
        let done = Arc::new(AtomicBool::new(false));
        HELD.with(|held| *held.borrow_mut() = Some(done.clone()));
        app.supcom.pretend_start = Some(pretend_held);
        let size = vec2(723.0, 461.0);
        let screen = Rect::from_min_size(Pos2::ZERO, size);
        app.apply(Action::SupcomOpen(Tab::Maps), &eframe::Frame::_new_kittest());
        let seen = |texts: &Texts, words: &str| {
            let (_, rect, clip) = texts.iter().find(|(text, _, _)| text == words).unwrap_or_else(|| panic!("{words}: {:?}", self::words(texts)));
            assert!(screen.contains_rect(*rect) && clip.contains_rect(*rect), "{words} in sight: {rect:?}, clipped to {clip:?}");
        };
        let texts = settled(&ctx, &mut app, size);
        for words in ["NEXT ›", "CLOSE", "1  FORGED ALLIANCE"] {
            seen(&texts, words);
        }
        app.apply(Action::SupcomWizardStep(Step::Choose), &eframe::Frame::_new_kittest());
        let texts = settled(&ctx, &mut app, size);
        for words in ["IMPORT", "BACK", "CLOSE", "2  WHAT TO MAKE"] {
            seen(&texts, words);
        }
        app.apply(Action::SupcomWizardImport, &eframe::Frame::_new_kittest());
        let texts = settled(&ctx, &mut app, size);
        for words in ["STOP", "CLOSE", "3  IMPORTING"] {
            seen(&texts, words);
        }
        app.apply(Action::SupcomWizardStop, &eframe::Frame::_new_kittest());
        done.store(true, Ordering::Relaxed);
        until_done(&ctx, &mut app, size);
        let texts = settled(&ctx, &mut app, size);
        for words in ["PLAY A SKIRMISH", "CLOSE", "4  DONE"] {
            seen(&texts, words);
        }
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }
}
