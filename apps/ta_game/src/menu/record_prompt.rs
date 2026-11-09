//! What is asked as a saved game or replay made with other mods or game data is opened, from the
//! saved games and replays page, Continue, or a file given to the game (Explorer, the launcher):
//! which mods it needs and how they stand now (`ta_ui::record_mods`), what that does to it, and
//! what would put it right: its mods turned on and the game started again, a missing one got in
//! the launcher; or open it anyway, or not.

use egui::RichText;
use ta_config::records::RecordMod;
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::record_mods::{self, Line, Tone};
use ta_ui::widgets::ActionButton;

use super::{Menu, MenuAction};
use crate::record_mods::Verdict;
use crate::replay::{Record, RecordKind};
use crate::setup::Assets;
use crate::theme::*;

/// A record whose battle would not be the one it recorded, and what is said of it.
pub struct RecordPrompt {
    pub path: std::path::PathBuf,
    pub record: Record,
    pub verdict: Verdict,
}

impl RecordPrompt {
    /// What it says of the mods: a line each (none for a record made before records named them).
    fn lines(&self) -> Vec<Line> {
        self.verdict.mods.as_ref().map(record_mods::lines).unwrap_or_default()
    }
}

/// Every mod installed, as a record names them (read from the settings on disk: the menu keeps
/// none of its own).
pub fn installed_now() -> Vec<RecordMod> {
    crate::record_mods::installed(&ta_config::Config::load())
}

impl Menu {
    /// A record read and about to be opened: opened at once (`Some`) when its battle can be
    /// played back as it was (or cannot be opened at all: the game says why), else asked about
    /// first (`None`, the prompt up).
    pub fn offer(&mut self, path: std::path::PathBuf, record: Record, assets: &Assets, installed: &[RecordMod]) -> Option<MenuAction> {
        let verdict = Verdict::of(record.version, record.metadata.as_ref(), record.data_hash, &record.setup, record.mods.as_deref(), assets, installed);
        // (A saved game of another simulation cannot carry on whatever its mods: refused as before.)
        let unplayable = record.kind == RecordKind::Save && !verdict.simulation;
        if verdict.opens() || unplayable || record.opened_anyway {
            return Some(MenuAction::Play(record));
        }
        log::info!("{} made with other game data or mods: asking ({:?})", path.display(), verdict.mods.as_ref().map(record_mods::lines));
        self.record_prompt = Some(RecordPrompt { path, record, verdict });
        None
    }

    /// The prompt, while one is up: what the player chose.
    pub(super) fn record_prompt_dialog(&mut self, ctx: &egui::Context, assets: &Assets) -> MenuAction {
        let Some(prompt) = &self.record_prompt else { return MenuAction::None };
        let save = prompt.record.kind == RecordKind::Save;
        let lines = prompt.lines();
        let mods_differ = prompt.verdict.mods.as_ref().is_some_and(|check| !check.matches());
        let missing: Vec<RecordMod> = prompt.verdict.mods.as_ref().map(|c| c.missing().cloned().collect()).unwrap_or_default();
        let switch = prompt.verdict.mods.as_ref().filter(|c| c.switchable()).map(|c| c.load_order());
        let title = if mods_differ { t("MADE WITH OTHER MODS") } else { t("MADE WITH OTHER GAME DATA") };
        let accent = if lines.iter().any(|l| l.tone == Tone::Bad) { RED } else { AMBER };
        let width = (ctx.content_rect().width() - 80.0).clamp(1.0, 620.0);
        let mut choice = None;
        let response = panel::modal(ctx, "record_mods_prompt", Panel::new(title).width(width).exit(Exit::Close).accent(accent), true, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add(egui::Label::new(RichText::new(&prompt.record.title).font(strong(15.0)).color(TEXT_BRIGHT)).wrap());
            if !lines.is_empty() {
                record_mods::show(ui, &lines, 8);
            }
            ui.add_space(2.0);
            let why = match (&prompt.verdict.mods, save) {
                // (Made before records named their mods: what is on now, to go by.)
                (None, _) => {
                    let on = assets.mods.known();
                    let now = if on.is_empty() { t("No mods are on now.").to_string() } else { tf("Mods on now: {mods}", &[("mods", &record_mods::names(&on))]) };
                    format!("{} {now}", t("It was made before saved games and replays named the mods they were made with."))
                }
                (Some(check), _) if check.matches() => t("Its mods are as they were, but other game data is not: another copy of Total Annihilation, or of TA Reborn's own maps.").to_string(),
                (Some(_), true) => t("A saved game is played over from its start to where it was saved. With other mods its orders can build other units, or none: it may not come out as the battle you saved.").to_string(),
                (Some(_), false) => t("A replay is its battle played over from the orders given in it. With other mods they can lead elsewhere: it may not show the battle it recorded.").to_string(),
            };
            ui.add(egui::Label::new(RichText::new(why).color(TEXT)).wrap());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                if let Some(order) = &switch {
                    let hover = if prompt.verdict.mods.as_ref().is_none_or(|c| c.needed.is_empty()) {
                        t("TA Reborn starts again with no mods on (map packs stay on), and opens this battle")
                    } else {
                        t("TA Reborn starts again with its mods on, in its order, and the others off (map packs stay on), and opens this battle")
                    };
                    if ui.add(ActionButton::primary("USE ITS MODS AND RESTART")).on_hover_text(hover).clicked() {
                        choice = Some(Choice::Switch(order.clone()));
                    }
                }
                if let Some(first) = missing.first() {
                    let button = if switch.is_none() { ActionButton::primary("GET IT IN THE LAUNCHER") } else { ActionButton::secondary("GET IT IN THE LAUNCHER") };
                    if ui.add(button).on_hover_text(tf("The launcher's Mods page opens on {mod}: get it there, then open this battle again", &[("mod", &first.label())])).clicked() {
                        choice = Some(Choice::Get(first.id.clone()));
                    }
                }
                let anyway = if save { n("LOAD ANYWAY") } else { n("WATCH ANYWAY") };
                if ui.add(ActionButton::secondary(anyway)).on_hover_text(t("Its orders are played over with the mods on now")).clicked() {
                    choice = Some(Choice::Anyway);
                }
                if ui.add(ActionButton::secondary("CANCEL")).clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        });
        if response.exit && choice.is_none() {
            choice = Some(Choice::Cancel);
        }
        let Some(choice) = choice else { return MenuAction::None };
        let Some(prompt) = self.record_prompt.take() else { return MenuAction::None };
        match choice {
            Choice::Switch(order) => MenuAction::RestartWithMods { path: prompt.path, replay: prompt.record.kind == RecordKind::Replay, order },
            Choice::Get(id) => MenuAction::GetModInLauncher(id),
            Choice::Anyway => {
                let mut record = prompt.record;
                record.opened_anyway = true;
                log::info!("{}: opened anyway, with today's mods and data", prompt.path.display());
                MenuAction::Play(record)
            }
            Choice::Cancel => MenuAction::None,
        }
    }
}

enum Choice {
    Switch(Vec<String>),
    Get(String),
    Anyway,
    Cancel,
}
