//! The Profile page: the active commander's service record (its head, with their picture,
//! motto, rank and showcase; its tabs: the overview, the achievements, the campaigns, the
//! battles) and the commanders of this device, to switch between, edit, delete or add to.
//! What it shows is read off the interface's thread when the page is shown.

use egui::{Align, Layout, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use ta_config::profiles::{Career, Profile, Unreadable};
use ta_ui::i18n::{grouped, n, t, tf};
use ta_ui::panel::{Exit, Panel};
use ta_ui::tooltip::TooltipExt as _;
use ta_ui::profiles::{CareerAction, CareerData, CareerTab, HeroSize};

use super::LauncherApp;
use super::profiles::{CareerFile, Details, DetailsFailed, ProfileTab};
use crate::theme::*;
use crate::widgets::{self, ActionButton};

/// What a commander's row in the Commanders tab asks for.
enum RowAction { Switch, Edit, Delete }

/// What a row of a profile that cannot be read asks for.
#[derive(Clone, Copy)]
enum UnreadableAction { Open, Remove }

/// A commander's rank as their row shows it: their career's, or that it is not read yet, or that
/// it cannot be.
#[derive(Clone, Copy)]
enum Rank<'a> { Of(&'a Career), Unknown, Unreadable }

impl LauncherApp {
    /// Why profiles cannot be switched, edited or deleted now, if they cannot.
    pub(super) fn profile_blocked(&self) -> Option<&'static str> {
        if self.game_running() {
            Some(t("Close the game first: commanders are switched between games."))
        } else if self.installer.busy() {
            Some(t("Wait for the mod being installed"))
        } else if self.maps.busy() {
            Some(t("Wait for the maps being imported"))
        } else if self.supcom.importing() {
            // (What a Supreme Commander import makes is turned on for the commander who asked for
            // it, once made.)
            Some(t("Wait for the Supreme Commander maps being made, then try again."))
        } else if self.supcom.factions.importing() {
            Some(t("Wait for the Supreme Commander factions being made, then try again."))
        } else if self.supcom.music.importing() {
            Some(t("Wait for the Supreme Commander music being made, then try again."))
        } else if self.skins.busy() {
            Some(t("Wait for the current skin operation to finish, then try again."))
        } else if self.skins.editor.as_ref().is_some_and(|e| e.changed()) {
            Some(t("Save or close the skin you are editing first"))
        } else { None }
    }

    pub(super) fn profile_page(&mut self, ui: &mut Ui) {
        let Some(profile) = self.profiles.active.clone() else {
            self.no_profile(ui);
            return;
        };
        self.load_profile_details(ui.ctx());
        self.poll_profile_details();
        // (In a page that scrolls whole, a short window or a large interface, the tabs are laid out
        // at their own height, without scrolls of their own; where it is shortest, the head is at
        // its most compact.)
        let natural = self.profiles.natural;
        let short = natural || ui.available_height() < 600.0;
        let roomy = !natural && ui.available_height() >= 640.0 && ui.available_width() >= 720.0;
        let compact = ui.available_height() < 500.0;
        // The heading, and what can be done with the profile.
        let blocked = self.profile_blocked();
        crate::pages::library_header(ui, n("PROFILE"), n("Your service record, your achievements and the commanders who play on this device."), short,
            &["EDIT PROFILE", "OPEN PROFILE FOLDER"], |ui, height| {
            let edit = ui.add_enabled(blocked.is_none(), ActionButton::primary("EDIT PROFILE").height(height));
            let edit = match blocked { Some(why) => edit.disabled_tip(why), None => edit.hover_tip(t("Change your name, motto, allegiance or picture")) };
            if edit.clicked() { self.profiles.edit(true); }
            if ui.add(ActionButton::secondary("OPEN PROFILE FOLDER").height(height)).hover_tip(t("Where this profile's settings, saved games, replays and pictures are kept")).clicked()
                && let Some(dir) = self.profiles.store.as_ref().and_then(|s| s.data_dir(&profile.id).ok()) {
                self.push(super::Action::OpenFolder(dir));
            }
        });
        // The head: the commander, or why they are not there yet.
        let mut open = None;
        let mut retry = false;
        if let CareerFile::Damaged(error) = self.profiles.career_file.clone() {
            self.career_problem(ui, &error);
        } else {
            match (&self.profiles.details, &self.profiles.details_error) {
                (Some(details), _) => {
                    let size = if compact { HeroSize::Compact } else if roomy { HeroSize::Large } else { HeroSize::Normal };
                    widgets::card(ui, |ui| open = ta_ui::profiles::hero(ui, &profile, &details.record, size));
                }
                (None, Some(error)) => {
                    widgets::card(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(tf("Could not load profile: {error}", &[("error", &ta_ui::profiles::first_line(error))])).color(RED)).wrap());
                        retry = ui.add(ActionButton::secondary("TRY AGAIN")).clicked();
                    });
                }
                (None, None) => {
                    widgets::card(ui, |ui| {
                        ui.label(RichText::new(t("Loading service record…")).color(STEEL));
                        ta_ui::panel::busy_bar(ui);
                    });
                }
            }
        }
        if retry { self.profiles.details_error = None; }
        if let Some(id) = open {
            self.profiles.view.achievement(id);
            self.profiles.tab = ProfileTab::Achievements;
        }
        let mut tab = self.profiles.tab;
        widgets::segmented_fill(ui, &mut tab, &[(ProfileTab::Overview, n("OVERVIEW")), (ProfileTab::Achievements, n("ACHIEVEMENTS")),
            (ProfileTab::Campaign, n("CAMPAIGN")), (ProfileTab::History, n("MATCH HISTORY")), (ProfileTab::Commanders, n("COMMANDERS"))]);
        self.profiles.tab = tab;
        // (The commanders can be switched to, or deleted, whatever became of this one's record.)
        if tab == ProfileTab::Commanders {
            self.commanders(ui);
            return;
        }
        self.profiles.view.tab = match tab {
            ProfileTab::Achievements => CareerTab::Achievements,
            ProfileTab::Campaign => CareerTab::Campaign,
            ProfileTab::History => CareerTab::History,
            _ => CareerTab::Overview,
        };
        self.profiles.view.natural = natural;
        let Some(details) = &self.profiles.details else { return };
        if let Some(CareerAction::Showcase(list)) = self.profiles.view.body(ui, &profile, &details.record) { self.set_showcase(&list); }
    }

    /// The commander's service record cannot be read: what that means, the file's own words, and
    /// the ways out (a fresh career, the damaged one kept; the folder, to repair it; reading it
    /// again).
    fn career_problem(&mut self, ui: &mut Ui, error: &str) {
        let blocked = self.profile_blocked();
        let folder = self.profiles.store.as_ref().zip(self.profiles.active.as_ref()).and_then(|(s, p)| s.data_dir(&p.id).ok());
        let (mut fresh, mut retry, mut open) = (false, false, false);
        widgets::card_accent(ui, Some(RED), |ui| {
            ui.horizontal(|ui| {
                widgets::status_mark(ui, widgets::Mark::Error);
                ui.label(RichText::new(t("YOUR SERVICE RECORD COULD NOT BE READ")).font(strong(13.0)).color(RED).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
            });
            ui.add(egui::Label::new(RichText::new(t("Your career file (career.toml) is damaged: your rank, XP, achievements and match history cannot be shown, and battles cannot start, as they could not be recorded. Start a fresh career (the damaged file is kept in your profile folder), or repair the file or put back a copy of it, and try again."))
                .color(TEXT)).wrap());
            ui.add(egui::Label::new(RichText::new(ta_ui::profiles::first_line(error)).font(mono(11.5)).color(STEEL)).wrap())
                .hover_tip(error);
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let start = ui.add_enabled(blocked.is_none(), ActionButton::primary("START A FRESH CAREER…"));
                fresh = match blocked { Some(why) => start.disabled_tip(why), None => start.hover_tip(t("Asked first: the damaged file is kept")) }.clicked();
                open = folder.is_some() && ui.add(ActionButton::secondary("OPEN PROFILE FOLDER")).clicked();
                retry = ui.add(ActionButton::secondary("TRY AGAIN")).hover_tip(t("Read your service record again, if you have repaired it")).clicked();
            });
        });
        if fresh { self.profiles.confirm_fresh = true; }
        if open && let Some(dir) = folder { self.push(super::Action::OpenFolder(dir)); }
        if retry { self.profiles.reread_career(); }
    }

    /// No profile chosen: an invitation to make one, and what a commander's record holds (the
    /// ranks to rise through, the achievements to earn, a place of one's own on the device).
    fn no_profile(&mut self, ui: &mut Ui) {
        widgets::card(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(t("Create your commander profile")).font(strong(20.0)).color(TEXT_BRIGHT)).wrap());
            ui.add(egui::Label::new(RichText::new(t("Your profile keeps your settings, saves and career separate from other players on this device."))
                .color(STEEL)).wrap());
            if let Some(error) = &self.profiles.error {
                ui.add(egui::Label::new(RichText::new(tf("Could not load profile: {error}", &[("error", &ta_ui::profiles::first_line(error))])).color(AMBER)).wrap());
            }
            ui.add_space(4.0);
            let create = ui.add_enabled(self.profile_blocked().is_none(), ActionButton::primary("CREATE PROFILE").height(36.0));
            let create = match self.profile_blocked() { Some(why) => create.disabled_tip(why), None => create };
            if create.clicked() {
                self.profiles.edit(false);
            }
        });
        ui.add_space(8.0);
        // (Profiles of this device that cannot be read: what can be done with them.)
        if !self.profiles.unreadable.is_empty() {
            let picked = unreadable_profiles(ui, &self.profiles.unreadable, self.profile_blocked());
            self.unreadable_picked(picked);
            ui.add_space(8.0);
        }
        let ranks = ta_config::profiles::RANKS.len();
        let achievements = ta_config::achievements::ALL.len();
        let columns = if ui.available_width() >= 760.0 { 3 } else { 1 };
        let tiles: [(&str, String, u8); 3] = [
            (n("RISE THROUGH THE RANKS"), tf("Every battle earns experience: {n} ranks from Recruit to Commander, each with its insignia.", &[("n", &ranks)]), 0),
            (n("EARN ACHIEVEMENTS"), tf("{n} feats of the campaigns, skirmishes, multiplayer and battle, in bronze, silver, gold and platinum.", &[("n", &achievements)]), 1),
            (n("A PLACE OF YOUR OWN"), t("Each commander keeps their own settings, saved games, replays and photos, and their own match history.").to_string(), 2),
        ];
        // (Side by side where there is room, each as wide as the others; else one under another.)
        let tile = |ui: &mut Ui, title: &str, text: &str, picture: u8| {
            widgets::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(if columns > 1 { 170.0 } else { 0.0 });
                // Its pictures: insignia from the first rank up, four badges, or the two sides'
                // commanders' frames.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    match picture {
                        0 => {
                            for grade in [0, ranks / 4, ranks / 2, ranks * 3 / 4, ranks - 1] {
                                ta_ui::badges::insignia(ui, grade, 40.0);
                            }
                        }
                        1 => {
                            for a in ta_config::achievements::ALL.iter().step_by((achievements / 4).max(1)).take(4) {
                                ta_ui::badges::badge(ui, a, 40.0, ta_ui::badges::Look::Earned);
                            }
                        }
                        _ => {
                            for side in [ta_config::profiles::Allegiance::Arm, ta_config::profiles::Allegiance::Core] {
                                let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(ui.cursor().min, vec2(40.0, 40.0))));
                                ta_ui::profiles::avatar_picture(&mut child, None, Some(side), 40.0);
                                ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                            }
                        }
                    }
                });
                ui.add_space(6.0);
                ui.label(RichText::new(t(title)).font(strong(13.0)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
                ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap());
            });
        };
        if columns > 1 {
            ui.columns(columns, |cols| {
                for (col, (title, text, picture)) in cols.iter_mut().zip(tiles) {
                    tile(col, title, &text, picture);
                }
            });
        } else {
            for (title, text, picture) in tiles {
                tile(ui, title, &text, picture);
                ui.add_space(8.0);
            }
        }
    }

    /// Reads the record and the careers of this device's commanders on a worker, once.
    fn load_profile_details(&mut self, ctx: &egui::Context) {
        if self.profiles.details.is_some() || self.profiles.details_error.is_some() || self.profiles.details_pending.is_some() { return; }
        let Some(store) = self.profiles.store.clone() else {
            self.profiles.details_error = Some(t("Choose a profile first.").to_string());
            return;
        };
        let Some(id) = self.profiles.active.as_ref().map(|p| p.id.clone()) else { return };
        let others: Vec<String> = self.profiles.list.iter().map(|p| p.id.clone()).collect();
        let install = self.detection.install().map(|i| i.root.clone());
        let enabled = self.config.mods.enabled.clone();
        let available: Vec<_> = self.mods.mods.iter().map(|m| m.info.clone()).collect();
        let (send, receive) = std::sync::mpsc::channel();
        let context = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<Details, DetailsFailed> {
                // (The career first, read once: it is the commander's too, and one that cannot be
                // read is told apart from their other files.)
                let (career, history) = store.career_with_history(&id).map_err(|e| DetailsFailed { career: true, error: e.to_string() })?;
                let mut record = CareerData::around(&store, &id, career, history).map_err(|e| DetailsFailed { career: false, error: e.to_string() })?;
                if let Some(install) = install {
                    let game = ta_vfs::mods::mount_enabled(Some(&install), ta_vfs::builtin_dir().as_deref(), &enabled, &available);
                    let vfs = game.vfs;
                    let mounted = game.install.map_err(|e| e.to_string()).and_then(|()| game.mods.map(|_| ()).map_err(|e| e.to_string()));
                    match mounted {
                        Ok(()) => record.campaigns = ta_data::mission::campaigns(&vfs).into_iter().map(|c|
                            ta_ui::profiles::CampaignSummary { id: c.id, title: c.title, missions: c.missions.into_iter().map(|m| m.name).collect() }).collect(),
                        Err(error) => record.warning = Some(error),
                    }
                }
                // (The other commanders' ranks; the active one's career is the record's.)
                let mut careers = std::collections::BTreeMap::new();
                let mut unreadable = std::collections::BTreeSet::new();
                for other in others.iter().filter(|other| **other != id) {
                    match store.career(other) {
                        Ok(career) => { careers.insert(other.clone(), career); }
                        Err(_) => { unreadable.insert(other.clone()); }
                    }
                }
                careers.insert(id.clone(), record.career.clone());
                Ok(Details { record, careers, unreadable })
            })();
            let _ = send.send(result);
            context.request_repaint();
        });
        self.profiles.details_pending = Some(receive);
    }

    fn poll_profile_details(&mut self) {
        let Some(receiver) = &self.profiles.details_pending else { return };
        match receiver.try_recv() {
            Ok(Ok(details)) => {
                self.profiles.career_from_record(Ok(&details.record.career));
                self.profiles.details = Some(details);
                self.profiles.details_pending = None;
            }
            Ok(Err(failed)) => {
                if failed.career { self.profiles.career_from_record(Err(&failed.error)); }
                self.profiles.details_error = Some(failed.error);
                self.profiles.details_pending = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => { self.profiles.details_error = Some(t("Could not load profile.").to_string()); self.profiles.details_pending = None; }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// The achievements the active profile shows, as they are now to be.
    fn set_showcase(&mut self, list: &[String]) {
        let (Some(store), Some(id)) = (self.profiles.store.clone(), self.profiles.active.as_ref().map(|p| p.id.clone())) else { return };
        match store.set_showcase(&id, list) {
            Ok(profile) => self.profiles.active = Some(profile),
            Err(error) => self.notify(tf("Could not update profile: {error}", &[("error", &ta_ui::profiles::error_text(&error))]), RED),
        }
    }

    /// The commanders of this device, a row each, and a new one.
    fn commanders(&mut self, ui: &mut Ui) {
        let blocked = self.profile_blocked();
        ui.horizontal(|ui| {
            ui.label(widgets::caption("COMMANDERS ON THIS DEVICE"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let new = ui.add_enabled(blocked.is_none(), ActionButton::secondary("NEW PROFILE").height(30.0));
                let new = match blocked { Some(why) => new.disabled_tip(why), None => new.hover_tip(t("Someone else playing here keeps their own settings, saves and career")) };
                if new.clicked() { self.profiles.edit(false); }
            });
        });
        if let Some(why) = blocked { ui.add(egui::Label::new(RichText::new(why).color(STEEL).size(12.0)).wrap()); }
        if let Some(error) = &self.profiles.list_error {
            ui.add(egui::Label::new(RichText::new(tf("The profiles' folder could not be read: {error}", &[("error", &ta_ui::profiles::first_line(error))])).color(AMBER).size(12.0)).wrap());
        }
        let active = self.profiles.active.as_ref().map(|p| p.id.clone());
        let list = self.profiles.list.clone();
        // (The last readable commander stays: another, readable, takes over first.)
        let only = match (list.len() <= 1, self.profiles.unreadable.is_empty()) {
            (false, _) => None,
            (true, true) => Some(t("The only profile cannot be deleted: create another first.")),
            (true, false) => Some(t("Create another profile first: the other profiles on this device cannot be read, so none can take over.")),
        };
        let mut chosen = None;
        let mut rows = |ui: &mut Ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for entry in &list {
                let is_active = active.as_deref() == Some(entry.id.as_str());
                let details = self.profiles.details.as_ref();
                let career = details.and_then(|d| d.careers.get(&entry.id)).cloned()
                    .or_else(|| (is_active && self.profiles.career_file == CareerFile::Read).then(|| self.profiles.career.clone()));
                let unreadable = details.is_some_and(|d| d.unreadable.contains(&entry.id))
                    || (is_active && matches!(self.profiles.career_file, CareerFile::Damaged(_)));
                let texture = self.profiles.store.as_ref().filter(|_| entry.has_avatar)
                    .and_then(|s| s.avatar_path(&entry.id).ok()).and_then(|path| ta_ui::portrait::texture_at(ui.ctx(), &path));
                let rank = if unreadable { Rank::Unreadable } else { career.as_ref().map_or(Rank::Unknown, Rank::Of) };
                if let Some(action) = commander_row(ui, entry, rank, texture.as_ref(), is_active, blocked, only) {
                    chosen = Some((entry.id.clone(), action));
                }
            }
        };
        // (Those that cannot be read after them; in a page that scrolls whole, all run on in it.)
        let unreadable = &self.profiles.unreadable;
        let mut picked = None;
        let mut all = |ui: &mut Ui| {
            rows(ui);
            if !unreadable.is_empty() {
                ui.add_space(6.0);
                picked = unreadable_profiles(ui, unreadable, blocked);
            }
        };
        if self.profiles.natural {
            all(ui);
        } else {
            let area = egui::ScrollArea::vertical().id_salt("profile_commanders").max_height(ui.available_height().max(96.0)).auto_shrink([false, false]);
            ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Plate, all);
        }
        match chosen {
            Some((id, RowAction::Switch)) => { self.switch_profile(&id); }
            Some((_, RowAction::Edit)) => self.profiles.edit(true),
            Some((id, RowAction::Delete)) => self.profiles.confirm_delete = Some(id),
            None => {}
        }
        self.unreadable_picked(picked);
    }

    /// What was asked of a profile that cannot be read: its folder opened, or it removed (asked
    /// first).
    fn unreadable_picked(&mut self, picked: Option<(String, std::path::PathBuf, UnreadableAction)>) {
        match picked {
            Some((_, folder, UnreadableAction::Open)) => self.push(super::Action::OpenFolder(folder)),
            Some((id, _, UnreadableAction::Remove)) => self.profiles.confirm_delete = Some(id),
            None => {}
        }
    }

    /// Asks before a profile goes to the Recycle Bin.
    pub(super) fn delete_profile_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.profiles.confirm_delete.clone() else { return };
        // (A commander, or a profile that cannot be read: its folder is all there is of it.)
        let (name, title, folder) = if let Some(entry) = self.profiles.list.iter().find(|p| p.id == id) {
            (entry.name.clone(), tf("DELETE {name}?", &[("name", &ta_ui::i18n::upper(&entry.name))]), self.profiles.store.as_ref().and_then(|s| s.data_dir(&entry.id).ok()))
        } else if let Some(entry) = self.profiles.unreadable.iter().find(|p| p.id == id) {
            (t("The unreadable profile").to_string(), t("DELETE THE UNREADABLE PROFILE?").to_string(), Some(entry.folder.clone()))
        } else {
            self.profiles.confirm_delete = None;
            return;
        };
        let width = (ctx.content_rect().width() - 80.0).clamp(1.0, 520.0);
        let max_height = (ctx.content_rect().height() - 24.0).max(1.0);
        let blocked = self.profile_blocked();
        let mut delete = false;
        let response = ta_ui::panel::modal(ctx, "launcher_profile_delete", Panel::new(&title).exit(Exit::Close).width(width).max_height(max_height).accent(RED), true, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add(egui::Label::new(RichText::new(t("Their settings, saved games, replays, career and achievements go to the Recycle Bin: put them back from there if you change your mind. Their pictures stay in your Pictures folder."))
                .color(TEXT)).wrap());
            if let Some(folder) = &folder { ui.add(egui::Label::new(RichText::new(folder.display().to_string()).font(mono(12.0)).color(STEEL)).wrap()); }
            if let Some(why) = blocked { ui.add(egui::Label::new(RichText::new(why).color(AMBER).size(12.0)).wrap()); }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                delete = ui.add_enabled(blocked.is_none(), ActionButton::danger("DELETE")).clicked();
                if ui.add(ActionButton::secondary("KEEP")).clicked() { self.profiles.confirm_delete = None; }
            });
        });
        if delete {
            self.profiles.confirm_delete = None;
            self.delete_profile(&id, &name);
        } else if response.exit {
            self.profiles.confirm_delete = None;
        }
    }

    /// Moves a profile to the Recycle Bin (one that cannot be read too); the active one hands over
    /// to another commander first. (The last one stays: the launcher's settings are a commander's.)
    pub(super) fn delete_profile(&mut self, id: &str, name: &str) {
        use ta_ui::recycle::Outcome;
        if let Some(why) = self.profile_blocked() { self.notify(why.to_string(), AMBER); return; }
        let Some(store) = self.profiles.store.clone() else { return };
        if self.profiles.active.as_ref().is_some_and(|p| p.id == id) {
            let Some(next) = self.profiles.list.iter().find(|p| p.id != id).map(|p| p.id.clone()) else { return };
            if !self.switch_profile(&next) { return; }
        }
        let remove = |dir: std::path::PathBuf| if cfg!(test) {
            // (Tests leave this computer's Recycle Bin alone.)
            std::fs::remove_dir_all(&dir).map(|()| Outcome::Deleted).map_err(|e| e.to_string())
        } else { ta_ui::recycle::recycle(&dir) };
        match store.release(id).map_err(|e| ta_ui::profiles::error_text(&e)).and_then(remove) {
            Ok(Outcome::Kept) => self.notify(tf("{name} is left where it is", &[("name", &name)]), AMBER),
            Ok(outcome) => {
                log::info!("removed profile {id} ({outcome:?})");
                let text = match outcome {
                    Outcome::Recycled => tf("{name} is removed (it is in the Recycle Bin)", &[("name", &name)]),
                    _ => tf("{name} is deleted (it could not go to the Recycle Bin)", &[("name", &name)]),
                };
                self.notify(text, AMBER);
            }
            Err(error) => {
                log::warn!("removing profile {id}: {error}");
                self.notify(tf("Could not remove {name}: {error}", &[("name", &name), ("error", &error)]), RED);
            }
        }
        self.profiles.refresh();
    }
}

/// One commander of this device: their picture, name, motto and rank; their insignia, and
/// switching to them (or, the active one, editing), and deleting them.
fn commander_row(ui: &mut Ui, entry: &Profile, career: Rank, texture: Option<&egui::TextureHandle>, active: bool,
    blocked: Option<&'static str>, only: Option<&'static str>) -> Option<RowAction> {
    let height = 68.0;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let (rank, rank_color) = match career {
        Rank::Of(career) => (ta_ui::profiles::rank_text(career), AMBER),
        Rank::Unknown => (String::new(), AMBER),
        Rank::Unreadable => (t("Service record unreadable").to_string(), RED),
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, if rank.is_empty() { entry.name.clone() } else { format!("{}, {rank}", entry.name) }));
    if !ui.is_rect_visible(rect) { return None; }
    ui.painter().rect_filled(rect, 4.0, if active { AMBER_DEEP } else { BG_DEEP });
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, if active { AMBER_DIM } else { LINE }), StrokeKind::Inside);
    // The buttons and the insignia, from the right; what is left is the commander's.
    let mut action = None;
    let mut right = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(12.0, 0.0))).layout(Layout::right_to_left(Align::Center)));
    right.spacing_mut().item_spacing.x = 8.0;
    let delete = right.add_enabled(blocked.is_none() && only.is_none(), ActionButton::secondary("DELETE").height(30.0));
    let delete = match (blocked, only) {
        (Some(why), _) | (None, Some(why)) => delete.disabled_tip(why),
        (None, None) => delete.hover_tip(t("Move this profile to the Recycle Bin")),
    };
    if delete.clicked() { action = Some(RowAction::Delete); }
    if active {
        let edit = right.add_enabled(blocked.is_none(), ActionButton::secondary("EDIT").height(30.0));
        let edit = match blocked { Some(why) => edit.disabled_tip(why), None => edit.hover_tip(t("Change your name, motto, allegiance or picture")) };
        if edit.clicked() { action = Some(RowAction::Edit); }
    } else {
        let switch = right.add_enabled(blocked.is_none(), ActionButton::primary("SWITCH TO").height(30.0));
        let switch = match blocked { Some(why) => switch.disabled_tip(why), None => switch.hover_tip(t("Play as this commander, with their settings, saves and career")) };
        if switch.clicked() { action = Some(RowAction::Switch); }
    }
    right.add_space(6.0);
    if let Rank::Of(career) = career {
        ta_ui::badges::insignia(&mut right, career.rank().grade, 44.0);
    }
    let text_right = right.min_rect().min.x - 12.0;
    let avatar = Rect::from_min_size(pos2(rect.min.x + 10.0, rect.center().y - 25.0), vec2(50.0, 50.0));
    let mut picture = ui.new_child(UiBuilder::new().max_rect(avatar));
    ta_ui::profiles::avatar_picture(&mut picture, texture, Some(entry.allegiance), 50.0);
    // The name (and, the active one, its tag), the motto, the rank: a line each, centred.
    let left = avatar.max.x + 12.0;
    let width = (text_right - left).max(40.0);
    let line = |text: String, font: egui::FontId, color: egui::Color32, italics: bool| {
        let mut job = egui::text::LayoutJob::single_section(text, egui::TextFormat { font_id: font, color, italics, ..Default::default() });
        job.wrap.max_width = width;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('\u{2026}');
        ui.painter().layout_job(job)
    };
    let name = line(entry.name.clone(), strong(15.0), TEXT_BRIGHT, false);
    let motto = (!entry.motto.is_empty()).then(|| line(format!("\u{201c}{}\u{201d}", entry.motto), body(12.5), CREAM, true));
    let facts = {
        let mut job = egui::text::LayoutJob::default();
        let allegiance = tf("{faction} allegiance", &[("faction", &entry.allegiance.label())]);
        // (The rank and the XP once the career is read; that it cannot be, when it cannot.)
        let rest = match career {
            Rank::Of(career) => format!(" \u{b7} {allegiance} \u{b7} {}", tf("{xp} career XP", &[("xp", &grouped(career.total_xp()))])),
            Rank::Unreadable => format!(" \u{b7} {allegiance}"),
            Rank::Unknown => allegiance,
        };
        job.append(&rank, 0.0, egui::TextFormat { font_id: strong(12.5), color: rank_color, ..Default::default() });
        job.append(&rest, 0.0, egui::TextFormat { font_id: body(12.0), color: STEEL, ..Default::default() });
        job.wrap.max_width = width;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('\u{2026}');
        ui.painter().layout_job(job)
    };
    let gap = 3.0;
    let total = name.size().y + motto.as_ref().map_or(0.0, |m| m.size().y + gap) + gap + facts.size().y;
    let mut y = rect.center().y - total / 2.0;
    let name_width = name.size().x;
    let name_height = name.size().y;
    ui.painter().galley(pos2(left, y), name, TEXT_BRIGHT);
    if active {
        // (The tag beside the name, where it fits.)
        let tag = widgets::galley(ui, RichText::new(t("ACTIVE PROFILE")).font(strong(10.5)));
        let tag_rect = Rect::from_min_size(pos2(left + name_width + 10.0, y + (name_height - 18.0) / 2.0), vec2(tag.size().x + 14.0, 18.0));
        if tag_rect.max.x <= text_right {
            let mut slot = ui.new_child(UiBuilder::new().max_rect(tag_rect.expand2(vec2(4.0, 2.0))).layout(Layout::left_to_right(Align::Center)));
            widgets::tag(&mut slot, n("ACTIVE PROFILE"), AMBER);
        }
    }
    y += name_height + gap;
    if let Some(motto) = motto {
        let h = motto.size().y;
        ui.painter().galley(pos2(left, y), motto, CREAM);
        y += h + gap;
    }
    ui.painter().galley(pos2(left, y), facts, AMBER);
    action
}

/// This device's profiles that cannot be read: what they are, a row each with its folder, and
/// what can be done with them. Returns the one asked about, its folder, and what was asked.
fn unreadable_profiles(ui: &mut Ui, list: &[Unreadable], blocked: Option<&'static str>) -> Option<(String, std::path::PathBuf, UnreadableAction)> {
    ui.label(widgets::caption("PROFILES THAT CANNOT BE READ"));
    ui.add(egui::Label::new(RichText::new(t("Their files are damaged, or from a newer version of TA Reborn. They are left as they are: repair them in their folders, or move them to the Recycle Bin."))
        .color(STEEL).size(12.0)).wrap());
    let mut picked = None;
    ui.spacing_mut().item_spacing.y = 6.0;
    for entry in list {
        if let Some(action) = unreadable_row(ui, entry, blocked) {
            picked = Some((entry.id.clone(), entry.folder.clone(), action));
        }
    }
    picked
}

/// A profile that cannot be read: the emblem, what it is, its folder and why (on hover); opening
/// the folder, and moving it to the Recycle Bin.
fn unreadable_row(ui: &mut Ui, entry: &Unreadable, blocked: Option<&'static str>) -> Option<UnreadableAction> {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 68.0), Sense::hover());
    let folder = entry.folder.display().to_string();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{}, {folder}", t("A profile that cannot be read"))));
    if !ui.is_rect_visible(rect) { return None; }
    ui.painter().rect_filled(rect, 4.0, BG_DEEP);
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, RED.gamma_multiply(0.5)), StrokeKind::Inside);
    let mut action = None;
    let mut right = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(12.0, 0.0))).layout(Layout::right_to_left(Align::Center)));
    right.spacing_mut().item_spacing.x = 8.0;
    let remove = right.add_enabled(blocked.is_none(), ActionButton::secondary("MOVE TO RECYCLE BIN").height(30.0));
    let remove = match blocked { Some(why) => remove.disabled_tip(why), None => remove.hover_tip(t("Move this profile to the Recycle Bin")) };
    if remove.clicked() { action = Some(UnreadableAction::Remove); }
    if right.add(ActionButton::secondary("OPEN FOLDER").height(30.0)).hover_tip(t("Its folder: repair its files there, or take what is in it")).clicked() {
        action = Some(UnreadableAction::Open);
    }
    let text_right = right.min_rect().min.x - 12.0;
    let avatar = Rect::from_min_size(pos2(rect.min.x + 10.0, rect.center().y - 25.0), vec2(50.0, 50.0));
    let mut picture = ui.new_child(UiBuilder::new().max_rect(avatar));
    ta_ui::profiles::avatar_picture(&mut picture, None, None, 50.0);
    let left = avatar.max.x + 12.0;
    let width = (text_right - left).max(40.0);
    let line = |text: String, font: egui::FontId, color: egui::Color32| {
        let mut job = egui::text::LayoutJob::single_section(text, egui::TextFormat { font_id: font, color, ..Default::default() });
        job.wrap.max_width = width;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('\u{2026}');
        ui.painter().layout_job(job)
    };
    let title = line(t("A profile that cannot be read").to_string(), strong(15.0), TEXT_BRIGHT);
    // (Its folder whole where it fits, else its end: where it is, the folder's own name.)
    let mut path = line(folder.clone(), mono(11.5), STEEL);
    if path.size().x >= width - 1.0 {
        let end: Vec<_> = entry.folder.components().rev().take(2).collect();
        let end: std::path::PathBuf = end.into_iter().rev().collect();
        path = line(format!("\u{2026}{}{}", std::path::MAIN_SEPARATOR, end.display()), mono(11.5), STEEL);
    }
    let reason = line(ta_ui::profiles::first_line(&entry.error), body(12.0), STEEL_DIM);
    let gap = 3.0;
    let mut y = rect.center().y - (title.size().y + path.size().y + reason.size().y + 2.0 * gap) / 2.0;
    for (galley, color) in [(title, TEXT_BRIGHT), (path, STEEL), (reason, STEEL_DIM)] {
        let h = galley.size().y;
        ui.painter().galley(pos2(left, y), galley, color);
        y += h + gap;
    }
    // (Where it is and the whole of why, on hover of the row.)
    let _ = ui.interact(Rect::from_min_max(rect.min, pos2(text_right, rect.max.y)), ui.id().with(("unreadable_profile", &entry.id)), Sense::hover())
        .hover_tip(format!("{folder}\n{}", entry.error));
    action
}
