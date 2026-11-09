//! Local profile selector and the settings/data handover between commanders.
use ta_ui::tooltip::TooltipExt as _;
use std::path::PathBuf;
use egui::{Align, Layout, Rect, RichText, Ui, UiBuilder, pos2, vec2};
use ta_config::profiles::{Career, Profile, ProfileStore};
use ta_ui::i18n::{grouped, t, tf};
use ta_ui::profiles::{CareerData, CareerView, ProfileDraft, ProfileEditor};
use ta_ui::panel::{Exit, Panel};
use crate::persist::ConfigStore;
use crate::theme::*;
use super::LauncherApp;

/// The parts of the Profile page: the service record's tabs and the commanders of this device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ProfileTab { #[default] Overview, Achievements, Campaign, History, Commanders }

/// What the Profile page loads off the interface's thread: the active commander's record, and
/// the careers of every commander of this device (their ranks, in the Commanders tab).
pub(super) struct Details {
    pub record: CareerData,
    pub careers: std::collections::BTreeMap<String, Career>,
}

pub(super) struct Profiles {
    pub store: Option<ProfileStore>,
    pub active: Option<Profile>,
    pub(super) list: Vec<Profile>,
    pub(super) career: Career,
    pub error: Option<String>,
    /// Damage to another identity is visible in the chooser, but never disables
    /// a healthy active commander or prevents reading their career.
    pub(super) list_error: Option<String>,
    editor: Option<ProfileEditor>,
    editing: bool,
    pub required: bool,
    /// The Profile page's tab, and a page asked to be shown (`--show`, the menu).
    pub(super) tab: ProfileTab,
    pub(super) open_page: Option<ProfileTab>,
    pub(super) view: CareerView,
    pub(super) details: Option<Details>,
    pub(super) details_pending: Option<std::sync::mpsc::Receiver<Result<Details, String>>>,
    pub(super) details_error: Option<String>,
    /// A commander whose profile is to be deleted, once asked.
    pub(super) confirm_delete: Option<String>,
    open_menu: bool,
    /// Notices of achievements the commander's earlier play has earned (an update having
    /// brought them), shown on opening the profile.
    pub toasts: ta_ui::profiles::CareerNotices,
}

impl Profiles {
    pub fn open(root: PathBuf, required: bool) -> Self {
        let mut state = Self { store: None, active: None, list: Vec::new(), career: Career::default(),
            error: None, list_error: None, editor: None, editing: false, required, tab: ProfileTab::Overview, open_page: None,
            view: CareerView::default(), details: None, details_pending: None, details_error: None, confirm_delete: None, open_menu: false,
            toasts: Default::default() };
        let result = (|| -> std::io::Result<()> {
            let store = ProfileStore::open(root.clone())?;
            state.store = Some(store.clone());
            state.active = if cfg!(test) { store.active()? } else { ta_config::profiles::initialize(root, None)? };
            if let Some(id) = state.active.as_ref().map(|p| p.id.clone()) {
                state.earned_from_record(&store, &id, required);
                state.career = store.career(&id)?;
            }
            state.store = Some(store);
            Ok(())
        })();
        if let Err(error) = result { state.error = Some(error.to_string()); }
        state.refresh_list();
        if state.active.is_none() && required && state.error.is_none() { state.edit(false); }
        state
    }
    pub(super) fn dialog_open(&self) -> bool { self.editor.is_some() || self.confirm_delete.is_some() }
    pub(super) fn unsaved_edits(&self) -> bool { self.editor.as_ref().is_some_and(ProfileEditor::changed) }

    /// Records the achievements the commander's play has met but not unlocked (an update
    /// having brought them); `notify`: with their notices.
    fn earned_from_record(&mut self, store: &ProfileStore, id: &str, notify: bool) {
        match store.refresh_achievements(id) {
            Ok(earned) if notify => self.toasts.push(&earned, &earned),
            Ok(_) => {}
            Err(error) => log::warn!("could not record achievements: {error}"),
        }
    }
    /// Whether the Profile page, when shown (`on_page`), has what it shows.
    pub(super) fn capture_ready(&self, on_page: bool) -> bool {
        !on_page || self.active.is_none() || self.details.is_some() || self.details_error.is_some()
    }

    /// Asks for the Profile page, at `tab` (the launcher shows it on its next frame).
    pub(super) fn show_page(&mut self, tab: ProfileTab) {
        self.open_page = Some(tab);
        self.view = CareerView::default();
        self.view.tab = match tab {
            ProfileTab::Achievements => ta_ui::profiles::CareerTab::Achievements,
            ProfileTab::Campaign => ta_ui::profiles::CareerTab::Campaign,
            ProfileTab::History => ta_ui::profiles::CareerTab::History,
            _ => ta_ui::profiles::CareerTab::Overview,
        };
    }
    pub(super) fn show_career(&mut self) { self.show_page(ProfileTab::Overview); }

    /// The active commander has not finished a battle yet (Quick start leads them to the
    /// tutorial).
    pub(crate) fn newcomer(&self) -> bool {
        self.active.is_some() && self.career.matches_finished == 0
    }
    pub(super) fn show_achievements(&mut self) { self.show_page(ProfileTab::Achievements); }
    pub(super) fn show_achievement(&mut self, id: &str) {
        self.show_achievements();
        if let Some(a) = ta_config::achievements::find(id) { self.view.achievement(a.id); }
    }
    pub(super) fn show_campaign(&mut self) { self.show_page(ProfileTab::Campaign); }
    pub(super) fn show_history(&mut self) { self.show_page(ProfileTab::History); }
    pub(super) fn show_menu(&mut self) { self.open_menu = true; }

    pub(super) fn edit(&mut self, existing: bool) {
        self.editing = existing;
        self.editor = Some(ProfileEditor::new(if existing { self.active.as_ref() } else { None }));
    }
    pub fn data_dir(&self, fallback: PathBuf) -> PathBuf {
        self.store.as_ref().zip(self.active.as_ref()).and_then(|(store, p)| store.data_dir(&p.id).ok()).unwrap_or(fallback)
    }
    pub fn refresh(&mut self) {
        // (What the Profile page shows is read again when it is next shown.)
        self.details = None;
        self.details_pending = None;
        self.details_error = None;
        let Some(store) = &self.store else { return };
        let result = (|| -> std::io::Result<()> {
            if let Some(active) = &mut self.active {
                *active = store.get(&active.id)?;
                self.career = store.career(&active.id)?;
            }
            Ok(())
        })();
        self.error = result.err().map(|e| e.to_string());
        self.refresh_list();
    }

    fn refresh_list(&mut self) {
        let Some(store) = &self.store else { return };
        match store.list_available() {
            Ok((list, warnings)) => {
                self.list = list;
                for warning in &warnings { log::warn!("profile list: {warning}"); }
                self.list_error = warnings.into_iter().next();
            }
            Err(error) => {
                // A directory-listing problem must not discard an already validated
                // active identity. The chooser still offers it and reports the issue.
                self.list = self.active.iter().cloned().collect();
                self.list_error = Some(error.to_string());
            }
        }
    }
}

impl LauncherApp {
    pub(super) fn refresh_profile(&mut self) {
        self.rescan();
        self.discover_mods();
        self.profiles.refresh();
    }

    pub(super) fn sync_profile_name(&mut self) {
        if !matches!(self.store.status, crate::persist::SaveStatus::Saved(_)) { return; }
        let Some(profile) = &self.profiles.active else { return };
        if profile.name == self.config.gameplay.player_name { return; }
        let Some(store) = &self.profiles.store else { return };
        // (The allegiance as the profile has it now: an edit just saved may have changed it
        // along with the name, after `active` was read.)
        let allegiance = store.get(&profile.id).map_or(profile.allegiance, |current| current.allegiance);
        match store.update(&profile.id, &self.config.gameplay.player_name, allegiance) {
            Ok(profile) => {
                // The store trims names. Adopt that canonical value immediately so the
                // next frame cannot keep writing the same rename transaction forever.
                self.config.gameplay.player_name = profile.name.clone();
                self.profiles.active = Some(profile);
                self.profiles.refresh();
            }
            Err(error) => {
                self.config.gameplay.player_name = profile.name.clone();
                self.notify(tf("Could not update profile: {error}", &[("error", &error)]), RED);
            }
        }
        // A normalized or rejected name must also reach disk before an immediate Play,
        // profile switch or Close; do not leave it waiting on the next frame's debounce.
        let _ = self.store.flush(&mut self.config);
    }

    /// A single compact identity control in the launcher's top-right corner. The
    /// full career and profile management live in its menu rather than another toolbar.
    pub(super) fn profile_widget(&mut self, ui: &mut Ui) {
        let mut choose = None;
        let blocked = self.profile_blocked();
        let running = blocked.is_some();
        let profile = self.profiles.active.clone();
        let name = profile.as_ref().map_or_else(|| t("CHOOSE PROFILE").to_string(), |p| p.name.clone());
        let rank = ta_ui::profiles::rank_text(&self.profiles.career);
        let response = ui.scope(|ui| {
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), egui::Sense::click());
            let fill = if response.hovered() || response.has_focus() { PANEL_HI } else { BG_DEEP };
            ui.painter().rect_filled(rect, 5.0, fill);
            ui.painter().rect_stroke(rect, 5.0, egui::Stroke::new(1.0, if response.hovered() || response.has_focus() { AMBER_DIM } else { LINE }), egui::StrokeKind::Inside);
            ta_ui::widgets::reveal_focus(ui, &response);
            let avatar = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(40.0, 40.0));
            let mut picture = ui.new_child(UiBuilder::new().max_rect(avatar).layout(Layout::top_down(Align::Min)));
            let texture = profile.as_ref().and_then(|profile| {
                let has_avatar = ta_ui::profiles::current(ui.ctx()).filter(|p| p.id == profile.id).map_or(profile.has_avatar, |p| p.has_avatar);
                self.profiles.store.as_ref().filter(|_| has_avatar).and_then(|s| s.avatar_path(&profile.id).ok())
                    .and_then(|p| ta_ui::portrait::texture_at(ui.ctx(), &p))
            });
            ta_ui::profiles::avatar_picture(&mut picture, texture.as_ref(), profile.as_ref().map(|p| p.allegiance), 40.0);
            // (The rank's insignia, before the menu's arrow.)
            let has_rank = profile.is_some();
            let insignia_size = 34.0;
            let right = if has_rank { 24.0 + insignia_size + 6.0 } else { 24.0 };
            if has_rank {
                let at = Rect::from_center_size(pos2(rect.max.x - 24.0 - insignia_size / 2.0, rect.center().y), vec2(insignia_size, insignia_size));
                ta_ui::badges::paint_insignia(ui.ctx(), ui.painter(), at, self.profiles.career.rank().grade);
            }
            // (A name over a rank; no profile's call to choose one alone, in the middle.)
            let text_rect = if profile.is_some() {
                Rect::from_min_max(rect.min + vec2(56.0, 5.0), rect.max - vec2(right, 5.0))
            } else {
                Rect::from_min_max(pos2(rect.min.x + 56.0, rect.center().y - 9.0), pos2(rect.max.x - right, rect.center().y + 10.0))
            };
            let mut text = ui.new_child(UiBuilder::new().max_rect(text_rect).layout(Layout::top_down(Align::Min)));
            text.set_clip_rect(text.clip_rect().intersect(text_rect));
            text.spacing_mut().item_spacing.y = 1.0;
            text.add(egui::Label::new(RichText::new(&name).font(strong(13.5)).color(TEXT_BRIGHT)).truncate());
            if profile.is_some() {
                text.add(egui::Label::new(RichText::new(&rank).font(body(10.5)).color(AMBER)).truncate());
                let bar = Rect::from_min_size(pos2(text_rect.min.x, rect.max.y - 10.0), vec2(text_rect.width(), 5.0));
                ta_ui::profiles::paint_xp_track(&text, bar, self.profiles.career.rank().progress);
            }
            let c = pos2(rect.max.x - 12.0, rect.center().y);
            ui.painter().add(egui::Shape::convex_polygon(vec![c + vec2(-3.5, -2.0), c + vec2(3.5, -2.0), c + vec2(0.0, 2.0)], STEEL, egui::Stroke::NONE));
            ta_ui::widgets::focus_ring(ui, &response, rect);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("{}: {name}, {rank}", t("PLAYER PROFILE"))));
            response
        }).inner;
        let response = response.hover_tip_ui(|ui| {
                ui.label(RichText::new(t("PLAYER PROFILE")).font(strong(12.0)).color(STEEL));
                ui.label(RichText::new(&name).color(TEXT_BRIGHT));
                if profile.is_some() {
                    ui.label(RichText::new(&rank).color(AMBER));
                    let rank = self.profiles.career.rank();
                    ui.label(ta_ui::profiles::progress_text(&rank));
                }
            });
        if self.profiles.open_menu {
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
            self.profiles.open_menu = false;
        }
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        egui::Popup::menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).align(egui::emath::RectAlign::BOTTOM_END)
            .width(320.0_f32.min(ui.ctx().content_rect().width() - 24.0))
            .show(|ui| {
                // Popup areas start with egui's 400-point default height. Allow the
                // menu to use the available screen before its contents need to scroll.
                let height = (ui.ctx().content_rect().height() - 110.0).max(120.0);
                ui.set_max_height(height);
                ta_ui::scroll::show(ui, egui::ScrollArea::vertical().id_salt("profile_menu_body").max_height(height), ta_ui::scroll::Behind::Fill(PANEL), |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    // Menu rows fill the width, but wrapped names and diagnostics
                    // must not inherit justified glyph spacing from that layout.
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        if profile.is_some() {
                            ui.label(ta_ui::widgets::caption("COMMANDER DOSSIER"));
                            ui.horizontal_top(|ui| {
                                ta_ui::profiles::rank_insignia(ui, &self.profiles.career, 48.0);
                                ui.vertical(|ui| {
                                    ui.add(egui::Label::new(RichText::new(&name).font(strong(19.0)).color(TEXT_BRIGHT)).wrap());
                                    ui.add(egui::Label::new(RichText::new(&rank).color(AMBER)).wrap());
                                    ui.add(egui::Label::new(RichText::new(ta_ui::profiles::next_rank_text(&self.profiles.career.rank())).small().color(STEEL)).wrap());
                                });
                            });
                            ta_ui::profiles::xp_progress(ui, &self.profiles.career);
                            ui.label(RichText::new(tf("{xp} XP · {wins} wins · {matches} matches", &[("xp", &grouped(self.profiles.career.total_xp())), ("wins", &grouped(self.profiles.career.wins)), ("matches", &grouped(self.profiles.career.matches_finished))])).small().color(STEEL));
                            if ui.add(ta_ui::widgets::ActionButton::primary("VIEW PROFILE").size(ui.available_width(), 34.0)).clicked() {
                                self.profiles.show_career(); ui.close();
                            }
                            if ui.add(ta_ui::widgets::ActionButton::secondary("ACHIEVEMENTS").size(ui.available_width(), 30.0)).clicked() {
                                self.profiles.show_achievements(); ui.close();
                            }
                            ui.separator();
                        }
                        ui.label(ta_ui::widgets::caption("PLAYING ON THIS DEVICE"));
                        if let Some(error) = &self.profiles.list_error {
                            ui.add(egui::Label::new(RichText::new(tf("Could not load profile: {error}", &[("error", error)])).color(AMBER).size(12.0)).wrap());
                        }
                        if let Some(reason) = &blocked { ui.add(egui::Label::new(RichText::new(*reason).color(STEEL).size(12.0)).wrap()); }
                        for entry in &self.profiles.list {
                            let selected = profile.as_ref().is_some_and(|p| p.id == entry.id);
                            let texture = self.profiles.store.as_ref().filter(|_| entry.has_avatar)
                                .and_then(|s| s.avatar_path(&entry.id).ok())
                                .and_then(|path| ta_ui::portrait::texture_at(ui.ctx(), &path));
                            if ui.add_enabled_ui(!running, |ui| ta_ui::profiles::profile_choice(ui, entry, texture.as_ref(), selected)).inner.clicked() {
                                if !selected { choose = Some(entry.id.clone()); }
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.add_enabled(!running, ta_ui::widgets::ActionButton::secondary("NEW PROFILE").size(ui.available_width(), 30.0)).clicked() {
                            self.profiles.edit(false); ui.close();
                        }
                        if profile.is_some() && ui.add_enabled(!running, ta_ui::widgets::ActionButton::secondary("EDIT PROFILE").size(ui.available_width(), 30.0)).clicked() {
                            self.profiles.edit(true); ui.close();
                        }
                        if profile.is_some() && ui.add(ta_ui::widgets::ActionButton::secondary("MANAGE PROFILES").size(ui.available_width(), 30.0))
                            .hover_tip(t("Every commander on this device: switch, edit or delete them")).clicked() {
                            self.profiles.show_page(ProfileTab::Commanders); ui.close();
                        }
                        if self.profiles.error.is_some() && ui.button(t("PROFILE ERROR")).clicked() { self.profiles.edit(profile.is_some()); ui.close(); }
                    });
                });
            });
        if let Some(id) = choose { self.switch_profile(&id); }
    }

    pub(super) fn profile_dialog(&mut self, ctx: &egui::Context) {
        if self.profiles.confirm_delete.is_some() { self.delete_profile_dialog(ctx); return; }
        let Some(editor) = &mut self.profiles.editor else { return };
        let required = self.profiles.active.is_none() && self.profiles.required;
        let width = (ctx.content_rect().width() - 76.0).clamp(180.0, 460.0);
        let panel = Panel::new(if self.profiles.editing { "EDIT PROFILE" } else { "CREATE PROFILE" }).width(width).max_height((ctx.content_rect().height() - 24.0).max(120.0))
            .exit(if required { Exit::None } else { Exit::Close });
        let response = ta_ui::panel::modal(ctx, "commander_setup", panel, !required, |ui| {
            if let Some(error) = &self.profiles.error { ui.colored_label(RED, tf("Could not load profile: {error}", &[("error", error)])); }
            ui.add_enabled_ui(!self.game.is_running() && !self.installer.busy() && !self.skins.busy() && !self.maps.busy(), |ui| editor.show(ui)).inner
        });
        if let Some(draft) = response.inner { self.save_profile(draft); }
        else if response.exit && !required { self.profiles.editor = None; }
    }

    pub(super) fn save_profile(&mut self, draft: ProfileDraft) {
        if let Some(why) = self.profile_blocked() { self.notify(why.to_string(), AMBER); return; }
        let result = (|| -> Result<String, String> {
            self.store.flush(&mut self.config)?;
            self.sync_profile_name();
            self.store.flush(&mut self.config)?;
            let store = self.profiles.store.as_ref().ok_or_else(|| self.profiles.error.clone().unwrap_or_default())?;
            if self.profiles.editing {
                let id = self.profiles.active.as_ref().ok_or_else(|| t("Choose a profile first.").to_string())?.id.clone();
                store.update(&id, &draft.name, draft.allegiance).map_err(|e| e.to_string())?;
                store.set_motto(&id, &draft.motto).map_err(|e| e.to_string())?;
                if let Some(bytes) = &draft.avatar_png { store.set_avatar(&id, bytes).map_err(|e| e.to_string())?; }
                else if draft.clear_avatar { store.remove_avatar(&id).map_err(|e| e.to_string())?; }
                Ok(id)
            } else {
                let profile = store.create(&draft.name, draft.allegiance, draft.avatar_png.as_deref(), &self.config).map_err(|e| e.to_string())?;
                if !draft.motto.is_empty() { store.set_motto(&profile.id, &draft.motto).map_err(|e| e.to_string())?; }
                if draft.clear_avatar { store.remove_avatar(&profile.id).map_err(|e| e.to_string())?; }
                Ok(profile.id)
            }
        })();
        match result {
            Ok(id) => { if self.switch_profile(&id) { self.profiles.editor = None; } }
            Err(error) => if let Some(editor) = &mut self.profiles.editor { editor.error = Some(error); },
        }
    }

    pub(super) fn switch_profile(&mut self, id: &str) -> bool {
        if let Some(why) = self.profile_blocked() { self.notify(why.to_string(), AMBER); return false; }
        let result = (|| -> Result<(), String> {
            self.store.flush(&mut self.config)?;
            self.sync_profile_name();
            self.store.flush(&mut self.config)?;
            let profiles = self.profiles.store.as_ref().ok_or_else(|| t("Choose a profile first.").to_string())?;
            let path = profiles.config_path(id).map_err(|e| e.to_string())?;
            // Prepare the exact values to adopt before changing either identity.
            // Do not reopen after selection: an intervening read failure would
            // silently replace this commander's settings with recovered defaults.
            let (store, config) = ConfigStore::try_open_in(path, self.store.dir()).map_err(|e| e.to_string())?;
            // The career can fail independently of the profile/config files. Read it
            // before selecting/pinning, so a failed handover cannot split identities.
            let career = profiles.career(id).map_err(|e| e.to_string())?;
            let profile = if cfg!(test) { profiles.select(id) } else { profiles.select_and_pin(id) }.map_err(|e| e.to_string())?;
            let profiles = profiles.clone();
            self.profiles.toasts = Default::default();
            self.profiles.earned_from_record(&profiles, id, !cfg!(test));
            self.store = store;
            self.config = config;
            self.profiles.active = Some(profile);
            // A record is a snapshot of one pinned identity, never shared on handover.
            self.profiles.details = None;
            self.profiles.details_pending = None;
            self.profiles.details_error = None;
            self.profiles.view = CareerView::default();
            self.profiles.career = career;
            self.profiles.error = None;
            Ok(())
        })();
        if let Err(error) = result {
            // Validation or selection failed before the in-memory handover.
            // Poisoning the active profile's error state here would disable Play
            // even though its settings and identity have never been replaced.
            if let Some(editor) = &mut self.profiles.editor { editor.error = Some(error.clone()); }
            self.notify(tf("Could not load profile: {error}", &[("error", &error)]), RED);
            return false;
        }
        self.profiles.refresh();
        self.battles = crate::battles::Battles::scan(&self.profiles.data_dir(self.store.dir()), ta_config::pictures_dir());
        self.settings_view = ta_ui::settings::SettingsView::default();
        self.confirm_reset = false;
        self.rescan();
        self.refresh_mods();
        self.skins.shutdown();
        self.skins = crate::skins::Library::new(self.store.dir().join("skins"), [self.config.graphics.unit_skin.clone(), self.config.graphics.enemy_skin().to_string()]);
        let language = std::env::var("TA_LANG").unwrap_or_else(|_| self.config.gameplay.language.clone());
        if let Err(error) = ta_ui::i18n::set_language(&language, &self.language_dirs) { log::warn!("{error}"); }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use ta_config::{Config, profiles::Allegiance};

    fn launcher(name: &str) -> (PathBuf, LauncherApp, ProfileStore, Profile, Profile) {
        let root = std::env::temp_dir().join(format!("ta_launcher_profiles_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profiles = ProfileStore::open(&root).unwrap();
        let first = profiles.create("First", Allegiance::Arm, None, &Config::default()).unwrap();
        let second = profiles.create("Second", Allegiance::Core, None, &Config::default()).unwrap();
        profiles.select(&first.id).unwrap();
        let path = root.join("config.toml");
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--config", path.to_str().unwrap()]);
        let ctx = egui::Context::default();
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx), args);
        app.skins.shutdown();
        (root, app, profiles, first, second)
    }

    #[test]
    fn launcher_qa_unsaved_profile_edits_are_kept_when_closing() {
        let (root, mut app, profiles, first, _) = launcher("qa_close_profile");
        app.profiles.edit(true);
        assert!(app.work_at_risk().is_empty(), "opening an unchanged editor creates no unsaved work");
        app.profiles.editor.as_mut().unwrap().motto = "Unsaved motto".into();
        assert!(!app.work_at_risk().is_empty(), "closing must protect a profile draft just like a skin draft");
        assert!(profiles.get(&first.id).unwrap().motto.is_empty(), "the draft has not been written yet");
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_names_are_normalized_and_rejected_edits_repaired_before_launch() {
        let (root, mut app, profiles, first, _) = launcher("canonical_name");
        app.config.gameplay.player_name = "  New Commander  ".into();
        app.store.flush(&mut app.config).unwrap();
        app.sync_profile_name();
        assert_eq!(app.config.gameplay.player_name, "New Commander");
        assert_eq!(profiles.get(&first.id).unwrap().name, "New Commander");
        let path = profiles.config_path(&first.id).unwrap();
        assert_eq!(Config::try_load_from(&path).unwrap().gameplay.player_name, "New Commander");
        app.config.gameplay.player_name = "Second".into();
        app.store.flush(&mut app.config).unwrap();
        app.sync_profile_name();
        assert_eq!(app.config.gameplay.player_name, "New Commander", "duplicate identity rejected");
        assert_eq!(Config::try_load_from(&path).unwrap().gameplay.player_name, "New Commander", "rejection persists before another frame");
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// EDIT PROFILE with a new name and a new allegiance: both are kept (the rename taken into
    /// the settings must not write the allegiance back as it was).
    #[test]
    fn editing_the_name_and_allegiance_together_keeps_both() {
        let (root, mut app, profiles, first, _) = launcher("rename_and_allegiance");
        assert_eq!(first.allegiance, Allegiance::Arm);
        app.profiles.edit(true);
        app.save_profile(ProfileDraft { name: "Renamed".into(), motto: "New motto".into(), allegiance: Allegiance::Core, avatar_png: None, clear_avatar: false });
        assert!(app.profiles.editor.is_none(), "the edit is saved: {:?}", app.profiles.editor.as_ref().and_then(|e| e.error.clone()));
        let saved = profiles.get(&first.id).unwrap();
        assert_eq!((saved.name.as_str(), saved.allegiance, saved.motto.as_str()), ("Renamed", Allegiance::Core, "New motto"));
        let active = app.profiles.active.as_ref().unwrap();
        assert_eq!((active.name.as_str(), active.allegiance), ("Renamed", Allegiance::Core));
        assert_eq!(app.config.gameplay.player_name, "Renamed");
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn switching_profiles_flushes_the_previous_commanders_pending_rename() {
        let (root, mut app, profiles, first, second) = launcher("switch_rename");
        app.config.gameplay.player_name = " Renamed ".into();
        assert!(app.switch_profile(&second.id));
        assert_eq!(profiles.get(&first.id).unwrap().name, "Renamed");
        assert_eq!(Config::try_load_from(&profiles.config_path(&first.id).unwrap()).unwrap().gameplay.player_name, "Renamed");
        assert_eq!(app.config.gameplay.player_name, "Second");
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unreadable_destination_career_keeps_the_selected_identity_and_settings_together() {
        let (root, mut app, profiles, first, second) = launcher("career_handover");
        let current_path = app.store.path.clone();
        let career = profiles.data_dir(&second.id).unwrap().join("career.toml");
        std::fs::write(&career, "version = [broken career").unwrap();
        app.config.audio.music = 0.25;
        assert!(!app.switch_profile(&second.id));
        assert_eq!(profiles.active().unwrap().unwrap().id, first.id, "a failed handover must not select a different on-disk commander");
        assert_eq!(app.profiles.active.as_ref().unwrap().id, first.id);
        assert_eq!(app.store.path, current_path);
        assert_eq!(Config::try_load_from(&current_path).unwrap().audio.music, 0.25);
        assert!(app.profiles.error.is_none(), "the current commander's profile remains usable");
        assert_eq!(std::fs::read_to_string(&career).unwrap(), "version = [broken career");
        std::fs::remove_file(&career).unwrap();
        assert!(app.switch_profile(&second.id), "a repaired career can be selected normally");
        app.skins.shutdown();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_handover_cannot_discard_an_unsaved_skin_editor() {
        let (root, mut app, profiles, first, second) = launcher("dirty_skin_handover");
        let mut editor = crate::skin_editor::Editor::open(ta_skins::load("builtin:navy", &root.join("skins")).unwrap(), None);
        editor.manifest.name = "Unsaved paint".into();
        assert!(editor.changed());
        app.skins.editor = Some(editor);
        assert!(!app.switch_profile(&second.id));
        assert_eq!(profiles.active().unwrap().unwrap().id, first.id);
        app.profiles.edit(false);
        app.save_profile(ProfileDraft { name: "Third".into(), motto: String::new(), allegiance: Allegiance::Arm, avatar_png: None, clear_avatar: false });
        assert_eq!(profiles.list().unwrap().len(), 2, "saving a new profile must respect the same editor guard");
        assert_eq!(app.skins.editor.as_ref().unwrap().manifest.name, "Unsaved paint");
        app.skins.editor = None;
        assert!(app.switch_profile(&second.id));
        app.skins.shutdown();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_destination_keeps_the_current_profile_usable_and_can_be_retried() {
        let (root, mut app, profiles, first, second) = launcher("invalid_destination");
        let second_path = profiles.config_path(&second.id).unwrap();
        let original = std::fs::read(&second_path).unwrap();
        let current_path = app.store.path.clone();
        app.config.audio.music = 0.25;
        for broken in [Some("graphics = [[[ invalid profile"), None] {
            if let Some(text) = broken { std::fs::write(&second_path, text).unwrap(); }
            else { std::fs::remove_file(&second_path).unwrap(); }
            assert!(!app.switch_profile(&second.id));
            assert_eq!(app.profiles.active.as_ref().unwrap().id, first.id);
            assert_eq!(profiles.active().unwrap().unwrap().id, first.id);
            assert_eq!(app.store.path, current_path);
            assert_eq!(app.config.audio.music, 0.25);
            assert!(app.profiles.error.is_none(), "an invalid destination must not disable the valid active profile");
            assert!(app.last_notice().is_some_and(|notice| notice.text.contains("Could not load profile")));
        }
        assert_eq!(Config::try_load_from(&current_path).unwrap().audio.music, 0.25, "pending edits to the old profile remain saved");
        std::fs::write(&second_path, original).unwrap();
        assert!(app.switch_profile(&second.id), "repairing the destination allows an ordinary retry");
        app.skins.shutdown();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn damaged_sibling_metadata_does_not_block_healthy_profiles_or_their_careers() {
        let (root, mut app, profiles, first, second) = launcher("damaged_sibling");
        let third = profiles.create("Third", Allegiance::Arm, None, &Config::default()).unwrap();
        profiles.select(&first.id).unwrap();
        let award = profiles.award(&first.id, "healthy-match", &ta_config::profiles::MatchCounters { kills: 3, ..Default::default() }).unwrap();
        assert!(award.xp > 0);
        let path = profiles.data_dir(&second.id).unwrap().join("profile.toml");
        let original = std::fs::read(&path).unwrap();
        std::fs::write(&path, "name = [broken metadata").unwrap();

        app.profiles.refresh();
        let reopened = Profiles::open(root.clone(), true);
        for state in [&app.profiles, &reopened] {
            assert!(state.error.is_none(), "unrelated metadata must not disable Play for a healthy commander");
            assert_eq!(state.active.as_ref().unwrap().id, first.id);
            assert_eq!(state.career.xp, award.xp, "career still loads after startup and refresh");
            assert!(state.list_error.as_ref().is_some_and(|warning| warning.contains(&second.id)));
            assert_eq!(state.list.len(), 2);
            assert!(!state.list.iter().any(|profile| profile.id == second.id));
        }
        assert!(app.switch_profile(&third.id), "other healthy commanders remain selectable");
        assert_eq!(app.profiles.career.xp, 0);
        assert!(app.profiles.error.is_none());
        std::fs::write(&path, original).unwrap();
        app.profiles.refresh();
        assert!(app.profiles.list_error.is_none());
        assert_eq!(app.profiles.list.len(), 3);
        app.skins.shutdown();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn switching_profiles_discards_open_dossier_and_pending_previous_player_data() {
        let (root, mut app, _, _, second) = launcher("dossier_handover");
        app.profiles.show_career();
        app.profiles.details = Some(Details { record: CareerData { career: Career { xp: 10000, ..Default::default() }, ..Default::default() }, careers: Default::default() });
        let (send, receive) = std::sync::mpsc::channel();
        app.profiles.details_pending = Some(receive);
        assert!(app.switch_profile(&second.id));
        assert!(app.profiles.details.is_none() && app.profiles.details_pending.is_none());
        assert!(send.send(Ok(Details { record: CareerData::default(), careers: Default::default() })).is_err(), "old reader cannot deliver into the new profile");
        app.profiles.show_career();
        assert_eq!(app.profiles.career.xp, 0);
        assert!(app.profiles.details.is_none());
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn deleting_the_active_commander_hands_over_and_the_last_one_stays() {
        let (root, mut app, profiles, first, second) = launcher("delete_active");
        let first_dir = profiles.data_dir(&first.id).unwrap();
        app.delete_profile(&first.id, &first.name);
        assert!(!first_dir.exists(), "the profile's folder is gone");
        assert_eq!(app.profiles.active.as_ref().unwrap().id, second.id, "the next commander takes over");
        assert_eq!(profiles.active().unwrap().unwrap().id, second.id);
        assert_eq!(app.profiles.list.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), [second.id.as_str()]);
        assert!(app.store.path.starts_with(profiles.data_dir(&second.id).unwrap()), "the settings are the new commander's");
        // (The last commander stays: the launcher's settings are theirs.)
        app.delete_profile(&second.id, &second.name);
        assert!(profiles.data_dir(&second.id).unwrap().exists());
        assert_eq!(app.profiles.active.as_ref().unwrap().id, second.id);
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_profile_page_shows_every_tab_once_its_record_is_read() {
        use eframe::App as _;
        let (root, mut app, profiles, first, _) = launcher("profile_page");
        profiles.set_motto(&first.id, "Hold the line").unwrap();
        profiles.award(&first.id, "page-match", &ta_config::profiles::MatchCounters { kills: 12, finished: true, won: true, ..Default::default() }).unwrap();
        app.profiles.refresh();
        let ctx = egui::Context::default();
        ta_ui::theme::install(&ctx);
        let mut frame = eframe::Frame::_new_kittest();
        for tab in [ProfileTab::Overview, ProfileTab::Achievements, ProfileTab::Campaign, ProfileTab::History, ProfileTab::Commanders] {
            app.profiles.show_page(tab);
            let start = std::time::Instant::now();
            for _ in 0..3 {
                loop {
                    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 720.0))), ..Default::default() };
                    ctx.run_ui(input, |ui| app.ui(ui, &mut frame)).drop_without_applying_deltas();
                    if app.profiles.details.is_some() || app.profiles.details_error.is_some() || start.elapsed().as_secs() > 60 { break; }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            assert!(app.profiles.details_error.is_none(), "{:?}", app.profiles.details_error);
            assert_eq!(app.profiles.details.as_ref().unwrap().record.career.kills, 12);
            assert_eq!(app.profiles.details.as_ref().unwrap().careers.len(), 2, "every commander's rank");
            assert_eq!((app.page, app.profiles.tab), (super::super::Page::Profile, tab));
        }
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn profile_menu_shows_both_commanders_and_management_controls_without_needless_scroll() {
        fn visible_text(shape: &egui::Shape, clip: Rect, expected: &str) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.text() == expected && clip.contains_rect(shape.visual_bounding_rect()),
                egui::Shape::Vec(shapes) => shapes.iter().any(|shape| visible_text(shape, clip, expected)),
                _ => false,
            }
        }
        let (root, mut app, _, _, _) = launcher("menu_height");
        let ctx = egui::Context::default();
        ta_ui::theme::install(&ctx);
        app.profiles.show_menu();
        for frame in 0..4 {
            let mut output = ctx.run_ui(egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
                time: Some(frame as f64 / 60.0), ..Default::default()
            }, |ui| { app.profile_widget(ui); });
            output.textures_delta.clear();
            if frame == 3 {
                for label in ["First", "Second", "NEW PROFILE", "EDIT PROFILE"] {
                    assert!(output.shapes.iter().any(|clipped| visible_text(&clipped.shape, clipped.clip_rect, label)),
                        "profile menu needlessly hides {label}");
                }
            }
        }
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

}
